use super::*;
use chad::{PhysicalRect, Selection, TextInputEvent, TextInputOptions, TextInputState, surrounding_text};
use std::sync::atomic::{AtomicU64, Ordering};

pub(super) fn next_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}
impl Editor {
    pub fn native_id(&self) -> u64 { self.native_id }
    pub fn native_input(&self) -> Option<TextInputState<'_>> {
        if !self.visible { return None; }
        let view = self.view?;
        let (document_start, text, selection) = surrounding_text(
            &self.value, Selection::new(self.anchor, self.caret.byte_index), 4096)?;
        Some(TextInputState {
            id: self.native_id,
            generation: self.native_revision,
            document_start,
            text,
            selection,
            field_rect: PhysicalRect::new(view.rect.x, view.rect.y, view.rect.width, view.rect.height),
            caret_rect: PhysicalRect::new(view.rect.x, view.rect.y, view.rect.width, view.rect.height),
            options: TextInputOptions { multiline: !self.single_line, secure: view.secret },
        })
    }
    pub fn accepts_native_edit(&self, edit: &TextInputEvent) -> bool {
        self.native_id == edit.id && self.native_revision == edit.generation
    }
    /// Native composition is applied to the real value immediately. Selection
    /// and composition-only events never write a draft.
    pub fn native_edit(&mut self, edit: TextInputEvent) -> bool {
        if !self.accepts_native_edit(&edit) { return false; }
        let mut candidate = self.value.clone();
        if let Some(splice) = edit.splice {
            if splice.range.start > splice.range.end
                || splice.range.end > candidate.len()
                || !candidate.is_char_boundary(splice.range.start)
                || !candidate.is_char_boundary(splice.range.end)
            {
                return false;
            }
            candidate.replace_range(splice.range, &splice.replacement);
        }
        if candidate.len() > tau_net::MAX_REQUEST_BYTES
            || edit.selection.anchor > candidate.len()
            || edit.selection.focus > candidate.len()
            || !candidate.is_char_boundary(edit.selection.anchor)
            || !candidate.is_char_boundary(edit.selection.focus)
            || edit.composition.as_ref().is_some_and(|range| {
                range.start > range.end
                    || range.end > candidate.len()
                    || !candidate.is_char_boundary(range.start)
                    || !candidate.is_char_boundary(range.end)
            })
        {
            return false;
        }
        let byte = |offset| normalize(&candidate[..offset], self.single_line).len();
        let anchor = byte(edit.selection.anchor);
        let caret = byte(edit.selection.focus);
        let composition = edit.composition.map(|range| byte(range.start)..byte(range.end));
        let value = normalize(&candidate, self.single_line);
        let changed = self.value != value;
        if !changed && self.anchor == anchor && self.caret.byte_index == caret
            && self.native_composition == composition { return false; }
        self.stop_scrolling();
        if changed {
            self.undo.push(self.snapshot());
            if self.undo.len() > 64 { self.undo.remove(0); }
            self.redo.clear();
            self.value = value;
            self.layout = None;
        }
        self.caret = Caret { byte_index: caret, line_index: 0 };
        self.anchor = anchor;
        self.composition = None;
        self.native_composition = composition;
        self.after_edit = true;
        self.goal = None;
        self.follow_caret = true;
        changed
    }
}

#[cfg(test)]
#[path = "../../tests/unit/editor/mobile.rs"]
mod tests;
