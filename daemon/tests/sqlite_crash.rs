#![cfg(unix)]
use std::path::Path;
use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
use std::time::Duration;
use axum::{Router, Json, body::Body, response::Response, routing::post};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::{connect_async, tungstenite::{Message, client::IntoClientRequest}};

const TOKEN: &str = "isolated-tau-sqlite-crash-test-token";
struct Client { socket: tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, seen: Vec<Value> }
impl Client {
    async fn until(&mut self, predicate: impl Fn(&Value) -> bool) -> Value {
        tokio::time::timeout(Duration::from_secs(10),async {
            loop {
                match self.socket.next().await.unwrap().unwrap() {
                    Message::Text(text) => { let value: Value = serde_json::from_str(&text).unwrap(); self.seen.push(value.clone()); if predicate(&value) { return value; } }
                    Message::Ping(_) => self.socket.flush().await.unwrap(),
                    other => panic!("Unexpected socket event {other:?}"),
                }
            }
        }).await.expect("Expected daemon event")
    }
    async fn request(&mut self, value: Value) -> Value {
        self.socket.send(Message::Text(value.to_string().into())).await.unwrap();
        self.until(|event| event["type"] == "response" && event["requestId"] == value["id"]).await
    }
    async fn open(&mut self, id: &str) -> Value {
        assert_eq!(self.request(json!({"id":"open","type":"open_session","sessionId":id,"requests":["first","second","deleted","edit","delete"]})).await["ok"],true);
        self.seen.iter().rev().find(|event| event["type"] == "transcript_snapshot").unwrap()["snapshot"].clone()
    }
}
async fn boot(root: &Path) -> (tokio::process::Child, Client) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap(); drop(listener);
    let log = std::fs::OpenOptions::new().create(true).append(true).open(root.join("daemon.log")).unwrap();
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_taud"))
        .env("TAU_BIND",address.to_string()).env("TAU_TRANSFER_BIND","127.0.0.1:0").env("TAU_TOKEN",TOKEN)
        .env("TAU_DATABASE_PATH",root.join("tau.sqlite3")).env("TAU_SETTINGS_PATH",root.join("settings.json"))
        .env("TAU_CWD",root).env("TAU_ATTACHMENT_ROOT",root.join("outbox")).env("TAU_UPLOAD_ROOT",root.join("uploads"))
        .env("TAU_TELEMETRY_PATH",root.join("crashes.jsonl")).env("RUST_LOG","info")
        .env_remove("TAU_IMPORT_PI_DIR").env_remove("TAU_STATE_PATH").env_remove("TAU_SESSION_DIR").env_remove("TAU_TITLE_COMMAND")
        .stdin(std::process::Stdio::null()).stdout(log.try_clone().unwrap()).stderr(log).kill_on_drop(true).spawn().unwrap();
    let socket = tokio::time::timeout(Duration::from_secs(10),async {
        loop {
            assert!(child.try_wait().unwrap().is_none(),"Daemon exited: {}",std::fs::read_to_string(root.join("daemon.log")).unwrap());
            let mut request = format!("ws://{address}/v1/ws").into_client_request().unwrap();
            request.headers_mut().insert("authorization",format!("Bearer {TOKEN}").parse().unwrap());
            if let Ok((socket,_)) = connect_async(request).await { break socket; }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.unwrap();
    let mut client = Client {socket,seen:Vec::new()}; client.until(|event| event["type"] == "hello").await;
    (child,client)
}

#[tokio::test]
async fn sigkill_after_ack_recovers_wal_queue_receipts_and_unfinished_turn_without_reexecution() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let (send,mut requests) = tokio::sync::mpsc::unbounded_channel();
    let count = Arc::new(AtomicUsize::new(0));
    let app = Router::new().route("/{*path}",post(move |Json(request): Json<Value>| {
        let send = send.clone(); let count = count.clone();
        async move {
            send.send(request).unwrap();
            if matches!(count.fetch_add(1,Ordering::SeqCst),0 | 2) { std::future::pending::<()>().await; }
            Response::builder().header("content-type","text/event-stream").body(Body::from(format!("data: {}\n\ndata: [DONE]\n\n",
                json!({"choices":[{"index":0,"delta":{"content":"Recovered after SIGKILL"},"finish_reason":"stop"}],"usage":{"total_tokens":100}})))).unwrap()
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let provider = format!("http://{}",listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener,app).await.unwrap(); });
    std::fs::write(root.path().join("settings.json"),json!({"agent":{"loadAgentsFiles":false,"retry":{"enabled":false}},"daemon":{"idleTimeoutSeconds":0},
        "providers":{"openai-codex":{"api":"chat_completions","baseUrl":provider,"webSearch":false}}}).to_string()).unwrap();
    let auth = root.path().join("auth.json");
    std::fs::write(&auth,json!({"openai-codex":{"type":"api_key","key":"crash-fixture-key"}}).to_string()).unwrap();
    std::fs::set_permissions(auth,std::fs::Permissions::from_mode(0o600)).unwrap();
    let (mut daemon,mut client) = boot(root.path()).await;
    let id = client.request(json!({"id":"create","type":"create_session"})).await["sessionId"].as_str().unwrap().to_owned();
    client.open(&id).await;
    assert_eq!(client.request(json!({"id":"first","type":"prompt","sessionId":id,"text":"First task"})).await["ok"],true);
    tokio::time::timeout(Duration::from_secs(10),requests.recv()).await.unwrap().unwrap();
    for (id_request,text) in [("second","Second task"),("deleted","Do not run this")] {
        assert_eq!(client.request(json!({"id":id_request,"type":"prompt","sessionId":id,"text":text})).await["disposition"],"queued");
    }
    let snapshot = client.open(&id).await;
    let original_generation=snapshot["generation"].clone();
    for (command,operation) in [("edit",json!({"type":"edit","requestId":"second","revision":0,"text":"Edited second task"})),
        ("delete",json!({"type":"delete","requestId":"deleted","revision":0}))] {
        assert_eq!(client.request(json!({"id":command,"type":"queue_control","sessionId":id,"generation":snapshot["generation"],"operation":operation})).await["ok"],true);
    }
    assert!(std::fs::metadata(root.path().join("tau.sqlite3-wal")).unwrap().len()>0);
    // No graceful shutdown, queue settlement, destructor or explicit checkpoint.
    daemon.kill().await.unwrap(); assert!(!daemon.wait().await.unwrap().success()); drop(client);
    let (mut daemon,mut client) = boot(root.path()).await;
    let snapshot = client.open(&id).await;
    assert_eq!(snapshot["queue"]["paused"],true);
    assert_eq!(snapshot["queue"]["requests"].as_array().unwrap().len(),1);
    assert_eq!(snapshot["queue"]["requests"][0]["text"],"Edited second task");
    assert_eq!(snapshot["queue"]["requests"][0]["revision"],1);
    assert_eq!(snapshot["delivered"].as_array().unwrap().len(),5,"Deleted prompts and queue controls remain durably acknowledged");
    assert_eq!(snapshot["events"].as_array().unwrap().iter().filter(|event| event["origin"]["requestId"] == "first").count(),1);
    for (request,text) in [("first","First task"),("second","Second task"),("deleted","Do not run this")] {
        assert_eq!(client.request(json!({"id":request,"type":"prompt","sessionId":id,"text":text})).await["ok"],true);
    }
    assert_eq!(client.request(json!({"id":"edit","type":"queue_control","sessionId":id,"generation":original_generation,"operation":{"type":"edit","requestId":"second","revision":0,"text":"Edited second task"}})).await["ok"],true,"A lost control response must reconcile before stale-generation/revision checks");
    assert!(requests.try_recv().is_err(),"Restart/retry must not start a provider turn");
    client.request(json!({"id":"resume","type":"queue_control","sessionId":id,"generation":snapshot["generation"],"operation":{"type":"resume","runId":null}})).await;
    let request = tokio::time::timeout(Duration::from_secs(10),requests.recv()).await.unwrap().unwrap();
    let messages = request["messages"].as_array().unwrap();
    assert_eq!(messages.iter().filter(|m| m["content"] == "First task").count(),1);
    assert_eq!(messages.last().unwrap()["content"],"Edited second task");
    assert!(!request.to_string().contains("Do not run this"));
    client.until(|event| event["type"] == "session_state" && event["status"] == "idle").await;
    let snapshot = client.open(&id).await;
    assert!(snapshot["queue"]["requests"].as_array().unwrap().is_empty());
    // A command can call a billed provider before its final acknowledgement.
    // Reserve its ID first; a killed compaction must not run again on a retry.
    client.socket.send(Message::Text(json!({"id":"compact","type":"prompt","sessionId":id,"text":"/compact"}).to_string().into())).await.unwrap();
    tokio::time::timeout(Duration::from_secs(10),requests.recv()).await.unwrap().unwrap();
    daemon.kill().await.unwrap(); daemon.wait().await.unwrap(); drop(client);
    let (mut daemon,mut client) = boot(root.path()).await;
    client.open(&id).await;
    let retry = client.request(json!({"id":"compact","type":"prompt","sessionId":id,"text":"/compact"})).await;
    assert_eq!(retry["ok"],false); assert!(retry["error"].as_str().unwrap().contains("interrupted"));
    assert!(requests.try_recv().is_err(),"An interrupted command receipt must prevent re-execution");
    let db = rusqlite::Connection::open(root.path().join("tau.sqlite3")).unwrap();
    assert_eq!(db.query_row("PRAGMA integrity_check",[],|row| row.get::<_,String>(0)).unwrap(),"ok");
    assert!(!db.prepare("PRAGMA foreign_key_check").unwrap().exists([]).unwrap());
    assert_eq!(std::fs::metadata(root.path().join("tau.sqlite3")).unwrap().permissions().mode() & 0o777,0o600);
    assert!(!root.path().join("state.json").exists());
    daemon.kill().await.unwrap(); daemon.wait().await.unwrap(); server.abort();
}
