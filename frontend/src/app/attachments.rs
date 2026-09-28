use super::*;
#[cfg(test)]
use tau_transfer::TransferStatus;

// Match Tau 1's byte labels without overflowing on large advertised sizes.
fn format_bytes(bytes: u64) -> String {
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
enum Progress {
    Known(f32),
    Unknown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Control {
    Download,
    Save,
    View,
    Open,
    Cancel,
    Retry,
    Busy,
}
impl Control {
    fn label(self) -> &'static str {
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
    fn description(self) -> &'static str {
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

struct AttachmentDisplay {
    status: String,
    control: Control,
    progress: Option<Progress>,
    failed: bool,
}
impl AttachmentDisplay {
    fn new(
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
    pub(super) fn attachment_card(
        &mut self,
        ctx: &impl RenderContext,
        layer: &mut Layer,
        session: &str,
        entry: &str,
        attachment: &ChatAttachment,
        surface: &'static str,
        rect: Rect,
        viewport: Rect,
    ) {
        let s = self.ui.scale;
        let image = attachment.kind == AttachmentKind::Image;
        let path = self.controller.attachment_path(session, entry);
        let key = Controller::download_key(session, entry);
        let cached = path.is_file()
            && self
                .controller
                .downloads
                .get(&key)
                .is_none_or(|d| d.status.done && d.status.failure.is_none());
        let exported = self.controller.saved_download(session, entry);
        #[cfg(not(target_os = "android"))]
        let exported = if exported
            .as_ref()
            .is_some_and(|saved| !std::path::Path::new(&saved.reference).is_file())
        {
            if let Err(error) = self.controller.forget_download(session, entry) {
                self.controller.notice = Some(error.to_string().into());
            }
            None
        } else {
            exported
        };
        // The same file can be visible in chat and the sidebar simultaneously.
        let info_key = format!("{surface}:{key}");
        let x = rect.x + 14. * s;
        let width = (rect.width - 28. * s).max(1.);
        let panel = control_panel(rect, s);
        let caption = attachment.caption.as_deref().filter(|text| !text.is_empty());
        let mut preview_available = false;
        if image {
            let preview = Rect::new(x, panel.y - (224. + if caption.is_some() { 24. } else { 0. }) * s,
                width, 216. * s);
            let clip = crate::render::intersect(preview, viewport);
            layer.clipped_rounded_rect(preview, 8. * s, color(0x101820), viewport);
            if cached && let Ok((w, h)) = self.services.renderer.image_size(ctx, &path) {
                preview_available = true;
                let fit = (width / w as f32).min(preview.height / h as f32);
                let image_rect = Rect::new(x + (width - w as f32 * fit) / 2.,
                    preview.y + (preview.height - h as f32 * fit) / 2., w as f32 * fit, h as f32 * fit);
                layer.images.push((path.clone(), image_rect, clip));
                if clip.width > 0. && clip.height > 0. {
                    self.root.legacy.hits.push(Hit { rect: clip, action: Action::Attachment(session.into(), entry.into(),
                        attachment.file_name.clone(), true) });
                }
            } else {
                let download = self.controller.downloads.get(&key);
                let label = if cached { "Preview unavailable · save original" }
                    else if download.is_some_and(|d| d.status.failure.is_some()) { "Preview unavailable" }
                    else if download.is_some_and(|d| !d.status.done) { "Loading preview…" }
                    else { "Download to preview" };
                self.services.renderer.clipped_icon(ctx, layer, Icon::Image,
                    Rect::new(x + width / 2. - 16. * s, preview.y + 74. * s, 32. * s, 32. * s), 0x687e8f, clip);
                // Center a measured one-line placeholder, rather than an off-center Loading label.
                let label_width = self.services.renderer.label_width(label, 12. * s, false).min(width - 16. * s);
                self.services.renderer.ellipsized_label(layer, label,
                    Rect::new(x + (width - label_width) / 2., preview.y + 118. * s, label_width, 20. * s),
                    12. * s, color(0x9eaebd), false, false, clip);
            }
            if !cached && self.controller.content_authorized() && !self.controller.downloads.contains_key(&key)
                && let Err(error) = self.controller.download(session, entry, 10_000_000) {
                self.controller.notice = Some(error.to_string().into());
            }
        }
        let mut display = AttachmentDisplay::new(attachment, cached, self.controller.downloads.get(&key),
            exported.is_some(), self.services.transfers.saving_downloads.contains(&key), self.services.transfers.export_errors.get(&key).map(String::as_str));
        if display.control == Control::View && !preview_available {
            display.status = format!("{}Preview unavailable", attachment.size
                .map(|n| format!("{} · ", format_bytes(n))).unwrap_or_default());
            display.failed = true;
        }
        let control = if display.control == Control::View && !preview_available { Control::Save } else { display.control };
        let action = match control {
            Control::Download | Control::Retry if image && self.services.transfers.export_errors.contains_key(&key) =>
                Some(Action::SaveAttachment(session.into(), entry.into(), attachment.file_name.clone())),
            Control::Download | Control::Retry | Control::View => Some(Action::Attachment(
                session.into(), entry.into(), attachment.file_name.clone(), image)),
            Control::Save => Some(Action::SaveAttachment(session.into(), entry.into(), attachment.file_name.clone())),
            Control::Open => Some(Action::UseSaved(session.into(), entry.into(), SavedAction::Open)),
            Control::Cancel => Some(Action::CancelDownload(key.clone())),
            Control::Busy => None,
        };
        let mut actions = vec![(control.label(), control.description(), action)];
        if exported.is_some() {
            if image && preview_available {
                actions.push(("View", "View image", Some(Action::Attachment(session.into(), entry.into(),
                    attachment.file_name.clone(), true))));
            }
            #[cfg(not(target_os = "android"))]
            if !self.ui.mobile {
                actions.push(("Show", "Show in folder", Some(Action::UseSaved(session.into(), entry.into(), SavedAction::Show))));
                if attachment.file_name.to_ascii_lowercase().ends_with(".zip") {
                    actions.push(("Extract", "Extract ZIP and open folder", Some(Action::UseSaved(session.into(), entry.into(), SavedAction::Extract))));
                }
            }
        } else if image && preview_available && control == Control::View {
            actions.push(("Save", "Save to Downloads", Some(Action::SaveAttachment(session.into(), entry.into(), attachment.file_name.clone()))));
        } else if image && preview_available && control == Control::Retry {
            // Retrying an OS save must not hide or discard a perfectly good preview.
            actions.push(("View", "View image", Some(Action::Attachment(session.into(), entry.into(), attachment.file_name.clone(), true))));
        }
        layer.clipped_rounded_rect(panel, 10. * s, color(0x101820), viewport);
        let target = if self.ui.mobile { 44. } else { 40. } * s;
        // Tau1 uses text for these actions. Measure each complete label instead
        // of reserving icon-sized slots or guessing widths from character counts.
        let widths = actions.iter().map(|(label, _, _)|
            (self.services.renderer.label_width(label, 14. * s, false).ceil() + 16. * s).max(target))
            .collect::<Vec<_>>();
        let actions_width = widths.iter().sum::<f32>();
        let text = Rect::new(panel.x + 12. * s, panel.y + 12. * s,
            (panel.width - actions_width - 20. * s).max(1.), 20. * s);
        self.services.renderer.ellipsized_label(layer, &attachment.file_name, text, 13. * s, color(0xe5eaf0), true, true, viewport);
        let status = Rect::new(text.x, panel.y + 34. * s, text.width, 18. * s);
        self.services.renderer.ellipsized_label(layer, &display.status, status, 11. * s,
            color(if display.failed { 0xffb4ab } else if exported.is_some() { 0x93cbb4 } else { 0x9eaebd }),
            false, false, viewport);
        if let Some(caption) = caption {
            let area = Rect::new(x, panel.y - 24. * s, width, 20. * s);
            self.services.renderer.ellipsized_label(layer, caption, area, 12. * s, color(0xb7c2ce), false, false, viewport);
        }
        if let Some(progress) = display.progress {
            let track = Rect::new(panel.x + 12. * s, panel.y + panel.height - 7. * s, panel.width - 24. * s, 3. * s);
            layer.clipped_rounded_rect(track, 1.5 * s, color(0x263947), viewport);
            let fill = match progress {
                Progress::Known(fraction) => Rect::new(track.x, track.y, track.width * fraction, track.height),
                Progress::Unknown => Rect::new(track.x + track.width *
                    (self.services.transfers.progress_clock.elapsed().as_secs_f32() * 0.45).fract() * 0.75,
                    track.y, track.width * 0.25, track.height),
            };
            layer.clipped_rounded_rect(fill, 1.5 * s, color(0x67d4ff), viewport);
        }
        let mut left = panel.x + panel.width - actions_width - 4. * s;
        for (index, ((label, description, action), width)) in actions.into_iter().zip(widths).enumerate() {
            let r = Rect::new(left, panel.y + (60. * s - target) / 2., width, target);
            left += width;
            let clip = crate::render::intersect(r, viewport);
            if clip.width <= 0. || clip.height <= 0. { continue; }
            let enabled = action.is_some();
            if enabled && layer.interaction.hover.is_some_and(|p| contains(clip, p)) {
                layer.clipped_rounded_rect(r, target / 2., layer.control_color(r, color(0x18212b)), viewport);
            }
            let label_width = self.services.renderer.label_width(label, 14. * s, false);
            let label_height = self.services.renderer.label_height(label, width, 14. * s, false);
            self.services.renderer.ellipsized_label(layer, label,
                Rect::new(r.x + (width - label_width) / 2., r.y + (target - label_height) / 2.,
                    label_width.ceil() + 1., label_height),
                14. * s, color(if enabled { 0x67d4ff } else { 0x687e8f }), false, false, viewport);
            self.attachment_info(r, viewport, format!("{info_key}:action:{index}"), description, &attachment.file_name);
            self.root.legacy.hits.push(Hit { rect: clip, action: action.unwrap_or(Action::Noop) });
        }
    }
    fn attachment_info(&mut self, rect: Rect, viewport: Rect, key: String, title: &str, detail: &str) {
        let rect = crate::render::intersect(rect, viewport);
        if rect.width > 0. && rect.height > 0. {
            let info = Info::Attachment(key, title.into(), detail.into());
            self.root.legacy.info_areas.push((rect, info));
        }

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
