use super::{Context, Controller, Event, Frame, Id, Request, Target, UiState, Widget};
use super::{composer::QuickModels, message_row::{MessageRow, ItemId, is_detail}, scroll::ScrollState};
use crate::{
    app::{Autoscroll, PlatformAction, attachments},
    notice::DownloadTarget,
    render::{color, contains},
};
use sanscale::{Rect, Vec2};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    time::Instant,
};
pub(in crate::app) struct Placed {
    pub key: String,
    item: ItemId,
    stamp: u64,
    pub top: f32,
    pub height: f32,
    sender: tau_protocol::EventRole,
    text_keys: Vec<String>,
    overflow: f32,
}
pub(in crate::app) struct Transcript {
    pub rows: Vec<MessageRow>,
    measured: Option<(u64, u32, u32)>,
    pub scroll: ScrollState,
    pub horizontal: ScrollState,
    pub models: QuickModels,
    pub placed: Vec<Placed>,
    pub placed_session: Option<String>,
    pub expansion_positions: HashMap<String, f32>,
    pub expansion_pin: Option<(String, f32)>,
    pub history_attempt: Option<(String, String, u64, u64)>,
    pub download: Option<DownloadTarget>,
    pub autoscroll: Option<Autoscroll>,
    pub interests: BTreeSet<String>,
    selection: Target,
    pub(super) selecting: bool,
    binding: Option<(String, Option<String>, Option<String>)>,
}
impl Transcript {
    pub fn new() -> Self {
        let id = Id::new();
        Self {
            rows: vec![], measured: None,
            scroll: ScrollState::new(id, false),
            horizontal: ScrollState::new(id, true),
            models: QuickModels::new(),
            placed: vec![],
            placed_session: None,
            expansion_positions: HashMap::new(),
            expansion_pin: None,
            history_attempt: None,
            download: None,
            autoscroll: None,
            interests: BTreeSet::new(),
            selection: Target { scope: id, widget: Id::new() },
            selecting: false,
            binding: None,
        }
    }
    pub fn hide(&mut self) {
        self.scroll.rect = Rect::new(0., 0., 0., 0.);
        self.horizontal.rect = self.scroll.rect;
        self.interests.clear();
        self.models.controls.begin();
        for row in &mut self.rows {
            row.hide();
        }
    }
    pub fn bind(&mut self, cx: &mut Context<'_>) {
        let next =
            (cx.model.identity.clone(), cx.model.account.source_lineage.clone(), cx.model.account.selected.clone());
        if self.binding.as_ref() != Some(&next) {
            cx.ui.detach(self.selection.scope);
            for row in &mut self.rows {
                row.detach(cx);
            }
            cx.services.renderer.selection = None;
            cx.services.renderer.retain_messages(&HashSet::new());
            *self = Self::new();
            self.binding = Some(next);
            cx.ui.dirty = true;
        }
    }
    pub fn cancel(&mut self) {
        self.scroll.stop();
        self.horizontal.stop();
        self.selecting = false;
        self.autoscroll = None;
        self.expansion_pin = None;
        for row in &mut self.rows {
            row.cancel();
        }
    }
    pub fn remember_scroll(&mut self, cx: &mut Context<'_>) {
        if self.download.is_some()
            || self.binding.as_ref().is_none_or(|(identity, lineage, session)| {
                identity != &cx.model.identity
                    || lineage != &cx.model.account.source_lineage
                    || session != &cx.model.account.selected
            })
        {
            return;
        }
        if let Some(id) = cx.model.account.selected.clone()
            && self.placed_session.as_deref() == Some(id.as_str())
            && let Some(anchor) = self.placed.iter().find(|r| r.top + r.height >= self.scroll.value)
            && let Some(chat) = cx.model.chats.get_mut(&id)
        {
            chat.local.position.key = Some(anchor.key.clone());
            chat.local.position.offset = (self.scroll.value - anchor.top) / cx.ui.scale;
            chat.local.position.follow = self.expansion_pin.is_none()
                && self.scroll.wheel.is_none()
                && self.autoscroll.is_none()
                && !self.scroll.dragging_bar()
                && self.scroll.max - self.scroll.value < cx.ui.scale;
        }
    }
    pub fn tail(&mut self, cx: &mut Context<'_>) {
        self.cancel();
        self.download = None;
        self.scroll.value = self.scroll.max;
        if let Some(chat) = cx.model.account.selected.clone().and_then(|id| cx.model.chats.get_mut(&id)) {
            chat.local.position.follow = true;
        }
        cx.ui.dirty = true;
    }
    pub fn toggle(&mut self, key: String, open: bool, cx: &mut Context<'_>) {
        self.scroll.stop();
        self.expansion_pin = self.expansion_positions.get(&key).map(|y| (key.clone(), *y - self.scroll.value));
        if let Some(session) = cx.model.account.selected.clone()
            && let Some(chat) = cx.model.chats.get_mut(&session)
        {
            if key.starts_with("details:") {
                chat.local.details_default = open;
            }
            chat.local.expansion.insert(key, open);
            chat.local.position.follow = false;
            let result = cx.model.save_chat(&session);
            cx.report(result);
        }
    }
    pub fn history_near_edge(&mut self, session: &str, near: bool, cx: &mut Context<'_>) {
        if cx.ui.covered || !near {
            return;
        }
        let feed = &cx.model.chats[session].feed;
        if !feed.synchronized || feed.loading {
            return;
        }
        if let (Some(before), Some(epoch)) = (feed.before, cx.model.epoch) {
            let attempt = (session.to_owned(), feed.generation.clone(), before, epoch);
            if self.history_attempt.as_ref() == Some(&attempt) {
                return;
            }
            self.history_attempt = Some(attempt);
            let result = cx.model.history();
            cx.report(result);
        }
    }
    pub(super) fn validate_download_jump(&mut self, cx: &mut Context<'_>) {
        if self.download.as_ref().is_some_and(|target| {
            !target.matches_source(&cx.model.identity, cx.model.account.source_lineage.as_deref())
                || cx.model.account.selected.as_deref() != Some(target.session.as_str())
        }) {
            self.download = None;
        }
    }

    /// Return true while explicit navigation owns the scroll position. Keep the
    /// request across empty/loading frames and older pages, but never persist an
    /// interim page as the user's destination.
    pub(super) fn locate_download(
        &mut self,
        placements: &[Placed],
        viewport: Rect,
        cx: &mut Context<'_>,
    ) -> bool {
        self.validate_download_jump(cx);
        let Some(target) = &self.download else {
            return false;
        };
        if let Some(placed) = placements.iter().find(|p| p.item.root(&cx.model.chats[&target.session])
            .and_then(|id| cx.model.chats[&target.session].feed.event(id)).is_some_and(|e| e.entry_id == target.entry && e.attachment.is_some()))
        {
            // Center the actual download controls, not the beginning of a long
            // message or image above them. This also works at mobile UI scales.
            let panel =
                attachments::control_panel(Rect::new(0., placed.top, viewport.width, placed.height), cx.ui.scale);
            self.scroll.value = (panel.y + panel.height / 2. - viewport.height / 2.).clamp(0., self.scroll.max);
            let position = &mut cx.model.chats.get_mut(&target.session).unwrap().local.position;
            position.key = Some(placed.key.clone());
            position.offset = (self.scroll.value - placed.top) / cx.ui.scale;
            position.follow = false;
            self.download = None;
        } else {
            self.scroll.value = 0.;
            let feed = &cx.model.chats[&target.session].feed;
            if cx.model.account.missing_chats.contains(&target.session)
                || feed.synchronized && !feed.loading && feed.before.is_none()
            {
                self.download = None;
                cx.model.notice = Some("The download widget is no longer available in this chat.".into());
            }
        }
        true
    }
}
impl Widget for Transcript {
    fn owns(&self, target: Target, model: &Controller, ui: &UiState) -> bool {
        self.binding.as_ref().is_some_and(|(identity, lineage, session)| {
            identity == &model.identity
                && lineage == &model.account.source_lineage
                && session == &model.account.selected
        }) && (self.selection == target
            || self.scroll.target == target
            || self.horizontal.target == target
            || model.account.selected.as_ref().is_some_and(|s| model.quick_start(s))
                && self.models.owns(target, model, ui)
            || self.rows.iter().any(|r| r.owns(target, model, ui)))
    }

    fn handle_event(&mut self, event: &Event<'_>, cx: &mut Context<'_>) -> bool {
        self.route(event, cx)
    }
    fn visit_perframe(&mut self, frame: &mut Frame<'_>, cx: &mut Context<'_>) {
        let Some(session) = cx.model.account.selected.clone().filter(|id| cx.model.chats.contains_key(id)) else {
            self.hide();
            return;
        };
        self.bind(cx);
        self.interests.clear();
        self.models.controls.begin();
        let s = cx.ui.scale;
        let viewport = frame.bounds;
        self.scroll.rect = viewport;
        self.horizontal.rect = viewport;
        let width = (viewport.width - 28. * s).min(900. * s).max(160. * s);
        let x = viewport.x + (viewport.width - width) / 2.;
        let bubble_width = width * 0.9;
        let quick_models = cx.model.quick_start(&session) && !cx.model.model_preferences.slugs.is_empty();
        let quick_h = if quick_models { self.models.height(cx, width) } else { 0. };
        let quick_top = if quick_models && cx.model.chats[&session].feed.order.is_empty() {
            ((viewport.height - quick_h) / 2.).max(12. * s)
        } else { 12. * s };
        let signature = (cx.model.chats[&session].feed.revision, bubble_width.to_bits(), s.to_bits());
        let mut owners = std::mem::take(&mut self.rows).into_iter().map(|row| (row.key.clone(), row)).collect::<HashMap<_, _>>();
        if self.measured != Some(signature) {
            // Presentation owns only ordered child identities. The feed already
            // decided local/queue/history ownership; no body/action descriptions
            // are assembled here. Details headers and each native tool are
            // independently virtualized, rather than retaining a giant Part list.
            let chat = &cx.model.chats[&session];
            let mut items = vec![];
            let mut i = 0;
            while i < chat.feed.order.len() {
                let id = &chat.feed.order[i];
                if !is_detail(chat, id) { items.push(ItemId::Message(id.clone())); i += 1; continue; }
                let group = chat.feed.order[i..].iter().take_while(|id| is_detail(chat, id))
                    .filter_map(|id| chat.feed.messages[id].event.as_deref().and_then(|id| chat.feed.event(id))).collect::<Vec<_>>();
                i += group.len();
                let (key, open) = crate::details::group_state(&group, &chat.local);
                items.push(ItemId::Details { key, first: group[0].id.clone() });
                if open { for e in group { items.push(if e.kind == tau_protocol::EventKind::Thinking && e.role != tau_protocol::EventRole::Tool {
                    ItemId::Thinking(e.id.clone())
                } else { ItemId::Tool(e.id.clone()) }); } }
            }
            let mut previous = std::mem::take(&mut self.placed).into_iter().map(|p| (p.key.clone(), p)).collect::<HashMap<_, _>>();
            for item in items {
                let key = item.key(&session);
                let chat = &cx.model.chats[&session];
                let stamp = item.stamp(chat);
                let sender = item.sender(chat);
                if let Some(mut old) = previous.remove(&key).filter(|p| p.stamp == stamp && self.measured.is_some_and(|(_, w, scale)| (w, scale) == (signature.1, signature.2))) {
                    old.item = item; old.sender = sender; self.placed.push(old); continue;
                }
                let mut temporary;
                let owner = if let Some(owner) = owners.get_mut(&key) { owner.item = item.clone(); owner }
                    else { temporary = MessageRow::new(item.clone(), &session); &mut temporary };
                let height = owner.measure(bubble_width, &session, cx);
                let text_keys = owner.text_keys(&session);
                let overflow = text_keys.iter().filter_map(|k| cx.services.renderer.messages.get(k))
                    .map(|m| m.view.width - (bubble_width - 28. * s - if matches!(item, ItemId::Tool(_)) { 16. * s } else { 0. })).fold(0., f32::max);
                self.placed.push(Placed { key, item, stamp, top: 0., height, sender, text_keys, overflow });
            }
            let keys = self.placed.iter().flat_map(|p| p.text_keys.iter().cloned()).collect();
            cx.services.renderer.retain_messages(&keys);
            self.measured = Some(signature);
        }
        cx.services.renderer.order_messages(self.placed.iter().flat_map(|p| p.text_keys.iter().cloned()));
        let mut y = quick_top + quick_h;
        for index in 0..self.placed.len() {
            let joins = self.placed.get(index + 1).is_some_and(|next| next.sender == self.placed[index].sender);
            let p = &mut self.placed[index]; p.top = y;
            y += p.height + if joins { 0. } else { 12. * s };
        }
        self.horizontal.max = self.placed.iter().map(|p| p.overflow).fold(0., f32::max);
        self.horizontal.value = self.horizontal.value.clamp(0., self.horizontal.max);
        self.scroll.max = (y - viewport.height).max(0.);
        if y < viewport.height { for p in &mut self.placed { p.top += viewport.height - y; } }
        self.expansion_positions.clear();
        for p in &self.placed { if let Some(row) = owners.get(&p.key) { self.expansion_positions.extend(row.expansions(p.top)); } }
        let old_scroll = self.scroll.value;
        let position = &cx.model.chats[&session].local.position;
        let can_remember = !self.placed.is_empty() && cx.model.chats[&session].feed.synchronized
            && (position.follow || position.key.as_ref().is_none_or(|key| self.placed.iter().any(|p| &p.key == key)) || cx.model.chats[&session].feed.before.is_none());
        if quick_models { self.scroll.value = self.scroll.value.clamp(0., self.scroll.max); }
        else if position.follow { self.scroll.value = self.scroll.max; }
        else if let Some(p) = position.key.as_ref().and_then(|key| self.placed.iter().find(|p| &p.key == key)) {
            self.scroll.value = (p.top + position.offset * s).clamp(0., self.scroll.max);
        } else { self.scroll.value = self.scroll.value.clamp(0., self.scroll.max); }
        if let Some((key, screen_y)) = &self.expansion_pin && let Some(top) = self.expansion_positions.get(key) {
            self.scroll.value = (top - screen_y).clamp(0., self.scroll.max);
        }
        let placements = std::mem::take(&mut self.placed);
        let locating_download = self.locate_download(&placements, viewport, cx);
        self.placed = placements;
        self.scroll.shift_wheel(self.scroll.value - old_scroll);
        let top = self.scroll.value - 2. * viewport.height;
        let bottom = self.scroll.value + 3. * viewport.height;
        self.placed_session = Some(session.clone());
        if can_remember || locating_download { self.remember_scroll(cx); }
        self.history_near_edge(&session, self.download.is_some() || self.scroll.value <= 2. * viewport.height, cx);
        if quick_models { self.models.visit_perframe(&mut Frame { layer: frame.layer, bounds: Rect::new(x, viewport.y + quick_top - self.scroll.value, width, quick_h), clip: viewport }, cx); }
        for (index, p) in self.placed.iter().enumerate() {
            let active = owners.get(&p.key).is_some_and(|row| cx.ui.capture.is_some_and(|c|
                c.target.scope == row.control.target.scope || row.attachment.as_ref().is_some_and(|a| a.controls.id == c.target.scope)));
            if !active && (p.top + p.height < top || p.top > bottom) { continue; }
            let mut row = if let Some(owner) = owners.remove(&p.key) { owner } else {
                let mut owner = MessageRow::new(p.item.clone(), &session); owner.measure(bubble_width, &session, cx); owner
            };
            row.item = p.item.clone(); row.hide(); row.horizontal = self.horizontal.value;
            let upper = if index > 0 && self.placed[index - 1].sender == p.sender { 0. } else { 12. * s };
            let lower = if self.placed.get(index + 1).is_some_and(|next| next.sender == p.sender) { 0. } else { 12. * s };
            row.corners = [upper, upper, lower, lower];
            if let Some(id) = row.interest(&cx.model.chats[&session]) { self.interests.insert(id); }
            self.expansion_positions.extend(row.expansions(p.top));
            let screen_top = viewport.y + p.top - self.scroll.value;
            if screen_top + p.height >= viewport.y && screen_top <= viewport.y + viewport.height {
                row.visit_perframe(&mut Frame { layer: frame.layer, bounds: Rect::new(x + if p.sender == tau_protocol::EventRole::User { width - bubble_width } else { 0. }, screen_top, bubble_width, p.height), clip: viewport }, cx);
            }
            self.rows.push(row);
        }
        for mut removed in owners.into_values() { removed.detach(cx); }
        // New owners may have touched existing text caches after order was set.
        cx.services.renderer.order_messages(self.placed.iter().flat_map(|p| p.text_keys.iter().cloned()));
        // Tick/reflow can move text under a stationary held pointer. Resolve
        // against the new scenes, not only when the host sends pointer motion.
        if self.selecting
            && let Some(capture) = cx.ui.capture.filter(|c| c.target == self.selection)
            && let Some(caret) = cx.services.renderer.nearest_text(capture.point)
        {
            cx.ui.dirty |= cx.services.renderer.extend_selection(caret);
        }
        self.scroll.paint(frame.layer, cx);
        if let Some(auto) = &self.autoscroll {
            paint_autoscroll(frame.layer, auto.anchor, s, cx);
        }
    }
}

impl Transcript {
    fn route(&mut self, event: &Event<'_>, cx: &mut Context<'_>) -> bool {
        if matches!(event, Event::Cancel) {
            self.cancel();
            return false;
        }
        if self.binding.as_ref().is_none_or(|(identity, lineage, session)| {
            identity != &cx.model.identity
                || lineage != &cx.model.account.source_lineage
                || session != &cx.model.account.selected
        }) {
            return false;
        }
        let capture = cx.ui.capture;
        let old_scroll = self.scroll.value;
        if let Event::Hover(point) = *event {
            if let (Some(auto), Some(point)) = (&mut self.autoscroll, point) {
                auto.pointer = point;
                cx.ui.dirty = true;
            }
        }
        if let Event::Middle { pressed, point } = *event {
            if pressed {
                if self.autoscroll.take().is_some() {
                    cx.ui.dirty = true;
                    return true;
                }
                if !contains(self.scroll.rect, point) || self.scroll.max <= 0. {
                    return false;
                }
                self.cancel();
                cx.ui.capture = None;
                self.autoscroll = Some(Autoscroll { anchor: point, pointer: point, pressed: Some(Instant::now()) });
            } else if let Some(auto) = &mut self.autoscroll
                && auto.pressed.take().is_some_and(|t| t.elapsed().as_millis() >= 220)
            {
                self.autoscroll = None;
            }
            self.remember_scroll(cx);
            cx.ui.dirty = true;
            return true;
        }
        if matches!(event, Event::Key { key: "c" | "C" | "x" | "X", ctrl: true, .. }) && cx.ui.focus.is_none() {
            if let Some(text) = cx.services.renderer.selected_text() {
                cx.services.platform.push(PlatformAction::Copy(text));
            }
            return true;
        }
        if let Event::Down { point, .. } = *event
            && contains(self.scroll.rect, point)
        {
            if self.autoscroll.take().is_some() {
                cx.ui.capture = None;
                cx.ui.dirty = true;
                return true;
            }
            self.expansion_pin = None;
            self.history_attempt = None;
        }
        if self.scroll.bar_event(event, cx) {
            self.download = None;
            self.remember_scroll(cx);
            return true;
        }
        let mut child = self.models.dispatch(event, cx);
        let mut toggle = None;
        for row in self.rows.iter_mut().rev() {
            if !child || event.broadcast() {
                child |= row.dispatch(event, cx);
            }
            if let Some(choice) = row.toggle.take() {
                toggle = Some(choice);
            }
        }
        if let Some((key, open)) = toggle {
            self.toggle(key, open, cx);
        }
        match *event {
            Event::Down { pointer, point, touch: false } if !child && contains(self.scroll.rect, point) => {
                if let Some(caret) = cx.services.renderer.nearest_text(point) {
                    cx.services.renderer.begin_selection(caret);
                    self.selecting = true;
                    cx.ui.focus = None;
                    cx.ui.capture = Some(super::Capture {
                        target: self.selection,
                        pointer,
                        start: point,
                        point,
                        touch: false,
                        dragged: false,
                        claimed: true,
                        started: Instant::now(),
                    });
                    child = true;
                }
            }
            Event::Move { pointer, point }
                if self.selecting && capture.is_some_and(|c| c.target == self.selection && c.pointer == pointer) =>
            {
                if let Some(caret) = cx.services.renderer.nearest_text(point) {
                    cx.services.renderer.extend_selection(caret);
                }
                let c = cx.ui.capture.as_mut().unwrap();
                c.point = point;
                c.dragged |= (point.x - c.start.x).abs() + (point.y - c.start.y).abs() > 4. * cx.ui.scale;
                cx.ui.dirty = true;
                return true;
            }
            Event::Up { pointer, point }
                if capture.is_some_and(|c| c.target == self.selection && c.pointer == pointer) =>
            {
                let c = cx.ui.capture.take().unwrap();
                self.selecting = false;
                if !c.dragged
                    && let Some(link) = cx.services.renderer.hit_link(point)
                {
                    cx.ui.requests.push_back(Request::Open(super::DialogSpec::Operation(super::Operation::Link(link))));
                }
                child = true;
            }
            Event::Wheel { amount: _, horizontal, point } if !child && contains(self.scroll.rect, point) => {
                self.autoscroll = None;
                self.expansion_pin = None;
                self.history_attempt = None;
                self.download = None;
                let handled = if horizontal {
                    self.horizontal.event(event, false, cx)
                } else {
                    self.scroll.event(event, false, cx)
                };
                self.remember_scroll(cx);
                return handled;
            }
            Event::Tick(dt) => {
                if let Some(auto) = &self.autoscroll {
                    self.scroll.set(self.scroll.value + auto.speed(cx.ui.scale) * dt.min(0.05));
                }
                if self.selecting
                    && let Some(c) = capture.filter(|c| c.target == self.selection && c.dragged)
                {
                    let margin = 16. * cx.ui.scale;
                    let r = self.scroll.rect;
                    let y = c.point.y;
                    let speed = if y < r.y + margin {
                        (y - r.y - margin) * 20.
                    } else if y > r.y + r.height - margin {
                        (y - r.y - r.height + margin) * 20.
                    } else {
                        0.
                    };
                    self.scroll
                        .set(self.scroll.value + speed.clamp(-1800. * cx.ui.scale, 1800. * cx.ui.scale) * dt.min(0.05));
                }
            }
            Event::Context(point) if !child && contains(self.scroll.rect, point) => {
                if let Some(session) = cx.model.account.selected.clone() {
                    cx.chat_menu(&session, point);
                }
                return true;
            }
            _ => {}
        }
        let horizontal = matches!(event,Event::Move {point,..} if capture.is_some_and(|c|(point.x-c.start.x).abs()>1.5*(point.y-c.start.y).abs()));
        let handled = if horizontal {
            let h = self.horizontal.event(event, child, cx);
            self.scroll.event(event, h, cx)
        } else {
            let v = self.scroll.event(event, child, cx);
            if !matches!(event, Event::Move { .. }) { self.horizontal.event(event, v, cx) } else { v }
        };
        if let Event::Up { pointer, .. } = *event
            && capture.is_some_and(|c| c.pointer == pointer)
        {
            self.remember_scroll(cx);
            if let Some(session) = cx.model.account.selected.clone() {
                let result = cx.model.save_chat(&session);
                cx.report(result);
            }
        }
        if self.scroll.value != old_scroll {
            self.download = None;
            self.remember_scroll(cx);
            cx.ui.dirty = true;
        }
        if let Event::Hover(Some(point)) = *event
            && !child
            && contains(self.scroll.rect, point)
        {
            if cx.services.renderer.hit_link(point).is_some() {
                cx.ui.hot = Some((self.selection, super::Cursor::Pointer));
            } else if cx.services.renderer.nearest_text(point).is_some() {
                cx.ui.hot = Some((self.selection, super::Cursor::Text));
            }
        }
        handled
    }
}
fn paint_autoscroll(layer: &mut crate::render::Layer, a: Vec2, s: f32, cx: &mut Context<'_>) {
    layer.above();
    let radius = 13.5 * s;
    layer.rounded_rect(Rect::new(a.x - radius, a.y - radius, radius * 2., radius * 2.), radius, color(0x67d4ff));
    layer.rounded_rect(
        Rect::new(a.x - radius + 2. * s, a.y - radius + 2. * s, (radius - 2. * s) * 2., (radius - 2. * s) * 2.),
        radius - 2. * s,
        color(0x18212b),
    );
    let size = 24. * s;
    cx.services.renderer.icon(
        &cx.services.gpu,
        layer,
        crate::icons::Icon::Autoscroll,
        Rect::new(a.x - size / 2., a.y - size / 2., size, size),
        0x67d4ff,
    );
}
