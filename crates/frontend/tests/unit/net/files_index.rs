use super::*;
use std::sync::{Mutex, atomic::{AtomicBool, Ordering}};
use anyhow::Result;
use futures_util::{future::BoxFuture, FutureExt};
use tau_net::blocks::{FeedRequest, FeedPage, BlockRequest, ContentRange};
use tau_net::native::{Backend, Client, Server};

struct Memory { reply: Mutex<FileReply>, calls: Mutex<Vec<FileRequest>>, fail: AtomicBool, hints: watch::Sender<u64> }
impl Backend for Memory {
fn feed(&self, _: FeedRequest) -> BoxFuture<'static, Result<FeedPage>> { async { anyhow::bail!("unused") }.boxed() }
fn read(&self, _: BlockRequest) -> BoxFuture<'static, Result<Option<ContentRange>>> { async { anyhow::bail!("unused") }.boxed() }
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
let client=Client::bind().await.unwrap();
client.configure(&server.authorize(&client.node_id(),"source".into()).unwrap(),"127.0.0.1").await.unwrap();
let (lineage,ready)=watch::channel(Some("source".into()));
let (send,mut updates)=watch::channel(None);
let (wake,mut wakes)=tokio::sync::mpsc::unbounded_channel();
let (plans,interest)=watch::channel(None);
let service=tokio::spawn(watch_index(client.clone(),ready,send,Arc::new(move || {let _=wake.send(());}),interest));
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
service.abort();client.shutdown().await;server.shutdown().await;
}
