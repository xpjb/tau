//! Exercise the phone-sized first frame, not just Controller construction.
use super::*;
use chad::{Config, HeadlessCtx};
use std::{sync::{Arc, mpsc}, thread, time::Duration};

#[test]
fn cached_phone_first_frame_does_not_wait_for_a_replica_writer() {
    let root = tempfile::tempdir().unwrap();
    let mut controller = Controller::new(Store::open(root.path().into()).unwrap(), Arc::new(|| {})).unwrap();
    controller.select("chat").unwrap();
    controller.draft("local draft".into()).unwrap();
    let event: Event = serde_json::from_value(serde_json::json!({
        "id":"saved", "entryId":"saved", "order":1, "role":"assistant", "phase":"saved", "kind":"text",
        "text":"verified offline body", "origin":{}, "isError":false
    })).unwrap();
    controller.preview("chat", vec![event], QueueState::default(), None).unwrap();
    controller.chats.get_mut("chat").unwrap().local.pending.push(crate::store::Pending {
        request: ClientRequest { id: "uncertain".into(), command: ClientCommand::Prompt { session_id: "chat".into(), text: "do not replay me".into(), model: None, create: None } },
        started_at_ms: None, text: "do not replay me".into(), files: vec![],
        status: crate::store::Delivery::Unconfirmed, detail: None,
    });
    controller.save_chat("chat").unwrap();
    let replica = root.path().join("blocks").join(format!("{}.sqlite3", crate::store::hash(&controller.identity)));
    let db = rusqlite::Connection::open(&replica).unwrap();
    let lineage = tau_blocks::cursor(&db).unwrap().lineage;
    controller.store.bind_source(&controller.identity, &lineage).unwrap();
    drop(controller);

    let ctx = HeadlessCtx::new(&Config {
        size: (360, 720), device_limits: crate::desktop::limits(), ..Default::default()
    }).unwrap();
    db.execute_batch("BEGIN IMMEDIATE").unwrap();
    let path = root.path().to_owned();
    let (send, receive) = mpsc::channel();
    let worker = thread::spawn(move || {
        let mut app = App::new(&ctx, Store::open(path).unwrap(), Arc::new(|| {}), true).unwrap();
        app.back(); // No credentials in this offline fixture: dismiss setup.
        app.resize(ctx.size(), 1., Vec2::new(0., 0.));
        app.tick(0.);
        app.frame(&ctx, ctx.view());
        assert_eq!(app.controller.selected().unwrap().local.draft, "local draft");
        let pending = &app.controller.selected().unwrap().local.pending;
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].text, "do not replay me");
        assert_eq!(pending[0].status, crate::store::Delivery::Unconfirmed);
        assert_eq!(app.services.renderer.messages["chat/saved"].source, "verified offline body");
        assert!(app.controller.notice.is_none(), "ordinary contention must not become a storage popup");
        let pixels = ctx.read_rgba8().unwrap();
        assert!(pixels.chunks_exact(4).any(|p| p[0] > 100 && p[1] > 100 && p[2] > 100), "cached text must actually paint");
        send.send(()).unwrap();
    });
    // Allow GPU/font setup headroom; the SQLite busy wait is five seconds.
    let rendered = receive.recv_timeout(Duration::from_secs(3));
    db.execute_batch("ROLLBACK").unwrap();
    worker.join().unwrap();
    rendered.expect("phone's first frame waited for the replica writer");
}
