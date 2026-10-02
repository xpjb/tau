//! Reusable fzf-style path matching. Full ranking runs on a coalesced client
//! worker; only visible rows need match indices on the render thread.
use nucleo_matcher::{Config, Matcher as FuzzyMatcher, Utf32Str, pattern::{CaseMatching, Normalization, Pattern}};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

pub struct Finder { matcher: FuzzyMatcher, pattern: Pattern, chars: Vec<char>, indices: Vec<u32> }
impl Finder {
    pub fn new(query: &str) -> Self {
        Self { matcher: FuzzyMatcher::new(Config::DEFAULT.match_paths()),
            pattern: Pattern::parse(query, CaseMatching::Smart, Normalization::Smart), chars: vec![], indices: vec![] }
    }
    pub fn query(&mut self, query: &str) {
        self.pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
    }
    pub fn score(&mut self, path: &str) -> Option<u32> {
        self.pattern.score(haystack(path, &mut self.chars), &mut self.matcher)
    }
    /// Byte ranges, not character offsets: safe for syntax paint and Unicode.
    pub fn highlights(&mut self, path: &str) -> Vec<Range<usize>> {
        self.indices.clear();
        if self.pattern.indices(haystack(path, &mut self.chars), &mut self.matcher, &mut self.indices).is_none() { return vec![]; }
        self.indices.sort_unstable(); self.indices.dedup();
        let mut matches = self.indices.iter().copied().peekable();
        let mut ranges: Vec<Range<usize>> = vec![];
        for (i, (byte, grapheme)) in path.grapheme_indices(true).enumerate() {
            if matches.peek().copied() == Some(i as u32) {
                matches.next();
                if let Some(last) = ranges.last_mut() && last.end == byte { last.end += grapheme.len(); }
                else { ranges.push(byte..byte + grapheme.len()); }
            }
        }
        ranges
    }
}
fn haystack<'a>(path: &'a str, chars: &'a mut Vec<char>) -> Utf32Str<'a> {
    if path.is_ascii() { Utf32Str::Ascii(path.as_bytes()) }
    else {
        chars.clear(); chars.extend(nucleo_matcher::chars::graphemes(path));
        Utf32Str::Unicode(chars)
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
        for path in ["src/cafe\u{301}_👩‍💻.rs", "src/é👩‍💻.rs"] {
            let ranges = Finder::new(".rs").highlights(path);
            assert_eq!(ranges.iter().map(|r| &path[r.clone()]).collect::<String>(), ".rs");
        }
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
    pub entries: Vec<tau_net::files::IndexedPath>,
    pub visible: usize,
}
impl PathIndex {
    pub fn apply(previous: Option<&Self>, reply: &tau_net::files::FileReply) -> anyhow::Result<Self> {
        use anyhow::{bail, ensure};
        use tau_net::files::*;
        let FileReply::Index { path, revision, base, entries, removed, .. } = reply else { bail!("Expected a file index") };
        ensure!(path.len() <= MAX_PATH_BYTES && revision.len() == 64, "Invalid index identity");
        ensure!(entries.len() <= MAX_INDEX_PATHS && removed.len() <= MAX_INDEX_PATHS, "Index change exceeds client limits");
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
        ensure!(paths.len() <= MAX_INDEX_PATHS && paths.values().map(IndexedPath::wire_bytes).sum::<usize>() <= MAX_INDEX_BYTES, "Index exceeds client limits");
        let entries: Vec<_> = paths.into_values().collect();
        let visible = entries.iter().filter(|p| !p.hidden()).count();
        Ok(Self { root: path.clone(), revision: revision.clone(), entries, visible })
    }
    pub fn absolute(&self, row: usize) -> String { format!("{}/{}", self.root.trim_end_matches('/'), self.entries[row].path) }
}

use std::sync::Arc;

/// One reusable matcher thread with latest-only input/output. A slow previous
/// query cannot publish over a newer keystroke or an updated index.
pub struct Matches { pub generation: u64, pub index: Arc<PathIndex>, pub rows: Vec<usize> }
struct Query { generation: u64, index: Arc<PathIndex>, text: String, hidden: bool }
struct Work {
    query: std::sync::Mutex<Option<Query>>, done: std::sync::Mutex<Option<Arc<Matches>>>,
    wake: std::sync::Condvar, generation: std::sync::atomic::AtomicU64, stop: std::sync::atomic::AtomicBool,
}
pub struct Matcher { work: Arc<Work> }
impl Matcher {
    pub fn new(wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
        let work = Arc::new(Work { query: Default::default(), done: Default::default(), wake: Default::default(), generation: AtomicU64::new(0), stop: AtomicBool::new(false) });
        let w = work.clone();
        std::thread::Builder::new().name("tau-file-match".into()).spawn(move || {
            let mut finder = Finder::new("");
            loop {
                let query = {
                    let mut slot = w.query.lock().unwrap();
                    while slot.is_none() && !w.stop.load(Ordering::Relaxed) { slot = w.wake.wait(slot).unwrap(); }
                    if w.stop.load(Ordering::Relaxed) { return; }
                    slot.take().unwrap()
                };
                finder.query(&query.text);
                let mut ranked = Vec::new();
                let cancelled = || w.stop.load(Ordering::Relaxed) || w.generation.load(Ordering::Relaxed) != query.generation;
                for (i, path) in query.index.entries.iter().enumerate() {
                    if i % 128 == 0 && cancelled() { break; }
                    if !query.hidden && path.hidden() { continue; }
                    if let Some(score) = finder.score(&path.path) { ranked.push((std::cmp::Reverse(score), path.path.len(), i)); }
                }
                if cancelled() { continue; }
                ranked.sort_unstable();
                if cancelled() { continue; }
                *w.done.lock().unwrap() = Some(Arc::new(Matches { generation: query.generation, index: query.index, rows: ranked.into_iter().map(|(_, _, i)| i).collect() }));
                (wake)();
            }
        }).expect("start local file matcher");
        Self { work }
    }
    pub fn query(&self, index: Arc<PathIndex>, text: String, hidden: bool) -> u64 {
        let mut slot = self.work.query.lock().unwrap();
        let generation = self.work.generation.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        *slot = Some(Query { generation, index, text, hidden });
        self.work.wake.notify_one(); generation
    }
    pub fn cancel(&self) -> u64 {
        let mut slot = self.work.query.lock().unwrap();
        let generation = self.work.generation.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        *slot = None;
        self.work.done.lock().unwrap().take();
        generation
    }
    pub fn take(&self) -> Option<Arc<Matches>> { self.work.done.lock().unwrap().take() }
}
impl Drop for Matcher {
    fn drop(&mut self) {
        let _slot = self.work.query.lock().unwrap();
        self.work.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        self.work.wake.notify_one();
    }
}


#[cfg(test)]
mod matcher_tests {
use super::*;
use std::time::Duration;
use tau_net::files::*;
#[test]
fn matching_is_latest_only_and_keeps_all_results_on_a_large_index() {
    let paths=(0..50_000).map(|i|IndexedPath {path:format!("frontend/src/app/module_{i:05}.rs"),symlink:false}).collect::<Vec<_>>();
    let reply=FileReply::Index {path:"/work".into(),revision:"a".repeat(64),base:None,entries:paths,removed:vec![],indexing:false,limited:false};
    let index=Arc::new(PathIndex::apply(None,&reply).unwrap());
    let (wake,wakes)=std::sync::mpsc::channel();
    let matcher=Matcher::new(Arc::new(move || {let _=wake.send(());}));
    let start=std::time::Instant::now();
    let mut generation=0;
    for query in ["zzzz", "module", "module4", "mdrs fnt"] {generation=matcher.query(index.clone(),query.into(),false);}
    loop {
        wakes.recv_timeout(Duration::from_secs(10).saturating_sub(start.elapsed())).expect("Local matcher must wake an idle UI");
        if let Some(done)=matcher.take() && done.generation==generation {
            assert_eq!(done.rows.len(),50_000);assert!(Arc::ptr_eq(&index,&done.index));break;
        }
    }
}

}
