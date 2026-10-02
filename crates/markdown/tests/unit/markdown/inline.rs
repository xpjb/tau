use super::*;
#[test]
fn nested_faces_projection_and_source_mapping() {
    let s = "A **bold _café_** &amp; `x*y` [link](https://x/(y))";
    let p = parse(s);
    assert_eq!(p.text, "A bold café & x*y link");
    let i = p.text.find("café").unwrap();
    assert!(
        p.runs
            .iter()
            .any(|r| r.range.contains(&i) && r.flags == STRONG | EMPHASIS)
    );
    let i = p.text.find('&').unwrap();
    assert_eq!(p.source_byte(i), s.find("&amp;").unwrap());
    let i = p.text.find("x*y").unwrap();
    assert!(
        p.runs
            .iter()
            .any(|r| r.range.contains(&i) && r.flags & CODE != 0)
    );
}
#[test]
fn triple_mixed_and_incomplete_delimiters() {
    assert_eq!(parse("***both***").runs[0].flags, STRONG | EMPHASIS);
    assert_eq!(parse("**bold *italic***").text, "bold italic");
    assert_eq!(parse("unfinished **hello").text, "unfinished **hello");
    assert_eq!(parse("snake_case_name").text, "snake_case_name");
    assert_eq!(parse("~~gone~~").runs[0].flags, STRIKE);
}
#[test]
fn literals_entities_and_breaks() {
    assert_eq!(
        parse("\\*yes\\* &#x1f30d; &unknown;").text,
        "*yes* 🌍 &unknown;"
    );
    assert_eq!(
        parse("one\ntwo  \nthree\\\nfour").text,
        "one\ntwo\nthree\nfour"
    );
    assert_eq!(parse("`one\ntwo`\nthree").text, "one two\nthree");
    assert_eq!(parse("`` `x` &amp; ``").text, "`x` &amp;");
    assert_eq!(parse(r"`a\`b`").text, r"a\b`");
    assert_eq!(parse("![alt](remote.png)").text, "alt");
    assert_eq!(
        parse("<script>alert(1)</script>").text,
        "<script>alert(1)</script>"
    );
}
#[test]
fn soft_breaks_preserve_formatting_and_source_mapping() {
    let source = "**café\n世界**\n[one\ntwo](https://example.org)";
    let rich = parse(source);
    assert_eq!(rich.text, "café\n世界\none\ntwo");
    for (byte, ch) in rich.text.char_indices() {
        let source_byte = rich.source_byte(byte);
        assert!(source[source_byte..].starts_with(ch));
    }
    let bold_break = rich.text.find('\n').unwrap();
    assert!(rich.runs.iter().any(|run| {
        run.range.contains(&bold_break) && run.flags & STRONG != 0
    }));
    let link_break = rich.text.rfind('\n').unwrap();
    assert!(rich.runs.iter().any(|run| {
        run.range.contains(&link_break) && run.flags & LINK != 0
    }));
}
#[test]
fn adversarial_unmatched_markers_are_bounded_and_utf8_safe() {
    for s in [
        "[".repeat(20000),
        "*a ".repeat(10000),
        "`x ``x ```x ".repeat(2000),
        "é\\世界 &x;".repeat(100),
    ] {
        let p = parse(&s);
        for m in p.mapping {
            assert!(s.is_char_boundary(m.source.start) && s.is_char_boundary(m.source.end));
        }
    }
}
