use super::{Capture, Context, Event, Frame, Id, Target, Widget};
use crate::{
    editor::Editor,
    render::{Interaction, Layer, Renderer, color, contains, contains_rounded},
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
                    cx.ui.hot = Some((self.target, text));
                }
                if let Some(info) = &self.info {
                    cx.ui.hint = Some((self.rect.unwrap(), info.clone()));
                }
                true
            }
            Event::Down { pointer, point, touch } if self.contains(point) => {
                if self.enabled && cx.ui.capture.is_none() {
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
                cx.ui.dirty = true;
                true
            }
            Event::Up { pointer, point }
                if cx.ui.capture.is_some_and(|c| c.target == self.target && c.pointer == pointer) =>
            {
                let capture = cx.ui.capture.take().unwrap();
                self.clicked = self.enabled && !capture.dragged && self.contains(point);
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
    fn handle_event(&mut self, event: &Event<'_>, cx: &mut Context<'_>) -> bool {
        self.control.handle(event, cx, false)
    }
    fn visit_perframe(&mut self, frame: &mut Frame<'_>, cx: &mut Context<'_>) {
        self.control.rect = Some(frame.bounds);
        self.control.clip = frame.clip;
        let old = frame.layer.interaction;
        let capture =
            cx.ui.capture.filter(|c| c.target == self.control.target && !c.dragged && self.control.contains(c.point));
        frame.layer.interaction = Interaction {
            hover: cx.ui.hover.filter(|p| self.control.contains(*p)),
            pressed: capture.map(|c| c.start),
            held: capture.is_some(),
        };
        frame.layer.with_clip(frame.clip, |layer| {
            paint_button(
                &mut cx.services.renderer,
                layer,
                frame.bounds,
                &self.label,
                cx.ui.scale,
                self.primary,
                self.destructive,
            )
        });
        frame.layer.interaction = old;
    }
}

pub(in crate::app) fn paint_button(
    renderer: &mut Renderer,
    layer: &mut Layer,
    rect: Rect,
    label: &str,
    scale: f32,
    primary: bool,
    destructive: bool,
) {
    if rect.width <= 0. || rect.height <= 0. {
        return;
    }
    layer.rounded_rect(
        rect,
        rect.height * 0.5,
        layer.control_color(
            rect,
            color(if destructive {
                0x402b30
            } else if primary {
                0x67d4ff
            } else {
                0x18212b
            }),
        ),
    );
    let style = sanscale::Style {
        chain: renderer.faces.prose[0],
        wrap_em: None,
        align: sanscale::Align::Left,
        line_spacing: 1.,
    };
    if let Some(block) = renderer.text.shape_transient(label, &style) {
        let layout = renderer.text.measure(block);
        let size = (if label.chars().count() == 1 { 22. } else { 14. } * scale)
            .min((rect.width - 12. * scale).max(1.) / layout.width_em().max(1.));
        layer.draws.push(sanscale::Draw {
            block,
            at: Vec2::new(
                rect.x + (rect.width - layout.width_em() * size) / 2.,
                rect.y + (rect.height - layout.height_em() * size) / 2.,
            ),
            size,
            color: color(if destructive {
                0xffb4ab
            } else if primary {
                0x003546
            } else {
                0x67d4ff
            }),
            clip: Some(rect),
            ..Default::default()
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
    pub fn new(id: Id, buttons: &[(A, &str)]) -> Self {
        Self { id, buttons: buttons.iter().map(|(action, label)| (action.clone(), Button::new(id, label))).collect() }
    }
    pub fn event(&mut self, event: &Event<'_>, fields: &mut [&mut TextField], cx: &mut Context<'_>) -> Option<A> {
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
            return None;
        }
        // Controls and fields are painted in this same stacking order. No field
        // list is reconstructed from last frame's semantic hit registrations.
        for (action, button) in self.buttons.iter_mut().rev() {
            let handled = button.handle_event(event, cx);
            if button.control.take_click() {
                return Some(action.clone());
            }
            if handled {
                return None;
            }
        }
        for field in fields.iter_mut().rev() {
            if field.handle_event(event, cx) {
                break;
            }
        }
        None
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
        button.primary = primary;
        button.destructive = destructive;
        button.visit_perframe(&mut Frame { layer: frame.layer, bounds: rect, clip: frame.clip }, cx);
    }
}

/// A component's retained buttons. Keys include the semantic destination so an
/// in-flight press never activates a newly rebound item. No root hit/action table.
pub(in crate::app) struct Controls<A> {
    pub id: Id,
    pub items: Vec<(String, Button, A)>,
}
impl<A: Clone + std::fmt::Debug> Controls<A> {
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
        self.place_key("", choice, rect, clip, rounded)
    }
    pub fn place_key(&mut self, slot: &str, choice: A, rect: Rect, clip: Rect, rounded: bool) -> &mut Button {
        let key = format!("{slot}:{choice:?}");
        let index = self.items.iter().position(|(k, _, _)| k == &key).unwrap_or_else(|| {
            self.items.push((key, Button::new(self.id, ""), choice));
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

impl<A: Clone + std::fmt::Debug> Controls<A> {
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
pub(in crate::app) fn button<A: Clone + std::fmt::Debug>(
    renderer: &mut Renderer,
    layer: &mut Layer,
    controls: &mut Controls<A>,
    rect: Rect,
    label: &str,
    choice: A,
    scale: f32,
    primary: bool,
) {
    paint_button(renderer, layer, rect, label, scale, primary, false);
    let b = controls.place(choice, rect, rect, true);
    b.label = label.into();
    b.primary = primary;
}
pub(in crate::app) fn icon_button<A: Clone + std::fmt::Debug>(
    cx: &mut Context<'_>,
    controls: &mut Controls<A>,
    layer: &mut Layer,
    r: Rect,
    icon: crate::icons::Icon,
    size: f32,
    choice: A,
    primary: bool,
    enabled: bool,
) {
    use crate::icons::Icon;
    let hovered = layer.interaction.hover.is_some_and(|p| contains(r, p));
    let tonal = matches!(icon, Icon::Stop | Icon::Play | Icon::Attachments);
    if primary || tonal || enabled && hovered {
        layer.rounded_rect(
            r,
            r.height / 2.,
            layer.control_color(
                r,
                color(if !enabled {
                    0x303942
                } else if primary {
                    0x67d4ff
                } else if tonal {
                    0x18212b
                } else {
                    0x24303b
                }),
            ),
        );
    }
    let pixels = size * cx.ui.scale;
    cx.services.renderer.icon(
        &cx.services.gpu,
        layer,
        icon,
        Rect::new(r.x + (r.width - pixels) / 2., r.y + (r.height - pixels) / 2., pixels, pixels),
        if !enabled {
            0x68727e
        } else if primary {
            0x003546
        } else if matches!(icon, Icon::Attach) {
            0xb7c2ce
        } else {
            0x67d4ff
        },
    );
    controls.place(choice, r, r, true).control.enabled = enabled;
}
