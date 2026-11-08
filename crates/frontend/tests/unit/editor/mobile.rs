use super::*;
use chad::{Selection, TextInputEvent, TextSplice};

fn edit(e: &Editor, range: std::ops::Range<usize>, replacement: &str,
    selection: Selection, composition: Option<std::ops::Range<usize>>) -> TextInputEvent {
    TextInputEvent {
        id: e.native_id,
        generation: e.native_revision,
        splice: Some(TextSplice { range, replacement: replacement.into() }),
        selection,
        composition,
    }
}
#[test]
fn native_splices_preserve_selection_composition_and_generation_fences() {
    let mut e = Editor::new(String::new());
    let revision = e.native_revision;
    assert!(e.native_edit(edit(&e, 0..0, "a😀世界", Selection::new(5, 11), Some(5..11))));
    assert_eq!(e.selected(), "世界");
    assert_eq!(e.native_composition, Some(5..11));
    assert!(e.composing());
    let selection = TextInputEvent {
        id: e.native_id,
        generation: e.native_revision,
        splice: None,
        selection: Selection::new(5, 5),
        composition: None,
    };
    assert!(!e.native_edit(selection));
    assert!(!e.composing());
    assert_eq!(e.native_revision, revision);
    let late = edit(&e, 5..11, "old", Selection::new(8, 8), None);
    e.replace("!");
    assert!(!e.native_edit(late));
    assert_eq!(e.value, "a😀!世界");
    let bad = TextInputEvent {
        id: e.native_id,
        generation: e.native_revision,
        splice: None,
        selection: Selection::new(2, 5),
        composition: None,
    };
    assert!(!e.native_edit(bad));
    assert_eq!(e.value, "a😀!世界");
}
