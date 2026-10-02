//! One coalesced viewer interest per UI. No filesystem bodies enter the replica
//! or control queues. Dropping/changing interest cancels its native stream.
use std::{sync::Arc, time::Duration};
use tau_net::files::*;
use tokio::sync::watch;

#[derive(Clone)]
pub struct FileInterest { pub preview: bool, pub generation: u64, pub request: FileRequest, pub document: Option<Arc<tau_code_viewer::Document>> }
pub struct FileUpdate {
    pub generation: u64, pub session: String, pub lineage: String,
    pub response: Result<FileReply, String>, pub document: Option<Arc<tau_code_viewer::Document>>,
}
pub(super) async fn watch_files(client: Arc<tau_net::native::Client>, mut ready: watch::Receiver<Option<String>>, updates: watch::Sender<Option<Arc<FileUpdate>>>, wake: crate::net::Wake, mut interest: watch::Receiver<Option<FileInterest>>) {
    loop {
        let plan = interest.borrow_and_update().clone();
        if plan.is_none() {updates.send_replace(None);}
        let lineage = ready.borrow_and_update().clone();
        let read = async {
            let (Some(plan), Some(lineage)) = (plan, lineage) else { return std::future::pending::<()>().await; };
            refresh_files(client.clone(), plan, lineage, &updates, &wake).await;
        };
        tokio::select! {
            _ = read => {},
            changed = interest.changed() => { if changed.is_err() { break; } },
            changed = ready.changed() => { if changed.is_err() { break; } },
        }
    }
}
async fn refresh_files(client: Arc<tau_net::native::Client>, plan: FileInterest, lineage: String, updates: &watch::Sender<Option<Arc<FileUpdate>>>, wake: &crate::net::Wake) {
    let mut request = plan.request.clone();
    if plan.preview { tokio::time::sleep(Duration::from_millis(75)).await; }
    let mut document = plan.document.clone();
    let mut previous = None;
    loop {
        let result = client.files(request.clone()).await;
        let response = match result {
            Ok(FileReply::Text {text,..}) if text.len()>MAX_FILE_BYTES || text.bytes().filter(|&b|b==b'\n').count()>MAX_FILE_LINES => Err("File exceeds code preview limits".into()),
            Ok(FileReply::Text {ref revision,ref text,..}) if blake3::hash(text.as_bytes()).to_hex().as_str()!=revision => Err("File revision integrity check failed".into()),
            Ok(FileReply::Text { path, revision, text }) => {
                let old = document.clone(); let p = path.clone(); let r = revision.clone();
                match tokio::task::spawn_blocking(move || tau_code_viewer::Document::replace(old.as_deref(), p, r, text)).await {
                    Ok(doc) => {
                        document = Some(Arc::new(doc));
                        request.path = Some(path.clone()); request.operation = FileOperation::Open { revision: Some(revision.clone()) };
                        Ok(FileReply::Text { path, revision, text: String::new() })
                    }
                    Err(_) => Err("Could not prepare code preview".into()),
                }
            }
            Ok(reply) => Ok(reply),
            Err(error) => Err(error.to_string()),
        };
        // Unchanged files don't wake a sleeping renderer. An unchanged response
        // after an error still clears the stale/error state and restores selection.
        let unchanged = matches!(response, Ok(FileReply::Unchanged {..})) && previous.as_ref().is_some_and(Result::is_ok);
        if !unchanged && previous.as_ref() != Some(&response) {
            previous = Some(response.clone());
            updates.send_replace(Some(Arc::new(FileUpdate { generation: plan.generation, session: request.session_id.clone(), lineage: lineage.clone(), response, document: document.clone() })));
            (wake)();
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

#[cfg(test)]
#[path = "../../tests/unit/net/files.rs"]
mod viewer_tests;

// Ahead-of-time name synchronization. Query text never crosses the network.
// One bounded memory-only snapshot, fenced by chat, root and source lineage.
use tau_code_viewer::finder::PathIndex;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexInterest { pub generation: u64, pub session: String, pub path: Option<String> }
pub struct IndexUpdate {
    pub generation: u64, pub session: String, pub lineage: String,
    pub index: Option<Arc<PathIndex>>, pub indexing: bool, pub limited: bool, pub error: Option<String>,
}
struct Cached { session: String, root: Option<String>, lineage: String, index: Arc<PathIndex>, indexing: bool, limited: bool }

pub(super) async fn watch_index(client: Arc<tau_net::native::Client>, mut ready: watch::Receiver<Option<String>>, updates: watch::Sender<Option<Arc<IndexUpdate>>>, wake: crate::net::Wake, mut interest: watch::Receiver<Option<IndexInterest>>) {
    let mut cached = None;
    loop {
        let plan = interest.borrow_and_update().clone();
        let lineage = ready.borrow_and_update().clone();
        if plan.is_none() { updates.send_replace(None); }
        if cached.as_ref().is_some_and(|c: &Cached| lineage.as_ref().is_some_and(|l| l != &c.lineage)) { cached = None; }
        let read = async {
            let (Some(plan), Some(lineage)) = (plan, lineage) else { return std::future::pending::<()>().await; };
            refresh_index(client.clone(), plan, lineage, &mut cached, &updates, &wake).await;
        };
        tokio::select! {
            _ = read => {},
            changed = interest.changed() => { if changed.is_err() { break; } },
            changed = ready.changed() => { if changed.is_err() { break; } },
        }
    }
}
async fn refresh_index(client: Arc<tau_net::native::Client>, plan: IndexInterest, lineage: String, cached: &mut Option<Cached>, updates: &watch::Sender<Option<Arc<IndexUpdate>>>, wake: &crate::net::Wake) {
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
            updates.send_replace(Some(Arc::new(IndexUpdate { generation: plan.generation, session: plan.session.clone(), lineage: lineage.clone(), index: visible.map(|c| c.index.clone()), indexing, limited, error: error.clone() })));
            (wake)();
        }
        tokio::time::sleep(if indexing || error.is_some() { Duration::from_secs(1) } else { Duration::from_secs(10) }).await;
    }
}


#[cfg(test)]
#[path = "../../tests/unit/net/files_index.rs"]
mod index_tests;
