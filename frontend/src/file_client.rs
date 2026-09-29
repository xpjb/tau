//! One coalesced viewer interest per UI. No filesystem bodies enter the replica
//! or control queues. Dropping/changing interest cancels its native stream.
use std::{sync::Arc, time::Duration};
use tau_protocol::files::*;
use tokio::sync::watch;

#[derive(Clone)]
pub struct Interest { pub preview: bool, pub generation: u64, pub request: FileRequest, pub document: Option<Arc<tau_code_viewer::Document>> }
pub struct Update {
    pub generation: u64, pub session: String, pub lineage: String,
    pub response: Result<FileReply, String>, pub document: Option<Arc<tau_code_viewer::Document>>,
}
pub(crate) struct Service { plans: watch::Sender<Option<Interest>>, task: tokio::task::JoinHandle<()> }
impl Service {
    pub fn start(mut client: watch::Receiver<Option<Arc<tau_transfer::blocks::Client>>>, mut ready: watch::Receiver<Option<String>>, updates: watch::Sender<Option<Arc<Update>>>, wake: crate::transport::Wake) -> Self {
        let (plans, mut interest) = watch::channel::<Option<Interest>>(None);
        let task = tokio::spawn(async move {
            loop {
                let plan = interest.borrow_and_update().clone();
                if plan.is_none() {updates.send_replace(None);}
                let endpoint = client.borrow_and_update().clone();
                let lineage = ready.borrow_and_update().clone();
                let read = async {
                    let (Some(plan), Some(endpoint), Some(lineage)) = (plan, endpoint, lineage) else { return std::future::pending::<()>().await; };
                    refresh(endpoint, plan, lineage, &updates, &wake).await;
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
async fn refresh(client: Arc<tau_transfer::blocks::Client>, plan: Interest, lineage: String, updates: &watch::Sender<Option<Arc<Update>>>, wake: &crate::transport::Wake) {
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
            updates.send_replace(Some(Arc::new(Update { generation: plan.generation, session: request.session_id.clone(), lineage: lineage.clone(), response, document: document.clone() })));
            (wake)();
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use futures_util::{future::BoxFuture, FutureExt};
    use std::sync::Mutex;
    use tau_blocks::{BlockRequest, ContentRange, FeedPage, FeedRequest};
    use tau_transfer::blocks::{Backend, Client, Server};
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
        let service=Service::start(clients,ready,send,Arc::new(||{}));
        let plan=|generation,path:String|Some(Interest {preview:true,generation,request:FileRequest {session_id:"chat".into(),path:Some(path),operation:FileOperation::Open {revision:None}},document:None});
        for generation in 0..20 {service.set(plan(generation,format!("/file{generation}.rs")));}
        until(||updates.borrow().as_ref().is_some_and(|u|u.generation==19)).await;
        assert_eq!(*backend.calls.lock().unwrap(),["/file19.rs"],"rapid row movement should fetch only the settled preview");
        assert!(updates.borrow().as_ref().unwrap().document.as_ref().unwrap().text.contains("/file19.rs"));
        service.set(plan(20,"/slow.rs".into()));until(||backend.calls.lock().unwrap().last().is_some_and(|p|p=="/slow.rs")).await;
        service.set(plan(21,"/new.rs".into()));until(||updates.borrow().as_ref().is_some_and(|u|u.generation==21)).await;
        assert!(updates.borrow().as_ref().unwrap().document.as_ref().unwrap().text.contains("/new.rs"));
        service.set(None);until(||updates.borrow().is_none()).await;
        assert_eq!(client.stats().active_streams,0);assert_eq!(client.stats().foreground_slots,0);
        drop(service);client.shutdown().await;server.shutdown().await;
    }
}
