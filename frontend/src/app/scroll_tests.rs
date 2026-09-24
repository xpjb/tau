use super::*;
use chad::{Config, HeadlessCtx};
use std::sync::Arc;

fn frame(app: &mut App, ctx: &HeadlessCtx) {
    app.tick(0.);
    app.frame(ctx, ctx.view());
}

fn switch(app: &mut App, ctx: &HeadlessCtx, id: &str) {
    if app.size.0 < 760 {
        app.back();
        frame(app, ctx);
    }
    let rect = app
        .hits
        .iter()
        .find(|h| matches!(&h.action, Action::Select(chat) if chat == id))
        .unwrap()
        .rect;
    let point = Vec2::new(rect.x + 30., rect.y + rect.height / 2.);
    app.press(1, point, app.size.0 < 760);
    app.release(1, point);
    frame(app, ctx);
    assert_eq!(app.controller.account.selected.as_deref(), Some(id));
}

#[test]
fn each_chat_restores_its_own_scroll_after_sidebar_switch_and_empty_loading_frame() {
    for size in [(1000, 800), (420, 780)] {
        let root = tempfile::tempdir().unwrap();
        let ctx = HeadlessCtx::new(&Config {
            size,
            device_limits: crate::desktop::limits(),
            ..Default::default()
        })
        .unwrap();
        let mut app = App::new(
            &ctx,
            Store::open(root.path().into()).unwrap(),
            Arc::new(|| {}),
            false,
        )
        .unwrap();
        app.back();
        crate::demo::populate(&mut app.controller).unwrap();
        app.resize(size, 1., Vec2::new(0., 0.));
        let mut events = app
            .controller
            .selected()
            .unwrap()
            .feed
            .events
            .values()
            .cloned()
            .collect::<Vec<_>>();
        events.last_mut().unwrap().text =
            "Long enough to scroll through this chat.\n\n".repeat(120);
        app.controller
            .message(ServerMessage::TranscriptSnapshot {
                session_id: "demo".into(),
                snapshot: TranscriptSnapshot {
                    generation: "demo".into(),
                    sequence: 1,
                    events: events.clone(),
                    queue: QueueState::default(),
                    before: None,
                    delivered: vec![],
                },
            })
            .unwrap();
        let other = events
            .iter()
            .cloned()
            .map(|mut event| {
                event.id = format!("two-{}", event.id);
                event.entry_id = format!("two-{}", event.entry_id);
                event
            })
            .collect::<Vec<_>>();
        app.controller
            .message(ServerMessage::TranscriptSnapshot {
                session_id: "two".into(),
                snapshot: TranscriptSnapshot {
                    generation: "two".into(),
                    sequence: 0,
                    events: other,
                    queue: QueueState::default(),
                    before: None,
                    delivered: vec![],
                },
            })
            .unwrap();
        frame(&mut app, &ctx);
        assert!(app.max_scroll > 200.);
        app.set_scroll(Lane::Transcript, app.max_scroll * 0.4);
        let demo_scroll = app.scroll;
        let demo_key = app.controller.chats["demo"]
            .local
            .position
            .key
            .clone()
            .unwrap();
        assert!(!app.controller.chats["demo"].local.position.follow);
        switch(&mut app, &ctx, "demo");
        assert!(
            (app.scroll - demo_scroll).abs() < 2.,
            "reselecting the active chat keeps its position"
        );

        switch(&mut app, &ctx, "two");
        let stored_demo = app
            .controller
            .store
            .load_chat(&app.controller.identity, "demo")
            .unwrap();
        assert_eq!(
            stored_demo.position.key.as_deref(),
            Some(demo_key.as_str()),
            "switch persists the old chat's anchor"
        );
        assert!(
            app.controller.chats["two"].local.position.follow,
            "pointer release must not store the previous chat's layout on the new chat"
        );
        assert!((app.max_scroll - app.scroll).abs() < 1.);
        app.set_scroll(Lane::Transcript, app.max_scroll * 0.25);
        let two_scroll = app.scroll;
        let two_key = app.controller.chats["two"]
            .local
            .position
            .key
            .clone()
            .unwrap();

        switch(&mut app, &ctx, "demo");
        let stored_two = app
            .controller
            .store
            .load_chat(&app.controller.identity, "two")
            .unwrap();
        assert_eq!(
            stored_two.position.key.as_deref(),
            Some(two_key.as_str()),
            "each chat persists its own position"
        );
        assert!(
            (app.scroll - demo_scroll).abs() < 2.,
            "demo returns to its own anchor"
        );
        switch(&mut app, &ctx, "two");
        assert_eq!(
            app.controller.chats["two"].local.position.key.as_deref(),
            Some(two_key.as_str())
        );
        assert!(
            (app.scroll - two_scroll).abs() < 2.,
            "two keeps its independent position"
        );
        switch(&mut app, &ctx, "demo");

        // Snapshot reloading can briefly draw an empty transcript. It must not
        // replace the saved anchor with a zero-scroll / follow-tail position.
        let anchor = app.controller.chats["demo"].local.position.key.clone();
        app.controller
            .message(ServerMessage::TranscriptSnapshot {
                session_id: "demo".into(),
                snapshot: TranscriptSnapshot {
                    generation: "loading".into(),
                    sequence: 0,
                    events: vec![],
                    queue: QueueState::default(),
                    before: None,
                    delivered: vec![],
                },
            })
            .unwrap();
        frame(&mut app, &ctx);
        assert_eq!(app.controller.chats["demo"].local.position.key, anchor);
        assert!(!app.controller.chats["demo"].local.position.follow);
        app.controller
            .message(ServerMessage::TranscriptSnapshot {
                session_id: "demo".into(),
                snapshot: TranscriptSnapshot {
                    generation: "reloaded".into(),
                    sequence: 0,
                    events,
                    queue: QueueState::default(),
                    before: None,
                    delivered: vec![],
                },
            })
            .unwrap();
        frame(&mut app, &ctx);
        assert!(
            (app.scroll - demo_scroll).abs() < 2.,
            "reloaded history restores its anchor"
        );
    }
}
