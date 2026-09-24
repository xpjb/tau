use super::*;
use chad::{Config, HeadlessCtx};
use std::sync::Arc;

#[test]
fn resume_replaces_stop_in_the_header_without_an_editor_row() {
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
        app.controller.epoch = Some(1);
        app.resize(ctx.size(), 1., Vec2::new(0., 0.));
        let frame = |app: &mut App| {
            app.tick(0.);
            app.frame(&ctx, ctx.view());
        };

        frame(&mut app);
        let stop = app
            .hits
            .iter()
            .find(|h| matches!(h.action, Action::Abort))
            .unwrap()
            .rect;
        let viewport_height = app.transcript.height;
        assert!(
            stop.y + stop.height < app.transcript.y,
            "stop is in the chat header"
        );

        app.controller
            .account
            .sessions
            .iter_mut()
            .find(|s| s.id == "demo")
            .unwrap()
            .status = SessionStatus::Idle;
        let queue = &mut app.controller.chats.get_mut("demo").unwrap().feed.queue;
        queue.paused = true;
        queue.run_id = Some("held-run".into());
        frame(&mut app);
        let resume = app.hits.iter().find(|h| matches!(&h.action,
            Action::Queue(QueueOperation::Resume { run_id }) if run_id.as_deref() == Some("held-run"))).unwrap().rect;
        assert_eq!(
            (resume.x, resume.y, resume.width, resume.height),
            (stop.x, stop.y, stop.width, stop.height),
            "play uses the exact stop hit target"
        );
        assert!(!app.hits.iter().any(|h| matches!(h.action, Action::Abort)));
        assert_eq!(
            app.transcript.height, viewport_height,
            "pausing must not add a row under the editor"
        );
        assert_ne!(
            Icon::Play.pixels(24, 0xffffff),
            Icon::Stop.pixels(24, 0xffffff)
        );

        app.controller
            .chats
            .get_mut("demo")
            .unwrap()
            .feed
            .queue
            .paused = false;
        frame(&mut app);
        assert!(!app.hits.iter().any(|h| matches!(
            h.action,
            Action::Abort | Action::Queue(QueueOperation::Resume { .. })
        )));
        assert_eq!(app.transcript.height, viewport_height);

        app.controller
            .chats
            .get_mut("demo")
            .unwrap()
            .feed
            .queue
            .paused = true;
        app.controller.epoch = None;
        frame(&mut app);
        assert!(
            !app.hits
                .iter()
                .any(|h| matches!(h.action, Action::Queue(QueueOperation::Resume { .. }))),
            "offline play is visible but not clickable"
        );

        app.controller.epoch = Some(1);
        app.controller
            .chats
            .get_mut("demo")
            .unwrap()
            .feed
            .queue
            .control = Some(QueueControl {
            command_id: "pause-control".into(),
            run_id: Some("held-run".into()),
            action: "pause".into(),
            boundary: None,
            requests: vec![],
            status: "waiting".into(),
            detail: None,
        });
        frame(&mut app);
        let cancel = app
            .hits
            .iter()
            .find(|h| {
                matches!(&h.action,
            Action::Queue(QueueOperation::Cancel { control_id }) if control_id == "pause-control")
            })
            .unwrap()
            .rect;
        let editor = app
            .hits
            .iter()
            .find(|h| matches!(h.action, Action::Focus(None)))
            .unwrap()
            .rect;
        assert_eq!(cancel.x, editor.x - 40.);
        assert_eq!(
            app.transcript.height,
            viewport_height - 40.,
            "only pending control occupies a composer row"
        );
        assert!(
            app.hits
                .iter()
                .any(|h| matches!(h.action, Action::Queue(QueueOperation::Resume { .. })))
        );
    }
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
    assert!(app.max_scroll > 0.);
    let point = Vec2::new(
        app.transcript.x + app.transcript.width / 2.,
        app.transcript.y + app.transcript.height / 2.,
    );
    let before = ctx.read_rgba8().unwrap();
    app.middle(true, point);
    let anchor = app.autoscroll.as_ref().unwrap().anchor;
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
