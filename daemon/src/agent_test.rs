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
    crate::settings::atomic_write(&root_path.join("auth.json"), json!({"openai-codex":auth}).to_string().as_bytes()).await.unwrap();
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
        call("image-read", "read", json!({"path":"outbox/pixel.png"})),
        call("image-send", "send_image", json!({"path":"outbox/pixel.png"})),
        call("escape", "send_file", json!({"path":"outside.txt"})),
        call("write-ambiguous", "write", json!({"path":"ambiguous.txt","content":"aaa"})),
        call("edit-ambiguous", "edit", json!({"path":"ambiguous.txt","edits":[{"oldText":"aa","newText":"incorrect"}]})),
        call("media", "send_file", json!({"path":"outbox/artifact.txt","caption":"Result"})),
        call("flag", "flag_it", json!({"str":"Fixture finding: missing optional documentation."})),
    ]); reply.gate = Some(gate.clone());
    let mut model = ModelServer::start(vec![reply, completion("Completed π🧠", vec![])]).await;
    let (root, manager, url, server) = fixture(&model, Api::ChatCompletions).await;
    use base64::Engine as _;
    let mut image = base64::engine::general_purpose::STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLbtAAAAABJRU5ErkJggg==").unwrap();
    // Large opaque image input must not consume its base64 byte length as context tokens.
    image.resize(1024 * 1024, 0);
    tokio::fs::write(root.path().join("outbox/pixel.png"), image).await.unwrap();
    tokio::fs::write(root.path().join("outside.txt"), "Not in the outbox").await.unwrap();
    let mut client = Client::connect(&url).await;
    let id = client.request(json!({"id":"create","type":"create_session"})).await["sessionId"].as_str().unwrap().to_owned();
    assert_eq!(client.request(json!({"id":"create2","type":"create_session"})).await["sessionId"], id);
    client.open(&id).await;
    let journal = manager.inner.state.get(&id).unwrap().session_file.unwrap();
    let backup = format!("{journal}.backup");
    tokio::fs::rename(&journal, &backup).await.unwrap(); tokio::fs::create_dir(&journal).await.unwrap();
    assert_eq!(client.request(json!({"id":"first","type":"prompt","sessionId":id,"text":"Make an artifact"})).await["ok"], false, "Failed persistence must not acknowledge or run a prompt");
    assert!(client.open(&id).await["queue"]["requests"].as_array().unwrap().is_empty());
    tokio::fs::remove_dir(&journal).await.unwrap(); tokio::fs::rename(&backup, &journal).await.unwrap();
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
    assert_eq!(client.request(json!({"id":"unsupported-boundary","type":"queue_control","sessionId":id,"generation":generation,"operation":{"type":"pause","runId":run,"boundary":"reasoning_checkpoint"}})).await["ok"], false);
    assert_eq!(client.request(json!({"id":"cancel-pause","type":"queue_control","sessionId":id,"generation":generation,"operation":{"type":"cancel","controlId":"pause"}})).await["ok"], true);
    assert_eq!(client.request(json!({"id":"prefix","type":"queue_control","sessionId":id,"generation":generation,"operation":{"type":"prefix","runId":run,"boundary":"turn","requests":[{"requestId":"queued","revision":1}]}})).await["ok"], true);
    assert_eq!(client.request(json!({"id":"invalidating-edit","type":"queue_control","sessionId":id,"generation":generation,"operation":{"type":"edit","requestId":"queued","revision":1,"text":"Would invalidate the prefix"}})).await["ok"], false);
    assert_eq!(client.request(json!({"id":"cancel-prefix","type":"queue_control","sessionId":id,"generation":generation,"operation":{"type":"cancel","controlId":"prefix"}})).await["ok"], true);
    assert_eq!(client.request(json!({"id":"pause-again","type":"queue_control","sessionId":id,"generation":generation,"operation":{"type":"pause","runId":run,"boundary":"turn"}})).await["ok"], true);
    gate.notify_one();
    client.until(|m| m["type"] == "session_state" && m["status"] == "idle" && m["sessionId"] == id).await;
    assert_eq!(tokio::fs::read(root.path().join("outbox/artifact.txt")).await.unwrap(), b"beta\n");
    assert_eq!(tokio::fs::read(root.path().join("count")).await.unwrap(), b"x");
    let flags = tokio::fs::read_to_string(root.path().join("flags.jsonl")).await.unwrap();
    assert_eq!(flags.lines().count(), 1);
    let snapshot = client.open(&id).await;
    let attachment = snapshot["events"].as_array().unwrap().iter().find(|event| event["attachment"]["fileName"] == "artifact.txt").unwrap();
    let resolved = manager.resolve_attachment(&id, attachment["entryId"].as_str().unwrap()).await.unwrap();
    assert_eq!(resolved.size, 5);
    assert_eq!(snapshot["events"].as_array().unwrap().iter().filter(|event| event["attachment"].is_object()).count(), 2);
    assert!(snapshot["events"].as_array().unwrap().iter().any(|event| event["toolName"] == "send_file" && event["isError"] == true));
    assert_eq!(tokio::fs::read_to_string(root.path().join("ambiguous.txt")).await.unwrap(), "aaa");
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
    assert!(second["messages"][0]["content"].as_str().unwrap().starts_with("Custom system prompt"));
    assert_eq!(second["messages"].as_array().unwrap().last().unwrap()["content"], "Edited queued text");
    assert!(!second.to_string().contains("Do not run me"));
    assert!(second["messages"].as_array().unwrap().iter().any(|message| message["content"].as_array().is_some_and(|parts| parts.iter().any(|part| part["type"] == "image_url"))));
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

fn codex(text: &str, extra: Vec<Value>) -> Reply {
    let mut output = extra;
    if !text.is_empty() { output.push(json!({"type":"message","id":"msg_fixture","role":"assistant","status":"completed","content":[{"type":"output_text","text":text,"annotations":[]}]})); }
    let mut frames = format!("data: {}\n\ndata: {}\n\n", json!({"type":"response.reasoning_summary_text.delta","delta":"Thinking π🧠"}), json!({"type":"response.output_text.delta","delta":text}));
    for (index, item) in output.iter().enumerate() { frames.push_str(&format!("data: {}\n\n", json!({"type":"response.output_item.done","output_index":index,"item":item}))); }
    // Codex often omits output in the terminal event. Replay must use completed items.
    frames.push_str(&format!("data: {}\n\n", json!({"type":"response.completed","response":{"status":"completed","usage":{"input_tokens":100,"output_tokens":20,"total_tokens":120}}})));
    Reply { status:200, bytes:frames.into_bytes(), gate:None }
}

#[tokio::test]
async fn codex_replays_encrypted_reasoning_and_native_compaction_without_exposing_it_to_clients() {
    let reasoning = json!({"type":"reasoning","id":"rs_fixture","encrypted_content":"private-reasoning-cipher","summary":[{"type":"summary_text","text":"Thinking π🧠"}]});
    let checkpoint = json!({"type":"compaction","encrypted_content":"private-compaction-cipher"});
    let mut model = ModelServer::start(vec![codex("First answer",vec![reasoning]),codex("Second answer",vec![]),codex("",vec![checkpoint]),codex("Third answer",vec![])]).await;
    let (root, manager, url, server) = fixture(&model, Api::Codex).await;
    let mut client = Client::connect(&url).await;
    let id = client.request(json!({"id":"create","type":"create_session"})).await["sessionId"].as_str().unwrap().to_owned();
    client.open(&id).await;
    for (request, text) in [("one","First task"),("two","Second task")] {
        assert_eq!(client.request(json!({"id":request,"type":"prompt","sessionId":id,"text":text})).await["ok"], true);
        let payload = model.request().await;
        assert_eq!(payload["model"], "gpt-6-astra"); assert_eq!(payload["reasoning"]["effort"], "max");
        assert_eq!(payload["store"], false);
        if request == "two" { assert!(payload["input"].to_string().contains("private-reasoning-cipher")); }
        client.until(|m| m["type"] == "session_state" && m["status"] == "idle").await;
    }
    let before = client.open(&id).await;
    let compact = client.request(json!({"id":"compact","type":"prompt","sessionId":id,"text":"/compact Keep the task goals"})).await;
    assert_eq!(compact["ok"], true, "{compact}");
    assert_eq!(client.request(json!({"id":"compact","type":"prompt","sessionId":id,"text":"/compact Keep the task goals"})).await["disposition"], "handled");
    let payload = model.request().await;
    assert_eq!(payload["input"].as_array().unwrap().last().unwrap()["type"], "compaction_trigger");
    assert!(payload["input"].to_string().contains("First task"));
    assert!(!payload["input"].to_string().contains("Second task"));
    manager.close_session(&id).await.unwrap();
    let snapshot = client.open(&id).await;
    assert_eq!(snapshot["events"].as_array().unwrap().len(), before["events"].as_array().unwrap().len()+1);
    client.request(json!({"id":"three","type":"prompt","sessionId":id,"text":"Third task"})).await;
    let payload = model.request().await;
    assert_eq!(payload["input"][0]["type"], "compaction");
    assert!(payload["input"].to_string().contains("private-compaction-cipher"));
    assert!(payload["input"].to_string().contains("Second task"));
    assert!(!payload["input"].to_string().contains("First task"));
    client.until(|m| m["type"] == "session_state" && m["status"] == "idle").await;
    for message in &client.seen { assert!(!message.to_string().contains("private-")); }
    let file = manager.inner.state.get(&id).unwrap().session_file.unwrap();
    let journal = tokio::fs::read_to_string(file).await.unwrap();
    assert!(journal.contains("private-compaction-cipher") && journal.contains("private-reasoning-cipher"));
    assert!(tokio::fs::read_to_string(root.path().join("auth.json")).await.unwrap().contains("fixture-access"));
    manager.shutdown().await; server.abort();
}

#[cfg(unix)]
#[tokio::test]
async fn incomplete_stream_never_executes_tools_abort_kills_shell_group_and_retry_recovers() {
    let attempted = call("bad", "bash", json!({"command":"touch must-not-exist"}));
    let mut partial = completion("Incomplete", vec![attempted]);
    let end = partial.bytes.windows(b"data: [DONE]".len()).position(|part| part == b"data: [DONE]").unwrap(); partial.bytes.truncate(end);
    let shell = call("sleep", "bash", json!({"command":"echo $$ > shell.pid; sleep 60 & echo $! > child.pid; wait"}));
    let mut model = ModelServer::start(vec![partial,completion("",vec![shell]),Reply {status:429,bytes:vec![],gate:None},completion("Recovered",vec![])]).await;
    let (root, manager, url, server) = fixture(&model, Api::ChatCompletions).await;
    let mut client = Client::connect(&url).await;
    let id = client.request(json!({"id":"create","type":"create_session"})).await["sessionId"].as_str().unwrap().to_owned();
    client.open(&id).await;
    client.request(json!({"id":"bad-stream","type":"prompt","sessionId":id,"text":"First attempt"})).await;
    model.request().await;
    let error = client.until(|m| m["type"] == "session_state" && m["status"] == "error").await;
    assert!(error["detail"].as_str().unwrap().contains("incomplete"));
    assert!(!root.path().join("must-not-exist").exists());
    let snapshot = client.open(&id).await;
    assert!(snapshot["events"].as_array().unwrap().iter().any(|event| event["stopReason"] == "error"));
    client.request(json!({"id":"sleep","type":"prompt","sessionId":id,"text":"Run a shell"})).await;
    client.request(json!({"id":"resume","type":"queue_control","sessionId":id,"generation":snapshot["generation"],"operation":{"type":"resume","runId":null}})).await;
    let payload = model.request().await;
    assert!(!payload["messages"].to_string().contains("must-not-exist"));
    tokio::time::timeout(Duration::from_secs(5), async {
        while !root.path().join("child.pid").exists() { tokio::time::sleep(Duration::from_millis(10)).await; }
    }).await.unwrap();
    client.request(json!({"id":"abort","type":"abort","sessionId":id})).await;
    client.until(|m| m["type"] == "session_state" && m["status"] == "idle").await;
    for file in ["shell.pid","child.pid"] {
        let pid: i32 = tokio::fs::read_to_string(root.path().join(file)).await.unwrap().trim().parse().unwrap();
        // A killed grandchild can briefly remain a zombie until reaped by the host.
        let stat = tokio::fs::read_to_string(format!("/proc/{pid}/stat")).await.unwrap_or_default();
        assert!(stat.is_empty() || stat.split_whitespace().nth(2) == Some("Z"), "Shell descendant still running: {stat}");
    }
    let snapshot = client.open(&id).await;
    assert!(snapshot["queue"]["paused"].as_bool().unwrap());
    client.request(json!({"id":"retry","type":"prompt","sessionId":id,"text":"Continue"})).await;
    client.request(json!({"id":"resume2","type":"queue_control","sessionId":id,"generation":snapshot["generation"],"operation":{"type":"resume","runId":null}})).await;
    let first = model.request().await; let retried = model.request().await;
    assert_eq!(first, retried, "Retry must retain the exact conversation");
    client.until(|m| m["type"] == "session_state" && m["status"] == "idle").await;
    assert!(client.open(&id).await["events"].as_array().unwrap().iter().any(|e| e["text"] == "Recovered"));
    manager.shutdown().await; server.abort();
}

#[tokio::test]
async fn imports_settings_and_active_pi_history_then_recovers_a_torn_tail_without_rerunning_tools() {
    let mut model = ModelServer::start(vec![codex("Migrated safely",vec![])]).await;
    let (root, manager, _, server) = fixture(&model, Api::Codex).await;
    let mut config = manager.inner.config.clone(); manager.shutdown().await; server.abort();
    tokio::fs::remove_file(&config.settings_path).await.unwrap();
    tokio::fs::remove_file(root.path().join("auth.json")).await.unwrap();
    let pi = root.path().join("legacy-agent"); tokio::fs::create_dir_all(&pi).await.unwrap();
    let project = root.path().join(".pi"); tokio::fs::create_dir_all(&project).await.unwrap();
    tokio::fs::write(pi.join("settings.json"),json!({"defaultProvider":"openai-codex","defaultModel":"gpt-6-astra","defaultThinkingLevel":"high","steeringMode":"one-at-a-time","compaction":{"enabled":false},"retry":{"maxRetries":2,"provider":{"maxRetries":1}}}).to_string()).await.unwrap();
    tokio::fs::write(pi.join("models-store.json"),json!({"openai-codex":{"models":[{"id":"gpt-6-astra","name":"Fixture Codex","contextWindow":272000,"thinkingLevelMap":{"max":"max","high":"high"}}]}}).to_string()).await.unwrap();
    tokio::fs::write(pi.join("SYSTEM.md"), "Global replacement").await.unwrap();
    tokio::fs::write(pi.join("APPEND_SYSTEM.md"), "Global append").await.unwrap();
    tokio::fs::write(project.join("SYSTEM.md"), "Project replacement").await.unwrap();
    tokio::fs::write(project.join("APPEND_SYSTEM.md"), "Project append").await.unwrap();
    tokio::fs::write(root.path().join("AGENTS.md"), "Project instructions").await.unwrap();
    tokio::fs::write(pi.join("codex-fast-mode.json"), "{\"version\":1,\"enabled\":true}").await.unwrap();
    tokio::fs::write(pi.join("codex-compaction.json"), "{\"providers\":{\"openai-codex\":\"native\"}}").await.unwrap();
    let legacy_auth = json!({"openai-codex":{"type":"oauth","access":"fixture-access","refresh":"unused","accountId":"fixture-account","expires":u64::MAX}}).to_string();
    tokio::fs::write(pi.join("auth.json"), &legacy_auth).await.unwrap();
    config.import_pi_dir = Some(pi.clone());
    tokio::fs::create_dir_all(&config.session_dir).await.unwrap();
    let path = config.session_dir.join("legacy.jsonl");
    let saved = concat!(
        "{\"type\":\"session\",\"version\":3}\n",
        "{\"id\":\"root\",\"type\":\"model_change\",\"provider\":\"openai-codex\",\"modelId\":\"gpt-6-astra\",\"parentId\":null}\n",
        "{\"id\":\"user\",\"type\":\"message\",\"parentId\":\"root\",\"message\":{\"role\":\"user\",\"content\":\"Legacy task\"}}\n",
        "{\"id\":\"tool\",\"type\":\"message\",\"parentId\":\"user\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"toolCall\",\"id\":\"call_old|fc_old\",\"name\":\"bash\",\"arguments\":{\"command\":\"touch must-not-rerun\"}}]}}\n",
        "malformed middle line\n",
        "{\"id\":\"abandoned\",\"type\":\"message\",\"parentId\":\"root\",\"message\":{\"role\":\"user\",\"content\":\"Abandoned task\"}}\n",
        "{\"id\":\"active\",\"type\":\"thinking_level_change\",\"parentId\":\"tool\",\"thinkingLevel\":\"max\"}\n"
    );
    tokio::fs::write(&path, format!("{saved}{{\"torn\":" )).await.unwrap();
    let state = StateStore::load(config.state_path.clone()).await.unwrap();
    let id = state.create("Legacy chat".into(),None,Some(path.to_string_lossy().into_owned()),None,false).await.unwrap();
    let mut state_json: Value = serde_json::from_slice(&tokio::fs::read(&config.state_path).await.unwrap()).unwrap();
    state_json["title_prompt"] = json!("Legacy title template"); tokio::fs::write(&config.state_path,state_json.to_string()).await.unwrap();
    let manager = AgentManager::new(config.clone(),StateStore::load(config.state_path.clone()).await.unwrap()).await.unwrap();
    let mut settings = manager.inner.settings.get();
    assert_eq!(settings.daemon.title_prompt, "Legacy title template"); assert!(settings.agent.fast_mode && settings.agent.compaction.native_codex);
    assert_eq!(settings.agent.thinking_level, "high"); assert_eq!(settings.agent.retry.max_retries,2);
    assert_eq!(settings.agent.system_prompt.as_deref(), Some("Global replacement"));
    settings.providers.get_mut("openai-codex").unwrap().base_url = model.url.clone();
    manager.set_settings(settings.revision,settings).await.unwrap();
    let (url,server) = serve(&manager).await; let mut client = Client::connect(&url).await;
    let snapshot = client.open(&id).await;
    assert_eq!(snapshot["events"].as_array().unwrap().len(),2);
    assert_eq!(tokio::fs::read_to_string(&path).await.unwrap(), saved, "Only the torn tail is repaired");
    assert_eq!(snapshot["queue"]["paused"], true, "A saved unfinished turn needs an explicit recovery action");
    assert_eq!(client.request(json!({"id":"continue","type":"prompt","sessionId":id,"text":"Continue carefully"})).await["disposition"], "queued");
    client.request(json!({"id":"resume-migrated","type":"queue_control","sessionId":id,"generation":snapshot["generation"],"operation":{"type":"resume","runId":null}})).await;
    let payload = model.request().await;
    assert_eq!(payload["reasoning"]["effort"],"max", "Saved chat thinking takes precedence over the default");
    assert_eq!(payload["service_tier"],"priority");
    assert!(payload["instructions"].as_str().unwrap().starts_with("Project replacement"));
    for text in ["Global append","Project append","Project instructions"] { assert!(payload["instructions"].as_str().unwrap().contains(text)); }
    assert!(!payload.to_string().contains("Abandoned task"));
    assert!(payload["input"].as_array().unwrap().iter().any(|item| item["type"] == "function_call_output" && item["call_id"] == "call_old" && item["output"].as_str().unwrap().contains("effects are unknown")));
    client.until(|message| message["type"] == "session_state" && message["status"] == "idle").await;
    assert!(!root.path().join("must-not-rerun").exists());
    assert_eq!(tokio::fs::read_to_string(pi.join("auth.json")).await.unwrap(),legacy_auth);
    manager.shutdown().await; server.abort();
    tokio::fs::write(pi.join("settings.json"),"deliberately invalid after import").await.unwrap();
    let restarted = AgentManager::new(config.clone(),StateStore::load(config.state_path.clone()).await.unwrap()).await.unwrap();
    assert_eq!(restarted.inner.settings.get().revision,1); restarted.shutdown().await;
}
