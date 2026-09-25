use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;
use axum::{Router, Json, body::Body, extract::State, http::{StatusCode, HeaderMap, Uri}, response::{IntoResponse, Response}, routing::post};
use futures_util::StreamExt;
use serde_json::{Value, json};
use tokio::sync::{Mutex, Notify, mpsc};
use crate::{Config, manager::AgentManager, settings::{Api, Settings}, state::StateStore};

struct Reply { status: u16, bytes: Vec<u8>, gate: Option<Arc<Notify>>, body_gate: Option<(usize, Arc<Notify>)> }
struct ModelServer { url: String, requests: mpsc::UnboundedReceiver<Value>, catalogs: mpsc::UnboundedReceiver<Value>, catalog: Arc<Mutex<Option<Value>>>, task: tokio::task::JoinHandle<()> }
impl ModelServer {
    async fn start(replies: Vec<Reply>) -> Self { Self::with_catalog(replies, None).await }
    async fn with_catalog(replies: Vec<Reply>, catalog: Option<Value>) -> Self {
        #[derive(Clone)] struct Script { replies: Arc<Mutex<VecDeque<Reply>>>, requests: mpsc::UnboundedSender<Value>, catalog:Arc<Mutex<Option<Value>>>, catalogs:mpsc::UnboundedSender<Value> }
        async fn respond(State(script): State<Script>, Json(body): Json<Value>) -> Response {
            script.requests.send(body).unwrap();
            let reply = script.replies.lock().await.pop_front().expect("Unexpected extra provider request");
            if let Some(gate) = reply.gate { gate.notified().await; }
            let fragments = reply.bytes.chunks(7).map(|bytes| Ok::<_, std::convert::Infallible>(bytes.to_vec())).collect::<Vec<_>>();
            let gated = reply.body_gate.map(|(offset,gate)| (offset.div_ceil(7),gate));
            let body = futures_util::stream::iter(fragments).enumerate().then(move |(index,fragment)| {
                let gate = gated.as_ref().filter(|(at,_)| *at == index).map(|(_,gate)| gate.clone());
                async move { if let Some(gate) = gate { gate.notified().await; } fragment }
            });
            Response::builder().status(StatusCode::from_u16(reply.status).unwrap()).header("content-type", "text/event-stream")
                .body(Body::from_stream(body)).unwrap()
        }
        async fn reply_catalog(State(script): State<Script>, uri: Uri, headers: HeaderMap) -> Response {
            script.catalogs.send(json!({"path":uri.path(), "query":uri.query(),
                "authorization":headers.get("authorization").and_then(|v|v.to_str().ok()),
                "account":headers.get("chatgpt-account-id").and_then(|v|v.to_str().ok()),
                "originator":headers.get("originator").and_then(|v|v.to_str().ok())})).unwrap();
            match script.catalog.lock().await.clone() { Some(data) => Json(data).into_response(), None => StatusCode::NOT_FOUND.into_response() }
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (tx, requests) = mpsc::unbounded_channel();
        let (catalog_tx, catalogs) = mpsc::unbounded_channel();
        let catalog = Arc::new(Mutex::new(catalog));
        let app = Router::new().route("/{*path}", post(respond).get(reply_catalog)).with_state(Script { replies:Arc::new(Mutex::new(replies.into())), requests:tx, catalog:catalog.clone(), catalogs:catalog_tx });
        Self { url, requests, catalogs, catalog, task:tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); }) }
    }
    async fn request(&mut self) -> Value { tokio::time::timeout(Duration::from_secs(10), self.requests.recv()).await.unwrap().unwrap() }
    async fn catalog_request(&mut self) -> Value { tokio::time::timeout(Duration::from_secs(10), self.catalogs.recv()).await.unwrap().unwrap() }
    async fn set_catalog(&self, value: Option<Value>) { *self.catalog.lock().await = value; }
}
impl Drop for ModelServer { fn drop(&mut self) { self.task.abort(); } }
fn completion(text: &str, calls: Vec<Value>) -> Reply {
    let mut delta = json!({"content":text});
    if !calls.is_empty() { delta["tool_calls"] = json!(calls.iter().enumerate().map(|(index, call)| { let mut call = call.clone(); call["index"] = json!(index); call }).collect::<Vec<_>>()); }
    Reply { status:200, gate:None, body_gate:None, bytes:format!("data: {}\r\n\r\ndata: {}\r\n\r\ndata: [DONE]\r\n\r\n",
        json!({"choices":[{"index":0,"delta":delta}]}), json!({"choices":[{"index":0,"delta":{},"finish_reason":if calls.is_empty() {"stop"} else {"tool_calls"}}],"usage":{"total_tokens":1024}})).into_bytes() }
}
fn call(id: &str, name: &str, arguments: Value) -> Value { json!({"id":id,"type":"function","function":{"name":name,"arguments":arguments.to_string()}}) }
async fn fixture(model: &ModelServer, api: Api) -> (tempfile::TempDir, AgentManager, String, tokio::task::JoinHandle<()>) {
    let root = tempfile::tempdir().unwrap(); let root_path = root.path().to_owned();
    let config = Config { bind:"127.0.0.1:0".parse().unwrap(), transfer_bind:"127.0.0.1:0".parse().unwrap(), token:Arc::from("isolated-test-token"),
        settings_path:root_path.join("settings.json"), import_pi_dir:None, codex_auth_source:None, cwd:root_path.clone(), database_path:root_path.join("tau.sqlite3"),
        telemetry_path:root_path.join("crashes.jsonl"), attachment_root:root_path.join("outbox"), upload_root:root_path.join("uploads") };
    tokio::fs::create_dir_all(&config.attachment_root).await.unwrap();
    let mut settings = Settings::default();
    settings.agent.load_agents_files = false; settings.agent.retry.base_delay_ms = 1; settings.daemon.idle_timeout_seconds = 0;
    let provider = settings.providers.get_mut("openai-codex").unwrap(); provider.api = api; provider.base_url = model.url.clone(); provider.web_search = false;
    tokio::fs::write(&config.settings_path, serde_json::to_vec(&settings).unwrap()).await.unwrap();
    let auth = if api == Api::Codex { json!({"type":"oauth","access":"fixture-access","refresh":"unused","accountId":"fixture-account","expires":u64::MAX}) } else { json!({"type":"api_key","key":"fixture-key"}) };
    crate::settings::atomic_write(&root_path.join("auth.json"), json!({"openai-codex":auth}).to_string().as_bytes()).await.unwrap();
    let manager = AgentManager::new(config, StateStore::load(root_path.join("tau.sqlite3")).await.unwrap()).await.unwrap();
    let (url, task) = serve(&manager).await;
    (root, manager, url, task)
}
async fn serve(manager: &AgentManager) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap(); let url = format!("ws://{}/v1/ws", listener.local_addr().unwrap());
    let manager = manager.clone(); let config = manager.inner.config.clone();
    (url, tokio::spawn(async move { crate::server::serve(config, manager, listener).await.unwrap(); }))
}
#[path = "../tests/support/mod.rs"]
mod native_client;
use native_client::Client;

#[tokio::test]
async fn client_named_create_is_immediate_and_idempotent_after_a_lost_ack() {
    let model = ModelServer::start(vec![]).await;
    let (_root, manager, _, server) = fixture(&model, Api::Codex).await;
    let first = uuid::Uuid::new_v4().to_string();
    assert_eq!(manager.create_session_requested(None,"general",Some(&first)).await.unwrap(),first);
    // New client-named chats never alias an existing empty tile.
    let candidate = uuid::Uuid::new_v4().to_string();
    assert_eq!(manager.create_session_requested(None,"general",Some(&candidate)).await.unwrap(),candidate);
    manager.rename_session(&first,"Old chat is no longer a starter").await.unwrap();
    assert_eq!(manager.create_session_requested(None,"general",Some(&candidate)).await.unwrap(),candidate,
        "A lost create acknowledgement must still resolve to its original chat after that starter becomes active");
    let second = uuid::Uuid::new_v4().to_string();
    assert_eq!(manager.create_session_requested(Some(&first),"general",Some(&second)).await.unwrap(),second);
    manager.rename_session(&second,"Authored chat").await.unwrap();
    let count = manager.inner.state.list().await.unwrap().len();
    assert_eq!(manager.create_session_requested(Some(&first),"general",Some(&second)).await.unwrap(),second);
    let sessions = manager.inner.state.list().await.unwrap();
    assert_eq!(sessions.len(),count,"Retry cannot create or retire a second chat");
    assert_eq!(sessions.iter().find(|(id,_)| id == &second).unwrap().1.title,"Authored chat");
    // Integration with topics: keep identities and the captured prompt scoped to
    // the requested topic, including aliases of a reused starter after an edit.
    let topic = uuid::Uuid::new_v4().to_string();
    manager.create_project(topic.clone(), "Work".into(), "Original topic prompt".into()).await.unwrap();
    let work = uuid::Uuid::new_v4().to_string();
    assert_eq!(manager.create_session_requested(None,&topic,Some(&work)).await.unwrap(),work);
    let alias = uuid::Uuid::new_v4().to_string();
    assert_eq!(manager.create_session_requested(None,&topic,Some(&alias)).await.unwrap(),alias);
    manager.update_project(topic.clone(),0,"Work".into(),"Changed topic prompt".into()).await.unwrap();
    assert_eq!(manager.create_session_requested(None,&topic,Some(&alias)).await.unwrap(),alias);
    assert!(manager.create_session_requested(None,"general",Some(&alias)).await.is_err(), "A create ID cannot be reused in another topic");
    assert!(manager.create_session_requested(Some(&first),&topic,Some(&work)).await.is_err(), "A create ID cannot change its keep-chat intent");
    let stored = manager.inner.state.get(&work).await.unwrap().unwrap();
    assert_eq!(stored.project_id,topic);
    assert_eq!(stored.project_prompt,"Original topic prompt");
    let fresh = uuid::Uuid::new_v4().to_string();
    assert_eq!(manager.create_session_requested(None,&topic,Some(&fresh)).await.unwrap(),fresh);
    assert_eq!(manager.inner.state.get(&fresh).await.unwrap().unwrap().project_prompt,"Changed topic prompt");
    manager.shutdown().await; server.abort();
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
        call("image-send", "send_file", json!({"path":"outbox/pixel.png"})),
        call("staged", "send_file", json!({"path":"@outside.txt","caption":" Report "})),
        call("big-image", "send_file", json!({"path":"large.png"})),
        call("fake-image", "send_file", json!({"path":"misleading.png"})),
        call("directory", "send_file", json!({"path":"outbox"})),
        call("too-large", "send_file", json!({"path":"too-large.bin"})),
        call("caption", "send_file", json!({"path":"outside.txt","caption":"x".repeat(1025)})),
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
    tokio::fs::write(root.path().join("misleading.png"), "Not a PNG").await.unwrap();
    tokio::fs::write(root.path().join("large.png"), b"\x89PNG\r\n\x1a\n").await.unwrap();
    tokio::fs::OpenOptions::new().write(true).open(root.path().join("large.png")).await.unwrap().set_len(crate::transcript::IMAGE_LIMIT + 1).await.unwrap();
    tokio::fs::File::create(root.path().join("too-large.bin")).await.unwrap().set_len(crate::transcript::FILE_LIMIT + 1).await.unwrap();
    let mut client = Client::connect(&url).await;
    let id = client.request(json!({"id":"create","type":"create_session"})).await["sessionId"].as_str().unwrap().to_owned();
    assert_eq!(client.request(json!({"id":"create2","type":"create_session"})).await["sessionId"], id);
    client.open(&id).await;
    manager.inner.state.access(|db| { db.execute_batch("CREATE TRIGGER reject_queue BEFORE INSERT ON queue BEGIN SELECT RAISE(ABORT,'fixture disk failure'); END;")?; Ok(()) }).await.unwrap();
    assert_eq!(client.request(json!({"id":"first","type":"prompt","sessionId":id,"text":"Make an artifact"})).await["ok"], false, "Failed transaction must not acknowledge or run a prompt");
    assert!(client.open(&id).await["queue"]["requests"].as_array().unwrap().is_empty());
    assert!(manager.inner.state.receipt(&id,"first").await.unwrap().is_none());
    assert!(manager.inner.state.get(&id).await.unwrap().unwrap().starter, "Receipt, queue and retention must roll back together");
    manager.inner.state.access(|db| { db.execute_batch("DROP TRIGGER reject_queue")?; Ok(()) }).await.unwrap();
    let response = tokio::time::timeout(Duration::from_millis(700), client.request(json!({"id":"first","type":"prompt","sessionId":id,"text":"Make an artifact"}))).await.unwrap();
    assert_eq!(response["ok"], true); assert_eq!(response["uncertain"], false);
    let first = model.request().await;
    let definitions = first["tools"].as_array().unwrap();
    assert_eq!(definitions.iter().filter(|tool| tool["function"]["name"] == "send_file").count(), 1);
    assert!(!definitions.iter().any(|tool| tool["function"]["name"] == "send_image" || tool["type"] == "image_generation"));
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
    client.request(json!({"id":"edit-receipt","type":"get_receipts","sessionId":id,"requests":["edit"]})).await;
    let report=client.seen.iter().rev().find(|m|m["type"]=="receipts").unwrap();
    assert_eq!(report["reports"][0]["accepted"],true,"The durable edit receipt is independent of gated display/provider work");
    assert_eq!(client.request(json!({"id":"stale","type":"queue_control","sessionId":id,"generation":generation,"operation":{"type":"edit","requestId":"queued","revision":0,"text":"Stale"}})).await["ok"], false);
    // Full settings, stale revisions, and secrets stay on the same real wire.
    client.request(json!({"id":"get","type":"get_settings"})).await;
    let mut settings = client.seen.iter().rev().find(|m| m["type"] == "settings").unwrap()["settings"].clone();
    settings["agent"]["systemPrompt"] = json!("Custom system prompt"); settings["daemon"]["titlePrompt"] = json!("Custom title {text}");
    settings["daemon"]["idleTimeoutSeconds"] = json!(2);
    assert_eq!(client.request(json!({"id":"save","type":"set_settings","revision":0,"settings":settings})).await["ok"], true);
    assert_eq!(client.request(json!({"id":"stale-settings","type":"set_settings","revision":0,"settings":settings})).await["ok"], false);
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
    let events = snapshot["events"].as_array().unwrap();
    assert_eq!(events.iter().filter(|event| event["attachment"].is_object()).count(), 5);
    assert_eq!(events.iter().filter(|event| event["role"] == "tool" && event["toolName"] == "send_file" && event["isError"] == true).count(), 3);
    for (name, kind) in [("pixel.png","image"),("large.png","file"),("misleading.png","file")] {
        let attachment = events.iter().find(|event| event["attachment"]["fileName"] == name).unwrap();
        assert_eq!(attachment["attachment"]["kind"], kind);
        manager.resolve_attachment(&id, attachment["entryId"].as_str().unwrap()).await.unwrap();
    }
    // The native file primitive materializes only after an explicit body read.
    let native_file=crate::blocks::file_id(attachment["entryId"].as_str().unwrap());
    let scope=id.clone();let file_id=native_file.clone();
    let before=manager.inner.state.access(move |db|Ok(tau_blocks::header(db,&scope,&file_id)?.unwrap())).await.unwrap();
    assert!(!before.sealed);assert_eq!(before.length,0);
    use tau_transfer::blocks::Backend;
    let range=manager.read(tau_blocks::BlockRequest {scope:id.clone(),id:native_file.clone(),version:0,offset:0,follow:false}).await.unwrap();
    assert_eq!(range.bytes,b"beta\n");assert!(range.header.sealed);assert_eq!(range.header.version,before.version);
    assert!(range.header.meta["sha256"].is_string());
    let end=manager.read(tau_blocks::BlockRequest {scope:id.clone(),id:native_file,version:range.header.version,offset:5,follow:false}).await.unwrap();
    assert!(end.bytes.is_empty());
    let staged = events.iter().find(|event| event["attachment"]["fileName"] == "outside.txt").unwrap();
    assert_eq!(staged["attachment"]["caption"], "Report");
    tokio::fs::write(root.path().join("outside.txt"), "changed").await.unwrap();
    let resolved = manager.resolve_attachment(&id, staged["entryId"].as_str().unwrap()).await.unwrap();
    use tokio::io::AsyncReadExt;
    let mut contents = String::new(); let mut file = resolved.file;
    file.read_to_string(&mut contents).await.unwrap(); assert_eq!(contents, "Not in the outbox");
    #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; assert_eq!(file.metadata().await.unwrap().permissions().mode() & 0o777, 0o600); }
    assert_eq!(tokio::fs::read_to_string(root.path().join("ambiguous.txt")).await.unwrap(), "aaa");
    assert!(!client.seen.iter().any(|m| m.to_string().contains("fixture-key")));
    client.until(|m| m["type"] == "session_state" && m["sessionId"] == id && m["status"] == "sleeping").await;
    assert!(manager.inner.runtimes.lock().await[&id].content.lock().await.agent.is_none(), "Paused work must not pin the in-memory agent forever");
    let restored = client.open(&id).await;
    assert_eq!(restored["queue"]["paused"], true); assert_eq!(restored["queue"]["requests"][0]["text"], "Edited queued text");
    let config = manager.inner.config.clone();
    client.socket.close(None).await.unwrap(); manager.shutdown().await; server.abort();
    let manager = AgentManager::new(config.clone(), StateStore::load(config.database_path.clone()).await.unwrap()).await.unwrap();
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
    Reply { status:200, bytes:frames.into_bytes(), gate:None, body_gate:None }
}

#[tokio::test]
async fn codex_replays_encrypted_reasoning_and_native_compaction_without_exposing_it_to_clients() {
    let reasoning = json!({"type":"reasoning","id":"rs_fixture","encrypted_content":"private-reasoning-cipher","summary":[{"type":"summary_text","text":"Thinking π🧠"}]});
    let checkpoint = json!({"type":"compaction","encrypted_content":"private-compaction-cipher"});
    let gate = Arc::new(Notify::new()); let mut first = codex("First answer",vec![reasoning]);
    let prefix = first.bytes.windows(2).position(|bytes| bytes == b"\n\n").unwrap()+2;
    first.body_gate = Some((prefix,gate.clone()));
    let mut model = ModelServer::start(vec![first,codex("Second answer",vec![]),codex("",vec![checkpoint]),codex("Third answer",vec![])]).await;
    let (root, manager, url, server) = fixture(&model, Api::Codex).await;
    let mut client = Client::connect(&url).await;
    let id = client.request(json!({"id":"create","type":"create_session"})).await["sessionId"].as_str().unwrap().to_owned();
    client.open(&id).await;
    let mut thinking_started = Value::Null;
    for (request, text) in [("one","First task"),("two","Second task")] {
        assert_eq!(client.request(json!({"id":request,"type":"prompt","sessionId":id,"text":text})).await["ok"], true);
        let payload = model.request().await;
        assert_eq!(payload["model"], "gpt-6-astra"); assert_eq!(payload["reasoning"]["effort"], "max");
        assert_eq!(payload["store"], false);
        if request == "two" { assert!(payload["input"].to_string().contains("private-reasoning-cipher")); }
        else {
            thinking_started = tokio::time::timeout(Duration::from_secs(5),async {
                loop {
                    let snapshot=client.page(&id,None).await;
                    if let Some(thinking)=snapshot["events"].as_array().unwrap().iter().find(|e|e["kind"]=="thinking") {break thinking["timestampMs"].clone();}
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }).await.unwrap();
            tokio::time::sleep(Duration::from_millis(20)).await; gate.notify_one();
        }
        client.until(|m| m["type"] == "session_state" && m["status"] == "idle").await;
    }
    let before = client.open(&id).await;
    let answer = before["events"].as_array().unwrap().iter().find(|e| e["text"] == "First answer").unwrap();
    let thinking = before["events"].as_array().unwrap().iter().find(|e| e["entryId"] == answer["entryId"] && e["kind"] == "thinking").unwrap();
    assert_eq!(thinking["timestampMs"],thinking_started);
    assert!(answer["timestampMs"].as_u64().unwrap() > thinking_started.as_u64().unwrap(), "Sections record their own first stream observation, not a cloned save time");
    let compact = client.request(json!({"id":"compact","type":"prompt","sessionId":id,"text":"/compact Keep the task goals"})).await;
    assert_eq!(compact["ok"], true, "{compact}");
    let completed=|m:&Value|m["type"]=="receipts" && m["reports"].as_array().is_some_and(|rs|rs.iter().any(|r|r["id"]=="compact" && r["complete"]==true));
    if !client.seen.iter().any(completed) {client.until(completed).await;}
    assert_eq!(client.request(json!({"id":"compact","type":"prompt","sessionId":id,"text":"/compact Keep the task goals"})).await["disposition"], "handled");
    let payload = model.request().await;
    assert_eq!(payload["input"].as_array().unwrap().last().unwrap()["type"], "compaction_trigger");
    assert!(payload["input"].to_string().contains("First task"));
    assert!(!payload["input"].to_string().contains("Second task"));
    manager.close_session(&id).await.unwrap();
    let snapshot = client.open(&id).await;
    assert_eq!(snapshot["events"].as_array().unwrap().len(), before["events"].as_array().unwrap().len()+1);
    for event in [answer,thinking] { assert!(snapshot["events"].as_array().unwrap().contains(event), "Section times must survive reopening and compaction"); }
    client.request(json!({"id":"three","type":"prompt","sessionId":id,"text":"Third task"})).await;
    let payload = model.request().await;
    assert_eq!(payload["input"][0]["type"], "compaction");
    assert!(payload["input"].to_string().contains("private-compaction-cipher"));
    assert!(payload["input"].to_string().contains("Second task"));
    assert!(!payload["input"].to_string().contains("First task"));
    client.until(|m| m["type"] == "session_state" && m["status"] == "idle").await;
    for message in &client.seen { assert!(!message.to_string().contains("private-")); }
    let journal = manager.inner.state.context(&id,&manager.inner.settings.get().agent.model).await.unwrap().iter().map(Value::to_string).collect::<String>();
    assert!(journal.contains("private-compaction-cipher") && !journal.contains("private-reasoning-cipher"), "Only retained context is loaded");
    assert!(manager.inner.state.entry(&id,answer["entryId"].as_str().unwrap()).await.unwrap().to_string().contains("private-reasoning-cipher"), "Compaction must not erase saved history");
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
    let mut model = ModelServer::start(vec![partial,completion("",vec![shell]),Reply {status:429,bytes:vec![],gate:None, body_gate:None},completion("Recovered",vec![])]).await;
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
    let queued = client.request(json!({"id":"sleep","type":"prompt","sessionId":id,"text":"Run a shell"})).await;
    assert_eq!(queued["disposition"],"queued");
    let status = client.seen.iter().rev().find(|m|m["type"] == "session_state" && m["sessionId"] == id).unwrap();
    let status = if status["status"] == "idle" { status.clone() }
        else { client.until(|m|m["type"] == "session_state" && m["sessionId"] == id && m["status"] == "idle").await };
    assert!(status["detail"].as_str().unwrap().contains("paused"));
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
async fn imports_deployed_pi_history_read_only_into_sqlite_without_rerunning_tools() {
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
    let path = root.path().join("legacy.jsonl");
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
    let id = uuid::Uuid::new_v4().to_string();
    let legacy_path = root.path().join("old-state.json");
    let legacy_state = json!({"schema":1,"title_prompt":"Legacy title template","sessions":{&id:{"title":"Legacy chat","session_file":path,"created_at_ms":1,"updated_at_ms":1}}}).to_string();
    tokio::fs::write(&legacy_path,&legacy_state).await.unwrap();
    assert_eq!(crate::import_state(config.clone(),legacy_path.clone()).await.unwrap(),1);
    assert!(crate::import_state(config.clone(),legacy_path.clone()).await.is_err(), "Import must not overwrite a populated database");
    assert_eq!(tokio::fs::read_to_string(legacy_path).await.unwrap(),legacy_state);
    let manager = AgentManager::new(config.clone(),StateStore::load(config.database_path.clone()).await.unwrap()).await.unwrap();
    let mut settings = manager.inner.settings.get();
    assert_eq!(settings.daemon.title_prompt, "Legacy title template"); assert!(settings.agent.fast_mode && settings.agent.compaction.native_codex);
    assert_eq!(settings.agent.thinking_level, "high"); assert_eq!(settings.agent.retry.max_retries,2);
    assert_eq!(settings.agent.system_prompt, "Global replacement\n\nGlobal append");
    settings.providers.get_mut("openai-codex").unwrap().base_url = model.url.clone();
    manager.set_settings(settings.revision,settings).await.unwrap();
    let (url,server) = serve(&manager).await; let mut client = Client::connect(&url).await;
    let snapshot = client.open(&id).await;
    assert_eq!(snapshot["events"].as_array().unwrap().len(),2);
    assert_eq!(tokio::fs::read_to_string(&path).await.unwrap(), format!("{saved}{{\"torn\":"), "The legacy source is read-only, including its torn tail");
    assert_eq!(snapshot["queue"]["paused"], true, "A saved unfinished turn needs an explicit recovery action");
    assert_eq!(client.request(json!({"id":"continue","type":"prompt","sessionId":id,"text":"Continue carefully"})).await["disposition"], "queued");
    client.request(json!({"id":"resume-migrated","type":"queue_control","sessionId":id,"generation":snapshot["generation"],"operation":{"type":"resume","runId":null}})).await;
    let payload = model.request().await;
    assert_eq!(payload["reasoning"]["effort"],"max", "Saved chat thinking takes precedence over the default");
    assert_eq!(payload["service_tier"],"priority");
    assert!(payload["instructions"].as_str().unwrap().starts_with("Global replacement\n\nGlobal append"));
    assert!(payload["instructions"].as_str().unwrap().contains("AGENTS.md instructions"));
    for text in ["Project replacement", "Project append"] { assert!(!payload["instructions"].as_str().unwrap().contains(text)); }
    assert!(!payload.to_string().contains("Abandoned task"));
    assert!(payload["input"].as_array().unwrap().iter().any(|item| item["type"] == "function_call_output" && item["call_id"] == "call_old" && item["output"].as_str().unwrap().contains("effects are unknown")));
    client.until(|message| message["type"] == "session_state" && message["status"] == "idle").await;
    assert!(!root.path().join("must-not-rerun").exists());
    assert_eq!(tokio::fs::read_to_string(pi.join("auth.json")).await.unwrap(),legacy_auth);
    manager.shutdown().await; server.abort();
    tokio::fs::write(pi.join("settings.json"),"deliberately invalid after import").await.unwrap();
    let restarted = AgentManager::new(config.clone(),StateStore::load(config.database_path.clone()).await.unwrap()).await.unwrap();
    assert_eq!(restarted.inner.settings.get().revision,1); restarted.shutdown().await;
}

const GENERATED_PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLbtAAAAABJRU5ErkJggg==";
fn generated(id: &str) -> Value { json!({"type":"image_generation_call","id":id,"status":"completed","output_format":"png","result":GENERATED_PNG}) }

#[tokio::test]
async fn native_images_are_delivered_reopened_and_replayed_as_references_without_journaling_bytes() {
    let edit = json!({"type":"function_call","id":"fc_edit","call_id":"edit","name":"bash","arguments":"{\"command\":\"printf ok > edited\"}","status":"completed"});
    let mut model = ModelServer::start(vec![codex("",vec![generated("img_first")]),
        codex("Edited images",vec![generated("img_second"),generated("img_third"),edit]), codex("Finished",vec![]), codex("Missing reference acknowledged",vec![])]).await;
    let (root, manager, url, server) = fixture(&model, Api::Codex).await;
    let mut client = Client::connect(&url).await;
    let id = client.request(json!({"id":"create","type":"create_session"})).await["sessionId"].as_str().unwrap().to_owned();
    client.open(&id).await;
    assert_eq!(client.request(json!({"id":"generate","type":"prompt","sessionId":id,"text":"Generate an image"})).await["ok"], true);
    let request = model.request().await;
    assert!(request["tools"].as_array().unwrap().contains(&json!({"type":"image_generation","model":"gpt-image-2","output_format":"png"})));
    assert!(!request["tools"].to_string().contains("send_image"));
    client.until(|m| m["type"] == "session_state" && m["status"] == "idle").await;
    let before = client.open(&id).await;
    let image = before["events"].as_array().unwrap().iter().find(|event| event["attachment"]["kind"] == "image").unwrap().clone();
    assert_eq!(image["role"], "assistant"); assert_eq!(image["kind"], "image");
    assert_eq!(before["queue"]["paused"], false, "An image-only answer finishes normally");
    let download = format!("{}/v1/sessions/{id}/attachments/{}",url.trim_end_matches("/v1/ws").replace("ws://","http://"),image["entryId"].as_str().unwrap());
    let http = reqwest::Client::new();
    assert_eq!(http.get(&download).send().await.unwrap().status(), reqwest::StatusCode::NOT_FOUND);
    assert_eq!(http.get(&download).bearer_auth("isolated-test-token").send().await.unwrap().status(), reqwest::StatusCode::NOT_FOUND);
    use base64::Engine as _;
    let bytes=client.body(&id,&format!("file:{}",image["entryId"].as_str().unwrap())).await;
    assert_eq!(bytes,base64::engine::general_purpose::STANDARD.decode(GENERATED_PNG).unwrap());
    let journal = manager.inner.state.context(&id,&manager.inner.settings.get().agent.model).await.unwrap();
    assert!(!serde_json::to_string(&journal).unwrap().contains(GENERATED_PNG));
    assert!(!client.seen.iter().any(|message| message.to_string().contains(GENERATED_PNG)));
    let config = manager.inner.config.clone();
    client.socket.close(None).await.unwrap(); manager.shutdown().await; server.abort();

    let manager = AgentManager::new(config.clone(),StateStore::load(config.database_path.clone()).await.unwrap()).await.unwrap();
    let (url,server) = serve(&manager).await; let mut client = Client::connect(&url).await;
    let after = client.open(&id).await;
    assert!(after["events"].as_array().unwrap().contains(&image));
    assert_eq!(after["queue"]["paused"],false);
    assert_eq!(client.seen.iter().rev().find(|m| m["type"] == "session_state" && m["sessionId"] == id).unwrap()["contextUsage"]["tokens"],120);
    assert_eq!(client.request(json!({"id":"generate","type":"prompt","sessionId":id,"text":"Generate an image"})).await["ok"],true);
    assert!(model.requests.try_recv().is_err(),"Retrying an accepted prompt must not regenerate an image");
    client.request(json!({"id":"edit-image","type":"prompt","sessionId":id,"text":"Make two variations of that image"})).await;
    let request = model.request().await;
    let input = request["input"].as_array().unwrap();
    assert!(!request.to_string().contains("img_first"), "A store:false image ID cannot be replayed");
    let reference = input.iter().find(|item| item["content"].as_array().is_some_and(|parts| parts.iter().any(|p| p["type"] == "input_image"))).unwrap();
    assert!(reference["content"][0]["text"].as_str().unwrap().contains("not a new user request"));
    assert!(reference.to_string().contains(GENERATED_PNG));
    let continuation = model.request().await;
    let input = continuation["input"].as_array().unwrap();
    assert_eq!(input.iter().flat_map(|item| item["content"].as_array().into_iter().flatten()).filter(|part| part["type"] == "input_image").count(),3);
    let tool_index = input.iter().position(|item| item["type"] == "function_call").unwrap();
    let result_index = input.iter().position(|item| item["type"] == "function_call_output").unwrap();
    assert!(result_index > tool_index && !input[tool_index..result_index].iter().any(|item| item["role"] == "user"), "Image references must not separate a function call from its result");
    client.until(|m| m["type"] == "session_state" && m["status"] == "idle").await;
    assert_eq!(tokio::fs::read(root.path().join("edited")).await.unwrap(),b"ok");
    let snapshot = client.open(&id).await;
    let images = snapshot["events"].as_array().unwrap().iter().filter(|event| event["attachment"]["kind"] == "image").collect::<Vec<_>>();
    assert_eq!(images.len(),3);
    for image in &images { manager.resolve_attachment(&id,image["entryId"].as_str().unwrap()).await.unwrap(); }
    let fork = client.request(json!({"id":"fork-image","type":"fork_session","sessionId":id,"entryId":images[0]["entryId"]})).await;
    let fork_id = fork["sessionId"].as_str().unwrap();
    assert!(client.open(fork_id).await["events"].as_array().unwrap().contains(&image));
    manager.resolve_attachment(fork_id,image["entryId"].as_str().unwrap()).await.unwrap();
    for message in &client.seen { assert!(!message.to_string().contains(GENERATED_PNG)); }
    let history = manager.inner.state.context(&id,&manager.inner.settings.get().agent.model).await.unwrap();
    let encoded = serde_json::to_string(&history).unwrap();
    assert!(!encoded.contains(GENERATED_PNG) && !encoded.contains("image_generation_call"));
    // A deleted original is an explicit missing reference, not a silently repeated paid generation.
    let entry = history.iter().find(|entry| entry["type"] == "tau_attachment").unwrap();
    tokio::fs::remove_file(entry["message"]["details"]["tauAttachment"]["path"].as_str().unwrap()).await.unwrap();
    client.open(&id).await;
    client.request(json!({"id":"missing","type":"prompt","sessionId":id,"text":"Discuss the original"})).await;
    assert!(model.request().await.to_string().contains("no longer available"));
    client.until(|m| m["type"] == "session_state" && m["sessionId"] == id && m["status"] == "idle").await;
    manager.shutdown().await; server.abort();
}

#[tokio::test]
async fn failed_native_images_never_publish_bytes_or_retry_a_started_generation() {
    let mut interrupted = codex("",vec![generated("img_interrupted")]);
    let end = interrupted.bytes.windows(b"data: {\"response\"".len()).position(|part| part == b"data: {\"response\"").unwrap();
    interrupted.bytes.truncate(end);
    let mut malformed = generated("img_malformed"); malformed["result"] = json!("not-base64!");
    let mut unfinished = generated("img_unfinished"); unfinished["status"] = json!("in_progress");
    let mut wrong_format = generated("img_format"); wrong_format["output_format"] = json!("jpeg");
    let error = Reply {status:200,gate:None, body_gate:None,bytes:format!("data: {}\n\ndata: {}\n\n",
        json!({"type":"response.image_generation_call.in_progress"}), json!({"type":"error","error":{"code":"server_error"}})).into_bytes()};
    let replies = vec![interrupted,codex("",vec![generated("img_duplicate"),generated("img_duplicate")]),
        codex("",vec![generated("img_valid"),malformed]),codex("",vec![unfinished]),codex("",vec![wrong_format]),error];
    let count = replies.len(); let mut model = ModelServer::start(replies).await;
    let (root, manager, url, server) = fixture(&model,Api::Codex).await;
    let mut client = Client::connect(&url).await;
    for index in 0..count {
        let id = client.request(json!({"id":format!("create-{index}"),"type":"create_session"})).await["sessionId"].as_str().unwrap().to_owned();
        client.open(&id).await;
        client.request(json!({"id":format!("bad-{index}"),"type":"prompt","sessionId":id,"text":"Generate an image"})).await;
        model.request().await;
        client.until(|m| m["type"] == "session_state" && m["sessionId"] == id && m["status"] == "error").await;
        let snapshot = client.open(&id).await;
        assert_eq!(snapshot["queue"]["paused"],true);
        assert!(!snapshot["events"].as_array().unwrap().iter().any(|event| event["attachment"].is_object()));
        assert!(model.requests.try_recv().is_err());
        let history = manager.inner.state.context(&id,&manager.inner.settings.get().agent.model).await.unwrap();
        assert!(!serde_json::to_string(&history).unwrap().contains(GENERATED_PNG));
    }
    assert!(tokio::fs::read_dir(root.path().join("outbox")).await.unwrap().next_entry().await.unwrap().is_none());
    assert!(!client.seen.iter().any(|message| message.to_string().contains(GENERATED_PNG)));
    manager.shutdown().await; server.abort();
}

#[tokio::test]
async fn sqlite_transactions_roll_back_consumption_and_forks_while_history_pages_stay_bounded() {
    let mut model = ModelServer::start(vec![completion("Recovered transaction",vec![])]).await;
    let (_root, manager, url, server) = fixture(&model,Api::ChatCompletions).await;
    let mut client = Client::connect(&url).await;
    let id = client.request(json!({"id":"create","type":"create_session"})).await["sessionId"].as_str().unwrap().to_owned();
    let snapshot = client.open(&id).await;
    assert_eq!(client.request(json!({"id":"pause","type":"queue_control","sessionId":id,"generation":snapshot["generation"],"operation":{"type":"pause","runId":null,"boundary":"turn"}})).await["ok"],true);
    for request in ["one","two"] {
        assert_eq!(client.request(json!({"id":request,"type":"prompt","sessionId":id,"text":request})).await["disposition"],"queued");
    }
    manager.inner.state.access(|db| {
        assert_eq!(db.pragma_query_value(None,"journal_mode",|row| row.get::<_,String>(0))?,"wal");
        assert_eq!(db.pragma_query_value(None,"synchronous",|row| row.get::<_,u32>(0))?,2);
        db.execute_batch("CREATE TRIGGER reject_user BEFORE INSERT ON entries WHEN NEW.kind='message' BEGIN SELECT RAISE(ABORT,'fixture failed history insert'); END;")?; Ok(())
    }).await.unwrap();
    client.request(json!({"id":"resume-fails","type":"queue_control","sessionId":id,"generation":snapshot["generation"],"operation":{"type":"resume","runId":null}})).await;
    client.until(|m| m["type"] == "session_state" && m["status"] == "error").await;
    let snapshot = client.open(&id).await;
    assert_eq!(snapshot["queue"]["requests"].as_array().unwrap().len(),2,"A failed user-entry insert must roll back removal of both queued requests");
    assert!(snapshot["events"].as_array().unwrap().is_empty());
    assert!(model.requests.try_recv().is_err());
    manager.inner.state.access(|db| { db.execute_batch("DROP TRIGGER reject_user")?; Ok(()) }).await.unwrap();
    client.request(json!({"id":"resume","type":"queue_control","sessionId":id,"generation":snapshot["generation"],"operation":{"type":"resume","runId":null}})).await;
    let request = model.request().await;
    assert_eq!(request["messages"][1]["content"],"one"); assert_eq!(request["messages"][2]["content"],"two");
    client.until(|m| m["type"] == "session_state" && m["status"] == "idle").await;
    // Seed a long completed history through the production commit/projection path.
    // Display reads must remain bounded independently of the provider payload size.
    let runtime = manager.runtime(&id).await.unwrap();
    let mut entries = Vec::new();
    for index in 0..80 {
        entries.push(json!({"type":"message","message":{"role":"user","content":format!("History question {index}")}}));
        entries.push(json!({"type":"message","message":{"role":"assistant","content":[{"type":"text","text":format!("History answer {index}")}],"stopReason":"stop",
            "tauModelMessage":{"role":"assistant","content":format!("History answer {index}"),"codex_output":[{"type":"reasoning","encrypted_content":"x".repeat(16384)}]}}}));
    }
    runtime.content.lock().await.commit(&id,entries,None,None).await.unwrap();
    manager.close_session(&id).await.unwrap();
    let snapshot = client.open(&id).await;
    assert!(snapshot["events"].as_array().unwrap().len() <= tau_blocks::MAX_FEED_PAGE);
    assert!(runtime.content.lock().await.transcript.is_none(),"Native reads must not reload the runtime");
    let mut all = snapshot["events"].as_array().unwrap().clone();
    let mut before = snapshot["before"].clone();
    while !before.is_null() {
        let position:tau_blocks::FeedPosition=serde_json::from_value(before).unwrap();
        let page=client.page(&id,Some(position.clone())).await;
        let older=page["events"].as_array().unwrap();
        assert!(older.len()<=tau_blocks::MAX_FEED_PAGE);
        assert!(older.iter().all(|e|e["order"].as_u64().unwrap()*2<=position.order));
        all.extend(older.iter().cloned());before=page["before"].clone();
    }
    all.sort_by_key(|e| e["order"].as_u64().unwrap());
    assert_eq!(all.len(),163); assert!(all.windows(2).all(|pair| pair[0]["order"].as_u64() < pair[1]["order"].as_u64()));
    let source = id.clone();
    manager.inner.state.access(move |db| {
        db.execute_batch(&format!("CREATE TRIGGER reject_branch BEFORE INSERT ON events WHEN NEW.session_id!='{source}' BEGIN SELECT RAISE(ABORT,'fixture failed branch copy'); END;"))?; Ok(())
    }).await.unwrap();
    assert_eq!(client.request(json!({"id":"bad-clone","type":"clone_session","sessionId":id})).await["ok"],false);
    assert_eq!(manager.inner.state.list().await.unwrap().len(),1,"A failed fork must not leave an orphan session or partial history");
    manager.inner.state.access(|db| { db.execute_batch("DROP TRIGGER reject_branch")?; Ok(()) }).await.unwrap();
    let clone = client.request(json!({"id":"clone","type":"clone_session","sessionId":id})).await;
    assert_eq!(clone["ok"],true); let clone_id = clone["sessionId"].as_str().unwrap();
    assert_eq!(client.open(clone_id).await["events"],snapshot["events"]);
    let fork = client.request(json!({"id":"fork-old","type":"fork_session","sessionId":id,"entryId":all[0]["entryId"]})).await;
    assert_eq!(fork["draft"],"one");
    assert!(client.open(fork["sessionId"].as_str().unwrap()).await["events"].as_array().unwrap().is_empty());
    assert_eq!(client.request(json!({"id":"delete","type":"delete_session","sessionId":id})).await["ok"],true);
    assert!(manager.inner.state.get(&id).await.unwrap().is_none());
    assert!(manager.inner.state.get(clone_id).await.unwrap().unwrap().parent_id.is_none());
    assert_eq!(client.open(clone_id).await["events"],snapshot["events"],"Deleting the parent must leave copied history intact");
    manager.inner.state.access(|db| {
        assert_eq!(db.query_row("PRAGMA integrity_check",[],|row| row.get::<_,String>(0))?,"ok");
        assert!(!db.prepare("PRAGMA foreign_key_check")?.exists([])?); Ok(())
    }).await.unwrap();
    manager.shutdown().await; server.abort();
}

#[tokio::test]
async fn cross_provider_image_tool_uses_codex_without_changing_the_chat_model_or_saving_image_bytes() {
    let gate=Arc::new(Notify::new()); let mut second=codex("",vec![generated("bridge-image-two")]); second.gate=Some(gate.clone());
    let mut images=ModelServer::start(vec![codex("",vec![generated("bridge-image")]),second]).await;
    let mut chat=ModelServer::start(vec![completion("",vec![call("draw","generate_image",json!({"prompt":"A tiny red square"}))]),completion("Image delivered",vec![]),
        completion("",vec![call("draw-two","generate_image",json!({"prompt":"A tiny blue square"}))]),completion("Delivery failed; ask before generating again",vec![])]).await;
    let (root,manager,url,server)=fixture(&chat,Api::ChatCompletions).await;
    let mut settings=manager.inner.settings.get();
    let main=settings.providers["openai-codex"].clone();
    settings.providers.insert("openrouter".into(),main);
    let image_provider=settings.providers.get_mut("openai-codex").unwrap(); image_provider.api=Api::Codex; image_provider.base_url=images.url.clone();
    let mut model=settings.models[0].clone(); model.provider="openrouter".into(); model.id="fixture-chat".into();
    settings.models.push(model); settings.agent.model=crate::state::SessionModel {provider:"openrouter".into(),model_id:"fixture-chat".into()};
    manager.set_settings(settings.revision,settings).await.unwrap();
    crate::settings::atomic_write(&root.path().join("auth.json"),json!({"openrouter":{"type":"api_key","key":"local-only"},"openai-codex":{"type":"oauth","access":"fixture-access","refresh":"unused","accountId":"fixture-account","expires":u64::MAX}}).to_string().as_bytes()).await.unwrap();
    let mut client=Client::connect(&url).await;
    let id=client.request(json!({"id":"new","type":"create_session"})).await["sessionId"].as_str().unwrap().to_owned(); client.open(&id).await;
    assert_eq!(client.request(json!({"id":"draw","type":"prompt","sessionId":id,"text":"Draw a square"})).await["ok"],true);
    let main=chat.request().await;
    assert!(main["tools"].as_array().unwrap().iter().any(|tool|tool["function"]["name"] == "generate_image"));
    let image=images.request().await;
    assert_eq!(image["tool_choice"]["type"],"image_generation");
    assert_eq!(image["tools"].as_array().unwrap(),&vec![json!({"type":"image_generation","model":"gpt-image-2","output_format":"png"})]);
    let continuation=chat.request().await;
    assert!(continuation.to_string().contains(GENERATED_PNG),"Generated references must also replay into the non-Codex chat");
    client.until(|m|m["type"] == "session_state" && m["sessionId"] == id && m["status"] == "idle").await;
    let snapshot=client.open(&id).await;
    let image=snapshot["events"].as_array().unwrap().iter().find(|e|e["attachment"]["kind"] == "image").unwrap();
    assert_eq!(image["role"],"assistant"); manager.resolve_attachment(&id,image["entryId"].as_str().unwrap()).await.unwrap();
    assert_eq!(manager.inner.settings.get().agent.model.provider,"openrouter");
    let export=root.path().join("history.json"); manager.inner.state.export_history(&id,&export).await.unwrap();
    let exported=tokio::fs::read_to_string(export).await.unwrap();
    assert!(!exported.contains(GENERATED_PNG) && exported.contains("tau-history") && exported.contains("tau_attachment"));
    assert!(!client.seen.iter().any(|message|message.to_string().contains(GENERATED_PNG)));
    // Lose the outbox after the provider accepted the paid operation, before delivery.
    assert_eq!(client.request(json!({"id":"draw-again","type":"prompt","sessionId":id,"text":"Now blue"})).await["ok"],true);
    chat.request().await; images.request().await;
    tokio::fs::rename(&manager.inner.config.attachment_root,root.path().join("outbox-offline")).await.unwrap(); gate.notify_one();
    let failed=chat.request().await;
    assert!(failed.to_string().contains("OpenAI generated the image, but Tau could not stage it"));
    assert!(failed.to_string().contains("Do not retry automatically; ask the user"));
    client.until(|m|m["type"] == "session_state" && m["sessionId"] == id && m["status"] == "idle").await;
    assert!(images.requests.try_recv().is_err(),"Failed image delivery must not trigger another generation request");
    manager.shutdown().await; server.abort();
}

#[tokio::test]
async fn native_title_request_uses_configured_prompt_and_never_overwrites_a_manual_name() {
    let gate=Arc::new(Notify::new()); let mut reply=completion("Generated title",vec![]); reply.gate=Some(gate.clone());
    let mut model=ModelServer::start(vec![reply,completion("Chat model title",vec![]),Reply {
        status:400, bytes:br#"{"error":{"message":"Title model rejected"}}"#.to_vec(), gate:None, body_gate:None,
    }]).await;
    let (_root,manager,_url,server)=fixture(&model,Api::ChatCompletions).await;
    let mut settings=manager.inner.settings.get(); settings.daemon.generate_titles=true; settings.daemon.title_prompt="Exact template: {text}\n".into();
    settings.daemon.title_model=Some("openai-codex/title-without-metadata".parse().unwrap());
    manager.set_settings(settings.revision,settings).await.unwrap();
    let id=manager.create_session(None, "general").await.unwrap(); let task_manager=manager.clone(); let task_id=id.clone();
    let title=tokio::spawn(async move { task_manager.title_after_prompt(&task_id,"Example task").await; });
    let request=model.request().await;
    assert_eq!(request["model"],"title-without-metadata");
    assert_eq!(manager.inner.state.get(&id).await.unwrap().unwrap().model.model_id,"gpt-6-astra");
    assert_eq!(manager.inner.settings.get().agent.model.model_id,"gpt-6-astra");
    assert_eq!(request["messages"][1]["content"],"Exact template: Example task\n");
    assert!(request["tools"].as_array().is_none_or(|tools|tools.is_empty()));
    manager.rename_session(&id,"Manually named").await.unwrap(); gate.notify_one(); title.await.unwrap();
    assert_eq!(manager.inner.state.get(&id).await.unwrap().unwrap().title,"Manually named");
    let mut settings=manager.inner.settings.get(); settings.daemon.title_model=None;
    manager.set_settings(settings.revision,settings).await.unwrap();
    let next=manager.create_session(None, "general").await.unwrap();
    manager.title_after_prompt(&next,"Next task").await;
    assert_eq!(model.request().await["model"],"gpt-6-astra");
    assert_eq!(manager.inner.state.get(&next).await.unwrap().unwrap().title,"Chat model title");
    let failed=manager.create_session(None, "general").await.unwrap();
    manager.title_after_prompt(&failed,"Fallback first line\nMore text").await;
    model.request().await;
    assert_eq!(manager.inner.state.get(&failed).await.unwrap().unwrap().title,"Fallback first line");
    manager.shutdown().await; server.abort();
}

#[tokio::test]
async fn model_prompt_overrides_and_unlisted_ids_reach_both_providers_and_survive_restart() {
    for api in [Api::Codex, Api::ChatCompletions] {
        let gate = Arc::new(Notify::new());
        let mut replies = Vec::new();
        for _ in 0..5 {
            let mut reply = if api == Api::Codex { codex("Reply",vec![]) } else { completion("Reply",vec![]) };
            reply.gate = Some(gate.clone()); replies.push(reply);
        }
        replies.push(if api == Api::Codex { codex("",vec![json!({"type":"compaction","encrypted_content":"fixture-checkpoint"})]) }
            else { completion("Summary",vec![]) });
        let credential = if api == Api::Codex { "fixture-access" } else { "fixture-key" };
        replies.push(Reply { status:400, bytes:json!({"error":{"message":format!("No such model: fixture-missing ({credential})")}}).to_string().into_bytes(),
            gate:Some(gate.clone()), body_gate:None });
        let mut model = ModelServer::start(replies).await;
        let (root, manager, url, server) = fixture(&model, api).await;
        tokio::fs::write(root.path().join("AGENTS.md"), "File context stays separate").await.unwrap();
        let mut client = Client::connect(&url).await;
        let mut settings = manager.inner.settings.get();
        settings.models.clear();
        settings.agent.system_prompt = "Default exact\n".into();
        settings.agent.model_system_prompts.insert("openai-codex/gpt-6-astra".into(), String::new());
        settings.agent.model_system_prompts.insert("openai-codex/gpt-6-sol".into(), "  Model exact\n".into());
        settings.agent.load_agents_files = true;
        settings.agent.compaction.reserve_tokens = 100_000_000;
        assert_eq!(client.request(json!({"id":"settings","type":"set_settings","revision":settings.revision,"settings":settings})).await["ok"],true);
        let id = client.request(json!({"id":"create","type":"create_session"})).await["sessionId"].as_str().unwrap().to_owned();
        client.open(&id).await;
        for (index, (model_id, prefix)) in [("gpt-6-astra",""), ("gpt-6-sol","  Model exact\n"), ("unlisted/exact-id","Default exact\n")].into_iter().enumerate() {
            assert_eq!(client.request(json!({"id":format!("select-{index}"),"type":"prompt","sessionId":id,"text":format!("/model openai-codex/{model_id}")})).await["ok"],true);
            assert_eq!(client.request(json!({"id":format!("turn-{index}"),"type":"prompt","sessionId":id,"text":"Check prompt"})).await["ok"],true);
            let request = model.request().await;
            assert_eq!(request["model"],model_id);
            let prompt = if api == Api::Codex { request["instructions"].as_str().unwrap() } else { request["messages"][0]["content"].as_str().unwrap() };
            assert!(prompt.starts_with(&format!("{prefix}\n\nAGENTS.md instructions")),"{prompt}");
            assert!(!prompt.contains(crate::settings::DEFAULT_SYSTEM_PROMPT));
            assert!(prompt.contains("File context stays separate"));
            assert!(request["tools"].as_array().unwrap().len()>1,"Empty prompt must keep tools");
            gate.notify_one();
            let idle = client.until(|m| m["type"] == "session_state" && m["sessionId"] == id && m["status"] == "idle").await;
            assert_eq!(idle["contextUsage"]["tokens"], if api == Api::Codex {120} else {1024});
            assert!(idle["contextUsage"]["contextWindow"].is_null(), "An unlisted model has no invented capacity");
        }
        let mut settings = manager.inner.settings.get(); settings.agent.system_prompt.clear();
        assert_eq!(client.request(json!({"id":"empty-default","type":"set_settings","revision":settings.revision,"settings":settings})).await["ok"],true);
        client.request(json!({"id":"empty-turn","type":"prompt","sessionId":id,"text":"Empty default stays empty"})).await;
        let request = model.request().await;
        let prompt = if api == Api::Codex { &request["instructions"] } else { &request["messages"][0]["content"] };
        assert!(prompt.as_str().unwrap().starts_with("\n\nAGENTS.md instructions"));
        gate.notify_one(); client.until(|m| m["type"] == "session_state" && m["status"] == "idle").await;
        let saved: Value = serde_json::from_slice(&tokio::fs::read(&manager.inner.config.settings_path).await.unwrap()).unwrap();
        assert_eq!(saved["agent"]["systemPrompt"],"");
        assert_eq!(saved["agent"]["modelSystemPrompts"]["openai-codex/gpt-6-astra"],"");
        assert!(saved["agent"].get("projectPrompts").is_none());
        assert!(saved["agent"].get("appendSystemPrompt").is_none());
        let config = manager.inner.config.clone();
        drop(client); manager.shutdown().await; server.abort();
        let manager = AgentManager::new(config.clone(),StateStore::load(config.database_path.clone()).await.unwrap()).await.unwrap();
        assert_eq!(manager.inner.settings.get().agent.system_prompt,"");
        assert_eq!(manager.inner.settings.get().agent.model_system_prompts["openai-codex/gpt-6-astra"],"");
        let (url,server) = serve(&manager).await; let mut client = Client::connect(&url).await;
        client.open(&id).await;
        let mut settings = manager.inner.settings.get();
        settings.agent.system_prompt = "Changed default".into(); settings.agent.model_system_prompts.remove("openai-codex/gpt-6-astra");
        client.request(json!({"id":"inherit-default","type":"set_settings","revision":settings.revision,"settings":settings})).await;
        client.request(json!({"id":"select-astra","type":"prompt","sessionId":id,"text":"/model openai-codex/gpt-6-astra"})).await;
        client.request(json!({"id":"inherit-turn","type":"prompt","sessionId":id,"text":"Use saved default"})).await;
        let request = model.request().await;
        let prompt = if api == Api::Codex { &request["instructions"] } else { &request["messages"][0]["content"] };
        assert!(prompt.as_str().unwrap().starts_with("Changed default\n\nAGENTS.md instructions"));
        gate.notify_one(); client.until(|m| m["type"] == "session_state" && m["status"] == "idle").await;
        let mut settings = manager.inner.settings.get(); settings.agent.model_system_prompts.insert("openai-codex/gpt-6-astra".into(),String::new());
        client.request(json!({"id":"empty-again","type":"set_settings","revision":settings.revision,"settings":settings})).await;
        assert_eq!(client.request(json!({"id":"compact-empty","type":"prompt","sessionId":id,"text":"/compact"})).await["ok"],true);
        let request = model.request().await;
        let prompt = if api == Api::Codex { &request["instructions"] } else { &request["messages"][0]["content"] };
        assert!(prompt.as_str().unwrap().starts_with("\n\nAGENTS.md instructions"),"Compaction uses the same selected-model prompt");
        let failed = client.request(json!({"id":"create-missing","type":"create_session"})).await["sessionId"].as_str().unwrap().to_owned();
        client.open(&failed).await;
        assert_eq!(client.request(json!({"id":"select-missing","type":"prompt","sessionId":failed,"text":"/model openai-codex/fixture-missing"})).await["ok"],true);
        client.request(json!({"id":"missing-turn","type":"prompt","sessionId":failed,"text":"Try the exact ID"})).await;
        assert_eq!(model.request().await["model"],"fixture-missing"); gate.notify_one();
        let error = client.until(|m| m["type"] == "session_state" && m["sessionId"] == failed && m["status"] == "error").await;
        let detail = error["detail"].as_str().unwrap();
        assert!(detail.contains("HTTP 400") && detail.contains("No such model: fixture-missing"),"{detail}");
        assert!(!detail.contains(credential));
        assert!(model.requests.try_recv().is_err(),"Provider 400 errors are not retried");
        manager.shutdown().await; server.abort();
    }
}

#[tokio::test]
async fn provider_catalog_outweighs_configured_limits_and_uses_exact_authenticated_model() {
    for api in [Api::Codex, Api::ChatCompletions] {
        let window = if api == Api::Codex { 320_000 } else { 64_000 };
        let catalog = if api == Api::Codex {
            json!({"models":[{"slug":"gpt-6-sol","context_window":window,"max_context_window":500000},
                             {"slug":"gpt-6-luna","max_context_window":270000}]})
        } else { json!({"data":[{"id":"gpt-6-sol","context_length":window}]}) };
        let mut model = ModelServer::with_catalog(vec![if api == Api::Codex { codex("Reply", vec![]) } else { completion("Reply", vec![]) }], Some(catalog)).await;
        let (_root, manager, url, server) = fixture(&model, api).await;
        let mut settings = manager.inner.settings.get();
        settings.models.push(crate::settings::ModelSettings { provider:"openai-codex".into(), id:"gpt-6-sol".into(),
            context_window:Some(272_000), ..Default::default() });
        let path = if api == Api::Codex { "backend-api/codex" } else { "api/v1" };
        settings.providers.get_mut("openai-codex").unwrap().base_url = format!("{}/{path}",model.url);
        manager.set_settings(settings.revision, settings).await.unwrap();
        let mut client = Client::connect(&url).await;
        let id = client.request(json!({"id":"create","type":"create_session"})).await["sessionId"].as_str().unwrap().to_owned();
        let call = model.catalog_request().await;
        assert_eq!(call["path"], format!("/{path}/models"));
        assert_eq!(call["authorization"], if api == Api::Codex { "Bearer fixture-access" } else { "Bearer fixture-key" });
        if api == Api::Codex {
            assert_eq!(call["account"], "fixture-account");
            assert_eq!(call["originator"], "tau");
            assert_eq!(call["query"],"client_version=0.156.1");
        } else { assert!(call["account"].is_null() && call["originator"].is_null()); }
        client.until(|m| m["type"] == "sessions" && m["sessions"].as_array().is_some_and(|list| list.iter().any(|s| s["id"] == id && s["contextUsage"].is_null()))).await;
        client.open(&id).await;
        let initial = client.seen.iter().rev().find(|m| m["type"] == "session_state" && m["sessionId"] == id).unwrap();
        // An authoritative catalog without Astra must not borrow Astra's
        // configured 272K limit from the imported metadata.
        assert!(initial["contextUsage"].is_null());
        assert_eq!(client.request(json!({"id":"model","type":"prompt","sessionId":id,"text":"/model openai-codex/gpt-6-sol"})).await["ok"],true);
        client.open(&id).await;
        let state = client.seen.iter().rev().find(|m| m["type"] == "session_state" && m["sessionId"] == id).unwrap();
        assert_eq!(state["contextUsage"],json!({"tokens":null,"contextWindow":window}));
        client.request(json!({"id":"turn","type":"prompt","sessionId":id,"text":"Use this catalog"})).await;
        assert_eq!(model.request().await["model"],"gpt-6-sol");
        let idle = client.until(|m| m["type"] == "session_state" && m["sessionId"] == id && m["status"] == "idle").await;
        assert_eq!(idle["contextUsage"],json!({"tokens":if api == Api::Codex {120} else {1024},"contextWindow":window}));
        client.request(json!({"id":"list","type":"list_sessions"})).await;
        let summary = client.seen.iter().rev().find(|m| m["type"] == "sessions").unwrap()["sessions"].as_array().unwrap().iter().find(|s| s["id"] == id).unwrap();
        assert_eq!(summary["contextUsage"],idle["contextUsage"]);
        manager.shutdown().await; server.abort();
    }
}

#[tokio::test]
async fn missing_catalog_alerts_manual_refresh_persists_and_old_file_survives_failure() {
    for api in [Api::Codex, Api::ChatCompletions] {
        let mut model = ModelServer::start(vec![if api == Api::Codex { codex("Reply", vec![]) } else { completion("Reply", vec![]) }]).await;
        let (root, manager, url, server) = fixture(&model, api).await;
        let mut alerts = manager.subscribe();
        let mut client = Client::connect(&url).await;
        let id = client.request(json!({"id":"create","type":"create_session"})).await["sessionId"].as_str().unwrap().to_owned();
        assert_eq!(model.catalog_request().await["path"],"/models");
        let alert = tokio::time::timeout(Duration::from_secs(10), async {
            loop { if let crate::protocol::ServerMessage::Notice { message, .. } = alerts.recv().await.unwrap() { break message; } }
        }).await.unwrap();
        assert!(alert.contains("Could not load openai-codex model catalog"),"{alert}");
        assert!(!root.path().join("model-catalog.json").exists());
        client.open(&id).await;
        assert_eq!(client.request(json!({"id":"select","type":"prompt","sessionId":id,"text":"/model openai-codex/gpt-6-sol"})).await["ok"],true);
        client.request(json!({"id":"turn","type":"prompt","sessionId":id,"text":"Report usage"})).await;
        assert_eq!(model.request().await["model"],"gpt-6-sol");
        let idle = client.until(|m| m["type"] == "session_state" && m["sessionId"] == id && m["status"] == "idle").await;
        let tokens = if api == Api::Codex { 120 } else { 1024 };
        assert_eq!(idle["contextUsage"], json!({"tokens":tokens,"contextWindow":null}));

        let response = if api == Api::Codex { json!({"models":[{"slug":"gpt-6-sol","context_window":200000}]}) }
            else { json!({"data":[{"id":"gpt-6-sol","context_length":200000}]}) };
        model.set_catalog(Some(response)).await;
        let refreshed = client.request(json!({"id":"refresh","type":"refresh_model_catalog","provider":"openai-codex"})).await;
        assert_eq!(refreshed["ok"],true,"{refreshed}");
        assert!(refreshed["notice"].as_str().unwrap().contains("1 models"));
        let saved = tokio::fs::read(root.path().join("model-catalog.json")).await.unwrap();
        assert!(!String::from_utf8_lossy(&saved).contains("fixture-access"));
        assert!(!String::from_utf8_lossy(&saved).contains("fixture-key"));
        client.open(&id).await;
        let state = client.seen.iter().rev().find(|m| m["type"] == "session_state" && m["sessionId"] == id).unwrap();
        assert_eq!(state["contextUsage"],json!({"tokens":tokens,"contextWindow":200000}));
        client.request(json!({"id":"commands","type":"get_commands","sessionId":id})).await;
        assert!(client.seen.iter().rev().find(|m| m["type"] == "commands" && m["sessionId"] == id).unwrap()["commands"]
            .as_array().unwrap().iter().any(|c| c["name"] == "model" && c["arguments"].as_array().unwrap().iter().any(|a| a["value"] == "openai-codex/gpt-6-sol")));

        model.set_catalog(None).await;
        let failed = client.request(json!({"id":"refresh-failed","type":"refresh_model_catalog","provider":"openai-codex"})).await;
        assert_eq!(failed["ok"],false);
        assert_eq!(tokio::fs::read(root.path().join("model-catalog.json")).await.unwrap(),saved,"A failed refresh must retain the last valid cache");
        assert_eq!(model.catalog_request().await["path"],"/models");
        assert_eq!(model.catalog_request().await["path"],"/models");
        client.socket.close(None).await.unwrap();
        let config = manager.inner.config.clone(); manager.shutdown().await; server.abort();

        let manager = AgentManager::new(config.clone(),StateStore::load(config.database_path.clone()).await.unwrap()).await.unwrap();
        let (url, server) = serve(&manager).await; let mut client = Client::connect(&url).await;
        client.until(|m| m["type"] == "sessions" && m["sessions"].as_array().is_some_and(|list| list.iter().any(|s| s["id"] == id && s["contextUsage"]["contextWindow"] == 200000))).await;
        assert!(model.catalogs.try_recv().is_err(),"A valid cached file needs no GET after restart");
        client.open(&id).await;
        let state = client.seen.iter().rev().find(|m| m["type"] == "session_state" && m["sessionId"] == id).unwrap();
        assert_eq!(state["contextUsage"],json!({"tokens":tokens,"contextWindow":200000}));
        assert_eq!(client.request(json!({"id":"switch","type":"prompt","sessionId":id,"text":"/model openai-codex/other-id"})).await["ok"],true);
        client.open(&id).await;
        let state = client.seen.iter().rev().find(|m| m["type"] == "session_state" && m["sessionId"] == id).unwrap();
        assert!(state["contextUsage"].is_null(),"Changing models clears old usage and never borrows another model's window");
        manager.shutdown().await; server.abort();
    }
}

#[tokio::test]
async fn saved_settings_keep_exact_text_without_a_third_prompt_layer() {
    use crate::settings::SettingsExt;
    let model = ModelServer::start(vec![]).await;
    let (_root, manager, _, server) = fixture(&model, Api::Codex).await;
    let config = manager.inner.config.clone(); manager.shutdown().await; server.abort();
    for (old, append, expected) in [(Value::Null,"",crate::settings::DEFAULT_SYSTEM_PROMPT), (json!(""),"",""), (json!("Custom\n"),"Append","Custom\n\n\nAppend")] {
        let mut value = serde_json::to_value(manager.inner.settings.get()).unwrap();
        value["schema"] = json!(1); value["agent"]["systemPrompt"] = old;
        value["agent"]["appendSystemPrompt"] = json!(append); value["agent"]["projectPrompts"] = json!({});
        let agent = value["agent"].as_object_mut().unwrap();
        agent.remove("loadAgentsFiles"); agent.insert("loadProjectInstructions".into(),json!(false));
        agent.remove("modelSystemPrompts");
        let bytes = serde_json::to_vec(&value).unwrap(); tokio::fs::write(&config.settings_path,&bytes).await.unwrap();
        let store = crate::settings::SettingsStore::load(&config,String::new()).await.unwrap();
        let settings = store.get();
        assert_eq!(settings.schema,2); assert_eq!(settings.agent.system_prompt,expected);
        assert_eq!(tokio::fs::read(&config.settings_path).await.unwrap(),bytes,"Loading does not rewrite the settings file");
        let mut settings = settings.clone();
        settings.agent.model_system_prompts.insert("openai-codex/gpt-6-astra".into(),String::new());
        assert!(settings.system_prompt(&settings.agent.model,&config.cwd).await.unwrap().starts_with("\n\nCurrent working directory:"));
        store.set(settings.revision,settings).await.unwrap();
        let restored = crate::settings::SettingsStore::load(&config,String::new()).await.unwrap().get();
        assert_eq!(restored.agent.model_system_prompts["openai-codex/gpt-6-astra"],"");
        assert_eq!(restored.agent.system_prompt,expected);
        value["agent"]["projectPrompts"] = json!({"/somewhere":{"systemPrompt":"Never silently discard text"}});
        tokio::fs::write(&config.settings_path,serde_json::to_vec(&value).unwrap()).await.unwrap();
        assert!(crate::settings::SettingsStore::load(&config,String::new()).await.is_err());
    }
    let mut settings = manager.inner.settings.get(); settings.models.clear(); settings.validate().unwrap();
    settings.agent.model_system_prompts.insert("invalid".into(),String::new()); assert!(settings.validate().is_err());
    settings.agent.model_system_prompts.clear();
    settings.agent.model_system_prompts.insert("openai-codex/example".into(),"x".repeat(crate::protocol::MAX_PROMPT_CHARS+1));
    assert!(settings.validate().is_err());
    settings.agent.model_system_prompts.clear();
    settings.daemon.title_model=Some("missing-provider/model".parse().unwrap()); assert!(settings.validate().is_err());
}

#[tokio::test]
async fn project_prompt_snapshots_reach_both_providers_and_moves_replace_them() {
    for api in [Api::Codex, Api::ChatCompletions] {
        let gate = Arc::new(Notify::new());
        let replies = (0..4).map(|_| {
            let mut reply = if api == Api::Codex { codex("Done",vec![]) } else { completion("Done",vec![]) };
            reply.gate = Some(gate.clone()); reply
        }).collect();
        let mut model = ModelServer::start(replies).await;
        let (_root, manager, url, server) = fixture(&model, api).await;
        let mut client = Client::connect(&url).await;
        let project = uuid::Uuid::new_v4().to_string();
        assert_eq!(client.request(json!({"id":"project","type":"create_project","projectId":project,"name":"Research","prompt":"  Project v1\n"})).await["ok"],true);
        let id = client.request(json!({"id":"chat","type":"create_session","projectId":project})).await["sessionId"].as_str().unwrap().to_owned();
        let general = client.request(json!({"id":"general","type":"create_session"})).await["sessionId"].as_str().unwrap().to_owned();
        assert_ne!(general,id,"Each project has its own starter");
        assert_eq!(client.request(json!({"id":"same","type":"create_session","projectId":project})).await["sessionId"],id);
        assert_eq!(client.request(json!({"id":"edit","type":"update_project","projectId":project,"revision":0,"name":"Research renamed","prompt":"Project v2"})).await["ok"],true);
        assert_eq!(client.request(json!({"id":"stale","type":"update_project","projectId":project,"revision":0,"name":"Lost edit","prompt":"WRONG"})).await["ok"],false);
        let fresh = client.request(json!({"id":"fresh","type":"create_session","projectId":project})).await["sessionId"].as_str().unwrap().to_owned();
        assert_ne!(id,fresh,"Never rewrite an existing starter's captured instructions");
        assert_eq!(manager.inner.state.get(&fresh).await.unwrap().unwrap().project_prompt,"Project v2");
        for (index, (session, suffix)) in [(&id, "  Project v1\n"), (&fresh, "Project v2"), (&general, "Project v2"), (&id, "General context")].into_iter().enumerate() {
            if index == 2 {
                assert_eq!(client.request(json!({"id":"move","type":"move_session","sessionId":general,"projectId":project})).await["ok"],true);
            }
            if index == 3 {
                assert_eq!(client.request(json!({"id":"general-prompt","type":"update_project","projectId":"general","revision":0,"name":"General","prompt":"General context"})).await["ok"],true);
                assert_eq!(client.request(json!({"id":"move-back","type":"move_session","sessionId":id,"projectId":"general"})).await["ok"],true);
            }
            client.open(session).await;
            assert_eq!(client.request(json!({"id":format!("turn-{index}"),"type":"prompt","sessionId":session,"text":"Check captured instructions"})).await["ok"],true);
            let request = model.request().await;
            let prompt = if api == Api::Codex { &request["instructions"] } else { &request["messages"][0]["content"] };
            assert!(prompt.as_str().unwrap().ends_with(&format!("\n\n{suffix}")),"{prompt}");
            assert!(prompt.as_str().unwrap().starts_with(&manager.inner.settings.get().agent.system_prompt));
            assert_eq!(request["messages"].as_array().map(|m| m.iter().filter(|v| v["role"] == "system").count()).unwrap_or(1),1);
            gate.notify_one();
            client.until(|m| m["type"] == "session_state" && m["sessionId"] == *session && m["status"] == "idle").await;
        }
        let clone = client.request(json!({"id":"clone","type":"clone_session","sessionId":general})).await["sessionId"].as_str().unwrap().to_owned();
        let copied = manager.inner.state.get(&clone).await.unwrap().unwrap();
        assert_eq!((copied.project_id.as_str(),copied.project_prompt.as_str()),(project.as_str(),"Project v2"));
        assert_eq!(client.request(json!({"id":"delete-general","type":"delete_project","projectId":"general","revision":1,"mode":"delete_chats"})).await["ok"],false);
        assert_eq!(client.request(json!({"id":"rename-general","type":"update_project","projectId":"general","revision":1,"name":"Other","prompt":""})).await["ok"],false);
        // Deleting a project with Keep is a real move, including its captured prompt.
        assert_eq!(client.request(json!({"id":"keep","type":"delete_project","projectId":project,"revision":1,"mode":"move_to_general"})).await["ok"],true);
        for session in [&fresh,&general,&clone] {
            let stored = manager.inner.state.get(session).await.unwrap().unwrap();
            assert_eq!((stored.project_id.as_str(),stored.project_prompt.as_str()),("general","General context"));
            assert!(!manager.inner.state.page(session,None).await.unwrap().events.is_empty());
        }
        let config = manager.inner.config.clone();
        manager.shutdown().await; server.abort(); drop(client);
        let reopened = StateStore::load(config.database_path).await.unwrap();
        assert_eq!(reopened.projects().await.unwrap(),vec![tau_protocol::Project { id:"general".into(),name:"General".into(),prompt:"General context".into(),revision:1 }]);
        assert_eq!(reopened.get(&id).await.unwrap().unwrap().project_prompt,"General context");
        assert!(model.requests.try_recv().is_err(),"Metadata edits must never invoke a provider");
    }
}

#[tokio::test]
async fn moving_during_tool_run_keeps_that_turn_stable_and_project_delete_cancels_work_atomically() {
    let gate = Arc::new(Notify::new());
    let mut first = completion("Checking",vec![call("read","read",json!({"path":"input.txt"}))]);
    first.gate = Some(gate.clone());
    let mut last = completion("Must be cancelled",vec![]); last.gate = Some(Arc::new(Notify::new()));
    let mut model = ModelServer::start(vec![first,completion("Done",vec![]),last]).await;
    let (root, manager, url, server) = fixture(&model,Api::ChatCompletions).await;
    tokio::fs::write(root.path().join("input.txt"),"test").await.unwrap();
    let mut client = Client::connect(&url).await;
    let project = uuid::Uuid::new_v4().to_string();
    client.request(json!({"id":"project","type":"create_project","projectId":project,"name":"Work","prompt":"DESTINATION"})).await;
    let session = client.request(json!({"id":"chat","type":"create_session"})).await["sessionId"].as_str().unwrap().to_owned();
    client.open(&session).await;
    client.request(json!({"id":"turn","type":"prompt","sessionId":session,"text":"Read input"})).await;
    let before = model.request().await["messages"][0].clone();
    assert_eq!(client.request(json!({"id":"move","type":"move_session","sessionId":session,"projectId":project})).await["ok"],true);
    assert_eq!(client.request(json!({"id":"edit","type":"update_project","projectId":project,"revision":0,"name":"Work","prompt":"LATER"})).await["ok"],true);
    gate.notify_one();
    assert_eq!(model.request().await["messages"][0],before,"A tool continuation must keep its in-flight instructions");
    client.until(|m| m["type"] == "session_state" && m["sessionId"] == session && m["status"] == "idle").await;
    let clone = manager.clone_session(&session).await.unwrap();
    manager.move_session(&clone,"general".into()).await.unwrap();
    client.request(json!({"id":"next","type":"prompt","sessionId":session,"text":"Next turn"})).await;
    let next = model.request().await;
    assert!(next["messages"][0]["content"].as_str().unwrap().ends_with("\n\nDESTINATION"));
    assert_eq!(manager.inner.state.get(&session).await.unwrap().unwrap().project_prompt,"DESTINATION");
    let upload = root.path().join("uploads").join(&session);
    let bytes=b"private upload";let hash=blake3::hash(bytes).to_hex().to_string();
    let spec=tau_blocks::UploadSpec {id:"project-delete-file".into(),length:bytes.len() as u64,hash:hash.clone(),purpose:tau_blocks::UploadPurpose::File {session_id:session.clone(),file_name:"file".into()}};
    manager.begin_upload(spec.clone()).await.unwrap();manager.write_upload(spec.clone(),0,bytes.to_vec()).await.unwrap();manager.finish_upload(spec).await.unwrap();
    // A forced transaction failure leaves the project and every chat intact.
    manager.inner.state.access(|db| { db.execute_batch("CREATE TRIGGER fail_project_delete BEFORE DELETE ON sessions BEGIN SELECT RAISE(ABORT,'fixture disk failure'); END;")?; Ok(()) }).await.unwrap();
    assert_eq!(client.request(json!({"id":"fail-delete","type":"delete_project","projectId":project,"revision":1,"mode":"delete_chats"})).await["ok"],false);
    assert!(manager.inner.state.get(&session).await.unwrap().is_some());
    assert!(manager.inner.state.projects().await.unwrap().iter().any(|p| p.id == project));
    assert!(upload.is_dir());
    manager.inner.state.access(|db| { db.execute_batch("DROP TRIGGER fail_project_delete")?; Ok(()) }).await.unwrap();
    assert_eq!(client.request(json!({"id":"delete","type":"delete_project","projectId":project,"revision":1,"mode":"delete_chats"})).await["ok"],true);
    assert!(manager.inner.state.get(&session).await.unwrap().is_none());
    assert!(!upload.exists());
    let survivor = manager.inner.state.get(&clone).await.unwrap().unwrap();
    assert!(survivor.parent_id.is_none());
    assert!(!manager.inner.state.page(&clone,None).await.unwrap().events.is_empty());
    manager.inner.state.access(|db| {
        assert_eq!(db.query_row("SELECT count(*) FROM receipts WHERE session_id NOT IN (SELECT id FROM sessions)",[],|r|r.get::<_,u32>(0))?,0);
        assert_eq!(db.query_row("PRAGMA integrity_check",[],|r|r.get::<_,String>(0))?,"ok");
        Ok(())
    }).await.unwrap();
    manager.shutdown().await; server.abort();
}

#[tokio::test]
async fn cold_acceptance_and_abort_do_not_wait_for_provider_context_preparation() {
    let model=ModelServer::start(vec![]).await;
    let (_root,manager,_,server)=fixture(&model,Api::ChatCompletions).await;
    let id=manager.create_session(None,"general").await.unwrap();manager.close_session(&id).await.unwrap();
    let gate=Arc::new(Notify::new());*manager.inner.state.context_gate.lock().unwrap()=Some(gate.clone());
    let first=tokio::time::timeout(Duration::from_millis(500),manager.prompt(&id,"first","cold-first")).await.unwrap().unwrap();
    assert!(matches!(first.disposition,crate::protocol::PromptDisposition::Submitted));
    tokio::time::sleep(Duration::from_millis(30)).await;
    tokio::time::timeout(Duration::from_millis(500),manager.prompt(&id,"second","cold-second")).await.unwrap().unwrap();
    assert!(manager.inner.state.receipt(&id,"cold-second").await.unwrap().is_some());
    tokio::time::timeout(Duration::from_millis(500),manager.abort(&id,"stop-cold")).await.unwrap().unwrap();
    assert!(manager.inner.state.queue(&id).await.unwrap().paused);
    gate.notify_waiters();manager.shutdown().await;server.abort();
}

#[path="agent_test_safety.rs"]
mod safety;
