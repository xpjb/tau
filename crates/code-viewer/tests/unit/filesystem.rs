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
    service.shared.roots.lock().unwrap().get_mut(&fs::canonicalize(&cwd).unwrap()).unwrap().refreshed = Some(Instant::now()-REFRESH);
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
