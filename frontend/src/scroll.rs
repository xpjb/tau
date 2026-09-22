//! Desktop scrolling: a one-pole wheel filter, middle-button autoscroll and scrollbar geometry.
use sanscale::{Rect, Vec2};
use std::time::Instant;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Lane {
    Transcript,
    Sidebar,
    Horizontal,
}
pub struct Wheel {
    pub lane: Lane,
    pub target: f32,
    pub last: Instant,
}
impl Wheel {
    pub fn step(&mut self, value: f32, max: f32, scale: f32) -> (f32, bool) {
        let now = Instant::now();
        let dt = now.duration_since(self.last).as_secs_f32().min(0.1);
        self.last = now;
        self.target = self.target.clamp(0., max);
        // Exact first-order low-pass response: frame-rate independent, no overshoot.
        let next = value + (self.target - value) * (1. - (-dt / 0.065).exp());
        let settled = (self.target - next).abs() < 0.25 * scale;
        (if settled { self.target } else { next }, settled)
    }
}
pub struct Autoscroll {
    pub anchor: Vec2,
    pub pointer: Vec2,
    pub pressed: Option<Instant>,
}
impl Autoscroll {
    pub fn speed(&self, scale: f32) -> f32 {
        let distance = self.pointer.y - self.anchor.y;
        distance.signum() * ((distance.abs() - 6. * scale).max(0.) * 30.).min(7200. * scale)
    }
}
#[derive(Clone, Copy)]
pub struct Scrollbar {
    pub lane: Lane,
    pub track: Rect,
    pub thumb: Rect,
    pub max: f32,
}
impl Scrollbar {
    pub fn new(lane: Lane, viewport: Rect, value: f32, max: f32, scale: f32) -> Option<Self> {
        if max <= 0. || viewport.height <= 0. {
            return None;
        }
        let track = Rect::new(
            viewport.x + viewport.width - 12. * scale,
            viewport.y,
            12. * scale,
            viewport.height,
        );
        let height = (track.height * viewport.height / (viewport.height + max))
            .max(28. * scale)
            .min(track.height);
        let thumb = Rect::new(
            track.x,
            track.y + (track.height - height) * (value / max).clamp(0., 1.),
            track.width,
            height,
        );
        Some(Self {
            lane,
            track,
            thumb,
            max,
        })
    }
    pub fn value_at(&self, pointer_y: f32, grab: f32) -> f32 {
        let travel = self.track.height - self.thumb.height;
        if travel <= 0. {
            return 0.;
        }
        ((pointer_y - self.track.y - grab * self.thumb.height) / travel).clamp(0., 1.) * self.max
    }
}
pub struct Drag {
    pub lane: Lane,
    pub grab: f32,
}
