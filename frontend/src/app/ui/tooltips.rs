use super::{Context, Event, Frame, Widget};
use crate::{
    app::Info,
    render::Layer,
    tooltip::{Content, Tooltip},
};
use sanscale::Rect;
use std::time::Instant;

pub(in crate::app) struct TooltipHost {
    pub usage: Tooltip,
    pub info: Tooltip,
    pub target: Info,
}
impl Default for TooltipHost {
    fn default() -> Self {
        Self { usage: Tooltip::default(), info: Tooltip::default(), target: Info::Connection }
    }
}
impl TooltipHost {
    pub fn hint(&mut self, cx: &mut Context<'_>) {
        if let Some((rect, target)) = &cx.ui.hint {
            if !self.target.same_anchor(target) {
                self.info = Tooltip::default();
            }
            self.target = target.clone();
            self.info.region = *rect;
            self.info.hover(true);
            self.usage.dismiss();
        }
    }
    pub fn pin(&mut self, target: Info, rect: Rect) {
        self.usage.dismiss();
        if !self.target.same_anchor(&target) {
            self.info = Tooltip::default();
        }
        self.target = target;
        self.info.region = rect;
        self.info.pinned = true;
        self.info.suppressed = false;
    }
    pub fn dismiss(&mut self) {
        self.usage.dismiss();
        self.info.dismiss();
    }
    pub(in crate::app) fn usage_frame(&mut self, cx: &mut Context<'_>, layer: &mut Layer, bounds: Rect) {
        if self.usage.progress <= 0. || self.usage.region.width <= 0. {
            return;
        }
        self.usage.frame(&mut cx.services.renderer, layer, "usage", bounds, cx.ui.scale, 320., true);
    }
    pub(in crate::app) fn info_frame(&mut self, cx: &mut Context<'_>, layer: &mut Layer, bounds: Rect) {
        if self.info.progress <= 0. || self.info.region.width <= 0. {
            return;
        }
        self.info.content = match &self.target {
            Info::Attachment(_, title, detail) => {
                let mut content = Content::default();
                content.strong(title, crate::tooltip::INK).line().dim(detail);
                content
            }
            Info::Connection => {
                let mut content = cx.model.health.tooltip(&cx.model.connection, Instant::now());
                if let Some(detail) = &cx.model.transport_error {
                    content.line().dim("Last transport issue: ").push(detail, false, crate::tooltip::WARNING);
                }
                content
            }
            Info::CacheTtl(id) => {
                let Some(session) = cx.model.account.sessions.iter().find(|session| &session.id == id) else {
                    return;
                };
                cx.model.cache_ttl(session).details()
            }
        };
        // Cover the New chat button beneath the connection card, including on phones.
        let width = if self.target == Info::Connection {
            if bounds.width / cx.ui.scale < 760. { bounds.width / cx.ui.scale - 16. } else { 300. }
        } else if matches!(self.target, Info::Attachment(..)) {
            300.
        } else {
            180.
        };
        self.info.frame(&mut cx.services.renderer, layer, "info", bounds, cx.ui.scale, width, false);
    }
}
impl Widget for TooltipHost {
    fn handle_event(&mut self, event: &Event<'_>, cx: &mut Context<'_>) -> bool {
        match *event {
            Event::Tick(_) => {
                cx.ui.dirty |= self.usage.tick() | self.info.tick();
            }
            Event::Back | Event::Key { key: "Escape", .. }
                if self.info.pinned || self.info.progress > 0. || self.usage.pinned || self.usage.progress > 0. =>
            {
                self.dismiss();
                cx.ui.dirty = true;
                return true;
            }
            Event::Down { point, .. } | Event::Context(point) | Event::Middle { point, .. }
                if self.info.contains_card(point) || self.usage.contains_card(point) =>
            {
                return true;
            }
            _ => {}
        }
        false
    }
    fn visit_perframe(&mut self, frame: &mut Frame<'_>, cx: &mut Context<'_>) {
        self.usage_frame(cx, frame.layer, frame.bounds);
        self.info_frame(cx, frame.layer, frame.bounds);
    }
}
