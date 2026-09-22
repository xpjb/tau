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
    // Optional fields really are absent in a protocol-10 delta (not null/[]).
    let patch:ServerMessage=serde_json::from_value(json!({"type":"transcript_update","sessionId":"chat","generation":"g","sequence":5,"change":{"delta":{"eventId":"live","text":"world** 🦀"}}})).unwrap();
    let ServerMessage::TranscriptUpdate {
        generation,
        sequence,
        change,
        ..
    } = patch
    else {
        unreachable!()
    };
    feed.update(&generation, sequence, change).unwrap();
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
        socket.send(axum::extract::ws::Message::Text(r#"{"type":"hello","protocolVersion":10,"daemonVersion":"fixture"}"#.into())).await.unwrap();
        while let Some(Ok(axum::extract::ws::Message::Text(text)))=socket.recv().await {
            let request:Value=serde_json::from_str(&text).unwrap();
            match request["type"].as_str().unwrap() {
                "prompt"=>{peer.prompts.fetch_add(1,Ordering::SeqCst);*peer.id.lock().unwrap()=Some(request["id"].as_str().unwrap().into());let _=socket.close().await;return;}
                "open_session"=>{
                    let mut cut=serde_json::to_value(snapshot(vec![],None,0)).unwrap();
                    if peer.deliver.load(Ordering::SeqCst){cut["delivered"]=json!([peer.id.lock().unwrap().clone().unwrap()]);}
                    socket.send(axum::extract::ws::Message::Text(json!({"type":"transcript_snapshot","sessionId":"chat","snapshot":cut}).to_string().into())).await.unwrap();
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
    wait(&mut c, |c| c.selected().unwrap().feed.synchronized).await;
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
    wait(&mut c, |c| c.selected().unwrap().feed.synchronized).await;
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
            r#"{"type":"hello","protocolVersion":10,"daemonVersion":"fixture"}"#.into(),
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
