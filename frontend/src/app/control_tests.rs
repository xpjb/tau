use super::*;
use chad::{Config, HeadlessCtx};
use std::sync::Arc;

#[test]
fn queue_controls_follow_run_state_and_pending_edits_stay_with_their_message() {
    use ui::composer::Choice as Composer;
    use ui::header::Choice as Header;
    let root = tempfile::tempdir().unwrap();
    let ctx =
        HeadlessCtx::new(&Config { size: (1000, 800), device_limits: crate::desktop::limits(), ..Default::default() })
            .unwrap();
    let mut app = App::new(&ctx, Store::open(root.path().into()).unwrap(), Arc::new(|| {}), false).unwrap();
    app.back();
    crate::demo::populate(&mut app.controller).unwrap();
    app.controller.epoch = Some(1);
    let frame = |app: &mut App| {
        app.tick(0.);
        app.frame(&ctx, ctx.view());
    };
    frame(&mut app);
    let chat = app.controller.chats.get_mut("demo").unwrap();
    chat.feed.queue.requests.push(QueuedRequest {
        request_id: "queued".into(),
        revision: 0,
        kind: "steer".into(),
        text: "old text".into(),
        images: 0,
        timestamp_ms: None,
    });
    chat.local.pending.push(crate::store::Pending {
        request: ClientRequest {
            id: "edit".into(),
            command: ClientCommand::QueueControl {
                session_id: "demo".into(),
                generation: "demo".into(),
                operation: QueueOperation::Edit { request_id: "queued".into(), revision: 0, text: "new text".into() },
            },
        },
        started_at_ms: None,
        text: "new text".into(),
        files: vec![],
        status: crate::store::Delivery::Sending,
        detail: None,
    });
    chat.reconcile();
    let rows = projection::rows(&app.controller, "demo");
    assert!(
        rows.iter()
            .any(|r| r.key == "message:demo:queued" && r.source == literal("new text") && r.title.contains("saving"))
    );
    assert!(!rows.iter().any(|r| r.key == "pending:edit"));
    let chat = app.controller.chats.get_mut("demo").unwrap();
    chat.local.pending.clear();
    chat.feed.queue.requests.clear();
    chat.reconcile();
    assert!(app.root.workspace.chat.header.controls.placed().any(|(a, _)| matches!(a, Header::Abort)));
    app.controller.account.sessions.iter_mut().find(|s| s.id == "demo").unwrap().status = SessionStatus::Idle;
    let queue = &mut app.controller.chats.get_mut("demo").unwrap().feed.queue;
    queue.paused = true;
    queue.run_id = Some("held-run".into());
    frame(&mut app);
    assert!(app.root.workspace.chat.header.controls.placed().any(
        |(a, _)| matches!(a,Header::Queue(QueueOperation::Resume {run_id}) if run_id.as_deref()==Some("held-run"))
    ));
    assert!(!app.root.workspace.chat.header.controls.placed().any(|(a, _)| matches!(a, Header::Abort)));
    app.controller.chats.get_mut("demo").unwrap().feed.queue.paused = false;
    frame(&mut app);
    assert!(
        !app.root
            .workspace
            .chat
            .header
            .controls
            .placed()
            .any(|(a, _)| matches!(a, Header::Abort | Header::Queue(QueueOperation::Resume { .. })))
    );
    app.controller.chats.get_mut("demo").unwrap().feed.queue.paused = true;
    app.controller.epoch = None;
    frame(&mut app);
    let resume = &app
        .root
        .workspace
        .chat
        .header
        .controls
        .items
        .iter()
        .find(|(_, _, a)| matches!(a, Header::Queue(QueueOperation::Resume { .. })))
        .unwrap()
        .1
        .control;
    assert!(resume.rect.is_some() && !resume.enabled, "Offline Resume remains visible but disabled");
    app.controller.epoch = Some(1);
    app.controller.chats.get_mut("demo").unwrap().feed.queue.control = Some(QueueControl {
        command_id: "pause-control".into(),
        run_id: Some("held-run".into()),
        action: "pause".into(),
        boundary: None,
        requests: vec![],
        status: "waiting".into(),
        detail: None,
    });
    frame(&mut app);
    assert!(
        app.root.workspace.chat.composer.controls.placed().any(
            |(a, _)| matches!(a,Composer::Queue(QueueOperation::Cancel {control_id}) if control_id=="pause-control")
        )
    );
    assert!(
        app.root
            .workspace
            .chat
            .header
            .controls
            .placed()
            .any(|(a, _)| matches!(a, Header::Queue(QueueOperation::Resume { .. })))
    );
}

#[test]
fn middle_click_marker_is_drawn_at_the_autoscroll_anchor_not_text_baseline() {
    let root = tempfile::tempdir().unwrap();
    let ctx = HeadlessCtx::new(&Config {
        size: (1000, 800),
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
    app.controller
        .chats
        .get_mut("demo")
        .unwrap()
        .feed
        .events
        .values_mut()
        .last()
        .unwrap()
        .text = "Enough text to scroll.\n\n".repeat(120);
    app.resize(ctx.size(), 1., Vec2::new(0., 0.));
    app.tick(0.);
    app.frame(&ctx, ctx.view());
    assert!(app.root.workspace.chat.transcript.scroll.max > 0.);
    let point = Vec2::new(
        app.root.workspace.chat.transcript.scroll.rect.x + app.root.workspace.chat.transcript.scroll.rect.width / 2.,
        app.root.workspace.chat.transcript.scroll.rect.y + app.root.workspace.chat.transcript.scroll.rect.height / 2.,
    );
    let before = ctx.read_rgba8().unwrap();
    app.middle(true, point);
    let anchor = app.root.workspace.chat.transcript.autoscroll.as_ref().unwrap().anchor;
    assert_eq!((anchor.x, anchor.y), (point.x, point.y));
    app.frame(&ctx, ctx.view());
    let during = ctx.read_rgba8().unwrap();
    let pixel = |image: &[u8], x: f32, y: f32| {
        let at = (y as usize * 1000 + x as usize) * 4;
        <[u8; 4]>::try_from(&image[at..at + 4]).unwrap()
    };
    assert_ne!(
        pixel(&before, point.x, point.y),
        pixel(&during, point.x, point.y)
    );
    assert_ne!(
        pixel(&before, point.x, point.y - 7.),
        pixel(&during, point.x, point.y - 7.)
    );
    assert_ne!(
        pixel(&before, point.x, point.y + 7.),
        pixel(&during, point.x, point.y + 7.)
    );
    app.cancel_autoscroll();
    app.frame(&ctx, ctx.view());
    assert_eq!(
        before,
        ctx.read_rgba8().unwrap(),
        "dismissing the badge restores the transcript"
    );
}

