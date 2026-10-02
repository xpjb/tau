//! Per-chat thinking metadata follows durable commands, not mutable defaults.
use super::*;
use crate::protocol::{ServerMessage, SessionStatus, SessionSummary};

async fn page(manager: &AgentManager) -> (u64, Vec<SessionSummary>) {
    let ServerMessage::SessionPage { revision, sessions, .. } = manager.list_page("thinking".into(), false, None, 0).await.unwrap() else { panic!() };
    (revision, sessions)
}

#[tokio::test]
async fn composer_thinking_metadata_tracks_commands_model_defaults_sleep_clone_and_reload() {
    let model = ModelServer::start(vec![]).await;
    let (_root, manager, url, server) = fixture(&model, Api::ChatCompletions).await;
    let mut settings = manager.inner.settings.get();
    settings.agent.thinking_level = "high".into();
    settings.agent.model_thinking_levels.insert("openai-codex/gpt-6-astra".into(), "minimal".into());
    settings.agent.model_thinking_levels.insert("openai-codex/other-fixture".into(), "xhigh".into());
    manager.set_settings(settings.revision, settings).await.unwrap();
    let id = manager.create_session(None, "general").await.unwrap();
    let (before, sessions) = page(&manager).await;
    assert_eq!(sessions[0].thinking_level.as_deref(), Some("minimal"));
    let mut client = Client::connect(&url).await;
    assert_eq!(client.request(json!({"id":"thinking-off","type":"prompt","sessionId":id,"text":"/thinking off"})).await["ok"], true);
    // The existing invalidation path carries the changed field over the real native connection.
    let wire = client.until(|m| m["type"] == "sessions" && m["sessions"].as_array().unwrap().iter()
        .any(|s| s["id"] == id && s["thinkingLevel"] == "off")).await;
    assert_eq!(wire["sessions"][0]["thinkingLevel"], "off");
    let ServerMessage::SessionPage { revision, after, .. } = manager.list_page("old-walk".into(), false, Some(id.clone()), before).await.unwrap() else { panic!() };
    assert!(revision > before && after.is_none(), "thinking changes must fence an in-flight catalogue walk");
    let clone = manager.clone_session(&id).await.unwrap();
    let mut settings = manager.inner.settings.get(); settings.agent.thinking_level = "medium".into();
    manager.set_settings(settings.revision, settings).await.unwrap();
    manager.close_session(&id).await.unwrap();
    let (_, sessions) = page(&manager).await;
    let sleeping = sessions.iter().find(|s| s.id == id).unwrap();
    assert_eq!(sleeping.status, SessionStatus::Sleeping);
    assert_eq!(sleeping.thinking_level.as_deref(), Some("off"), "a new global default does not change saved chat metadata");
    assert_eq!(sessions.iter().find(|s| s.id == clone).unwrap().thinking_level.as_deref(), Some("off"));
    manager.prompt(&id, "/model openai-codex/other-fixture", "choose-model").await.unwrap();
    let (revision, sessions) = page(&manager).await;
    let changed = sessions.iter().find(|s| s.id == id).unwrap();
    assert_eq!(changed.model.as_ref().unwrap().model_id, "other-fixture");
    assert_eq!(changed.thinking_level.as_deref(), Some("xhigh"));
    assert!(manager.prompt(&id, "/thinking invalid", "invalid-level").await.is_err());
    assert_eq!(page(&manager).await.0, revision, "a rejected level does not change catalogue metadata");
    let config = manager.inner.config.clone();
    drop(client); manager.shutdown().await; server.abort(); drop(manager);
    let manager = AgentManager::new(config.clone(), StateStore::load(config.database_path.clone()).await.unwrap()).await.unwrap();
    let (_, sessions) = page(&manager).await;
    assert!(sessions.iter().all(|s| s.status == SessionStatus::Sleeping));
    assert_eq!(sessions.iter().find(|s| s.id == id).unwrap().thinking_level.as_deref(), Some("xhigh"));
    assert_eq!(sessions.iter().find(|s| s.id == clone).unwrap().thinking_level.as_deref(), Some("off"));
    assert!(model.requests.is_empty(), "reporting thinking must not make a provider completion");
    manager.shutdown().await;
}
