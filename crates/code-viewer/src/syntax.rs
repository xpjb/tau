use crate::{Line, Paint};
use std::sync::OnceLock;
use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

const NAMES: &[&str] = &["comment", "string", "number", "constant", "keyword", "function", "type", "property", "operator", "punctuation", "variable"];
const COLORS: &[u32] = &[0x859783, 0xa6d189, 0xefbc8e, 0xefbc8e, 0xc6a0f6, 0x8ccef4, 0xe5c890, 0x99d1db, 0x81c8be, 0x9ba8b9, 0xd8dee9];
fn configuration(path: &str) -> Option<&'static HighlightConfiguration> {
    static RUST: OnceLock<Option<HighlightConfiguration>> = OnceLock::new();
    static PYTHON: OnceLock<Option<HighlightConfiguration>> = OnceLock::new();
    static JS: OnceLock<Option<HighlightConfiguration>> = OnceLock::new();
    static JSON: OnceLock<Option<HighlightConfiguration>> = OnceLock::new();
    let (cell, language, name, query) = match path.rsplit('.').next()? {
        "rs" => (&RUST, tree_sitter_rust::LANGUAGE, "rust", tree_sitter_rust::HIGHLIGHTS_QUERY),
        "py" | "pyi" => (&PYTHON, tree_sitter_python::LANGUAGE, "python", tree_sitter_python::HIGHLIGHTS_QUERY),
        "js" | "jsx" | "mjs" | "cjs" => (&JS, tree_sitter_javascript::LANGUAGE, "javascript", tree_sitter_javascript::HIGHLIGHT_QUERY),
        "json" => (&JSON, tree_sitter_json::LANGUAGE, "json", tree_sitter_json::HIGHLIGHTS_QUERY),
        _ => return None,
    };
    cell.get_or_init(|| {
        let mut config = HighlightConfiguration::new(language.into(), name, query, "", "").ok()?;
        config.configure(NAMES); Some(config)
    }).as_ref()
}
pub fn paint(path: &str, text: &str, lines: &mut [Line]) {
    // Parsing never delays a huge file preview indefinitely. Plain text is still
    // selectable/copyable when the syntax budget or language coverage is exceeded.
    if text.len() > 512 * 1024 { return; }
    let Some(config) = configuration(path) else { return; };
    let mut highlighter = Highlighter::new();
    let Ok(events) = highlighter.highlight(config, text.as_bytes(), None, |_| None) else { return; };
    let mut stack = vec![]; let mut line = 0; let mut start = 0;
    let offsets: Vec<_> = text.split_inclusive('\n').scan(0, |at, s| { *at += s.len(); Some(*at) }).collect();
    for event in events {
        match event {
            Ok(HighlightEvent::HighlightStart(kind)) => stack.push(kind.0),
            Ok(HighlightEvent::HighlightEnd) => { stack.pop(); }
            Ok(HighlightEvent::Source { start: from, end }) => {
                while line < lines.len() && start < end {
                    let finish = offsets.get(line).copied().unwrap_or(text.len());
                    if from < finish && let Some(&kind) = stack.last() {
                        let a = from.max(start)-start;
                        let b = end.min(start + lines[line].text.len())-start;
                        if a < b { lines[line].paint.push(Paint { range: a..b, color: COLORS[kind] }); }
                    }
                    if end < finish || line+1 == lines.len() { break; }
                    start = finish; line += 1;
                }
            }
            Err(_) => break,
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn syntax_handles_multiline_comments_and_unicode() {
        let doc = crate::Document::replace(None, "main.rs".into(), "1".into(), "/* café\nstill comment */\nfn main() { let s = \"🦀\"; }\n".into());
        assert!(doc.lines.iter().all(|line| !line.paint.is_empty()));
        assert!(doc.lines[1].paint.iter().any(|p| p.color == super::COLORS[0]));
        for line in doc.lines { for span in line.paint { assert!(line.text.get(span.range).is_some()); } }
    }
}
