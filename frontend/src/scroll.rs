//! Shared scrolling physics, independent of widgets, text layout and redraws.
use sanscale::Vec2;
use std::{collections::VecDeque, time::{Duration, Instant}};

/// One axis. Positions and velocities are physical pixels; fling physics runs
/// in logical pixels so the same gesture feels the same at every display scale.
#[derive(Default)]
pub(crate) struct ScrollMotion {
    pub velocity: f32,
    pub wheel: Option<(f32, Instant)>,
    fling: Option<Fling>,
    samples: VecDeque<(Instant, f32)>,
    direction: f32,
}
impl ScrollMotion {
    pub fn active(&self) -> bool { self.wheel.is_some() || self.fling.is_some() }
    pub fn stop(&mut self) { *self = Self::default(); }
    pub fn begin_drag(&mut self, position: f32, now: Instant) {
        self.stop();
        self.samples.push_back((now, position));
    }
    pub fn sample(&mut self, position: f32, scale: f32, now: Instant) {
        if let Some(&(last, previous)) = self.samples.back() {
            if now <= last { return; }
            let delta = position - previous;
            // A pause or a deliberate reversal starts a new velocity estimate;
            // subpixel lift-off jitter must not erase an otherwise clean swipe.
            if now.duration_since(last) > Duration::from_millis(40)
                || delta.abs() > 3. * scale && self.direction * delta < 0.
            {
                self.samples.clear();
                self.samples.push_back((last, previous));
            }
            if delta.abs() > 3. * scale { self.direction = delta.signum(); }
        }
        self.samples.push_back((now, position));
        while self.samples.len() > 20 || self.samples.front().is_some_and(|(t, _)|
            now.duration_since(*t) > Duration::from_millis(100)) {
            self.samples.pop_front();
        }
    }
    pub fn end_drag(&mut self, now: Instant, scale: f32) {
        self.velocity = self.release_velocity(now).clamp(-8000. * scale, 8000. * scale);
        self.fling = Fling::new(self.velocity, scale);
        if self.fling.is_none() { self.velocity = 0.; }
        self.samples.clear();
    }
    fn release_velocity(&self, now: Instant) -> f32 {
        let (Some(&(first, _)), Some(&(last, _))) = (self.samples.front(), self.samples.back()) else { return 0.; };
        if now.saturating_duration_since(last) > Duration::from_millis(80)
            || last.duration_since(first) < Duration::from_millis(8) { return 0.; }
        // Least-squares velocity over recent samples rather than delta / the
        // last event interval. Coalescing and a tiny final move stay harmless.
        let (mut t, mut p, mut tt, mut tp) = (0., 0., 0., 0.);
        for &(at, position) in &self.samples {
            let time = -(last.duration_since(at).as_secs_f64());
            let position = position as f64;
            t += time; p += position; tt += time * time; tp += time * position;
        }
        let n = self.samples.len() as f64;
        ((tp - t * p / n) / (tt - t * t / n)) as f32
    }
    /// Precision deltas already include the platform's touchpad momentum. Pass
    /// them through; animate only discrete wheel notches, never a second fling.
    pub fn scroll(&mut self, value: f32, max: f32, amount: f32, precise: bool, now: Instant) -> f32 {
        if !amount.is_finite() || amount == 0. { return value; }
        let target = self.wheel.map_or(value, |(target, _)| target);
        self.stop();
        if precise { return (value + amount).clamp(0., max); }
        let base = if (target - value) * amount < 0. { value } else { target };
        let target = (base + amount).clamp(0., max);
        if target != value { self.wheel = Some((target, now)); }
        value
    }
    /// Exact displacement integration: no Euler drift, frame-rate dependence or
    /// "too little moved" cutoff that would kill a fling on a zero-time frame.
    pub fn advance(&mut self, value: f32, max: f32, dt: f32, scale: f32, now: Instant) -> f32 {
        let mut next = value;
        if let Some((target, last)) = self.wheel {
            let target = target.clamp(0., max);
            let elapsed = now.saturating_duration_since(last).as_secs_f32().min(0.1);
            next += (target - value) * (1. - (-elapsed / 0.065).exp());
            let settled = (target - next).abs() < 0.25 * scale;
            if settled { next = target; }
            self.wheel = (!settled).then_some((target, now));
        }
        if let Some(fling) = &mut self.fling {
            let delta = fling.advance(if dt.is_finite() { dt.max(0.) } else { 0. });
            next = (next + delta).clamp(0., max);
            self.velocity = fling.velocity();
            if fling.elapsed >= fling.duration || next <= 0. && self.velocity < 0.
                || next >= max && self.velocity > 0. {
                self.fling = None;
                self.velocity = 0.;
            }
        }
        next
    }
}

struct Fling { initial_velocity: f32, duration: f32, elapsed: f32 }
impl Fling {
    // Android's default fling distance law, with the restart-safe deceleration
    // used by Flutter's ClampingScrollSimulation. No bounce or invented wheel
    // momentum. Reference: packages/flutter/lib/src/widgets/scroll_simulation.dart.
    const RATE: f32 = 2.3582018; // ln(0.78) / ln(0.9)
    fn new(velocity: f32, scale: f32) -> Option<Self> {
        let logical_velocity = velocity / scale;
        if !logical_velocity.is_finite() || logical_velocity.abs() < 50. { return None; }
        let reference_velocity = 0.015 * (9.80665 * 39.37 * 160. * 0.84) / 0.35;
        let duration = Self::RATE * 0.35 * (logical_velocity.abs() / reference_velocity).powf(1. / (Self::RATE - 1.));
        Some(Self { initial_velocity: velocity, duration, elapsed: 0. })
    }
    fn remaining(&self) -> f32 { (1. - self.elapsed / self.duration).max(0.) }
    fn velocity(&self) -> f32 { self.initial_velocity * self.remaining().powf(Self::RATE - 1.) }
    fn advance(&mut self, dt: f32) -> f32 {
        let before = self.remaining().powf(Self::RATE);
        self.elapsed = (self.elapsed + dt).min(self.duration);
        self.initial_velocity * self.duration / Self::RATE * (before - self.remaining().powf(Self::RATE))
    }
}

/// Middle-button autoscroll math shared with the retained transcript.
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

#[cfg(test)]
mod tests;
