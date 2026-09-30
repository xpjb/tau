//! Production controller/cache/native watch workers through a real UDP blackhole.
//! Control is a read-only protocol fixture: no provider, command replay or daemon
//! restart is needed to isolate native transcript recovery from WebSocket health.
use super::*;
use axum::{Router, extract::{State, WebSocketUpgrade}, response::IntoResponse, routing::get};
use futures_util::{future::BoxFuture, FutureExt};
use rusqlite::Connection;
use tau_blocks::*;
use tau_transfer::blocks::{Backend, Server};
use tokio::sync::watch;

struct Source {
    db: Arc<std::sync::Mutex<Connection>>,
    changed: watch::Sender<u64>,
    reads: Arc<std::sync::Mutex<Vec<BlockRequest>>>,
}
impl Backend for Source {
    fn feed(&self, request: FeedRequest) -> BoxFuture<'static, anyhow::Result<FeedPage>> {
        let db = self.db.clone(); async move { tau_blocks::feed(&db.lock().unwrap(), &request) }.boxed()
    }
    fn read(&self, request: BlockRequest) -> BoxFuture<'static, anyhow::Result<ContentRange>> {
        self.reads.lock().unwrap().push(request.clone());
        let db = self.db.clone(); async move { tau_blocks::read(&db.lock().unwrap(), &request) }.boxed()
    }
    fn changes(&self) -> watch::Receiver<u64> { self.changed.subscribe() }
}
#[derive(Clone)]
struct Control { server: Arc<Server>, lineage: String, pings: Arc<AtomicU64> }
async fn control(State(peer): State<Control>, ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(move |mut socket| async move {
        async fn send(socket: &mut axum::extract::ws::WebSocket, message: ServerMessage) {
            socket.send(axum::extract::ws::Message::Text(serde_json::to_string(&message).unwrap().into())).await.unwrap();
        }
        send(&mut socket, ServerMessage::Hello { protocol_version: PROTOCOL_VERSION, daemon_version: "block-outage-fixture".into(), lineage: Some(peer.lineage.clone()) }).await;
        while let Some(Ok(frame)) = socket.recv().await {
            match frame {
                axum::extract::ws::Message::Ping(data) => { peer.pings.fetch_add(1,Ordering::Relaxed); socket.send(axum::extract::ws::Message::Pong(data)).await.unwrap(); }
                axum::extract::ws::Message::Text(text) => {
                    let request: ClientRequest = serde_json::from_str(&text).unwrap();
                    let reply = match request.command {
                        ClientCommand::ConnectBlocks { node_id } => ServerMessage::BlockConnection { offer: peer.server.authorize(&node_id,peer.lineage.clone()).unwrap() },
                        ClientCommand::ListSessions => ServerMessage::Sessions { sessions: ["foreground", "background"].map(|id| SessionSummary {
                            id: id.into(), project_id: general_project_id(), title: id.into(), starter: false, status: SessionStatus::Running,
                            detail: None, context_usage: None, model: None, thinking_level: None, parent_id: None, created_at_ms: 1, updated_at_ms: 1,
                        }).to_vec() },
                        ClientCommand::GetSession { .. } | ClientCommand::GetCommands { .. } => ServerMessage::success(request.id, None, None),
                        ClientCommand::GetReceipts { session_id, .. } => ServerMessage::Receipts { session_id, reports: vec![] },
                        other => panic!("Read-only recovery must not execute {other:?}"),
                    };
                    send(&mut socket, reply).await;
                }
                axum::extract::ws::Message::Close(_) => break,
                _ => {},
            }
        }
    })
}
fn cached(c: &Controller, scope: &str) -> Vec<u8> {
    let path = c.store.root.join("blocks").join(format!("{}.sqlite3",tau_frontend::store::hash(&c.identity)));
    let db = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    tau_blocks::cached_content(&db,scope,"reply").unwrap()
}

#[tokio::test(flavor="multi_thread", worker_threads=4)]
async fn native_blocks_resume_after_udp_outage_while_websocket_stays_healthy() {
    let db = Connection::open_in_memory().unwrap(); tau_blocks::initialize(&db).unwrap();
    let lineage = tau_blocks::cursor(&db).unwrap().lineage;
    let tx = db.unchecked_transaction().unwrap();
    for scope in ["foreground", "background"] {
        let e = Event { id: "reply".into(), entry_id: "reply".into(), order: 0, phase: EventPhase::Live, origin: Origin::default(),
            role: EventRole::Assistant, kind: EventKind::Text, text: String::new(), timestamp: None, timestamp_ms: None,
            tool_call_id: None, tool_name: None, stop_reason: None, error_message: None, is_error: false, attachment: None };
        for (id, order, kind, meta, sealed, bytes) in [
            ("reply", 0, BlockKind::Text, serde_json::json!({"event":e}), false, b"verified prefix".to_vec()),
            ("@queue", i64::MAX as u64, BlockKind::Queue, serde_json::json!({}), true, serde_json::to_vec(&QueueState::native()).unwrap()),
        ] {
            tau_blocks::put(&tx,scope,BlockHeader { id:id.into(), parent:None, order, kind, meta, version:0, length:0, sealed, revision:0 },&bytes).unwrap();
        }
    }
    tx.commit().unwrap();
    let source = Arc::new(Source { db: Arc::new(std::sync::Mutex::new(db)), changed: watch::channel(0).0, reads: Default::default() });
    let server = Arc::new(Server::bind("127.0.0.1:0".parse().unwrap(),source.clone()).await.unwrap());
    let probe = tau_transfer::blocks::Client::bind().await.unwrap();
    let udp_port = server.authorize(&probe.node_id(),lineage.clone()).unwrap().port;
    probe.shutdown().await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap(); let tcp = listener.local_addr().unwrap();
    let pings = Arc::new(AtomicU64::new(0));
    let app = Router::new().route("/v1/ws",get(control)).with_state(Control { server:server.clone(), lineage, pings:pings.clone() });
    let control_task = tokio::spawn(async move { axum::serve(listener,app).await.unwrap() });
    let link = Link::new();
    let proxy = Proxy::new("127.0.0.2",tcp,format!("127.0.0.1:{udp_port}").parse().unwrap(),link.clone()).await;
    let local = tempfile::tempdir().unwrap(); let store = Store::open(local.path().into()).unwrap();
    store.put("","settings",&Settings { server_url:format!("http://{}",proxy.address), token:"read-only-block-outage-fixture".into() }).unwrap();
    let mut c = Controller::new(store,Arc::new(||{})).unwrap();
    tokio::time::timeout(Duration::from_secs(10),async {
        loop { c.poll(); if c.account.sessions.len()==2 { break; } tokio::time::sleep(Duration::from_millis(10)).await; }
        c.select("foreground").unwrap();
        loop { c.poll(); if ["foreground","background"].iter().all(|s| cached(&c,s)==b"verified prefix") { break; } tokio::time::sleep(Duration::from_millis(10)).await; }
    }).await.unwrap();
    let epoch = c.epoch; let ping_count = pings.load(Ordering::Relaxed);
    link.blackhole.store(true,Ordering::Relaxed);
    let suffix = (0..500).map(|n| format!("{}\n",blake3::hash(format!("outage-{n}").as_bytes()).to_hex())).collect::<String>();
    {
        let mut db = source.db.lock().unwrap(); let tx = db.transaction().unwrap();
        for scope in ["foreground","background"] {
            let h = tau_blocks::header(&tx,scope,"reply").unwrap().unwrap();
            tau_blocks::append(&tx,scope,"reply",h.version,h.length,suffix.as_bytes(),true).unwrap();
        }
        let sequence = tau_blocks::cursor(&tx).unwrap().sequence; tx.commit().unwrap();
        source.changed.send_replace(sequence);
    }
    let until = Instant::now()+Duration::from_secs(8);
    while Instant::now()<until { c.poll(); assert_eq!(c.epoch,epoch); tokio::time::sleep(Duration::from_millis(10)).await; }
    assert!(pings.load(Ordering::Relaxed)>ping_count && c.health.latest().is_some(),"Actual WebSocket ping/pong continues during a UDP-only outage");
    assert!(link.blackholed.load(Ordering::Relaxed)>0);
    for scope in ["foreground","background"] { assert_eq!(cached(&c,scope),b"verified prefix"); }
    let restored = Instant::now(); link.blackhole.store(false,Ordering::Relaxed);
    let expected = format!("verified prefix{suffix}").into_bytes(); let mut progress = [None,None];
    loop {
        c.poll();
        for (index,scope) in ["foreground","background"].iter().enumerate() {
            let bytes = cached(&c,scope);
            if bytes.len()>b"verified prefix".len() { progress[index].get_or_insert(restored.elapsed()); }
            assert!(progress[index].is_some() || restored.elapsed()<Duration::from_secs(15),
                "{scope} made no per-body progress after UDP recovery; reads={:?}; {}",source.reads.lock().unwrap(),c.diagnostics());
        }
        if ["foreground","background"].iter().all(|s| cached(&c,s)==expected)
            && c.selected().unwrap().feed.event("reply").is_some_and(|e| e.text.as_bytes()==expected) { break; }
        assert!(restored.elapsed()<Duration::from_secs(25),"Native suffix did not complete: {}",c.diagnostics());
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(c.epoch,epoch,"Native recovery cannot require replacing a healthy control socket");
    assert_eq!(c.account.selected.as_deref(),Some("foreground"));
    assert!(!c.account.read_at.contains_key("background"));
    for scope in ["foreground","background"] {
        assert!(source.reads.lock().unwrap().iter().any(|r| r.scope==scope && r.offset==b"verified prefix".len() as u64),
            "Recovery must request the verified suffix, not lose the cached prefix");
    }
    eprintln!("native-block-outage: first per-body progress={progress:?}, completion={:?}, UDP blackholed={}; {}",restored.elapsed(),link.blackholed.load(Ordering::Relaxed),c.diagnostics());
    drop(c); control_task.abort(); let _ = control_task.await;
    // Keep the UDP proxy available while the endpoint closes.
    server.shutdown().await; drop(proxy);
}
