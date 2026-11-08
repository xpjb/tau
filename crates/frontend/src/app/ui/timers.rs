use super::*;
use crate::app::{COUNTER_REFRESH, Info};
use std::time::Instant;
impl RootWidget {
    pub fn timers(&mut self, cx: &mut Context<'_>) {
        let visible = self.workspace.chat_visible(cx);
        // Refresh the selected Codex account periodically, even with its card closed.
        // The account view bounds requests to five minutes (30s after failure).
        if visible && cx.ui.visible {
            match cx.model.refresh_codex_usage() {
                Ok(sent) => cx.ui.dirty |= sent,
                Err(error) => cx.model.report_error(error),
            }
        }
        if visible
            && self.tooltips.usage.region.width > 0.
            && (self.tooltips.usage.progress > 0. || self.tooltips.usage.pinned)
        {
            let quota = cx.model.codex_usage.content(cx.model.epoch.is_some());
            if cx
                .model
                .account
                .selected
                .as_ref()
                .and_then(|id| cx.model.account.sessions.iter().find(|s| &s.id == id))
                .and_then(|s| s.model.as_ref())
                .is_some_and(|m| m.provider == "openai-codex")
                && !self.tooltips.usage.content.text.ends_with(&quota.text)
            {
                cx.ui.dirty = true;
            }
        }
        if cx.ui.visible
            && self.tooltips.info.progress > 0.
            && self.tooltips.info.region.width > 0.
            && let Info::CacheTtl(id) = &self.tooltips.target
            && let Some(session) = cx.model.account.sessions.iter().find(|s| &s.id == id)
        {
            cx.ui.dirty |= self.tooltips.info.content != cx.model.cache_ttl(session).details();
        }
        if self.dialog.is_some() || self.viewer.is_some() {
            self.tooltips.usage.dismiss();
            self.tooltips.info.dismiss();
        }
        let now = Instant::now();
        let dot_color = cx.model.health.color(now);
        cx.ui.dirty |= cx.services.dot_color != dot_color;
        cx.services.dot_color = dot_color;
        let card_visible = cx.ui.visible
            && self.tooltips.target == Info::Connection
            && self.tooltips.info.progress > 0.
            && self.tooltips.info.region.width > 0.
            && self.dialog.is_none()
            && self.viewer.is_none()
            && self.menu.is_none();
        let counter_bucket = card_visible
            .then(|| cx.model.health.counter(now))
            .flatten()
            .map(|(_, ms)| ms / COUNTER_REFRESH.as_millis());
        let next_wake = if !cx.ui.visible || self.dialog.is_some() || self.viewer.is_some() {
            None
        } else if card_visible && counter_bucket.is_some() {
            Some(COUNTER_REFRESH)
        } else {
            cx.model.health.next_color_wake(now)
        };
        let indeterminate = cx.ui.visible
            && self.dialog.is_none()
            && self.viewer.is_none()
            && (!cx.ui.mobile || !self.workspace.show_chats || self.workspace.attachments.show)
            && (cx.model.downloads.values().any(|d| !d.status.done && d.status.total == 0)
                || !cx.services.transfers.saving_downloads.is_empty());
        let progress_bucket =
            indeterminate.then(|| now.duration_since(cx.services.transfers.progress_clock).as_millis() / 80);
        cx.ui.dirty |= cx.services.transfers.progress_bucket != progress_bucket;
        cx.services.transfers.progress_bucket = progress_bucket;
        let next_wake = if indeterminate {
            Some(next_wake.map_or(std::time::Duration::from_millis(80), |duration| {
                duration.min(std::time::Duration::from_millis(80))
            }))
        } else {
            next_wake
        };
        // Quota reset / TTL text stays current when pinned, even while offline.
        // Share the existing timer; closed cards do not acquire a redraw loop.
        let timed_tooltip = cx.ui.visible
            && self.dialog.is_none()
            && self.viewer.is_none()
            && self.menu.is_none()
            && (self.tooltips.usage.progress > 0. && self.tooltips.usage.region.width > 0.
                || self.tooltips.info.progress > 0.
                    && self.tooltips.info.region.width > 0.
                    && matches!(self.tooltips.target, Info::CacheTtl(_)));
        let next_wake =
            if timed_tooltip {
                Some(next_wake.map_or(std::time::Duration::from_secs(1), |duration| {
                    duration.min(std::time::Duration::from_secs(1))
                }))
            } else {
                next_wake
            };

        let next_wake = match (next_wake, self.notice.popup.remaining(now)) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        let next_wake = if cx.ui.visible && matches!(self.dialog, Some(Dialog::CodexLogin(_))) {
            Some(next_wake.map_or(std::time::Duration::from_secs(1), |d| d.min(std::time::Duration::from_secs(1))))
        } else { next_wake };
        cx.services.wake.sync(next_wake);
        // Redraw only when the visible counter or dot actually changes.
        cx.ui.dirty |= cx.services.counter_bucket != counter_bucket;
        cx.services.counter_bucket = counter_bucket;
    }
}
