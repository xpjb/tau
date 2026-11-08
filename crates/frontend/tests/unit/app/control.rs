use super::*;
use chad::{Config, HeadlessCtx};
use std::sync::Arc;

#[test]
fn queue_controls_follow_run_state() {
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
    // The cancellation escape is named for the action and remains reachable on
    // a phone-sized layout. Play remains available without using it first.
    app.controller.chats.get_mut("demo").unwrap().feed.queue.control.as_mut().unwrap().action = "prefix".into();
    app.ui.mobile = true;
    app.resize((390, 740), 1., Vec2::new(0., 0.));
    app.root.workspace.show_chats = false;
    frame(&mut app);
    let cancel = app.root.workspace.chat.composer.controls.items.iter().find(|(_, _, a)|
        matches!(a, Composer::Queue(QueueOperation::Cancel {control_id}) if control_id == "pause-control")).unwrap();
    assert_eq!(cancel.1.label, "Cancel run limit");
    assert!(cancel.1.control.enabled && cancel.1.control.rect.is_some_and(|r|
        r.y >= cancel.1.control.clip.y && r.y + r.height <= cancel.1.control.clip.y + cancel.1.control.clip.height));
    assert!(app.root.workspace.chat.header.controls.placed().any(|(a, _)| matches!(a, Header::Queue(QueueOperation::Resume { .. }))));
    app.controller.chats.get_mut("demo").unwrap().feed.queue.control = None;
    frame(&mut app);
    assert!(!app.root.workspace.chat.composer.controls.placed().any(|(a, _)| matches!(a, Composer::Queue(QueueOperation::Cancel { .. }))));
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
    let mut events = app.controller.chats["demo"].feed.events.values().cloned().collect::<Vec<_>>();
    events.last_mut().unwrap().text = "Enough text to scroll.\n\n".repeat(120);
    app.controller.preview("demo", events, Default::default(), None).unwrap();
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
    assert!(app.root.workspace.chat.transcript.autoscroll.is_none());
}
