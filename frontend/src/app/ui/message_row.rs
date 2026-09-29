use super::{Context, Controller, Event, Frame, Id, Menu, MenuChoice, Request, Target, UiState, Widget};
use super::{attachments::AttachmentCard, controls::Control};
use crate::{
    app::{DetailLine, Row},
    icons::Icon,
    render::color,
};
use sanscale::{Rect, Vec2};
use std::collections::HashMap;
pub(in crate::app) struct Part {
    pub line: DetailLine,
    pub control: Control,
}
pub(in crate::app) struct MessageRow {
    pub row: Row,
    pub control: Control,
    pub parts: Vec<Part>,
    pub attachment: Option<AttachmentCard>,
    pub toggle: Option<(String, bool)>,
    pub layout: Vec<(f32, f32)>,
    pub corners: [f32; 4],
    pub horizontal: f32,
}
impl MessageRow {
    pub fn new(row: Row, cx: &mut Context<'_>) -> Self {
        let id = Id::new();
        let mut out = Self {
            row: row.clone(),
            control: Control::new(id, false),
            parts: vec![],
            attachment: None,
            toggle: None,
            layout: vec![],
            corners: [0.; 4],
            horizontal: 0.,
        };
        out.update(row, cx);
        out
    }
    pub fn update(&mut self, row: Row, cx: &mut Context<'_>) {
        let session = cx.model.account.selected.as_deref().unwrap_or("").to_owned();
        let mut old = std::mem::take(&mut self.parts)
            .into_iter()
            .map(|part| (part.line.key.clone(), part))
            .collect::<HashMap<_, _>>();
        for line in &row.details {
            let mut part = old.remove(&line.key).unwrap_or_else(|| Part {
                line: line.clone(),
                control: Control::new(self.control.target.scope, false),
            });
            part.line = line.clone();
            part.control.rect = None;
            self.parts.push(part);
        }
        for part in old.into_values() {
            cx.ui.detach_target(part.control.target);
        }
        if let Some((entry, file)) = &row.attachment {
            if self.attachment.as_ref().is_none_or(|c| c.target.entry != *entry) {
                if let Some(old) = self.attachment.take() {
                    cx.ui.detach(old.controls.id);
                }
                self.attachment = Some(AttachmentCard::new(&session, entry, file, "chat", cx));
            }
            self.attachment.as_mut().unwrap().file = file.clone();
        } else if let Some(old) = self.attachment.take() {
            cx.ui.detach(old.controls.id);
        }
        self.row = row;
        self.hide();
    }
    pub fn hide(&mut self) {
        self.control.rect = None;
        for part in &mut self.parts {
            part.control.rect = None;
        }
        if let Some(card) = &mut self.attachment {
            card.hide();
        }
    }
    pub fn detach(&mut self, cx: &mut Context<'_>) {
        cx.ui.detach(self.control.target.scope);
        if let Some(card) = &self.attachment {
            cx.ui.detach(card.controls.id);
        }
    }
    fn menu(&self, point: Vec2, cx: &mut Context<'_>) {
        let mut options = vec![];
        if cx.services.renderer.selected_text().is_some_and(|s| !s.is_empty()) {
            options.push(("Copy selection".into(), MenuChoice::CopySelection));
        }
        options.extend(self.row.actions.clone());
        if options.is_empty() {
            if let Some(session) = cx.model.account.selected.clone() {
                cx.chat_menu(&session, point);
            }
        } else {
            let menu = Menu::new(point, Some(self.row.key.clone()), None, options, cx);
            cx.ui.requests.push_back(Request::Menu(Box::new(menu)));
        }
    }
}
impl Widget for MessageRow {
    fn owns(&self, target: Target, model: &Controller, ui: &UiState) -> bool {
        let local = self.control.target == target
            || self.parts.iter().any(|p| p.control.target == target && p.control.rect.is_some());
        (local
            && self
                .row
                .block
                .as_ref()
                .is_none_or(|id| model.selected().is_some_and(|chat| chat.feed.events.values().any(|e| &e.id == id))))
            || self.attachment.as_ref().is_some_and(|c| c.owns(target, model, ui))
    }

    fn handle_event(&mut self, event: &Event<'_>, cx: &mut Context<'_>) -> bool {
        if let Some(card) = &mut self.attachment
            && card.dispatch(event, cx) && !event.broadcast()
        {
            return true;
        }
        for part in self.parts.iter_mut().rev() {
            if part.line.toggle.is_some() || !part.line.source.is_empty() && matches!(event, Event::Hover(_)) {
                let text = part.line.toggle.is_none()
                    && !cx.ui.hover.is_some_and(|p| cx.services.renderer.hit_link(p).is_some());
                let handled = part.control.handle(event, cx, text);
                if part.control.take_click() {
                    self.toggle = Some((part.line.key.clone(), !part.line.toggle.unwrap()));
                }
                if handled {
                    return true;
                }
            }
        }
        if let Event::Context(point) = *event
            && self.control.contains(point)
        {
            self.menu(point, cx);
            return true;
        }
        if matches!(event, Event::Tick(_) | Event::Up { .. })
            && cx.ui.capture.is_some_and(|c| {
                c.target == self.control.target && c.touch && !c.dragged && c.started.elapsed().as_millis() >= 450
            })
        {
            let p = cx.ui.capture.take().unwrap().point;
            self.menu(p, cx);
            return true;
        }
        if matches!(event, Event::Hover(_)) {
            let text =
                !self.row.source.is_empty() && !cx.ui.hover.is_some_and(|p| cx.services.renderer.hit_link(p).is_some());
            return self.control.handle(event, cx, text);
        }
        if matches!(event, Event::Down { touch: false, .. }) {
            return false;
        }
        let handled = self.control.handle(event, cx, false);
        if self.control.take_click()
            && let Event::Up { point, .. } = *event
            && let Some(link) = cx.services.renderer.hit_link(point)
        {
            cx.ui.requests.push_back(Request::Open(super::DialogSpec::Operation(super::Operation::Link(link))));
        }
        handled
    }
    fn visit_perframe(&mut self, frame: &mut Frame<'_>, cx: &mut Context<'_>) {
        let s = cx.ui.scale;
        let row = &self.row;
        let rect = frame.bounds;
        let viewport = frame.clip;
        let x = rect.x;
        let top = rect.y;
        let text_width = rect.width - 28. * s;
        let corners = self.corners;
        let joined_above = corners[0] == 0.;
        let session = cx.model.account.selected.clone().unwrap();
        let layer = &mut *frame.layer;
        layer.clipped_corners(rect, corners, color(if row.user { 0x164e63 } else { 0x18212b }), viewport);
        if joined_above {
            layer.clipped_rect(
                Rect::new(x + 14. * s, top, text_width, s),
                color(if row.user { 0x286176 } else { 0x2a3541 }),
                viewport,
            );
        }
        self.control.rect = Some(rect);
        self.control.clip = viewport;
        self.control.corners = Some(corners);
        let pinned = cx.ui.menu_section.as_deref() == Some(row.key.as_str());
        self.control.highlight(layer, cx.ui, pinned);
        cx.services.renderer.clipped_label(
            layer,
            &row.timestamp,
            Rect::new(x + 14. * s, top + 8. * s, text_width, 16. * s),
            11. * s,
            color(if row.user { 0xa7bdc9 } else { 0x82909f }),
            false,
            viewport,
        );
        if !self.layout.is_empty() {
            let layout = &self.layout;
            for (part, (offset, h)) in self.parts.iter_mut().zip(layout) {
                let line = &part.line;
                let lx = x + 14. * s + line.indent * s;
                let line_rect = Rect::new(lx - 4. * s, top + offset, text_width - line.indent * s + 8. * s, *h);
                let inner_corners = [4. * s; 4];
                part.control.rect = Some(line_rect);
                part.control.clip = viewport;
                part.control.corners = Some(inner_corners);
                if line.tool {
                    layer.clipped_rect(line_rect, color(0x111922), viewport);
                }
                if !line.source.is_empty() {
                    cx.services.renderer.message(
                        layer,
                        &format!("{session}/{}", line.key),
                        Vec2::new(lx, top + offset),
                        Rect::new(lx, viewport.y, text_width - line.indent * s, viewport.height),
                        self.horizontal,
                    );
                } else {
                    let label_x = if let Some(open) = line.toggle {
                        cx.services.renderer.clipped_icon(
                            &cx.services.gpu,
                            layer,
                            if open { Icon::ChevronDown } else { Icon::ChevronRight },
                            Rect::new(lx, top + offset + 6. * s, 14. * s, 14. * s),
                            if line.error { 0xffb4ab } else { 0xb7c2ce },
                            viewport,
                        );
                        lx + 18. * s
                    } else {
                        lx
                    };

                    cx.services.renderer.clipped_label(
                        layer,
                        &line.label,
                        Rect::new(
                            label_x,
                            top + offset + 5. * s,
                            text_width - line.indent * s - (label_x - lx),
                            20. * s,
                        ),
                        if line.toggle.is_some() { 12. * s } else { 11. * s },
                        color(if line.error { 0xffb4ab } else { 0xb7c2ce }),
                        false,
                        viewport,
                    );
                }
                part.control.highlight(layer, cx.ui, false);
            }
            return;
        }
        let label_rect = Rect::new(x + 14. * s, top + 28. * s, text_width, 20. * s);
        if row.header {
            cx.services.renderer.clipped_label(
                layer,
                &row.title,
                label_rect,
                12. * s,
                color(if row.error {
                    0xffb4ab
                } else if row.user {
                    0x67d4ff
                } else {
                    0xa5b4fc
                }),
                true,
                viewport,
            );
        }
        let text_top = top + if row.header { 50. * s } else { 28. * s };
        if !row.source.is_empty() {
            cx.services.renderer.message(
                layer,
                &row.key,
                Vec2::new(x + 14. * s, text_top),
                Rect::new(x + 14. * s, viewport.y, text_width, viewport.height),
                self.horizontal,
            );
        }
        if let Some(card) = &mut self.attachment {
            card.visit_perframe(&mut Frame { layer, bounds: rect, clip: viewport }, cx);
        }
    }
}
