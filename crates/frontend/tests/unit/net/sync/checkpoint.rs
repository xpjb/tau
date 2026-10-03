//! Exercise the production watch consumer and QUIC server, not a timer model.
use super::*;
use std::sync::Mutex;
use rusqlite::Connection;
use std::sync::atomic::{AtomicUsize, Ordering};
use tau_net::native::{Backend, Server};

struct SlowSource {
    db: Arc<Mutex<Connection>>,
    changed: watch::Receiver<u64>,
    slow_feed: bool,
    feed_calls: AtomicUsize,
    reads: Arc<Mutex<Vec<BlockRequest>>>,
}
impl Backend for SlowSource {
    fn feed(&self, request: FeedRequest) -> futures_util::future::BoxFuture<'static, Result<FeedPage>> {
        let db = self.db.clone();
        let slow = self.slow_feed && self.feed_calls.fetch_add(1, Ordering::SeqCst) == 0;
        Box::pin(async move {
            // Longer than the production five-second scheduling quantum, but
            // comfortably inside the existing transport IO deadlines.
            if slow { tokio::time::sleep(Duration::from_secs(6)).await; }
            tau_block_store::feed(&db.lock().unwrap(), &request)
        })
    }
    fn read(&self, request: BlockRequest) -> futures_util::future::BoxFuture<'static, Result<Option<ContentRange>>> {
        let db = self.db.clone();
        let slow = !self.slow_feed && self.reads.lock().unwrap().is_empty();
        self.reads.lock().unwrap().push(request.clone());
        Box::pin(async move {
            if slow { tokio::time::sleep(Duration::from_secs(6)).await; }
            tau_block_store::read(&db.lock().unwrap(), &request)
        })
    }
    fn changes(&self) -> watch::Receiver<u64> { self.changed.clone() }
}

struct Fixture {
    source: Arc<Mutex<Connection>>,
    cache: Cache,
    lineage: String,
    reads: Arc<Mutex<Vec<BlockRequest>>>,
    _root: tempfile::TempDir,
}
impl Fixture {
    fn replica(&self) -> Connection {
        Connection::open_with_flags(self._root.path().join("cache.db"),rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap()
    }
    fn new() -> Self {
        let source = Connection::open_in_memory().unwrap();
        source.execute_batch("PRAGMA foreign_keys=ON").unwrap();
        tau_block_store::initialize(&source).unwrap();
        let lineage = tau_block_store::cursor(&source).unwrap().lineage;
        let root = tempfile::tempdir().unwrap();
        let cache = Cache::open(&root.path().join("cache.db")).unwrap();
        cache.configure(&lineage).unwrap();
        Self { source: Arc::new(Mutex::new(source)), cache, lineage, reads: Default::default(), _root: root }
    }
    fn put(&self, id: &str, parent: Option<&str>, bytes: &[u8]) {
        let mut db = self.source.lock().unwrap();
        let tx = db.transaction().unwrap();
        tau_block_store::put(&tx, "chat", BlockHeader {
            id: id.into(), parent: parent.map(str::to_owned), order: 1,
            kind: BlockKind::Text, meta: serde_json::json!({}),
            version: 0, length: 0, sealed: true, revision: 0,
        }, bytes).unwrap();
        tx.commit().unwrap();
    }
    async fn connect(&self, slow_feed: bool) -> (Server, Client, Peer, watch::Sender<u64>) {
        let (changes, changed) = watch::channel(0);
        let server = Server::bind("127.0.0.1:0".parse().unwrap(), Arc::new(SlowSource {
            db: self.source.clone(), changed, slow_feed, feed_calls: AtomicUsize::new(0), reads: self.reads.clone(),
        })).await.unwrap();
        let client = Client::bind().await.unwrap();
        let peer=client.configure(&server.authorize(&client.node_id(), self.lineage.clone()).unwrap(), "127.0.0.1").await.unwrap();
        (server, client, peer, changes)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn checkpoint_scheduling_commits_a_slow_metadata_round_instead_of_restarting_it() {
    let f = Fixture::new();
    f.put("parent", None, b"");
    f.put("child", Some("parent"), b"not requested");
    let (server, client, peer, _changes) = f.connect(true).await;
    let (notices, mut received) = mpsc::channel(32);
    let wake: crate::net::Wake = Arc::new(|| {});
    let key=Key::Feeds("chat".into(), vec![None, Some("parent".into())], false);
    let worker=watch_once(&key,&peer,&f.cache,&notices,&wake);
    tokio::pin!(worker);
    tokio::time::timeout(Duration::from_secs(10),async {
        loop {tokio::select! {
            result=&mut worker=>panic!("Live interest terminated: {result:?}"),
            notice=received.recv()=>{
                assert!(matches!(notice,Some(ReplicaNotice::Changed(_))));
                if client.stats().streams>=2 && [None,Some("parent")].iter().all(|parent|
                    tau_block_store::cached_feed(&f.replica(),"chat",*parent).unwrap().is_some()) {break;}
            }
        }}
    }).await.expect("The entire slow metadata round must commit and renew, not starve later feeds");
    assert!(f.reads.lock().unwrap().is_empty(), "Metadata scheduling must not fetch content");
    assert_eq!(client.stats().cancelled_streams,0,"Cooperative renewal is not a cancelled transfer");
    client.shutdown().await;
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn checkpoint_scheduling_resumes_after_a_slow_chunk_without_replaying_its_prefix() {
    let f = Fixture::new();
    let bytes = vec![b'x'; BLOCK_CHUNK_BYTES * 2 + 17];
    f.put("body", None, &bytes);
    let (server, client, peer, _changes) = f.connect(false).await;
    let (notices, _received) = mpsc::channel(32);
    let wake: crate::net::Wake = Arc::new(|| {});
    let key = Key::Block("chat".into(), "body".into(), true);
    assert!(tokio::time::timeout(Duration::from_secs(10),watch_once(
        &key,&peer,&f.cache,&notices,&wake,
    )).await.unwrap().unwrap());
    assert_eq!(client.stats().streams,2,"The slow chunk must commit before the transport renews its slice");
    assert_eq!(tau_block_store::cached_content(&f.replica(), "chat", "body").unwrap(), bytes);
    assert_eq!(f.reads.lock().unwrap().iter().map(|r| r.offset).collect::<Vec<_>>(),
        [0, BLOCK_CHUNK_BYTES as u64, (BLOCK_CHUNK_BYTES * 2) as u64]);
    assert_eq!(client.stats().connections, 1, "Yielding must reuse the native connection");
    assert_eq!(client.stats().cancelled_streams, 0);
    client.shutdown().await;
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn background_live_body_releases_its_slot_after_catching_up_and_resumes_new_bytes() {
    let f=Fixture::new();f.put("live",None,b"prefix");
    {
        let mut db=f.source.lock().unwrap();let tx=db.transaction().unwrap();
        // Publish a genuinely unsealed body through the normal source path.
        let mut h=tau_block_store::header(&tx,"chat","live").unwrap().unwrap();h.sealed=false;
        tau_block_store::put(&tx,"chat",h,b"prefix").unwrap();tx.commit().unwrap();
    }
    let (server,client,peer,_changes)=f.connect(true).await; // Only the unused feed path is slow.
    let (notices,_received)=mpsc::channel(32);let wake:crate::net::Wake=Arc::new(||{});
    let key=Key::BackgroundBlock("chat".into(),"live".into());
    assert!(tokio::time::timeout(Duration::from_secs(2),watch_once(&key,&peer,&f.cache,&notices,&wake)).await.unwrap().unwrap(),
        "a background catch-up must End, not wait five seconds for more live bytes");
    assert_eq!(client.stats().bulk_slots,0);
    {
        let mut db=f.source.lock().unwrap();let tx=db.transaction().unwrap();
        let h=tau_block_store::header(&tx,"chat","live").unwrap().unwrap();
        tau_block_store::append(&tx,"chat","live",h.version,h.length,b" suffix",false).unwrap();tx.commit().unwrap();
    }
    assert!(tokio::time::timeout(Duration::from_secs(2),watch_once(&key,&peer,&f.cache,&notices,&wake)).await.unwrap().unwrap());
    assert_eq!(tau_block_store::cached_content(&f.replica(),"chat","live").unwrap(),b"prefix suffix");
    let reads=f.reads.lock().unwrap().clone();assert_eq!(reads.len(),2);assert_eq!(reads[1].offset,6);
    assert_eq!(client.stats().bulk_slots,0);client.shutdown().await;server.shutdown().await;
}

#[tokio::test]
async fn disappearing_body_is_not_an_alert_or_an_unordered_replica_tombstone() {
    let f=Fixture::new();f.put("queued:consumed",None,b"original text");
    let request=f.cache.feed_request("chat",None,None).unwrap();
    let page=tau_block_store::feed(&f.source.lock().unwrap(),&request).unwrap();
    f.cache.page_at(&f.lineage,&request,&page,f.cache.epoch()).unwrap();
    let before=f.cache.feed_request("chat",None,None).unwrap();
    {
        let mut db=f.source.lock().unwrap();let tx=db.transaction().unwrap();
        tau_block_store::remove(&tx,"chat","queued:consumed").unwrap();tx.commit().unwrap();
    }
    let (server,client,peer,_changes)=f.connect(false).await;
    // Skip the separate slow-chunk fixture behavior in this race regression.
    f.reads.lock().unwrap().push(BlockRequest {scope:"chat".into(),id:"setup".into(),version:0,offset:0,follow:false});
    let (notices,mut received)=mpsc::channel(32);let wake:crate::net::Wake=Arc::new(||{});
    assert!(watch_once(&Key::Block("chat".into(),"queued:consumed".into(),true),&peer,&f.cache,&notices,&wake).await.unwrap());
    assert!(received.try_recv().is_err(),"An obsolete body interest must not produce an error or fabricate a change");
    assert!(f.cache.cached_header("chat","queued:consumed").unwrap().is_some(),"Absence does not certify a directory deletion");
    assert_eq!(f.cache.feed_request("chat",None,None).unwrap().cursor,before.cursor);
    let mut reader=peer.read(BlockWatch::Feed(before),Priority::Foreground).await.unwrap();
    let update=reader.next().await.unwrap().unwrap();
    f.cache.apply(&f.lineage,update,f.cache.epoch()).await.unwrap();
    assert!(f.cache.cached_header("chat","queued:consumed").unwrap().is_none(),"The ordered directory tombstone owns deletion");
    client.shutdown().await;server.shutdown().await;
}
