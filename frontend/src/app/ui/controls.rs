use super::{Capture, Context, Controller, Event, Frame, Id, Target, UiState, Widget};
use crate::{
    editor::Editor,
    render::{Layer, color, contains, contains_rounded},
};
use sanscale::{Rect, Vec2};

pub(in crate::app) struct Control {
    pub target: Target,
    pub rect: Option<Rect>,
    pub clip: Rect,
    pub enabled: bool,
    pub rounded: bool,
    pub corners: Option<[f32; 4]>,
    clicked: bool,
    pub info: Option<crate::app::Info>,
    pub ripple: Option<crate::app::Ripple>,
}
impl Control {
    pub fn new(scope: Id, rounded: bool) -> Self {
        Self {
            target: Target { scope, widget: Id::new() },
            rect: None,
            clip: Rect::new(0., 0., 0., 0.),
            enabled: true,
            rounded,
            corners: None,
            clicked: false,
            info: None,
            ripple: None,
        }
    }
    pub fn contains(&self, point: Vec2) -> bool {
        contains(self.clip, point)
            && self.rect.is_some_and(|r| {
                if let Some(corners) = self.corners {
                    contains_rounded(r, corners, point)
                } else if self.rounded {
                    contains_rounded(r, [r.height * 0.5; 4], point)
                } else {
                    contains(r, point)
                }
            })
    }
    pub fn take_click(&mut self) -> bool {
        std::mem::take(&mut self.clicked)
    }
    pub fn handle(&mut self, event: &Event<'_>, cx: &mut Context<'_>, text: bool) -> bool {
        match *event {
            Event::Hover(Some(point)) if self.contains(point) => {
                if self.enabled {
                    cx.ui.hot = Some((self.target, if text { super::Cursor::Text } else { super::Cursor::Pointer }));
                }
                if let Some(info) = &self.info {
                    cx.ui.hint = Some((self.rect.unwrap(), info.clone()));
                }
                true
            }
            Event::Down { pointer, point, touch } if self.contains(point) => {
                if self.enabled && cx.ui.capture.is_none() {
                    if !text {
                        self.ripple = Some(crate::app::Ripple::new(self.rect.unwrap(), point));
                    }
                    cx.ui.capture = Some(Capture {
                        target: self.target,
                        pointer,
                        start: point,
                        point,
                        touch,
                        dragged: false,
                        claimed: false,
                        started: std::time::Instant::now(),
                    });
                    if text && !touch {
                        cx.ui.focus = Some(self.target);
                    }
                }
                cx.ui.dirty = true;
                true
            }
            Event::Move { pointer, point }
                if cx.ui.capture.is_some_and(|c| c.target == self.target && c.pointer == pointer) =>
            {
                let capture = cx.ui.capture.as_mut().unwrap();
                capture.dragged |=
                    (point.x - capture.start.x).abs() + (point.y - capture.start.y).abs() > 7. * cx.ui.scale;
                capture.point = point;
                if capture.dragged {
                    self.ripple = None;
                }
                cx.ui.dirty = true;
                true
            }
            Event::Up { pointer, point }
                if cx.ui.capture.is_some_and(|c| c.target == self.target && c.pointer == pointer) =>
            {
                let capture = cx.ui.capture.take().unwrap();
                self.clicked = self.enabled && !capture.dragged && self.contains(point);
                if self.clicked {
                    if let Some(ripple) = &mut self.ripple {
                        ripple.release();
                    }
                } else {
                    self.ripple = None;
                }
                if self.clicked
                    && capture.touch
                    && capture.started.elapsed().as_millis() >= 450
                    && let Some(info) = &self.info
                {
                    self.clicked = false;
                    cx.ui.requests.push_back(super::Request::Tip { info: info.clone(), rect: self.rect.unwrap() });
                }
                cx.ui.dirty = true;
                true
            }
            Event::Cancel => {
                self.ripple = None;
                false
            }
            _ => false,
        }
    }
    pub fn highlight(&mut self, layer: &mut Layer, ui: &mut UiState, pinned: bool) {
        let Some(rect) = self.rect else {
            return;
        };
        if !self.enabled { self.ripple = None; }
        let capture = ui.capture.filter(|c| c.target == self.target && !c.dragged);
        let now = std::time::Instant::now();
        let ripple = self.ripple.as_ref().and_then(|r| r.paint(rect, now, capture.is_some()));
        if ripple.is_none() {
            self.ripple = None;
        }
        ui.dirty |= self.ripple.as_ref().is_some_and(|r| r.animating(now));
        let hovering = self.enabled
            && ui.hot.is_some_and(|(target, _)| target == self.target)
            && ui.hover.is_some_and(|p| self.contains(p))
            && (ui.capture.is_none() || capture.is_some_and(|c| self.contains(c.point)));
        let corners = self.corners.unwrap_or([if self.rounded { rect.height / 2. } else { 0. }; 4]);
        layer.surface_highlight(rect, corners, self.clip, pinned, hovering, ripple);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum ButtonStyle {
    Primary,
    Tonal,
    Quiet,
    Destructive,
}

pub(in crate::app) struct Button {
    pub control: Control,
    pub label: String,
    pub style: ButtonStyle,
    pub icon: Option<(crate::icons::Icon, f32)>,
}
impl Button {
    pub fn new(scope: Id, label: &str) -> Self {
        Self { control: Control::new(scope, true), label: label.into(), style: ButtonStyle::Tonal, icon: None }
    }
}
impl Widget for Button {
    fn owns(&self, target: Target, _: &Controller, _: &UiState) -> bool {
        self.control.target == target
    }

    fn handle_event(&mut self, event: &Event<'_>, cx: &mut Context<'_>) -> bool {
        self.control.handle(event, cx, false)
    }
    fn visit_perframe(&mut self, frame: &mut Frame<'_>, cx: &mut Context<'_>) {
        let rect = frame.bounds;
        self.control.rect = Some(rect);
        self.control.clip = frame.clip;
        let (background, ink) = match self.style {
            ButtonStyle::Primary => (Some(0x67d4ff), 0x003546),
            ButtonStyle::Tonal => (Some(0x18212b), 0x67d4ff),
            ButtonStyle::Quiet => (None, 0x67d4ff),
            ButtonStyle::Destructive => (Some(0x402b30), 0xffb4ab),
        };
        let enabled = self.control.enabled;
        frame.layer.with_clip(frame.clip, |layer| {
            if let Some(background) = background {
                layer.rounded_rect(rect, rect.height / 2., color(if enabled { background } else { 0x303942 }));
            }
            self.control.highlight(layer, cx.ui, cx.ui.focus == Some(self.control.target));
            if let Some((icon, size)) = self.icon {
                let size = size * cx.ui.scale;
                cx.services.renderer.icon(
                    &cx.services.gpu,
                    layer,
                    icon,
                    Rect::new(rect.x + (rect.width - size) / 2., rect.y + (rect.height - size) / 2., size, size),
                    if enabled { ink } else { 0x68727e },
                );
            } else {
                let renderer = &mut cx.services.renderer;
                let style = sanscale::Style {
                    chain: renderer.faces.prose[0],
                    wrap_em: None,
                    align: sanscale::Align::Left,
                    line_spacing: 1.,
                };
                if let Some(block) = renderer.text.shape_transient(&self.label, &style) {
                    let layout = renderer.text.measure(block);
                    let size = (if self.label.chars().count() == 1 { 22. } else { 14. } * cx.ui.scale)
                        .min((rect.width - 12. * cx.ui.scale).max(1.) / layout.width_em().max(1.));
                    layer.draws.push(sanscale::Draw {
                        block,
                        at: Vec2::new(
                            rect.x + (rect.width - layout.width_em() * size) / 2.,
                            rect.y + (rect.height - layout.height_em() * size) / 2.,
                        ),
                        size,
                        color: color(if enabled { ink } else { 0x68727e }),
                        clip: Some(rect),
                        ..Default::default()
                    });
                }
            }
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
    pub decorated: bool,
}
impl TextField {
    pub fn new(scope: Id, label: &str, editor: Editor) -> Self {
        Self {
            control: Control::new(scope, false),
            editor,
            label: label.into(),
            secret: false,
            placeholder: String::new(),
            size: 15.,
            decorated: true,
        }
    }
}
impl Widget for TextField {
    fn owns(&self, target: Target, _: &Controller, _: &UiState) -> bool {
        self.control.target == target
    }

    fn handle_event(&mut self, event: &Event<'_>, cx: &mut Context<'_>) -> bool {
        let target = self.control.target;
        let capture = cx.ui.capture.filter(|c| c.target == target);
        let focused = cx.ui.focus == Some(target);
        let renderer = &mut cx.services.renderer;
        match *event {
            Event::Down { point, touch: false, .. }
                if self.control.enabled && self.control.contains(point) && cx.ui.capture.is_none() =>
            {
                self.editor.hit(&mut renderer.text, renderer.faces.prose[0], point, false);
            }
            Event::Move { pointer, point } if capture.is_some_and(|c| c.pointer == pointer) => {
                let capture = capture.unwrap();
                if capture.touch {
                    if let Some(c) = &mut cx.ui.capture {
                        c.claimed = true;
                    }
                    self.editor.wheel(&mut renderer.text, renderer.faces.prose[0], capture.point.y - point.y, false);
                } else {
                    self.editor.hit(&mut renderer.text, renderer.faces.prose[0], point, true);
                }
            }
            Event::Wheel { amount, horizontal, point } if self.control.contains(point) => {
                self.editor.wheel(&mut renderer.text, renderer.faces.prose[0], amount, horizontal);
                cx.ui.dirty = true;
                return true;
            }
            Event::Tick(dt) if capture.is_some_and(|c| !c.touch && c.dragged) => {
                cx.ui.dirty |=
                    self.editor.drag_scroll(&mut renderer.text, renderer.faces.prose[0], capture.unwrap().point, dt);
                return true;
            }
            Event::Cancel => {
                if self.editor.composing() && cx.ui.native.is_none() {
                    self.editor.preedit(String::new(), None);
                }
                return false;
            }
            Event::Preedit(text, cursor) if focused => {
                self.editor.preedit(text.into(), cursor);
                cx.ui.dirty = true;
                return true;
            }
            Event::Text(text) if focused && self.control.enabled => {
                self.editor.replace(text);
                cx.ui.dirty = true;
                return true;
            }
            Event::Paste { target: owner, text } if owner == target && self.control.enabled => {
                self.editor.replace(text);
                cx.ui.dirty = true;
                return true;
            }
            Event::Key { key, ctrl, shift } if focused && self.control.enabled => {
                if self.editor.composing() {
                    if key == "Escape" {
                        self.editor.preedit(String::new(), None);
                    }
                } else if ctrl && key.eq_ignore_ascii_case("v") {
                    cx.paste(target);
                } else if ctrl && (key.eq_ignore_ascii_case("c") || key.eq_ignore_ascii_case("x")) {
                    cx.services.platform.push(super::PlatformAction::Copy(self.editor.selected().into()));
                    if key.eq_ignore_ascii_case("x") {
                        self.editor.replace("");
                    }
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
        if let Some(capture) = &mut cx.ui.capture
            && capture.target == target
            && !capture.touch
        {
            capture.claimed = true;
        }
        if self.control.take_click() && cx.ui.mobile {
            cx.ui.focus = Some(target);
            if let Event::Up { point, .. } = event {
                let renderer = &mut cx.services.renderer;
                self.editor.hit(&mut renderer.text, renderer.faces.prose[0], *point, false);
                if capture.is_some_and(|c| c.started.elapsed().as_millis() >= 450) {
                    self.editor.select_word();
                    cx.services.platform.push(super::PlatformAction::InputMenu);
                }
            }
            cx.focus_native(target, self.editor.native_id());
        }
        handled
    }
    fn visit_perframe(&mut self, frame: &mut Frame<'_>, cx: &mut Context<'_>) {
        self.control.rect = Some(frame.bounds);
        self.control.clip = frame.clip;
        frame.layer.with_clip(frame.clip, |layer| {
            self.editor.draw(
                &mut cx.services.renderer,
                layer,
                frame.bounds,
                self.size * cx.ui.scale,
                self.control.enabled && cx.ui.focus == Some(self.control.target),
                self.secret,
                &self.placeholder,
                self.decorated,
            )
        });
    }
}

/// A small shared form controller; dialog fields remain named/typed on their owner.
/// This is not a schema engine and knows nothing about model commands.
pub(in crate::app) struct Form<A> {
    pub id: Id,
    pub buttons: Vec<(A, Button)>,
}
impl<A: Clone + PartialEq> Form<A> {
    pub fn owns(&self, target: Target) -> bool {
        self.buttons.iter().any(|(_, b)| b.control.target == target && b.control.rect.is_some())
    }

    pub fn new(id: Id, buttons: &[(A, &str)]) -> Self {
        Self { id, buttons: buttons.iter().map(|(action, label)| (action.clone(), Button::new(id, label))).collect() }
    }
    pub fn event(
        &mut self,
        event: &Event<'_>,
        fields: &mut [&mut TextField],
        cx: &mut Context<'_>,
    ) -> (bool, Option<A>) {
        if let Event::Key { key: "Tab", ctrl: false, shift } = event
            && !fields.iter().any(|f| f.editor.composing())
        {
            let eligible = fields.iter().filter(|f| f.control.enabled).map(|f| f.control.target).collect::<Vec<_>>();
            if !eligible.is_empty() {
                let current = eligible.iter().position(|t| Some(*t) == cx.ui.focus);
                let i = if *shift {
                    current.map_or(eligible.len() - 1, |i| (i + eligible.len() - 1) % eligible.len())
                } else {
                    current.map_or(0, |i| (i + 1) % eligible.len())
                };
                cx.ui.focus = Some(eligible[i]);
                cx.ui.dirty = true;
            }
            return (true, None);
        }
        // Controls and fields are painted in this same stacking order. No field
        // list is reconstructed from last frame's semantic hit registrations.
        for (action, button) in self.buttons.iter_mut().rev() {
            let handled = button.handle_event(event, cx);
            if button.control.take_click() {
                return (true, Some(action.clone()));
            }
            if handled {
                return (true, None);
            }
        }
        let handled = super::dispatch_children(fields.iter_mut().map(|f| &mut **f as &mut dyn Widget), event, cx);
        (handled, None)
    }
    pub fn begin_frame(&mut self) {
        for (_, button) in &mut self.buttons {
            button.control.rect = None;
        }
    }
    pub fn button(
        &mut self,
        action: A,
        rect: Rect,
        primary: bool,
        destructive: bool,
        frame: &mut Frame<'_>,
        cx: &mut Context<'_>,
    ) {
        let button = &mut self.buttons.iter_mut().find(|(a, _)| *a == action).expect("declared form button").1;
        button.style = if destructive {
            ButtonStyle::Destructive
        } else if primary {
            ButtonStyle::Primary
        } else {
            ButtonStyle::Tonal
        };
        button.visit_perframe(&mut Frame { layer: frame.layer, bounds: rect, clip: frame.clip }, cx);
    }
}

/// A component's retained buttons. Typed keys include the semantic destination so an
/// in-flight press never activates a newly rebound item. No root hit/action table.
pub(in crate::app) struct Controls<A> {
    pub id: Id,
    pub items: Vec<(Option<usize>, Button, A)>,
}
impl<A: Clone + PartialEq> Controls<A> {
    pub fn owns(&self, target: Target) -> bool {
        self.items.iter().any(|(_, b, _)| b.control.target == target && b.control.rect.is_some())
    }

    pub fn new(id: Id) -> Self {
        Self { id, items: vec![] }
    }
    pub fn begin(&mut self) {
        for (_, b, _) in &mut self.items {
            b.control.rect = None;
        }
    }
    pub fn finish(&mut self, cx: &mut Context<'_>) {
        self.items.retain(|(_, b, _)| {
            if b.control.rect.is_some() {
                true
            } else {
                cx.ui.detach_target(b.control.target);
                false
            }
        });
    }
    pub fn place(&mut self, choice: A, rect: Rect, clip: Rect, rounded: bool) -> &mut Button {
        self.place_in(None, choice, rect, clip, rounded)
    }
    pub fn place_in(&mut self, slot: Option<usize>, choice: A, rect: Rect, clip: Rect, rounded: bool) -> &mut Button {
        let index =
            self.items.iter().position(|(key, _, action)| *key == slot && *action == choice).unwrap_or_else(|| {
                self.items.push((slot, Button::new(self.id, ""), choice));
                self.items.len() - 1
            });
        let item = self.items.remove(index);
        self.items.push(item);
        let button = &mut self.items.last_mut().unwrap().1;
        button.control.rect = Some(rect);
        button.control.clip = clip;
        button.control.rounded = rounded;
        button
    }
    pub fn event(&mut self, event: &Event<'_>, cx: &mut Context<'_>) -> (bool, Option<A>) {
        for (_, button, choice) in self.items.iter_mut().rev() {
            if button.handle_event(event, cx) {
                return (true, button.control.take_click().then(|| choice.clone()));
            }
        }
        (false, None)
    }
    pub fn contains(&self, point: Vec2) -> bool {
        self.items.iter().any(|(_, b, _)| b.control.contains(point))
    }
}

impl<A: Clone + PartialEq> Controls<A> {
    pub fn context(&self, event: &Event<'_>, cx: &Context<'_>) -> Option<(A, Vec2)> {
        let point = match *event {
            Event::Context(point) => point,
            Event::Tick(_) | Event::Up { .. } => {
                let capture = cx.ui.capture?;
                if !capture.touch || capture.dragged || capture.started.elapsed().as_millis() < 450 {
                    return None;
                }
                let (_, button, choice) = self
                    .items
                    .iter()
                    .find(|(_, b, _)| b.control.target == capture.target && b.control.contains(capture.point))?;
                let _ = button;
                return Some((choice.clone(), capture.point));
            }
            _ => return None,
        };
        self.items.iter().rev().find(|(_, b, _)| b.control.contains(point)).map(|(_, _, a)| (a.clone(), point))
    }
    pub fn hints(&self) -> impl Iterator<Item = (Rect, &crate::app::Info)> {
        self.items
            .iter()
            .filter_map(|(_, b, _)| {
                b.control
                    .rect
                    .zip(b.control.info.as_ref())
                    .map(|(r, i)| (crate::render::intersect(r, b.control.clip), i))
            })
            .filter(|(r, _)| r.width > 0. && r.height > 0.)
    }
}
impl<A: Clone + PartialEq> Controls<A> {
    pub fn button(
        &mut self,
        cx: &mut Context<'_>,
        layer: &mut Layer,
        rect: Rect,
        label: &str,
        choice: A,
        style: ButtonStyle,
        clip: Rect,
    ) {
        let button = self.place(choice, rect, clip, true);
        button.label = label.into();
        button.style = style;
        button.icon = None;
        button.visit_perframe(&mut Frame { layer, bounds: rect, clip }, cx);
    }
    pub fn icon(
        &mut self,
        cx: &mut Context<'_>,
        layer: &mut Layer,
        rect: Rect,
        icon: crate::icons::Icon,
        size: f32,
        choice: A,
        style: ButtonStyle,
        enabled: bool,
        clip: Rect,
    ) {
        let button = self.place(choice, rect, clip, true);
        button.style = style;
        button.icon = Some((icon, size));
        button.control.enabled = enabled;
        button.visit_perframe(&mut Frame { layer, bounds: rect, clip }, cx);
    }
}
