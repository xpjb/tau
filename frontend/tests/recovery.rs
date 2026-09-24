use axum::{
    Router,
    extract::{State, WebSocketUpgrade},
    response::IntoResponse,
    routing::get,
};
use futures_util::SinkExt;
use serde_json::{Value, json};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tau_frontend::{
    controller::Controller,
    feed::Feed,
    store::{Delivery, Settings, Store},
    transport::{Command, Event as NetworkEvent, Network},
};
use tau_protocol::*;

fn event(id: &str, order: u64, phase: &str) -> Value {
    json!({"id":id,"order":order,"entryId":id,"phase":phase,"origin":{},"role":"assistant","kind":"text","text":"Hello **","timestamp":null,"timestampMs":null,"toolCallId":null,"toolName":null,"stopReason":null,"errorMessage":null,"isError":false,"attachment":null})
}
fn snapshot(events: Vec<Value>, before: Option<u64>, sequence: u64) -> TranscriptSnapshot {
    serde_json::from_value(json!({"generation":"g","sequence":sequence,"events":events,"queue":{"available":true,"requests":[],"runId":null,"paused":false,"control":null,"capabilities":[],"boundaries":[]},"before":before,"delivered":[]})).unwrap()
}
#[test]
fn retained_history_delta_gap_and_stale_page_are_transactional() {
    let mut feed = Feed::default();
    feed.snapshot(snapshot(vec![event("live", 2, "live")], Some(2), 4))
        .unwrap();
    feed.page(
        "g",
        2,
        HistoryPage {
            events: vec![serde_json::from_value(event("older", 1, "saved")).unwrap()],
            before: Some(1),
        },
    )
    .unwrap();
    // The renderer's internal delta adapter is not a network message.
    let change:TranscriptChange=serde_json::from_value(json!({"delta":{"eventId":"live","text":"world** 🦀"}})).unwrap();
    feed.update("g",5,change).unwrap();
    assert_eq!(feed.event("live").unwrap().text, "Hello **world** 🦀");
    let change: TranscriptChange =
        serde_json::from_value(json!({"events":[event("collision",1,"saved")],"removed":["live"]}))
            .unwrap();
    assert!(feed.update("g", 6, change).is_err());
    assert_eq!(feed.events.len(), 2);
    assert!(feed.event("live").is_some());
    assert_eq!(feed.sequence, 5);
    assert!(!feed.synchronized);
    // A reconnect cut with overlap preserves loaded older saved history.
    feed.snapshot(snapshot(vec![event("live", 2, "saved")], Some(2), 6))
        .unwrap();
    assert!(feed.event("older").is_some());
    assert_eq!(feed.before, Some(1));
    assert!(
        !feed
            .page(
                "old-generation",
                1,
                HistoryPage {
                    events: vec![],
                    before: None
                }
            )
            .unwrap()
    );
    let old = feed.events.clone();
    assert!(
        feed.page(
            "g",
            1,
            HistoryPage {
                events: vec![serde_json::from_value(event("older", 0, "saved")).unwrap()],
                before: None
            }
        )
        .is_err()
    );
    assert_eq!(feed.events, old);
    // A cut without overlap drops the stale window rather than hiding a gap.
    feed.snapshot(snapshot(vec![event("new-tail", 50, "saved")], Some(50), 10))
        .unwrap();
    assert_eq!(feed.events.len(), 1);
}

#[test]
fn offline_new_chat_and_send_are_durable_before_any_server_ack() {
    let dir = tempfile::tempdir().unwrap();
    let mut c = Controller::new(Store::open(dir.path().into()).unwrap(), Arc::new(|| {})).unwrap();
    c.new_chat().unwrap();
    let id = c.account.selected.clone().unwrap();
    let request = c.account.pending_create.as_ref().unwrap().id.clone();
    assert_eq!(id, request);
    assert_eq!(c.account.sessions[0].title, "Creating chat…");
    c.draft("A message typed while offline".into()).unwrap();
    c.send_prompt().unwrap();
    let prompt = c.selected().unwrap().local.pending[0].request.id.clone();
    assert_eq!(c.selected().unwrap().local.pending[0].status, Delivery::WaitingForChat);
    drop(c);
    let mut c = Controller::new(Store::open(dir.path().into()).unwrap(), Arc::new(|| {})).unwrap();
    assert_eq!(c.account.pending_create.as_ref().unwrap().id, request);
    assert_eq!(c.account.selected.as_deref(), Some(id.as_str()));
    assert_eq!(c.selected().unwrap().local.pending[0].request.id, prompt);
    assert_eq!(c.selected().unwrap().local.pending[0].status, Delivery::WaitingForChat);
    let summary = tau_protocol::SessionSummary { id:id.clone(), project_id:general_project_id(), title:"New chat".into(), starter:true,
        status:SessionStatus::Idle, detail:None, context_usage:None, model:None,
        parent_id:None, created_at_ms:1, updated_at_ms:1 };
    c.message(ServerMessage::Sessions { sessions:vec![summary] }).unwrap();
    assert!(c.account.pending_create.is_none());
    assert_eq!(c.selected().unwrap().local.pending[0].request.id, prompt);
    assert_eq!(c.selected().unwrap().local.pending[0].status, Delivery::WaitingForChat,
        "A saved prompt cannot be sent before the transport is ready");
}

#[test]
fn offline_create_keeps_its_topic_across_reuse_and_late_ack_without_changing_selection() {
    let dir = tempfile::tempdir().unwrap();
    let mut c = Controller::new(Store::open(dir.path().into()).unwrap(), Arc::new(|| {})).unwrap();
    let topic = uuid::Uuid::new_v4().to_string();
    c.account.projects.push(Project { id:topic.clone(), name:"Work".into(), prompt:String::new(), revision:0 });
    c.select_project(&topic).unwrap();
    c.new_chat().unwrap();
    let provisional = c.account.selected.clone().unwrap();
    assert_eq!(c.account.sessions[0].project_id,topic);
    assert!(matches!(&c.account.pending_create.as_ref().unwrap().command,
        ClientCommand::CreateSession { project_id, .. } if project_id == &topic));
    c.draft("Keep this message in Work".into()).unwrap();
    c.send_prompt().unwrap();
    let prompt = c.selected().unwrap().local.pending[0].request.id.clone();
    drop(c);
    let mut c = Controller::new(Store::open(dir.path().into()).unwrap(), Arc::new(|| {})).unwrap();
    assert_eq!(c.account.selected_project,topic);
    c.select_project(GENERAL_PROJECT_ID).unwrap();
    c.message(ServerMessage::Sessions { sessions:vec![SessionSummary { id:"existing-work".into(), project_id:topic.clone(),
        title:"New chat".into(),starter:true,status:SessionStatus::Idle,detail:None,context_usage:None,model:None,
        parent_id:None,created_at_ms:1,updated_at_ms:1 }] }).unwrap();
    assert_eq!(c.account.sessions.iter().find(|s| s.id == provisional).unwrap().project_id,topic);
    c.message(ServerMessage::success(provisional,Some("existing-work".into()),None)).unwrap();
    assert!(c.account.pending_create.is_none());
    assert_eq!(c.account.selected_project,GENERAL_PROJECT_ID,"A late acknowledgement cannot switch the current topic");
    assert_eq!(c.account.last_chat_by_project.get(&topic).map(String::as_str),Some("existing-work"));
    drop(c);
    let mut c = Controller::new(Store::open(dir.path().into()).unwrap(), Arc::new(|| {})).unwrap();
    c.select_project(&topic).unwrap();
    assert_eq!(c.account.selected.as_deref(),Some("existing-work"));
    assert_eq!(c.selected().unwrap().local.pending[0].request.id,prompt);
    assert!(matches!(&c.selected().unwrap().local.pending[0].request.command,
        ClientCommand::Prompt { session_id, .. } if session_id == "existing-work"));
}

#[test]
fn existing_starter_coalesces_pending_create_without_losing_draft_or_files() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("reference.png");
    std::fs::write(&source, b"fixture attachment").unwrap();
    let mut c = Controller::new(Store::open(dir.path().join("local")).unwrap(), Arc::new(|| {})).unwrap();
    c.new_chat().unwrap();
    let provisional = c.account.selected.clone().unwrap();
    c.attach(&source,None).unwrap();
    c.draft("First message".into()).unwrap();
    c.send_prompt().unwrap();
    let original = c.selected().unwrap().local.pending[0].request.id.clone();
    c.draft("Unsent second draft".into()).unwrap();
    c.attach(&source,None).unwrap();
    c.message(ServerMessage::Sessions { sessions:vec![SessionSummary { id:"existing".into(), project_id:general_project_id(), title:"New chat".into(),starter:true,
        status:SessionStatus::Idle,detail:None,context_usage:None,model:None,parent_id:None,created_at_ms:1,updated_at_ms:1 }] }).unwrap();
    assert_eq!(c.account.selected.as_deref(),Some(provisional.as_str()), "A remote starter is not proof that our create was accepted");
    c.message(serde_json::from_value(json!({"type":"response","requestId":provisional,"ok":true,
        "sessionId":"existing","uncertain":false})).unwrap()).unwrap();
    assert_eq!(c.account.selected.as_deref(),Some("existing"));
    assert!(c.account.pending_create.is_none());
    assert!(c.account.sessions.iter().all(|s| s.id != provisional));
    let chat = c.selected().unwrap();
    assert_eq!(chat.local.draft,"Unsent second draft");
    assert_eq!(chat.local.pending[0].request.id,original);
    assert_eq!(chat.local.pending[0].status,Delivery::WaitingForChat);
    assert!(matches!(&chat.local.pending[0].request.command,ClientCommand::Prompt {session_id,..} if session_id == "existing"));
    assert!(chat.local.pending[0].files[0].path.exists() && chat.local.files[0].path.exists());
    assert_eq!(std::fs::read(&chat.local.pending[0].files[0].path).unwrap(),b"fixture attachment");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lost_ack_survives_restart_without_replay_and_reconciles_by_id() {
    #[derive(Clone)]
    struct Peer {
        prompts: Arc<AtomicUsize>,
        id: Arc<Mutex<Option<String>>>,
        deliver: Arc<std::sync::atomic::AtomicBool>,
    }
    async fn ws(State(peer): State<Peer>, ws: WebSocketUpgrade) -> impl IntoResponse {
        ws.on_upgrade(move|mut socket|async move {
        socket.send(axum::extract::ws::Message::Text(serde_json::to_string(&tau_protocol::ServerMessage::Hello { protocol_version:tau_protocol::PROTOCOL_VERSION,daemon_version:"fixture".into() }).unwrap().into())).await.unwrap();
        while let Some(Ok(axum::extract::ws::Message::Text(text)))=socket.recv().await {
            let request:Value=serde_json::from_str(&text).unwrap();
            match request["type"].as_str().unwrap() {
                "prompt"=>{peer.prompts.fetch_add(1,Ordering::SeqCst);*peer.id.lock().unwrap()=Some(request["id"].as_str().unwrap().into());let _=socket.close().await;return;}
                "get_session"=>{
                    socket.send(axum::extract::ws::Message::Text(json!({"type":"response","requestId":request["id"],"ok":true,"uncertain":false}).to_string().into())).await.unwrap();
                }
                "get_receipts"=>{
                    if peer.deliver.load(Ordering::SeqCst) {
                        socket.send(axum::extract::ws::Message::Text(json!({"type":"receipts","sessionId":"chat","reports":[{
                            "id":peer.id.lock().unwrap().clone().unwrap(),"accepted":true,"complete":true,"error":null,"notice":null
                        }]}).to_string().into())).await.unwrap();
                    }
                }
                _=>{},
            }
        }
    })
    }
    let peer = Peer {
        prompts: Arc::new(AtomicUsize::new(0)),
        id: Arc::new(Mutex::new(None)),
        deliver: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = Router::new()
        .route("/v1/ws", get(ws))
        .with_state(peer.clone());
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().into()).unwrap();
    store
        .put(
            "",
            "settings",
            &Settings {
                server_url: format!("http://{address}"),
                token: "fixture-token".into(),
            },
        )
        .unwrap();
    let mut c = Controller::new(store, Arc::new(|| {})).unwrap();
    async fn wait(c: &mut Controller, f: impl Fn(&Controller) -> bool) {
        tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                c.poll();
                if f(c) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }
    wait(&mut c, |c| c.epoch.is_some()).await;
    c.select("chat").unwrap();
    wait(&mut c, |c| c.epoch.is_some() && !c.selected().unwrap().feed.opening).await;
    c.draft("send exactly once, even if the ack is lost".into())
        .unwrap();
    c.send_prompt().unwrap();
    wait(&mut c, |c| {
        c.selected().unwrap().local.pending[0].status == Delivery::Unconfirmed
    })
    .await;
    let original = c.selected().unwrap().local.pending[0].request.id.clone();
    drop(c);
    let mut c = Controller::new(Store::open(dir.path().into()).unwrap(), Arc::new(|| {})).unwrap();
    assert_eq!(c.selected().unwrap().local.pending[0].request.id, original);
    wait(&mut c, |c| c.epoch.is_some() && !c.selected().unwrap().feed.opening).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    c.poll();
    assert_eq!(peer.prompts.load(Ordering::SeqCst), 1);
    peer.deliver.store(true, Ordering::SeqCst);
    c.open("chat").unwrap();
    wait(&mut c, |c| c.selected().unwrap().local.pending.is_empty()).await;
    assert_eq!(peer.prompts.load(Ordering::SeqCst), 1);
    drop(c);
    task.abort();
}

#[tokio::test]
async fn protocol_mismatch_and_wrong_epoch_cannot_send() {
    use futures_util::StreamExt;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let seen = Arc::new(AtomicUsize::new(0));
    let count = seen.clone();
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(socket).await.unwrap();
        use futures_util::SinkExt;
        ws.send(tokio_tungstenite::tungstenite::Message::Text(
            r#"{"type":"hello","protocolVersion":999,"daemonVersion":"future"}"#.into(),
        ))
        .await
        .unwrap();
        while let Some(Ok(_)) = ws.next().await {
            count.fetch_add(1, Ordering::SeqCst);
        }
    });
    let mut n = Network::start(
        Settings {
            server_url: format!("http://{address}"),
            token: "fixture".into(),
        },
        Arc::new(|| {}),
    );
    n.send(Command::Request {
        epoch: 0,
        request: ClientRequest {
            id: "stale".into(),
            command: ClientCommand::DeleteSession {
                session_id: "never-delete".into(),
            },
        },
    })
    .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while let Some(e) = n.events.recv().await {
            if let NetworkEvent::Fatal(detail) = e {
                assert!(detail.contains("999"));
                break;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(seen.load(Ordering::SeqCst), 0);
    drop(n);
    server.abort();
}

#[tokio::test]
async fn stale_socket_epoch_is_rejected_after_a_successful_handshake() {
    use futures_util::{SinkExt, StreamExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(socket).await.unwrap();
        ws.send(tokio_tungstenite::tungstenite::Message::Text(
            serde_json::to_string(&tau_protocol::ServerMessage::Hello {
                protocol_version: tau_protocol::PROTOCOL_VERSION,
                daemon_version: "fixture".into(),
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
        let text = ws.next().await.unwrap().unwrap().into_text().unwrap();
        let request: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(request["type"], "list_sessions");
        assert_eq!(request["id"], "current");
    });
    let mut n = Network::start(
        Settings {
            server_url: format!("http://{address}"),
            token: "fixture".into(),
        },
        Arc::new(|| {}),
    );
    let epoch = match tokio::time::timeout(Duration::from_secs(3), n.events.recv())
        .await
        .unwrap()
        .unwrap()
    {
        NetworkEvent::Ready(epoch) => epoch,
        _ => panic!("Expected hello"),
    };
    for (epoch, id, command) in [
        (
            epoch - 1,
            "stale",
            ClientCommand::DeleteSession {
                session_id: "never-delete".into(),
            },
        ),
        (epoch, "current", ClientCommand::ListSessions),
    ] {
        n.send(Command::Request {
            epoch,
            request: ClientRequest {
                id: id.into(),
                command,
            },
        })
        .unwrap();
    }
    let reply = tokio::time::timeout(Duration::from_secs(3), n.events.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(reply,NetworkEvent::NotSent(id,_) if id=="stale"));
    tokio::time::timeout(Duration::from_secs(3), server)
        .await
        .unwrap()
        .unwrap();
}

#[test]
fn accepted_queue_edit_retains_optimistic_text_until_complete_replication_after_restart() {
    let root=tempfile::tempdir().unwrap();
    let mut c=Controller::new(Store::open(root.path().into()).unwrap(),Arc::new(||{})).unwrap();
    c.ensure_chat("chat").unwrap();c.account.selected=Some("chat".into());
    c.store.put(&c.identity,"account",&c.account).unwrap();
    let pending=tau_frontend::store::Pending {request:ClientRequest {id:"edit".into(),command:ClientCommand::QueueControl {session_id:"chat".into(),generation:"g".into(),operation:QueueOperation::Edit {request_id:"queued".into(),revision:0,text:"new complete text".into()}}},started_at_ms:None,text:"new complete text".into(),files:vec![],status:Delivery::Sending,detail:None};
    let chat=c.chats.get_mut("chat").unwrap();chat.local.pending.push(pending);chat.feed.queue=QueueState::native();
    chat.feed.queue.requests.push(QueuedRequest {request_id:"queued".into(),revision:0,kind:"steer".into(),text:"old text".into(),images:0,timestamp_ms:None});
    c.message(ServerMessage::success("edit".into(),Some("chat".into()),None)).unwrap();
    assert_eq!(c.chats["chat"].local.pending[0].status,Delivery::Accepted);
    drop(c);
    let mut c=Controller::new(Store::open(root.path().into()).unwrap(),Arc::new(||{})).unwrap();
    let chat=c.chats.get_mut("chat").unwrap();assert_eq!(chat.local.pending[0].status,Delivery::Accepted);
    chat.feed.queue=QueueState::native();chat.feed.queue.requests.push(QueuedRequest {request_id:"queued".into(),revision:1,kind:"steer".into(),text:"new".into(),images:0,timestamp_ms:None});
    chat.feed.incomplete.insert("queued:queued".into());
    chat.local.reconcile_complete(&chat.feed.queue,&[],&chat.feed.incomplete);assert_eq!(chat.local.pending.len(),1);
    chat.feed.queue.requests[0].text="new complete text".into();chat.feed.incomplete.clear();
    chat.local.reconcile_complete(&chat.feed.queue,&[],&chat.feed.incomplete);assert!(chat.local.pending.is_empty());
}

#[tokio::test(flavor="multi_thread",worker_threads=2)]
async fn generic_control_outbox_recovers_original_outcome_without_reexecuting_after_restart() {
    #[derive(Clone)]
    struct Peer {request:Arc<Mutex<Option<ClientRequest>>>,mutations:Arc<AtomicUsize>}
    let peer=Peer {request:Arc::new(Mutex::new(None)),mutations:Arc::new(AtomicUsize::new(0))};
    let counted=peer.clone();
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let address=listener.local_addr().unwrap();
    let app=Router::new().route("/v1/ws",get(move |ws:WebSocketUpgrade| {let peer=peer.clone();async move {ws.on_upgrade(move |mut socket|async move {
        socket.send(axum::extract::ws::Message::Text(serde_json::to_string(&ServerMessage::Hello {protocol_version:PROTOCOL_VERSION,daemon_version:"fixture".into()}).unwrap().into())).await.unwrap();
        while let Some(Ok(frame))=socket.recv().await {
            let axum::extract::ws::Message::Text(text)=frame else {continue;};
            let request:ClientRequest=serde_json::from_str(&text).unwrap();
            match &request.command {
                ClientCommand::RenameSession {..}=>{peer.mutations.fetch_add(1,Ordering::SeqCst);*peer.request.lock().unwrap()=Some(request);let _=socket.close().await;return;}
                ClientCommand::GetOperation {operation_id}=>{
                    let saved=peer.request.lock().unwrap().clone().unwrap();assert_eq!(&saved.id,operation_id);
                    let response=ServerMessage::Operation {operation_id:operation_id.clone(),registered:true,response:Some(Box::new(ServerMessage::success(saved.id,Some("chat".into()),None)))};
                    socket.send(axum::extract::ws::Message::Text(serde_json::to_string(&response).unwrap().into())).await.unwrap();
                }
                _=>{},
            }
        }
    })}}));
    let server=tokio::spawn(async move {axum::serve(listener,app).await.unwrap()});
    let root=tempfile::tempdir().unwrap();let store=Store::open(root.path().into()).unwrap();
    store.put("","settings",&Settings {server_url:format!("http://{address}"),token:"fixture".into()}).unwrap();
    let mut c=Controller::new(store,Arc::new(||{})).unwrap();
    tokio::time::timeout(Duration::from_secs(5),async {loop {c.poll();if c.epoch.is_some() {break;}tokio::time::sleep(Duration::from_millis(10)).await;}}).await.unwrap();
    let id=c.request(ClientCommand::RenameSession {session_id:"chat".into(),title:"durable title".into()}).unwrap();
    assert_eq!(c.account.pending_controls[&id].request.id,id);
    tokio::time::timeout(Duration::from_secs(5),async {while counted.mutations.load(Ordering::SeqCst)==0 {tokio::time::sleep(Duration::from_millis(10)).await;}}).await.unwrap();
    drop(c);
    let mut c=Controller::new(Store::open(root.path().into()).unwrap(),Arc::new(||{})).unwrap();assert!(c.account.pending_controls.contains_key(&id));
    tokio::time::timeout(Duration::from_secs(5),async {loop {c.poll();if c.account.pending_controls.is_empty() {break;}tokio::time::sleep(Duration::from_millis(10)).await;}}).await.unwrap();
    assert_eq!(counted.mutations.load(Ordering::SeqCst),1,"Receipt reconciliation must not execute another mutation");
    drop(c);server.abort();
}
