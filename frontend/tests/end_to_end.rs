#![cfg(unix)]
//! Production controller/transport/store against the real daemon and its existing
//! deterministic Pi subprocess fixture. No GPU, mock controller, or live account.
use std::{
    os::unix::fs::PermissionsExt,
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
            "Timed out: connection={}, notice={:?}",
            c.connection,
            c.notice
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_daemon_chat_queue_upload_extension_fork_and_restart() {
    let server = tempfile::tempdir().unwrap();
    let root = server.path();
    let pi = root.join("pi.py");
    std::fs::write(&pi, include_str!("../../daemon/tests/fixtures/pi.py")).unwrap();
    std::fs::set_permissions(&pi, std::fs::Permissions::from_mode(0o700)).unwrap();
    let extension = root.join("extension.ts");
    std::fs::write(&extension, "").unwrap();
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let token = "fixture-bearer-with-at-least-thirty-two-characters";
    let config = taud::Config {
        bind: port,
        transfer_bind: "127.0.0.1:0".parse().unwrap(),
        token: Arc::from(token),
        pi_command: pi,
        default_thinking_level: "high".into(),
        cwd: root.into(),
        state_path: root.join("state.json"),
        session_dir: root.join("sessions"),
        telemetry_path: root.join("crash.jsonl"),
        pi_extension_path: extension,
        attachment_root: root.join("outbox"),
        upload_root: root.join("uploads"),
        title_command: None,
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
    c.draft("queued café 😀".into()).unwrap();
    c.send_prompt().unwrap();
    until(&mut c, |c| {
        c.selected().unwrap().feed.queue.requests.len() == 1
    })
    .await;
    let q = c.selected().unwrap().feed.queue.requests[0].clone();
    let generation = c.selected().unwrap().feed.generation.clone();
    c.control(ClientCommand::QueueControl {
        session_id: session.clone(),
        generation: generation.clone(),
        operation: QueueOperation::Edit {
            request_id: q.request_id.clone(),
            revision: q.revision,
            text: "edited queue".into(),
        },
    })
    .unwrap();
    until(&mut c, |c| {
        c.selected().unwrap().feed.queue.requests[0].text == "edited queue"
    })
    .await;
    let rev = c.selected().unwrap().feed.queue.requests[0].revision;
    c.control(ClientCommand::QueueControl {
        session_id: session.clone(),
        generation,
        operation: QueueOperation::Delete {
            request_id: q.request_id,
            revision: rev,
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
    let file = local.path().join("résumé.txt");
    std::fs::write(&file, "locally attached contents\n").unwrap();
    c.attach(&file, None).unwrap();
    c.draft("Read this file".into()).unwrap();
    c.send_prompt().unwrap();
    until(&mut c, |c| {
        c.selected().unwrap().feed.events.values().any(|e| {
            e.role == EventRole::User && e.text.contains("Attached files are available at:")
        })
    })
    .await;
    assert!(
        std::fs::read_dir(root.join("uploads").join(&session))
            .unwrap()
            .next()
            .is_some()
    );
    c.draft("/choose".into()).unwrap();
    c.send_prompt().unwrap();
    until(&mut c, |c| !c.dialogs.is_empty()).await;
    let (_, dialog) = c.dialogs[0].clone();
    c.extension_response(session.clone(), dialog.id, Some("Two".into()), None, false)
        .unwrap();
    until(&mut c, |c| c.selected().unwrap().local.pending.is_empty()).await;
    assert_eq!(
        std::fs::read_to_string(root.join("sessions").join("extension-response")).unwrap(),
        "Two"
    );
    c.request(ClientCommand::SetTitlePrompt {
        prompt: "\n  exact whitespace  \n".into(),
    })
    .unwrap();
    c.request(ClientCommand::GetTitlePrompt).unwrap();
    until(&mut c, |c| {
        c.title_prompt
            .as_ref()
            .is_some_and(|p| p.0 == "\n  exact whitespace  \n")
    })
    .await;
    c.draft("durable unsent draft 🦀".into()).unwrap();
    drop(c);
    let mut c =
        Controller::new(Store::open(local.path().into()).unwrap(), Arc::new(|| {})).unwrap();
    assert_eq!(c.selected().unwrap().local.draft, "durable unsent draft 🦀");
    assert!(
        c.selected().unwrap().feed.events.is_empty(),
        "remote history was not persisted"
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
    // Deletion shuts down both fixture workers before ending the server task.
    c.request(ClientCommand::DeleteSession {
        session_id: child.clone(),
    })
    .unwrap();
    c.request(ClientCommand::DeleteSession {
        session_id: session.clone(),
    })
    .unwrap();
    until(&mut c, |c| {
        !c.account
            .sessions
            .iter()
            .any(|s| s.id == session || s.id == child)
    })
    .await;
    drop(c);
    task.abort();
    let _ = task.await;
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
                r#"{"type":"hello","protocolVersion":10,"daemonVersion":"fixture"}"#.into(),
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
