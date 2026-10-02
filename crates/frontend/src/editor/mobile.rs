use super::*;
use crate::mobile_input::{Edit, Input};
use std::sync::atomic::{AtomicU64, Ordering};

pub(super) fn next_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}
fn utf16_byte(text: &str, offset: i32) -> usize {
    let mut units = 0;
    for (byte, ch) in text.char_indices() {
        if units + ch.len_utf16() > offset.max(0) as usize { return byte; }
        units += ch.len_utf16();
    }
    text.len()
}
impl Editor {
    pub fn native_id(&self) -> u64 { self.native_id }
    pub fn native_input(&self, request: u64) -> Option<Input> {
        if !self.visible { return None; }
        let view = self.view?;
        Some(Input {
            id: self.native_id, revision: self.native_revision, request,
            text: self.value.clone(), start: self.value[..self.anchor].encode_utf16().count() as i32,
            end: self.value[..self.caret.byte_index].encode_utf16().count() as i32,
            secret: view.secret, single_line: self.single_line,
            rect: [view.rect.x, view.rect.y, view.rect.width, view.rect.height], size: view.size,
            selection_rect: [view.rect.x, view.rect.y, view.rect.width, view.rect.height],
            max_bytes: tau_net::MAX_REQUEST_BYTES,
        })
    }
    pub fn accepts_native_edit(&self, edit: &Edit) -> bool {
        self.native_id == edit.id && self.native_revision == edit.revision
    }
    /// IME snapshots include composing text, so backgrounding also saves an
    /// unfinished word. Selection/composition-only updates never write a draft.
    pub fn native_edit(&mut self, edit: Edit) -> bool {
        if !self.accepts_native_edit(&edit) || edit.text.len() > tau_net::MAX_REQUEST_BYTES { return false; }
        let byte = |offset| normalize(&edit.text[..utf16_byte(&edit.text, offset)], self.single_line).len();
        let anchor = byte(edit.start);
        let caret = byte(edit.end);
        let composition = (edit.composing_start >= 0 && edit.composing_end > edit.composing_start)
            .then(|| byte(edit.composing_start)..byte(edit.composing_end));
        let value = normalize(&edit.text, self.single_line);
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
mod tests {
    use super::*;
    fn edit(e: &Editor, text: &str, start: i32, end: i32, composition: (i32,i32)) -> Edit {
        Edit { id: e.native_id, revision: e.native_revision, text: text.into(), start, end,
            composing_start: composition.0, composing_end: composition.1 }
    }
    #[test]
    fn native_selection_and_composition_use_utf16_without_resetting_the_ime_revision() {
        let mut e = Editor::new(String::new());
        let revision = e.native_revision;
        assert!(e.native_edit(edit(&e, "a😀世界", 3, 5, (3,5))));
        assert_eq!(e.selected(), "世界");
        assert_eq!(e.native_composition, Some(5..11));
        assert!(e.composing());
        assert!(!e.native_edit(edit(&e, "a😀世界", 3, 3, (-1,-1))));
        assert!(!e.composing());
        assert_eq!(e.native_revision, revision);
        let late = edit(&e, "a😀old", 6, 6, (-1,-1));
        e.replace("!");
        assert!(!e.native_edit(late));
        assert_eq!(e.value, "a😀!世界");
        // A bad UTF-16 offset inside a surrogate must never become a bad UTF-8 slice.
        assert!(e.native_edit(edit(&e, "a😀b", 2, 3, (-1,-1))));
        assert_eq!(e.selected(), "😀");
    }
}
