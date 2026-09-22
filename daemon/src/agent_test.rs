use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;
use axum::{Router, Json, body::Body, extract::State, http::StatusCode, response::Response, routing::post};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::{Mutex, Notify, mpsc};
use tokio_tungstenite::{connect_async, tungstenite::{Message, client::IntoClientRequest}};
use crate::{Config, manager::AgentManager, settings::{Api, Settings}, state::StateStore};

struct Reply { status: u16, bytes: Vec<u8>, gate: Option<Arc<Notify>> }
struct ModelServer { url: String, requests: mpsc::UnboundedReceiver<Value>, task: tokio::task::JoinHandle<()> }
impl ModelServer {
    async fn start(replies: Vec<Reply>) -> Self {
        #[derive(Clone)] struct Script { replies: Arc<Mutex<VecDeque<Reply>>>, requests: mpsc::UnboundedSender<Value> }
        async fn respond(State(script): State<Script>, Json(body): Json<Value>) -> Response {
            script.requests.send(body).unwrap();
            let reply = script.replies.lock().await.pop_front().expect("Unexpected extra provider request");
            if let Some(gate) = reply.gate { gate.notified().await; }
            let fragments = reply.bytes.chunks(7).map(|bytes| Ok::<_, std::convert::Infallible>(bytes.to_vec())).collect::<Vec<_>>();
            Response::builder().status(StatusCode::from_u16(reply.status).unwrap()).header("content-type", "text/event-stream")
                .body(Body::from_stream(futures_util::stream::iter(fragments))).unwrap()
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (tx, requests) = mpsc::unbounded_channel();
        let app = Router::new().route("/{*path}", post(respond)).with_state(Script { replies:Arc::new(Mutex::new(replies.into())), requests:tx });
        Self { url, requests, task:tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); }) }
    }
    async fn request(&mut self) -> Value { tokio::time::timeout(Duration::from_secs(10), self.requests.recv()).await.unwrap().unwrap() }
}
impl Drop for ModelServer { fn drop(&mut self) { self.task.abort(); } }
fn completion(text: &str, calls: Vec<Value>) -> Reply {
    let mut delta = json!({"content":text});
    if !calls.is_empty() { delta["tool_calls"] = json!(calls.iter().enumerate().map(|(index, call)| { let mut call = call.clone(); call["index"] = json!(index); call }).collect::<Vec<_>>()); }
    Reply { status:200, gate:None, bytes:format!("data: {}\r\n\r\ndata: {}\r\n\r\ndata: [DONE]\r\n\r\n",
        json!({"choices":[{"index":0,"delta":delta}]}), json!({"choices":[{"index":0,"delta":{},"finish_reason":if calls.is_empty() {"stop"} else {"tool_calls"}}],"usage":{"total_tokens":1024}})).into_bytes() }
}
fn call(id: &str, name: &str, arguments: Value) -> Value { json!({"id":id,"type":"function","function":{"name":name,"arguments":arguments.to_string()}}) }
async fn fixture(model: &ModelServer, api: Api) -> (tempfile::TempDir, AgentManager, String, tokio::task::JoinHandle<()>) {
    let root = tempfile::tempdir().unwrap(); let root_path = root.path().to_owned();
    let config = Config { bind:"127.0.0.1:0".parse().unwrap(), transfer_bind:"127.0.0.1:0".parse().unwrap(), token:Arc::from("isolated-test-token"),
        settings_path:root_path.join("settings.json"), import_pi_dir:None, cwd:root_path.clone(), state_path:root_path.join("state.json"), session_dir:root_path.join("sessions"),
        telemetry_path:root_path.join("crashes.jsonl"), attachment_root:root_path.join("outbox"), upload_root:root_path.join("uploads"), title_command:Some("sleep 1; printf '{\"title\":\"Generated title\"}'".into()) };
    tokio::fs::create_dir_all(&config.attachment_root).await.unwrap();
    let mut settings = Settings::default();
    settings.agent.load_project_instructions = false; settings.agent.retry.base_delay_ms = 1; settings.daemon.idle_timeout_seconds = 0;
    let provider = settings.providers.get_mut("openai-codex").unwrap(); provider.api = api; provider.base_url = model.url.clone(); provider.web_search = false;
    tokio::fs::write(&config.settings_path, serde_json::to_vec(&settings).unwrap()).await.unwrap();
    let auth = if api == Api::Codex { json!({"type":"oauth","access":"fixture-access","refresh":"unused","accountId":"fixture-account","expires":u64::MAX}) } else { json!({"type":"api_key","key":"fixture-key"}) };
    tokio::fs::write(root_path.join("auth.json"), json!({"openai-codex":auth}).to_string()).await.unwrap();
    let manager = AgentManager::new(config, StateStore::load(root_path.join("state.json")).await.unwrap()).await.unwrap();
    let (url, task) = serve(&manager).await;
    (root, manager, url, task)
}
async fn serve(manager: &AgentManager) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap(); let url = format!("ws://{}/v1/ws", listener.local_addr().unwrap());
    let manager = manager.clone(); let config = manager.inner.config.clone();
    (url, tokio::spawn(async move { crate::server::serve(config, manager, listener).await.unwrap(); }))
}
struct Client { socket: tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, seen: Vec<Value> }
impl Client {
    async fn connect(url: &str) -> Self {
        let mut request = url.into_client_request().unwrap(); request.headers_mut().insert("authorization", "Bearer isolated-test-token".parse().unwrap());
        let (socket, _) = connect_async(request).await.unwrap();
        let mut client = Self { socket, seen:vec![] };
        let hello = client.until(|m| m["type"] == "hello").await;
        assert_eq!(hello["protocolVersion"], crate::protocol::PROTOCOL_VERSION);
        client
    }
    async fn until(&mut self, predicate: impl Fn(&Value) -> bool) -> Value {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                match self.socket.next().await.unwrap().unwrap() {
                    Message::Text(text) => { let message: Value = serde_json::from_str(&text).unwrap(); self.seen.push(message.clone()); if predicate(&message) { return message; } }
                    Message::Ping(_) => self.socket.flush().await.unwrap(),
                    other => panic!("Unexpected client message: {other:?}"),
                }
            }
        }).await.expect("Expected WebSocket event")
    }
    async fn request(&mut self, value: Value) -> Value {
        let id = value["id"].clone(); self.socket.send(Message::Text(value.to_string().into())).await.unwrap();
        self.until(|message| message["type"] == "response" && message["requestId"] == id).await
    }
    async fn open(&mut self, id: &str) -> Value {
        let request = format!("open-{}",uuid::Uuid::new_v4());
        assert_eq!(self.request(json!({"id":request,"type":"open_session","sessionId":id})).await["ok"], true);
        self.seen.iter().rev().find(|m| m["type"] == "transcript_snapshot" && m["sessionId"] == id).unwrap()["snapshot"].clone()
    }
}

#[tokio::test]
async fn websocket_acceptance_tools_queue_restart_and_settings_are_one_native_path() {
    let gate = Arc::new(Notify::new());
    let mut reply = completion("Working π🧠", vec![
        call("write", "write", json!({"path":"outbox/artifact.txt","content":"alpha\n"})),
        call("edit", "edit", json!({"path":"outbox/artifact.txt","edits":[{"oldText":"alpha","newText":"beta"}]})),
        call("shell", "bash", json!({"command":"printf x >> count; cat outbox/artifact.txt"})),
        call("read", "read", json!({"path":"outbox/artifact.txt"})),
        call("media", "send_file", json!({"path":"outbox/artifact.txt","caption":"Result"})),
        call("flag", "flag_it", json!({"str":"Fixture finding: missing optional documentation."})),
    ]); reply.gate = Some(gate.clone());
    let mut model = ModelServer::start(vec![reply, completion("Completed π🧠", vec![])]).await;
    let (root, manager, url, server) = fixture(&model, Api::ChatCompletions).await;
    let mut client = Client::connect(&url).await;
    let id = client.request(json!({"id":"create","type":"create_session"})).await["sessionId"].as_str().unwrap().to_owned();
    assert_eq!(client.request(json!({"id":"create2","type":"create_session"})).await["sessionId"], id);
    client.open(&id).await;
    let response = tokio::time::timeout(Duration::from_millis(700), client.request(json!({"id":"first","type":"prompt","sessionId":id,"text":"Make an artifact"}))).await.unwrap();
    assert_eq!(response["ok"], true); assert_eq!(response["uncertain"], false);
    let first = model.request().await;
    assert_eq!(first["messages"].as_array().unwrap().last().unwrap()["content"], "Make an artifact");
    for (request, text) in [("queued","Original queued text"),("deleted","Do not run me")] {
        assert_eq!(client.request(json!({"id":request,"type":"prompt","sessionId":id,"text":text})).await["disposition"], "queued");
    }
    let snapshot = client.open(&id).await; let generation = snapshot["generation"].clone(); let run = snapshot["queue"]["runId"].clone();
    for (id_cmd, operation) in [
        ("edit", json!({"type":"edit","requestId":"queued","revision":0,"text":"Edited queued text"})),
        ("delete", json!({"type":"delete","requestId":"deleted","revision":0})),
        ("pause", json!({"type":"pause","runId":run,"boundary":"turn"})),
    ] { assert_eq!(client.request(json!({"id":id_cmd,"type":"queue_control","sessionId":id,"generation":generation,"operation":operation})).await["ok"], true); }
    assert_eq!(client.request(json!({"id":"stale","type":"queue_control","sessionId":id,"generation":generation,"operation":{"type":"edit","requestId":"queued","revision":0,"text":"Stale"}})).await["ok"], false);
    // Full settings, title alias, stale revisions, and secrets stay on the same real wire.
    client.request(json!({"id":"get","type":"get_settings"})).await;
    let mut settings = client.seen.iter().rev().find(|m| m["type"] == "settings").unwrap()["settings"].clone();
    settings["agent"]["systemPrompt"] = json!("Custom system prompt"); settings["daemon"]["titlePrompt"] = json!("Custom title {text}");
    assert_eq!(client.request(json!({"id":"save","type":"set_settings","revision":0,"settings":settings})).await["ok"], true);
    assert_eq!(client.request(json!({"id":"stale-settings","type":"set_settings","revision":0,"settings":settings})).await["ok"], false);
    client.request(json!({"id":"title","type":"get_title_prompt"})).await;
    assert_eq!(client.seen.iter().rev().find(|m| m["type"] == "title_prompt").unwrap()["prompt"], "Custom title {text}");
    gate.notify_one();
    client.until(|m| m["type"] == "session_state" && m["status"] == "idle" && m["sessionId"] == id).await;
    assert_eq!(tokio::fs::read(root.path().join("outbox/artifact.txt")).await.unwrap(), b"beta\n");
    assert_eq!(tokio::fs::read(root.path().join("count")).await.unwrap(), b"x");
    let flags = tokio::fs::read_to_string(root.path().join("flags.jsonl")).await.unwrap();
    assert_eq!(flags.lines().count(), 1);
    let snapshot = client.open(&id).await;
    let attachment = snapshot["events"].as_array().unwrap().iter().find(|event| event["attachment"].is_object()).unwrap();
    let resolved = manager.resolve_attachment(&id, attachment["entryId"].as_str().unwrap()).await.unwrap();
    assert_eq!(resolved.size, 5);
    assert!(!client.seen.iter().any(|m| m.to_string().contains("fixture-key")));
    let config = manager.inner.config.clone();
    client.socket.close(None).await.unwrap(); manager.shutdown().await; server.abort();
    let manager = AgentManager::new(config.clone(), StateStore::load(config.state_path.clone()).await.unwrap()).await.unwrap();
    let (url, server) = serve(&manager).await; let mut client = Client::connect(&url).await;
    let snapshot = client.open(&id).await;
    assert_eq!(snapshot["queue"]["paused"], true); assert_eq!(snapshot["queue"]["requests"][0]["revision"], 1);
    assert_eq!(client.request(json!({"id":"queued","type":"prompt","sessionId":id,"text":"Original queued text"})).await["ok"], true);
    assert_eq!(client.request(json!({"id":"first","type":"prompt","sessionId":id,"text":"Changed text"})).await["ok"], false);
    let resumed = client.request(json!({"id":"resume","type":"queue_control","sessionId":id,"generation":snapshot["generation"],"operation":{"type":"resume","runId":null}})).await;
    assert_eq!(resumed["ok"], true);
    let second = model.request().await;
    assert_eq!(second["messages"][0]["content"].as_str().unwrap().starts_with("Custom system prompt"), true);
    assert_eq!(second["messages"].as_array().unwrap().last().unwrap()["content"], "Edited queued text");
    assert!(!second.to_string().contains("Do not run me"));
    assert!(second["messages"].as_array().unwrap().iter().any(|m| m["role"] == "tool" && m["content"] == "beta\n"));
    client.until(|m| m["type"] == "session_state" && m["status"] == "idle").await;
    let snapshot = client.open(&id).await;
    assert!(snapshot["events"].as_array().unwrap().iter().any(|event| event["text"] == "Completed π🧠" && event["phase"] == "saved"));
    assert_eq!(snapshot["queue"]["available"], true);
    assert!(snapshot["queue"]["requests"].as_array().unwrap().is_empty());
    assert_eq!(tokio::fs::read(root.path().join("count")).await.unwrap(), b"x", "Tool must execute exactly once across recovery");
    let fork_at = snapshot["events"].as_array().unwrap().iter().find(|event| event["role"] == "user").unwrap()["entryId"].clone();
    let fork = client.request(json!({"id":"fork","type":"fork_session","sessionId":id,"entryId":fork_at})).await;
    assert_eq!(fork["draft"], "Make an artifact");
    assert!(client.open(fork["sessionId"].as_str().unwrap()).await["events"].as_array().unwrap().is_empty());
    let clone = client.request(json!({"id":"clone","type":"clone_session","sessionId":id})).await;
    assert_eq!(client.open(clone["sessionId"].as_str().unwrap()).await["events"].as_array().unwrap().len(), snapshot["events"].as_array().unwrap().len());
    manager.shutdown().await; server.abort();
}
