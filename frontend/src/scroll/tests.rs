use super::*;

fn fling(velocity: f32, scale: f32) -> ScrollMotion {
    let mut motion = ScrollMotion::default();
    let now = Instant::now();
    motion.begin_drag(0., now);
    for ms in [16, 32, 48] {
        motion.sample(velocity * scale * ms as f32 / 1000., scale, now + Duration::from_millis(ms));
    }
    motion.end_drag(now + Duration::from_millis(52), scale);
    motion
}

#[test]
fn android_distance_law_is_cadence_and_display_scale_independent() {
    for scale in [1., 2.5] {
        for hz in [30., 60., 120., 240.] {
            let mut motion = fling(1000., scale);
            let mut value = 500. * scale;
            let start = value;
            let now = Instant::now();
            let mut frames = 0;
            while motion.active() {
                value = motion.advance(value, 10000. * scale, 1. / hz, scale, now);
                frames += 1;
                assert!(frames < hz as usize * 2);
            }
            // Independently calculated Android default-friction distance for
            // a 1000 logical-pixel/second fling (not the old 111ms decay).
            assert!(((value - start) / scale - 194.31).abs() < 0.1, "scale={scale}, hz={hz}, value={value}");
            assert_eq!(motion.velocity, 0.);
        }
    }
}

#[test]
fn a_stalled_frame_catches_up_and_zero_time_does_not_cancel() {
    let mut motion = fling(1500., 1.);
    let now = Instant::now();
    assert_eq!(motion.advance(100., 5000., 0., 1., now), 100.);
    assert!(motion.active());
    assert!(motion.advance(100., 5000., 2., 1., now) > 450.);
    assert!(!motion.active());
}

#[test]
fn release_velocity_survives_lift_jitter_but_not_a_pause() {
    let mut motion = ScrollMotion::default();
    let now = Instant::now();
    motion.begin_drag(0., now);
    for (ms, p) in [(16, 24.), (32, 48.), (48, 72.), (49, 72.1)] {
        motion.sample(p, 1., now + Duration::from_millis(ms));
    }
    motion.end_drag(now + Duration::from_millis(55), 1.);
    assert!(motion.velocity > 1400. && motion.velocity < 1550.);
    let mut paused = fling(1500., 1.);
    paused.begin_drag(0., now);
    paused.sample(24., 1., now + Duration::from_millis(16));
    paused.end_drag(now + Duration::from_millis(200), 1.);
    assert!(!paused.active());
    assert_eq!(paused.velocity, 0.);
}

#[test]
fn reversal_and_fling_limits_follow_the_latest_deliberate_movement() {
    let mut motion = ScrollMotion::default();
    let now = Instant::now();
    motion.begin_drag(0., now);
    for (ms, p) in [(16, 24.), (32, 48.), (48, 40.), (64, 32.)] {
        motion.sample(p, 1., now + Duration::from_millis(ms));
    }
    motion.end_drag(now + Duration::from_millis(68), 1.);
    assert!((motion.velocity + 500.).abs() < 0.1);
    assert!(!fling(20., 1.).active(), "Slow drags do not turn into an unwanted fling");
    assert_eq!(fling(15000., 2.5).velocity, 8000. * 2.5);
}

#[test]
fn boundaries_and_explicit_stop_leave_no_residual_animation() {
    let now = Instant::now();
    for velocity in [-1000., 1000.] {
        let mut motion = fling(velocity, 1.);
        let value = if velocity < 0. { 5. } else { 995. };
        let value = motion.advance(value, 1000., 1. / 60., 1., now);
        assert_eq!(value, if velocity < 0. { 0. } else { 1000. });
        assert!(!motion.active());
    }
    let mut motion = fling(1000., 1.);
    motion.stop();
    assert!(!motion.active());
    assert_eq!(motion.advance(123., 5000., 1., 1., now), 123.);
}

#[test]
fn precision_input_is_not_smoothed_or_given_duplicate_momentum() {
    let now = Instant::now();
    let mut motion = fling(1000., 1.);
    let mut value = 500.;
    for amount in [30., 20., 10., 5., 2., 0.] {
        let old = value;
        value = motion.scroll(value, 1000., amount, true, now);
        assert_eq!(value, old + amount);
        assert!(!motion.active());
    }
    assert_eq!(value, 567.);
    assert_eq!(motion.advance(value, 1000., 1., 1., now), value);
    assert_eq!(motion.scroll(value, 1000., -2000., true, now), 0.);
    assert_eq!(motion.scroll(value, 1000., f32::NAN, true, now), value);
}

#[test]
fn wheel_bursts_accumulate_but_reversal_interrupts_the_old_target() {
    let now = Instant::now();
    let mut motion = ScrollMotion::default();
    let mut value = motion.scroll(500., 1000., 48., false, now);
    value = motion.scroll(value, 1000., 48., false, now);
    assert_eq!(motion.wheel.unwrap().0, 596.);
    value = motion.advance(value, 1000., 0.016, 1., now + Duration::from_millis(16));
    assert!(value > 500. && value < 596.);
    motion.scroll(value, 1000., -48., false, now + Duration::from_millis(16));
    assert_eq!(motion.wheel.unwrap().0, value - 48.);
    for ms in (116..1116).step_by(100) {
        value = motion.advance(value, 1000., 0.1, 1., now + Duration::from_millis(ms));
    }
    assert!(!motion.active());
}
