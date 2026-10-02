use super::*;
use anyhow::Result;
use futures_util::{future::BoxFuture, FutureExt};
use std::sync::Mutex;
use tau_net::blocks::{BlockRequest, ContentRange, FeedPage, FeedRequest};
use tau_net::native::{Backend, Client, Server};
struct Memory { calls: Mutex<Vec<String>>, hints: watch::Sender<u64> }
impl Backend for Memory {
    fn feed(&self, _: FeedRequest) -> BoxFuture<'static, Result<FeedPage>> { async { anyhow::bail!("unused") }.boxed() }
    fn read(&self, _: BlockRequest) -> BoxFuture<'static, Result<Option<ContentRange>>> { async { anyhow::bail!("unused") }.boxed() }
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
    let client=Client::bind().await.unwrap();
    client.configure(&server.authorize(&client.node_id(),"source".into()).unwrap(),"127.0.0.1").await.unwrap();
    let (_lineage,ready)=watch::channel(Some("source".into()));
    let (send,updates)=watch::channel(None);
    let (plans,interest)=watch::channel(None);
    let service=tokio::spawn(watch_files(client.clone(),ready,send,Arc::new(||{}),interest));
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
