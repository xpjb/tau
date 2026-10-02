use super::*;
fn doc(old: Option<&Document>, text: &str) -> Document { Document::replace(old, "/root/code.rs".into(), text.into(), text.into()) }
#[test]
fn paragraph_identity_and_comments_follow_unchanged_text_not_line_numbers() {
    let a = doc(None, "a\nb\nc\nd\ne\nf\ng\n");
    let selection = Selection::new(&a, 3, 2);
    let b = doc(Some(&a), "inserted\na\nb\nc\nd\ne\nchanged\ng\n");
    assert_eq!(b.namespace, a.namespace);
    assert_eq!(selection.reference(&b).as_deref(), Some("/root/code.rs:4-5"));
    assert_eq!(a.lines[6].id, b.lines[7].id);
    for text in ["inserted\na\nb\nnew\nd\ne\nchanged\ng\n", "inserted\na\nb\nc\ninside\nd\ne\nchanged\ng\n", "a\nb\ne\nf\n"] {
        assert!(selection.range(&doc(Some(&b), text)).is_none());
    }
}
#[test]
fn unicode_crlf_empty_and_directory_components() {
    let a = doc(None, "é\r\n🦀\r\n");
    assert_eq!(a.lines[0].text, "é");
    assert_eq!(Selection::new(&a, 0, 1).reference(&a).unwrap(), "/root/code.rs:1-2");
    assert_eq!(doc(None, "").lines.len(), 1);
    assert!(fuzzy_score("src app", "src/frontend/app.rs").is_some());
    assert!(fuzzy_score("不存在", "src/app.rs").is_none());
    assert!(fuzzy_score("app", "src/app.rs") > fuzzy_score("app", "a_long_path/program.rs"));
}
