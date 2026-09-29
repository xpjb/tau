//! UI-independent code buffers. Filesystem workers and syntax parsers are optional;
//! neither the daemon nor the text service owns a conversation's selection.
use std::{ops::Range, sync::atomic::{AtomicU64, Ordering}, time::Duration};

#[cfg(feature = "filesystem")]
pub mod filesystem;
#[cfg(feature = "syntax")]
mod syntax;
pub mod finder;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paint { pub range: Range<usize>, pub color: u32 }
#[derive(Clone, Debug)]
pub struct Line { pub id: u64, pub text: String, pub paint: Vec<Paint> }
#[derive(Debug)]
pub struct Document {
    pub namespace: u64,
    pub path: String,
    pub revision: String,
    pub lines: Vec<Line>,
    pub text: String,
    next_id: u64,
}
static NAMESPACE: AtomicU64 = AtomicU64::new(1);
impl Document {
    /// Called on a worker, never the render thread. A bounded patience diff keeps
    /// all equal paragraphs, including equal runs between separate edits.
    pub fn replace(previous: Option<&Self>, path: String, revision: String, text: String) -> Self {
        let previous = previous.filter(|old| old.path == path);
        let namespace = previous.map_or_else(|| NAMESPACE.fetch_add(1, Ordering::Relaxed), |old| old.namespace);
        let mut next_id = previous.map_or(1, |old| old.next_id);
        let parts: Vec<&str> = text.split_inclusive('\n').collect();
        let parts = if parts.is_empty() { vec![""] } else { parts };
        let mut ids = vec![0; parts.len()];
        if let Some(old) = previous {
            let diff = similar::TextDiff::configure().algorithm(similar::Algorithm::Patience)
                .timeout(Duration::from_millis(30)).diff_lines(&old.text, &text);
            for op in diff.ops() {
                if op.tag() == similar::DiffTag::Equal {
                    for (a, b) in op.old_range().zip(op.new_range()) {
                        if let (Some(line), Some(id)) = (old.lines.get(a), ids.get_mut(b)) { *id = line.id; }
                    }
                }
            }
        }
        let mut lines: Vec<_> = parts.into_iter().zip(ids).map(|(part, mut id)| {
            if id == 0 { id = next_id; next_id += 1; }
            Line { id, text: part.trim_end_matches('\n').strip_suffix('\r').unwrap_or(part.trim_end_matches('\n')).into(), paint: vec![] }
        }).collect();
        #[cfg(feature = "syntax")]
        syntax::paint(&path, &text, &mut lines);
        #[cfg(not(feature = "syntax"))]
        let _ = &mut lines;
        Self { namespace, path, revision, lines, text, next_id }
    }
    pub fn position(&self, id: u64) -> Option<usize> { self.lines.iter().position(|line| line.id == id) }
    pub fn selected_text(&self, selection: &Selection) -> Option<String> {
        let range = selection.range(self)?;
        Some(self.lines[range].iter().map(|line| line.text.as_str()).collect::<Vec<_>>().join("\n"))
    }
}

/// Retain the entire selected run, not just its endpoints: replacing or inserting
/// inside the run must not silently turn a comment into a different selection.
#[derive(Clone, Debug)]
pub struct Selection { pub anchor: u64, pub end: u64, ids: Vec<u64> }
impl Selection {
    pub fn new(doc: &Document, anchor: usize, end: usize) -> Self {
        let a = anchor.min(doc.lines.len()-1); let b = end.min(doc.lines.len()-1);
        Self { anchor: doc.lines[a].id, end: doc.lines[b].id,
            ids: doc.lines[a.min(b)..=a.max(b)].iter().map(|line| line.id).collect() }
    }
    pub fn range(&self, doc: &Document) -> Option<Range<usize>> {
        let first = doc.position(*self.ids.first()?)?;
        let end = first.checked_add(self.ids.len())?;
        let lines = doc.lines.get(first..end)?;
        lines.iter().map(|line| line.id).eq(self.ids.iter().copied()).then_some(first..end)
    }
    pub fn reference(&self, doc: &Document) -> Option<String> {
        let range = self.range(doc)?;
        Some(format!("{}:{}-{}", doc.path, range.start+1, range.end))
    }
}

/// Compatibility helper; interactive callers reuse a Finder on a worker.
pub fn fuzzy_score(query: &str, path: &str) -> Option<i64> {
    finder::Finder::new(query).score(path).map(i64::from)
}

#[cfg(test)]
mod tests {
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
}
