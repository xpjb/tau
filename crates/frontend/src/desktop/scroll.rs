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
#[path = "../../tests/unit/desktop/scroll.rs"]
mod tests;
