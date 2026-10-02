//! Same-window IME adapter. All input destinations are retained, source-bound fields.
use super::*;
use crate::mobile_input::{Edit, Input};
impl App {
    fn native_editor(&self) -> Option<&Editor> {
        if !self.ui.mobile || self.root.viewer.is_some() || self.root.menu.is_some() {
            return None;
        }
        let edit = self.ui.native.as_ref()?;
        if !self.root.owns(edit.target, &self.controller, &self.ui) { return None; }
        if !edit.matches(edit.token, &self.controller) || self.ui.focus != Some(edit.target) {
            return None;
        }
        let field = self.root.editor_ref(Some(edit.target))?;
        if !field.control.enabled
            || field.control.rect.is_none()
            || field.control.clip.width <= 0.
            || field.control.clip.height <= 0.
        {
            return None;
        }
        (field.editor.native_id() == edit.token).then_some(&field.editor)
    }
    pub fn native_input(&self) -> Option<Input> {
        let mut input = self.native_editor()?.native_input(self.ui.input_request)?;
        let field = self.root.editor_ref(Some(self.ui.native.as_ref()?.target))?;
        let rect = crate::render::intersect(field.control.rect?, field.control.clip);
        if rect.width <= 0. || rect.height <= 0. {
            return None;
        }
        input.rect = [rect.x, rect.y, rect.width, rect.height];
        let selection = crate::render::intersect(field.editor.selection_rect(&self.services.renderer.text).unwrap_or(rect), rect);
        input.selection_rect = [selection.x, selection.y, selection.width, selection.height];
        Some(input)
    }
    pub fn native_edit(&mut self, edit: Edit) {
        if !self.native_editor().is_some_and(|e| e.accepts_native_edit(&edit)) {
            return;
        }
        let target = self.ui.native.as_ref().unwrap().target;
        let changed = self.root.editor(Some(target)).is_some_and(|f| f.editor.native_edit(edit));
        if changed {
            self.edited();
        }
        self.ui.dirty = true;
    }
}

#[cfg(test)]
#[path = "../../tests/unit/app/mobile_input.rs"]
mod tests;
