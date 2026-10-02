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
        scroll.event(&Event::Down { pointer: 1, point: start, touch: true }, false, cx);
        scroll.candidate.as_mut().unwrap().started -= Duration::from_millis(16);
        scroll.event(&Event::Move { pointer: 1, point: end }, false, cx);
        scroll.event(&Event::Up { pointer: 1, point: end }, false, cx);
        assert!(scroll.velocity > 1000.);
        scroll.update(0., cx);
        assert!(scroll.velocity > 1000., "A redraw with no elapsed time must not kill a freshly released fling");
        let released = scroll.value;
        scroll.update(1. / 60., cx);
        assert!(scroll.value > released, "Scrolling continues after the finger lifts");
    });
}
