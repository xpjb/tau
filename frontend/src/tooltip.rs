use sanscale::{Rect, Vec2};
use std::{ops::Range, time::Instant};

pub(crate) mod text;

// Dark accents stay readable on the light tooltip surface.
pub const INK: u32 = 0x252b36;
pub const MUTED: u32 = 0x596170;
pub const ACCENT: u32 = 0x075b82;
pub const GOOD: u32 = 0x17653b;
pub const WARNING: u32 = 0x835000;
pub const ORANGE: u32 = 0x944200;
pub const DANGER: u32 = 0xa62432;

pub fn status_tint(indicator: u32) -> u32 {
    match indicator {
        0x4ade80 => GOOD,
        0xfbbf24 => WARNING,
        0xfb923c => ORANGE,
        0xff5a5f => DANGER,
        0x67d4ff => ACCENT,
        _ => MUTED,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    pub range: Range<usize>,
    pub bold: bool,
    pub tint: u32,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Content {
    pub text: String,
    pub spans: Vec<Span>,
}
impl Content {
    pub fn push(&mut self, value: impl AsRef<str>, bold: bool, tint: u32) -> &mut Self {
        let start = self.text.len();
        self.text.push_str(value.as_ref());
        if self.text.len() > start {
            self.spans.push(Span { range: start..self.text.len(), bold, tint });
        }
        self
    }
    pub fn dim(&mut self, value: impl AsRef<str>) -> &mut Self { self.push(value, false, MUTED) }
    pub fn strong(&mut self, value: impl AsRef<str>, tint: u32) -> &mut Self { self.push(value, true, tint) }
    pub fn line(&mut self) -> &mut Self { self.text.push('\n'); self }
    pub fn append(&mut self, other: Self) {
        let offset = self.text.len();
        self.text.push_str(&other.text);
        self.spans.extend(other.spans.into_iter().map(|mut s| {
            s.range = s.range.start + offset..s.range.end + offset; s
        }));
    }
}

pub struct Tooltip {
    pub region: Rect,
    pub content: Content,
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
            content: Content::default(),
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
    pub fn frame(&mut self, renderer: &mut crate::render::Renderer, layer: &mut crate::render::Layer,
        key: &'static str, bounds: Rect, scale: f32, width: f32, above: bool) {
        use crate::render::color;
        let margin = 8. * scale;
        let padding = 12. * scale;
        let width = (width * scale).min((bounds.width - 2. * margin).max(1.));
        let height = renderer.tooltip_height(key, &self.content, (width - 2. * padding).max(1.), 12. * scale)
            + 2. * padding;
        let anchor = self.region;
        let x = (anchor.x + anchor.width / 2. - width / 2.).clamp(bounds.x + margin,
            (bounds.x + bounds.width - margin - width).max(bounds.x + margin));
        let y = if above { anchor.y - 8. * scale - height } else { anchor.y + anchor.height + 6. * scale };
        let y = y.clamp(bounds.y + margin, (bounds.y + bounds.height - margin - height).max(bounds.y + margin));
        let full = Rect::new(x, y, width, height);
        self.card = full;
        let t = self.progress;
        let pivot = (anchor.x + anchor.width / 2.).clamp(full.x, full.x + full.width);
        let animated = Rect::new(pivot + (x - pivot) * t, if above { y + height * (1. - t) } else { y }, width * t, height * t);
        layer.above();
        layer.rounded_rect(animated, 8. * scale, color(0xe5e1e6));
        let clip = crate::render::intersect(bounds, Rect::new(animated.x + 6. * scale, animated.y + 6. * scale,
            (animated.width - 12. * scale).max(0.), (animated.height - 12. * scale).max(0.)));
        renderer.tooltip(layer, key, Rect::new(full.x + padding, full.y + padding,
            (full.width - 2. * padding).max(1.), (full.height - 2. * padding).max(0.)), 12. * scale, clip);
    }
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
