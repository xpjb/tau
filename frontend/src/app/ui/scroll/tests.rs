use super::*;
use crate::app::{App, Store};
use chad::{Config, HeadlessCtx};
use std::{sync::Arc, time::Duration};

#[test]
fn releasing_touch_keeps_momentum_across_a_zero_time_frame() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = HeadlessCtx::new(&Config {
        size: (360, 720), device_limits: crate::desktop::limits(), ..Default::default()
    }).unwrap();
    let mut app = App::new(&ctx, Store::open(dir.path().into()).unwrap(), Arc::new(|| {}), true).unwrap();
    app.with_ui(|_, cx| {
        let mut scroll = ScrollState::new(Id::new(), false);
        scroll.rect = Rect::new(0., 0., 360., 720.);
        scroll.max = 5000.;
        let start = Vec2::new(100., 300.);
        let end = Vec2::new(100., 276.);
        let now = Instant::now();
        scroll.event_at(&Event::Down { pointer: 1, point: start, touch: true }, false, cx, now);
        scroll.event_at(&Event::Move { pointer: 1, point: end }, false, cx, now + Duration::from_millis(16));
        scroll.event_at(&Event::Up { pointer: 1, point: end }, false, cx, now + Duration::from_millis(20));
        assert!(scroll.motion.velocity > 1000.);
        scroll.update(0., cx);
        assert!(scroll.motion.velocity > 1000., "A redraw with no elapsed time must not kill a freshly released fling");
        let released = scroll.value;
        scroll.update(1. / 60., cx);
        assert!(scroll.value > released, "Scrolling continues after the finger lifts");
    });
}

#[test]
fn dragging_back_inside_touch_slop_does_not_freeze_the_claimed_scroll() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = HeadlessCtx::new(&Config {
        size: (360, 720), device_limits: crate::desktop::limits(), ..Default::default()
    }).unwrap();
    let mut app = App::new(&ctx, Store::open(dir.path().into()).unwrap(), Arc::new(|| {}), true).unwrap();
    app.with_ui(|_, cx| {
        let mut scroll = ScrollState::new(Id::new(), false);
        scroll.rect = Rect::new(0., 0., 360., 720.);
        scroll.max = 5000.; scroll.value = 500.;
        let now = Instant::now();
        scroll.event_at(&Event::Down { pointer: 1, point: Vec2::new(100., 300.), touch: true }, false, cx, now);
        scroll.event_at(&Event::Move { pointer: 1, point: Vec2::new(100., 280.) }, false, cx, now + Duration::from_millis(16));
        assert_eq!(scroll.value, 520.);
        scroll.event_at(&Event::Move { pointer: 1, point: Vec2::new(100., 298.) }, false, cx, now + Duration::from_millis(32));
        assert_eq!(scroll.value, 502., "Touch slop is only an acquisition threshold, not a dead zone after acquisition");
    });
}
