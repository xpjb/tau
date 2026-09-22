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
        assert!(index < 3, "Unexpected extra provider execution");
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
    daemon_settings.agent.load_project_instructions = false;
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
        transfer_bind: "127.0.0.1:0".parse().unwrap(),
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
    until(&mut c, |c| {
        c.selected().is_some_and(|chat| chat.feed.synchronized)
    })
    .await;
    let session = c.account.selected.clone().unwrap();
    assert!(
        c.account
            .sessions
            .iter()
            .any(|s| s.id == session && s.model.is_some())
    );
    c.draft("hold".into()).unwrap();
    c.send_prompt().unwrap();
    until(&mut c, |c| {
        c.selected().unwrap().feed.queue.run_id.is_some()
            && c.selected().unwrap().local.pending.is_empty()
    })
    .await;
    tokio::time::timeout(Duration::from_secs(5), received.recv())
        .await
        .unwrap()
        .unwrap();
    c.draft("queued café 😀".into()).unwrap();
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
    })
    .await;
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
    let mut document = c.daemon_settings.as_ref().unwrap().0.clone();
    document.daemon.title_prompt = "\n  exact whitespace  \n".into();
    document.agent.system_prompt = Some(String::new());
    c.request(ClientCommand::SetSettings {
        revision: document.revision,
        settings: Box::new(document.clone()),
    })
    .unwrap();
    until(&mut c, |c| {
        c.daemon_settings
            .as_ref()
            .is_some_and(|(s, _)| s.revision == 1)
    })
    .await;
    assert_eq!(
        c.daemon_settings
            .as_ref()
            .unwrap()
            .0
            .agent
            .system_prompt
            .as_deref(),
        Some("")
    );
    assert_eq!(
        c.daemon_settings.as_ref().unwrap().0.daemon.title_prompt,
        "\n  exact whitespace  \n"
    );
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
        c.selected().unwrap().feed.events.is_empty(),
        "Remote history must not become a second writable local store"
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
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    assert!(!root.join("state.json").exists() && !root.join("sessions").exists());
    drop(c);
    task.abort();
    let _ = task.await;
    provider.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_grant_to_native_quic_and_offline_cache() {
    use axum::{
        Json, Router,
        extract::{Query, State},
        http::{HeaderMap, StatusCode},
        routing::get,
    };
    use std::collections::HashMap;
    #[derive(Clone)]
    struct Grant {
        provider: Arc<tau_transfer::TransferProvider>,
        source: std::path::PathBuf,
        hits: Arc<std::sync::atomic::AtomicUsize>,
    }
    async fn grant(
        State(state): State<Grant>,
        headers: HeaderMap,
        Query(query): Query<HashMap<String, String>>,
    ) -> Result<Json<tau_transfer::TransferOffer>, StatusCode> {
        assert_eq!(headers["authorization"], "Bearer fixture-token");
        state.hits.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(Json(
            state
                .provider
                .offer(
                    std::fs::File::open(state.source).unwrap(),
                    &query["transferNode"],
                    50_000_000,
                )
                .await
                .unwrap(),
        ))
    }
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("original");
    let bytes = (0..150_000).map(|n| (n % 251) as u8).collect::<Vec<_>>();
    std::fs::write(&source, &bytes).unwrap();
    let provider = Arc::new(
        tau_transfer::TransferProvider::bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap(),
    );
    let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let app = Router::new()
        .route("/v1/sessions/{session}/attachments/{entry}", get(grant))
        .with_state(Grant {
            provider: provider.clone(),
            source,
            hits: hits.clone(),
        });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    // The production download path shares its event loop with a real protocol peer.
    async fn ws(ws: axum::extract::WebSocketUpgrade) -> impl axum::response::IntoResponse {
        ws.on_upgrade(|mut ws| async move {
            ws.send(axum::extract::ws::Message::Text(
                serde_json::to_string(&tau_protocol::ServerMessage::Hello {
                    protocol_version: tau_protocol::PROTOCOL_VERSION,
                    daemon_version: "fixture".into(),
                })
                .unwrap()
                .into(),
            ))
            .await
            .unwrap();
            while ws.recv().await.is_some() {}
        })
    }
    let app = app.route("/v1/ws", get(ws));
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let store = Store::open(root.path().join("local")).unwrap();
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
    until(&mut c, |c| c.epoch.is_some()).await;
    let target = c
        .download("chat/escaped", "entry?escaped", 50_000_000)
        .unwrap();
    until(&mut c, |c| c.downloads.values().any(|d| d.status.done)).await;
    let d = c.downloads.values().next().unwrap();
    assert!(d.status.failure.is_none(), "{:?}", d.status.failure);
    assert_eq!(std::fs::read(&target).unwrap(), bytes);
    c.epoch = None;
    assert_eq!(
        c.download("chat/escaped", "entry?escaped", 50_000_000)
            .unwrap(),
        target
    );
    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 1);
    drop(c);
    server.abort();
    provider.shutdown().await;
}
