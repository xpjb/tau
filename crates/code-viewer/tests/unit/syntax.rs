#[test]
fn syntax_handles_multiline_comments_and_unicode() {
    let doc = crate::Document::replace(None, "main.rs".into(), "1".into(), "/* café\nstill comment */\nfn main() { let s = \"🦀\"; }\n".into());
    assert!(doc.lines.iter().all(|line| !line.paint.is_empty()));
    assert!(doc.lines[1].paint.iter().any(|p| p.color == super::COLORS[0]));
    for line in doc.lines { for span in line.paint { assert!(line.text.get(span.range).is_some()); } }
}
