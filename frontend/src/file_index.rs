//! Ahead-of-time name synchronization. Query text never crosses the network.
//! One bounded memory-only snapshot, fenced by chat, root and source lineage.
use std::{sync::Arc, time::Duration};
use tau_code_viewer::finder::PathIndex;
use tau_protocol::files::*;
use tokio::sync::watch;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Interest { pub generation: u64, pub session: String, pub path: Option<String> }
pub struct Update {
    pub generation: u64, pub session: String, pub lineage: String,
    pub index: Option<Arc<PathIndex>>, pub indexing: bool, pub limited: bool, pub error: Option<String>,
}
pub(crate) struct Service { plans: watch::Sender<Option<Interest>>, task: tokio::task::JoinHandle<()> }
struct Cached { session: String, root: Option<String>, lineage: String, index: Arc<PathIndex>, indexing: bool, limited: bool }
impl Service {
    pub fn start(mut client: watch::Receiver<Option<Arc<tau_transfer::blocks::Client>>>, mut ready: watch::Receiver<Option<String>>, updates: watch::Sender<Option<Arc<Update>>>, wake: crate::transport::Wake) -> Self {
        let (plans, mut interest) = watch::channel::<Option<Interest>>(None);
        let task = tokio::spawn(async move {
            let mut cached = None;
            loop {
                let plan = interest.borrow_and_update().clone();
                let endpoint = client.borrow_and_update().clone();
                let lineage = ready.borrow_and_update().clone();
                if plan.is_none() { updates.send_replace(None); }
                if cached.as_ref().is_some_and(|c: &Cached| lineage.as_ref().is_some_and(|l| l != &c.lineage)) { cached = None; }
                let read = async {
                    let (Some(plan), Some(endpoint), Some(lineage)) = (plan, endpoint, lineage) else { return std::future::pending::<()>().await; };
                    refresh(endpoint, plan, lineage, &mut cached, &updates, &wake).await;
                };
                tokio::select! {
                    _ = read => {},
                    changed = interest.changed() => { if changed.is_err() { break; } },
                    changed = client.changed() => { if changed.is_err() { break; } },
                    changed = ready.changed() => { if changed.is_err() { break; } },
                }
            }
        });
        Self { plans, task }
    }
    pub fn set(&self, interest: Option<Interest>) { self.plans.send_replace(interest); }
}
impl Drop for Service { fn drop(&mut self) { self.task.abort(); } }
async fn refresh(client: Arc<tau_transfer::blocks::Client>, plan: Interest, lineage: String, cached: &mut Option<Cached>, updates: &watch::Sender<Option<Arc<Update>>>, wake: &crate::transport::Wake) {
    if cached.as_ref().is_some_and(|c| c.lineage != lineage) { *cached = None; }
    let mut previous = None;
    let mut reset = false;
    loop {
        let revision = (!reset).then(|| cached.as_ref().map(|c| c.index.revision.clone())).flatten();
        let result = client.files(FileRequest { session_id: plan.session.clone(), path: plan.path.clone(), operation: FileOperation::Index { revision } }).await;
        let received_reply = result.is_ok();
        let result = async {
            let reply = result?;
            let FileReply::Index { ref path, ref revision, indexing, limited, .. } = reply else { anyhow::bail!("Expected a file index") };
            let old = cached.as_ref().map(|c| c.index.clone());
            let unchanged = matches!(&reply, FileReply::Index { base: Some(base), entries, removed, .. } if base == revision && entries.is_empty() && removed.is_empty())
                && old.as_ref().is_some_and(|c| c.root == *path && c.revision == *revision);
            let index = if unchanged { old.unwrap() } else {
                tokio::task::spawn_blocking(move || PathIndex::apply(old.as_deref(), &reply).map(Arc::new)).await??
            };
            *cached = Some(Cached { session: plan.session.clone(), root: plan.path.clone(), lineage: lineage.clone(), index, indexing, limited });
            Ok::<_, anyhow::Error>(())
        }.await;
        // Connection loss does not invalidate verified names. Only a bad
        // delta/reply requires a full snapshot on retry.
        if result.is_ok() { reset = false; } else if received_reply { reset = true; }
        let error = result.err().map(|e| e.to_string());
        // A request for a different chat/root may reuse a revision on the wire,
        // but may not expose that snapshot until the daemon confirms the root.
        let visible = cached.as_ref().filter(|c| c.session == plan.session && c.root == plan.path);
        let indexing = visible.is_none_or(|c| c.indexing);
        let limited = visible.is_some_and(|c| c.limited);
        let state = (visible.map(|c| c.index.revision.clone()), indexing, limited, error.clone());
        if previous.as_ref() != Some(&state) {
            previous = Some(state);
            updates.send_replace(Some(Arc::new(Update { generation: plan.generation, session: plan.session.clone(), lineage: lineage.clone(), index: visible.map(|c| c.index.clone()), indexing, limited, error: error.clone() })));
            (wake)();
        }
        tokio::time::sleep(if indexing || error.is_some() { Duration::from_secs(1) } else { Duration::from_secs(10) }).await;
    }
}

/// One reusable matcher thread with latest-only input/output. A slow previous
/// query cannot publish over a newer keystroke or an updated index.
pub(crate) struct Matches { pub generation: u64, pub index: Arc<PathIndex>, pub rows: Vec<usize> }
struct Query { generation: u64, index: Arc<PathIndex>, text: String, hidden: bool }
struct Work {
    query: std::sync::Mutex<Option<Query>>, done: std::sync::Mutex<Option<Arc<Matches>>>,
    wake: std::sync::Condvar, generation: std::sync::atomic::AtomicU64, stop: std::sync::atomic::AtomicBool,
}
pub(crate) struct Matcher { work: Arc<Work> }
impl Matcher {
    pub fn new(wake: crate::transport::Wake) -> Self {
        use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
        let work = Arc::new(Work { query: Default::default(), done: Default::default(), wake: Default::default(), generation: AtomicU64::new(0), stop: AtomicBool::new(false) });
        let w = work.clone();
        std::thread::Builder::new().name("tau-file-match".into()).spawn(move || {
            let mut finder = tau_code_viewer::finder::Finder::new("");
            loop {
                let query = {
                    let mut slot = w.query.lock().unwrap();
                    while slot.is_none() && !w.stop.load(Ordering::Relaxed) { slot = w.wake.wait(slot).unwrap(); }
                    if w.stop.load(Ordering::Relaxed) { return; }
                    slot.take().unwrap()
                };
                finder.query(&query.text);
                let mut ranked = Vec::new();
                let cancelled = || w.stop.load(Ordering::Relaxed) || w.generation.load(Ordering::Relaxed) != query.generation;
                for (i, path) in query.index.entries.iter().enumerate() {
                    if i % 128 == 0 && cancelled() { break; }
                    if !query.hidden && path.hidden() { continue; }
                    if let Some(score) = finder.score(&path.path) { ranked.push((std::cmp::Reverse(score), path.path.len(), i)); }
                }
                if cancelled() { continue; }
                ranked.sort_unstable();
                if cancelled() { continue; }
                *w.done.lock().unwrap() = Some(Arc::new(Matches { generation: query.generation, index: query.index, rows: ranked.into_iter().map(|(_, _, i)| i).collect() }));
                (wake)();
            }
        }).expect("start local file matcher");
        Self { work }
    }
    pub fn query(&self, index: Arc<PathIndex>, text: String, hidden: bool) -> u64 {
        let mut slot = self.work.query.lock().unwrap();
        let generation = self.work.generation.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        *slot = Some(Query { generation, index, text, hidden });
        self.work.wake.notify_one(); generation
    }
    pub fn cancel(&self) -> u64 {
        let mut slot = self.work.query.lock().unwrap();
        let generation = self.work.generation.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        *slot = None;
        self.work.done.lock().unwrap().take();
        generation
    }
    pub fn take(&self) -> Option<Arc<Matches>> { self.work.done.lock().unwrap().take() }
}
impl Drop for Matcher {
    fn drop(&mut self) {
        let _slot = self.work.query.lock().unwrap();
        self.work.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        self.work.wake.notify_one();
    }
}

#[cfg(test)]
mod tests;
