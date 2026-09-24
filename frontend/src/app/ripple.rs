use sanscale::{Rect, Vec2};
use std::time::{Duration, Instant};

const EXPAND: Duration = Duration::from_millis(340);
const MIN_HOLD: Duration = Duration::from_millis(180);
const FADE: Duration = Duration::from_millis(220);

/// A press follows its logical section, not its old screen position when a
/// transcript update/scroll moves that section. The sole active ripple is
/// discarded on a new press, drag, session change or after its fade.
pub(super) struct Ripple {
    pub key: String,
    offset: Vec2,
    started: Instant,
    fade_at: Option<Instant>,
}
impl Ripple {
    pub fn new(key: String, rect: Rect, point: Vec2) -> Self {
        Self {
            key,
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
    pub fn paint(&self, key: &str, rect: Rect, now: Instant) -> Option<(Vec2, f32, f32)> {
        if self.key != key || self.finished(now) {
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
mod tests {
    use super::*;
    #[test]
    fn grows_from_press_until_it_covers_the_section_then_fades_only_after_release() {
        let rect = Rect::new(50., 100., 200., 80.);
        let mut ripple = Ripple::new("message".into(), rect, Vec2::new(60., 110.));
        let start = ripple.started;
        assert_eq!(ripple.paint("other", rect, start), None);
        let (center, small, _) = ripple
            .paint("message", rect, start + Duration::from_millis(20))
            .unwrap();
        assert_eq!((center.x, center.y), (60., 110.));
        let moved = Rect::new(50., 130., 200., 80.);
        assert_eq!(
            ripple
                .paint("message", moved, start + Duration::from_millis(20))
                .unwrap()
                .0
                .y,
            140.
        );
        let (_, full, alpha) = ripple.paint("message", rect, start + EXPAND).unwrap();
        assert!(full > small && full >= 190.);
        assert_eq!(alpha, 0.11);
        assert!(
            !ripple.animating(start + EXPAND + Duration::from_millis(1)),
            "no idle redraw while held"
        );
        ripple.release();
        let fade = ripple.fade_at.unwrap();
        assert!(ripple.animating(fade + FADE / 2));
        assert!(ripple.paint("message", rect, fade + FADE / 2).unwrap().2 < alpha);
        assert!(ripple.finished(fade + FADE));
        assert!(ripple.paint("message", rect, fade + FADE).is_none());
    }
}
