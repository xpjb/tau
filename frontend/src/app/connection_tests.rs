//! Real GPU status card with injected heartbeat events, no live socket.
use super::*;
use chad::{Config, HeadlessCtx};
use std::{sync::Arc, time::Duration};

fn elapsed(text: &str, label: &str) -> u128 {
    text.lines()
        .find_map(|line| line.strip_prefix(label))
        .unwrap_or_else(|| panic!("missing {label} in {text}"))
        .strip_suffix("ms")
        .unwrap()
        .parse()
        .unwrap()
}

#[test]
fn connection_card_shows_live_ack_and_waiting_counters_but_leaves_unread_dot_alone() {
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

    app.preview_connection(ConnectionPreview::Received);
    app.frame(&ctx, ctx.view());
    let received = ctx.read_rgba8().unwrap();
    assert!(
        app.info_tip
            .text
            .starts_with("Connected\nmin: 123ms\nmax: 420ms\nreceived: "),
        "{}",
        app.info_tip.text
    );
    assert!(elapsed(&app.info_tip.text, "received: ") >= 1234);
    assert!(!app.info_tip.text.contains("tau.example.invalid"));
    assert_eq!(app.controller.health.color(Instant::now()), 0x4ade80);

    app.controller
        .health
        .sent(Instant::now() - Duration::from_millis(1350));
    assert!(app.tick(0.), "waiting must repaint");
    app.frame(&ctx, ctx.view());
    let waiting = ctx.read_rgba8().unwrap();
    assert_ne!(received, waiting, "waiting must change the GPU frame");
    assert!(elapsed(&app.info_tip.text, "waiting: ") >= 1350);
    assert_eq!(app.controller.health.color(Instant::now()), 0xfb923c);
    assert_eq!(app.info_tip.text.lines().count(), 4);

    // Pong: the live timer switches to age since receipt, not the previous send.
    app.controller
        .health
        .reply(Duration::from_millis(1350), Instant::now());
    assert!(app.tick(0.));
    app.frame(&ctx, ctx.view());
    assert!(
        app.info_tip
            .text
            .starts_with("Connected\nmin: 123ms\nmax: 1350ms\nreceived: ")
    );
    let before = elapsed(&app.info_tip.text, "received: ");
    std::thread::sleep(Duration::from_millis(60));
    assert!(app.tick(0.), "received timer must continue while visible");
    app.frame(&ctx, ctx.view());
    assert!(elapsed(&app.info_tip.text, "received: ") > before);
    assert_eq!(app.controller.health.color(Instant::now()), 0xfb923c);
    app.controller
        .health
        .sent(Instant::now() - Duration::from_millis(21));
    app.controller
        .health
        .reply(Duration::from_millis(21), Instant::now());
    app.tick(0.);
    app.frame(&ctx, ctx.view());
    assert!(
        app.info_tip
            .text
            .starts_with("Connected\nmin: 21ms\nmax: 1350ms\nreceived: ")
    );
    assert_eq!(app.controller.health.color(Instant::now()), 0x4ade80);

    app.set_connection_visible(false); // Suspended Android / occluded desktop.
    assert!(!app.tick(0.), "hidden surface must not keep rendering");
    app.set_connection_visible(true);
    assert!(app.tick(0.));
    app.frame(&ctx, ctx.view());
    app.info_tip = Tooltip::default(); // Closed card: no 50ms redraw loop.
    assert!(app.tick(0.)); // One final frame to close the card.
    assert_eq!(app.counter_bucket, None);
    assert!(!app.tick(0.));

    // Socket loss does not rewrite the chat's last known Working label or unread marker.
    app.preview_connection(ConnectionPreview::Disconnected);
    app.frame(&ctx, ctx.view());
    assert!(
        app.info_tip
            .text
            .starts_with("Reconnecting…\nmin: 123ms\nmax: 420ms\nreceived: ")
    );
    assert!(app.info_tip.text.ends_with("\nPing timed out"));
    assert_eq!(app.controller.health.color(Instant::now()), 0xff5a5f);
    let disconnected = ctx.read_rgba8().unwrap();
    let pixel = |image: &[u8], x, y| {
        let offset = ((y * ctx.size().0 + x) * 4) as usize;
        image[offset..offset + 3].to_vec()
    };
    // Tau's dot stays solid in both states; only its color changes.
    assert_ne!(pixel(&received, 86, 39), pixel(&received, 95, 39));
    assert_ne!(pixel(&disconnected, 86, 39), pixel(&disconnected, 95, 39));
    assert_ne!(pixel(&received, 86, 39), pixel(&disconnected, 86, 39));

    app.preview_connection(ConnectionPreview::Unconfigured);
    app.frame(&ctx, ctx.view());
    assert_eq!(app.info_tip.text, "Offline\nmin: —\nmax: —\nreceived: —");
    assert_eq!(app.controller.health.color(Instant::now()), 0xff5a5f);
}

#[test]
fn hidden_card_wakes_only_when_the_dot_crosses_a_color_boundary() {
    let root = tempfile::tempdir().unwrap();
    let ctx = HeadlessCtx::new(&Config {
        size: (1000, 700),
        device_limits: crate::desktop::limits(),
        ..Default::default()
    })
    .unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = App::new(
        &ctx,
        Store::open(root.path().into()).unwrap(),
        Arc::new(move || {
            let _ = tx.send(());
        }),
        false,
    )
    .unwrap();
    app.back();
    crate::demo::populate(&mut app.controller).unwrap();
    app.resize(ctx.size(), 1., Vec2::new(0., 0.));
    app.tick(0.);
    app.preview_connection(ConnectionPreview::Received);
    app.frame(&ctx, ctx.view());
    app.info_tip = Tooltip::default();
    app.controller
        .health
        .sent(Instant::now() - Duration::from_millis(920));
    app.tick(0.);
    assert_eq!(app.dot_color, 0xfbbf24);
    assert_eq!(app.counter_bucket, None);
    let deadline = Instant::now() + Duration::from_secs(2);
    while app.dot_color != 0xfb923c {
        assert!(
            Instant::now() < deadline,
            "hidden color threshold did not wake the UI"
        );
        rx.recv_timeout(Duration::from_millis(300)).unwrap();
        app.tick(0.);
    }
    app.frame(&ctx, ctx.view());
    assert_eq!(
        app.counter_bucket, None,
        "hidden card must not start a 50ms loop"
    );
}
