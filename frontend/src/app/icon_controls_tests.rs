use super::*;
use chad::{Config, HeadlessCtx};
use std::sync::Arc;

fn setup(size: (u32, u32)) -> (App, HeadlessCtx, tempfile::TempDir) {
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
    (app, ctx, root)
}

#[test]
fn settings_gear_stays_in_sidebar_header_and_opens_settings() {
    let icon = Icon::Gear.pixels(24, 0xffffff);
    let alpha = |x: usize, y: usize| icon[(y * 24 + x) * 4 + 3];
    assert_eq!(alpha(12, 12), 0, "the gear's hub is hollow");
    assert!(alpha(12, 4) > 0, "the gear has a visible tooth");

    for size in [(1000, 800), (420, 780)] {
        let (mut app, ctx, _root) = setup(size);
        app.tick(0.);
        if size.0 < 760 {
            app.show_chats = true;
        }
        app.frame(&ctx, ctx.view());
        let settings = app
            .hits
            .iter()
            .find(|hit| matches!(hit.action, Action::Settings))
            .unwrap()
            .rect;
        assert_eq!((settings.width, settings.height), (40., 40.));
        let sidebar_width = if size.0 >= 760 { 300. } else { size.0 as f32 };
        assert_eq!(settings.x + settings.width, sidebar_width - 16.);
        assert_eq!(settings.y, 16.);
        let point = Vec2::new(settings.x + 20., settings.y + 20.);
        app.press(1, point, false);
        app.release(1, point);
        assert!(matches!(
            app.modal.as_ref().map(|modal| &modal.kind),
            Some(ModalKind::Settings)
        ));
    }
}

#[test]
fn latest_is_a_circular_chevron_only_when_scrolled_away_from_tail() {
    let icon = Icon::ChevronDown.pixels(24, 0xffffff);
    let alpha = |x: usize, y: usize| icon[(y * 24 + x) * 4 + 3];
    assert_eq!(
        alpha(12, 6),
        0,
        "no text or circle is baked into the chevron icon"
    );
    assert!(alpha(12, 15) > 0, "the chevron points down");

    for size in [(1000, 800), (420, 780)] {
        let (mut app, ctx, _root) = setup(size);
        let chat = app.controller.chats.get_mut("demo").unwrap();
        chat.feed.events.values_mut().last().unwrap().text =
            "A long reply that needs scrolling.\n\n".repeat(120);
        app.tick(0.);
        app.frame(&ctx, ctx.view());
        assert!(app.max_scroll > 0.);
        assert!(
            !app.hits
                .iter()
                .any(|hit| matches!(hit.action, Action::Tail))
        );

        app.controller
            .chats
            .get_mut("demo")
            .unwrap()
            .local
            .position
            .follow = false;
        app.controller
            .chats
            .get_mut("demo")
            .unwrap()
            .local
            .position
            .key = None;
        app.scroll = 0.;
        app.frame(&ctx, ctx.view());
        let latest = app
            .hits
            .iter()
            .find(|hit| matches!(hit.action, Action::Tail))
            .unwrap()
            .rect;
        assert_eq!((latest.width, latest.height), (40., 40.));
        assert_eq!(
            latest.y + latest.height + 8.,
            app.transcript.y + app.transcript.height
        );
        let point = Vec2::new(latest.x + 20., latest.y + 20.);
        app.press(2, point, false);
        app.release(2, point);
        assert_eq!(app.scroll, app.max_scroll);
        app.frame(&ctx, ctx.view());
        assert!(
            !app.hits
                .iter()
                .any(|hit| matches!(hit.action, Action::Tail))
        );
    }
}
