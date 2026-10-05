//! Authenticated native streaming, scheduling, recovery and durable uploads.
use anyhow::Result;
use futures_util::future::BoxFuture;
use std::{sync::{Arc, Mutex}, time::{Duration, Instant}};
use tau_net::{blocks::*, native::*};
use tokio::sync::watch;
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
        tau_block_store::initialize(&db).unwrap();
        Arc::new(Self { db:Arc::new(Mutex::new(db)),notify:watch::channel(0).0,reads:AtomicUsize::new(0) })
    }
    fn put(&self, id: &str, parent: Option<&str>, kind: BlockKind, bytes: &[u8], sealed: bool) {
        let mut db = self.db.lock().unwrap(); let tx = db.transaction().unwrap();
        let header = BlockHeader { id:id.into(),parent:parent.map(str::to_owned),kind,order:1,meta:serde_json::json!({"label":id}),
            version:0,length:0,sealed,revision:0 };
        tau_block_store::put(&tx,"chat",header,bytes).unwrap();
        let revision = tau_block_store::cursor(&tx).unwrap().sequence; tx.commit().unwrap();
        self.notify.send_replace(revision);
    }
    fn lineage(&self) -> String { tau_block_store::cursor(&self.db.lock().unwrap()).unwrap().lineage }
}
impl Backend for Memory {
    fn feed(&self, request: FeedRequest) -> BoxFuture<'static,Result<FeedPage>> {
        let db = self.db.clone(); async move { tau_block_store::feed(&db.lock().unwrap(),&request) }.boxed()
    }
    fn read(&self, request: BlockRequest) -> BoxFuture<'static,Result<Option<ContentRange>>> {
        self.reads.fetch_add(1,Ordering::SeqCst);
        let db = self.db.clone(); async move { tau_block_store::read(&db.lock().unwrap(),&request) }.boxed()
    }
    fn files(&self, request: tau_net::files::FileRequest) -> BoxFuture<'static,Result<tau_net::files::FileReply>> {
        self.reads.fetch_add(1,Ordering::SeqCst);
        async move {
            if request.path.as_deref()==Some("/wait") {std::future::pending::<()>().await;}
            let text="credit-window café 🦀\n".repeat(12_000);
            Ok(tau_net::files::FileReply::Text {path:"/fixture.rs".into(),revision:blake3::hash(text.as_bytes()).to_hex().to_string(),text})
        }.boxed()
    }
    fn changes(&self) -> watch::Receiver<u64> { self.notify.subscribe() }
}
async fn fixture() -> (Arc<Memory>,Server,Client,Peer) {
    let backend = Memory::new();
    let server = Server::bind("127.0.0.1:0".parse().unwrap(),backend.clone()).await.unwrap();
    let client = Client::bind().await.unwrap();
    let offer = server.authorize(&client.node_id(),backend.lineage()).unwrap();
    let peer=client.configure(&offer,"127.0.0.1").await.unwrap();
    (backend,server,client,peer)
}
fn feed_request(parent: Option<&str>) -> BlockWatch {
    BlockWatch::Feed(FeedRequest { scope:"chat".into(),parent:parent.map(str::to_owned),cursor:None,floor:0,before:None })
}
fn block_request(id: &str, version: u64, offset: u64, follow: bool) -> BlockWatch {
    BlockWatch::Block(BlockRequest { scope:"chat".into(),id:id.into(),version,offset,follow })
}
async fn update(reader:&mut Reader)->Update {
    tokio::time::timeout(Duration::from_secs(4),reader.next()).await.unwrap().unwrap().unwrap()
}
async fn collect(mut reader:Reader)->(Vec<u8>,usize) {
    let mut bytes=vec![];let mut ranges=0;
    while let Some(update)=tokio::time::timeout(Duration::from_secs(4),reader.next()).await.unwrap().unwrap() {
        match update {
            Update::Range {range,..}=>{bytes.extend(range.bytes);ranges+=1;}
            Update::Block {..}=>{},other=>panic!("Unexpected body update {other:?}"),
        }
    }(bytes,ranges)
}

#[tokio::test]
async fn connection_loss_is_status_without_hiding_content_or_storage_errors() {
    let (backend,server,client,peer)=fixture().await;
    let offer=server.authorize(&client.node_id(),backend.lineage()).unwrap();
    let mut watch=peer.read(feed_request(None),Priority::Foreground).await.unwrap();
    assert!(matches!(update(&mut watch).await,Update::Page {..}));
    server.shutdown().await;
    let lost=tokio::time::timeout(Duration::from_secs(3),watch.next()).await.unwrap().unwrap_err();
    assert!(is_connection_error(&lost.context("Content sync")),"Real connection loss stays typed through context");
    // NUL cannot be passed to the resolver; no external DNS request is made.
    let address=client.configure(&offer,"\0").await.unwrap_err();
    assert!(is_connection_error(&address));
    for kind in [std::io::ErrorKind::PermissionDenied,std::io::ErrorKind::Other,std::io::ErrorKind::InvalidData] {
        assert!(!is_connection_error(&anyhow::Error::from(std::io::Error::from(kind)).context("Local content IO")));
    }
    assert!(!is_connection_error(&anyhow::anyhow!("Unknown block")));
    client.shutdown().await;
}

#[tokio::test]
async fn lost_initial_discovery_keeps_one_attempt_then_negotiates_short_idle() {
    let backend=Memory::new();backend.put("first",None,BlockKind::Text,b"first",true);
    let server=Server::bind("127.0.0.1:0".parse().unwrap(),backend.clone()).await.unwrap();
    let client=Client::bind().await.unwrap();
    let mut offer=server.authorize(&client.node_id(),backend.lineage()).unwrap();
    let peer=std::net::SocketAddr::from(([127,0,0,1],offer.port));
    let socket=tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    offer.port=socket.local_addr().unwrap().port();
    let blackhole=Arc::new(std::sync::atomic::AtomicBool::new(false));
    let dropped=Arc::new(AtomicUsize::new(0));
    let proxy=tokio::spawn({let blackhole=blackhole.clone();let dropped=dropped.clone();async move {
        let mut client=None;let mut buf=[0;65536];
        loop {
            let (n,from)=socket.recv_from(&mut buf).await.unwrap();
            let target=if from==peer {let Some(client)=client else {continue;};client} else {
                client=Some(from);
                // Seed 29 lost the initial discovery ping and first QUIC packet.
                if dropped.load(Ordering::Relaxed)<2 {dropped.fetch_add(1,Ordering::Relaxed);continue;}
                peer
            };
            if !blackhole.load(Ordering::Relaxed) {socket.send_to(&buf[..n],target).await.unwrap();}
        }
    }});
    let peer=client.configure(&offer,"127.0.0.1").await.unwrap();
    let started=Instant::now();
    let watch=peer.read(block_request("first",0,0,false),Priority::Foreground).await.unwrap();
    assert_eq!(collect(watch).await.0,b"first");
    let connected=started.elapsed();
    assert_eq!(dropped.load(Ordering::Relaxed),2);
    assert_eq!(client.stats().connection_attempts,1,"Discovery loss must not fail the application connect attempt");
    assert_eq!(client.stats().connections,1);
    let mut probe=peer.read(feed_request(None),Priority::Foreground).await.unwrap();
    while !matches!(update(&mut probe).await,Update::Page {..}) {}
    blackhole.store(true,Ordering::Relaxed);let silent=Instant::now();
    let error=tokio::time::timeout(Duration::from_secs(7),probe.next()).await
        .expect("Established peers must retain the negotiated five-second silence limit").unwrap_err();
    assert!(is_connection_error(&error));
    eprintln!("native-discovery-loss: connected={connected:?}, established peer timeout={:?}",silent.elapsed());
    blackhole.store(false,Ordering::Relaxed);
    client.shutdown().await;server.shutdown().await;proxy.abort();let _=proxy.await;
}

#[tokio::test]
async fn healthy_idle_connection_is_preserved_across_native_peer_timeout_windows() {
    let (backend,server,client,peer)=fixture().await;
    backend.put("first",None,BlockKind::Text,b"first",true);
    assert_eq!(collect(peer.read(block_request("first",0,0,false),Priority::Foreground).await.unwrap()).await.0,b"first");
    let connections=client.stats().connections;
    assert_eq!(client.stats().active_streams,0);
    // More than two five-second peer windows, with no application streams.
    // Transport keep-alive ACKs, not periodic re-downloads, preserve the peer.
    tokio::time::sleep(Duration::from_secs(12)).await;
    backend.put("second",None,BlockKind::Text,b"second",true);
    assert_eq!(collect(peer.read(block_request("second",0,0,false),Priority::Foreground).await.unwrap()).await.0,b"second");
    assert_eq!(client.stats().connections,connections);
    assert_eq!(client.stats().connections,1,"Healthy idle peers cannot be recycled by the recovery policy");
    client.shutdown().await;server.shutdown().await;
}

#[tokio::test]
async fn requested_feed_never_opens_unrequested_tool_content() {
    let (backend,server,client,peer) = fixture().await;
    backend.put("tool",None,BlockKind::Tool,b"",true);
    backend.put("code",Some("tool"),BlockKind::Code,&vec![b'x';1024*1024],true);
    let mut watch = peer.read(feed_request(None),Priority::Foreground).await.unwrap();
    let Update::Page {page,..}=update(&mut watch).await else {panic!()};
    assert!(matches!(&page.records[..],[BlockRecord::Put {block}] if block.id=="tool"));
    assert_eq!(backend.reads.load(Ordering::SeqCst),0);
    let mut children = peer.read(feed_request(Some("tool")),Priority::Foreground).await.unwrap();
    assert!(matches!(update(&mut children).await,Update::Page {page,..} if !page.records.is_empty()));
    assert_eq!(backend.reads.load(Ordering::SeqCst),0,"Requesting child headers still is not requesting their content");
    let (bytes,_) = collect(peer.read(block_request("code",0,0,false),Priority::Foreground).await.unwrap()).await;
    assert_eq!(bytes.len(),1024*1024);
    client.shutdown().await; server.shutdown().await;
}

#[tokio::test]
async fn live_block_appends_resume_and_seal_without_replaying_prefix() {
    let (backend,server,client,peer) = fixture().await;
    backend.put("code",None,BlockKind::Code,b"a",false);
    let mut watch = peer.read(block_request("code",0,0,true),Priority::Foreground).await.unwrap();
    assert!(matches!(update(&mut watch).await,Update::Block { .. }));
    assert!(matches!(update(&mut watch).await,Update::Range {range,..} if range.bytes==b"a"));
    backend.put("code",None,BlockKind::Code,b"abc",false);
    assert!(matches!(update(&mut watch).await,Update::Block { .. }));
    assert!(matches!(update(&mut watch).await,Update::Range {range,..} if range.offset==1 && range.bytes==b"bc"));
    drop(watch);
    backend.put("code",None,BlockKind::Code,b"abcdef",true);
    let (bytes,_) = collect(peer.read(block_request("code",1,3,false),Priority::Foreground).await.unwrap()).await;
    assert_eq!(bytes,b"def");
    let (bytes,frames) = collect(peer.read(block_request("code",1,6,false),Priority::Foreground).await.unwrap()).await;
    assert!(bytes.is_empty()); assert_eq!(frames,0);
    client.shutdown().await; server.shutdown().await;
}

#[tokio::test]
async fn no_credit_bounds_a_slow_stream_without_blocking_another_feed() {
    let (backend,server,client,peer) = fixture().await;
    let mut bytes = vec![0;2*1024*1024]; rand::thread_rng().fill_bytes(&mut bytes);
    backend.put("large",None,BlockKind::File,&bytes,true);
    let stalled = peer.read(block_request("large",0,0,false),Priority::Foreground).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut watch = peer.read(feed_request(None),Priority::Foreground).await.unwrap();
    assert!(matches!(update(&mut watch).await,Update::Page {page,..} if !page.records.is_empty()));
    assert!(backend.reads.load(Ordering::SeqCst) <= 6,"Credit, not an unbounded outbound FIFO, limits read-ahead");
    drop(stalled);
    client.shutdown().await; server.shutdown().await;
}

#[tokio::test]
async fn one_peer_can_request_multiple_blocks_and_renew_grant_without_replacing_streams() {
    let (backend,server,client,peer) = fixture().await;
    backend.put("a",None,BlockKind::Text,b"first",true);
    backend.put("b",None,BlockKind::Text,b"second",true);
    let mut feed = peer.read(feed_request(None),Priority::Foreground).await.unwrap();
    assert!(matches!(update(&mut feed).await,Update::Page {page,..} if !page.records.is_empty()));
    let connections = client.stats().connections;
    let (a,_) = collect(peer.read(block_request("a",0,0,false),Priority::Foreground).await.unwrap()).await;
    let offer = server.authorize(&client.node_id(),backend.lineage()).unwrap();
    assert_eq!(client.configure(&offer,"127.0.0.1").await.unwrap(),peer);
    let (b,_) = collect(peer.read(block_request("b",0,0,false),Priority::Foreground).await.unwrap()).await;
    assert_eq!(a,b"first"); assert_eq!(b,b"second");
    assert_eq!(client.stats().connections,connections);
    backend.put("after-renewal",None,BlockKind::Text,b"new",true);
    assert!(matches!(update(&mut feed).await,Update::Page {page,..} if page.records.iter().any(|record|matches!(record,BlockRecord::Put {block} if block.id=="after-renewal"))));
    client.shutdown().await; server.shutdown().await;
}

#[tokio::test]
async fn ungranted_endpoint_cannot_read_blocks() {
    let (backend,server,client,peer) = fixture().await;
    backend.put("private",None,BlockKind::Text,b"private",true);
    server.revoke(&client.node_id());
    let result = peer.read(block_request("private",0,0,false),Priority::Foreground).await;
    if let Ok(mut watcher) = result { assert!(tokio::time::timeout(Duration::from_secs(4),watcher.next()).await.unwrap().is_err()); }
    assert_eq!(backend.reads.load(Ordering::SeqCst),0);
    client.shutdown().await; server.shutdown().await;
}

#[tokio::test]
async fn batched_feeds_share_a_stream_without_fetching_children_or_bodies() {
    let (backend,server,client,peer)=fixture().await;
    for n in 0..12 {let parent=format!("tool-{n}");backend.put(&parent,None,BlockKind::Tool,b"",true);backend.put(&format!("input-{n}"),Some(&parent),BlockKind::Code,b"private arguments",true);}
    let requests=(0..12).map(|n|FeedRequest {scope:"chat".into(),parent:Some(format!("tool-{n}")),cursor:None,floor:0,before:None}).collect();
    let mut watch=peer.read(BlockWatch::Feeds {requests},Priority::Foreground).await.unwrap();
    for n in 0..12 {
        let Update::Page {request,page}=update(&mut watch).await else {panic!()};
        assert_eq!(request.parent,Some(format!("tool-{n}")));
        assert!(matches!(&page.records[..],[BlockRecord::Put {block}] if block.id==format!("input-{n}")));
    }
    assert_eq!(backend.reads.load(Ordering::SeqCst),0);
    let (bytes,_)=collect(peer.read(block_request("input-5",0,0,false),Priority::Foreground).await.unwrap()).await;
    assert_eq!(bytes,b"private arguments");
    client.shutdown().await;server.shutdown().await;
}

#[tokio::test]
async fn stalled_bulk_admission_reserves_capacity_for_foreground_and_metadata() {
    let (backend,server,client,peer)=fixture().await;
    let mut bytes=vec![0;256*1024];rand::thread_rng().fill_bytes(&mut bytes);
    backend.put("file",None,BlockKind::File,&bytes,true);backend.put("answer",None,BlockKind::Text,b"responsive",true);
    let mut stalled=vec![];
    for _ in 0..6 {stalled.push(peer.read(block_request("file",0,0,false),Priority::Background).await.unwrap());}
    assert!(tokio::time::timeout(Duration::from_millis(50),peer.read(block_request("file",0,0,false),Priority::Background)).await.is_err());
    let mut feed=peer.read(feed_request(None),Priority::Foreground).await.unwrap();assert!(matches!(update(&mut feed).await,Update::Page {page,..} if !page.records.is_empty()));
    let (bytes,_)=tokio::time::timeout(Duration::from_secs(2),collect(peer.read(block_request("answer",0,0,false),Priority::Foreground).await.unwrap())).await.unwrap();
    assert_eq!(bytes,b"responsive");drop(stalled);
    client.shutdown().await;server.shutdown().await;
}

#[tokio::test]
async fn metadata_fanout_cannot_consume_foreground_or_descriptor_slots() {
    let (backend,server,client,peer)=fixture().await;backend.put("answer",None,BlockKind::Text,b"reserved",true);
    let _first=peer.read(feed_request(None),Priority::Foreground).await.unwrap();let _second=peer.read(feed_request(None),Priority::Foreground).await.unwrap();
    assert!(tokio::time::timeout(Duration::from_millis(30),peer.read(feed_request(None),Priority::Foreground)).await.is_err());
    assert_eq!(collect(peer.read(block_request("answer",0,0,false),Priority::Foreground).await.unwrap()).await.0,b"reserved");
    assert_eq!(collect(peer.read(block_request("answer",0,0,false),Priority::Descriptor).await.unwrap()).await.0,b"reserved");
    client.shutdown().await;server.shutdown().await;
}

#[tokio::test]
async fn native_ipv6_direct_route_and_many_peer_live_fanout() {
    let backend=Memory::new();backend.put("live",None,BlockKind::Text,b"prefix",false);
    let server=Server::bind("127.0.0.1:0".parse().unwrap(),backend.clone()).await.unwrap();
    let mut peers=Vec::new();
    for n in 0..16 {
        let client=Client::bind().await.unwrap();let offer=server.authorize(&client.node_id(),backend.lineage()).unwrap();assert!(offer.port_v6.is_some());
        let peer=client.configure(&offer,if n%2==0 {"[::1]"} else {"127.0.0.1"}).await.unwrap();
        let mut watcher=peer.read(block_request("live",0,0,true),Priority::Foreground).await.unwrap();assert!(matches!(update(&mut watcher).await,Update::Block {..}));assert!(matches!(update(&mut watcher).await,Update::Range {range,..} if range.bytes==b"prefix"));
        peers.push((client,watcher));
    }
    let bytes=[b"prefix".as_slice(),&vec![9;128*1024]].concat();backend.put("live",None,BlockKind::Text,&bytes,true);
    let results=futures_util::future::join_all(peers.into_iter().map(|(client,watcher)|async move {
        let (tail,_)=collect(watcher).await;assert_eq!(tail,vec![9;128*1024]);let stats=client.stats();assert_eq!(stats.connections,1);assert_eq!(stats.integrity_failures,0);assert_eq!(stats.content_rx_bytes,128*1024+6);
    })).await;
    assert_eq!(results.len(),16);
}

#[tokio::test]
async fn scheduled_idle_watches_yield_capacity_to_waiters_without_reconnecting() {
    let (_backend,server,client,peer) = fixture().await;
    let mut first = peer.read(feed_request(None),Priority::Foreground).await.unwrap();
    let mut second = peer.read(feed_request(None),Priority::Foreground).await.unwrap();
    assert!(matches!(update(&mut first).await,Update::Page {..}));
    assert!(matches!(update(&mut second).await,Update::Page {..}));

    // Keep the waiting future alive across the timeout, so it retains its FIFO
    // position. A renewing interest must go behind this earlier waiter.
    let waiting = peer.read(feed_request(None),Priority::Foreground);
    tokio::pin!(waiting);
    assert!(tokio::time::timeout(Duration::from_millis(30),&mut waiting).await.is_err());
    let renewing=first.next();tokio::pin!(renewing);
    let mut next=tokio::time::timeout(Duration::from_secs(7),async {
        tokio::select! {
            result=&mut renewing=>panic!("Renewal overtook an earlier waiter: {result:?}"),
            result=&mut waiting=>result.unwrap(),
        }
    }).await.unwrap();
    assert!(matches!(update(&mut next).await,Update::Page {..}));
    assert!(tokio::time::timeout(Duration::from_millis(30),&mut renewing).await.is_err(),
        "The earlier waiter and second watch own both metadata slots");
    let stats = client.stats();
    assert_eq!(stats.connections,1);
    assert_eq!(stats.cancelled_streams,0,"Yield is successful scheduling, not failed/cancelled IO");
    drop(next);
    assert!(matches!(tokio::time::timeout(Duration::from_secs(2),&mut renewing).await.unwrap().unwrap(),Some(Update::Page {..})));
    drop(second);
    client.shutdown().await; server.shutdown().await;
}

#[tokio::test]
async fn scheduled_finite_history_and_sealed_content_end_instead_of_rescheduling() {
    let (backend,server,client,peer) = fixture().await;
    backend.put("answer",None,BlockKind::Text,b"complete",true);
    let mut history = feed_request(None);
    if let BlockWatch::Feed(request) = &mut history {
        request.before = Some(FeedPosition {order:2,id:String::new()});
    }
    let mut history = peer.read(history,Priority::Foreground).await.unwrap();
    assert!(matches!(update(&mut history).await,Update::Page {page,..} if !page.records.is_empty()));
    assert!(history.next().await.unwrap().is_none());
    assert_eq!(collect(peer.read(block_request("answer",0,0,false),Priority::Background).await.unwrap()).await.0,b"complete");
    drop(history);
    assert_eq!(client.stats().active_streams,0);
    assert_eq!(client.stats().cancelled_streams,0);
    client.shutdown().await; server.shutdown().await;
}

#[tokio::test]
async fn background_metadata_cannot_occupy_selected_chat_stream_reservations() {
    let (backend,server,client,peer)=fixture().await;
    backend.put("answer",None,BlockKind::Text,b"foreground",true);
    let mut background=Vec::new();
    for _ in 0..6 { background.push(peer.read(feed_request(None),Priority::Background).await.unwrap()); }
    assert_eq!(client.stats().bulk_slots,6);
    assert_eq!(client.stats().metadata_slots,0);
    assert_eq!(client.stats().foreground_slots,0);
    assert!(tokio::time::timeout(Duration::from_millis(30),peer.read(feed_request(None),Priority::Background)).await.is_err());
    let mut metadata=tokio::time::timeout(Duration::from_secs(2),peer.read(feed_request(None),Priority::Foreground)).await.unwrap().unwrap();
    assert!(matches!(update(&mut metadata).await,Update::Page {page,..} if !page.records.is_empty()));
    let body=tokio::time::timeout(Duration::from_secs(2),peer.read(block_request("answer",0,0,false),Priority::Foreground)).await.unwrap().unwrap();
    assert_eq!(collect(body).await.0,b"foreground");
    drop(metadata);drop(background);client.shutdown().await;server.shutdown().await;
}

#[tokio::test]
async fn filesystem_streams_require_grants_share_connection_and_release_credit_slots_on_cancel() {
    use tau_net::files::*;
    let (backend,server,client,peer)=fixture().await;
    let offer=server.authorize(&client.node_id(),backend.lineage()).unwrap();
    let anonymous=Client::bind().await.unwrap();let unauthorized=anonymous.configure(&offer,"127.0.0.1").await.unwrap();
    let request=FileRequest {session_id:"chat".into(),path:Some("/fixture.rs".into()),operation:FileOperation::Open {revision:None}};
    assert!(tokio::time::timeout(Duration::from_secs(3),unauthorized.files(request.clone())).await.unwrap().is_err());
    assert_eq!(backend.reads.load(Ordering::SeqCst),0,"No filesystem call before native authorization");
    anonymous.shutdown().await;
    let FileReply::Text {text,..}=peer.files(request.clone()).await.unwrap() else {panic!()};
    assert_eq!(text,"credit-window café 🦀\n".repeat(12_000));
    assert!(client.stats().content_rx_bytes>BLOCK_WINDOW_BYTES as u64);
    assert_eq!(client.stats().active_streams,0);
    let mut waiting=request.clone();waiting.path=Some("/wait".into());
    assert!(tokio::time::timeout(Duration::from_millis(100),peer.files(waiting)).await.is_err());
    assert_eq!(client.stats().active_streams,0);assert_eq!(client.stats().foreground_slots,0);
    assert!(client.stats().cancelled_streams>0);
    assert!(matches!(peer.files(request.clone()).await.unwrap(),FileReply::Text {..}));
    assert_eq!(client.stats().connections,1,"Cancelled file reads do not replace the shared connection");
    let calls=backend.reads.load(Ordering::SeqCst);server.revoke(&client.node_id());
    assert!(peer.files(request).await.is_err());assert_eq!(backend.reads.load(Ordering::SeqCst),calls);
    client.shutdown().await;server.shutdown().await;
}

#[tokio::test]
async fn warming_file_names_uses_background_slots_and_does_not_block_file_previews() {
    use tau_net::files::*;
    let (_,server,client,peer)=fixture().await;
    let mut background=Box::pin(peer.files(FileRequest {session_id:"chat".into(),path:Some("/wait".into()),operation:FileOperation::Index {revision:None}}));
    tokio::select! {_ = &mut background => panic!("fixture must stall"), _ = tokio::time::sleep(Duration::from_millis(150)) => {}}
    assert_eq!(client.stats().bulk_slots,1);assert_eq!(client.stats().foreground_slots,0);
    let preview=peer.files(FileRequest {session_id:"chat".into(),path:Some("/fixture.rs".into()),operation:FileOperation::Open {revision:None}});
    assert!(matches!(tokio::time::timeout(Duration::from_secs(3),preview).await.unwrap().unwrap(),FileReply::Text {..}));
    drop(background);assert_eq!(client.stats().active_streams,0);assert_eq!(client.stats().bulk_slots,0);
    client.shutdown().await;server.shutdown().await;
}

mod uploads {
use std::sync::{Arc,Mutex};
use futures_util::{FutureExt,future::BoxFuture};
use rusqlite::Connection;
use tau_net::blocks::*;
use tau_block_store::*;
use tau_net::native::{Backend,Server,Client,Peer,Priority,Update};
use tokio::sync::watch;

struct Store {db:Arc<Mutex<Connection>>,changes:watch::Sender<u64>}
impl Store {
    fn open(path:&std::path::Path)->Arc<Self> {
        let db=Connection::open(path).unwrap();db.execute_batch("PRAGMA foreign_keys=ON;PRAGMA journal_mode=WAL;PRAGMA synchronous=FULL").unwrap();tau_block_store::initialize(&db).unwrap();
        Arc::new(Self {db:Arc::new(Mutex::new(db)),changes:watch::channel(0).0})
    }
    fn lineage(&self)->String {cursor(&self.db.lock().unwrap()).unwrap().lineage}
}
impl Backend for Store {
    fn feed(&self,r:FeedRequest)->BoxFuture<'static,anyhow::Result<FeedPage>> {let db=self.db.clone();async move {feed(&db.lock().unwrap(),&r)}.boxed()}
    fn read(&self,r:BlockRequest)->BoxFuture<'static,anyhow::Result<Option<ContentRange>>> {let db=self.db.clone();async move {read(&db.lock().unwrap(),&r)}.boxed()}
    fn changes(&self)->watch::Receiver<u64> {self.changes.subscribe()}
    fn upload_begin(&self,s:UploadSpec)->BoxFuture<'static,anyhow::Result<UploadStatus>> {let db=self.db.clone();async move {let mut db=db.lock().unwrap();let tx=db.transaction()?;let status=begin_upload(&tx,&s)?;tx.commit()?;Ok(status)}.boxed()}
    fn upload_write(&self,s:UploadSpec,offset:u64,bytes:Vec<u8>)->BoxFuture<'static,anyhow::Result<()>> {let db=self.db.clone();async move {let mut db=db.lock().unwrap();let tx=db.transaction()?;write_upload(&tx,&s,offset,&bytes)?;tx.commit()?;Ok(())}.boxed()}
    fn upload_finish(&self,s:UploadSpec)->BoxFuture<'static,anyhow::Result<UploadStatus>> {let db=self.db.clone();async move {let mut db=db.lock().unwrap();let tx=db.transaction()?;let bytes=cached_content(&tx,UPLOAD_SCOPE,&s.id)?;let status=seal_upload(&tx,&s,&blake3::hash(&bytes).to_hex(),None)?;tx.commit()?;Ok(status)}.boxed()}
}
async fn connect(store:Arc<Store>)->(Server,Client,Peer) {
    let server=Server::bind("127.0.0.1:0".parse().unwrap(),store.clone()).await.unwrap();
    let client=Client::bind().await.unwrap();
    let offer=server.authorize(&client.node_id(),store.lineage()).unwrap();let peer=client.configure(&offer,"127.0.0.1").await.unwrap();(server,client,peer)
}

#[tokio::test]
async fn interrupted_upload_resumes_durable_bytes_after_both_endpoints_restart() {
    let root=tempfile::tempdir().unwrap();let path=root.path().join("source.db");let store=Store::open(&path);
    let bytes=(0..900_000).map(|n|((n*31)%251) as u8).collect::<Vec<_>>();
    let spec=UploadSpec {id:"operation".into(),length:bytes.len() as u64,hash:blake3::hash(&bytes).to_hex().to_string(),purpose:UploadPurpose::Command};
    let (server,client,peer)=connect(store.clone()).await;
    let mut upload=peer.uploader(spec.clone()).await.unwrap();
    assert!(upload.copy_from(&mut std::io::Cursor::new(&bytes[..128*1024])).await.is_err(),"An interrupted local source must not publish a partial upload");
    // A second stream proves progress is committed, not just client queued.
    tokio::time::timeout(std::time::Duration::from_secs(5),async {
        loop {let status=upload_status(&store.db.lock().unwrap(),&spec).unwrap();if status.offset>=64*1024 {break;}tokio::task::yield_now().await;}
    }).await.unwrap();
    drop(upload);client.shutdown().await;server.shutdown().await;drop(client);drop(server);drop(store);
    let store=Store::open(&path);let saved=upload_status(&store.db.lock().unwrap(),&spec).unwrap().offset;
    assert!(saved>=64*1024 && saved<spec.length);
    let (server,client,peer)=connect(store.clone()).await;
    let mut upload=peer.uploader(spec.clone()).await.unwrap();assert_eq!(upload.status.offset,saved);
    upload.copy_from(&mut std::io::Cursor::new(&bytes)).await.unwrap();
    let status=upload.finish().await.unwrap();assert!(status.sealed);drop(upload);
    let retry=peer.uploader(spec.clone()).await.unwrap();assert!(retry.status.sealed);assert_eq!(retry.status.offset,spec.length);drop(retry);
    assert_eq!(cached_content(&store.db.lock().unwrap(),UPLOAD_SCOPE,&spec.id).unwrap(),bytes);
    let mut wrong=spec.clone();wrong.hash=blake3::hash(b"other").to_hex().to_string();assert!(peer.uploader(wrong).await.is_err());
    let mut feed=peer.read(BlockWatch::Feed(FeedRequest {scope:UPLOAD_SCOPE.into(),parent:None,cursor:None,floor:0,before:None}),Priority::Foreground).await.unwrap();
    assert!(matches!(feed.next().await.unwrap(),Some(Update::Page {page,..}) if !page.records.is_empty()));
    client.shutdown().await;server.shutdown().await;
}

#[tokio::test]
async fn upload_checks_authorization_hashes_size_gaps_and_unsealed_references() {
    let root=tempfile::tempdir().unwrap();let store=Store::open(&root.path().join("source.db"));
    let (server,client,peer)=connect(store.clone()).await;
    let spec=UploadSpec {id:"bound".into(),length:4,hash:blake3::hash(b"good").to_hex().to_string(),purpose:UploadPurpose::Command};
    let mut upload=peer.uploader(spec.clone()).await.unwrap();upload.copy_from(&mut std::io::Cursor::new(b"evil")).await.unwrap();assert!(upload.finish().await.is_err());drop(upload);
    assert!(!upload_status(&store.db.lock().unwrap(),&spec).unwrap().sealed);
    let reference=ContentRef {lineage:store.lineage(),scope:UPLOAD_SCOPE.into(),id:spec.id.clone(),length:4,hash:spec.hash.clone()};
    assert!(uploaded_input(&store.db.lock().unwrap(),&reference).is_err());
    let mut too_big=spec.clone();too_big.length=MAX_COMMAND_BYTES+1;assert!(peer.uploader(too_big).await.is_err());
    let outsider=Client::bind().await.unwrap();
    let offer=server.authorize(&client.node_id(),store.lineage()).unwrap();let unauthorized=outsider.configure(&offer,"127.0.0.1").await.unwrap();
    assert!(unauthorized.uploader(spec.clone()).await.is_err());
    server.revoke(&client.node_id());assert!(peer.uploader(spec).await.is_err());
    outsider.shutdown().await;client.shutdown().await;server.shutdown().await;
}

}


#[tokio::test]
async fn an_existing_read_cannot_renew_into_a_different_authenticated_source() {
    let (first,server,client,peer)=fixture().await;first.put("same-id",None,BlockKind::Text,b"original",false);
    let mut reader=peer.read(block_request("same-id",0,0,true),Priority::Foreground).await.unwrap();
    assert!(matches!(update(&mut reader).await,Update::Block {..}));
    assert!(matches!(update(&mut reader).await,Update::Range {range,..} if range.bytes==b"original"));
    // Leave a cooperative Yield queued, then change the client's configuration.
    tokio::time::sleep(Duration::from_secs(6)).await;
    let second=Memory::new();second.put("same-id",None,BlockKind::Text,b"not the requested source",true);
    let other=Server::bind("127.0.0.1:0".parse().unwrap(),second.clone()).await.unwrap();
    let replacement=client.configure(&other.authorize(&client.node_id(),second.lineage()).unwrap(),"127.0.0.1").await.unwrap();
    assert!(tokio::time::timeout(Duration::from_secs(3),reader.next()).await.unwrap().is_err());
    assert_eq!(second.reads.load(Ordering::SeqCst),0,"An existing interest is pinned to its original peer, including renewal");
    assert_eq!(collect(replacement.read(block_request("same-id",0,0,false),Priority::Foreground).await.unwrap()).await.0,b"not the requested source");
    client.shutdown().await;server.shutdown().await;other.shutdown().await;
}

#[tokio::test]
async fn retiring_a_peer_cancels_admission_without_waiting_for_busy_slots() {
    let (backend,server,client,peer)=fixture().await;
    backend.put("body",None,BlockKind::Text,b"held",true);
    let mut held=Vec::new();
    for _ in 0..4 {held.push(peer.read(block_request("body",0,0,false),Priority::Foreground).await.unwrap());}
    let waiting=peer.read(block_request("body",0,0,false),Priority::Foreground);tokio::pin!(waiting);
    assert!(tokio::time::timeout(Duration::from_millis(30),&mut waiting).await.is_err());
    let replacement=client.configure(&server.authorize(&client.node_id(),"replacement-source".into()).unwrap(),"127.0.0.1").await.unwrap();
    let error=tokio::time::timeout(Duration::from_secs(1),&mut waiting).await.unwrap().err().expect("A retired peer must release its queued admission");
    assert!(is_connection_error(&error));
    assert_eq!(peer.lineage(),backend.lineage());assert_eq!(replacement.lineage(),"replacement-source");
    assert_ne!(peer,replacement);
    assert_eq!(client.stats().foreground_slots,4,"The test must not depend on dropping the old readers first");
    assert!(peer.read(block_request("body",0,0,false),Priority::Foreground).await.is_err());
    drop(held);
    assert_eq!(collect(replacement.read(block_request("body",0,0,false),Priority::Foreground).await.unwrap()).await.0,b"held");
    client.shutdown().await;server.shutdown().await;
}
