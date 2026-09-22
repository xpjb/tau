use sanscale::Rect;
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
