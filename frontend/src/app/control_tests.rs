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
