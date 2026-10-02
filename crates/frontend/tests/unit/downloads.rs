use super::*;
use std::io::Write;
fn archive(path: &Path, entries: &[(&str, &[u8])]) {
    let mut writer = zip::ZipWriter::new(fs::File::create(path).unwrap());
    for (name, content) in entries {
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        if name.ends_with('/') {
            writer.add_directory(*name, options).unwrap();
        } else {
            writer.start_file(*name, options).unwrap();
            writer.write_all(content).unwrap();
        }
    }
    writer.finish().unwrap();
}
#[test]
fn zip_extracts_to_distinct_folders_and_rejects_escape_and_duplicates() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bundle.zip");
    archive(&path, &[("notes/readme.txt", b"safe")]);
    let first = extract_zip(&path).unwrap();
    let second = extract_zip(&path).unwrap();
    assert_eq!(fs::read(first.join("readme.txt")).unwrap(), b"safe");
    assert_eq!(second.file_name().unwrap(), "notes (2)");
    archive(&path, &[("../outside", b"bad")]);
    assert!(extract_zip(&path).is_err());
    assert!(!dir.path().join("outside").exists());
    archive(&path, &[("FILE", b"first"), ("file", b"second")]);
    assert!(extract_zip(&path).is_err());
}
#[test]
fn zip_opens_its_single_root_without_an_extra_wrapper() {
    for explicit_root in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("download-name.ZIP");
        let mut entries: Vec<(&str, &[u8])> = vec![];
        if explicit_root {
            entries.push(("project/", b""));
        }
        entries.extend([("project/src/main.rs", b"fn main() {}".as_slice()), ("project/README.md", b"read me")]);
        archive(&path, &entries);
        let folder = extract_zip(&path).unwrap();
        assert_eq!(folder, dir.path().join("project"));
        assert_eq!(fs::read(folder.join("src/main.rs")).unwrap(), b"fn main() {}");
        assert_eq!(fs::read(folder.join("README.md")).unwrap(), b"read me");
        assert!(!folder.join("project").exists());
        assert!(!dir.path().join("download-name").exists());
    }
}
#[test]
fn zip_keeps_loose_files_and_multiple_roots_inside_an_archive_named_folder() {
    for entries in [
        vec![("README.md", b"read me".as_slice()), ("src/main.rs", b"fn main() {}")],
        vec![("one/file.txt", b"one".as_slice()), ("two/file.txt", b"two")],
        vec![("only.txt", b"single file".as_slice())],
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bundle.zip");
        archive(&path, &entries);
        let folder = extract_zip(&path).unwrap();
        assert_eq!(folder, dir.path().join("bundle"));
        for (name, content) in entries {
            assert_eq!(fs::read(folder.join(name)).unwrap(), content);
        }
    }
}
#[test]
fn zip_root_collisions_preserve_user_files_and_leave_no_staging_folder() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bundle.zip");
    archive(&path, &[("project/src/main.rs", b"safe")]);
    fs::write(dir.path().join("project"), b"user file").unwrap();
    fs::create_dir(dir.path().join("project (2)")).unwrap();
    fs::write(dir.path().join("project (2)/notes.txt"), b"user edits").unwrap();
    let folder = extract_zip(&path).unwrap();
    assert_eq!(folder, dir.path().join("project (3)"));
    assert_eq!(fs::read(folder.join("src/main.rs")).unwrap(), b"safe");
    assert_eq!(fs::read(dir.path().join("project")).unwrap(), b"user file");
    assert_eq!(fs::read(dir.path().join("project (2)/notes.txt")).unwrap(), b"user edits");
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 4);
}
#[test]
fn zip_rejects_invalid_archives_without_publishing_partial_contents() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bundle.zip");
    for unsafe_path in ["../escape", "..\\escape", "CON.txt", "project/trailing. ", "FILE"] {
        archive(&path, &[("file", b"safe"), (unsafe_path, b"unsafe")]);
        assert!(extract_zip(&path).is_err(), "accepted {unsafe_path}");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1, "partial extraction leaked");
    }
    let mut writer = zip::ZipWriter::new(fs::File::create(&path).unwrap());
    writer.add_symlink("project/link", "../../escape", zip::write::SimpleFileOptions::default()).unwrap();
    writer.finish().unwrap();
    assert!(extract_zip(&path).is_err());
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    archive(&path, &[]);
    assert!(extract_zip(&path).is_err());
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    archive(&path, &[("empty/", b"")]);
    assert_eq!(extract_zip(&path).unwrap(), dir.path().join("empty"));
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
}
#[test]
fn remembered_export_is_scoped_and_survives_a_store_restart() {
    let dir = tempfile::tempdir().unwrap();
    let store = crate::store::Store::open(dir.path().join("local")).unwrap();
    let saved = SavedDownload {
        location: "Downloads/Tau/report.txt".into(),
        reference: "record".into(),
        mime_type: "text/plain".into(),
    };
    store
        .record_download("account", "lineage", "chat", "file", &saved)
        .unwrap();
    drop(store);
    let store = crate::store::Store::open(dir.path().join("local")).unwrap();
    assert_eq!(
        store
            .saved_download("account", "lineage", "chat", "file")
            .unwrap(),
        Some(saved)
    );
    assert_eq!(
        store
            .saved_download("account", "new-source", "chat", "file")
            .unwrap(),
        None
    );
    assert_eq!(
        store
            .saved_download("another-account", "lineage", "chat", "file")
            .unwrap(),
        None
    );
    store
        .forget_download("account", "lineage", "chat", "file")
        .unwrap();
    assert_eq!(
        store
            .saved_download("account", "lineage", "chat", "file")
            .unwrap(),
        None
    );
}
#[test]
fn saves_in_downloads_without_overwriting_and_sanitizes_names() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in");
    fs::write(&input, b"hello").unwrap();
    let out = dir.path().join("Downloads/Tau");
    let first = save_into(&out, &input, "../../CON.zip").unwrap();
    let second = save_into(&out, &input, "..\\CON.zip").unwrap();
    assert_eq!(Path::new(&first.reference).file_name().unwrap(), "_CON.zip");
    assert_eq!(
        Path::new(&second.reference).file_name().unwrap(),
        "_CON (2).zip"
    );
    assert_eq!(fs::read(first.reference).unwrap(), b"hello");
    assert_eq!(fs::read(second.reference).unwrap(), b"hello");
    assert_eq!(name("../.."), "tau-attachment");
    assert_eq!(mime("image.PNG"), "image/png");
}
