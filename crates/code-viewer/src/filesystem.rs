//! One daemon-owned index worker, bounded roots and ephemeral file reads. No
//! filesystem path or file body is persisted in chat/attachment storage.
use anyhow::{Context, Result, ensure};
use std::{collections::{BTreeMap, HashMap}, fs, io::Read, path::{Path, PathBuf}, sync::{Arc, Condvar, Mutex, atomic::{AtomicBool, Ordering}}, time::{Duration, Instant}};
use tau_protocol::files::*;

const INDEX_PATHS: usize = MAX_INDEX_PATHS;
const INDEX_BYTES: usize = MAX_INDEX_BYTES;
const ROOTS: usize = 4;
const REFRESH: Duration = Duration::from_secs(10);
#[derive(Clone)]
struct Snapshot { revision: String, items: Arc<Vec<IndexedPath>>, limited: bool }
struct Index { snapshot: Snapshot, previous: Option<Snapshot>, scanning: bool, refreshed: Option<Instant>, used: Instant }
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
                FileOperation::Index { revision } => {
                    ensure!(revision.as_ref().is_none_or(|r| r.len() <= 64), "Invalid index revision");
                    ensure!(path.is_dir(), "Search root is not a directory");
                    want_index(&shared, &path);
                    let (snapshot, previous, indexing) = {
                        let mut roots = shared.roots.lock().unwrap();
                        let index = roots.get_mut(&path).context("Index was evicted; retry")?;
                        index.used = Instant::now();
                        (index.snapshot.clone(), index.previous.clone(), index.scanning || index.refreshed.is_none())
                    };
                    Ok(index_reply(absolute, &snapshot, previous.as_ref(), revision, indexing))
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
        roots.insert(path.to_owned(), Index { snapshot: snapshot(vec![], false), previous: None, scanning: false, refreshed: None, used: Instant::now() });
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
        // Prune dot-directories before descending, not after collecting names.
        // Dotfiles such as .env remain eligible for the local Show hidden toggle;
        // explicit browsing/Here roots are independent of this recursive policy.
        let walk = ignore::WalkBuilder::new(&path).hidden(false).git_ignore(true).git_global(true).git_exclude(true)
            .require_git(false).follow_links(false).filter_entry(|e| e.depth() == 0 || (e.file_name() != ".git"
                && !(e.file_type().is_some_and(|kind| kind.is_dir()) && e.file_name().as_encoded_bytes().starts_with(b".")))).build();
        for found in walk {
            if shared.stop.load(Ordering::Acquire) { return; }
            if started.elapsed() > Duration::from_secs(15) { limited = true; break; }
            let Ok(found) = found else { limited = true; continue; };
            if found.depth() == 0 { continue; }
            let Some(kind) = found.file_type() else { limited = true; continue; };
            if !(kind.is_file() || kind.is_symlink()) { continue; }
            if kind.is_symlink() && !fs::metadata(found.path()).is_ok_and(|m| m.is_file()) { continue; }
            if wire_path(found.path()).is_err() { limited = true; continue; }
            let Ok(relative) = found.path().strip_prefix(&path) else { limited = true; continue; };
            let Ok(relative) = wire_path(relative) else { limited = true; continue; };
            let item = IndexedPath { path: relative, symlink: kind.is_symlink() };
            // Count escaping and framing without allocating serialized names.
            let size = item.wire_bytes();
            if items.len() >= INDEX_PATHS || bytes + size > INDEX_BYTES { limited = true; break; }
            bytes += size; items.push(item);
        }
        let snapshot = snapshot(items, limited);
        let mut roots = shared.roots.lock().unwrap();
        if let Some(index) = roots.get_mut(&path) {
            if index.snapshot.revision != snapshot.revision {
                index.previous = Some(std::mem::replace(&mut index.snapshot, snapshot));
            }
            index.scanning = false; index.refreshed = Some(Instant::now());
        }
    }
}

fn snapshot(mut items: Vec<IndexedPath>, limited: bool) -> Snapshot {
    items.sort_unstable_by(|a, b| a.path.cmp(&b.path));
    items.dedup_by(|a, b| a.path == b.path);
    let mut hash = blake3::Hasher::new();
    hash.update(&[u8::from(limited)]);
    for item in &items { hash.update(item.path.as_bytes()); hash.update(&[0, u8::from(item.symlink)]); }
    Snapshot { revision: hash.finalize().to_hex().to_string(), items: Arc::new(items), limited }
}
fn index_reply(path: String, current: &Snapshot, previous: Option<&Snapshot>, revision: Option<String>, indexing: bool) -> FileReply {
    let known = if revision.as_ref() == Some(&current.revision) { Some(current) }
        else { previous.filter(|old| revision.as_ref() == Some(&old.revision)) };
    let (base, entries, removed) = if let Some(old) = known {
        if old.revision == current.revision { (revision, vec![], vec![]) }
        else {
            let before: BTreeMap<_, _> = old.items.iter().map(|p| (p.path.as_str(), p)).collect();
            let after: BTreeMap<_, _> = current.items.iter().map(|p| (p.path.as_str(), p)).collect();
            let entries: Vec<_> = after.iter().filter(|(key, value)| before.get(*key) != Some(*value)).map(|(_, p)| (*p).clone()).collect();
            let removed: Vec<_> = before.keys().filter(|key| !after.contains_key(*key)).map(|p| (*p).to_owned()).collect();
            // A replacement storm can make a delta larger than a snapshot.
            let delta_bytes: usize = entries.iter().map(IndexedPath::wire_bytes).chain(removed.iter().map(|p| p.len()*2+4)).sum();
            if delta_bytes > MAX_INDEX_BYTES { (None, current.items.as_ref().clone(), vec![]) }
            else { (revision, entries, removed) }
        }
    } else { (None, current.items.as_ref().clone(), vec![]) };
    FileReply::Index { path, revision: current.revision.clone(), base, entries, removed, indexing, limited: current.limited }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(path: Option<&Path>, operation: FileOperation) -> FileRequest { FileRequest { session_id: "chat".into(), path: path.map(|p| p.to_str().unwrap().into()), operation } }
    #[test]
    fn index_snapshots_are_stable_conditional_and_send_additions_and_deletions() {
        let item = |path: &str| IndexedPath { path: path.into(), symlink: false };
        let first = snapshot(vec![item("src/b.rs"), item("src/a.rs")], false);
        assert_eq!(first.revision, snapshot(vec![item("src/a.rs"), item("src/b.rs")], false).revision);
        let full = index_reply("/work".into(), &first, None, None, false);
        let client = crate::finder::PathIndex::apply(None, &full).unwrap();
        let second = snapshot(vec![item("src/b.rs"), item("src/c.rs"), item(".config/tool")], true);
        let delta = index_reply("/work".into(), &second, Some(&first), Some(first.revision.clone()), false);
        assert!(matches!(&delta, FileReply::Index { base:Some(_), entries,removed,limited:true,.. } if entries.len()==2 && removed==&["src/a.rs"]));
        let client = crate::finder::PathIndex::apply(Some(&client), &delta).unwrap();
        assert_eq!(client.entries, *second.items); assert_eq!(client.visible, 2);
        assert!(crate::finder::PathIndex::apply(None, &delta).is_err());
        let unchanged = index_reply("/work".into(), &second, Some(&first), Some(second.revision.clone()), true);
        assert!(matches!(unchanged, FileReply::Index { base:Some(_),entries,removed,indexing:true,.. } if entries.is_empty() && removed.is_empty()));
        assert!(matches!(index_reply("/work".into(), &second, None, Some("unknown".into()), false), FileReply::Index {base:None,..}));
    }
    #[tokio::test]
    async fn name_sync_prunes_dot_directories_but_keeps_dotfiles_and_explicit_browsing() {
        let root = tempfile::tempdir().unwrap(); let cwd = root.path().to_owned();
        for name in ["src", ".config", "src/.nested", ".git", "ignored"] { fs::create_dir_all(cwd.join(name)).unwrap(); }
        for i in 0..350 { fs::write(cwd.join(format!("src/file{i}.rs")), "").unwrap(); }
        for name in [".config/tool", "src/.nested/file.rs", "src/.dot", ".git/config", "ignored/file.rs"] { fs::write(cwd.join(name), "").unwrap(); }
        // Ignore-file exceptions must not reopen dot-directory recursion.
        fs::write(cwd.join(".gitignore"), "ignored/\n!.config/**\n!src/.nested/**\n").unwrap();
        let service = FileSystem::new(cwd.clone());
        let end = Instant::now() + Duration::from_secs(5);
        loop {
            let reply = service.request(cwd.clone(), request(None, FileOperation::Index { revision: None })).await.unwrap();
            if matches!(&reply, FileReply::Index { indexing:false,.. }) {
                let index = crate::finder::PathIndex::apply(None, &reply).unwrap();
                assert_eq!(index.visible, 350); assert_eq!(index.entries.len(), 352);
                assert!(index.entries.iter().all(|p| p.path != ".git" && !p.path.starts_with("ignored/")
                    && p.path.split('/').rev().skip(1).all(|part| !part.starts_with('.'))));
                assert!(index.entries.iter().any(|p| p.path == "src/.dot"));
                assert!(matches!(&reply, FileReply::Index { limited: false, .. }));
                break;
            }
            assert!(Instant::now()<end); tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let FileReply::Directory { entries, .. } = service.request(cwd.clone(), request(None, FileOperation::List { after: None })).await.unwrap() else { panic!() };
        assert!(entries.iter().any(|e| e.name == ".config" && e.directory), "Dot-directories remain browsable");
        assert!(matches!(service.request(cwd.clone(), request(Some(&cwd.join(".config/tool")), FileOperation::Open { revision: None })).await.unwrap(), FileReply::Text {..}));
        // Here can explicitly opt into a hidden root; its own hidden descendants
        // are still pruned. Ordinary Show hidden never changes index interests.
        let hidden = cwd.join(".config");
        fs::create_dir(hidden.join(".cache")).unwrap(); fs::write(hidden.join(".cache/noise"), "").unwrap();
        loop {
            let reply = service.request(cwd.clone(), request(Some(&hidden), FileOperation::Index { revision: None })).await.unwrap();
            if let FileReply::Index { entries, indexing: false, limited: false, .. } = reply {
                assert_eq!(entries.iter().map(|p| p.path.as_str()).collect::<Vec<_>>(), ["tool"]); break;
            }
            assert!(Instant::now()<end); tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    #[tokio::test]
    async fn refreshed_filesystem_sync_sends_renames_and_removals_without_replaying_names() {
        let root = tempfile::tempdir().unwrap(); let cwd = root.path().to_owned();
        fs::write(cwd.join("before.rs"), "fn main() {}\n").unwrap();
        let service = FileSystem::new(cwd.clone());
        let end = Instant::now() + Duration::from_secs(5);
        let first = loop {
            let reply = service.request(cwd.clone(), request(None, FileOperation::Index {revision:None})).await.unwrap();
            if matches!(&reply, FileReply::Index {indexing:false,..}) { break crate::finder::PathIndex::apply(None, &reply).unwrap(); }
            assert!(Instant::now()<end); tokio::time::sleep(Duration::from_millis(10)).await;
        };
        fs::rename(cwd.join("before.rs"), cwd.join("after.rs")).unwrap();
        // Advance only this owned fixture's refresh clock, without a 10s sleep.
        service.shared.roots.lock().unwrap().get_mut(&cwd).unwrap().refreshed = Some(Instant::now()-REFRESH);
        service.shared.wake.notify_one();
        loop {
            let reply = service.request(cwd.clone(), request(None, FileOperation::Index {revision:Some(first.revision.clone())})).await.unwrap();
            if let FileReply::Index {revision,base,entries,removed,..} = &reply && revision != &first.revision {
                assert_eq!(base.as_ref(),Some(&first.revision));assert_eq!(removed,&["before.rs"]);
                assert_eq!(entries.len(),1);assert_eq!(entries[0].path,"after.rs");
                let current=crate::finder::PathIndex::apply(Some(&first),&reply).unwrap();assert_eq!(current.entries.len(),1);
                break;
            }
            assert!(Instant::now()<end); tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    #[tokio::test]
    async fn index_ignores_gitignored_paths_but_explicit_browsing_is_not_a_jail() {
        let root = tempfile::tempdir().unwrap(); let cwd = root.path().join("cwd"); fs::create_dir(&cwd).unwrap();
        fs::write(cwd.join(".gitignore"), "ignored/\n").unwrap(); fs::create_dir(cwd.join("ignored")).unwrap();
        fs::write(cwd.join("ignored/secret.rs"), "ignored, not forbidden").unwrap();
        fs::create_dir(cwd.join("src")).unwrap(); fs::write(cwd.join("src/main.rs"), "fn main() {}\n").unwrap();
        fs::write(root.path().join("outside.txt"), "outside\n").unwrap();
        let service = FileSystem::new(cwd.clone());
        let query = || request(None, FileOperation::Index { revision: None });
        let end = Instant::now()+Duration::from_secs(5);
        loop {
            let FileReply::Index { entries, indexing, .. } = service.request(cwd.clone(), query()).await.unwrap() else { panic!() };
            if !indexing { assert_eq!(entries.iter().filter(|p| crate::fuzzy_score("srcmain", &p.path).is_some()).count(), 1); break; }
            assert!(Instant::now() < end); tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let FileReply::Index { entries, .. } = service.request(cwd.clone(), query()).await.unwrap() else { panic!() };
        assert!(!entries.iter().any(|p| p.path.contains("secret")));
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
