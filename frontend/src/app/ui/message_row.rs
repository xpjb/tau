use super::{Context, Controller, Event, Frame, Id, Menu, MenuChoice, Request, Target, UiState, Widget};
use super::{attachments::AttachmentCard, controls::Control};
use crate::{app::{attachments, code, literal}, clock, controller::Chat, feed::{MessageId, MessageBody}, icons::Icon, render::color};
use sanscale::{Rect, Vec2};
use std::hash::{Hash, Hasher};
use tau_protocol::{EventKind, EventRole, EventPhase, ClientCommand, QueueOperation, QueueRef};

/// Only identities are kept in the transcript's order/height index. Text,
/// metadata and commands are borrowed from their existing owners when needed.
#[derive(Clone, Debug)]
pub(in crate::app) enum ItemId {
    Message(MessageId),
    Details { key: String, first: String },
    Tool(String),
    Thinking(String),
}
impl ItemId {
    pub fn key(&self, session: &str) -> String {
        match self {
            Self::Message(id) => id.key(session), Self::Details { key, .. } => format!("{session}/{key}"),
            Self::Tool(id) => format!("{session}/tool:{id}"), Self::Thinking(id) => format!("{session}/thinking:{id}"),
        }
    }
    pub fn root<'a>(&'a self, chat: &'a Chat) -> Option<&'a str> {
        match self {
            Self::Message(id) => chat.feed.messages.get(id)?.event.as_deref(),
            Self::Details { first, .. } => Some(first), Self::Tool(id) | Self::Thinking(id) => Some(id),
        }
    }
    pub fn sender(&self, chat: &Chat) -> EventRole {
        let Self::Message(id) = self else { return EventRole::Assistant; };
        let m = &chat.feed.messages[id];
        if let Some(e) = m.event.as_deref().and_then(|id| chat.feed.event(id)) { return if e.role == EventRole::Tool { EventRole::Assistant } else { e.role }; }
        if m.queue.is_some() { return EventRole::User; }
        if m.intent.as_ref().is_some_and(|id| chat.local.pending.iter().any(|p| &p.request.id == id
            && matches!(p.request.command, ClientCommand::QueueControl { .. } | ClientCommand::Abort { .. }))) { EventRole::System }
        else { EventRole::User }
    }
    pub fn stamp(&self, chat: &Chat) -> u64 {
        let mut hash = std::hash::DefaultHasher::new();
        let feed = &chat.feed;
        if let Some(id) = self.root(chat) {
            for id in std::iter::once(id).chain(feed.children.get(id).into_iter().flatten().map(String::as_str)) {
                feed.event(id).hash(&mut hash); feed.bodies.get(id).hash(&mut hash);
                feed.block_states.get(id).map(|s| s.as_str()).hash(&mut hash);
            }
        }
        if let Self::Message(id) = self {
            let m = &feed.messages[id];
            m.body.hash(&mut hash);
            if let Some(q) = m.queue.as_ref().and_then(|id| feed.queue.requests.iter().find(|q| &q.request_id == id)) {
                q.hash(&mut hash); feed.queue.paused.hash(&mut hash); feed.queue_transitions.get(&q.request_id).hash(&mut hash);
            }
            if let Some(p) = m.intent.as_ref().and_then(|id| chat.local.pending.iter().find(|p| &p.request.id == id)) {
                (&p.text, &p.detail, p.status.label(), p.started_at_ms).hash(&mut hash);
            }
        }
        // Expansion changes only invalidate the relevant retained disclosure.
        match self {
            Self::Details { first, .. } => {
                let group = detail_group(chat, first);
                crate::details::group_state(&group, &chat.local).1.hash(&mut hash);
                group.iter().find_map(|e| clock::event_ms(e)).hash(&mut hash);
            },
            Self::Tool(id) => for key in [format!("tool:{id}"), format!("tool:{id}:Input"), format!("tool:{id}:Output"), format!("tool:{id}:Error")] { chat.local.expansion.get(&key).hash(&mut hash); },
            _ => {},
        }
        hash.finish()
    }
}
pub(in crate::app) fn is_detail(chat: &Chat, id: &MessageId) -> bool {
    chat.feed.messages[id].event.as_deref().and_then(|id| chat.feed.event(id)).is_some_and(|e|
        e.attachment.is_none() && (matches!(e.kind, EventKind::Thinking | EventKind::Tool) || e.role == EventRole::Tool))
}

struct Disclosure {
    key: String,
    control: Control,
    open: Option<bool>,
    label: String,
    error: bool,
    top: f32,
}
impl Disclosure {
    fn new(scope: Id, key: String) -> Self { Self { key, control: Control::new(scope, false), open: None, label: String::new(), error: false, top: 0. } }
    fn height(&self, s: f32) -> f32 { if self.open.is_some() { 28. * s } else { 18. * s } }
    fn event(&mut self, event: &Event<'_>, toggle: &mut Option<(String, bool)>, cx: &mut Context<'_>) -> bool {
        if let Some(open) = self.open {
            let handled = self.control.handle(event, cx, false);
            if self.control.take_click() { *toggle = Some((self.key.clone(), !open)); }
            handled
        } else { false }
    }
    fn paint(&mut self, frame: &mut Frame<'_>, x: f32, top: f32, width: f32, cx: &mut Context<'_>) {
        let s = cx.ui.scale;
        let r = Rect::new(x - 4. * s, top + self.top, width + 8. * s, self.height(s));
        self.control.rect = Some(r); self.control.clip = frame.clip; self.control.corners = Some([4. * s; 4]);
        let ink = if self.error { 0xffb4ab } else { 0xb7c2ce };
        let inset = if let Some(open) = self.open {
            cx.services.renderer.clipped_icon(&cx.services.gpu, frame.layer, if open { Icon::ChevronDown } else { Icon::ChevronRight },
                Rect::new(x, r.y + 6. * s, 14. * s, 14. * s), ink, frame.clip);
            18. * s
        } else { 0. };
        cx.services.renderer.clipped_label(frame.layer, &self.label, Rect::new(x + inset, r.y + 5. * s, width - inset, 20. * s),
            if self.open.is_some() { 12. * s } else { 11. * s }, color(ink), false, frame.clip);
        if self.open.is_some() { self.control.highlight(frame.layer, cx.ui, false); }
    }
}
struct Section {
    heading: Disclosure,
    key: String,
    text: Control,
    top: f32,
    height: f32,
    visible: bool,
}
impl Section {
    fn new(scope: Id, key: String) -> Self {
        Self { key: format!("{key}:text"), heading: Disclosure::new(scope, key), text: Control::new(scope, false), top: 0., height: 0., visible: false }
    }
    fn measure(&mut self, source: &str, length: u64, missing: bool, limited: bool, width: f32, top: f32, session: &str, cx: &mut Context<'_>) -> f32 {
        self.visible = length > 0 || !source.is_empty() || missing;
        if !self.visible { return top; }
        let s = cx.ui.scale;
        let large = length > 1200 || source.bytes().filter(|b| *b == b'\n').count() >= 16;
        let open = !large || cx.model.chats[session].local.expansion.get(&self.heading.key).copied().unwrap_or(false);
        self.heading.open = large.then_some(open); self.heading.top = top;
        self.top = top + self.heading.height(s);
        self.height = 0.;
        if open {
            let mut text = if missing && source.is_empty() { "Loading…".into() } else { code(source) };
            if missing && !source.is_empty() { text.push_str("\n\n[More output is loading.]"); }
            if limited { text.push_str("\n\n[Preview limited. Fetch the complete message with Copy.]"); }
            self.height = cx.services.renderer.message_height(&format!("{session}/{}", self.key), &text, width, 12. / 0.9 * s) + 6. * s;
        }
        self.top + self.height
    }
    fn paint(&mut self, frame: &mut Frame<'_>, x: f32, top: f32, width: f32, session: &str, horizontal: f32, cx: &mut Context<'_>) {
        if !self.visible { return; }
        self.heading.paint(frame, x, top, width, cx);
        if self.height > 0. {
            let r = Rect::new(x, top + self.top, width, self.height);
            self.text.rect = Some(r); self.text.clip = frame.clip;
            cx.services.renderer.message(frame.layer, &format!("{session}/{}", self.key), Vec2::new(x, r.y),
                crate::render::intersect(frame.clip, Rect::new(x, frame.clip.y, width, frame.clip.height)), horizontal);
        }
    }
}
struct ToolView { heading: Disclosure, input: Section, output: Section, open: bool }
impl ToolView {
    fn new(scope: Id, id: &str) -> Self {
        Self { heading: Disclosure::new(scope, format!("tool:{id}")), input: Section::new(scope, format!("tool:{id}:Input")),
            output: Section::new(scope, format!("tool:{id}:Output")), open: false }
    }
    fn controls(&self) -> [&Control; 5] { [&self.heading.control, &self.input.heading.control, &self.input.text, &self.output.heading.control, &self.output.text] }
    fn hide(&mut self) { for control in [&mut self.heading.control, &mut self.input.heading.control, &mut self.input.text, &mut self.output.heading.control, &mut self.output.text] { control.rect = None; } }
    fn measure(&mut self, id: &str, width: f32, session: &str, cx: &mut Context<'_>) -> f32 {
        let chat = &cx.model.chats[session]; let feed = &chat.feed; let e = feed.event(id).unwrap();
        self.open = chat.local.expansion.get(&self.heading.key).copied().unwrap_or(false);
        self.heading.open = Some(self.open);
        self.heading.label = format!("Tool · {}{}", e.tool_name.as_deref().unwrap_or("tool"), feed.block_states.get(id).map(|s| format!(" · {}", s.as_str())).unwrap_or_default());
        let results = if e.role == EventRole::Tool { vec![e] } else { feed.children.get(id).into_iter().flatten().filter_map(|id| feed.event(id)).filter(|e| e.kind == EventKind::Text && e.attachment.is_none()).collect() };
        let error = e.is_error || results.last().is_some_and(|e| e.is_error);
        self.heading.error = error;
        let s = cx.ui.scale; let mut top = 28. * s;
        if self.open {
            let output = results.iter().map(|e| e.text.as_str()).collect::<Vec<_>>().join("\n\n");
            let length = results.iter().map(|e| feed.bodies.get(&e.id).map_or(e.text.len() as u64, |b| b.length())).sum();
            let missing = results.iter().any(|e| feed.bodies.get(&e.id).is_some_and(|b| b.missing()));
            let limited = results.iter().any(|e| feed.bodies.get(&e.id).is_some_and(|b| b.limited));
            let input = if e.role == EventRole::Tool { String::new() } else { e.text.clone() };
            let input_body = (e.role != EventRole::Tool).then(|| feed.bodies.get(id)).flatten();
            let input_length = input_body.map_or(input.len() as u64, |b| b.length());
            let input_missing = input_body.is_some_and(|b| b.missing()); let input_limited = input_body.is_some_and(|b| b.limited);
            self.input.heading.label = "Input".into();
            let label = if error { "Error" } else { "Output" };
            let key = format!("tool:{id}:{label}");
            if self.output.heading.key != key { cx.ui.detach_target(self.output.heading.control.target); self.output = Section::new(self.heading.control.target.scope, key); }
            self.output.heading.label = label.into(); self.output.heading.error = error;
            top = self.input.measure(&input, input_length, input_missing, input_limited, width, top, session, cx);
            top = self.output.measure(&output, length, missing, limited, width, top, session, cx);
        }
        top + 8. * s
    }
}

/// The actual retained owner. No Row/Line description, source clone, or action
/// catalogue is stored here. Renderer text documents are retained independently.
pub(in crate::app) struct MessageRow {
    pub item: ItemId,
    pub key: String,
    pub control: Control,
    disclosure: Option<Disclosure>,
    tool: Option<ToolView>,
    pub attachment: Option<AttachmentCard>,
    pub toggle: Option<(String, bool)>,
    pub corners: [f32; 4],
    pub horizontal: f32,
    timestamp: String,
}
impl MessageRow {
    pub fn new(item: ItemId, session: &str) -> Self {
        let scope = Id::new();
        let disclosure = if let ItemId::Details { key, .. } = &item { Some(Disclosure::new(scope, key.clone())) } else { None };
        let tool = if let ItemId::Tool(id) = &item { Some(ToolView::new(scope, id)) } else { None };
        Self { key: item.key(session), item, control: Control::new(scope, false), disclosure, tool, attachment: None, toggle: None, corners: [0.; 4], horizontal: 0., timestamp: String::new() }
    }
    pub fn hide(&mut self) {
        self.control.rect = None;
        if let Some(d) = &mut self.disclosure { d.control.rect = None; }
        if let Some(t) = &mut self.tool { t.hide(); }
        if let Some(card) = &mut self.attachment { card.hide(); }
    }
    pub fn detach(&mut self, cx: &mut Context<'_>) {
        cx.ui.detach(self.control.target.scope);
        if let Some(card) = &self.attachment { cx.ui.detach(card.controls.id); }
    }
    pub fn cancel(&mut self) { self.control.ripple = None; if let Some(t) = &mut self.tool { for control in [&mut t.heading.control, &mut t.input.heading.control, &mut t.output.heading.control] { control.ripple = None; } } }
    pub fn text_keys(&self, session: &str) -> Vec<String> {
        if let Some(tool) = &self.tool {
            if !tool.open { return vec![]; }
            [&tool.input, &tool.output].into_iter().filter(|s| s.visible && s.height > 0.).map(|s| format!("{session}/{}", s.key)).collect()
        } else if self.disclosure.is_some() { vec![] } else { vec![self.key.clone()] }
    }
    pub fn expansions(&self, top: f32) -> Vec<(String, f32)> {
        if let Some(d) = &self.disclosure { return vec![(d.key.clone(), top + d.top)]; }
        if let Some(t) = &self.tool {
            let mut out = vec![(t.heading.key.clone(), top)];
            if t.open { for section in [&t.input, &t.output] { if section.visible && section.heading.open.is_some() { out.push((section.heading.key.clone(), top + section.heading.top)); } } }
            out
        } else { vec![] }
    }
    pub fn interest(&self, chat: &Chat) -> Option<String> {
        if matches!(self.item, ItemId::Details { .. }) { None } else { self.item.root(chat).map(str::to_owned) }
    }
    pub fn measure(&mut self, width: f32, session: &str, cx: &mut Context<'_>) -> f32 {
        self.hide();
        let s = cx.ui.scale; let text_width = width - 28. * s;
        if let ItemId::Tool(id) = &self.item { return self.tool.as_mut().unwrap().measure(id, text_width - 16. * s, session, cx); }
        if let Some(d) = &mut self.disclosure {
            let ItemId::Details { first, .. } = &self.item else { unreachable!() };
            let chat = &cx.model.chats[session];
            let group = detail_group(chat, first);
            self.timestamp = clock::label(group.iter().find_map(|e| clock::event_ms(e)));
            d.open = Some(crate::details::group_state(&group, &chat.local).1); d.label = "Details".into(); d.top = 26. * s;
            return 62. * s;
        }
        let chat = &cx.model.chats[session];
        let (source, header, file) = match &self.item {
            ItemId::Thinking(id) => (display_body(chat, id, false), false, None),
            ItemId::Message(id) => {
                let m = &chat.feed.messages[id];
                let event = m.event.as_deref().and_then(|id| chat.feed.event(id));
                let header = event.is_none_or(|e| e.role == EventRole::System || e.is_error || e.error_message.is_some());
                let mut source = match &m.body { MessageBody::Remote(id) => display_body(chat, id, self.item.sender(chat) == EventRole::User), MessageBody::Queue(id) if chat.feed.bodies.get(&format!("queued:{id}")).is_some_and(|b| b.missing()) => "Loading…".into(), _ => literal(m.text(&chat.feed, &chat.local)) };
                if m.event.is_none() && m.queue.is_none() && let Some(p) = m.intent.as_ref().and_then(|id| chat.local.pending.iter().find(|p| &p.request.id == id)) && let Some(detail) = &p.detail { source = literal(&format!("{}\n{detail}", m.text(&chat.feed, &chat.local))); }
                (source, header, event.and_then(|e| e.attachment.as_ref().map(|a| (e.entry_id.clone(), a.clone()))))
            }
            _ => unreachable!(),
        };
        if let Some((entry, file)) = file {
            if self.attachment.as_ref().is_none_or(|card| card.target.entry != entry) {
                if let Some(old) = self.attachment.take() { cx.ui.detach(old.controls.id); }
                self.attachment = Some(AttachmentCard::new(session, &entry, &file, "chat", cx));
            }
            self.attachment.as_mut().unwrap().file = file;
        } else if let Some(old) = self.attachment.take() { cx.ui.detach(old.controls.id); }
        let thinking = matches!(self.item, ItemId::Thinking(_));
        let height = cx.services.renderer.message_height(&self.key, &source, text_width, if thinking { 12. * s } else { 16. * s });
        (if thinking { 6. } else if header { 62. } else { 40. }) * s + if source.is_empty() { 0. } else { height }
            + self.attachment.as_ref().map_or(0., |c| attachments::card_height(&c.file) * s)
    }
    fn menu(&self, point: Vec2, cx: &mut Context<'_>) {
        let Some(session) = cx.model.account.selected.as_deref() else { return; };
        let chat = &cx.model.chats[session];
        let mut options = if cx.services.renderer.selected_text().is_some_and(|s| !s.is_empty()) { vec![("Copy selection".into(), MenuChoice::CopySelection)] } else { vec![] };
        if let ItemId::Message(id) = &self.item { options.extend(message_actions(chat, id, session)); }
        else if let Some(id) = self.item.root(chat) { options.push(("Copy message".into(), MenuChoice::CopyDetails(session.into(), detail_group(chat, id).iter().map(|e| e.id.clone()).collect()))); }
        let menu = Menu::new(point, Some(self.key.clone()), None, options, cx);
        cx.ui.requests.push_back(Request::Menu(Box::new(menu)));
    }
}
impl Widget for MessageRow {
    fn update(&mut self, _dt: f32, cx: &mut Context<'_>) {
        if let Some(point) = self.control.held(cx) {
            cx.ui.capture = None;
            self.menu(point, cx);
        }
    }
    fn owns(&self, target: Target, model: &Controller, ui: &UiState) -> bool {
        let Some(chat) = model.selected() else { return false; };
        let exists = match &self.item { ItemId::Message(id) => chat.feed.messages.contains_key(id), _ => self.item.root(chat).is_some_and(|id| chat.feed.event(id).is_some()) };
        exists && (self.control.target == target || self.disclosure.as_ref().is_some_and(|d| d.control.target == target && d.control.rect.is_some())
            || self.tool.as_ref().is_some_and(|t| t.controls().iter().any(|c| c.target == target && c.rect.is_some()))
            || self.attachment.as_ref().is_some_and(|c| c.owns(target, model, ui)))
    }
    fn handle_event(&mut self, event: &Event<'_>, cx: &mut Context<'_>) -> bool {
        if let Some(card) = &mut self.attachment && card.dispatch(event, cx) { return true; }
        let capture = cx.ui.capture;
        if let Some(t) = &mut self.tool {
            for section in [&mut t.output, &mut t.input] {
                if t.open && section.visible {
                    if matches!(event, Event::Hover(_)) && section.text.handle(event, cx, true) { return true; }
                    if section.heading.event(event, &mut self.toggle, cx) { return true; }
                }
            }
            if t.heading.event(event, &mut self.toggle, cx) { return true; }
        }
        if let Some(d) = &mut self.disclosure && d.event(event, &mut self.toggle, cx) { return true; }
        if self.toggle.is_some() || cx.ui.capture.map(|c| c.target) != capture.map(|c| c.target) { return true; }
        if let Event::Context(point) = *event && self.control.contains(point) { self.menu(point, cx); return true; }
        if matches!(event, Event::Up { .. }) && cx.ui.capture.is_some_and(|c| c.target == self.control.target && c.touch && !c.dragged && c.started.elapsed().as_millis() >= 450) {
            let p = cx.ui.capture.take().unwrap().point; self.menu(p, cx); return true;
        }
        if matches!(event, Event::Hover(_)) {
            let text = self.disclosure.is_none() && self.tool.is_none() && !cx.ui.hover.is_some_and(|p| cx.services.renderer.hit_link(p).is_some());
            return self.control.handle(event, cx, text);
        }
        if matches!(event, Event::Down { touch: false, .. }) { return false; }
        let handled = self.control.handle(event, cx, false);
        if self.control.take_click() && let Event::Up { point, .. } = *event && let Some(link) = cx.services.renderer.hit_link(point) {
            cx.ui.requests.push_back(Request::Open(super::DialogSpec::Operation(super::Operation::Link(link))));
        }
        handled
    }
    fn visit_perframe(&mut self, frame: &mut Frame<'_>, cx: &mut Context<'_>) {
        let session = cx.model.account.selected.clone().unwrap();
        let chat = &cx.model.chats[&session]; let s = cx.ui.scale; let r = frame.bounds;
        let user = self.item.sender(chat) == EventRole::User;
        frame.layer.clipped_corners(r, self.corners, color(if user { 0x164e63 } else { 0x18212b }), frame.clip);
        self.control.rect = Some(r); self.control.clip = frame.clip; self.control.corners = Some(self.corners);
        self.control.highlight(frame.layer, cx.ui, cx.ui.menu_section.as_deref() == Some(&self.key));
        let x = r.x + 14. * s; let width = r.width - 28. * s;
        if let Some(t) = &mut self.tool {
            frame.layer.clipped_rect(Rect::new(x + 4. * s, r.y, width - 4. * s, r.height), color(0x111922), frame.clip);
            t.heading.paint(frame, x + 8. * s, r.y, width - 8. * s, cx);
            if t.open { t.input.paint(frame, x + 16. * s, r.y, width - 16. * s, &session, self.horizontal, cx); t.output.paint(frame, x + 16. * s, r.y, width - 16. * s, &session, self.horizontal, cx); }
            return;
        }
        let thinking = matches!(self.item, ItemId::Thinking(_));
        let (timestamp, title, error) = match &self.item {
            ItemId::Message(id) => message_heading(chat, id),
            ItemId::Details { .. } => (self.timestamp.clone(), None, false),
            _ => (String::new(), None, false),
        };
        if !thinking { cx.services.renderer.clipped_label(frame.layer, &timestamp, Rect::new(x, r.y + 8. * s, width, 16. * s), 11. * s, color(if user { 0xa7bdc9 } else { 0x82909f }), false, frame.clip); }
        if let Some(d) = &mut self.disclosure { d.paint(frame, x, r.y, width, cx); return; }
        let text_top = r.y + if thinking { 0. } else if title.is_some() { 50. * s } else { 28. * s };
        if let Some(title) = title { cx.services.renderer.clipped_label(frame.layer, &title, Rect::new(x, r.y + 28. * s, width, 20. * s), 12. * s, color(if error { 0xffb4ab } else if user { 0x67d4ff } else { 0xa5b4fc }), true, frame.clip); }
        cx.services.renderer.message(frame.layer, &self.key, Vec2::new(x, text_top), crate::render::intersect(frame.clip, Rect::new(x, frame.clip.y, width, frame.clip.height)), self.horizontal);
        if let Some(card) = &mut self.attachment { card.visit_perframe(frame, cx); }
    }
}
fn display_body(chat: &Chat, id: &str, user: bool) -> String {
    let e = chat.feed.event(id).unwrap(); let body = chat.feed.bodies.get(id);
    if e.attachment.is_none() && body.is_some_and(|b| b.missing()) { return "Loading…".into(); }
    let mut source = if user { literal(&e.text) } else { e.text.clone() };
    if body.is_some_and(|b| b.limited) { source.push_str("\n\n[Preview limited. Fetch the complete message with Copy.]"); }
    source
}
fn detail_group<'a>(chat: &'a Chat, root: &str) -> Vec<&'a tau_protocol::Event> {
    let order = &chat.feed.order;
    let Some(index) = order.iter().position(|id| chat.feed.messages[id].event.as_deref() == Some(root)) else { return vec![]; };
    let start = order[..index].iter().rposition(|id| !is_detail(chat, id)).map_or(0, |i| i + 1);
    order[start..].iter().take_while(|id| is_detail(chat, id)).filter_map(|id| chat.feed.messages[id].event.as_deref().and_then(|id| chat.feed.event(id))).collect()
}
fn message_heading(chat: &Chat, id: &MessageId) -> (String, Option<String>, bool) {
    let m = &chat.feed.messages[id];
    let pending = m.intent.as_ref().and_then(|id| chat.local.pending.iter().find(|p| &p.request.id == id));
    if let Some(e) = m.event.as_deref().and_then(|id| chat.feed.event(id)) {
        let error = e.is_error || e.error_message.is_some();
        let title = (e.role == EventRole::System || error).then(|| {
            if e.role == EventRole::User { if pending.is_some() { "You · synchronizing" } else { "You" }.into() }
            else if let Some(error) = &e.error_message { format!("Assistant · {error}") }
            else { format!("{}{}", if e.role == EventRole::Assistant { "Tau" } else { "System" }, if e.phase == EventPhase::Live { " · writing" } else if e.phase == EventPhase::Interrupted { " · interrupted" } else { "" }) }
        });
        return (clock::label(clock::event_ms(e)), title, error);
    }
    if let Some(qid) = &m.queue {
        let q = chat.feed.queue.requests.iter().find(|q| &q.request_id == qid).unwrap();
        let title = if chat.feed.queue_transitions.contains_key(qid) { "Synchronizing message".into() }
            else if let Some(p) = pending.filter(|p| matches!(p.request.command, ClientCommand::QueueControl { .. })) {
                format!("{} · {}", if matches!(p.request.command, ClientCommand::QueueControl { operation: QueueOperation::Edit { .. }, .. }) { "Queue edit" } else { "Queue delete" },
                    if p.status == crate::store::Delivery::Unconfirmed { "unconfirmed · not resent" } else if p.status == crate::store::Delivery::Accepted { "accepted · synchronizing…" } else { "saving…" })
            } else { format!("Queued{}", if chat.feed.queue.paused { " · held" } else { "" }) };
        return (clock::label(q.timestamp_ms), Some(title), false);
    }
    let p = pending.unwrap();
    let prefix = match p.request.command { ClientCommand::QueueControl { operation: QueueOperation::Edit { .. }, .. } => "Queue edit · ", ClientCommand::QueueControl { operation: QueueOperation::Delete { .. }, .. } => "Queue delete · ", ClientCommand::QueueControl { .. } | ClientCommand::Abort { .. } => "Queue action · ", _ => "" };
    (clock::label(p.started_at_ms), Some(format!("{prefix}{}", p.status.label())), matches!(p.status, crate::store::Delivery::Rejected | crate::store::Delivery::Unconfirmed))
}
fn message_actions(chat: &Chat, id: &MessageId, session: &str) -> Vec<(String, MenuChoice)> {
    let feed = &chat.feed; let m = &feed.messages[id]; let text = m.text(feed, &chat.local);
    let mut actions = vec![("Copy text".into(), MenuChoice::Copy(text.into()))];
    if let Some(e) = m.event.as_deref().and_then(|id| feed.event(id)) {
        if matches!(m.body, MessageBody::Remote(_)) && feed.incomplete.contains(&e.id) { actions = vec![("Fetch complete message to copy".into(), MenuChoice::CopyDetails(session.into(), vec![e.id.clone()]))]; }
        if e.phase == EventPhase::Saved { actions.push(("Fork here".into(), MenuChoice::Fork(e.entry_id.clone()))); }
    } else if let Some(qid) = &m.queue {
        if m.intent.is_some() { return actions; }
        let state = &feed.queue;
        let (index, q) = state.requests.iter().enumerate().find(|(_, q)| &q.request_id == qid).unwrap();
        let complete = !feed.incomplete.contains(&format!("queued:{qid}"));
        if !complete { actions.clear(); }
        if !feed.queue_transitions.contains_key(qid) {
            if complete && state.capabilities.iter().any(|c| c == "queue_edit") { actions.push(("Edit".into(), MenuChoice::EditQueue(qid.clone(), q.revision, q.text.clone()))); }
            if state.capabilities.iter().any(|c| c == "queue_delete") { actions.push(("Delete".into(), MenuChoice::Queue(QueueOperation::Delete { request_id: qid.clone(), revision: q.revision }))); }
        }
        if state.available && feed.queue_transitions.is_empty() && state.capabilities.iter().any(|c| c == "queue_run_prefix")
            && let Some(boundary) = state.boundaries.iter().find(|b| b.as_str() == "reasoning_checkpoint").or(state.boundaries.first()) {
            actions.push(("Run through here".into(), MenuChoice::Queue(QueueOperation::Prefix { run_id: state.run_id.clone(), requests: state.requests[..=index].iter().map(|q| QueueRef { request_id: q.request_id.clone(), revision: q.revision }).collect(), boundary: boundary.clone() })));
        }
    } else if let Some(p) = m.intent.as_ref().and_then(|id| chat.local.pending.iter().find(|p| &p.request.id == id)) {
        if matches!(p.status, crate::store::Delivery::Rejected | crate::store::Delivery::Unconfirmed) && matches!(p.request.command, ClientCommand::Prompt { .. }) {
            actions.push(("Restore draft".into(), MenuChoice::Restore(p.request.id.clone())));
            actions.push(("Retry saved message".into(), MenuChoice::RetryPending(p.request.id.clone())));
        }
        actions.push(("Dismiss".into(), MenuChoice::Dismiss(p.request.id.clone())));
    }
    actions
}
