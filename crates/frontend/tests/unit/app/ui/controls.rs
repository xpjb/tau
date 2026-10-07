use super::*;
use std::time::{Duration, Instant};

#[test]
fn mouse_clicks_are_timed_scaled_owner_scoped_and_reset_after_three_or_other_input() {
    let mut clicks = MouseClicks::default();
    let now = Instant::now();
    let point = Vec2::new(10.,10.);
    assert_eq!(clicks.press(1,1,point,1.,now),1);
    assert_eq!(clicks.press(1,2,point,1.,now+Duration::from_millis(200)),2);
    assert_eq!(clicks.press(1,3,point,1.,now+Duration::from_millis(350)),3);
    assert_eq!(clicks.press(1,4,point,1.,now+Duration::from_millis(380)),1);
    assert_eq!(clicks.press(1,5,point,1.,now+Duration::from_secs(1)),1);
    assert_eq!(clicks.press(2,6,point,1.,now+Duration::from_secs(1)),1,"new editor buffer");
    assert_eq!(clicks.press(2,8,point,1.,now+Duration::from_secs(1)),1,"intervening input/another field");
    assert_eq!(clicks.press(2,9,Vec2::new(18.,10.),1.,now+Duration::from_secs(1)),1,"too far away");
    assert_eq!(clicks.press(2,10,point,2.,now+Duration::from_secs(1)),2,"DPI-scaled tolerance");
}
