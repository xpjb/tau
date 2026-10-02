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
