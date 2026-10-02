//! Popup lifetime is separate from persistent inline settings/recovery errors.
use std::time::{Duration, Instant};
use crate::notice::Notice;

pub(super) const LIFETIME: Duration = Duration::from_secs(4);
#[derive(Default)]
pub(super) struct NoticePopup {
    message: Option<Notice>,
    until: Option<Instant>,
}
impl NoticePopup {
    pub fn observe(&mut self, message: Option<&Notice>, now: Instant) -> bool {
        if self.message.as_ref() != message {
            self.message = message.cloned();
            self.until = message.map(|_| now + LIFETIME);
            return true;
        }
        if self.until.is_some_and(|until| now >= until) {
            self.until = None;
            return true;
        }
        false
    }
    pub fn remaining(&self, now: Instant) -> Option<Duration> {
        self.until.map(|until| until.saturating_duration_since(now))
    }
    pub fn visible(&self) -> bool { self.until.is_some() }
}

#[cfg(test)]
#[path = "../../tests/unit/app/notices.rs"]
mod tests;

#[cfg(all(test, not(target_os = "android")))]
#[path = "../../tests/unit/app/notices_render.rs"]
mod render_tests;
