use super::{Capture, Context, EditorTarget, Event, Frame, Id, Target, Widget};
use crate::{editor::Editor, render::{color, contains, contains_rounded, Interaction, Layer, Renderer}};
use sanscale::{Rect, Vec2};

pub(in crate::app) struct Control {
    pub target: Target,
    pub rect: Option<Rect>,
    pub clip: Rect,
    pub enabled: bool,
    pub rounded: bool,
    clicked: bool,
}
impl Control {
    pub fn new(scope: Id, rounded: bool) -> Self {
        Self { target: Target { scope, widget: Id::new() }, rect: None, clip: Rect::new(0., 0., 0., 0.), enabled: true, rounded, clicked: false }
    }
    pub fn contains(&self, point: Vec2) -> bool {
        contains(self.clip, point) && self.rect.is_some_and(|r| if self.rounded { contains_rounded(r, [r.height * 0.5; 4], point) } else { contains(r, point) })
    }
    pub fn take_click(&mut self) -> bool { std::mem::take(&mut self.clicked) }
    pub fn handle(&mut self, event: &Event<'_>, cx: &mut Context<'_>, text: bool) -> bool {
        match *event {
            Event::Hover(Some(point)) if self.contains(point) => {
                if self.enabled { cx.ui.hot = Some((self.target, text)); } true
            }
            Event::Down { pointer, point, touch } if self.contains(point) => {
                if self.enabled && cx.ui.capture.is_none() {
                    cx.ui.capture = Some(Capture { target: self.target, pointer, start: point, point, touch, dragged: false });
                    if text { cx.ui.focus = Some(self.target); }
                }
                cx.ui.dirty = true; true
            }
            Event::Move { pointer, point } if cx.ui.capture.is_some_and(|c| c.target == self.target && c.pointer == pointer) => {
                let capture = cx.ui.capture.as_mut().unwrap();
                capture.dragged |= (point.x - capture.start.x).abs() + (point.y - capture.start.y).abs() > 7. * cx.ui.scale;
                capture.point = point; cx.ui.dirty = true; true
            }
            Event::Up { pointer, point } if cx.ui.capture.is_some_and(|c| c.target == self.target && c.pointer == pointer) => {
                let capture = cx.ui.capture.take().unwrap();
                self.clicked = self.enabled && !capture.dragged && self.contains(point);
                cx.ui.dirty = true; true
            }
            _ => false,
        }
    }
}

pub(in crate::app) struct Button {
    pub control: Control,
    pub label: String,
    pub primary: bool,
    pub destructive: bool,
}
impl Button {
    pub fn new(scope: Id, label: &str) -> Self {
        Self { control: Control::new(scope, true), label: label.into(), primary: false, destructive: false }
    }
}
impl Widget for Button {
    fn handle_event(&mut self, event: &Event<'_>, cx: &mut Context<'_>) -> bool { self.control.handle(event, cx, false) }
    fn visit_perframe(&mut self, frame: &mut Frame<'_>, cx: &mut Context<'_>) {
        self.control.rect = Some(frame.bounds); self.control.clip = frame.clip;
        let old = frame.layer.interaction;
        let capture = cx.ui.capture.filter(|c| c.target == self.control.target && !c.dragged && self.control.contains(c.point));
        frame.layer.interaction = Interaction {
            hover: cx.ui.hover.filter(|p| self.control.contains(*p)),
            pressed: capture.map(|c| c.start), held: capture.is_some(),
        };
        frame.layer.with_clip(frame.clip, |layer| paint_button(&mut cx.services.renderer, layer, frame.bounds, &self.label, cx.ui.scale, self.primary, self.destructive));
        frame.layer.interaction = old;
    }
}

pub(in crate::app) fn paint_button(renderer: &mut Renderer, layer: &mut Layer, rect: Rect, label: &str, scale: f32, primary: bool, destructive: bool) {
    if rect.width <= 0. || rect.height <= 0. { return; }
    layer.rounded_rect(rect, rect.height * 0.5,
        layer.control_color(rect, color(if destructive { 0x402b30 } else if primary { 0x67d4ff } else { 0x18212b })));
    let style = sanscale::Style { chain: renderer.faces.prose[0], wrap_em: None, align: sanscale::Align::Left, line_spacing: 1. };
    if let Some(block) = renderer.text.shape_transient(label, &style) {
        let layout = renderer.text.measure(block);
        let size = (if label.chars().count() == 1 { 22. } else { 14. } * scale)
            .min((rect.width - 12. * scale).max(1.) / layout.width_em().max(1.));
        layer.draws.push(sanscale::Draw {
            block, at: Vec2::new(rect.x + (rect.width - layout.width_em() * size) / 2., rect.y + (rect.height - layout.height_em() * size) / 2.),
            size, color: color(if destructive { 0xffb4ab } else if primary { 0x003546 } else { 0x67d4ff }),
            clip: Some(rect), ..Default::default()
        });
    }
}

pub(in crate::app) struct TextField {
    pub control: Control,
    pub editor: Editor,
    pub label: String,
    pub secret: bool,
    pub placeholder: String,
    pub size: f32,
}
impl TextField {
    pub fn new(scope: Id, label: &str, editor: Editor) -> Self {
        Self { control: Control::new(scope, false), editor, label: label.into(), secret: false, placeholder: String::new(), size: 15. }
    }
}
impl Widget for TextField {
    fn handle_event(&mut self, event: &Event<'_>, cx: &mut Context<'_>) -> bool {
        let target = self.control.target;
        let capture = cx.ui.capture.filter(|c| c.target == target);
        let focused = cx.ui.focus == Some(target);
        let renderer = &mut cx.services.renderer;
        match *event {
            Event::Down { point, touch: false, .. } if self.control.enabled && self.control.contains(point) && cx.ui.capture.is_none() => {
                self.editor.hit(&mut renderer.text, renderer.faces.prose[0], point, false);
            }
            Event::Move { pointer, point } if capture.is_some_and(|c| c.pointer == pointer) => {
                let capture = capture.unwrap();
                if capture.touch {
                    self.editor.wheel(&mut renderer.text, renderer.faces.prose[0], capture.point.y - point.y, false);
                } else { self.editor.hit(&mut renderer.text, renderer.faces.prose[0], point, true); }
            }
            Event::Wheel { amount, horizontal, point } if self.control.contains(point) => {
                self.editor.wheel(&mut renderer.text, renderer.faces.prose[0], amount, horizontal);
                cx.ui.dirty = true; return true;
            }
            Event::Tick(dt) if capture.is_some_and(|c| !c.touch && c.dragged) => {
                cx.ui.dirty |= self.editor.drag_scroll(&mut renderer.text, renderer.faces.prose[0], capture.unwrap().point, dt);
                return true;
            }
            Event::Cancel => { self.editor.preedit(String::new(), None); return false; }
            Event::Preedit(text, cursor) if focused => {
                self.editor.preedit(text.into(), cursor); cx.ui.dirty = true; return true;
            }
            Event::Text(text) if focused && self.control.enabled => {
                self.editor.replace(text); cx.ui.dirty = true; return true;
            }
            Event::Native { target: owner, text, replace } if owner == target && self.control.enabled => {
                if replace { self.editor.replace_all(text); } else { self.editor.replace(text); }
                cx.ui.dirty = true; return true;
            }
            Event::Key { key, ctrl, shift } if focused && self.control.enabled => {
                if self.editor.composing() {
                    if key == "Escape" { self.editor.preedit(String::new(), None); }
                } else if ctrl && key.eq_ignore_ascii_case("v") {
                    cx.paste(EditorTarget::Widget(target));
                } else if ctrl && (key.eq_ignore_ascii_case("c") || key.eq_ignore_ascii_case("x")) {
                    cx.services.platform.push(super::PlatformAction::Copy(self.editor.selected().into()));
                    if key.eq_ignore_ascii_case("x") { self.editor.replace(""); }
                } else {
                    self.editor.key(&mut renderer.text, renderer.faces.prose[0], key, ctrl, shift);
                }
                cx.ui.dirty = true;
                // Navigation and composition consume keys even if Editor reports
                // no content mutation. Dialog-level Tab/Enter are routed first.
                return true;
            }
            _ => {}
        }
        let handled = self.control.handle(event, cx, true);
        if self.control.take_click() && cx.ui.mobile {
            cx.native_edit(EditorTarget::Widget(target), self.label.clone(), self.editor.value.clone(), self.secret, self.editor.single_line);
        }
        handled
    }
    fn visit_perframe(&mut self, frame: &mut Frame<'_>, cx: &mut Context<'_>) {
        self.control.rect = Some(frame.bounds); self.control.clip = frame.clip;
        frame.layer.with_clip(frame.clip, |layer| self.editor.draw(&mut cx.services.renderer, layer, frame.bounds, self.size * cx.ui.scale,
            self.control.enabled && cx.ui.focus == Some(self.control.target), self.secret, &self.placeholder, true));
    }
}

/// A small shared form controller; dialog fields remain named/typed on their owner.
/// This is not a schema engine and knows nothing about model commands.
pub(in crate::app) struct Form<A> { pub id: Id, pub buttons: Vec<(A, Button)> }
impl<A: Copy + PartialEq> Form<A> {
    pub fn new(id: Id, buttons: &[(A, &str)]) -> Self {
        Self { id, buttons: buttons.iter().map(|(action, label)| (*action, Button::new(id, label))).collect() }
    }
    pub fn event(&mut self, event: &Event<'_>, fields: &mut [&mut TextField], cx: &mut Context<'_>) -> Option<A> {
        if let Event::Key { key: "Tab", ctrl: false, shift } = event
            && !fields.iter().any(|f| f.editor.composing()) {
            let eligible = fields.iter().filter(|f| f.control.enabled).map(|f| f.control.target).collect::<Vec<_>>();
            if !eligible.is_empty() {
                let current = eligible.iter().position(|t| Some(*t) == cx.ui.focus);
                let i = if *shift { current.map_or(eligible.len() - 1, |i| (i + eligible.len() - 1) % eligible.len()) }
                    else { current.map_or(0, |i| (i + 1) % eligible.len()) };
                cx.ui.focus = Some(eligible[i]); cx.ui.dirty = true;
            }
            return None;
        }
        // Controls and fields are painted in this same stacking order. No field
        // list is reconstructed from last frame's semantic hit registrations.
        for (action, button) in self.buttons.iter_mut().rev() {
            let handled = button.handle_event(event, cx);
            if button.control.take_click() { return Some(*action); }
            if handled { return None; }
        }
        for field in fields.iter_mut().rev() {
            if field.handle_event(event, cx) { break; }
        }
        None
    }
    pub fn begin_frame(&mut self) { for (_, button) in &mut self.buttons { button.control.rect = None; } }
    pub fn button(&mut self, action: A, rect: Rect, primary: bool, destructive: bool, frame: &mut Frame<'_>, cx: &mut Context<'_>) {
        let button = &mut self.buttons.iter_mut().find(|(a, _)| *a == action).expect("declared form button").1;
        button.primary = primary; button.destructive = destructive;
        button.visit_perframe(&mut Frame { layer: frame.layer, bounds: rect, clip: frame.clip }, cx);
    }
}
