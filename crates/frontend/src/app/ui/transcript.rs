use super::{Context, Controller, Event, Frame, Id, Request, Target, UiState, Widget};
use super::{composer::QuickModels, message_row::{MessageRow, ItemId, is_detail}, scroll::ScrollState};
use crate::{
    app::{Autoscroll, PlatformAction, attachments},
    store::Position,
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
    sender: tau_net::EventRole,
    text_keys: Vec<String>,
    overflow: f32,
}
// Live policy belongs to this source-bound transcript. LocalChat.position is a
// restart/checkpoint preference, not a second controller read back on every paint.
enum Reading {
    Position(Position),
    Disclosure(Position),
    Download(String),
}
pub(in crate::app) struct Transcript {
    pub rows: Vec<MessageRow>,
    measured: Option<(u64, u32, u32)>,
    pub scroll: ScrollState,
    pub horizontal: ScrollState,
    pub models: QuickModels,
    pub placed: Vec<Placed>,
    reading: Reading,
    pub history_attempt: Option<(String, String, u64, u64)>,
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
            reading: Reading::Position(Position { follow: true, ..Position::default() }),
            history_attempt: None,
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
            let result = self.save(cx);
            cx.report(result);
            cx.ui.detach(self.selection.scope);
            for row in &mut self.rows {
                row.detach(cx);
            }
            cx.services.renderer.selection = None;
            cx.services.renderer.retain_messages(&HashSet::new());
            *self = Self::new();
            if let Some(chat) = cx.model.selected() {
                self.reading = Reading::Position(chat.local.position.clone());
            }
            self.binding = Some(next);
            cx.ui.dirty = true;
        }
    }
    pub fn cancel(&mut self) {
        self.scroll.stop();
        self.horizontal.stop();
        self.selecting = false;
        self.autoscroll = None;
        for row in &mut self.rows {
            row.cancel();
        }
    }
    pub fn cancel_autoscroll(&mut self, cx: &mut Context<'_>) -> bool {
        let active = self.autoscroll.take().is_some();
        if active { self.remember_scroll(cx); cx.ui.dirty = true; }
        active
    }
    fn anchor<'a>(&self, key: &str, rows: impl IntoIterator<Item = &'a MessageRow>) -> Option<f32> {
        self.placed.iter().find(|p| p.key == key).map(|p| p.top).or_else(|| {
            rows.into_iter().find_map(|row| {
                let offset = row.anchor(key)?;
                self.placed.iter().find(|p| p.key == row.key).map(|p| p.top + offset)
            })
        })
    }
    fn position(&self, follow: bool) -> Option<Position> {
        let scale = f32::from_bits(self.measured?.2);
        let anchor = self.placed.iter().find(|p| p.top + p.height >= self.scroll.value)?;
        Some(Position { key: Some(anchor.key.clone()), offset: (self.scroll.value - anchor.top) / scale, follow })
    }
    /// A pending jump or unloaded anchor never checkpoints an interim page.
    /// Keep a row anchor's intended offset through temporary layout clamps.
    /// Only nested disclosure anchors need a row-identity checkpoint.
    fn checkpoint(&self, cx: &mut Context<'_>) -> Option<String> {
        let (identity, lineage, Some(session)) = self.binding.as_ref()? else { return None; };
        if identity != &cx.model.identity || lineage != &cx.model.account.source_lineage { return None; }
        let position = match &self.reading {
            Reading::Position(p) if p.follow || p.key.as_ref().is_some_and(|key| self.placed.iter().any(|row| &row.key == key)) => Some(p.clone()),
            Reading::Position(p) | Reading::Disclosure(p)
                if p.key.as_ref().is_none_or(|key| self.anchor(key, &self.rows).is_some()) => self.position(false),
            _ => None,
        };
        let chat = cx.model.chats.get_mut(session)?;
        if let Some(position) = position { chat.local.position = position; }
        Some(session.clone())
    }
    pub fn save(&self, cx: &mut Context<'_>) -> anyhow::Result<()> {
        if let Some(session) = self.checkpoint(cx) { cx.model.save_chat(&session)?; }
        Ok(())
    }
    /// Called for actual scrolling, not as a paint-time save/restore round trip.
    pub fn remember_scroll(&mut self, cx: &mut Context<'_>) {
        let follow = !self.scroll.moving() && self.autoscroll.is_none()
            && self.scroll.max - self.scroll.value < cx.ui.scale;
        self.reading = Reading::Position(self.position(follow).unwrap_or_default());
        self.checkpoint(cx);
    }
    pub fn tail(&mut self, cx: &mut Context<'_>) {
        self.cancel();
        self.reading = Reading::Position(Position { follow: true, ..Position::default() });
        self.scroll.value = self.scroll.max;
        self.checkpoint(cx);
        cx.ui.dirty = true;
    }
    pub fn jump_to_download(&mut self, entry: String, cx: &mut Context<'_>) {
        self.cancel();
        self.reading = Reading::Download(entry);
        self.horizontal.value = 0.;
        self.history_attempt = None;
        cx.ui.dirty = true;
    }
    pub fn cancel_download_jump(&mut self, cx: &mut Context<'_>) {
        if self.locating_download() {
            self.reading = Reading::Position(cx.model.selected().map(|c| c.local.position.clone()).unwrap_or_default());
        }
    }
    pub fn locating_download(&self) -> bool { matches!(self.reading, Reading::Download(_)) }
    pub fn toggle(&mut self, key: String, open: bool, cx: &mut Context<'_>) {
        self.scroll.stop();
        if let Some(top) = self.anchor(&key, &self.rows) {
            let scale = f32::from_bits(self.measured.unwrap().2);
            self.reading = Reading::Disclosure(Position {
                key: Some(key.clone()), offset: (self.scroll.value - top) / scale, follow: false,
            });
        }
        if let Some(chat) = cx.model.account.selected.as_ref().and_then(|s| cx.model.chats.get_mut(s)) {
            if key.starts_with("details:") { chat.local.details_default = open; }
            chat.local.expansion.insert(key, open);
            let result = self.save(cx);
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
    fn place_reading(&mut self, session: &str, quick_models: bool, owners: &HashMap<String, MessageRow>, cx: &mut Context<'_>) {
        let old = self.scroll.value;
        let chat = &cx.model.chats[session];
        // A removed/renamed control is not unloaded history. Resume the row
        // checkpoint instead of seeking older pages for a dead disclosure key.
        if let Reading::Disclosure(p) = &self.reading
            && p.key.as_ref().is_none_or(|key| self.anchor(key, owners.values()).is_none()) {
            self.reading = Reading::Position(chat.local.position.clone());
        }
        match &self.reading {
            Reading::Download(entry) => {
                if let Some(p) = self.placed.iter().find(|p| p.item.root(chat).and_then(|id| chat.feed.event(id))
                    .is_some_and(|e| e.entry_id == *entry && e.attachment.is_some())) {
                    // Center the controls, not the long message/image above them.
                    let panel = attachments::control_panel(Rect::new(0., p.top, self.scroll.rect.width, p.height), cx.ui.scale);
                    self.scroll.set(panel.y + panel.height / 2. - self.scroll.rect.height / 2.);
                    self.reading = Reading::Position(Position {
                        key: Some(p.key.clone()), offset: (self.scroll.value - p.top) / cx.ui.scale, follow: false,
                    });
                    self.checkpoint(cx);
                } else {
                    self.scroll.value = 0.;
                    if cx.model.account.missing_chats.contains(session)
                        || chat.feed.synchronized && !chat.feed.loading && chat.feed.before.is_none() {
                        self.reading = Reading::Position(self.position(false).unwrap_or_default());
                        cx.model.notice = Some("The download widget is no longer available in this chat.".into());
                    }
                }
            }
            Reading::Position(p) | Reading::Disclosure(p) => {
                if p.follow && !quick_models { self.scroll.value = self.scroll.max; }
                else if let Some(top) = p.key.as_ref().and_then(|key| self.anchor(key, owners.values())) {
                    self.scroll.set(top + p.offset * cx.ui.scale);
                } else {
                    self.scroll.set(if p.key.is_some() && chat.feed.before.is_some() { 0. } else { self.scroll.value });
                    // Keep an unresolved preference across partial/empty windows.
                    // A complete nonempty layout can retire a deleted anchor.
                    if chat.feed.synchronized && !chat.feed.loading && chat.feed.before.is_none()
                        && let Some(position) = self.position(p.follow) {
                        self.reading = Reading::Position(position);
                    }
                }
            }
        }
        self.scroll.shift_wheel(self.scroll.value - old);
    }

}
impl Widget for Transcript {
    fn update(&mut self, dt: f32, cx: &mut Context<'_>) {
        let capture = cx.ui.capture;
        let old_scroll = self.scroll.value;
        let was_scrolling = self.scroll.moving();
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
            self.scroll.set(self.scroll.value + speed.clamp(-1800. * cx.ui.scale, 1800. * cx.ui.scale) * dt.min(0.05));
        }
        self.scroll.update(dt, cx);
        self.horizontal.update(dt, cx);
        for row in &mut self.rows {
            row.update(dt, cx);
        }
        if self.scroll.value != old_scroll || was_scrolling && !self.scroll.moving() {
            self.remember_scroll(cx);
            cx.ui.dirty = true;
        }
    }
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
                if open { for e in group { items.push(if e.kind == tau_net::EventKind::Thinking && e.role != tau_net::EventRole::Tool {
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
        self.place_reading(&session, quick_models, &owners, cx);
        let top = self.scroll.value - 2. * viewport.height;
        let bottom = self.scroll.value + 3. * viewport.height;
        self.history_near_edge(&session, self.locating_download() || self.scroll.value <= 2. * viewport.height, cx);
        if quick_models { self.models.visit_perframe(&mut Frame { layer: frame.layer, bounds: Rect::new(x, viewport.y + quick_top - self.scroll.value, width, quick_h), clip: viewport }, cx); }
        for (index, p) in self.placed.iter().enumerate() {
            // A pinned disclosure survives a temporarily tiny viewport, like capture.
            let active = owners.get(&p.key).is_some_and(|row| {
                cx.ui.capture.is_some_and(|c| c.target.scope == row.control.target.scope
                    || row.attachment.as_ref().is_some_and(|a| a.controls.id == c.target.scope))
                    || matches!(&self.reading, Reading::Disclosure(position)
                        if position.key.as_ref().is_some_and(|key| row.anchor(key).is_some()))
            });
            if !active && (p.top + p.height < top || p.top > bottom) { continue; }
            let mut row = if let Some(owner) = owners.remove(&p.key) { owner } else {
                let mut owner = MessageRow::new(p.item.clone(), &session); owner.measure(bubble_width, &session, cx); owner
            };
            row.item = p.item.clone(); row.hide(); row.horizontal = self.horizontal.value;
            let upper = if index > 0 && self.placed[index - 1].sender == p.sender { 0. } else { 12. * s };
            let lower = if self.placed.get(index + 1).is_some_and(|next| next.sender == p.sender) { 0. } else { 12. * s };
            row.corners = [upper, upper, lower, lower];
            if let Some(id) = row.interest(&cx.model.chats[&session]) { self.interests.insert(id); }
            let screen_top = viewport.y + p.top - self.scroll.value;
            if screen_top + p.height >= viewport.y && screen_top <= viewport.y + viewport.height {
                row.visit_perframe(&mut Frame { layer: frame.layer, bounds: Rect::new(x + if p.sender == tau_net::EventRole::User { width - bubble_width } else { 0. }, screen_top, bubble_width, p.height), clip: viewport }, cx);
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
                if self.cancel_autoscroll(cx) { return true; }
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
            if self.cancel_autoscroll(cx) {
                cx.ui.capture = None;
                return true;
            }
            self.history_attempt = None;
        }
        if self.scroll.bar_event(event, cx) {
            if matches!(event, Event::Down { .. }) { self.remember_scroll(cx); }
            return true;
        }
        let mut child = self.models.dispatch(event, cx);
        let mut toggle = None;
        for row in self.rows.iter_mut().rev() {
            if !child {
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
            Event::Wheel { horizontal, point, .. } if !child && contains(self.scroll.rect, point) => {
                self.autoscroll = None;
                self.history_attempt = None;
                let handled = if horizontal {
                    self.horizontal.event(event, false, cx)
                } else {
                    self.scroll.event(event, false, cx)
                };
                self.remember_scroll(cx);
                return handled;
            }

            Event::Context(point) if !child && contains(self.scroll.rect, point) => {
                if let Some(session) = cx.model.account.selected.clone() {
                    cx.chat_menu(&session, point);
                }
                return true;
            }
            _ => {}
        }
        let handled = ScrollState::axes_event(&mut self.scroll, &mut self.horizontal, event, child, cx);
        if let Event::Up { pointer, .. } = *event
            && capture.is_some_and(|c| c.pointer == pointer)
        {
            if capture.is_some_and(|c| c.target == self.scroll.target || c.target == self.selection && c.dragged) {
                self.remember_scroll(cx);
            }
            let result = self.save(cx);
            cx.report(result);
        }
        if self.scroll.value != old_scroll {
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
