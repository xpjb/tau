use super::*;
use crate::replica::{tests::Fixture, QUEUE};
use tau_net::QueueState;
use rusqlite::Connection;
use std::sync::Mutex;
use serde_json::json;
use crate::replica::tests::{local_prompt, user_body};

async fn content(cache: Cache, notices: mpsc::Sender<ReplicaNotice>) -> (Content, Subscriptions, super::super::EventReceiver) {
    let (events,receiver)=super::super::mailbox::channel(Arc::new(||{}));
    let (subscriptions,interests)=subscriptions();
    (Content::start(cache,events,notices,watch::channel(None).0,watch::channel(None).0,interests).await.unwrap(),subscriptions,receiver)
}

#[tokio::test(flavor="multi_thread", worker_threads=2)]
async fn native_watch_fetches_unknown_body_but_never_downloads_locally_known_input() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tau_net::native::Backend;
    struct Source {
        db: Arc<Mutex<Connection>>,
        known_reads: Arc<AtomicUsize>,
        unknown_reads: Arc<AtomicUsize>,
        changes: watch::Receiver<u64>,
    }
    impl Backend for Source {
        fn feed(&self, req: FeedRequest) -> futures_util::future::BoxFuture<'static, Result<FeedPage>> {
            let db=self.db.clone(); Box::pin(async move { tau_block_store::feed(&db.lock().unwrap(),&req) })
        }
        fn read(&self, req: BlockRequest) -> futures_util::future::BoxFuture<'static, Result<ContentRange>> {
            let db=self.db.clone();let known=self.known_reads.clone();let unknown=self.unknown_reads.clone();
            Box::pin(async move {
                if req.id=="saved" {known.fetch_add(1,Ordering::SeqCst);}
                if req.id=="unknown" {unknown.fetch_add(1,Ordering::SeqCst);}
                tau_block_store::read(&db.lock().unwrap(),&req)
            })
        }
        fn changes(&self) -> watch::Receiver<u64> {self.changes.clone()}
    }
    let mut f=Fixture::new();
    let text="authored café 😀".repeat(4096);
    let local=local_prompt("request",&text);
    f.cache.remember_local("chat",&local,&f.lineage).unwrap();
    f.put(QUEUE,None,100,BlockKind::Queue,json!({}),&serde_json::to_vec(&QueueState::native()).unwrap());
    f.put("saved",None,1,BlockKind::Text,user_body("saved","request",&text),text.as_bytes());
    f.put("unknown",None,2,BlockKind::Text,user_body("unknown","another-client","remote text"),b"remote text");
    let known=Arc::new(AtomicUsize::new(0));let unknown=Arc::new(AtomicUsize::new(0));
    let (_changes,changed)=watch::channel(0);
    let server=tau_net::native::Server::bind("127.0.0.1:0".parse().unwrap(),Arc::new(Source {
        db:Arc::new(Mutex::new(f.source)),known_reads:known.clone(),unknown_reads:unknown.clone(),changes:changed,
    })).await.unwrap();
    let (notices,mut received)=mpsc::channel(32);
    let (service,subscriptions,_events)=content(f.cache.clone(),notices).await;
    let offer=server.authorize(&service.node_id(),f.lineage.clone()).unwrap();
    service.configure(offer,"127.0.0.1".into());
    subscriptions.plans.send_replace(vec![f.cache.plan("chat",&local,&[]).unwrap()]);
    tokio::time::timeout(Duration::from_secs(5),async {
        loop {
            let notice=received.recv().await.unwrap();assert!(matches!(notice,ReplicaNotice::Changed(_)),"{notice:?}");
            subscriptions.plans.send_replace(vec![f.cache.plan("chat",&local,&[]).unwrap()]);
            if f.cache.cached_header("chat","unknown").unwrap().is_some() && f.cache.copy_ready("chat",&["unknown".into()]).unwrap().as_deref()==Some("remote text") {
                assert_eq!(f.cache.copy_ready("chat",&["saved".into()]).unwrap().unwrap(),text);
                break;
            }
        }
    }).await.unwrap();
    assert!(unknown.load(Ordering::SeqCst)>0,"the test must actually run the production body scheduler");
    assert_eq!(known.load(Ordering::SeqCst),0,"no read request at all for our already-known input");
    drop(service);server.shutdown().await;
}

#[tokio::test(flavor="multi_thread", worker_threads=2)]
async fn a_delayed_plan_does_not_refetch_known_text_after_queue_consumption() {
    use tau_net::native::Backend;
    struct Source { db:Arc<Mutex<Connection>>, reads:Arc<Mutex<Vec<String>>>, changes:watch::Receiver<u64> }
    impl Backend for Source {
        fn feed(&self, req:FeedRequest)->futures_util::future::BoxFuture<'static,Result<FeedPage>> {
            let db=self.db.clone();Box::pin(async move {tau_block_store::feed(&db.lock().unwrap(),&req)})
        }
        fn read(&self, req:BlockRequest)->futures_util::future::BoxFuture<'static,Result<ContentRange>> {
            let db=self.db.clone();let reads=self.reads.clone();Box::pin(async move {
                reads.lock().unwrap().push(req.id.clone());tau_block_store::read(&db.lock().unwrap(),&req)
            })
        }
        fn changes(&self)->watch::Receiver<u64> {self.changes.clone()}
    }
    let mut f=Fixture::new();let text="already held text café 😀";let local=local_prompt("request",text);
    f.cache.remember_local("chat",&local,&f.lineage).unwrap();
    f.put(QUEUE,None,100,BlockKind::Queue,json!({}),&serde_json::to_vec(&QueueState::native()).unwrap());
    f.put("queued:request",Some(QUEUE),0,BlockKind::Text,
        json!({"request":{"requestId":"request","revision":0,"kind":"steer","text":"","images":0},
            "bodyHash":blake3::hash(text.as_bytes()).to_hex().to_string()}),text.as_bytes());
    f.page(None,None);f.page(Some(QUEUE),None);f.body(QUEUE);
    let pending_plan=f.cache.plan("chat",&local,&[]).unwrap();
    assert!(pending_plan.blocks.iter().any(|(id,head)|id=="queued:request" && head.is_some_and(|(_,len,sealed,stored)|sealed && len==stored)));
    // The UI has built its plan, but hasn't handed it to the service yet.
    // Meanwhile the existing body-reuse fix correctly completes the merge.
    let tx=f.source.transaction().unwrap();tau_block_store::remove(&tx,"chat","queued:request").unwrap();tx.commit().unwrap();
    f.put("saved",None,1,BlockKind::Text,user_body("saved","request",text),text.as_bytes());
    f.page(Some(QUEUE),None);f.page(None,None);
    assert_eq!(f.cache.copy_ready("chat",&["saved".into()]).unwrap().unwrap(),text);
    let merged_plan=f.cache.plan("chat",&local,&[]).unwrap();
    let reads=Arc::new(Mutex::new(vec![]));let (_changes,changed)=watch::channel(0);
    let server=tau_net::native::Server::bind("127.0.0.1:0".parse().unwrap(),Arc::new(Source {
        db:Arc::new(Mutex::new(f.source)),reads:reads.clone(),changes:changed,
    })).await.unwrap();
    let (notices,mut received)=mpsc::channel(32);let (service,subscriptions,_events)=content(f.cache.clone(),notices).await;
    service.configure(server.authorize(&service.node_id(),f.lineage.clone()).unwrap(),"127.0.0.1".into());
    subscriptions.plans.send_replace(vec![pending_plan]);
    let first=tokio::time::timeout(Duration::from_secs(5),received.recv()).await.unwrap().unwrap();
    assert!(matches!(first,ReplicaNotice::Changed(_)),"The content was already present: {first:?}");
    for plan in [None,Some(merged_plan)] {
        if let Some(plan)=plan {subscriptions.plans.send_replace(vec![plan]);}
        let until=tokio::time::Instant::now()+Duration::from_millis(250);
        loop {
            tokio::select! {
                _=tokio::time::sleep_until(until)=>break,
                notice=received.recv()=>assert!(matches!(notice.unwrap(),ReplicaNotice::Changed(_)),"A stale plan must not cause a content error"),
            }
        }
    }
    let reads=reads.lock().unwrap().clone();
    assert!(reads.is_empty(),"Neither the old queue ID nor the merged body needs a read: {reads:?}");
    assert_eq!(f.cache.copy_ready("chat",&["saved".into()]).unwrap().unwrap(),text);
    drop(service);server.shutdown().await;
}

#[test]
fn background_feed_batches_cover_every_chat_with_bounded_wire_requests() {
    let f=Fixture::new();
    let feeds=(0..37).flat_map(|n| [None,Some(QUEUE.into())].map(|parent|(format!("chat-{n:03}"),parent))).collect::<BTreeSet<_>>();
    let keys=background_batches(&f.cache,feeds.clone()).unwrap();
    assert_eq!(keys.len(),5,"seventy-four feeds should not occupy thirty-seven streams");
    let mut seen=BTreeSet::new();
    for key in keys {
        let Key::BackgroundFeeds(feeds)=key else {panic!("not a background batch");};
        let requests=feeds.iter().map(|(scope,parent)|f.cache.feed_request(scope,parent.as_deref(),None).unwrap()).collect::<Vec<_>>();
        assert!(requests.len()<=16);assert!(serde_json::to_vec(&BlockWatch::Feeds {requests}).unwrap().len()<=MAX_BLOCK_HEADER_BYTES);
        for key in feeds {assert!(seen.insert(key));}
    }
    assert_eq!(seen,feeds);
}
