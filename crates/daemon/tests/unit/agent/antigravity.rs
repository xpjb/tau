use super::{AgentManager, Client, Config, Settings, StateStore, Value, json, serve};
use std::{os::unix::fs::PermissionsExt, path::Path, time::Duration};
use tau_net::SessionStatus;

async fn fixture() -> (tempfile::TempDir, Config, AgentManager) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path();
    let cwd = path.join("workspace");
    let home = path.join("home");
    let bin = path.join("agy");
    std::fs::create_dir(&cwd).unwrap();
    std::fs::create_dir(&home).unwrap();
    std::fs::write(&bin, include_bytes!("../../support/fake-agy.py")).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o700)).unwrap();
    let config = Config {
        bind: "127.0.0.1:0".parse().unwrap(), transfer_bind: "127.0.0.1:0".parse().unwrap(), transfer_bind_v6: None,
        settings_path: path.join("settings.json"), import_pi_dir: None, codex_auth_source: None,
        antigravity_bin: Some(bin), antigravity_home: Some(home), cwd,
        database_path: path.join("tau.sqlite3"), telemetry_path: path.join("crashes.jsonl"),
        attachment_root: path.join("outbox"), upload_root: path.join("uploads"),
    };
    let mut settings = Settings::default();
    settings.daemon.generate_titles = true;
    settings.daemon.idle_timeout_seconds = 0;
    std::fs::write(&config.settings_path, serde_json::to_vec(&settings).unwrap()).unwrap();
    let manager = AgentManager::new(config.clone(), StateStore::load(config.database_path.clone()).await.unwrap()).await.unwrap();
    (root, config, manager)
}

async fn state(manager: &AgentManager, id: &str, expected: SessionStatus) -> Option<String> {
    let runtime = manager.runtime(id).await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let snapshot = runtime.snapshot();
            if snapshot.status == expected { return snapshot.detail; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.expect("chat did not settle as expected")
}

fn wire(cwd: &Path, method: &str) -> Vec<Value> {
    std::fs::read_to_string(cwd.join("wire.jsonl")).unwrap().lines().map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|frame| frame["method"] == method).collect()
}

#[tokio::test]
async fn catalog_stream_queue_restart_and_fork_use_the_same_backend() {
    let (_root, config, manager) = fixture().await;
    let (url, server) = serve(&manager).await;
    let mut client = Client::connect(&url).await;
    assert!(manager.model_catalog().models.iter().any(|model| model.value == "antigravity/default"));
    let refresh = client.request(json!({"id":"refresh","type":"refresh_model_catalog","provider":"antigravity"})).await;
    assert_eq!(refresh["ok"], true, "{refresh}");
    assert!(manager.model_catalog().models.iter().any(|model| model.value == "antigravity/gemini-fixture"));
    assert!(wire(&config.cwd, "session/prompt").is_empty(), "Catalog refresh must not run a prompt");
    let id = client.request(json!({"id":"create","type":"create_session"})).await["sessionId"].as_str().unwrap().to_owned();
    client.open(&id).await;
    let response = client.request(json!({"id":"first","type":"prompt","sessionId":id,"text":"hold first",
        "model":{"provider":"antigravity","modelId":"gemini-fixture"}})).await;
    assert_eq!(response["ok"], true, "{response}");
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let page = client.page(&id, None).await;
            if page["events"].as_array().unwrap().iter().any(|event| event["phase"] == "live" && event["text"] == "Streamed hold first with gemini-fixture") { break; }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.expect("reply should arrive before the turn is finished");
    assert_eq!(client.request(json!({"id":"second","type":"prompt","sessionId":id,"text":"queued second"})).await["disposition"], "queued");
    std::fs::write(config.cwd.join("release-first"), "").unwrap();
    state(&manager, &id, SessionStatus::Idle).await;
    let page = client.open(&id).await;
    let events = page["events"].as_array().unwrap();
    assert!(events.iter().any(|e| e["phase"] == "saved" && e["text"] == "Streamed queued second with gemini-fixture — done"));
    assert!(events.iter().any(|e| e["kind"] == "thinking" && e["text"] == "Thinking π"));
    assert!(events.iter().any(|e| e["kind"] == "tool" && e["toolName"] == "Antigravity · failed" && e["text"] == "Run fixture"));
    let transcript = manager.inner.state.get(&id).await.unwrap().unwrap();
    assert!(!transcript.needs_turn, "external tool activity is not a new Tau tool request");
    assert_eq!(transcript.tokens, Some(24));
    let first = wire(&config.cwd, "session/prompt");
    assert_eq!(first.len(), 2, "Automatic titles must not open an agent turn");
    let session = &first[0]["params"]["sessionId"];
    assert_eq!(first[1]["params"]["sessionId"], *session);
    assert_eq!(first[1]["params"]["prompt"], json!([{"type":"text","text":"queued second"}]));
    manager.shutdown().await;
    server.abort();
    drop(client);
    drop(manager);

    let manager = AgentManager::new(config.clone(), StateStore::load(config.database_path.clone()).await.unwrap()).await.unwrap();
    manager.prompt(&id, "after restart", "third").await.unwrap();
    state(&manager, &id, SessionStatus::Idle).await;
    let fork = manager.clone_session(&id).await.unwrap();
    manager.prompt(&fork, "different chat", "fork-prompt").await.unwrap();
    state(&manager, &fork, SessionStatus::Idle).await;
    let prompts = wire(&config.cwd, "session/prompt");
    assert_eq!(prompts.len(), 4);
    assert_eq!(prompts[2]["params"]["sessionId"], *session);
    assert_ne!(prompts[3]["params"]["sessionId"], *session);
    assert!(prompts[3]["params"]["prompt"][0]["text"].as_str().unwrap().contains("Streamed after restart"));
    assert_eq!(prompts[3]["params"]["prompt"][1]["text"], "different chat");
    std::fs::write(config.cwd.join("missing-cache"), "").unwrap();
    manager.prompt(&id, "lost CLI cache", "cache").await.unwrap();
    state(&manager, &id, SessionStatus::Idle).await;
    let prompts = wire(&config.cwd, "session/prompt");
    assert_eq!(prompts.len(), 5);
    assert_ne!(prompts[4]["params"]["sessionId"], *session);
    assert!(prompts[4]["params"]["prompt"][0]["text"].as_str().unwrap().contains("Streamed after restart"));
    assert_eq!(wire(&config.cwd, "session/set_config_option").len(), 5, "Chosen model must be set after every open, including resume");
    manager.shutdown().await;
}

#[tokio::test]
async fn login_and_unconfirmed_stop_pause_without_retrying_or_reusing_an_uncommitted_session() {
    let (_root, config, manager) = fixture().await;
    let id = manager.create_session(None, "general").await.unwrap();
    let runtime = manager.runtime(&id).await.unwrap();
    let model = "antigravity/default".parse().unwrap();
    std::fs::write(config.cwd.join("needs-login"), "").unwrap();
    manager.prompt_with_model(&id, "wait for stop", "first", Some(&model)).await.unwrap();
    assert!(state(&manager, &id, SessionStatus::Error).await.unwrap().contains("agy acp --login"));
    assert!(wire(&config.cwd, "session/prompt").is_empty());
    assert!(manager.inner.state.queue(&id).await.unwrap().paused);
    std::fs::remove_file(config.cwd.join("needs-login")).unwrap();
    let generation = runtime.content.lock().await.transcript.as_ref().unwrap().generation.clone();
    manager.queue_control(&id, &generation, "play", tau_net::QueueOperation::Resume { run_id: None }).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if runtime.content.lock().await.transcript.as_ref().unwrap().events().any(|e| e.text == "Streamed wait for stop with default") { break; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.unwrap();
    manager.abort(&id, "stop").await.unwrap();
    let detail = state(&manager, &id, SessionStatus::Idle).await.unwrap();
    assert!(detail.contains("Tau did not retry"), "{detail}");
    assert!(manager.inner.state.queue(&id).await.unwrap().paused);
    assert_eq!(wire(&config.cwd, "session/prompt").len(), 1);
    assert_eq!(wire(&config.cwd, "session/cancel").len(), 1);
    assert!(wire(&config.cwd, "session/resume").is_empty());
    let last = manager.inner.state.get(&id).await.unwrap().unwrap().head.unwrap();
    let saved = manager.inner.state.entry(&id, &last).await.unwrap();
    assert_eq!(saved["message"]["stopReason"], "aborted");
    assert!(saved["message"]["tauAntigravitySession"].is_null());
    manager.prompt(&id, "new direction", "next").await.unwrap();
    manager.queue_control(&id, &generation, "continue", tau_net::QueueOperation::Resume { run_id: None }).await.unwrap();
    state(&manager, &id, SessionStatus::Idle).await;
    let prompts = wire(&config.cwd, "session/prompt");
    assert_eq!(prompts.len(), 2);
    assert_ne!(prompts[0]["params"]["sessionId"], prompts[1]["params"]["sessionId"]);
    assert!(prompts[1]["params"]["prompt"][0]["text"].as_str().unwrap().contains("external effects"));
    manager.shutdown().await;
}
