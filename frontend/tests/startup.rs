//! Android uses this same Store -> Controller path before its first frame.
//! Keep a real WAL writer alive until startup finishes: sleeps cannot hide the race.
use rusqlite::Connection;
use std::{path::Path, sync::{Arc, mpsc}, thread, time::{Duration, Instant}};
use tau_frontend::{blocks::Cache, controller::Controller, store::{Settings, Store, hash}};
use tau_protocol::*;

fn while_writing<T: Send + 'static>(path: &Path, open: impl FnOnce() -> anyhow::Result<T> + Send + 'static) -> T {
    let writer = Connection::open(path).unwrap();
    writer.execute_batch("BEGIN IMMEDIATE").unwrap();
    let (send, receive) = mpsc::channel();
    let worker = thread::spawn(move || {
        let started = Instant::now();
        let result = open();
        send.send((result, started.elapsed())).unwrap();
    });
    let result = receive.recv_timeout(Duration::from_secs(1));
    // Release before asserting, including on the old, blocking implementation.
    writer.execute_batch("ROLLBACK").unwrap();
    worker.join().unwrap();
    let (result, elapsed) = result.expect("startup waited for an unrelated SQLite writer");
    eprintln!("startup with WAL writer still held: {elapsed:?}");
    result.unwrap()
}

#[test]
fn current_authored_store_opens_without_waiting_for_a_writer() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(root.path().into()).unwrap();
    store.put("pair", "draft", &"keep authored bytes").unwrap();
    drop(store);
    let path = root.path().to_owned();
    let store = while_writing(&root.path().join("client.sqlite3"), move || Store::open(path));
    assert_eq!(store.get::<String>("pair", "draft").unwrap(), "keep authored bytes");
    store.put("pair", "after", &"still writable").unwrap();
}

#[test]
fn current_replica_opens_without_waiting_for_a_writer() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(root.path().into()).unwrap();
    let cache = store.block_cache("pair").unwrap();
    let path = root.path().join("blocks").join(format!("{}.sqlite3", hash("pair")));
    let reopen = path.clone();
    let second = while_writing(&path, move || Cache::open(&reopen));
    // Both leases remain live; neither opener may unlink/recreate the replica.
    cache.clear().unwrap();
    second.clear().unwrap();
}

#[test]
fn selected_cached_chat_and_draft_restore_without_waiting_for_a_writer() {
    let root = tempfile::tempdir().unwrap();
    let mut c = Controller::new(Store::open(root.path().into()).unwrap(), Arc::new(|| {})).unwrap();
    let identity = Settings::default().identity();
    c.select("chat").unwrap();
    c.draft("unsent local work".into()).unwrap();
    let event: Event = serde_json::from_value(serde_json::json!({
        "id":"saved", "entryId":"saved", "order":1, "role":"assistant", "phase":"saved", "kind":"text",
        "text":"verified offline body", "origin":{}, "isError":false
    })).unwrap();
    c.preview("chat", vec![event], QueueState::default(), None).unwrap();
    let replica = root.path().join("blocks").join(format!("{}.sqlite3", hash(&identity)));
    let source = tau_blocks::cursor(&Connection::open(&replica).unwrap()).unwrap().lineage;
    c.store.bind_source(&identity, &source).unwrap();
    drop(c);
    let path = root.path().to_owned();
    let c = while_writing(&replica, move || Controller::new(Store::open(path)?, Arc::new(|| {})));
    assert_eq!(c.account.selected.as_deref(), Some("chat"));
    let chat = c.selected().unwrap();
    assert_eq!(chat.local.draft, "unsent local work");
    assert_eq!(chat.feed.events[&1].text, "verified offline body");
    assert!(c.epoch.is_none());
}

#[test]
fn overlapping_replica_admin_does_not_fail_startup() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(root.path().into()).unwrap();
    let live = store.block_cache("pair").unwrap();
    let admin = std::fs::OpenOptions::new().read(true).write(true)
        .open(root.path().join("blocks/.replica-admin.lock")).unwrap();
    admin.lock().unwrap();
    let path = root.path().to_owned();
    let (send, receive) = mpsc::channel();
    let worker = thread::spawn(move || send.send(Store::open(path).and_then(|s| s.block_cache("pair"))).unwrap());
    // A second opener must wait for the short GC/lease handoff, not fail fast.
    let early = receive.recv_timeout(Duration::from_millis(100));
    admin.unlock().unwrap();
    let result = early.unwrap_or_else(|error| {
        assert!(matches!(error, mpsc::RecvTimeoutError::Timeout));
        receive.recv_timeout(Duration::from_secs(1)).unwrap()
    });
    worker.join().unwrap();
    result.unwrap().clear().unwrap();
    live.clear().unwrap();
}
