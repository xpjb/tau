//! Exercise the production watch consumer and QUIC server, not a timer model.
use super::*;
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
    fn read(&self, request: BlockRequest) -> futures_util::future::BoxFuture<'static, Result<ContentRange>> {
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
    async fn connect(&self, slow_feed: bool) -> (Server, Client, watch::Sender<u64>) {
        let (changes, changed) = watch::channel(0);
        let server = Server::bind("127.0.0.1:0".parse().unwrap(), Arc::new(SlowSource {
            db: self.source.clone(), changed, slow_feed, feed_calls: AtomicUsize::new(0), reads: self.reads.clone(),
        })).await.unwrap();
        let client = Client::bind().await.unwrap();
        client.configure(&server.authorize(&client.node_id(), self.lineage.clone()).unwrap(), "127.0.0.1").await.unwrap();
        (server, client, changes)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn checkpoint_scheduling_commits_a_slow_metadata_round_instead_of_restarting_it() {
    let f = Fixture::new();
    f.put("parent", None, b"");
    f.put("child", Some("parent"), b"not requested");
    let (server, client, _changes) = f.connect(true).await;
    let (notices, _received) = mpsc::channel(32);
    let wake: crate::transport::Wake = Arc::new(|| {});
    let result = tokio::time::timeout(Duration::from_secs(10), watch_once(
        &Key::Feeds("chat".into(), vec![None, Some("parent".into())], false),
        &client, &f.cache, &f.lineage, &notices, &wake,
    )).await.expect("A slow but completing read must not monopolize its stream").unwrap();
    assert!(!result, "A live feed yields; it does not become permanently complete");
    for parent in [None, Some("parent")] {
        assert!(tau_block_store::cached_feed(&f.cache.db.lock().unwrap(), "chat", parent).unwrap().is_some(),
            "The entire bounded metadata round must commit before yielding, including later feeds");
    }
    assert!(f.reads.lock().unwrap().is_empty(), "Metadata scheduling must not fetch content");
    assert_eq!(client.stats().cancelled_streams, 0, "A cooperative yield is not a cancelled transfer");
    client.shutdown().await;
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn checkpoint_scheduling_resumes_after_a_slow_chunk_without_replaying_its_prefix() {
    let f = Fixture::new();
    let bytes = vec![b'x'; BLOCK_CHUNK_BYTES * 2 + 17];
    f.put("body", None, &bytes);
    let (server, client, _changes) = f.connect(false).await;
    let (notices, _received) = mpsc::channel(32);
    let wake: crate::transport::Wake = Arc::new(|| {});
    let key = Key::Block("chat".into(), "body".into(), true);
    assert!(!tokio::time::timeout(Duration::from_secs(10), watch_once(
        &key, &client, &f.cache, &f.lineage, &notices, &wake,
    )).await.unwrap().unwrap());
    let request = f.cache.block_request("chat", "body").unwrap();
    assert_eq!(request.offset, BLOCK_CHUNK_BYTES as u64,
        "The scheduling quantum may expire, but the in-flight chunk must commit before yielding");
    assert!(tokio::time::timeout(Duration::from_secs(3), watch_once(
        &key, &client, &f.cache, &f.lineage, &notices, &wake,
    )).await.unwrap().unwrap());
    assert_eq!(tau_block_store::cached_content(&f.cache.db.lock().unwrap(), "chat", "body").unwrap(), bytes);
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
    let (server,client,_changes)=f.connect(true).await; // Only the unused feed path is slow.
    let (notices,_received)=mpsc::channel(32);let wake:crate::transport::Wake=Arc::new(||{});
    let key=Key::BackgroundBlock("chat".into(),"live".into());
    assert!(tokio::time::timeout(Duration::from_secs(2),watch_once(&key,&client,&f.cache,&f.lineage,&notices,&wake)).await.unwrap().unwrap(),
        "a background catch-up must End, not wait five seconds for more live bytes");
    assert_eq!(client.stats().bulk_slots,0);
    {
        let mut db=f.source.lock().unwrap();let tx=db.transaction().unwrap();
        let h=tau_block_store::header(&tx,"chat","live").unwrap().unwrap();
        tau_block_store::append(&tx,"chat","live",h.version,h.length,b" suffix",false).unwrap();tx.commit().unwrap();
    }
    assert!(tokio::time::timeout(Duration::from_secs(2),watch_once(&key,&client,&f.cache,&f.lineage,&notices,&wake)).await.unwrap().unwrap());
    assert_eq!(tau_block_store::cached_content(&f.cache.db.lock().unwrap(),"chat","live").unwrap(),b"prefix suffix");
    let reads=f.reads.lock().unwrap().clone();assert_eq!(reads.len(),2);assert_eq!(reads[1].offset,6);
    assert_eq!(client.stats().bulk_slots,0);client.shutdown().await;server.shutdown().await;
}
