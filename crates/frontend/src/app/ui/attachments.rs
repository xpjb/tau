use super::controls::{ButtonStyle, Controls};
use super::{Context, Event, Frame, Id, Request, Target, UiState, Widget};
use crate::{
    app::{
        Info, PlatformAction, SavedAction,
        attachments::{AttachmentDisplay, Control, Progress, card_height, control_panel, format_bytes},
    },
    controller::Controller,
    icons::Icon,
    notice::DownloadTarget,
    render::{color, contains},
};
use sanscale::Rect;
use std::collections::{BTreeSet, HashMap};
use tau_net::*;

// Destinations belong to the mounted card, not to a second per-button route.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum CardChoice {
    Acquire,
    Save,
    UseSaved(SavedAction),
    Cancel,
    Noop,
}
pub(in crate::app) struct AttachmentCard {
    pub target: DownloadTarget,
    pub file: ChatAttachment,
    surface: &'static str,
    pub controls: Controls<CardChoice>,
}
impl AttachmentCard {
    pub fn new(session: &str, entry: &str, file: &ChatAttachment, surface: &'static str, cx: &mut Context<'_>) -> Self {
        Self {
            target: cx.export_target(session, entry),
            file: file.clone(),
            surface,
            controls: Controls::new(Id::new()),
        }
    }
    pub fn bind_file(&mut self, file: &ChatAttachment, cx: &mut Context<'_>) {
        if self.file != *file {
            cx.ui.detach(self.controls.id);
            self.controls = Controls::new(Id::new());
            self.file = file.clone();
        }
    }
    pub fn hide(&mut self) {
        self.controls.begin();
    }
}
impl Widget for AttachmentCard {
    fn owns(&self, target: Target, model: &Controller, _ui: &UiState) -> bool {
        self.controls.owns(target)
            && model.account.selected.as_ref() == Some(&self.target.session)
            && self.target.matches_source(&model.identity, model.account.source_lineage.as_deref())
            && model.selected().is_some_and(|chat| {
                chat.feed
                    .events
                    .values()
                    .any(|e| e.entry_id == self.target.entry && e.attachment.as_ref() == Some(&self.file))
            })
    }

    fn handle_event(&mut self, event: &Event<'_>, cx: &mut Context<'_>) -> bool {
        if !self.target.matches_source(&cx.model.identity, cx.model.account.source_lineage.as_deref())
            || Some(&self.target.session) != cx.model.account.selected.as_ref()
        {
            return false;
        }
        if let Event::Context(point) = *event
            && let Some((_, button, _)) = self.controls.items.iter().find(|(_, b, _)| b.control.contains(point))
        {
            if let Some(info) = &button.control.info {
                cx.ui.requests.push_back(Request::Tip { info: info.clone(), rect: button.control.rect.unwrap() });
            }
            return true;
        }
        let (handled, choice) = self.controls.event(event, cx);
        let session = &self.target.session;
        let entry = &self.target.entry;
        let result = match choice {
            Some(CardChoice::Acquire | CardChoice::Save) => cx.download_attachment(
                session,
                entry,
                &self.file.file_name,
                self.file.kind == AttachmentKind::Image,
                choice == Some(CardChoice::Save),
            ),
            Some(CardChoice::UseSaved(action)) => {
                if let Some(saved) = cx.model.saved_download(session, entry) {
                    let target = self.target.clone();
                    // Claim before queueing, not on repaint or in the OS worker:
                    // another click/card can arrive before either of those runs.
                    #[cfg(not(target_os = "android"))]
                    if matches!(action, SavedAction::Extract)
                        && !cx.services.transfers.extracting_downloads.insert(target.clone())
                    {
                        return handled;
                    }
                    cx.services.platform.push(PlatformAction::UseDownload(saved, action, target));
                }
                Ok(())
            }
            Some(CardChoice::Cancel) => {
                let key = Controller::download_key(session, entry);
                cx.services.transfers.pending_exports.remove(&key);
                cx.services.transfers.export_targets.remove(&key);
                cx.model.cancel_download(&key)
            }
            _ => return handled,
        };
        cx.report(result);
        handled
    }
    fn visit_perframe(&mut self, frame: &mut Frame<'_>, cx: &mut Context<'_>) {
        let s = cx.ui.scale;
        self.controls.begin();
        let session = self.target.session.clone();
        let session = session.as_str();
        let entry = self.target.entry.clone();
        let entry = entry.as_str();
        let attachment = self.file.clone();
        let attachment = &attachment;
        let surface = self.surface;
        let rect = frame.bounds;
        let viewport = frame.clip;
        let layer = &mut *frame.layer;
        let ctx = &cx.services.gpu;
        let image = attachment.kind == AttachmentKind::Image;
        let path = cx.model.attachment_path(session, entry);
        let key = Controller::download_key(session, entry);
        let cached =
            path.is_file() && cx.model.downloads.get(&key).is_none_or(|d| d.status.done && d.status.failure.is_none());
        let exported = cx.model.saved_download(session, entry);
        #[cfg(not(target_os = "android"))]
        let exported = if exported.as_ref().is_some_and(|saved| !std::path::Path::new(&saved.reference).is_file()) {
            if let Err(error) = cx.model.forget_download(session, entry) {
                cx.model.notice = Some(error.to_string().into());
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
            let preview =
                Rect::new(x, panel.y - (224. + if caption.is_some() { 24. } else { 0. }) * s, width, 216. * s);
            let clip = crate::render::intersect(preview, viewport);
            layer.clipped_rounded_rect(preview, 8. * s, color(0x101820), viewport);
            if cached && let Ok((w, h)) = cx.services.renderer.image_size(ctx, &path) {
                preview_available = true;
                let fit = (width / w as f32).min(preview.height / h as f32);
                let image_rect = Rect::new(
                    x + (width - w as f32 * fit) / 2.,
                    preview.y + (preview.height - h as f32 * fit) / 2.,
                    w as f32 * fit,
                    h as f32 * fit,
                );
                layer.images.push((path.clone(), image_rect, clip));
                if clip.width > 0. && clip.height > 0. {
                    self.controls.place(CardChoice::Acquire, preview, viewport, false);
                }
            } else {
                let download = cx.model.downloads.get(&key);
                let label = if cached {
                    "Preview unavailable · save original"
                } else if download.is_some_and(|d| d.status.failure.is_some()) {
                    "Preview unavailable"
                } else if download.is_some_and(|d| !d.status.done) {
                    "Loading preview…"
                } else {
                    "Download to preview"
                };
                cx.services.renderer.clipped_icon(
                    ctx,
                    layer,
                    Icon::Image,
                    Rect::new(x + width / 2. - 16. * s, preview.y + 74. * s, 32. * s, 32. * s),
                    0x687e8f,
                    clip,
                );
                // Center a measured one-line placeholder, rather than an off-center Loading label.
                let label_width = cx.services.renderer.label_width(label, 12. * s, false).min(width - 16. * s);
                cx.services.renderer.ellipsized_label(
                    layer,
                    label,
                    Rect::new(x + (width - label_width) / 2., preview.y + 118. * s, label_width, 20. * s),
                    12. * s,
                    color(0x9eaebd),
                    false,
                    false,
                    clip,
                );
            }
            if !cached
                && cx.model.content_authorized()
                && !cx.model.downloads.contains_key(&key)
                && let Err(error) = cx.model.download(session, entry, 10_000_000)
            {
                cx.model.notice = Some(error.to_string().into());
            }
        }
        let mut display = AttachmentDisplay::new(
            attachment,
            cached,
            cx.model.downloads.get(&key),
            exported.is_some(),
            cx.services.transfers.saving_downloads.contains(&key),
            cx.services.transfers.export_errors.get(&key).map(String::as_str),
        );
        if display.control == Control::View && !preview_available {
            display.status = format!(
                "{}Preview unavailable",
                attachment.size.map(|n| format!("{} · ", format_bytes(n))).unwrap_or_default()
            );
            display.failed = true;
        }
        let control =
            if display.control == Control::View && !preview_available { Control::Save } else { display.control };
        let action = match control {
            Control::Download | Control::Retry if image && cx.services.transfers.export_errors.contains_key(&key) => {
                Some(CardChoice::Save)
            }
            Control::Download | Control::Retry | Control::View => Some(CardChoice::Acquire),
            Control::Save => Some(CardChoice::Save),
            Control::Open => Some(CardChoice::UseSaved(SavedAction::Open)),
            Control::Cancel => Some(CardChoice::Cancel),
            Control::Busy => None,
        };
        let mut actions = vec![(control.label(), Some(control.description()), action)];
        if exported.is_some() {
            if image && preview_available {
                actions.push(("View", Some("View image"), Some(CardChoice::Acquire)));
            }
            #[cfg(not(target_os = "android"))]
            if !cx.ui.mobile {
                actions.push(("Show", Some("Show in folder"), Some(CardChoice::UseSaved(SavedAction::Show))));
                if attachment.file_name.to_ascii_lowercase().ends_with(".zip") {
                    let extracting = cx.services.transfers.extracting_downloads.contains(&self.target);
                    actions.push((
                        if extracting { "Extracting…" } else { "Extract" },
                        None,
                        (!extracting).then(|| CardChoice::UseSaved(SavedAction::Extract)),
                    ));
                }
            }
        } else if image && preview_available && control == Control::View {
            actions.push(("Save", Some("Save to Downloads"), Some(CardChoice::Save)));
        } else if image && preview_available && control == Control::Retry {
            // Retrying an OS save must not hide or discard a perfectly good preview.
            actions.push(("View", Some("View image"), Some(CardChoice::Acquire)));
        }
        layer.clipped_rounded_rect(panel, 10. * s, color(0x101820), viewport);
        let target = if cx.ui.mobile { 44. } else { 40. } * s;
        // Tau1 uses text for these actions. Measure each complete label instead
        // of reserving icon-sized slots or guessing widths from character counts.
        let widths = actions
            .iter()
            .map(|(label, _, _)| (cx.services.renderer.label_width(label, 14. * s, false).ceil() + 16. * s).max(target))
            .collect::<Vec<_>>();
        let actions_width = widths.iter().sum::<f32>();
        let text =
            Rect::new(panel.x + 12. * s, panel.y + 12. * s, (panel.width - actions_width - 20. * s).max(1.), 20. * s);
        cx.services.renderer.ellipsized_label(
            layer,
            &attachment.file_name,
            text,
            13. * s,
            color(0xe5eaf0),
            true,
            true,
            viewport,
        );
        let status = Rect::new(text.x, panel.y + 34. * s, text.width, 18. * s);
        cx.services.renderer.ellipsized_label(
            layer,
            &display.status,
            status,
            11. * s,
            color(if display.failed {
                0xffb4ab
            } else if exported.is_some() {
                0x93cbb4
            } else {
                0x9eaebd
            }),
            false,
            false,
            viewport,
        );
        if let Some(caption) = caption {
            let area = Rect::new(x, panel.y - 24. * s, width, 20. * s);
            cx.services.renderer.ellipsized_label(
                layer,
                caption,
                area,
                12. * s,
                color(0xb7c2ce),
                false,
                false,
                viewport,
            );
        }
        if let Some(progress) = display.progress {
            let track = Rect::new(panel.x + 12. * s, panel.y + panel.height - 7. * s, panel.width - 24. * s, 3. * s);
            layer.clipped_rounded_rect(track, 1.5 * s, color(0x263947), viewport);
            let fill = match progress {
                Progress::Known(fraction) => Rect::new(track.x, track.y, track.width * fraction, track.height),
                Progress::Unknown => Rect::new(
                    track.x
                        + track.width
                            * (cx.services.transfers.progress_clock.elapsed().as_secs_f32() * 0.45).fract()
                            * 0.75,
                    track.y,
                    track.width * 0.25,
                    track.height,
                ),
            };
            layer.clipped_rounded_rect(fill, 1.5 * s, color(0x67d4ff), viewport);
        }
        let mut left = panel.x + panel.width - actions_width - 4. * s;
        for (index, ((label, description, action), width)) in actions.into_iter().zip(widths).enumerate() {
            let r = Rect::new(left, panel.y + (60. * s - target) / 2., width, target);
            left += width;
            let clip = crate::render::intersect(r, viewport);
            if clip.width <= 0. || clip.height <= 0. {
                continue;
            }
            let enabled = action.is_some();
            let button = self.controls.place_in(Some(index), action.unwrap_or(CardChoice::Noop), r, viewport, true);
            button.control.enabled = enabled;
            button.label = label.into();
            button.style = ButtonStyle::Quiet;
            button.visit_perframe(&mut Frame { layer, bounds: r, clip: viewport }, cx);
            let control = &mut button.control;
            control.info = description.map(|description| {
                Info::Attachment(format!("{info_key}:action:{index}"), description.into(), attachment.file_name.clone())
            });
        }

        self.controls.finish(cx);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BrowserChoice {
    Close,
    History,
}
pub(in crate::app) struct AttachmentBrowser {
    pub show: bool,
    pub side: bool,
    pub scroll: super::scroll::ScrollState,
    pub cards: HashMap<String, AttachmentCard>,
    form: super::controls::Form<BrowserChoice>,
    bounds: Rect,
    binding: Option<(String, Option<String>, String)>,
    history_attempt: Option<(String, String, u64, u64)>,
    pub interests: BTreeSet<String>,
}
impl AttachmentBrowser {
    pub fn new() -> Self {
        let id = Id::new();
        Self {
            show: false,
            side: false,
            scroll: super::scroll::ScrollState::new(id, false),
            cards: HashMap::new(),
            form: super::controls::Form::new(
                id,
                &[(BrowserChoice::Close, "Back"), (BrowserChoice::History, "Load older files")],
            ),
            bounds: Rect::new(0., 0., 0., 0.),
            binding: None,
            history_attempt: None,
            interests: BTreeSet::new(),
        }
    }
    pub fn reset(&mut self) {
        self.scroll.stop();
        self.scroll.value = 0.;
        self.scroll.max = 0.;
        self.cards.clear();
        self.binding = None;
        self.history_attempt = None;
        self.hide();
    }
    pub fn hide(&mut self) {
        self.bounds = Rect::new(0., 0., 0., 0.);
        self.scroll.rect = self.bounds;
        for card in self.cards.values_mut() {
            card.hide();
        }
        self.form.begin_frame();
        self.interests.clear();
    }
    fn bound(&self, model: &Controller) -> bool {
        self.binding.as_ref().is_some_and(|(identity, lineage, session)| {
            identity == &model.identity
                && lineage == &model.account.source_lineage
                && Some(session) == model.account.selected.as_ref()
        })
    }
    fn history(&mut self, cx: &mut Context<'_>) {
        let Some((_, _, session)) = &self.binding else {
            return;
        };
        let Some(feed) = cx.model.chats.get(session).map(|c| &c.feed) else {
            return;
        };
        if feed.synchronized
            && !feed.loading
            && let (Some(before), Some(epoch)) = (feed.before, cx.model.epoch)
        {
            let attempt = (session.clone(), feed.generation.clone(), before, epoch);
            if self.history_attempt.as_ref() != Some(&attempt) {
                self.history_attempt = Some(attempt);
                let result = cx.model.history();
                cx.report(result);
            }
        }
    }
}
impl Widget for AttachmentBrowser {
    fn update(&mut self, dt: f32, cx: &mut Context<'_>) {
        if self.show && self.bound(cx.model) {
            self.scroll.update(dt, cx);
        }
    }
    fn owns(&self, target: Target, model: &Controller, ui: &UiState) -> bool {
        self.show
            && self.bound(model)
            && (self.scroll.target == target
                || self.form.owns(target)
                || self.cards.values().any(|c| c.owns(target, model, ui)))
    }

    fn handle_event(&mut self, event: &Event<'_>, cx: &mut Context<'_>) -> bool {
        if !self.show || !self.bound(cx.model) {
            return false;
        }
        if matches!(event, Event::Back) {
            cx.ui.requests.push_back(Request::Attachments(false));
            return true;
        }
        if self.scroll.bar_event(event, cx) {
            return true;
        }
        let (handled, choice) = self.form.event(event, std::iter::empty(), cx);
        if let Some(choice) = choice {
            match choice {
                BrowserChoice::Close => cx.ui.requests.push_back(Request::Attachments(false)),
                BrowserChoice::History => {
                    self.history_attempt = None;
                    self.history(cx);
                }
            }
            return true;
        }
        if handled {
            return true;
        }
        let child = self.cards.values_mut().any(|card| card.dispatch(event, cx));
        let handled = self.scroll.event(event, child, cx);
        // Header/backdrop space belongs to this pane, never the transcript behind it.
        handled
            || match *event {
                Event::Down { point, .. } | Event::Up { point, .. } | Event::Context(point) => {
                    contains(self.bounds, point)
                }
                _ => false,
            }
    }
    fn visit_perframe(&mut self, frame: &mut Frame<'_>, cx: &mut Context<'_>) {
        self.hide();
        if !self.show {
            return;
        }
        let Some(session) = cx.model.account.selected.clone() else {
            return;
        };
        let binding = (cx.model.identity.clone(), cx.model.account.source_lineage.clone(), session.clone());
        if self.binding.as_ref() != Some(&binding) {
            self.reset();
            self.binding = Some(binding);
        }
        let Some(chat) = cx.model.chats.get(&session) else {
            return;
        };
        let files = chat
            .feed
            .events
            .values()
            .rev()
            .filter_map(|e| e.attachment.clone().map(|f| (e.id.clone(), e.entry_id.clone(), f)))
            .collect::<Vec<_>>();
        let older = chat.feed.before.is_some();
        let loading = chat.feed.loading;
        let synchronized = chat.feed.synchronized;
        let b = frame.bounds;
        let s = cx.ui.scale;
        self.bounds = b;
        frame.layer.rect(b, color(0x0e141b));
        if self.side {
            frame.layer.rect(Rect::new(b.x, b.y, s, b.height), color(0x2a3541));
        }
        frame.layer.rect(Rect::new(b.x, b.y + 56. * s, b.width, s), color(0x2a3541));
        let tx = b.x + if self.side { 14. * s } else { 80. * s };
        let tw = b.width - if self.side { 70. * s } else { 94. * s };
        cx.services.renderer.label(
            frame.layer,
            "Attachments",
            Rect::new(tx, b.y + 8. * s, tw, 22. * s),
            16. * s,
            color(0xe5eaf0),
            true,
        );
        cx.services.renderer.label(
            frame.layer,
            &format!("{} loaded · newest first", files.len()),
            Rect::new(tx, b.y + 30. * s, tw, 18. * s),
            12. * s,
            color(0x82909f),
            false,
        );
        self.form.buttons[0].1.label = if self.side { "×" } else { "Back" }.into();
        self.form.button(
            BrowserChoice::Close,
            Rect::new(
                if self.side { b.x + b.width - 48. * s } else { b.x + 8. * s },
                b.y + 8. * s,
                if self.side { 40. * s } else { 64. * s },
                40. * s,
            ),
            ButtonStyle::Tonal,
            frame,
            cx,
        );
        let viewport = Rect::new(b.x + s, b.y + 57. * s, b.width - s, (b.height - 57. * s).max(0.));
        self.scroll.rect = viewport;
        let heights = files.iter().map(|(_, _, f)| card_height(f) * s).collect::<Vec<_>>();
        let content = 12. * s + heights.iter().map(|h| h + 12. * s).sum::<f32>();
        self.scroll.max = (content + if older { 52. * s } else { 0. } - viewport.height).max(0.);
        self.scroll.set(self.scroll.value);
        let mut y = viewport.y + 12. * s - self.scroll.value;
        for ((id, entry, file), height) in files.iter().zip(heights) {
            let rect = Rect::new(b.x + 12. * s, y, b.width - 28. * s, height);
            if y + height >= viewport.y - viewport.height && y <= viewport.y + 2. * viewport.height {
                self.interests.insert(id.clone());
            }
            if y + height >= viewport.y && y <= viewport.y + viewport.height {
                frame.layer.clipped_corners(rect, [12. * s; 4], color(0x18212b), viewport);
                let card = self
                    .cards
                    .entry(entry.clone())
                    .or_insert_with(|| AttachmentCard::new(&session, entry, file, "attachments", cx));
                card.bind_file(file, cx);
                card.visit_perframe(
                    &mut Frame {
                        layer: frame.layer,
                        bounds: rect,
                        clip: crate::render::intersect(frame.clip, viewport),
                    },
                    cx,
                );
            }
            y += height + 12. * s;
        }
        if files.is_empty() {
            let text = if cx.model.epoch.is_none() {
                "No cached attachments.\nConnect to load sent files."
            } else if older || !synchronized {
                "Loading attachments…"
            } else {
                "No attachments yet.\nFiles sent in this chat appear here."
            };
            cx.services.renderer.label(
                frame.layer,
                text,
                Rect::new(b.x + 24. * s, viewport.y + 80. * s, b.width - 48. * s, 100. * s),
                14. * s,
                color(0xb7c2ce),
                false,
            );
        }
        if older {
            let r = Rect::new(b.x + 16. * s, y, b.width - 44. * s, 36. * s);
            if loading || !synchronized || cx.model.epoch.is_none() {
                cx.services.renderer.clipped_label(
                    frame.layer,
                    if cx.model.epoch.is_none() { "Connect to load older files" } else { "Loading older files…" },
                    r,
                    12. * s,
                    color(0x82909f),
                    false,
                    viewport,
                );
            } else {
                self.form.button(
                    BrowserChoice::History,
                    r,
                    ButtonStyle::Tonal,
                    &mut Frame { layer: frame.layer, bounds: b, clip: viewport },
                    cx,
                );
            }
            if self.scroll.max - self.scroll.value <= 2. * viewport.height {
                self.history(cx);
            }
        }
        self.scroll.paint(frame.layer, cx);
        self.cards.retain(|_, card| {
            let mounted = card.controls.items.iter().any(|(_, button, _)| button.control.rect.is_some());
            if !mounted {
                cx.ui.detach(card.controls.id);
            }
            mounted
        });
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/app/ui/attachments.rs"]
mod tests;
