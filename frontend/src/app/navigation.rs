//! UI navigation binds the composer and measured layout to the controller's
//! account/chat selection. Explicit clicks and asynchronous selection changes
//! use the same reconciliation; the controller still owns topics and persistence.
//! Attachment destinations resolve only after history has a measured layout.
use super::*;

pub(super) struct Navigation {
    pub identity: String,
    pub session: Option<String>,
    pub download: Option<DownloadTarget>,
}
impl Navigation {
    pub fn new(controller: &Controller) -> Self {
        Self { identity: controller.identity.clone(), session: controller.account.selected.clone(), download: None }
    }
}

impl App {
    pub(super) fn sync_navigation(&mut self) {
        self.validate_download_jump();
        let selected = self.controller.account.selected.clone();
        if selected != self.root.legacy.navigation.session || self.controller.identity != self.root.legacy.navigation.identity {
            self.ui.navigation_changed();
            // Also handles server-driven changes. Never save the old layout
            // through a controller that has already switched accounts.
            if self.controller.identity == self.root.legacy.navigation.identity
                && let Some(previous) = &self.root.legacy.navigation.session
                && self.root.legacy.placed_session.as_deref() == Some(previous.as_str())
                && let Err(error) = self.controller.save_chat(previous) {
                self.controller.report_error(error);
            }
            // Cancel the old chat/account's file interest before rebinding the
            // composer; do not wait for the next UI tick after navigation.
            self.close_code();
            self.root.legacy.placed.clear();
            self.root.legacy.placed_session = None;
            self.cancel_pointer();
            self.root.legacy.history_attempt = None;
            self.root.menu = None;
            self.root.tooltips.usage = Tooltip::default();
            self.root.legacy.navigation.identity = self.controller.identity.clone();
            self.root.legacy.navigation.session = selected;
            self.root.legacy.scroll = 0.;
            self.root.legacy.horizontal = 0.;
            self.root.legacy.velocity = 0.;
            self.root.composer.field.editor = Editor::composer(
                self.controller
                    .selected()
                    .map(|c| c.local.draft.clone())
                    .unwrap_or_default(),
            );
            self.root.legacy.show_chats = self.root.legacy.navigation.session.is_none();
            self.root.attachments.scroll.value = 0.;
            self.root.attachments.scroll.max = 0.;
            if self.root.legacy.show_chats { self.root.attachments.show = false; }
            self.ui.dirty = true;
        } else if let Some(chat) = self.controller.selected()
            && self.root.composer.field.editor.value != chat.local.draft
        {
            self.replace_composer(chat.local.draft.clone());
            self.ui.dirty = true;
        }
        self.with_ui(|root,cx|root.composer.bind(cx));
    }

    pub(super) fn navigate_chat(&mut self, id: &str) -> Result<()> {
        self.save()?;
        let same_chat = self.controller.account.selected.as_deref() == Some(id);
        let same_topic = self.controller.account.sessions.iter().find(|s| s.id == id)
            .is_none_or(|s| s.project_id == self.controller.account.selected_project);
        if !same_chat || !same_topic {
            self.controller.select(id)?;
        }
        // Explicit chat/download navigation also exits the browser when the
        // destination is the already-selected chat.
        self.close_code();
        self.sync_navigation();
        self.root.legacy.show_chats = false;
        self.ui.focus=Some(self.root.composer.field.control.target);
        Ok(())
    }

    pub(super) fn navigate_project(&mut self, id: &str) -> Result<()> {
        if self.controller.account.selected_project == id
            && self.controller.account.selected.as_ref().is_some_and(|chat|
                self.controller.account.sessions.iter().any(|s| s.id == *chat && s.project_id == id)) {
            return Ok(());
        }
        self.save()?;
        self.controller.select_project(id, self.ui.size.0 as f32 / self.ui.scale >= 760.)?;
        self.sync_navigation();
        self.root.sidebar.scroll.value = 0.;
        self.root.legacy.show_chats = self.controller.account.selected.is_none();
        self.ui.focus = None;
        Ok(())
    }

    pub(super) fn open_download_notice(&mut self, target: DownloadTarget) -> Result<()> {
        anyhow::ensure!(target.matches_source(&self.controller.identity,
            self.controller.account.source_lineage.as_deref()),
            "This download belongs to a different account or source history.");
        anyhow::ensure!(self.controller.account.sessions.iter().any(|s| s.id == target.session)
            && !self.controller.account.missing_chats.contains(&target.session),
            "The chat for this download is no longer available.");
        // The ordinary chat route follows current topic membership and preserves
        // the outgoing draft/anchor. Only the final viewport destination differs.
        self.navigate_chat(&target.session)?;
        self.cancel_pointer();
        self.close_ui();
        
        self.root.viewer = None;
        self.ui.focus = None;
        self.root.legacy.show_chats = false;
        self.root.attachments.show = false;
        self.root.sidebar.scroll.value = 0.;
        self.root.legacy.horizontal = 0.;
        self.root.legacy.scroll = 0.;
        self.root.legacy.placed.clear();
        self.root.legacy.placed_session = None;
        self.root.legacy.history_attempt = None;
        self.root.legacy.navigation.download = Some(target);
        Ok(())
    }

    pub(super) fn validate_download_jump(&mut self) {
        if self.root.legacy.navigation.download.as_ref().is_some_and(|target|
            !target.matches_source(&self.controller.identity, self.controller.account.source_lineage.as_deref())
                || self.controller.account.selected.as_deref() != Some(target.session.as_str())) {
            self.root.legacy.navigation.download = None;
        }
    }

    /// Return true while explicit navigation owns the scroll position. Keep the
    /// request across empty/loading frames and older pages, but never persist an
    /// interim page as the user's destination.
    pub(super) fn locate_download(&mut self, rows: &[Row], placements: &[Placed], viewport: Rect) -> bool {
        self.validate_download_jump();
        let Some(target) = &self.root.legacy.navigation.download else { return false; };
        if let Some((_, placed)) = rows.iter().zip(placements).find(|(row, _)|
            row.attachment.as_ref().is_some_and(|(entry, _)| entry == &target.entry)) {
            // Center the actual download controls, not the beginning of a long
            // message or image above them. This also works at mobile UI scales.
            let panel = attachments::control_panel(Rect::new(0., placed.top, viewport.width, placed.height), self.ui.scale);
            self.root.legacy.scroll = (panel.y + panel.height / 2. - viewport.height / 2.).clamp(0., self.root.legacy.max_scroll);
            let position = &mut self.controller.chats.get_mut(&target.session).unwrap().local.position;
            position.key = Some(placed.key.clone());
            position.offset = (self.root.legacy.scroll - placed.top) / self.ui.scale;
            position.follow = false;
            self.root.legacy.navigation.download = None;
        } else {
            self.root.legacy.scroll = 0.;
            let feed = &self.controller.chats[&target.session].feed;
            if self.controller.account.missing_chats.contains(&target.session)
                || feed.synchronized && !feed.loading && feed.before.is_none() {
                self.root.legacy.navigation.download = None;
                self.controller.notice = Some("The download widget is no longer available in this chat.".into());
            }
        }
        true
    }
}
