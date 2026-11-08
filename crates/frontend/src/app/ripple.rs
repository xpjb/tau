use sanscale::{Rect, Vec2};
use std::time::{Duration, Instant};

const EXPAND: Duration = Duration::from_millis(340);
const MIN_HOLD: Duration = Duration::from_millis(180);
const FADE: Duration = Duration::from_millis(220);

/// The owning control retains the click origin through reflow. Abandoned
/// gestures disappear; a normal release finishes its short fade.
pub(super) struct Ripple {
    offset: Vec2,
    started: Instant,
    fade_at: Option<Instant>,
}
impl Ripple {
    pub fn new(rect: Rect, point: Vec2) -> Self {
        Self {
            offset: Vec2::new(point.x - rect.x, point.y - rect.y),
            started: Instant::now(),
            fade_at: None,
        }
    }
    pub fn release(&mut self) {
        self.fade_at
            .get_or_insert_with(|| Instant::now().max(self.started + MIN_HOLD));
    }
    pub fn animating(&self, now: Instant) -> bool {
        now < self.started + EXPAND || self.fade_at.is_some_and(|fade| now < fade + FADE)
    }
    pub fn finished(&self, now: Instant) -> bool {
        self.fade_at.is_some_and(|fade| now >= fade + FADE)
    }
    pub fn paint(&self, rect: Rect, now: Instant, held: bool) -> Option<(Vec2, f32, f32)> {
        if self.finished(now) || self.fade_at.is_none() && !held {
            return None;
        }
        let center = Vec2::new(rect.x + self.offset.x, rect.y + self.offset.y);
        let radius = [
            (self.offset.x, self.offset.y),
            (rect.width - self.offset.x, self.offset.y),
            (self.offset.x, rect.height - self.offset.y),
            (rect.width - self.offset.x, rect.height - self.offset.y),
        ]
        .into_iter()
        .map(|(x, y)| x.hypot(y))
        .fold(0., f32::max);
        let progress = (now.saturating_duration_since(self.started).as_secs_f32()
            / EXPAND.as_secs_f32())
        .clamp(0., 1.);
        let expansion = 1. - (1. - progress).powi(3);
        let opacity = self.fade_at.map_or(1., |fade| {
            1. - (now.saturating_duration_since(fade).as_secs_f32() / FADE.as_secs_f32())
                .clamp(0., 1.)
        });
        Some((center, radius * expansion, 0.11 * opacity))
    }
}

#[cfg(test)]
#[path = "../../tests/unit/app/ripple.rs"]
mod tests;
