//! Saved per-chat metadata must survive offline use without guessing defaults.
use std::sync::Arc;
use tau_frontend::{controller::Controller, store::Store};
use tau_net::*;

#[test]
fn composer_thinking_metadata_updates_per_chat_and_survives_offline_restart() {
    let root = tempfile::tempdir().unwrap();
    let mut c = Controller::new(Store::open(root.path().into()).unwrap(), Arc::new(|| {})).unwrap();
    // Existing account caches predate thinkingLevel; they must still load as unknown.
    let legacy = serde_json::json!({"id":"a","projectId":"general","title":"A","starter":false,
        "status":"sleeping","detail":null,"contextUsage":null,"model":{"provider":"openai-codex","modelId":"fixture"},
        "parentId":null,"createdAtMs":1,"updatedAtMs":1});
    let a: SessionSummary = serde_json::from_value(legacy).unwrap();
    assert!(a.thinking_level.is_none());
    c.message(ServerMessage::Sessions { sessions: vec![a.clone()] }).unwrap();
    c.select("a").unwrap(); c.draft("keep this offline draft".into()).unwrap();
    let mut a = a; a.thinking_level = Some("off".into());
    let mut b = a.clone(); b.id = "b".into(); b.thinking_level = Some("xhigh".into());
    // Exercise the actual camelCase wire round trip, not a settings fallback.
    let update = serde_json::to_vec(&ServerMessage::Sessions { sessions: vec![a, b] }).unwrap();
    c.message(serde_json::from_slice(&update).unwrap()).unwrap();
    c.select("b").unwrap();
    assert_eq!(c.account.sessions.iter().find(|s| s.id == "b").unwrap().thinking_level.as_deref(), Some("xhigh"));
    c.select("a").unwrap();
    drop(c);
    let c = Controller::new(Store::open(root.path().into()).unwrap(), Arc::new(|| {})).unwrap();
    assert!(c.epoch.is_none());
    assert_eq!(c.account.selected.as_deref(), Some("a"));
    assert_eq!(c.selected().unwrap().local.draft, "keep this offline draft");
    assert_eq!(c.account.sessions.iter().find(|s| s.id == "a").unwrap().thinking_level.as_deref(), Some("off"));
    assert_eq!(c.account.sessions.iter().find(|s| s.id == "b").unwrap().thinking_level.as_deref(), Some("xhigh"));
}
