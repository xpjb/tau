use chad::winit::event::MouseScrollDelta;
use std::time::{Duration, Instant};

/// Winit reports Windows precision touchpads as fractional *line* deltas, not
/// necessarily PixelDelta. Like VS Code's wheel classifier, reserve smoothing
/// for discrete notches and retain precision classification across a burst.
#[derive(Default)]
pub(super) struct WheelDecoder { precision: Option<Instant> }
impl WheelDecoder {
    pub fn decode(&mut self, delta: &MouseScrollDelta, scale: f32, shift: bool, now: Instant) -> (f32, bool, bool) {
        let (x, y, precision) = match *delta {
            MouseScrollDelta::PixelDelta(p) => (-p.x as f32, -p.y as f32, true),
            MouseScrollDelta::LineDelta(x, y) => {
                let fractional = |v: f32| (v - v.round()).abs() > 0.001;
                let precision = fractional(x) || fractional(y) || x != 0. && y != 0.
                    || self.precision.is_some_and(|last| now.saturating_duration_since(last) < Duration::from_millis(200));
                (-x * 48. * scale, -y * 48. * scale, precision)
            }
        };
        if precision { self.precision = Some(now); }
        let horizontal = x.abs() > y.abs();
        (if horizontal { x } else { y }, horizontal || shift, precision)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chad::winit::dpi::PhysicalPosition;
    #[test]
    fn notches_and_shift_keep_the_existing_line_units() {
        let mut decoder = WheelDecoder::default();
        let now = Instant::now();
        assert_eq!(decoder.decode(&MouseScrollDelta::LineDelta(0., 1.), 2.5, false, now), (-120., false, false));
        assert_eq!(decoder.decode(&MouseScrollDelta::LineDelta(0., -2.), 1., true, now), (96., true, false));
        assert_eq!(decoder.decode(&MouseScrollDelta::LineDelta(1., 0.), 1., false, now), (-48., true, false));
    }
    #[test]
    fn windows_precision_line_stream_is_not_mistaken_for_wheel_notches() {
        let mut decoder = WheelDecoder::default();
        let now = Instant::now();
        let (amount, horizontal, precision) = decoder.decode(&MouseScrollDelta::LineDelta(0., 1. / 120.), 1., false, now);
        assert!((amount + 0.4).abs() < 0.001);
        assert!(!horizontal && precision);
        assert!(decoder.decode(&MouseScrollDelta::LineDelta(0., 1.), 1., false, now + Duration::from_millis(16)).2);
        assert!(!decoder.decode(&MouseScrollDelta::LineDelta(0., 1.), 1., false, now + Duration::from_millis(500)).2);
    }
    #[test]
    fn pixel_delta_is_physical_and_preserves_the_platform_momentum_stream() {
        let mut decoder = WheelDecoder::default();
        assert_eq!(decoder.decode(&MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.5, -12.25)), 2.5, false, Instant::now()), (12.25, false, true));
        assert_eq!(decoder.decode(&MouseScrollDelta::PixelDelta(PhysicalPosition::new(-3.5, 0.25)), 1., false, Instant::now()), (3.5, true, true));
    }
}
