//! Reusable fzf-style path matching. Full ranking runs on a coalesced client
//! worker; only visible rows need match indices on the render thread.
use nucleo_matcher::{Config, Matcher, Utf32Str, pattern::{CaseMatching, Normalization, Pattern}};
use std::ops::Range;

pub struct Finder { matcher: Matcher, pattern: Pattern, chars: Vec<char>, indices: Vec<u32> }
impl Finder {
    pub fn new(query: &str) -> Self {
        Self { matcher: Matcher::new(Config::DEFAULT.match_paths()),
            pattern: Pattern::parse(query, CaseMatching::Smart, Normalization::Smart), chars: vec![], indices: vec![] }
    }
    pub fn query(&mut self, query: &str) {
        self.pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
    }
    pub fn score(&mut self, path: &str) -> Option<u32> {
        self.pattern.score(Utf32Str::new(path, &mut self.chars), &mut self.matcher)
    }
    /// Byte ranges, not character offsets: safe for syntax paint and Unicode.
    pub fn highlights(&mut self, path: &str) -> Vec<Range<usize>> {
        self.indices.clear();
        if self.pattern.indices(Utf32Str::new(path, &mut self.chars), &mut self.matcher, &mut self.indices).is_none() { return vec![]; }
        self.indices.sort_unstable(); self.indices.dedup();
        let mut matches = self.indices.iter().copied().peekable();
        let mut ranges: Vec<Range<usize>> = vec![];
        for (i, (byte, ch)) in path.char_indices().enumerate() {
            if matches.peek().copied() == Some(i as u32) {
                matches.next();
                if let Some(last) = ranges.last_mut() && last.end == byte { last.end += ch.len_utf8(); }
                else { ranges.push(byte..byte + ch.len_utf8()); }
            }
        }
        ranges
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn forgiving_paths_unordered_words_boundaries_and_extended_terms() {
        for (query, path) in [("fntapcdvw", "frontend/src/app/code_view.rs"), ("main src", "src/main.rs"),
            ("cafe", "src/café.rs"), ("src !test .rs$", "src/main.rs"), ("'code_view", "frontend/src/app/code_view.rs")] {
            assert!(Finder::new(query).score(path).is_some(), "{query}: {path}");
        }
        assert!(Finder::new("src !test").score("src/test.rs").is_none());
        assert!(Finder::new("Main").score("src/main.rs").is_none());
        assert!(Finder::new("main").score("src/main.rs") > Finder::new("main").score("many/arbitrary/intermediate/names.rs"));
        let path = "src/café_🦀.rs";
        let ranges = Finder::new("cafe 🦀").highlights(path);
        assert_eq!(ranges.iter().map(|r| &path[r.clone()]).collect::<Vec<_>>(), ["café", "🦀"]);
    }
}

/// A verified, sorted client snapshot. Only names live here, never file bodies.
#[derive(Debug)]
pub struct PathIndex {
    pub root: String,
    pub revision: String,
    pub entries: Vec<tau_protocol::files::IndexedPath>,
    pub visible: usize,
}
impl PathIndex {
    pub fn apply(previous: Option<&Self>, reply: &tau_protocol::files::FileReply) -> anyhow::Result<Self> {
        use anyhow::{bail, ensure};
        use tau_protocol::files::*;
        let FileReply::Index { path, revision, base, entries, removed, .. } = reply else { bail!("Expected a file index") };
        ensure!(path.len() <= MAX_PATH_BYTES && revision.len() == 64, "Invalid index identity");
        let mut paths = if let Some(base) = base {
            let previous = previous.filter(|p| p.root == *path && p.revision == *base).ok_or_else(|| anyhow::anyhow!("Index revision changed; resyncing"))?;
            previous.entries.iter().cloned().map(|p| (p.path.clone(), p)).collect::<std::collections::BTreeMap<_, _>>()
        } else {
            ensure!(removed.is_empty(), "Snapshot cannot remove paths");
            std::collections::BTreeMap::new()
        };
        for path in removed { paths.remove(path); }
        for entry in entries {
            ensure!(!entry.path.is_empty() && entry.path.len() <= MAX_PATH_BYTES
                && !entry.path.chars().any(char::is_control)
                && entry.path.split('/').all(|p| !matches!(p, "" | "." | "..")), "Invalid indexed path");
            paths.insert(entry.path.clone(), entry.clone());
        }
        ensure!(paths.len() <= MAX_INDEX_PATHS && paths.values().map(|p| p.path.len()*2+64).sum::<usize>() <= MAX_INDEX_BYTES, "Index exceeds client limits");
        let entries: Vec<_> = paths.into_values().collect();
        let visible = entries.iter().filter(|p| !p.hidden()).count();
        Ok(Self { root: path.clone(), revision: revision.clone(), entries, visible })
    }
    pub fn absolute(&self, row: usize) -> String { format!("{}/{}", self.root.trim_end_matches('/'), self.entries[row].path) }
}
