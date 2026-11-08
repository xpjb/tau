use super::controls::Control;
use super::{Context, Controller, Event, Frame, Id, Request, Target, UiState, Widget};
use crate::{
    app::notices::NoticePopup,
    icons::Icon,
    render::color,
};
use sanscale::Rect;
use std::time::Instant;

pub(in crate::app) struct NoticeWidget {
    pub popup: NoticePopup,
    pub body: Control,
    pub close: Control,
}
impl NoticeWidget {
    pub fn new() -> Self {
        let id = Id::new();
        Self { popup: NoticePopup::default(), body: Control::new(id, false), close: Control::new(id, true) }
    }
    pub fn hide(&mut self) {
        self.body.rect = None;
        self.close.rect = None;
    }
    pub fn suspend(&mut self, cx: &mut Context<'_>) {
        if self.popup.visible() || self.body.rect.is_some() {
            cx.ui.detach(self.body.target.scope);
            self.popup = NoticePopup::default();
            self.hide();
        }
    }
    fn sync(&mut self, cx: &mut Context<'_>) {
        let changed = self.popup.observe(cx.model.notice.as_ref(), Instant::now());
        if changed {
            cx.ui.detach(self.body.target.scope);
            let id = Id::new();
            self.body = Control::new(id, false);
            self.close = Control::new(id, true);
            cx.ui.dirty = true;
        }
    }
}
impl Widget for NoticeWidget {
    fn update(&mut self, _dt: f32, cx: &mut Context<'_>) {
        self.sync(cx);
    }
    fn owns(&self, target: Target, _model: &Controller, _ui: &UiState) -> bool {
        self.popup.visible() && (self.body.target == target || self.close.target == target)
    }

    fn handle_event(&mut self, event: &Event<'_>, cx: &mut Context<'_>) -> bool {
        self.sync(cx);
        if !self.popup.visible() {
            return false;
        }
        if self.close.handle(event, cx, false) {
            if self.close.take_click() {
                cx.model.notice = None;
                self.hide();
            }
            return true;
        }
        if self.body.handle(event, cx, false) {
            if self.body.take_click() {
                if let Some(target) = cx.model.notice.as_ref().and_then(|n| n.download.clone()) {
                    cx.ui.requests.push_back(Request::Download(target));
                } else {
                    cx.model.notice = None;
                    self.hide();
                }
            }
            return true;
        }
        match *event {
            Event::Context(point) | Event::Wheel { point, .. } | Event::Middle { point, .. } => {
                self.body.contains(point)
            }
            _ => false,
        }
    }
    fn visit_perframe(&mut self, frame: &mut Frame<'_>, cx: &mut Context<'_>) {
        self.sync(cx);
        self.hide();
        if !self.popup.visible() {
            return;
        }
        let Some(notice) = cx.model.notice.as_deref() else {
            return;
        };
        let b = frame.bounds;
        let s = cx.ui.scale;
        let size = 16. * s;
        let max_width = (b.width - 32. * s).max(1.).min(560. * s);
        let style = sanscale::Style {
            chain: cx.services.renderer.faces.prose[0],
            wrap_em: None,
            align: sanscale::Align::Left,
            line_spacing: 1.15,
        };
        let natural = cx
            .services
            .renderer
            .text
            .shape_transient(notice, &style)
            .map(|key| cx.services.renderer.text.measure(key).width_em() * size)
            .unwrap_or(max_width);
        let width = (natural + 72. * s).max(240. * s).min(max_width);
        let text_width = (width - 72. * s).max(1.);
        let text_height = cx.services.renderer.label_height(notice, text_width, size, false);
        let height = (text_height + 24. * s).max(48. * s).min((b.height - 32. * s).max(1.));
        let rect = Rect::new(b.x + (b.width - width) / 2., b.y + 16. * s, width, height);
        self.body.rect = Some(rect);
        self.body.clip = frame.clip;
        frame.layer.above();
        frame.layer.rounded_rect(rect, 12. * s, color(0x263340));
        self.body.corners = Some([12. * s; 4]);
        self.body.highlight(frame.layer, cx.ui, false);
        cx.services.renderer.clipped_label(
            frame.layer,
            notice,
            Rect::new(rect.x + 16. * s, rect.y + ((height - text_height) / 2.).max(12. * s), text_width, text_height),
            size,
            color(0xe5eaf0),
            false,
            rect,
        );
        let close = Rect::new(rect.x + width - 44. * s, rect.y + (height - 40. * s) / 2., 40. * s, 40. * s);
        self.close.rect = Some(close);
        self.close.clip = frame.clip;
        self.close.highlight(frame.layer, cx.ui, false);
        cx.services.renderer.icon(
            &cx.services.gpu,
            frame.layer,
            Icon::Close,
            Rect::new(close.x + 10. * s, close.y + 10. * s, 20. * s, 20. * s),
            0xe5eaf0,
        );
    }
}
