//! Each scrolling widget owns its extent, wheel filter, capture and inertia.
//! Children get first refusal; a touch drag may promote a button press to the
//! containing scroll view, but a claimed text/code selection is never stolen.
use super::{Capture, Context, Event, Id, Target};
use crate::{render::{Layer, color, contains}, scroll::ScrollMotion};
use sanscale::{Rect, Vec2};
use std::time::Instant;

pub(in crate::app) struct ScrollState {
    pub target: Target,
    pub value: f32,
    pub max: f32,
    pub motion: ScrollMotion,
    pub rect: Rect,
    pub horizontal: bool,
    candidate: Option<Capture>,
    drag: Option<f32>,
}
impl ScrollState {
    pub fn update(&mut self, dt: f32, cx: &mut Context<'_>) {
        let old = self.value;
        let active = self.motion.active();
        if self.candidate.is_none() {
            self.value = self.motion.advance(self.value, self.max, dt, cx.ui.scale, Instant::now());
        }
        cx.ui.dirty |= active || self.value != old;
    }

    pub fn new(scope: Id, horizontal: bool) -> Self {
        Self {
            target: Target { scope, widget: Id::new() },
            value: 0.,
            max: 0.,
            motion: ScrollMotion::default(),
            rect: Rect::new(0., 0., 0., 0.),
            horizontal,
            candidate: None,
            drag: None,
        }
    }
    pub fn stop(&mut self) {
        self.motion.stop();
        self.candidate = None;
        self.drag = None;
    }
    pub fn set(&mut self, value: f32) {
        self.value = value.clamp(0., self.max);
    }
    pub fn shift_wheel(&mut self, delta: f32) {
        if let Some((target, _)) = &mut self.motion.wheel {
            *target = (*target + delta).clamp(0., self.max);
        }
    }
    pub fn moving(&self) -> bool {
        self.motion.active() || self.candidate.is_some_and(|c| c.dragged) || self.dragging_bar()
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
    /// Two-axis views share gesture direction locking as well as physics.
    pub fn axes_event(vertical: &mut Self, horizontal: &mut Self, event: &Event<'_>, child: bool, cx: &mut Context<'_>) -> bool {
        let sideways = match *event {
            Event::Wheel { horizontal, .. } => horizontal,
            Event::Move { point, .. } => cx.ui.capture.is_some_and(|c|
                if c.claimed { c.target == horizontal.target }
                else { (point.x - c.start.x).abs() > 1.5 * (point.y - c.start.y).abs() }),
            _ => false,
        };
        if sideways {
            let handled = horizontal.event(event, child, cx);
            vertical.event(event, handled, cx)
        } else {
            let handled = vertical.event(event, child, cx);
            if matches!(event, Event::Move { .. } | Event::Wheel { .. }) { handled }
            else { horizontal.event(event, handled, cx) }
        }
    }
    pub fn event(&mut self, event: &Event<'_>, child_handled: bool, cx: &mut Context<'_>) -> bool {
        self.event_at(event, child_handled, cx, Instant::now())
    }
    fn event_at(&mut self, event: &Event<'_>, child_handled: bool, cx: &mut Context<'_>, now: Instant) -> bool {
        let old = self.value;
        let mut handled = child_handled;
        match *event {
            Event::Down { pointer, point, touch }
                if contains(self.rect, point) && cx.ui.capture.is_none_or(|c| c.pointer == pointer) =>
            {
                let catching = touch && self.motion.active() && cx.ui.capture.is_none_or(|c| !c.claimed);
                self.motion.begin_drag(-self.coordinate(point), now);
                self.candidate = Some(Capture {
                    target: self.target,
                    pointer,
                    start: point,
                    point,
                    touch,
                    dragged: false,
                    claimed: catching,
                    started: now,
                });
                if catching || cx.ui.capture.is_none() {
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
                    if gesture.touch { self.motion.sample(-self.coordinate(point), cx.ui.scale, now); }
                    if self.max > 0. && (gesture.dragged || distance > 7. * cx.ui.scale) && (owned || available) {
                        self.set(self.value + delta);
                        gesture.dragged = true;
                        gesture.claimed = true;
                        handled = true;
                    }
                    gesture.point = point;
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
                if let Some(gesture) = self.candidate.filter(|c| c.pointer == pointer) {
                    if gesture.touch && gesture.claimed && gesture.dragged {
                        self.motion.end_drag(now, cx.ui.scale);
                        cx.ui.dirty |= self.motion.active();
                    } else { self.motion.stop(); }
                    self.candidate = None;
                }
                self.drag = None;
            }
            Event::Wheel { amount, point, precise, .. } if !child_handled && contains(self.rect, point) => {
                if let Some(candidate) = self.candidate.take()
                    && cx.ui.capture.is_some_and(|c| c.pointer == candidate.pointer && (!c.claimed || c.target == self.target))
                {
                    cx.ui.capture = None;
                }
                self.drag = None;
                self.value = self.motion.scroll(self.value, self.max, amount, precise, now);
                // OnDemand must wake for both wheel easing and precision input.
                cx.ui.dirty |= self.motion.active() || self.value != old;
                handled = true;
            }

            _ => {}
        }
        cx.ui.dirty |= self.value != old;
        handled
    }
}

#[cfg(all(test, not(target_os = "android")))]
#[path = "../../../tests/unit/app/ui/scroll.rs"]
mod tests;
