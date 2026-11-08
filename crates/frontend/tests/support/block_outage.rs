//! Production controller/cache/native watch workers through a real UDP blackhole.
//! Control is a read-only protocol fixture: no provider, command replay or daemon
//! restart is needed to isolate native transcript recovery from WebSocket health.
use super::*;
use axum::{Router, extract::{State, WebSocketUpgrade}, response::IntoResponse, routing::get};
use futures_util::{future::BoxFuture, FutureExt};
use rusqlite::Connection;
use tau_net::blocks::*;
use tau_net::native::{Backend, Server};
use tokio::sync::watch;

struct Source {
    db: Arc<std::sync::Mutex<Connection>>,
    changed: watch::Sender<u64>,
    reads: Arc<std::sync::Mutex<Vec<BlockRequest>>>,
}
impl Backend for Source {
    fn feed(&self, request: FeedRequest) -> BoxFuture<'static, anyhow::Result<FeedPage>> {
        let db = self.db.clone(); async move {
            let page=tau_block_store::feed(&db.lock().unwrap(), &request)?;
            if !page.records.is_empty() { native_trace::event(&format!("source metadata scope={} records={} cursor={}",request.scope,page.records.len(),page.cursor.sequence)); }
            Ok(page)
        }.boxed()
    }
    fn read(&self, request: BlockRequest) -> BoxFuture<'static, anyhow::Result<Option<ContentRange>>> {
        native_trace::event(&format!("source read scope={} offset={}",request.scope,request.offset));
        self.reads.lock().unwrap().push(request.clone());
        let db = self.db.clone(); async move { tau_block_store::read(&db.lock().unwrap(), &request) }.boxed()
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
                        ClientCommand::ListSessions => {
                            send(&mut socket, ServerMessage::ProjectPage {catalog_id:request.id.clone(),revision:1,after:None,next:None,projects:vec![tau_net::Project::general()]}).await;
                            ServerMessage::SessionPage {catalog_id:request.id.clone(),revision:1,after:None,next:None,states:Default::default(),sessions: ["foreground", "background"].map(|id| SessionSummary {
                            id: id.into(), project_id: general_project_id(), title: id.into(), starter: false, status: SessionStatus::Running,
                            detail: None, context_usage: None, model: None, thinking_level: None, parent_id: None, created_at_ms: 1, updated_at_ms: 1,
                        }).to_vec() } },
                        ClientCommand::GetModelCatalog => ServerMessage::ModelCatalog { catalog: ModelCatalog::default() },
                        ClientCommand::GetSession { .. } => ServerMessage::success(request.id, None, None),
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
    tau_block_store::cached_content(&db,scope,"reply").unwrap()
}

#[tokio::test(flavor="multi_thread", worker_threads=4)]
async fn native_blocks_resume_after_udp_outage_while_websocket_stays_healthy() {
    outage(Duration::from_secs(8), Duration::from_secs(5), true).await;
}

#[tokio::test(flavor="multi_thread", worker_threads=4)]
async fn native_blocks_reacquire_promptly_after_extended_udp_outage() {
    outage(Duration::from_secs(17), Duration::from_secs(5), true).await;
}

async fn outage(duration: Duration, progress_budget: Duration, downloads: bool) {
    native_trace::init(); native_trace::event("fixture start");
    let db = Connection::open_in_memory().unwrap(); tau_block_store::initialize(&db).unwrap();
    let lineage = tau_block_store::cursor(&db).unwrap().lineage;
    let tx = db.unchecked_transaction().unwrap();
    for scope in ["foreground", "background"] {
        let e = Event { id: "reply".into(), entry_id: "reply".into(), order: 0, phase: EventPhase::Live, origin: Origin::default(),
            role: EventRole::Assistant, kind: EventKind::Text, text: String::new(), timestamp: None, timestamp_ms: None,
            tool_call_id: None, tool_name: None, stop_reason: None, error_message: None, is_error: false, attachment: None };
        for (id, order, kind, meta, sealed, bytes) in [
            ("reply", 0, BlockKind::Text, serde_json::json!({"event":e}), false, b"verified prefix".to_vec()),
            ("@queue", i64::MAX as u64, BlockKind::Queue, serde_json::json!({}), true, serde_json::to_vec(&QueueState::native()).unwrap()),
        ] {
            tau_block_store::put(&tx,scope,BlockHeader { id:id.into(), parent:None, order, kind, meta, version:0, length:0, sealed, revision:0 },&bytes).unwrap();
        }
    }
    // A sealed file remains immutable through the outage. Its original
    // download must survive reconnection, not require another user click.
    let mut file = vec![0u8;128*1024];
    blake3::Hasher::new().update(b"native outage file").finalize_xof().fill(&mut file);
    if downloads {
        use sha2::Digest;
        let e = Event { id: "file-card".into(), entry_id: "outage-file".into(), order: 1, phase: EventPhase::Saved, origin: Origin::default(),
            role: EventRole::Assistant, kind: EventKind::Text, text: String::new(), timestamp: None, timestamp_ms: None,
            tool_call_id: None, tool_name: None, stop_reason: None, error_message: None, is_error: false,
            attachment:Some(ChatAttachment { source_path:None, kind:AttachmentKind::File, file_name:"outage.bin".into(), caption:None, size:Some(file.len() as u64) }) };
        tau_block_store::put(&tx,"foreground",BlockHeader { id:e.id.clone(), parent:None, order:1, kind:BlockKind::Text,
            meta:serde_json::json!({"event":e}), version:0, length:0, sealed:true, revision:0 },b"").unwrap();
        tau_block_store::put(&tx,"foreground",BlockHeader { id:"file:outage-file".into(), parent:Some("file-card".into()), order:0,
            kind:BlockKind::File, meta:serde_json::json!({"sha256":format!("{:x}",sha2::Sha256::digest(&file))}),
            version:0, length:0, sealed:true, revision:0 },&file).unwrap();
    }
    tx.commit().unwrap();
    let source = Arc::new(Source { db: Arc::new(std::sync::Mutex::new(db)), changed: watch::channel(0).0, reads: Default::default() });
    let server = Arc::new(Server::bind("127.0.0.1:0".parse().unwrap(),source.clone()).await.unwrap());
    native_trace::event("server bound");
    let probe = tau_net::native::Client::bind().await.unwrap();
    let udp_port = server.authorize(&probe.node_id(),lineage.clone()).unwrap().port;
    native_trace::event("probe closing"); probe.shutdown().await; native_trace::event("probe closed");
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
    let download_key=Controller::download_key("foreground","outage-file");
    let download = if downloads {
        let path=c.download("foreground","outage-file",MAX_UPLOAD_BYTES as u64).unwrap();
        tokio::time::timeout(Duration::from_secs(10),async {
            loop { c.poll(); if c.downloads[&download_key].status.transferred>=BLOCK_CHUNK_BYTES as u64 { break; }
                tokio::time::sleep(Duration::from_millis(10)).await; }
        }).await.unwrap();
        assert!(!c.downloads[&download_key].status.done);
        Some(path)
    } else { None };
    let epoch = c.epoch; let ping_count = pings.load(Ordering::Relaxed);
    native_trace::event("OUTAGE BEGINS"); link.udp_blackhole.store(true,Ordering::Relaxed);
    let suffix = (0..500).map(|n| format!("{}\n",blake3::hash(format!("outage-{n}").as_bytes()).to_hex())).collect::<String>();
    {
        let mut db = source.db.lock().unwrap(); let tx = db.transaction().unwrap();
        for scope in ["foreground","background"] {
            let h = tau_block_store::header(&tx,scope,"reply").unwrap().unwrap();
            tau_block_store::append(&tx,scope,"reply",h.version,h.length,suffix.as_bytes(),true).unwrap();
        }
        let sequence = tau_block_store::cursor(&tx).unwrap().sequence; tx.commit().unwrap();
        source.changed.send_replace(sequence);
    }
    let until = Instant::now()+duration;
    while Instant::now()<until { c.poll(); assert_eq!(c.epoch,epoch); tokio::time::sleep(Duration::from_millis(10)).await; }
    assert!(pings.load(Ordering::Relaxed)>ping_count && c.health.latest().is_some(),"Actual WebSocket ping/pong continues during a UDP-only outage");
    assert!(link.blackholed.load(Ordering::Relaxed)>0);
    for scope in ["foreground","background"] { assert_eq!(cached(&c,scope),b"verified prefix"); }
    if downloads { assert!(!c.downloads[&download_key].status.done,
        "An interrupted read must wait/resume, not become a failed download: {:?}",c.downloads[&download_key].status.failure); }
    let file_prefix=c.downloads.get(&download_key).map_or(0,|d|d.status.transferred);
    let restored = Instant::now(); native_trace::event("LINK RESTORED"); link.udp_blackhole.store(false,Ordering::Relaxed);
    let expected = format!("verified prefix{suffix}").into_bytes(); let mut progress = [None,None]; let mut file_progress=None;
    loop {
        c.poll();
        for (index,scope) in ["foreground","background"].iter().enumerate() {
            let bytes = cached(&c,scope);
            if bytes.len()>b"verified prefix".len() && progress[index].is_none() {
                native_trace::event(&format!("cache progress scope={scope} prefix={}",bytes.len())); progress[index]=Some(restored.elapsed());
            }
            assert!(progress[index].is_some() || restored.elapsed()<progress_budget,
                "{scope} made no per-body progress within {progress_budget:?} after UDP recovery; reads={:?}; {}",source.reads.lock().unwrap(),c.diagnostics());
        }
        if downloads {
            let d=&c.downloads[&download_key].status;
            assert!(d.failure.is_none(),"File recovery failed: {:?}",d.failure);
            if d.transferred>file_prefix {file_progress.get_or_insert(restored.elapsed());}
            assert!(file_progress.is_some() || restored.elapsed()<progress_budget,"File did not resume within {progress_budget:?}");
        }
        if ["foreground","background"].iter().all(|s| cached(&c,s)==expected)
            && c.selected().unwrap().feed.event("reply").is_some_and(|e| e.text.as_bytes()==expected)
            && (!downloads || c.downloads[&download_key].status.done) { break; }
        assert!(restored.elapsed()<Duration::from_secs(25),"Native suffix did not complete: {}",c.diagnostics());
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    if let Some(path)=download { assert_eq!(std::fs::read(path).unwrap(),file); }
    assert_eq!(c.epoch,epoch,"Native recovery cannot require replacing a healthy control socket");
    assert_eq!(c.account.selected.as_deref(),Some("foreground"));
    assert!(!c.account.read_at.contains_key("background"));
    for scope in ["foreground","background"] {
        assert!(source.reads.lock().unwrap().iter().any(|r| r.scope==scope && r.offset==b"verified prefix".len() as u64),
            "Recovery must request the verified suffix, not lose the cached prefix");
    }
    eprintln!("native-block-outage: first per-body progress={progress:?}, file progress={file_progress:?}, completion={:?}, UDP blackholed={}; {}",restored.elapsed(),link.blackholed.load(Ordering::Relaxed),c.diagnostics());
    native_trace::event("recovery complete; cleanup");
    drop(c); control_task.abort(); let _ = control_task.await;
    // Keep the UDP proxy available while the endpoint closes.
    let _=tokio::time::timeout(Duration::from_secs(2),server.shutdown()).await;
    drop(proxy); native_trace::event("fixture end");
}
