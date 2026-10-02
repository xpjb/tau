use super::*;
fn file(size: Option<u64>) -> ChatAttachment {
    ChatAttachment {
        source_path: None,
        kind: AttachmentKind::File,
        file_name: "archive.zip".into(),
        caption: None,
        size,
    }
}
fn download(
    transferred: u64,
    total: u64,
    done: bool,
    failure: Option<&str>,
) -> crate::controller::Download {
    crate::controller::Download::new(
        TransferStatus {
            transferred,
            total,
            network_bytes: 0,
            done,
            failure: failure.map(str::to_owned),
        },
        PathBuf::from("/not-a-file"),
    )
}
#[test]
fn byte_labels_are_human_readable_and_safe_at_boundaries() {
    for (bytes, label) in [
        (0, "0 B"),
        (1023, "1023 B"),
        (1024, "1.0 KB"),
        (1536, "1.5 KB"),
        (10 * 1024, "10 KB"),
        (50 * 1024 * 1024, "50 MB"),
        (u64::MAX, "16 EB"),
    ] {
        assert_eq!(format_bytes(bytes), label);
    }
}
#[test]
fn file_saves_wait_for_verified_completion_once_and_discard_failed_or_stale_intents() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("verified");
    std::fs::write(&path, b"verified").unwrap();
    let key = "chat:entry".to_owned();
    let mut pending = HashMap::from([(key.clone(), (path.clone(), "report.txt".into()))]);
    let mut downloads = HashMap::from([(
        key.clone(),
        crate::controller::Download::new(
            TransferStatus {
                transferred: 4,
                total: 8,
                network_bytes: 4,
                done: false,
                failure: None,
            },
            path.clone(),
        ),
    )]);
    assert!(completed_exports(&mut pending, &downloads).is_empty());
    assert_eq!(pending.len(), 1);
    downloads.get_mut(&key).unwrap().status.done = true;
    downloads.get_mut(&key).unwrap().status.failure = Some("Cancelled".into());
    assert!(completed_exports(&mut pending, &downloads).is_empty());
    assert!(pending.is_empty());
    pending.insert(key.clone(), (path.clone(), "report.txt".into()));
    downloads.get_mut(&key).unwrap().status.failure = None;
    let exports = completed_exports(&mut pending, &downloads);
    assert_eq!(exports.len(), 1);
    assert!(
        matches!(&exports[0], PlatformAction::SaveDownload { key:k,source:p,name:n } if k==&key && p==&path && n=="report.txt")
    );
    assert!(pending.is_empty());
    assert!(completed_exports(&mut pending, &downloads).is_empty());
    pending.insert(key.clone(), (path.clone(), "report.txt".into()));
    downloads.get_mut(&key).unwrap().path = root.path().join("another-account");
    assert!(completed_exports(&mut pending, &downloads).is_empty());
    assert!(pending.is_empty());
}
#[test]
fn display_covers_all_download_and_export_stages() {
    let attachment = file(Some(12 * 1024 * 1024));
    let view = |cached, download, external, saving, err| {
        AttachmentDisplay::new(&attachment, cached, download, external, saving, err)
    };
    assert_eq!(
        view(false, None, false, false, None).status,
        "12 MB"
    );
    assert_eq!(
        view(false, None, false, false, None).control,
        Control::Download
    );
    let active = download(3 * 1024 * 1024, 12 * 1024 * 1024, false, None);
    let display = view(false, Some(&active), false, false, None);
    assert_eq!(display.status, "3.0 MB / 12 MB · 25%");
    assert_eq!(display.control, Control::Cancel);
    assert_eq!(display.progress, Some(Progress::Known(0.25)));
    let unknown = download(2048, 0, false, None);
    let display =
        AttachmentDisplay::new(&file(None), false, Some(&unknown), false, false, None);
    assert_eq!(display.status, "2.0 KB · Downloading…");
    assert_eq!(display.progress, Some(Progress::Unknown));
    let failed = download(512, 1024, true, Some("Download cancelled"));
    let display = view(false, Some(&failed), false, false, None);
    assert_eq!(display.control, Control::Retry);
    assert!(display.failed);
    assert_eq!(display.status, "512 B / 1.0 KB · Download cancelled");
    assert_eq!(
        view(true, Some(&failed), false, false, None).control,
        Control::Save
    );
    assert_eq!(
        view(true, None, false, true, None).status,
        "12 MB · Saving…"
    );
    assert_eq!(view(true, None, false, true, None).control, Control::Busy);
    assert_eq!(
        view(true, None, false, false, Some("Disk full")).control,
        Control::Retry
    );
    assert_eq!(
        view(false, None, true, false, None).status,
        "12 MB · Downloaded"
    );
    assert_eq!(view(false, None, true, false, None).control, Control::Open);
}
