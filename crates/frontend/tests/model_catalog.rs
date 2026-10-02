#![cfg(unix)]
//! Manually entered quick model, real controller/daemon, local provider only.
use std::{sync::{Arc, atomic::{AtomicUsize, Ordering}}, time::{Duration, Instant}};
use axum::{Json, Router, extract::State, routing::{get, post}};
use serde_json::{Value, json};
use tau_frontend::{controller::Controller, store::{Settings, Store}};
use tau_protocol::*;

async fn until(c: &mut Controller, condition: impl Fn(&Controller) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(12);
    loop {
        c.poll();
        if condition(c) { return; }
        assert!(Instant::now() < deadline, "Timed out: {}, {:?}", c.connection, c.notice);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[derive(Clone)]
struct Provider {
    catalog_calls: Arc<AtomicUsize>,
    turn_calls: Arc<AtomicUsize>,
    catalog_gate: Arc<tokio::sync::Notify>,
}
async fn catalog(State(p): State<Provider>) -> Json<Value> {
    p.catalog_calls.fetch_add(1, Ordering::SeqCst);
    p.catalog_gate.notified().await;
    Json(json!({"data":[{"id":"gpt-6-astra","context_length":200000}, {"id":"gpt-6.1-sol","context_length":272000}]}))
}
async fn turn(State(p): State<Provider>, Json(body): Json<Value>) -> impl axum::response::IntoResponse {
    assert_eq!(body["model"], "gpt-6.1-sol", "Manual choices must reach the provider unchanged");
    assert_eq!(p.turn_calls.fetch_add(1, Ordering::SeqCst), 0, "No metadata request may execute a turn");
    ([ ("content-type", "text/event-stream") ], format!("data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({"choices":[{"index":0,"delta":{"content":"Reply"}}]}),
        json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"total_tokens":1024}})))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn manual_quick_model_refreshes_catalog_and_keeps_reported_tokens_visible() {
    let server_root = tempfile::tempdir().unwrap(); let root = server_root.path();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let script = Provider {catalog_calls:Arc::new(AtomicUsize::new(0)), turn_calls:Arc::new(AtomicUsize::new(0)), catalog_gate:Arc::new(tokio::sync::Notify::new())};
    let app = Router::new().route("/models", get(catalog)).route("/{*path}", post(turn)).with_state(script.clone());
    let provider = tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
    let mut settings = tau_protocol::settings::Settings::default();
    settings.daemon.idle_timeout_seconds = 0; settings.agent.load_agents_files = false;
    let endpoint = settings.providers.get_mut("openai-codex").unwrap();
    endpoint.api = tau_protocol::settings::Api::ChatCompletions; endpoint.base_url = base_url.clone(); endpoint.web_search = false;
    std::fs::write(root.join("settings.json"), serde_json::to_vec(&settings).unwrap()).unwrap();
    std::fs::write(root.join("auth.json"), r#"{"openai-codex":{"type":"api_key","key":"fixture-key"}}"#).unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(root.join("auth.json"), std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;
    // A still-fresh authenticated cache predates the newly entered model.
    // Identity is URL-safe base64(SHA-256("fixture-key\0")), not a credential.
    std::fs::write(root.join("model-catalog.json"), json!({"schema":1,"providers":[{
        "provider":"openai-codex","baseUrl":base_url,"api":"chat_completions",
        "identity":"6py8QlDovDWOSnihdSWKaqiDJZn5SFHLDIjeYXpi2RE","fetchedAtMs":now-300000,
        "windows":{"gpt-6-astra":200000}
    }]}).to_string()).unwrap();
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
    let token = "isolated-catalog-test-token";
    let daemon = tokio::spawn(taud::run(taud::Config {bind:port, transfer_bind:"127.0.0.1:0".parse().unwrap(), transfer_bind_v6:None,
        token:Arc::from(token), settings_path:root.join("settings.json"), import_pi_dir:None, codex_auth_source:None,
        cwd:root.into(), database_path:root.join("tau.sqlite3"), telemetry_path:root.join("crash.jsonl"),
        attachment_root:root.join("outbox"), upload_root:root.join("uploads")}));
    let local = tempfile::tempdir().unwrap(); let store = Store::open(local.path().into()).unwrap();
    store.put("", "settings", &Settings {server_url:format!("http://{port}"),token:token.into()}).unwrap();
    let mut c = Controller::new(store, Arc::new(|| {})).unwrap();
    until(&mut c, |c| c.epoch.is_some()).await;
    c.new_chat().unwrap(); let session = c.account.selected.clone().unwrap();
    until(&mut c, |c| c.account.pending_create.is_none() && c.quick_start(&session)
        && c.account.sessions.iter().any(|s| s.id == session && s.context_usage.is_some_and(|u| u.context_window == Some(200000)))).await;
    assert_eq!(script.catalog_calls.load(Ordering::SeqCst), 0, "A fresh known model needs no GET");
    let slug = "openai-codex/gpt-6.1-sol";
    let mut preferences = c.model_preferences.clone(); preferences.slugs = vec![slug.into()];
    c.save_model_preferences(preferences).unwrap();
    c.draft("Keep my draft while selecting a manual model".into()).unwrap();
    c.choose_model(&session, slug).unwrap();
    assert_eq!(c.selected_model(&session), Some(&slug.parse().unwrap()));
    assert!(c.selected().unwrap().local.pending.is_empty(), "Picking a model has no network command");
    assert_eq!(c.selected().unwrap().local.draft, "Keep my draft while selecting a manual model");
    c.send_prompt().unwrap();
    until(&mut c, |c| c.account.sessions.iter().any(|s| s.id == session && s.status == SessionStatus::Idle
        && s.context_usage == Some(ContextUsage {tokens:Some(1024), context_window:None}))).await;
    // Metadata is asynchronous: reported tokens remain visible before its GET completes.
    script.catalog_gate.notify_one();
    until(&mut c, |c| c.account.sessions.iter().any(|s| s.id == session && s.context_usage == Some(ContextUsage {tokens:Some(1024), context_window:Some(272000)}))).await;
    assert_eq!(script.catalog_calls.load(Ordering::SeqCst), 1, "Model misses coalesce across lists, reads and turns");
    assert_eq!(script.turn_calls.load(Ordering::SeqCst), 1);
    assert_eq!(c.model_preferences.slugs, [slug]);
    until(&mut c, |c| c.model_catalog.models.iter().any(|m| m.value == slug)).await;
    drop(c);
    // The account cache works without a selected/loaded chat or another GET.
    let store = Store::open(local.path().into()).unwrap();
    let settings: Settings = store.get("", "settings").unwrap();
    let cached: ModelCatalog = store.get(&settings.identity(), "model-catalog").unwrap();
    assert!(cached.models.iter().any(|m| m.value == slug));
    daemon.abort(); provider.abort();
}
