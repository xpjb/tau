//! Each scrolling widget owns its extent, wheel filter, capture and inertia.
//! Children get first refusal; a touch drag may promote a button press to the
//! containing scroll view, but a claimed text/code selection is never stolen.
use super::{Capture, Context, Event, Id, Target};
use crate::render::{Layer, color, contains};
use sanscale::{Rect, Vec2};
use std::time::Instant;

pub(in crate::app) struct ScrollState {
    pub target: Target,
    pub value: f32,
    pub max: f32,
    pub velocity: f32,
    pub rect: Rect,
    pub horizontal: bool,
    pub(in crate::app) wheel: Option<(f32, Instant)>,
    candidate: Option<Capture>,
    drag: Option<f32>,
}
impl ScrollState {
    pub fn update(&mut self, dt: f32, cx: &mut Context<'_>) {
        let old = self.value;
        if let Some((target, last)) = self.wheel {
            let target = target.clamp(0., self.max);
            let now = Instant::now();
            let elapsed = now.duration_since(last).as_secs_f32().min(0.1);
            let next = self.value + (target - self.value) * (1. - (-elapsed / 0.065).exp());
            let settled = (target - next).abs() < 0.25 * cx.ui.scale;
            self.value = if settled { target } else { next };
            self.wheel = (!settled).then_some((target, now));
            cx.ui.dirty = true;
        }
        if self.candidate.is_none() && self.velocity.abs() > 4. {
            self.set(self.value + self.velocity * dt.min(0.05));
            self.velocity *= (-9. * dt).exp();
            if (old - self.value).abs() < 0.1 {
                self.velocity = 0.;
            }
            cx.ui.dirty = true;
        }
        cx.ui.dirty |= self.value != old;
    }

    pub fn new(scope: Id, horizontal: bool) -> Self {
        Self {
            target: Target { scope, widget: Id::new() },
            value: 0.,
            max: 0.,
            velocity: 0.,
            rect: Rect::new(0., 0., 0., 0.),
            horizontal,
            wheel: None,
            candidate: None,
            drag: None,
        }
    }
    pub fn stop(&mut self) {
        self.wheel = None;
        self.candidate = None;
        self.drag = None;
        self.velocity = 0.;
    }
    pub fn set(&mut self, value: f32) {
        self.value = value.clamp(0., self.max);
    }
    pub fn shift_wheel(&mut self, delta: f32) {
        if let Some((target, _)) = &mut self.wheel {
            *target = (*target + delta).clamp(0., self.max);
        }
    }
    pub fn dragging_bar(&self) -> bool {
        self.drag.is_some()
    }
    fn coordinate(&self, p: Vec2) -> f32 {
        if self.horizontal { p.x } else { p.y }
    }
    fn thumb(&self, scale: f32) -> Option<(Rect, Rect)> {
        if self.horizontal || self.max <= 0. || self.rect.height <= 0. {
            return None;
        }
        let r = self.rect;
        let track = Rect::new(r.x + r.width - 12. * scale, r.y, 12. * scale, r.height);
        let h = (r.height * r.height / (r.height + self.max)).max(28. * scale).min(r.height);
        Some((
            track,
            Rect::new(track.x, track.y + (track.height - h) * (self.value / self.max).clamp(0., 1.), track.width, h),
        ))
    }
    pub fn paint(&self, layer: &mut Layer, cx: &mut Context<'_>) {
        if let Some((track, thumb)) = self.thumb(cx.ui.scale) {
            layer.above();
            let active = cx.ui.capture.is_some_and(|c| c.target == self.target) || cx.ui.hot.is_some_and(|(target, _)| target == self.target);
            let w = if active { 8. } else { 6. } * cx.ui.scale;
            layer.rounded_rect(
                Rect::new(track.x + (track.width - w) / 2., thumb.y, w, thumb.height),
                w / 2.,
                color(if active { 0xa0aaba } else { 0x596575 }),
            );
        }
    }
    /// Call before children for scrollbar priority, then after children for
    /// gesture promotion. A child that owns a drag marks capture.claimed.
    pub fn bar_event(&mut self, event: &Event<'_>, cx: &mut Context<'_>) -> bool {
        if let Event::Hover(point) = *event {
            let over = point.is_some_and(|p| self.thumb(cx.ui.scale).is_some_and(|(track, _)| contains(track, p)));
            if over {
                cx.ui.hot = Some((self.target, super::Cursor::Default));
                return true;
            }
        }
        if let Event::Down { pointer, point, touch } = *event
            && !touch
            && let Some((track, thumb)) = self.thumb(cx.ui.scale)
            && contains(track, point)
        {
            self.stop();
            if contains(thumb, point) {
                self.drag = Some((point.y - thumb.y) / thumb.height);
            } else {
                self.set(self.value + if point.y < thumb.y { -track.height * 0.9 } else { track.height * 0.9 });
            }
            cx.ui.capture = Some(Capture {
                target: self.target,
                pointer,
                start: point,
                point,
                touch,
                dragged: true,
                claimed: true,
                started: Instant::now(),
            });
            cx.ui.dirty = true;
            return true;
        }
        false
    }
    pub fn event(&mut self, event: &Event<'_>, child_handled: bool, cx: &mut Context<'_>) -> bool {
        let old = self.value;
        let mut handled = child_handled;
        match *event {
            Event::Down { pointer, point, touch }
                if contains(self.rect, point) && cx.ui.capture.is_none_or(|c| c.pointer == pointer) =>
            {
                self.wheel = None;
                self.velocity = 0.;
                self.candidate = Some(Capture {
                    target: self.target,
                    pointer,
                    start: point,
                    point,
                    touch,
                    dragged: false,
                    claimed: false,
                    started: Instant::now(),
                });
                if cx.ui.capture.is_none() {
                    cx.ui.capture = self.candidate;
                }
                handled = true;
            }
            Event::Move { pointer, point }
                if self.candidate.is_some_and(|c| c.pointer == pointer)
                    || cx.ui.capture.is_some_and(|c| c.target == self.target && c.pointer == pointer) =>
            {
                if self.drag.is_some() {
                    if let Some((track, thumb)) = self.thumb(cx.ui.scale) {
                        self.set(
                            (point.y - track.y - self.drag.unwrap() * thumb.height)
                                / (track.height - thumb.height).max(1.)
                                * self.max,
                        );
                    }
                    handled = true;
                } else if let Some(mut gesture) = self.candidate {
                    let delta = self.coordinate(gesture.point) - self.coordinate(point);
                    let distance = (self.coordinate(gesture.start) - self.coordinate(point)).abs();
                    let owned = cx.ui.capture.is_some_and(|c| c.target == self.target);
                    let available = cx.ui.capture.is_none_or(|c| c.pointer == pointer && !c.claimed);
                    if self.max > 0. && distance > 7. * cx.ui.scale && (owned || available) {
                        self.set(self.value + delta);
                        self.velocity = if gesture.touch {
                            (delta / gesture.started.elapsed().as_secs_f32().max(0.008))
                                .clamp(-3000. * cx.ui.scale, 3000. * cx.ui.scale)
                        } else {
                            0.
                        };
                        gesture.dragged = true;
                        gesture.claimed = true;
                        handled = true;
                    }
                    gesture.point = point;
                    gesture.started = Instant::now();
                    if gesture.claimed {
                        cx.ui.capture = Some(gesture);
                    }
                    self.candidate = Some(gesture);
                }
            }
            Event::Up { pointer, .. } => {
                if cx.ui.capture.is_some_and(|c| c.target == self.target && c.pointer == pointer) {
                    cx.ui.capture = None;
                    handled = true;
                }
                if self.candidate.is_some_and(|c| c.pointer == pointer) {
                    if self.candidate.unwrap().started.elapsed().as_millis() > 150 {
                        self.velocity = 0.;
                    }
                    self.candidate = None;
                }
                self.drag = None;
            }
            Event::Wheel { amount, point, .. } if !child_handled && contains(self.rect, point) => {
                if let Some(candidate) = self.candidate.take()
                    && cx.ui.capture.is_some_and(|c| c.pointer == candidate.pointer && !c.claimed)
                {
                    cx.ui.capture = None;
                }
                self.velocity = 0.;
                self.drag = None;
                let target = self.wheel.map_or(self.value, |(t, _)| t);
                self.wheel = Some(((target + amount).clamp(0., self.max), Instant::now()));
                // OnDemand needs a first frame before update() can move value
                // and keep the easing animation's redraw chain alive.
                cx.ui.dirty = true;
                handled = true;
            }

            _ => {}
        }
        cx.ui.dirty |= self.value != old;
        handled
    }
}
