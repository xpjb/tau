use super::*;
use futures_util::FutureExt;
use rand::RngCore;
use rusqlite::Connection as Sqlite;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Memory {
    db: Arc<Mutex<Sqlite>>,
    notify: watch::Sender<u64>,
    reads: AtomicUsize,
}
impl Memory {
    fn new() -> Arc<Self> {
        let db = Sqlite::open_in_memory().unwrap(); db.execute_batch("PRAGMA foreign_keys=ON").unwrap();
        tau_blocks::initialize(&db).unwrap();
        Arc::new(Self { db:Arc::new(Mutex::new(db)),notify:watch::channel(0).0,reads:AtomicUsize::new(0) })
    }
    fn put(&self, id: &str, parent: Option<&str>, kind: BlockKind, bytes: &[u8], sealed: bool) {
        let mut db = self.db.lock().unwrap(); let tx = db.transaction().unwrap();
        let header = BlockHeader { id:id.into(),parent:parent.map(str::to_owned),kind,order:1,meta:serde_json::json!({"label":id}),
            version:0,length:0,sealed,revision:0 };
        tau_blocks::put(&tx,"chat",header,bytes).unwrap();
        let revision = tau_blocks::cursor(&tx).unwrap().sequence; tx.commit().unwrap();
        self.notify.send_replace(revision);
    }
    fn lineage(&self) -> String { tau_blocks::cursor(&self.db.lock().unwrap()).unwrap().lineage }
}
impl Backend for Memory {
    fn feed(&self, request: FeedRequest) -> BoxFuture<'static,Result<FeedPage>> {
        let db = self.db.clone(); async move { tau_blocks::feed(&db.lock().unwrap(),&request) }.boxed()
    }
    fn read(&self, request: BlockRequest) -> BoxFuture<'static,Result<ContentRange>> {
        self.reads.fetch_add(1,Ordering::SeqCst);
        let db = self.db.clone(); async move { tau_blocks::read(&db.lock().unwrap(),&request) }.boxed()
    }
    fn changes(&self) -> watch::Receiver<u64> { self.notify.subscribe() }
}
async fn fixture() -> (Arc<Memory>,Server,Client) {
    let backend = Memory::new();
    let server = Server::bind("127.0.0.1:0".parse().unwrap(),backend.clone()).await.unwrap();
    let client = Client::bind().await.unwrap();
    let offer = server.authorize(&client.node_id(),backend.lineage()).unwrap();
    client.configure(&offer,"127.0.0.1").await.unwrap();
    (backend,server,client)
}
fn feed_request(parent: Option<&str>) -> BlockWatch {
    BlockWatch::Feed(FeedRequest { scope:"chat".into(),parent:parent.map(str::to_owned),cursor:None,floor:0,before:None })
}
fn block_request(id: &str, version: u64, offset: u64, follow: bool) -> BlockWatch {
    BlockWatch::Block(BlockRequest { scope:"chat".into(),id:id.into(),version,offset,follow })
}
async fn frame(watch: &mut Watcher) -> Frame {
    let (frame,n) = tokio::time::timeout(Duration::from_secs(4),watch.next()).await.unwrap().unwrap();
    if !matches!(frame.header,Header::End | Header::Error { .. }) { let _ = watch.consumed(n).await; }
    frame
}
async fn collect(mut watch: Watcher) -> (Vec<u8>,usize) {
    let mut bytes = vec![]; let mut frames = 0;
    loop {
        let frame = frame(&mut watch).await;
        match frame.header {
            Header::Data { .. } => { bytes.extend(frame.decoded().unwrap()); frames += 1; }
            Header::End => return (bytes,frames),
            Header::Error { message } => panic!("{message}"),
            _ => {}
        }
    }
}

#[tokio::test]
async fn requested_feed_never_opens_unrequested_tool_content() {
    let (backend,server,client) = fixture().await;
    backend.put("tool",None,BlockKind::Tool,b"",true);
    backend.put("code",Some("tool"),BlockKind::Code,&vec![b'x';1024*1024],true);
    let mut watch = client.watch(feed_request(None)).await.unwrap();
    match frame(&mut watch).await.header {
        Header::Record { record:BlockRecord::Put { block }, .. } => assert_eq!(block.id,"tool"),
        other => panic!("{other:?}"),
    }
    assert!(matches!(frame(&mut watch).await.header,Header::Page { .. }));
    assert_eq!(backend.reads.load(Ordering::SeqCst),0);
    let mut children = client.watch(feed_request(Some("tool"))).await.unwrap();
    assert!(matches!(frame(&mut children).await.header,Header::Record { .. }));
    assert_eq!(backend.reads.load(Ordering::SeqCst),0,"Requesting child headers still is not requesting their content");
    let (bytes,_) = collect(client.watch(block_request("code",0,0,false)).await.unwrap()).await;
    assert_eq!(bytes.len(),1024*1024);
    client.shutdown().await; server.shutdown().await;
}

#[tokio::test]
async fn live_block_appends_resume_and_seal_without_replaying_prefix() {
    let (backend,server,client) = fixture().await;
    backend.put("code",None,BlockKind::Code,b"a",false);
    let mut watch = client.watch(block_request("code",0,0,true)).await.unwrap();
    assert!(matches!(frame(&mut watch).await.header,Header::Block { .. }));
    assert_eq!(frame(&mut watch).await.decoded().unwrap(),b"a");
    backend.put("code",None,BlockKind::Code,b"abc",false);
    assert!(matches!(frame(&mut watch).await.header,Header::Block { .. }));
    let next = frame(&mut watch).await;
    assert!(matches!(next.header,Header::Data {offset:1,..})); assert_eq!(next.decoded().unwrap(),b"bc");
    drop(watch);
    backend.put("code",None,BlockKind::Code,b"abcdef",true);
    let (bytes,_) = collect(client.watch(block_request("code",1,3,false)).await.unwrap()).await;
    assert_eq!(bytes,b"def");
    let (bytes,frames) = collect(client.watch(block_request("code",1,6,false)).await.unwrap()).await;
    assert!(bytes.is_empty()); assert_eq!(frames,0);
    client.shutdown().await; server.shutdown().await;
}

#[tokio::test]
async fn no_credit_bounds_a_slow_stream_without_blocking_another_feed() {
    let (backend,server,client) = fixture().await;
    let mut bytes = vec![0;2*1024*1024]; rand::thread_rng().fill_bytes(&mut bytes);
    backend.put("large",None,BlockKind::File,&bytes,true);
    let stalled = client.watch(block_request("large",0,0,false)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut watch = client.watch(feed_request(None)).await.unwrap();
    assert!(matches!(frame(&mut watch).await.header,Header::Record { .. }));
    assert!(matches!(frame(&mut watch).await.header,Header::Page { .. }));
    assert!(backend.reads.load(Ordering::SeqCst) <= 6,"Credit, not an unbounded outbound FIFO, limits read-ahead");
    drop(stalled);
    client.shutdown().await; server.shutdown().await;
}

#[tokio::test]
async fn one_peer_can_request_multiple_blocks_and_renew_grant_without_replacing_streams() {
    let (backend,server,client) = fixture().await;
    backend.put("a",None,BlockKind::Text,b"first",true);
    backend.put("b",None,BlockKind::Text,b"second",true);
    let mut feed = client.watch(feed_request(None)).await.unwrap();
    assert!(matches!(frame(&mut feed).await.header,Header::Record { .. }));
    let first_connection = client.connection().await.unwrap().stable_id();
    let (a,_) = collect(client.watch(block_request("a",0,0,false)).await.unwrap()).await;
    let offer = server.authorize(&client.node_id(),backend.lineage()).unwrap();
    client.configure(&offer,"127.0.0.1").await.unwrap();
    let (b,_) = collect(client.watch(block_request("b",0,0,false)).await.unwrap()).await;
    assert_eq!(a,b"first"); assert_eq!(b,b"second");
    assert_eq!(client.connection().await.unwrap().stable_id(),first_connection);
    assert!(matches!(frame(&mut feed).await.header,Header::Record { .. }));
    client.shutdown().await; server.shutdown().await;
}

#[tokio::test]
async fn ungranted_endpoint_cannot_read_blocks() {
    let (backend,server,client) = fixture().await;
    backend.put("private",None,BlockKind::Text,b"private",true);
    server.revoke(&client.node_id());
    let result = client.watch(block_request("private",0,0,false)).await;
    if let Ok(mut watcher) = result { assert!(tokio::time::timeout(Duration::from_secs(4),watcher.next()).await.unwrap().is_err()); }
    assert_eq!(backend.reads.load(Ordering::SeqCst),0);
    client.shutdown().await; server.shutdown().await;
}

#[test]
fn compression_is_chunk_local_bounded_and_verified_after_decompression() {
    let header = BlockHeader { id:"a".into(),parent:None,order:0,kind:BlockKind::Code,meta:serde_json::json!({}),version:1,length:BLOCK_CHUNK_BYTES as u64,sealed:true,revision:1 };
    let bytes = vec![b'x';BLOCK_CHUNK_BYTES];
    let range = ContentRange { header,offset:0,hash:blake3::hash(&bytes).to_hex().to_string(),bytes:bytes.clone() };
    let mut frame = Frame::content(&range).unwrap();
    assert!(matches!(frame.header,Header::Data {codec:Codec::Zstd,..}));
    assert!(frame.data.len() < 100); assert_eq!(frame.decoded().unwrap(),bytes);
    if let Header::Data { length,.. } = &mut frame.header { *length = (BLOCK_CHUNK_BYTES+1) as u32; }
    assert!(frame.decoded().is_err());
    if let Header::Data { length,hash,.. } = &mut frame.header { *length = BLOCK_CHUNK_BYTES as u32; *hash = "bad".into(); }
    assert!(frame.decoded().is_err());
}

#[tokio::test]
async fn batched_feeds_share_a_stream_without_fetching_children_or_bodies() {
    let (backend,server,client)=fixture().await;
    for n in 0..12 {let parent=format!("tool-{n}");backend.put(&parent,None,BlockKind::Tool,b"",true);backend.put(&format!("input-{n}"),Some(&parent),BlockKind::Code,b"private arguments",true);}
    let requests=(0..12).map(|n|FeedRequest {scope:"chat".into(),parent:Some(format!("tool-{n}")),cursor:None,floor:0,before:None}).collect();
    let mut watch=client.watch(BlockWatch::Feeds {requests}).await.unwrap();
    for n in 0..12 {
        assert!(matches!(frame(&mut watch).await.header,Header::Record {watch,record:BlockRecord::Put {block}} if watch==n && block.id==format!("input-{n}")));
        assert!(matches!(frame(&mut watch).await.header,Header::Page {watch,..} if watch==n));
    }
    assert_eq!(backend.reads.load(Ordering::SeqCst),0);
    let (bytes,_)=collect(client.watch(block_request("input-5",0,0,false)).await.unwrap()).await;
    assert_eq!(bytes,b"private arguments");
    client.shutdown().await;server.shutdown().await;
}

#[tokio::test]
async fn stalled_bulk_admission_reserves_capacity_for_foreground_and_metadata() {
    let (backend,server,client)=fixture().await;
    let mut bytes=vec![0;256*1024];rand::thread_rng().fill_bytes(&mut bytes);
    backend.put("file",None,BlockKind::File,&bytes,true);backend.put("answer",None,BlockKind::Text,b"responsive",true);
    let mut stalled=vec![];
    for _ in 0..6 {stalled.push(client.watch_bulk(block_request("file",0,0,false)).await.unwrap());}
    assert!(tokio::time::timeout(Duration::from_millis(50),client.watch_bulk(block_request("file",0,0,false))).await.is_err());
    let mut feed=client.watch(feed_request(None)).await.unwrap();assert!(matches!(frame(&mut feed).await.header,Header::Record {..}));
    let (bytes,_)=tokio::time::timeout(Duration::from_secs(2),collect(client.watch(block_request("answer",0,0,false)).await.unwrap())).await.unwrap();
    assert_eq!(bytes,b"responsive");drop(stalled);
    client.shutdown().await;server.shutdown().await;
}

#[tokio::test]
async fn metadata_fanout_cannot_consume_foreground_or_descriptor_slots() {
    let (backend,server,client)=fixture().await;backend.put("answer",None,BlockKind::Text,b"reserved",true);
    let _first=client.watch(feed_request(None)).await.unwrap();let _second=client.watch(feed_request(None)).await.unwrap();
    assert!(tokio::time::timeout(Duration::from_millis(30),client.watch(feed_request(None))).await.is_err());
    assert_eq!(collect(client.watch(block_request("answer",0,0,false)).await.unwrap()).await.0,b"reserved");
    assert_eq!(collect(client.watch_descriptor(block_request("answer",0,0,false)).await.unwrap()).await.0,b"reserved");
    client.shutdown().await;server.shutdown().await;
}

#[tokio::test]
async fn native_ipv6_direct_route_and_many_peer_live_fanout() {
    let backend=Memory::new();backend.put("live",None,BlockKind::Text,b"prefix",false);
    let server=Server::bind("127.0.0.1:0".parse().unwrap(),backend.clone()).await.unwrap();
    let mut peers=Vec::new();
    for n in 0..16 {
        let client=Client::bind().await.unwrap();let offer=server.authorize(&client.node_id(),backend.lineage()).unwrap();assert!(offer.port_v6.is_some());
        client.configure(&offer,if n%2==0 {"[::1]"} else {"127.0.0.1"}).await.unwrap();
        let mut watcher=client.watch(block_request("live",0,0,true)).await.unwrap();assert!(matches!(frame(&mut watcher).await.header,Header::Block {..}));assert_eq!(frame(&mut watcher).await.decoded().unwrap(),b"prefix");
        peers.push((client,watcher));
    }
    let bytes=[b"prefix".as_slice(),&vec![9;128*1024]].concat();backend.put("live",None,BlockKind::Text,&bytes,true);
    let results=futures_util::future::join_all(peers.into_iter().map(|(client,watcher)|async move {
        let (tail,_)=collect(watcher).await;assert_eq!(tail,vec![9;128*1024]);let stats=client.stats();assert_eq!(stats.connections,1);assert_eq!(stats.integrity_failures,0);assert_eq!(stats.content_rx_bytes,128*1024+6);
    })).await;
    assert_eq!(results.len(),16);
}
