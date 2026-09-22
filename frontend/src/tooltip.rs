use sanscale::{Rect, Vec2};
use std::time::Instant;

pub struct Tooltip {
    pub region: Rect,
    pub text: String,
    pub card: Rect,
    pub progress: f32,
    pub pinned: bool,
    pub suppressed: bool,
    hover: Option<Instant>,
    from: f32,
    target: f32,
    at: Instant,
}
impl Default for Tooltip {
    fn default() -> Self {
        Self {
            region: Rect::new(0., 0., 0., 0.),
            text: String::new(),
            card: Rect::new(0., 0., 0., 0.),
            progress: 0.,
            pinned: false,
            suppressed: false,
            hover: None,
            from: 0.,
            target: 0.,
            at: Instant::now(),
        }
    }
}
impl Tooltip {
    pub fn contains_card(&self, point: Vec2) -> bool {
        self.region.width > 0. && self.progress > 0. && crate::render::contains(self.card, point)
    }
    pub fn contains(&self, point: Vec2) -> bool {
        use crate::render::contains;
        if contains(self.region, point) || self.contains_card(point) {
            return true;
        }
        if self.region.width <= 0. || self.progress <= 0. {
            return false;
        }
        // A narrow hover bridge prevents a tooltip closing while crossing the
        // small gap between its indicator and card.
        let x = self.region.x.max(self.card.x);
        let width = (self.region.x + self.region.width).min(self.card.x + self.card.width) - x;
        let (y, height) = if self.card.y >= self.region.y + self.region.height {
            (
                self.region.y + self.region.height,
                self.card.y - self.region.y - self.region.height,
            )
        } else {
            (
                self.card.y + self.card.height,
                self.region.y - self.card.y - self.card.height,
            )
        };
        contains(Rect::new(x, y, width.max(0.), height.max(0.)), point)
    }
    pub fn hover(&mut self, inside: bool) {
        if inside {
            if self.hover.is_none() {
                self.hover = Some(Instant::now());
                self.suppressed = false;
            }
        } else {
            self.hover = None;
            self.suppressed = false;
        }
    }
    pub fn dismiss(&mut self) {
        self.pinned = false;
        self.suppressed = true;
    }
    pub fn tick(&mut self) -> bool {
        let target = if !self.suppressed
            && (self.pinned || self.hover.is_some_and(|at| at.elapsed().as_millis() >= 250))
        {
            1.
        } else {
            0.
        };
        if target != self.target {
            self.from = self.progress;
            self.target = target;
            self.at = Instant::now();
        }
        let t = (self.at.elapsed().as_secs_f32() / 0.16).min(1.);
        self.progress = self.from + (self.target - self.from) * (1. - (1. - t).powi(3));
        (self.progress - self.target).abs() > 0.001
            || !self.suppressed && self.hover.is_some_and(|at| at.elapsed().as_millis() < 250)
    }
}
