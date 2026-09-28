//! One daemon-owned index worker, bounded roots and ephemeral file reads. No
//! filesystem path or file body is persisted in chat/attachment storage.
use anyhow::{Context, Result, ensure};
use std::{collections::{BTreeMap, HashMap}, fs, io::Read, path::{Path, PathBuf}, sync::{Arc, Condvar, Mutex, atomic::{AtomicBool, Ordering}}, time::{Duration, Instant}};
use tau_protocol::files::*;

const INDEX_PATHS: usize = 200_000;
const INDEX_BYTES: usize = 32 * 1024 * 1024;
const ROOTS: usize = 4;
const REFRESH: Duration = Duration::from_secs(10);
struct Index { items: Arc<Vec<FileEntry>>, scanning: bool, limited: bool, refreshed: Option<Instant>, used: Instant }
struct Shared { roots: Mutex<HashMap<PathBuf, Index>>, wake: Condvar, stop: AtomicBool }
pub struct FileSystem { shared: Arc<Shared>, reads: Arc<tokio::sync::Semaphore> }
impl FileSystem {
    pub fn new(root: PathBuf) -> Self {
        let root = fs::canonicalize(&root).unwrap_or(root);
        let shared = Arc::new(Shared { roots: Mutex::new(HashMap::new()), wake: Condvar::new(), stop: AtomicBool::new(false) });
        want_index(&shared, &root);
        let worker = shared.clone();
        std::thread::Builder::new().name("tau-file-index".into()).spawn(move || index_worker(worker)).expect("start file index");
        Self { shared, reads: Arc::new(tokio::sync::Semaphore::new(2)) }
    }
    pub async fn request(&self, cwd: PathBuf, request: FileRequest) -> Result<FileReply> {
        ensure!(request.path.as_ref().is_none_or(|s| s.len() <= MAX_PATH_BYTES && !s.contains('\0')), "Invalid path");
        let permit = self.reads.clone().acquire_owned().await?;
        let shared = self.shared.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let path = request.path.as_deref().map(Path::new).map_or(cwd.clone(), |p| if p.is_absolute() { p.to_owned() } else { cwd.join(p) });
            let path = fs::canonicalize(path).context("Path is unavailable")?;
            let absolute = wire_path(&path)?;
            match request.operation {
                FileOperation::List { after } => list(&path, absolute, after),
                FileOperation::Open { revision } => read_text(&path, absolute, revision),
                FileOperation::Search { query } => {
                    ensure!(query.len() <= 256, "Search query is too long");
                    ensure!(path.is_dir(), "Search root is not a directory");
                    want_index(&shared, &path);
                    let (items, indexing, limited) = {
                        let mut roots = shared.roots.lock().unwrap();
                        let index = roots.get_mut(&path).context("Index was evicted; retry")?;
                        index.used = Instant::now();
                        (index.items.clone(), index.scanning || index.refreshed.is_none(), index.limited)
                    };
                    let mut best = BTreeMap::new();
                    for item in items.iter() {
                        let relative = item.path.strip_prefix(&absolute).unwrap_or(&item.path).trim_start_matches('/');
                        if let Some(score) = crate::fuzzy_score(&query, relative) {
                            best.insert((std::cmp::Reverse(score), item.path.as_str()), item);
                            if best.len() > SEARCH_RESULTS { best.pop_last(); }
                        }
                    }
                    Ok(FileReply::Search { path: absolute, entries: best.into_values().cloned().collect(), indexing, limited })
                }
            }
        }).await?
    }
}
impl Drop for FileSystem { fn drop(&mut self) { self.shared.stop.store(true, Ordering::Release); self.shared.wake.notify_all(); } }
fn wire_path(path: &Path) -> Result<String> {
    let text = path.to_str().context("Non-UTF-8 paths are not supported by the viewer")?;
    ensure!(text.len() <= MAX_PATH_BYTES, "Path is too long for the viewer");
    ensure!(!text.chars().any(char::is_control), "Control characters in paths are not supported by the viewer");
    Ok(text.into())
}
fn entry(path: &Path, name: String, metadata: &fs::Metadata) -> Result<FileEntry> {
    Ok(FileEntry { path: wire_path(path)?, name, directory: metadata.is_dir(), symlink: fs::symlink_metadata(path)?.file_type().is_symlink() })
}
fn list(path: &Path, absolute: String, after: Option<String>) -> Result<FileReply> {
    let mut page = BTreeMap::new();
    // Retain one page, not an unbounded directory. The cursor is the exact name,
    // independent of lossy display/sorting. Explicit browsing includes ignored files.
    for child in fs::read_dir(path).context("Cannot list directory")? {
        let child = child?;
        let Some(name) = child.file_name().to_str().map(str::to_owned) else { continue; };
        if after.as_ref().is_some_and(|after| &name <= after) { continue; }
        let Ok(metadata) = fs::metadata(child.path()) else { continue; };
        if !(metadata.is_file() || metadata.is_dir()) { continue; }
        if let Ok(item) = entry(&child.path(), name.clone(), &metadata) { page.insert(name, item); }
        if page.len() > DIRECTORY_PAGE+1 { page.pop_last(); }
    }
    let more = page.len() > DIRECTORY_PAGE;
    if more { page.pop_last(); }
    let next = more.then(|| page.last_key_value().unwrap().0.clone());
    Ok(FileReply::Directory { path: absolute, parent: path.parent().map(wire_path).transpose()?, entries: page.into_values().collect(), next })
}
fn read_text(path: &Path, absolute: String, revision: Option<String>) -> Result<FileReply> {
    ensure!(fs::metadata(path)?.is_file(), "Only regular text files can be viewed");
    let mut options = fs::OpenOptions::new(); options.read(true);
    // Close the metadata/open race with FIFOs/devices: no blocking on a pipe if
    // a path is swapped after inspection. Validate the opened handle as well.
    #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; options.custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY); }
    let file = options.open(path).context("Cannot read file")?;
    let before = file.metadata()?;
    ensure!(before.is_file(), "Only regular text files can be viewed");
    ensure!(before.len() <= MAX_FILE_BYTES as u64, "Preview limit is 4 MiB");
    let mut bytes = Vec::with_capacity(before.len() as usize);
    (&file).take(MAX_FILE_BYTES as u64+1).read_to_end(&mut bytes)?;
    let after = file.metadata()?;
    ensure!(bytes.len() <= MAX_FILE_BYTES, "Preview limit is 4 MiB");
    ensure!(before.len() == after.len() && before.modified().ok() == after.modified().ok() && bytes.len() as u64 == after.len(), "File changed during read; retrying");
    let text = String::from_utf8(bytes).context("Not a UTF-8 text file")?;
    ensure!(!text.chars().any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t')), "Binary file; no code preview");
    ensure!(text.bytes().filter(|&b|b==b'\n').count() <= MAX_FILE_LINES, "Preview limit is 100,000 lines");
    let hash = blake3::hash(text.as_bytes()).to_hex().to_string();
    if revision.as_deref() == Some(hash.as_str()) { return Ok(FileReply::Unchanged { path: absolute, revision: hash }); }
    Ok(FileReply::Text { path: absolute, revision: hash, text })
}
fn want_index(shared: &Shared, path: &Path) {
    let mut roots = shared.roots.lock().unwrap();
    if !roots.contains_key(path) {
        if roots.len() >= ROOTS && let Some(old) = roots.iter().min_by_key(|(_, i)| i.used).map(|(p, _)| p.clone()) { roots.remove(&old); }
        roots.insert(path.to_owned(), Index { items: Arc::new(vec![]), scanning: false, limited: false, refreshed: None, used: Instant::now() });
    }
    drop(roots); shared.wake.notify_one();
}
fn index_worker(shared: Arc<Shared>) {
    while !shared.stop.load(Ordering::Acquire) {
        let path = {
            let mut roots = shared.roots.lock().unwrap();
            let path = roots.iter().filter(|(_, i)| !i.scanning && i.refreshed.is_none_or(|at| at.elapsed() >= REFRESH))
                .min_by_key(|(_, i)| i.refreshed).map(|(p, _)| p.clone());
            if let Some(path) = &path { roots.get_mut(path).unwrap().scanning = true; }
            else { let _ = shared.wake.wait_timeout(roots, REFRESH).unwrap(); }
            path
        };
        let Some(path) = path else { continue; };
        let mut items = vec![]; let mut bytes = 0; let mut limited = false;
        let started = Instant::now();
        // Discover project/folder names before one enormous cache subtree can
        // consume a bounded root index. Both passes use identical ignore rules.
        'scan: for depth in [Some(2), None] {
        let walk = ignore::WalkBuilder::new(&path).hidden(false).git_ignore(true).git_global(true).git_exclude(true)
            .require_git(false).follow_links(false).max_depth(depth).filter_entry(|e| e.file_name() != ".git").build();
        for found in walk {
            if shared.stop.load(Ordering::Acquire) { return; }
            if items.len() >= INDEX_PATHS || bytes >= INDEX_BYTES || started.elapsed() > Duration::from_secs(15) { limited = true; break 'scan; }
            let Ok(found) = found else { limited = true; continue; };
            if found.depth() == 0 || depth.is_none() && found.depth() <= 2 { continue; }
            let Some(kind) = found.file_type() else { continue; };
            if !(kind.is_file() || kind.is_dir() || kind.is_symlink()) { continue; }
            let Ok(metadata) = fs::metadata(found.path()) else { continue; };
            let Some(name) = found.file_name().to_str() else { continue; };
            if let Ok(item) = entry(found.path(), name.into(), &metadata) { bytes += item.path.len()+item.name.len(); items.push(item); }
        }
        }
        let mut roots = shared.roots.lock().unwrap();
        if let Some(index) = roots.get_mut(&path) { index.items = Arc::new(items); index.scanning = false; index.limited = limited; index.refreshed = Some(Instant::now()); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(path: Option<&Path>, operation: FileOperation) -> FileRequest { FileRequest { session_id: "chat".into(), path: path.map(|p| p.to_str().unwrap().into()), operation } }
    #[tokio::test]
    async fn index_ignores_gitignored_paths_but_explicit_browsing_is_not_a_jail() {
        let root = tempfile::tempdir().unwrap(); let cwd = root.path().join("cwd"); fs::create_dir(&cwd).unwrap();
        fs::write(cwd.join(".gitignore"), "ignored/\n").unwrap(); fs::create_dir(cwd.join("ignored")).unwrap();
        fs::write(cwd.join("ignored/secret.rs"), "ignored, not forbidden").unwrap();
        fs::create_dir(cwd.join("src")).unwrap(); fs::write(cwd.join("src/main.rs"), "fn main() {}\n").unwrap();
        fs::write(root.path().join("outside.txt"), "outside\n").unwrap();
        let service = FileSystem::new(cwd.clone());
        let query = || request(None, FileOperation::Search { query: "srcmain".into() });
        let end = Instant::now()+Duration::from_secs(5);
        loop {
            let FileReply::Search { entries, indexing, .. } = service.request(cwd.clone(), query()).await.unwrap() else { panic!() };
            if !indexing { assert_eq!(entries.len(), 1); break; }
            assert!(Instant::now() < end); tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let FileReply::Search { entries, .. } = service.request(cwd.clone(), request(None, FileOperation::Search { query: "secret".into() })).await.unwrap() else { panic!() };
        assert!(entries.is_empty());
        for path in [cwd.join("ignored/secret.rs"), cwd.join("../outside.txt")] {
            let FileReply::Text { path: returned, text, revision } = service.request(cwd.clone(), request(Some(&path), FileOperation::Open { revision: None })).await.unwrap() else { panic!() };
            assert!(!text.is_empty()); assert!(Path::new(&returned).is_absolute());
            assert!(matches!(service.request(cwd.clone(), request(Some(&path), FileOperation::Open { revision: Some(revision) })).await.unwrap(), FileReply::Unchanged {..}));
        }
    }
    #[tokio::test]
    async fn paging_live_replacements_binary_size_and_symlink_safety() {
        let root = tempfile::tempdir().unwrap(); let cwd = root.path().to_owned();
        for i in 0..DIRECTORY_PAGE+3 { fs::write(cwd.join(format!("f{i:04}")), "before\n").unwrap(); }
        let service = FileSystem::new(cwd.clone());
        let FileReply::Directory { entries, next, .. } = service.request(cwd.clone(), request(None, FileOperation::List { after: None })).await.unwrap() else { panic!() };
        assert_eq!(entries.len(), DIRECTORY_PAGE);
        let FileReply::Directory { entries, next, .. } = service.request(cwd.clone(), request(None, FileOperation::List { after: next })).await.unwrap() else { panic!() };
        assert_eq!(entries.len(), 3); assert!(next.is_none());
        let file = cwd.join("live"); fs::write(&file, "before\n").unwrap();
        let FileReply::Text { revision, .. } = service.request(cwd.clone(), request(Some(&file), FileOperation::Open { revision: None })).await.unwrap() else { panic!() };
        fs::write(cwd.join("replace"), "after\n").unwrap(); fs::rename(cwd.join("replace"), &file).unwrap();
        assert!(matches!(service.request(cwd.clone(), request(Some(&file), FileOperation::Open { revision: Some(revision) })).await.unwrap(), FileReply::Text {text,..} if text=="after\n"));
        fs::write(&file, b"binary\0").unwrap(); assert!(service.request(cwd.clone(), request(Some(&file), FileOperation::Open { revision: None })).await.is_err());
        fs::File::create(&file).unwrap().set_len(MAX_FILE_BYTES as u64+1).unwrap();
        assert!(service.request(cwd.clone(), request(Some(&file), FileOperation::Open { revision: None })).await.is_err());
        #[cfg(unix)] {
            std::os::unix::fs::symlink(".", cwd.join("loop")).unwrap();
            assert!(matches!(service.request(cwd.clone(), request(Some(&cwd.join("loop")), FileOperation::List { after: None })).await.unwrap(), FileReply::Directory {..}));
        }
    }
}
