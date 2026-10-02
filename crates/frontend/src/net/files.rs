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
pub(super) async fn watch_files(mut client: watch::Receiver<Option<Arc<tau_net::native::Client>>>, mut ready: watch::Receiver<Option<String>>, updates: watch::Sender<Option<Arc<FileUpdate>>>, wake: crate::net::Wake, mut interest: watch::Receiver<Option<FileInterest>>) {
    loop {
        let plan = interest.borrow_and_update().clone();
        if plan.is_none() {updates.send_replace(None);}
        let endpoint = client.borrow_and_update().clone();
        let lineage = ready.borrow_and_update().clone();
        let read = async {
            let (Some(plan), Some(endpoint), Some(lineage)) = (plan, endpoint, lineage) else { return std::future::pending::<()>().await; };
            refresh_files(endpoint, plan, lineage, &updates, &wake).await;
        };
        tokio::select! {
            _ = read => {},
            changed = interest.changed() => { if changed.is_err() { break; } },
            changed = client.changed() => { if changed.is_err() { break; } },
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
mod viewer_tests {
    use super::*;
    use anyhow::Result;
    use futures_util::{future::BoxFuture, FutureExt};
    use std::sync::Mutex;
    use tau_net::blocks::{BlockRequest, ContentRange, FeedPage, FeedRequest};
    use tau_net::native::{Backend, Client, Server};
    struct Memory { calls: Mutex<Vec<String>>, hints: watch::Sender<u64> }
    impl Backend for Memory {
        fn feed(&self, _: FeedRequest) -> BoxFuture<'static, Result<FeedPage>> { async { anyhow::bail!("unused") }.boxed() }
        fn read(&self, _: BlockRequest) -> BoxFuture<'static, Result<ContentRange>> { async { anyhow::bail!("unused") }.boxed() }
        fn changes(&self) -> watch::Receiver<u64> { self.hints.subscribe() }
        fn files(&self, request: FileRequest) -> BoxFuture<'static, Result<FileReply>> {
            let path=request.path.unwrap();self.calls.lock().unwrap().push(path.clone());
            async move {
                if path=="/slow.rs" {std::future::pending::<()>().await;}
                let text=format!("// {path}\nfn main() {{}}\n");
                Ok(FileReply::Text {path,revision:blake3::hash(text.as_bytes()).to_hex().to_string(),text})
            }.boxed()
        }
    }
    async fn until(mut ready: impl FnMut()->bool) {
        tokio::time::timeout(Duration::from_secs(5),async {while !ready() {tokio::time::sleep(Duration::from_millis(5)).await;}}).await.unwrap();
    }
    #[tokio::test]
    async fn previews_coalesce_rapid_selection_and_cancel_stale_native_reads() {
        let backend=Arc::new(Memory {calls:Mutex::new(vec![]),hints:watch::channel(0).0});
        let server=Server::bind("127.0.0.1:0".parse().unwrap(),backend.clone()).await.unwrap();
        let client=Arc::new(Client::bind().await.unwrap());
        client.configure(&server.authorize(&client.node_id(),"source".into()).unwrap(),"127.0.0.1").await.unwrap();
        let (_endpoint,clients)=watch::channel(Some(client.clone()));let (_lineage,ready)=watch::channel(Some("source".into()));
        let (send,updates)=watch::channel(None);
        let (plans,interest)=watch::channel(None);
        let service=tokio::spawn(watch_files(clients,ready,send,Arc::new(||{}),interest));
        let plan=|generation,path:String|Some(FileInterest {preview:true,generation,request:FileRequest {session_id:"chat".into(),path:Some(path),operation:FileOperation::Open {revision:None}},document:None});
        for generation in 0..20 {plans.send_replace(plan(generation,format!("/file{generation}.rs")));}
        until(||updates.borrow().as_ref().is_some_and(|u|u.generation==19)).await;
        assert_eq!(*backend.calls.lock().unwrap(),["/file19.rs"],"rapid row movement should fetch only the settled preview");
        assert!(updates.borrow().as_ref().unwrap().document.as_ref().unwrap().text.contains("/file19.rs"));
        plans.send_replace(plan(20,"/slow.rs".into()));until(||backend.calls.lock().unwrap().last().is_some_and(|p|p=="/slow.rs")).await;
        plans.send_replace(plan(21,"/new.rs".into()));until(||updates.borrow().as_ref().is_some_and(|u|u.generation==21)).await;
        assert!(updates.borrow().as_ref().unwrap().document.as_ref().unwrap().text.contains("/new.rs"));
        plans.send_replace(None);until(||updates.borrow().is_none()).await;
        assert_eq!(client.stats().active_streams,0);assert_eq!(client.stats().foreground_slots,0);
        service.abort();client.shutdown().await;server.shutdown().await;
    }
}

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

pub(super) async fn watch_index(mut client: watch::Receiver<Option<Arc<tau_net::native::Client>>>, mut ready: watch::Receiver<Option<String>>, updates: watch::Sender<Option<Arc<IndexUpdate>>>, wake: crate::net::Wake, mut interest: watch::Receiver<Option<IndexInterest>>) {
    let mut cached = None;
    loop {
        let plan = interest.borrow_and_update().clone();
        let endpoint = client.borrow_and_update().clone();
        let lineage = ready.borrow_and_update().clone();
        if plan.is_none() { updates.send_replace(None); }
        if cached.as_ref().is_some_and(|c: &Cached| lineage.as_ref().is_some_and(|l| l != &c.lineage)) { cached = None; }
        let read = async {
            let (Some(plan), Some(endpoint), Some(lineage)) = (plan, endpoint, lineage) else { return std::future::pending::<()>().await; };
            refresh_index(endpoint, plan, lineage, &mut cached, &updates, &wake).await;
        };
        tokio::select! {
            _ = read => {},
            changed = interest.changed() => { if changed.is_err() { break; } },
            changed = client.changed() => { if changed.is_err() { break; } },
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
mod index_tests {
use super::*;
use std::sync::{Mutex, atomic::{AtomicBool, Ordering}};
use anyhow::Result;
use futures_util::{future::BoxFuture, FutureExt};
use tau_net::blocks::{FeedRequest, FeedPage, BlockRequest, ContentRange};
use tau_net::native::{Backend, Client, Server};

struct Memory { reply: Mutex<FileReply>, calls: Mutex<Vec<FileRequest>>, fail: AtomicBool, hints: watch::Sender<u64> }
impl Backend for Memory {
    fn feed(&self, _: FeedRequest) -> BoxFuture<'static, Result<FeedPage>> { async { anyhow::bail!("unused") }.boxed() }
    fn read(&self, _: BlockRequest) -> BoxFuture<'static, Result<ContentRange>> { async { anyhow::bail!("unused") }.boxed() }
    fn changes(&self) -> watch::Receiver<u64> { self.hints.subscribe() }
    fn files(&self, request: FileRequest) -> BoxFuture<'static, Result<FileReply>> {
        self.calls.lock().unwrap().push(request);
        let reply = self.reply.lock().unwrap().clone(); let fail = self.fail.load(Ordering::Relaxed);
        async move { if fail { anyhow::bail!("temporarily unavailable") } Ok(reply) }.boxed()
    }
}
fn snapshot(revision: &str, base: Option<&str>, paths: &[&str], removed: &[&str]) -> FileReply {
    FileReply::Index {path:"/work".into(), revision:revision.repeat(64), base:base.map(|b|b.repeat(64)),
        entries:paths.iter().map(|p|IndexedPath {path:(*p).into(),symlink:false}).collect(),removed:removed.iter().map(|s|(*s).into()).collect(),indexing:false,limited:false}
}
async fn update(updates: &mut watch::Receiver<Option<Arc<IndexUpdate>>>, generation: u64) -> Arc<IndexUpdate> {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(u)=updates.borrow_and_update().clone() && u.generation==generation {return u;}
            updates.changed().await.unwrap();
        }
    }).await.expect("index sync stalled")
}
#[tokio::test]
async fn sync_prefetches_names_then_uses_conditional_deltas_across_chat_switches_and_suspension() {
    let backend=Arc::new(Memory {reply:Mutex::new(snapshot("a",None,&["src/a.rs","src/b.rs"],&[])),calls:Mutex::new(vec![]),fail:AtomicBool::new(false),hints:watch::channel(0).0});
    let server=Server::bind("127.0.0.1:0".parse().unwrap(),backend.clone()).await.unwrap();
    let client=Arc::new(Client::bind().await.unwrap());
    client.configure(&server.authorize(&client.node_id(),"source".into()).unwrap(),"127.0.0.1").await.unwrap();
    let (endpoint,clients)=watch::channel(Some(client.clone())); let (lineage,ready)=watch::channel(Some("source".into()));
    let (send,mut updates)=watch::channel(None);
    let (wake,mut wakes)=tokio::sync::mpsc::unbounded_channel();
    let (plans,interest)=watch::channel(None);
    let service=tokio::spawn(watch_index(clients,ready,send,Arc::new(move || {let _=wake.send(());}),interest));
    let plan=|generation,session:&str|Some(IndexInterest {generation,session:session.into(),path:None});
    plans.send_replace(plan(1,"chat"));
    let first=update(&mut updates,1).await;assert_eq!(first.index.as_ref().unwrap().entries.len(),2);
    tokio::time::timeout(Duration::from_secs(1),wakes.recv()).await.unwrap().unwrap();
    assert!(wakes.try_recv().is_err(),"One names publication emits one wake, not a repaint loop");
    assert!(matches!(backend.calls.lock().unwrap()[0].operation,FileOperation::Index {revision:None}));
    // No viewer/file-open interest is needed to warm the index.
    plans.send_replace(None);tokio::time::sleep(Duration::from_millis(20)).await;
    *backend.reply.lock().unwrap()=snapshot("a",Some("a"),&[],&[]);
    plans.send_replace(plan(2,"other-chat"));
    let same=update(&mut updates,2).await;
    assert!(Arc::ptr_eq(first.index.as_ref().unwrap(),same.index.as_ref().unwrap()),"unchanged names reuse the entire snapshot");
    assert_eq!(same.session,"other-chat");
    assert!(matches!(&backend.calls.lock().unwrap().last().unwrap().operation,FileOperation::Index {revision:Some(r)} if r==&"a".repeat(64)));
    *backend.reply.lock().unwrap()=snapshot("b",Some("a"),&["src/c.rs"],&["src/a.rs"]);
    plans.send_replace(plan(3,"other-chat"));
    let changed=update(&mut updates,3).await;assert_eq!(changed.index.as_ref().unwrap().entries.iter().map(|p|p.path.as_str()).collect::<Vec<_>>(),["src/b.rs","src/c.rs"]);
    backend.fail.store(true,Ordering::Relaxed);plans.send_replace(plan(4,"other-chat"));
    let failed=update(&mut updates,4).await;assert!(failed.error.is_some());assert!(Arc::ptr_eq(changed.index.as_ref().unwrap(),failed.index.as_ref().unwrap()),"a temporary failure doesn't empty the picker");
    plans.send_replace(Some(IndexInterest {generation:40,session:"unavailable-chat".into(),path:Some("/other-root".into())}));
    let foreign=update(&mut updates,40).await;assert!(foreign.error.is_some());assert!(foreign.index.is_none(),"an unverified root cannot inherit names from a previous chat");
    backend.fail.store(false,Ordering::Relaxed);
    *backend.reply.lock().unwrap()=snapshot("b",Some("b"),&[],&[]);plans.send_replace(plan(5,"other-chat"));
    assert!(update(&mut updates,5).await.error.is_none());
    assert!(matches!(&backend.calls.lock().unwrap().last().unwrap().operation,FileOperation::Index {revision:Some(r)} if r==&"b".repeat(64)));
    // An incompatible delta triggers a full resync, never a partial publication.
    *backend.reply.lock().unwrap()=snapshot("c",Some("f"),&["src/new.rs"],&[]);plans.send_replace(plan(6,"other-chat"));
    let bad=update(&mut updates,6).await;assert!(bad.error.is_some());assert_eq!(bad.index.as_ref().unwrap().revision,"b".repeat(64));
    *backend.reply.lock().unwrap()=snapshot("c",None,&["src/new.rs"],&[]);
    tokio::time::timeout(Duration::from_secs(5),async {
        loop {updates.changed().await.unwrap();if updates.borrow().as_ref().is_some_and(|u|u.generation==6 && u.error.is_none()){break;}}
    }).await.unwrap();
    assert!(matches!(&backend.calls.lock().unwrap().last().unwrap().operation,FileOperation::Index {revision:None}));
    *backend.reply.lock().unwrap()=snapshot("d",None,&["fresh.rs"],&[]);
    lineage.send_replace(Some("new-source".into()));
    tokio::time::timeout(Duration::from_secs(5),async {
        loop {updates.changed().await.unwrap();if updates.borrow().as_ref().is_some_and(|u|u.lineage=="new-source"){break;}}
    }).await.unwrap();
    assert!(matches!(&backend.calls.lock().unwrap().last().unwrap().operation,FileOperation::Index {revision:None}),"new source cannot inherit old names");
    plans.send_replace(None);tokio::time::sleep(Duration::from_millis(40)).await;
    assert!(updates.borrow().is_none());assert_eq!(client.stats().active_streams,0);assert_eq!(client.stats().foreground_slots,0);assert_eq!(client.stats().bulk_slots,0);
    let calls=backend.calls.lock().unwrap().len();tokio::time::sleep(Duration::from_millis(80)).await;assert_eq!(backend.calls.lock().unwrap().len(),calls);
    service.abort();drop(endpoint);client.shutdown().await;server.shutdown().await;
}

}
