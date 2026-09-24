//! Exercise the real GPU card using injected heartbeat state; no live socket.
use super::*;
use chad::{Config, HeadlessCtx};
use std::{sync::Arc, time::Duration};

#[test]
fn connection_counter_repaints_while_visible_and_stops_after_reply() {
    let root = tempfile::tempdir().unwrap();
    let ctx = HeadlessCtx::new(&Config {
        size: (1000, 700),
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
    app.resize(ctx.size(), 1., Vec2::new(0., 0.));
    app.tick(0.);
    app.preview_connection(ConnectionPreview::Waiting);
    app.frame(&ctx, ctx.view());
    let first = ctx.read_rgba8().unwrap();
    assert!(
        app.info_tip.text.contains("Waiting:"),
        "{}",
        app.info_tip.text
    );

    // Simulate an in-flight ping advancing without waiting on a real clock.
    app.controller
        .health
        .sent(Instant::now() - Duration::from_millis(1350));
    assert!(
        app.tick(0.),
        "visible counter must repaint: region={:?} progress={} suppressed={} modal={} epoch={:?} bucket={:?} wait={:?}",
        app.info_tip.region,
        app.info_tip.progress,
        app.info_tip.suppressed,
        app.modal.is_some(),
        app.controller.epoch,
        app.counter_bucket,
        app.controller.health.waiting_ms(Instant::now())
    );
    app.frame(&ctx, ctx.view());
    let second = ctx.read_rgba8().unwrap();
    assert_ne!(
        first, second,
        "the millisecond label must change on the GPU"
    );
    let elapsed: u128 = app
        .info_tip
        .text
        .split("Waiting: ")
        .nth(1)
        .unwrap()
        .split(" ms")
        .next()
        .unwrap()
        .parse()
        .unwrap();
    assert!(elapsed >= 1350, "{}", app.info_tip.text);

    app.controller.health.reply(Duration::from_millis(21));
    assert!(app.tick(0.));
    app.frame(&ctx, ctx.view());
    assert_eq!(
        app.info_tip.text,
        "Connected\nhttps://tau.example.invalid\nRTT: 21 ms"
    );
    assert!(!app.tick(0.), "no counter redraws after the pong");

    app.controller.health.sent(Instant::now());
    app.set_connection_visible(false); // Occluded desktop / suspended Android.
    assert!(!app.tick(0.), "hidden surfaces must not keep ticking");
    app.set_connection_visible(true);
    assert!(
        app.tick(0.),
        "a restored surface resumes the visible counter"
    );
    app.controller.health.reply(Duration::from_millis(21));
    app.tick(0.);
    app.frame(&ctx, ctx.view());
    assert!(!app.tick(0.));

    app.info_tip = Tooltip::default(); // Hide the card before the next in-flight probe.
    app.controller.health.sent(Instant::now());
    assert!(
        !app.tick(0.),
        "a hidden counter must not keep the GPU awake"
    );
    assert_eq!(app.counter_bucket, None);

    app.preview_connection(ConnectionPreview::Disconnected);
    app.frame(&ctx, ctx.view());
    assert_eq!(
        app.info_tip.text,
        "Reconnecting…\nhttps://tau.example.invalid\nPing timed out"
    );
    assert_eq!(app.controller.health.color(), 0xfbbf24);
    assert_ne!(
        first,
        ctx.read_rgba8().unwrap(),
        "disconnected card must render on the GPU"
    );
}
