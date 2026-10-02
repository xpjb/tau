use super::*;
#[test]
fn grows_from_press_until_it_covers_the_section_then_fades_only_after_release() {
    let rect = Rect::new(50., 100., 200., 80.);
    let mut ripple = Ripple::new(rect, Vec2::new(60., 110.));
    let start = ripple.started;
    let (center, small, _) = ripple
        .paint(rect, start + Duration::from_millis(20), true)
        .unwrap();
    assert_eq!((center.x, center.y), (60., 110.));
    let moved = Rect::new(50., 130., 200., 80.);
    assert_eq!(
        ripple
            .paint(moved, start + Duration::from_millis(20), true)
            .unwrap()
            .0
            .y,
        140.
    );
    let (_, full, alpha) = ripple.paint(rect, start + EXPAND, true).unwrap();
    assert!(full > small && full >= 190.);
    assert_eq!(alpha, 0.11);
    assert!(
        !ripple.animating(start + EXPAND + Duration::from_millis(1)),
        "no idle redraw while held"
    );
    ripple.release();
    let fade = ripple.fade_at.unwrap();
    assert!(ripple.animating(fade + FADE / 2));
    assert!(ripple.paint(rect, fade + FADE / 2, false).unwrap().2 < alpha);
    assert!(ripple.finished(fade + FADE));
    assert!(ripple.paint(rect, fade + FADE, false).is_none());
}
