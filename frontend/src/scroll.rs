//! Middle-button autoscroll math shared with the retained transcript.
use sanscale::Vec2;
use std::time::Instant;

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
