#![cfg(unix)]
//! Production controller/transport/store against the real daemon and a
//! local native-provider fixture. No GPU, Pi subprocess, or live account.
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tau_frontend::{
    controller::Controller,
    store::{Settings, Store},
};
use tau_protocol::*;

#[path = "../src/daemon_settings.rs"]
mod daemon_settings;

async fn until(c: &mut Controller, condition: impl Fn(&Controller) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(12);
    loop {
        c.poll();
        if condition(c) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "Timed out: connection={}, notice={:?}, feed={:?}",
            c.connection,
            c.notice,
            c.selected().map(|chat| (
                &chat.feed.queue,
                &chat.local.pending,
                chat.feed
                    .events
                    .values()
                    .map(|e| (&e.kind, &e.text, &e.error_message))
                    .collect::<Vec<_>>()
            ))
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_native_daemon_chat_queue_upload_settings_fork_and_client_restart() {
    use axum::{Json, Router, extract::State, response::IntoResponse, routing::post};
    use serde_json::{Value, json};
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[derive(Clone)]
    struct Script {
        calls: Arc<AtomicUsize>,
        gate: Arc<tokio::sync::Notify>,
        requests: tokio::sync::mpsc::UnboundedSender<Value>,
    }
    async fn model(State(script): State<Script>, Json(body): Json<Value>) -> impl IntoResponse {
        let index = script.calls.fetch_add(1, Ordering::SeqCst);
        script.requests.send(body).unwrap();
        if index == 0 {
            script.gate.notified().await;
        }
        assert!(index < 4, "Unexpected extra provider execution");
        let delta = if index == 1 {
            json!({"tool_calls":[{"index":0,"id":"inspect","type":"function","function":{"name":"bash","arguments":"{\"command\":\"find uploads -type f -exec cat {} \\\\; > inspected.txt\"}"}}]})
        } else {
            json!({"content":if index == 0 {"Aborted response must not appear"} else {"Native reply café 😀"}})
        };
        let body = format!(
            "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
            json!({"choices":[{"index":0,"delta":delta}]}),
            json!({"choices":[{"index":0,"delta":{},"finish_reason":if index == 1 {"tool_calls"} else {"stop"}}],"usage":{"total_tokens":300}})
        );
        ([("content-type", "text/event-stream")], body)
    }
    let server = tempfile::tempdir().unwrap();
    let root = server.path();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let model_address = listener.local_addr().unwrap();
    let gate = Arc::new(tokio::sync::Notify::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let (requests, mut received) = tokio::sync::mpsc::unbounded_channel();
    let script = Script {
        calls: calls.clone(),
        gate: gate.clone(),
        requests,
    };
    let provider = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/{*path}", post(model))
                .with_state(script),
        )
        .await
        .unwrap()
    });
    let mut daemon_settings = tau_protocol::settings::Settings::default();
    daemon_settings.daemon.idle_timeout_seconds = 0;
    daemon_settings.agent.load_agents_files = false;
    let endpoint = daemon_settings.providers.get_mut("openai-codex").unwrap();
    endpoint.api = tau_protocol::settings::Api::ChatCompletions;
    endpoint.base_url = format!("http://{model_address}");
    endpoint.web_search = false;
    std::fs::write(
        root.join("settings.json"),
        serde_json::to_vec(&daemon_settings).unwrap(),
    )
    .unwrap();
    std::fs::write(
        root.join("auth.json"),
        r#"{"openai-codex":{"type":"api_key","key":"local-fixture-only"}}"#,
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            root.join("auth.json"),
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
    }
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let token = "fixture-bearer-with-at-least-thirty-two-characters";
    let config = taud::Config {
        bind: port,
        transfer_bind: "127.0.0.1:0".parse().unwrap(), transfer_bind_v6:None,
        token: Arc::from(token),
        settings_path: root.join("settings.json"),
        import_pi_dir: None, codex_auth_source:None,
        cwd: root.into(),
        database_path: root.join("tau.sqlite3"),
        telemetry_path: root.join("crash.jsonl"),
        attachment_root: root.join("outbox"),
        upload_root: root.join("uploads"),
    };
    let task = tokio::spawn(taud::run(config));
    let local = tempfile::tempdir().unwrap();
    let store = Store::open(local.path().into()).unwrap();
    store
        .put(
            "",
            "settings",
            &Settings {
                server_url: format!("http://{port}"),
                token: token.into(),
            },
        )
        .unwrap();
    let mut c = Controller::new(store, Arc::new(|| {})).unwrap();
    until(&mut c, |c| c.epoch.is_some()).await;
    c.new_chat().unwrap();
    let session = c.account.selected.clone().unwrap();
    assert_eq!(c.account.pending_create.as_ref().unwrap().id, session);
    assert!(!c.selected().unwrap().feed.synchronized, "The new chat must appear before a server round-trip");
    c.draft("hold".into()).unwrap();
    c.send_prompt().unwrap();
    assert_eq!(c.selected().unwrap().local.pending[0].status, tau_frontend::store::Delivery::WaitingForChat);
    // A provider response is deliberately gated below. Confirmation, the user
    // entry, and release of the local pending send must not await that response.
    until(&mut c, |c| c.account.pending_create.is_none()
        && c.selected().is_some_and(|chat| chat.feed.synchronized)
        && c.account.sessions.iter().any(|s| s.id == session && s.model.is_some())).await;
    until(&mut c, |c| {
        c.selected().unwrap().feed.queue.run_id.is_some()
            && c.selected().unwrap().local.pending.is_empty()
    })
    .await;
    tokio::time::timeout(Duration::from_secs(5), received.recv())
        .await
        .unwrap()
        .unwrap();
    c.draft("queued café 😀".repeat(4096)).unwrap();
    c.send_prompt().unwrap();
    until(&mut c, |c| {
        c.selected().unwrap().feed.queue.requests.len() == 1
    })
    .await;
    let queued = c.selected().unwrap().feed.queue.requests[0].clone();
    let generation = c.selected().unwrap().feed.generation.clone();
    c.control(ClientCommand::QueueControl {
        session_id: session.clone(),
        generation: generation.clone(),
        operation: QueueOperation::Edit {
            request_id: queued.request_id.clone(),
            revision: queued.revision,
            text: "edited queue".into(),
        },
    })
    .unwrap();
    until(&mut c, |c| {
        c.selected().unwrap().feed.queue.requests[0].text == "edited queue"
            && c.selected().unwrap().local.pending.iter().all(|p|!matches!(&p.request.command,
                ClientCommand::QueueControl { operation:QueueOperation::Edit {..},.. }))
    })
    .await;
    assert!(c.selected().unwrap().local.pending.iter().all(|p| !matches!(
        &p.request.command,
        ClientCommand::QueueControl { operation: QueueOperation::Edit { .. }, .. }
    )), "The durable edit receipt must settle before the gated model responds");
    let revision = c.selected().unwrap().feed.queue.requests[0].revision;
    c.control(ClientCommand::QueueControl {
        session_id: session.clone(),
        generation: generation.clone(),
        operation: QueueOperation::Delete {
            request_id: queued.request_id,
            revision,
        },
    })
    .unwrap();
    until(&mut c, |c| {
        c.selected().unwrap().feed.queue.requests.is_empty()
    })
    .await;
    c.control(ClientCommand::Abort {
        session_id: session.clone(),
    })
    .unwrap();
    until(&mut c, |c| {
        c.selected().unwrap().feed.queue.run_id.is_none()
    })
    .await;
    gate.notify_one();
    let file = local.path().join("résumé.txt");
    std::fs::write(&file, "locally attached contents\n").unwrap();
    c.attach(&file, None).unwrap();
    c.draft("Read this file".into()).unwrap();
    c.send_prompt().unwrap();
    until(&mut c, |c| {
        c.selected().unwrap().feed.queue.requests.len() == 1
    })
    .await;
    c.control(ClientCommand::QueueControl {
        session_id: session.clone(),
        generation,
        operation: QueueOperation::Resume { run_id: None },
    })
    .unwrap();
    until(&mut c, |c| {
        c.selected()
            .unwrap()
            .feed
            .events
            .values()
            .any(|e| e.text == "Native reply café 😀" && e.phase == EventPhase::Saved)
    })
    .await;
    assert_eq!(
        std::fs::read_to_string(root.join("inspected.txt")).unwrap(),
        "locally attached contents\n"
    );
    let request = received.recv().await.unwrap();
    assert!(request["messages"].as_array().unwrap().iter().any(|m| {
        m["role"] == "user"
            && m["content"]
                .as_str()
                .is_some_and(|t| t.contains("Attached files are available at:"))
    }));
    assert!(
        !c.selected()
            .unwrap()
            .feed
            .events
            .values()
            .any(|e| e.text.contains("Aborted response must not appear"))
    );
    c.request(ClientCommand::GetSettings).unwrap();
    until(&mut c, |c| c.daemon_settings.is_some()).await;
    let mut draft = daemon_settings::Draft::new(c.daemon_settings.as_ref().unwrap(), c.identity.clone()).unwrap();
    assert_eq!(draft.identity, c.identity);
    assert_eq!(draft.revision, 0);
    assert_eq!(daemon_settings::SECTIONS[draft.section], "Prompts");
    assert_eq!(draft.definition().name, "Default system prompt");
    assert!(!draft.definition().help.is_empty());
    draft.apply("Default exact\n").unwrap();
    draft.field = 1;
    draft.apply("openai-codex/gpt-6-astra").unwrap();
    draft.field = 2;
    assert_eq!(draft.text().unwrap(), "Default exact\n");
    assert!(draft.inherit);
    draft.inherit = false;
    draft.apply("").unwrap();
    assert_eq!(draft.text().unwrap(), "");
    assert!(!draft.inherit);
    assert_eq!(draft.reset().unwrap(), "Default exact\n");
    draft.inherit = false;
    draft.apply("").unwrap();
    draft.section = 0;
    draft.field = 1;
    draft.apply("openai-codex/title-without-metadata").unwrap();
    draft.field = 2;
    draft.apply("\n  exact whitespace  \n").unwrap();
    let document = draft.document().unwrap();
    c.request(ClientCommand::SetSettings {
        revision: document.revision,
        settings: Box::new(document.clone()),
    })
    .unwrap();
    until(&mut c, |c| {
        c.daemon_settings
            .as_ref()
            .is_some_and(|s| s.revision == 1)
    })
    .await;
    let saved = c.daemon_settings.as_ref().unwrap();
    assert_eq!(saved.agent.system_prompt, "Default exact\n");
    assert_eq!(saved.agent.model_system_prompts["openai-codex/gpt-6-astra"], "");
    assert_eq!(saved.daemon.title_model.as_ref().unwrap().model_id, "title-without-metadata");
    assert_eq!(saved.daemon.title_prompt, "\n  exact whitespace  \n");
    c.notice = None;
    c.request(ClientCommand::SetSettings {
        revision: 0,
        settings: Box::new(document),
    })
    .unwrap();
    until(&mut c, |c| c.notice.is_some()).await;
    assert!(
        c.notice
            .as_ref()
            .unwrap()
            .to_lowercase()
            .contains("settings")
    );
    c.draft("durable unsent draft 🦀".into()).unwrap();
    drop(c);
    let mut c =
        Controller::new(Store::open(local.path().into()).unwrap(), Arc::new(|| {})).unwrap();
    assert_eq!(c.selected().unwrap().local.draft, "durable unsent draft 🦀");
    assert!(
        !c.selected().unwrap().feed.events.is_empty(),
        "Verified remote blocks must render from cache before the network reconnects"
    );
    until(&mut c, |c| {
        c.selected().unwrap().feed.synchronized && !c.selected().unwrap().feed.events.is_empty()
    })
    .await;
    let entry = c
        .selected()
        .unwrap()
        .feed
        .events
        .values()
        .find(|e| e.role == EventRole::User)
        .unwrap()
        .entry_id
        .clone();
    c.request(ClientCommand::ForkSession {
        session_id: session.clone(),
        entry_id: entry,
    })
    .unwrap();
    until(&mut c, |c| {
        c.account.selected.as_ref().is_some_and(|s| s != &session)
    })
    .await;
    let child = c.account.selected.clone().unwrap();
    for id in [&child, &session] {
        c.request(ClientCommand::DeleteSession {
            session_id: id.clone(),
        })
        .unwrap();
    }
    until(&mut c, |c| {
        !c.account
            .sessions
            .iter()
            .any(|s| s.id == session || s.id == child)
    })
    .await;
    c.new_chat().unwrap();
    until(&mut c, |c| c.selected().is_some_and(|chat| chat.feed.synchronized)).await;
    let starter = c.account.selected.clone().unwrap();
    c.draft("Keep my draft".into()).unwrap();
    c.chats.get_mut(&starter).unwrap().commands.clear();
    c.chats.get_mut(&starter).unwrap().commands_loaded = false;
    c.choose_model(&starter, "openai-codex/unlisted-exact-id").unwrap();
    c.send_prompt().unwrap();
    let waiting = c.chats[&starter].local.pending.iter().find(|p|p.text=="Keep my draft").unwrap();
    assert!(c.chats[&starter].local.pending.iter().any(|p|p.text=="/model openai-codex/unlisted-exact-id"),"Model selection intent must also be durable");
    assert_eq!(waiting.status,tau_frontend::store::Delivery::WaitingForModel);
    assert_eq!(waiting.text,"Keep my draft");
    assert_eq!(calls.load(Ordering::SeqCst),3,"A send queued behind model selection must not run under the previous model");
    until(&mut c, |c| c.chats[&starter].model_request.is_none()
        && c.account.sessions.iter().any(|s| s.id == starter && s.model.as_ref().is_some_and(|m| m.model_id == "unlisted-exact-id"))
        && c.chats[&starter].local.pending.iter().all(|p| p.status != tau_frontend::store::Delivery::WaitingForModel)).await;
    let previous = tokio::time::timeout(Duration::from_secs(5), received.recv()).await.unwrap().unwrap();
    assert_eq!(previous["model"],"gpt-6-astra");
    let payload = tokio::time::timeout(Duration::from_secs(5), received.recv()).await.unwrap().unwrap();
    assert_eq!(payload["model"],"unlisted-exact-id");
    // Control status can arrive before the separately streamed display state.
    until(&mut c, |c| c.account.sessions.iter().any(|s| s.id == starter && s.status == SessionStatus::Idle)
        && c.chats[&starter].local.pending.is_empty()).await;
    assert!(c.chats[&starter].local.pending.is_empty());
    c.request(ClientCommand::DeleteSession { session_id:starter.clone() }).unwrap();
    until(&mut c, |c| !c.account.sessions.iter().any(|s| s.id == starter)).await;
    assert_eq!(calls.load(Ordering::SeqCst), 4);
    assert!(!root.join("state.json").exists() && !root.join("sessions").exists());
    drop(c);
    task.abort();
    let _ = task.await;
    provider.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn files_share_native_blocks_authorization_and_verified_offline_cache() {
    use axum::{Router,extract::{State,WebSocketUpgrade},http::{HeaderMap,StatusCode},routing::get};
    use futures_util::FutureExt;
    use std::sync::{Mutex,atomic::{AtomicUsize,Ordering}};
    use tau_transfer::blocks::{Backend,Server};
    use tau_blocks::*;
    struct Data {db:Arc<Mutex<rusqlite::Connection>>,changes:tokio::sync::watch::Sender<u64>,reads:AtomicUsize}
    impl Backend for Data {
        fn feed(&self, req:FeedRequest) -> futures_util::future::BoxFuture<'static,anyhow::Result<FeedPage>> {
            let db=self.db.clone(); async move {tau_blocks::feed(&db.lock().unwrap(),&req)}.boxed()
        }
        fn read(&self, req:BlockRequest) -> futures_util::future::BoxFuture<'static,anyhow::Result<ContentRange>> {
            self.reads.fetch_add(1,Ordering::SeqCst);
            let db=self.db.clone(); async move {tau_blocks::read(&db.lock().unwrap(),&req)}.boxed()
        }
        fn changes(&self) -> tokio::sync::watch::Receiver<u64> {self.changes.subscribe()}
    }
    let root=tempfile::tempdir().unwrap();
    let bytes=(0..150_000).map(|n|(n%251)as u8).collect::<Vec<_>>();
    let mut db=rusqlite::Connection::open_in_memory().unwrap();
    db.execute_batch("PRAGMA foreign_keys=ON").unwrap();tau_blocks::initialize(&db).unwrap();
    use sha2::Digest;
    let tx=db.transaction().unwrap();
    for entry in ["entry?escaped","second"] {
        tau_blocks::put(&tx,"chat/escaped",BlockHeader {id:format!("file:{entry}"),parent:None,order:0,kind:BlockKind::File,
            meta:serde_json::json!({"sha256":format!("{:x}",sha2::Sha256::digest(&bytes))}),version:0,length:0,sealed:true,revision:0},&bytes).unwrap();
    }
    tx.commit().unwrap();
    let lineage=tau_blocks::cursor(&db).unwrap().lineage;
    let data=Arc::new(Data {db:Arc::new(Mutex::new(db)),changes:tokio::sync::watch::channel(0).0,reads:AtomicUsize::new(0)});
    let bulk=Arc::new(Server::bind("127.0.0.1:0".parse().unwrap(),data.clone()).await.unwrap());
    #[derive(Clone)] struct Peer {bulk:Arc<Server>,lineage:String,grants:Arc<AtomicUsize>,http:Arc<AtomicUsize>}
    async fn ws(State(peer):State<Peer>, headers:HeaderMap, ws:WebSocketUpgrade) -> impl axum::response::IntoResponse {
        assert_eq!(headers["authorization"],"Bearer fixture-token");
        ws.on_upgrade(move |mut ws|async move {
            ws.send(axum::extract::ws::Message::Text(serde_json::to_string(&ServerMessage::Hello {protocol_version:PROTOCOL_VERSION,daemon_version:"fixture".into(),lineage:Some("fixture".into())}).unwrap().into())).await.unwrap();
            while let Some(Ok(frame))=ws.recv().await {
                let axum::extract::ws::Message::Text(text)=frame else {continue;};
                let req:ClientRequest=serde_json::from_str(&text).unwrap();
                if let ClientCommand::ConnectBlocks {node_id}=req.command {
                    peer.grants.fetch_add(1,Ordering::SeqCst);
                    let offer=peer.bulk.authorize(&node_id,peer.lineage.clone()).unwrap();
                    ws.send(axum::extract::ws::Message::Text(serde_json::to_string(&ServerMessage::BlockConnection {offer}).unwrap().into())).await.unwrap();
                }
            }
        })
    }
    async fn forbidden(State(peer):State<Peer>) -> StatusCode {peer.http.fetch_add(1,Ordering::SeqCst);StatusCode::GONE}
    let peer=Peer {bulk:bulk.clone(),lineage,grants:Arc::new(AtomicUsize::new(0)),http:Arc::new(AtomicUsize::new(0))};
    let app=Router::new().route("/v1/ws",get(ws)).route("/v1/sessions/{session}/attachments/{entry}",get(forbidden)).with_state(peer.clone());
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let address=listener.local_addr().unwrap();
    let server=tokio::spawn(async move {axum::serve(listener,app).await.unwrap()});
    let store=Store::open(root.path().join("local")).unwrap();
    store.put("","settings",&Settings {server_url:format!("http://{address}"),token:"fixture-token".into()}).unwrap();
    let mut c=Controller::new(store,Arc::new(||{})).unwrap();
    until(&mut c,|c|c.epoch.is_some() && c.content_authorized()).await;
    assert_eq!(data.reads.load(Ordering::SeqCst),0,"Authorization alone must not read a file");
    for entry in ["entry?escaped","second"] {
        let target=c.download("chat/escaped",entry,50_000_000).unwrap();
        let key=Controller::download_key("chat/escaped",entry);
        until(&mut c,|c|c.downloads.get(&key).is_some_and(|d|d.status.done)).await;
        assert!(c.downloads[&key].status.failure.is_none(),"{:?}",c.downloads[&key].status.failure);
        assert_eq!(std::fs::read(&target).unwrap(),bytes);
    }
    assert_eq!(peer.grants.load(Ordering::SeqCst),1,"Both files reuse the authenticated endpoint");
    assert_eq!(peer.http.load(Ordering::SeqCst),0,"Files do not use the old HTTP/per-file grant path");
    let target=c.attachment_path("chat/escaped","entry?escaped");
    c.epoch=None;
    assert_eq!(c.download("chat/escaped","entry?escaped",50_000_000).unwrap(),target);
    // An exported file is not trusted merely because its size is unchanged.
    std::fs::write(&target,vec![0;bytes.len()]).unwrap();
    let before=data.reads.load(Ordering::SeqCst);
    c.download("chat/escaped","entry?escaped",50_000_000).unwrap();
    until(&mut c,|c|c.downloads[&Controller::download_key("chat/escaped","entry?escaped")].status.done).await;
    assert_eq!(std::fs::read(&target).unwrap(),bytes);
    assert_eq!(data.reads.load(Ordering::SeqCst),before,"Repair an exported file from verified cache, without network reads");
    drop(c);server.abort();bulk.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn projects_sync_between_real_clients_preserve_drafts_and_recover_selection() {
    let server = tempfile::tempdir().unwrap();
    let root = server.path();
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
    let config = taud::Config { bind:port, transfer_bind:"127.0.0.1:0".parse().unwrap(), transfer_bind_v6:None, token:Arc::from("local-project-fixture"),
        settings_path:root.join("settings.json"), import_pi_dir:None, codex_auth_source:None, cwd:root.into(), database_path:root.join("tau.sqlite3"),
        telemetry_path:root.join("crashes.jsonl"), attachment_root:root.join("outbox"), upload_root:root.join("uploads") };
    let task = tokio::spawn(taud::run(config));
    let a_root = tempfile::tempdir().unwrap(); let b_root = tempfile::tempdir().unwrap();
    let connect = |path: &std::path::Path| {
        let store = Store::open(path.into()).unwrap();
        store.put("","settings",&Settings { server_url:format!("http://{port}"),token:"local-project-fixture".into() }).unwrap();
        Controller::new(store,Arc::new(|| {})).unwrap()
    };
    let mut a = connect(a_root.path()); let mut b = connect(b_root.path());
    until(&mut a, |c| c.epoch.is_some()).await; until(&mut b, |c| c.epoch.is_some()).await;
    a.new_chat().unwrap();
    until(&mut a, |c| c.selected().is_some_and(|chat| chat.feed.synchronized) && c.account.sessions.len() == 1).await;
    let general = a.account.selected.clone().unwrap();
    a.draft("Do not lose this unsent draft".into()).unwrap();
    let file = root.join("draft.txt"); std::fs::write(&file,"attachment").unwrap(); a.attach(&file,None).unwrap();
    let project = uuid::Uuid::new_v4().to_string();
    a.request(ClientCommand::CreateProject { project_id:project.clone(),name:"Build".into(),prompt:"Exact\n  project text\n".into() }).unwrap();
    until(&mut a, |c| c.account.selected_project == project && c.account.projects.iter().any(|p| p.id == project)).await;
    until(&mut b, |c| c.account.projects.iter().any(|p| p.id == project)).await;
    a.new_chat().unwrap();
    until(&mut a, |c| c.selected().is_some_and(|chat| chat.feed.synchronized) && c.account.sessions.len() == 2).await;
    let work = a.account.selected.clone().unwrap(); assert_ne!(general,work);
    assert_eq!(a.account.sessions.iter().find(|s| s.id == work).unwrap().project_id,project);
    a.request(ClientCommand::MoveSession { session_id:general.clone(),project_id:project.clone() }).unwrap();
    until(&mut a, |c| c.account.sessions.iter().filter(|s| s.project_id == project).count() == 2).await;
    assert_eq!(a.account.selected.as_deref(),Some(work.as_str()),"Moving a clicked chat must not retarget the selected chat");
    assert_eq!(a.chats[&general].local.draft,"Do not lose this unsent draft");
    assert_eq!(a.chats[&general].local.files.len(),1);
    a.request(ClientCommand::RenameSession { session_id:work.clone(),title:"Work started".into() }).unwrap();
    until(&mut b, |c| c.account.sessions.iter().any(|s| s.id == work && s.title == "Work started") && c.account.sessions.iter().all(|s| s.project_id == project)).await;
    assert!(b.project_unread(&project));
    b.select_project(&project).unwrap(); assert!(b.project_unread(&project),"Opening the tab does not mark its chats read");
    b.select(&general).unwrap(); b.select(&work).unwrap(); assert!(!b.project_unread(&project));
    b.request(ClientCommand::UpdateProject { project_id:project.clone(),revision:0,name:"Build renamed".into(),prompt:"Revised".into() }).unwrap();
    until(&mut a, |c| c.account.projects.iter().any(|p| p.id == project && p.revision == 1)).await;
    a.request(ClientCommand::UpdateProject { project_id:project.clone(),revision:0,name:"Stale".into(),prompt:"Wrong".into() }).unwrap();
    until(&mut a, |c| c.project_result.as_ref().is_some_and(|(_,ok)| !ok)).await;
    assert_eq!(a.account.projects.iter().find(|p| p.id == project).unwrap().prompt,"Revised");
    drop(b);
    let mut b = connect(b_root.path());
    assert_eq!(b.account.selected_project,project);
    until(&mut b, |c| c.epoch.is_some() && c.selected().is_some_and(|chat| chat.feed.synchronized)).await;
    a.request(ClientCommand::DeleteProject { project_id:project.clone(),revision:1,mode:DeleteProjectMode::MoveToGeneral }).unwrap();
    until(&mut b, |c| c.account.projects.len() == 1 && c.account.sessions.iter().all(|s| s.project_id == GENERAL_PROJECT_ID)).await;
    until(&mut a, |c| c.account.projects.len() == 1 && c.account.sessions.iter().all(|s| s.project_id == GENERAL_PROJECT_ID)).await;
    assert_eq!(b.account.selected_project,GENERAL_PROJECT_ID);
    assert_eq!(a.chats[&general].local.files.len(),1);
    a.request(ClientCommand::CreateProject { project_id:project.clone(),name:"Temporary".into(),prompt:String::new() }).unwrap();
    until(&mut a, |c| c.account.projects.len() == 2 && c.account.selected_project == project).await;
    a.request(ClientCommand::MoveSession { session_id:general.clone(),project_id:project.clone() }).unwrap();
    until(&mut a, |c| c.account.sessions.iter().any(|s| s.id == general && s.project_id == project)).await;
    until(&mut b, |c| c.account.sessions.iter().any(|s| s.id == general && s.project_id == project)).await;
    b.select(&general).unwrap();
    a.request(ClientCommand::DeleteProject { project_id:project.clone(),revision:0,mode:DeleteProjectMode::DeleteChats }).unwrap();
    until(&mut a, |c| c.account.projects.len() == 1 && c.project_result.as_ref().is_some_and(|(_,ok)| *ok) && !c.chats.contains_key(&general)).await;
    until(&mut b, |c| c.account.selected.is_none() && c.account.sessions.len() == 1).await;
    assert!(!a.store.load_chat(&a.identity,&general).unwrap().has_work());
    assert_eq!(b.account.sessions[0].id,work,"Unrelated General chats survive");
    drop(a); drop(b); task.abort(); let _ = task.await;
}
