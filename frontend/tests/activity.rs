//! Chat ordering is a mix of daemon activity and durable, device-local intent.
use std::sync::Arc;
use tau_frontend::{controller::Controller, store::Store};
use tau_protocol::*;

fn controller(root: &std::path::Path) -> Controller {
    Controller::new(Store::open(root.into()).unwrap(), Arc::new(|| {})).unwrap()
}
fn session(id: &str, at: u64) -> SessionSummary {
    SessionSummary {
        id: id.into(), project_id: general_project_id(), title: id.into(), starter: false,
        status: SessionStatus::Sleeping, detail: None, context_usage: None, model: None,
        parent_id: None, created_at_ms: at, updated_at_ms: at,
    }
}
fn catalog(c: &mut Controller, sessions: &[SessionSummary]) {
    c.message(ServerMessage::Sessions { sessions: sessions.to_vec() }).unwrap();
}
fn order(c: &Controller) -> Vec<&str> {
    c.account.sessions.iter().map(|s| s.id.as_str()).collect()
}

#[test]
fn chat_activity_sorts_catalog_and_persists_typing_attachments_and_sends() {
    let root = tempfile::tempdir().unwrap();
    let mut c = controller(root.path());
    // This is the ID order supplied by the keyset-paged daemon catalogue.
    let sessions = vec![session("a", 10), session("b", 30), session("c", 20)];
    catalog(&mut c, &sessions);
    assert_eq!(order(&c), ["b", "c", "a"]);
    c.select("a").unwrap();
    assert_eq!(order(&c), ["b", "c", "a"], "viewing is not activity");
    c.draft("first draft".into()).unwrap();
    assert_eq!(order(&c), ["a", "b", "c"]);
    assert_eq!(c.account.sessions[0].updated_at_ms, 10);
    assert!(!c.unread(&c.account.sessions[0]), "local input must not forge unread/cache timestamps");
    c.select("b").unwrap();
    c.draft("second draft".into()).unwrap();
    c.select("a").unwrap();
    c.draft("first draft".into()).unwrap();
    assert_eq!(order(&c), ["b", "a", "c"], "unchanged editor state is not typing");
    c.draft("first draft edited".into()).unwrap();
    assert_eq!(order(&c), ["a", "b", "c"]);
    catalog(&mut c, &sessions);
    assert_eq!(order(&c), ["a", "b", "c"], "refresh must not discard local bumps");
    // Persist only the chat row on input, not the entire account/catalogue.
    c.select("c").unwrap();
    c.draft("third draft".into()).unwrap();
    drop(c);
    let mut c = controller(root.path());
    assert_eq!(order(&c), ["c", "a", "b"]);
    assert!(!c.chats.contains_key("a"), "sorting must not load all drafts/outboxes");

    let file = root.path().join("attachment.txt");
    std::fs::write(&file, "file bytes").unwrap();
    let identity = c.identity.clone();
    c.attach_to(&identity, "a", &file, None).unwrap();
    assert_eq!(order(&c), ["a", "c", "b"]);
    assert_eq!(c.account.selected.as_deref(), Some("c"), "late picker result targets its original chat");
    c.draft("third draft edited".into()).unwrap();
    assert_eq!(order(&c), ["c", "a", "b"]);
    c.select("a").unwrap();
    c.send_prompt().unwrap();
    assert_eq!(order(&c), ["a", "c", "b"], "send bumps even when its draft was already saved");
    assert_eq!(c.selected().unwrap().local.pending.len(), 1);
    assert!(c.selected().unwrap().local.draft.is_empty());
    drop(c);
    let mut c = controller(root.path());
    catalog(&mut c, &sessions);
    assert_eq!(order(&c), ["a", "c", "b"]);
    assert_eq!(c.selected().unwrap().local.pending[0].files.len(), 1);
}

#[test]
fn chat_activity_new_chat_is_immediate_but_refresh_does_not_pin_it() {
    let root = tempfile::tempdir().unwrap();
    let mut c = controller(root.path());
    // Daemon clock can be far ahead of the device's clock.
    let sessions = vec![session("old", u64::MAX - 1000)];
    catalog(&mut c, &sessions);
    c.new_chat().unwrap();
    let new = c.account.selected.clone().unwrap();
    assert_eq!(order(&c), [new.as_str(), "old"]);
    let created = c.account.sessions[0].created_at_ms;
    c.select("old").unwrap();
    c.draft("new activity in the older chat".into()).unwrap();
    catalog(&mut c, &sessions);
    assert_eq!(order(&c), ["old", new.as_str()]);
    assert_eq!(c.account.sessions[1].created_at_ms, created);
    drop(c);
    let mut c = controller(root.path());
    assert_eq!(order(&c), ["old", new.as_str()]);
    let mut confirmed = sessions.clone();
    confirmed.push(session(&new, u64::MAX - 999));
    catalog(&mut c, &confirmed);
    assert!(c.account.pending_create.is_none());
    assert_eq!(order(&c), [new.as_str(), "old"]);
    c.draft("later activity in old".into()).unwrap();
    catalog(&mut c, &confirmed);
    assert_eq!(order(&c), ["old", new.as_str()], "confirmation is not replayed as a new local bump");
}

#[test]
fn chat_activity_source_completion_wins_without_bumping_intermediate_or_replayed_state() {
    let root = tempfile::tempdir().unwrap();
    let mut c = controller(root.path());
    // Source times are deliberately unlike the local wall clock.
    let mut sessions = vec![session("a", 10), session("b", 20), session("c", 30)];
    catalog(&mut c, &sessions);
    c.select("a").unwrap();
    c.draft("my draft".into()).unwrap();
    c.viewing(false).unwrap();
    for (revision, status) in [(1, SessionStatus::Running), (2, SessionStatus::Running), (3, SessionStatus::Idle)] {
        c.message(ServerMessage::SessionState {
            session_id: "b".into(), revision, restore_review: None, status,
            context_usage: None, detail: None,
        }).unwrap();
        assert_eq!(order(&c), ["a", "c", "b"], "status/usage updates alone are not new activity");
    }
    c.message(ServerMessage::TranscriptSnapshot {
        session_id: "b".into(),
        snapshot: TranscriptSnapshot {
            generation: "history".into(), sequence: 1, events: vec![],
            queue: QueueState::default(), before: None, delivered: vec![],
        },
    }).unwrap();
    catalog(&mut c, &sessions);
    assert_eq!(order(&c), ["a", "c", "b"]);
    sessions[1].updated_at_ms = 31; // durable settled-run bump
    catalog(&mut c, &sessions);
    assert_eq!(order(&c), ["b", "a", "c"]);
    assert!(c.unread(&c.account.sessions[0]));
    c.draft("new local activity after completion".into()).unwrap();
    catalog(&mut c, &sessions);
    assert_eq!(order(&c), ["a", "b", "c"], "same completion cannot re-bump on reconnect");
    sessions[2].updated_at_ms = 32;
    catalog(&mut c, &sessions);
    assert_eq!(order(&c), ["c", "a", "b"]);
    c.account.last_chat_by_project.clear();
    c.account.selected = None;
    c.select_project(GENERAL_PROJECT_ID, true).unwrap();
    assert_eq!(c.account.selected.as_deref(), Some("c"), "topic fallback uses the displayed activity order");
}

#[test]
fn chat_activity_alias_merge_keeps_the_bump_and_picker_target() {
    let root = tempfile::tempdir().unwrap();
    let mut c = controller(root.path());
    let sessions = vec![session("old", 10), session("confirmed", 5)];
    catalog(&mut c, &sessions);
    c.new_chat().unwrap();
    let provisional = c.account.selected.clone().unwrap();
    c.draft("draft in provisional chat".into()).unwrap();
    c.message(ServerMessage::success(provisional.clone(), Some("confirmed".into()), None)).unwrap();
    assert_eq!(order(&c), ["confirmed", "old"]);
    c.select("old").unwrap();
    c.draft("later draft".into()).unwrap();
    let file = root.path().join("picked.txt");
    std::fs::write(&file, "picked before the alias resolved").unwrap();
    c.attach_to(&c.identity.clone(), &provisional, &file, None).unwrap();
    assert_eq!(order(&c), ["confirmed", "old"]);
    assert_eq!(c.chats["confirmed"].local.files.len(), 1);
    drop(c);
    let mut c = controller(root.path());
    catalog(&mut c, &sessions);
    assert_eq!(order(&c), ["confirmed", "old"]);
}

#[test]
fn chat_activity_failed_local_writes_do_not_bump_or_consume_the_draft() {
    let root = tempfile::tempdir().unwrap();
    let mut c = controller(root.path());
    catalog(&mut c, &[session("a", 10), session("b", 20)]);
    c.select("a").unwrap();
    c.draft("keep me".into()).unwrap();
    c.select("b").unwrap();
    c.draft("most recent".into()).unwrap();
    c.select("a").unwrap();
    let activity = c.selected().unwrap().local.activity;
    let db = rusqlite::Connection::open(root.path().join("client.sqlite3")).unwrap();
    db.execute_batch("CREATE TRIGGER reject_chat_write BEFORE UPDATE ON local WHEN NEW.key LIKE 'chat:%' BEGIN SELECT RAISE(ABORT,'fixture disk failure'); END;").unwrap();
    assert!(c.draft("must not stick".into()).is_err());
    assert!(c.send_prompt().is_err());
    let file = root.path().join("picked.txt");
    std::fs::write(&file, "attachment").unwrap();
    assert!(c.attach(&file, None).is_err());
    assert_eq!(order(&c), ["b", "a"]);
    let chat = &c.selected().unwrap().local;
    assert_eq!(chat.draft, "keep me");
    assert_eq!(chat.activity, activity);
    assert!(chat.pending.is_empty() && chat.files.is_empty());
}

#[test]
fn chat_activity_ties_are_deterministic_and_old_local_records_still_load() {
    let root = tempfile::tempdir().unwrap();
    let mut c = controller(root.path());
    c.store.put(&c.identity, "chat:a", &serde_json::json!({"draft":"legacy draft"})).unwrap();
    catalog(&mut c, &[session("z", 10), session("a", 10), session("m", 10)]);
    assert_eq!(order(&c), ["a", "m", "z"]);
    c.select("a").unwrap();
    assert_eq!(c.selected().unwrap().local.draft, "legacy draft");
    assert_eq!(c.selected().unwrap().local.activity.order, 0);
    drop(c);
    let c = controller(root.path());
    assert_eq!(order(&c), ["a", "m", "z"]);
}
