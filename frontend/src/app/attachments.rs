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
            Self::Download => "↓ Download",
            Self::Save => "↓ Save",
            Self::View => "↗ View",
            Self::Open => "↗ Open",
            Self::Cancel => "× Cancel",
            Self::Retry => "Retry",
            Self::Busy => "",
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
                status: format!("{label}Saving to Downloads/Tau…"),
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
                status: format!("{label}Saved in Tau"),
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
                    format!("{label}Ready to download")
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
impl App {
    /// A file button starts a verified cache transfer, then saves to Downloads/Tau
    /// on completion. Image previews are cache-only until Save is explicitly pressed.
    pub(super) fn finish_exports(&mut self) {
        let actions = completed_exports(&mut self.pending_exports, &self.controller.downloads);
        for action in actions {
            if let PlatformAction::SaveDownload { key, .. } = &action {
                if self.export_targets.get(key).is_some_and(|t| {
                    t.identity == self.controller.identity
                        && t.lineage
                            == self
                                .controller
                                .account
                                .source_lineage
                                .as_deref()
                                .unwrap_or_default()
                }) {
                    self.saving_downloads.insert(key.clone());
                    self.platform.push(action);
                } else {
                    self.export_targets.remove(key);
                }
            }
        }
        self.export_targets.retain(|key, _| {
            self.pending_exports.contains_key(key) || self.saving_downloads.contains(key)
        });
    }
    pub(super) fn export_target(&self, session: &str, entry: &str) -> ExportTarget {
        ExportTarget {
            identity: self.controller.identity.clone(),
            lineage: self
                .controller
                .account
                .source_lineage
                .clone()
                .unwrap_or_default(),
            session: session.into(),
            entry: entry.into(),
        }
    }
    pub(super) fn begin_save(&mut self, session: &str, entry: &str, path: PathBuf, name: String) {
        let key = Controller::download_key(session, entry);
        if !self.saving_downloads.insert(key.clone()) {
            return;
        }
        self.export_targets
            .insert(key.clone(), self.export_target(session, entry));
        self.export_errors.remove(&key);
        self.platform.push(PlatformAction::SaveDownload {
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
        self.saving_downloads.remove(key);
        let Some(target) = self.export_targets.remove(key) else {
            return;
        };
        let result = result.and_then(|saved| {
            self.controller
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
                self.export_errors.remove(key);
                if target.identity == self.controller.identity
                    && target.lineage
                        == self
                            .controller
                            .account
                            .source_lineage
                            .as_deref()
                            .unwrap_or_default()
                {
                    self.controller.notice = Some(format!("Saved to {}", saved.location));
                }
            }
            Err(error) => {
                if target.identity == self.controller.identity
                    && target.lineage
                        == self
                            .controller
                            .account
                            .source_lineage
                            .as_deref()
                            .unwrap_or_default()
                {
                    self.export_errors.insert(key.into(), error.clone());
                    self.controller.notice = Some(error);
                }
            }
        }
        self.dirty = true;
    }
    pub(super) fn attachment_card(
        &mut self,
        ctx: &impl RenderContext,
        layer: &mut Layer,
        session: &str,
        entry: &str,
        attachment: &ChatAttachment,
        rect: Rect,
        viewport: Rect,
    ) {
        let s = self.scale;
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
                self.controller.notice = Some(error.to_string());
            }
            None
        } else {
            exported
        };
        let x = rect.x + 14. * s;
        let width = rect.width - 28. * s;
        let bottom = rect.y + rect.height;
        let control_top = bottom - 104. * s;
        if image {
            let preview = Rect::new(x, bottom - 332. * s, width, 216. * s);
            let clip = crate::render::intersect(preview, viewport);
            if clip.width > 0. && clip.height > 0. {
                layer.clipped_rounded_rect(preview, 7. * s, color(0x111a23), viewport);
                if cached && let Ok((w, h)) = self.renderer.image_size(ctx, &path) {
                    let fit = (width / w as f32).min(216. * s / h as f32);
                    let image_rect = Rect::new(x, preview.y, w as f32 * fit, h as f32 * fit);
                    layer.images.push((path.clone(), image_rect, clip));
                    self.hits.push(Hit {
                        rect: crate::render::intersect(image_rect, clip),
                        action: Action::Attachment(
                            session.into(),
                            entry.into(),
                            attachment.file_name.clone(),
                            true,
                        ),
                    });
                } else if !cached {
                    let label = if self
                        .controller
                        .downloads
                        .get(&key)
                        .is_some_and(|d| d.status.failure.is_some())
                    {
                        "Preview unavailable"
                    } else {
                        "Fetching preview"
                    };
                    self.renderer.clipped_label(
                        layer,
                        label,
                        Rect::new(x + 12. * s, preview.y + 12. * s, width - 24. * s, 20. * s),
                        12. * s,
                        color(0x82909f),
                        false,
                        clip,
                    );
                }
            }
            if !cached
                && self.controller.content_authorized()
                && !self.controller.downloads.contains_key(&key)
                && let Err(error) = self.controller.download(session, entry, 10_000_000)
            {
                self.controller.notice = Some(error.to_string());
            }
        }
        let display = AttachmentDisplay::new(
            attachment,
            cached,
            self.controller.downloads.get(&key),
            exported.is_some(),
            self.saving_downloads.contains(&key),
            self.export_errors.get(&key).map(String::as_str),
        );
        let label_width = width
            - if exported.is_some() {
                145. * s
            } else {
                95. * s
            };
        let title = Rect::new(x, control_top + 10. * s, label_width.max(1.), 20. * s);
        self.renderer.clipped_label(
            layer,
            &attachment.file_name,
            title,
            13. * s,
            color(0xe5eaf0),
            true,
            crate::render::intersect(title, viewport),
        );
        if let Some(caption) = &attachment.caption {
            let area = Rect::new(x, control_top + 34. * s, width, 27. * s);
            self.renderer.clipped_label(
                layer,
                caption,
                area,
                12. * s,
                color(0xb7c2ce),
                false,
                crate::render::intersect(area, viewport),
            );
        }
        let status_area = Rect::new(x, control_top + 69. * s, width, 18. * s);
        self.renderer.clipped_label(
            layer,
            &display.status,
            status_area,
            11. * s,
            color(if display.failed { 0xffb4ab } else { 0x9eaebd }),
            false,
            crate::render::intersect(status_area, viewport),
        );
        if let Some(progress) = display.progress {
            let track = Rect::new(x, bottom - 12. * s, width, 4. * s);
            layer.clipped_rounded_rect(track, 2. * s, color(0x30404f), viewport);
            let fill = match progress {
                Progress::Known(fraction) => Rect::new(x, track.y, width * fraction, track.height),
                Progress::Unknown => Rect::new(
                    x + width * (self.progress_clock.elapsed().as_secs_f32() * 0.45).fract() * 0.75,
                    track.y,
                    width * 0.25,
                    track.height,
                ),
            };
            layer.clipped_rounded_rect(fill, 2. * s, color(0x67d4ff), viewport);
        }
        let action = match display.control {
            Control::Download | Control::Retry
                if image && self.export_errors.contains_key(&key) =>
            {
                Some(Action::SaveAttachment(
                    session.into(),
                    entry.into(),
                    attachment.file_name.clone(),
                ))
            }
            Control::Download | Control::Retry | Control::View => Some(Action::Attachment(
                session.into(),
                entry.into(),
                attachment.file_name.clone(),
                image,
            )),
            Control::Save => Some(Action::SaveAttachment(
                session.into(),
                entry.into(),
                attachment.file_name.clone(),
            )),
            Control::Open => Some(Action::UseSaved(
                session.into(),
                entry.into(),
                SavedAction::Open,
            )),
            Control::Cancel => Some(Action::CancelDownload(key.clone())),
            Control::Busy => None,
        };
        let mut actions = Vec::new();
        if let Some(action) = action {
            actions.push((display.control.label(), action));
        }
        if exported.is_some() {
            #[cfg(not(target_os = "android"))]
            {
                actions.push((
                    "▣",
                    Action::UseSaved(session.into(), entry.into(), SavedAction::Show),
                ));
                if attachment.file_name.to_ascii_lowercase().ends_with(".zip") {
                    actions.push((
                        "⇣",
                        Action::UseSaved(session.into(), entry.into(), SavedAction::Extract),
                    ));
                }
            }
            if image && cached {
                actions.push((
                    "↗ View",
                    Action::Attachment(
                        session.into(),
                        entry.into(),
                        attachment.file_name.clone(),
                        true,
                    ),
                ));
            }
        } else if image && cached && !self.saving_downloads.contains(&key) {
            actions.push((
                "↓ Save",
                Action::SaveAttachment(session.into(), entry.into(), attachment.file_name.clone()),
            ));
        }
        let mut right = rect.x + rect.width - 10. * s;
        for (label, action) in actions {
            let button_width = (label.chars().count() as f32 * 7. + 22.) * s;
            right -= button_width;
            let r = crate::render::intersect(
                Rect::new(right, control_top + 7. * s, button_width, 27. * s),
                viewport,
            );
            if r.width > 0. && r.height > 0. {
                button(
                    &mut self.renderer,
                    layer,
                    &mut self.hits,
                    r,
                    label,
                    action,
                    s,
                    false,
                );
            }
            right -= 5. * s;
        }
    }
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
            "12 MB · Ready to download"
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
            "12 MB · Saving to Downloads/Tau…"
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
