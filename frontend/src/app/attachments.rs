use super::*;
#[cfg(test)]
use tau_transfer::TransferStatus;

// Match Tau 1's byte labels without overflowing on large advertised sizes.
pub(in crate::app) fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut unit = 1024_u128;
    let mut index = 0;
    let labels = ["KB", "MB", "GB", "TB", "PB", "EB"];
    while index < labels.len() - 1 && u128::from(bytes) >= unit * 1024 {
        unit *= 1024;
        index += 1;
    }
    let tenths = (u128::from(bytes) * 10 + unit / 2) / unit;
    if tenths < 100 {
        format!("{}.{} {}", tenths / 10, tenths % 10, labels[index])
    } else {
        format!("{} {}", (tenths + 5) / 10, labels[index])
    }
}

#[derive(Debug, PartialEq)]
pub(in crate::app) enum Progress {
    Known(f32),
    Unknown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum Control {
    Download,
    Save,
    View,
    Open,
    Cancel,
    Retry,
    Busy,
}
impl Control {
    pub(in crate::app) fn label(self) -> &'static str {
        match self {
            Self::Download => "Download",
            Self::Save => "Save",
            Self::View => "View",
            Self::Open => "Open",
            Self::Cancel => "Cancel",
            Self::Retry => "Retry",
            Self::Busy => "Saving…",
        }
    }
    pub(in crate::app) fn description(self) -> &'static str {
        match self {
            Self::Save => "Save to Downloads",
            Self::View => "View image",
            Self::Open => "Open file",
            Self::Cancel => "Cancel download",
            Self::Busy => "Saving to Downloads…",
            _ => self.label(),
        }
    }
}

pub(in crate::app) struct AttachmentDisplay {
    pub(in crate::app) status: String,
    pub(in crate::app) control: Control,
    pub(in crate::app) progress: Option<Progress>,
    pub(in crate::app) failed: bool,
}
impl AttachmentDisplay {
    pub(in crate::app) fn new(
        file: &ChatAttachment,
        cached: bool,
        download: Option<&crate::controller::Download>,
        external: bool,
        saving: bool,
        save_failure: Option<&str>,
    ) -> Self {
        let status = download.map(|d| &d.status);
        let total = status
            .and_then(|s| (s.total > 0).then_some(s.total))
            .or(file.size);
        let label = total
            .map(|n| format!("{} · ", format_bytes(n)))
            .unwrap_or_default();
        if external {
            return Self {
                status: format!("{label}Downloaded"),
                control: Control::Open,
                progress: None,
                failed: false,
            };
        }
        if saving {
            return Self {
                status: format!("{label}Saving…"),
                control: Control::Busy,
                progress: Some(Progress::Unknown),
                failed: false,
            };
        }
        if let Some(failure) = save_failure {
            return Self {
                status: format!("{label}{failure}"),
                control: Control::Retry,
                progress: None,
                failed: true,
            };
        }
        if cached {
            return Self {
                status: format!("{label}{}", if file.kind == AttachmentKind::Image { "Preview ready" } else { "Ready to save" }),
                control: if file.kind == AttachmentKind::Image {
                    Control::View
                } else {
                    Control::Save
                },
                progress: None,
                failed: false,
            };
        }
        let Some(status) = status else {
            return Self {
                status: if label.is_empty() {
                    "Ready to download".into()
                } else {
                    total.map(format_bytes).unwrap()
                },
                control: Control::Download,
                progress: None,
                failed: false,
            };
        };
        let bytes = total.map_or_else(
            || format_bytes(status.transferred),
            |n| format!("{} / {}", format_bytes(status.transferred), format_bytes(n)),
        );
        if status.done {
            return Self {
                status: format!(
                    "{bytes} · {}",
                    status
                        .failure
                        .as_deref()
                        .unwrap_or("File unavailable; retry")
                ),
                control: Control::Retry,
                progress: None,
                failed: true,
            };
        }
        let progress = total.filter(|n| *n > 0).map_or(Progress::Unknown, |n| {
            Progress::Known((status.transferred as f64 / n as f64).clamp(0., 1.) as f32)
        });
        let percentage = total.filter(|n| *n > 0).map(|n| {
            format!(
                " · {}%",
                (u128::from(status.transferred) * 100 / u128::from(n)).min(100)
            )
        });
        let rate = download
            .and_then(|d| d.bytes_per_second)
            .filter(|n| *n > 0)
            .map(|n| format!(" · {}/s", format_bytes(n)))
            .unwrap_or_default();
        Self {
            status: format!(
                "{bytes}{}{}",
                percentage.unwrap_or_else(|| " · Downloading…".into()),
                rate
            ),
            control: Control::Cancel,
            progress: Some(progress),
            failed: false,
        }
    }
}

/// Includes the same padding in transcript and Attachments; no empty caption row.
pub(super) fn card_height(file: &ChatAttachment) -> f32 {
    84. + if file.caption.as_ref().is_some_and(|s| !s.is_empty()) { 24. } else { 0. }
        + if file.kind == AttachmentKind::Image { 224. } else { 0. }
}

/// Shared by painting and attachment navigation, so changing the widget's
/// geometry cannot leave notification clicks targeting an unrelated row offset.
pub(super) fn control_panel(rect: Rect, scale: f32) -> Rect {
    Rect::new(rect.x + 14. * scale, rect.y + rect.height - 76. * scale,
        (rect.width - 28. * scale).max(1.), 68. * scale)
}

/// File saves are bound to the exact cache path of the requested transfer.
/// An old completion, a failed transfer or a changed account cannot start an export.
fn completed_exports(
    pending: &mut HashMap<String, (PathBuf, String)>,
    downloads: &HashMap<String, crate::controller::Download>,
) -> Vec<PlatformAction> {
    let mut actions = Vec::new();
    pending.retain(|key, (path, name)| {
        let Some(download) = downloads.get(key) else {
            return false;
        };
        if download.path != *path {
            return false;
        }
        if !download.status.done {
            return true;
        }
        if download.status.failure.is_none() && path.is_file() {
            actions.push(PlatformAction::SaveDownload {
                key: key.clone(),
                source: path.clone(),
                name: name.clone(),
            });
        }
        false // Exactly once; cancel/failure never opens a picker.
    });
    actions
}
pub(in crate::app) struct Transfers {
    pub(in crate::app) pending_exports: HashMap<String, (PathBuf, String)>,
    pub(in crate::app) export_targets: HashMap<String, DownloadTarget>,
    pub(in crate::app) saving_downloads: HashSet<String>,
    pub(in crate::app) export_errors: HashMap<String, String>,
    pub(in crate::app) download_identity: String,
    pub(in crate::app) progress_clock: Instant,
    pub(in crate::app) progress_bucket: Option<u128>,
}
impl ui::Context<'_> {
    /// A file button starts a verified cache transfer, then saves to Downloads/Tau
    /// on completion. Image previews are cache-only until Save is explicitly pressed.
    pub(in crate::app) fn finish_exports(&mut self) {
        let actions = completed_exports(&mut self.services.transfers.pending_exports, &self.model.downloads);
        for action in actions {
            if let PlatformAction::SaveDownload { key, .. } = &action {
                if self.services.transfers.export_targets.get(key).is_some_and(|target|
                    target.matches_source(&self.model.identity, self.model.account.source_lineage.as_deref())) {
                    self.services.transfers.saving_downloads.insert(key.clone());
                    self.services.platform.push(action);
                } else {
                    self.services.transfers.export_targets.remove(key);
                }
            }
        }
        self.services.transfers.export_targets.retain(|key, _| {
            self.services.transfers.pending_exports.contains_key(key) || self.services.transfers.saving_downloads.contains(key)
        });
    }
    pub(in crate::app) fn export_target(&self, session: &str, entry: &str) -> DownloadTarget {
        DownloadTarget {
            identity: self.model.identity.clone(),
            lineage: self
                .model
                .account
                .source_lineage
                .clone()
                .unwrap_or_default(),
            session: session.into(),
            entry: entry.into(),
        }
    }
    pub(in crate::app) fn begin_save(&mut self, session: &str, entry: &str, path: PathBuf, name: String) {
        let key = Controller::download_key(session, entry);
        if !self.services.transfers.saving_downloads.insert(key.clone()) {
            return;
        }
        self.services.transfers.export_targets
            .insert(key.clone(), self.export_target(session, entry));
        self.services.transfers.export_errors.remove(&key);
        self.services.platform.push(PlatformAction::SaveDownload {
            key,
            source: path,
            name,
        });
    }
    pub fn complete_save(
        &mut self,
        key: &str,
        result: Result<crate::store::SavedDownload, String>,
    ) {
        self.services.transfers.saving_downloads.remove(key);
        let Some(target) = self.services.transfers.export_targets.remove(key) else {
            return;
        };
        let result = result.and_then(|saved| {
            self.model
                .record_download(
                    &target.identity,
                    &target.lineage,
                    &target.session,
                    &target.entry,
                    saved.clone(),
                )
                .map_err(|e| {
                    format!("Saved to {} but could not remember it: {e}", saved.location)
                })?;
            Ok(saved)
        });
        match result {
            Ok(saved) => {
                self.services.transfers.export_errors.remove(key);
                if target.matches_source(&self.model.identity, self.model.account.source_lineage.as_deref()) {
                    self.model.notice = Some(crate::notice::Notice::download(format!("Saved to {}", saved.location), target));
                }
            }
            Err(error) => {
                if target.matches_source(&self.model.identity, self.model.account.source_lineage.as_deref()) {
                    self.services.transfers.export_errors.insert(key.into(), error.clone());
                    self.model.notice = Some(error.into());
                }
            }
        }
        self.ui.dirty = true;
    }
}
impl App {
    pub(super) fn attachment_card(&mut self, _gpu: &impl RenderContext, layer: &mut Layer, session: &str, entry: &str, file: &ChatAttachment, surface: &'static str, rect: Rect, clip: Rect) {
        self.with_ui(|root,cx| root.legacy.cards.paint(session,entry,file,surface,&mut ui::Frame { layer,bounds:rect,clip },cx));
    }


}


impl App {
    pub(super) fn finish_exports(&mut self) { self.with_ui(|_, cx| cx.finish_exports()); }
    pub(super) fn export_target(&self, session: &str, entry: &str) -> DownloadTarget {
        DownloadTarget { identity: self.controller.identity.clone(), lineage: self.controller.account.source_lineage.clone().unwrap_or_default(), session: session.into(), entry: entry.into() }
    }
    pub(super) fn begin_save(&mut self, session: &str, entry: &str, path: PathBuf, name: String) { self.with_ui(|_, cx| cx.begin_save(session, entry, path, name)); }
    pub fn complete_save(&mut self, key: &str, result: Result<crate::store::SavedDownload, String>) { self.with_ui(|_, cx| cx.complete_save(key, result)); }
}

#[cfg(test)]
mod tests {
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
    fn incomplete_attachment_shows_its_card_without_a_loading_body() {
        let ctx = chad::HeadlessCtx::new(&chad::Config {
            size: (420, 780),
            device_limits: crate::desktop::limits(),
            ..Default::default()
        })
        .unwrap();
        let root = tempfile::tempdir().unwrap();
        let mut app = App::new(
            &ctx,
            Store::open(root.path().join("local")).unwrap(),
            std::sync::Arc::new(|| {}),
            false,
        )
        .unwrap();
        app.back();
        crate::demo::populate(&mut app.controller).unwrap();
        let feed = &mut app.controller.chats.get_mut("demo").unwrap().feed;
        let event = feed.events.get_mut(&0).unwrap();
        event.text = "Loading…".into();
        event.attachment = Some(file(Some(1024)));
        feed.incomplete.insert(event.id.clone());
        let row = app
            .rows("demo")
            .into_iter()
            .find(|row| row.key == "demo/event-0")
            .unwrap();
        assert!(row.source.is_empty());
        assert!(row.attachment.is_some());
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
}

impl ui::Context<'_> {
    pub(in crate::app) fn download_attachment(&mut self, session: &str, entry: &str, name: &str, image: bool, save: bool) -> Result<()> {
        let key = Controller::download_key(session, entry);
        let path = match self.model.download(session, entry, if image { 10_000_000 } else { 50_000_000 }) {
            Ok(path) => path,
            Err(error) => { self.services.transfers.export_errors.insert(key, error.to_string()); return Err(error); }
        };
        let in_progress = self.model.downloads.get(&key).is_some_and(|d| !d.status.done);
        if path.is_file() && !in_progress {
            if image && !save {
                self.ui.requests.push_back(ui::Request::View(ui::ImageSpec { path, name: name.into(), target: self.export_target(session,entry) }));
            } else { self.begin_save(session,entry,path,name.into()); }
        } else if !image || save {
            self.services.transfers.export_targets.insert(key.clone(), self.export_target(session,entry));
            self.services.transfers.export_errors.remove(&key);
            self.services.transfers.pending_exports.insert(key,(path,name.into()));
        }
        Ok(())
    }
}
