//! Touch selection uses the same shaped caret geometry and byte/IME fences as
//! mouse editing. No second text layout in the transparent Android IME view.
use super::*;

#[derive(Clone, Copy)]
pub(crate) struct SelectionHandle {
    pub start: bool,
    pub caret: Rect,
    pub knob: Rect,
}
impl SelectionHandle {
    pub fn center(self) -> Vec2 {
        Vec2::new(self.knob.x + self.knob.width / 2., self.knob.y + self.knob.height / 2.)
    }
}
impl Editor {
    pub(crate) fn selection_handles(&self, text: &TextService) -> Vec<SelectionHandle> {
        if !self.visible || self.range().is_empty() || self.composing() { return vec![]; }
        let Some(view) = self.view else { return vec![]; };
        let Some(cached) = &self.layout else { return vec![]; };
        let layout = text.measure(cached.block);
        let origin = self.origin(layout, view);
        let inner = view.inner();
        let range = self.range();
        [(true, range.start), (false, range.end)].into_iter().filter_map(|(start, byte)| {
            let caret = layout.caret_rect(layout.caret_after_edit(self.display_byte(byte, view.secret)));
            let caret = Rect::new(origin.x + caret.x_em * view.size, origin.y + caret.y_em * view.size,
                1.5, caret.height_em * view.size);
            // Offscreen endpoints have no fake handle on the viewport edge.
            if caret.x < inner.x - 1. || caret.x > inner.x + inner.width + 1.
                || caret.y + caret.height <= inner.y || caret.y >= inner.y + inner.height { return None; }
            let diameter = (view.size * 0.9).min(view.rect.height);
            let x = (caret.x - if start { diameter } else { 0. })
                .clamp(view.rect.x, (view.rect.x + view.rect.width - diameter).max(view.rect.x));
            let y = (caret.y + caret.height).min(view.rect.y + view.rect.height - diameter).max(view.rect.y);
            Some(SelectionHandle { start, caret, knob: Rect::new(x, y, diameter, diameter) })
        }).collect()
    }
    pub(crate) fn draw_selection_handles(&self, text: &TextService, layer: &mut Layer) {
        for handle in self.selection_handles(text) {
            layer.rounded_rect(handle.knob, handle.knob.width / 2., color(0x67d4ff));
            let top = handle.knob.y.min(handle.caret.y + handle.caret.height);
            layer.rect(Rect::new(handle.caret.x, top, 1.5,
                (handle.knob.y + handle.knob.height / 2. - top).max(1.)), color(0x67d4ff));
        }
    }
    /// Claim the closest handle in a finger-sized target. Keep the untouched
    /// endpoint fixed even when the dragged endpoint crosses it or shrinks back.
    /// The offset keeps a press anywhere in the knob from jumping the caret.
    pub(crate) fn grab_selection_handle(&mut self, text: &TextService, point: Vec2) -> Option<Vec2> {
        let radius = self.view?.size * 1.5;
        let handle = self.selection_handles(text).into_iter().filter_map(|h| {
            let center = h.center();
            let distance = (center.x - point.x).powi(2) + (center.y - point.y).powi(2);
            (distance <= radius * radius).then_some((h, distance))
        }).min_by(|a,b| a.1.total_cmp(&b.1))?.0;
        let range = self.range();
        self.native_changed();
        self.anchor = if handle.start { range.end } else { range.start };
        self.caret.byte_index = if handle.start { range.start } else { range.end };
        self.after_edit = true;
        self.follow_caret = false;
        self.goal = None;
        Some(Vec2::new(point.x - handle.caret.x, point.y - handle.caret.y - handle.caret.height / 2.))
    }
    pub(crate) fn selection_rect(&self, text: &TextService) -> Option<Rect> {
        let caret = self.ime_rect(text)?;
        let mut bounds = caret;
        for handle in self.selection_handles(text) {
            let right = (bounds.x + bounds.width).max(handle.caret.x + handle.caret.width);
            let bottom = (bounds.y + bounds.height).max(handle.caret.y + handle.caret.height);
            bounds.x = bounds.x.min(handle.caret.x);
            bounds.y = bounds.y.min(handle.caret.y);
            bounds.width = right - bounds.x;
            bounds.height = bottom - bounds.y;
        }
        Some(crate::render::intersect(bounds, self.view?.inner()))
    }
}
