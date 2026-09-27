//! A one-hour provider-cache *estimate*, not a daemon worker deadline.
//! Use existing source timestamps; receiving history/heartbeats never renews it.
use crate::{clock, feed::Feed, tooltip::{Content, ACCENT, INK, WARNING}};
use tau_protocol::{EventKind, EventRole, SessionStatus, SessionSummary};

const ESTIMATED_TTL_MS: u64 = 60 * 60 * 1000;
#[derive(Clone, Copy)]
enum Basis {
    Working,
    Reply(u64),
    Activity(u64),
    NoReply,
    Unknown,
}
impl Basis {
    fn timestamp(self) -> Option<u64> {
        match self {
            Self::Reply(at) | Self::Activity(at) => Some(at),
            Self::Working | Self::NoReply | Self::Unknown => None,
        }
    }
}
pub struct Estimate {
    basis: Basis,
    remaining_ms: Option<u64>,
}
impl Estimate {
    pub fn from_session(session: &SessionSummary, feed: Option<&Feed>) -> Self {
        // Latest assistant output in transcript order, including thinking/tool
        // calls, but not local tool results or synthetic failure/hidden entries.
        let reply = feed.and_then(|feed| {
            feed.events.values().rev().find(|event| {
                event.role == EventRole::Assistant
                    && event.kind != EventKind::Hidden
                    && !event.is_error
                    && event.error_message.is_none()
            })
        });
        let basis = if session.status == SessionStatus::Running {
            Basis::Working
        } else if let Some(reply) = reply {
            clock::event_ms(reply)
                .filter(|at| *at > 0)
                .map(Basis::Reply)
                .unwrap_or(Basis::Unknown)
        } else if session.starter || feed.is_some_and(|f| f.synchronized && f.before.is_none()) {
            Basis::NoReply
        } else if session.updated_at_ms > 0 {
            // An unopened chat already has an activity timestamp in the list.
            // This is explicitly a weaker proxy (renames also update it); do
            // not fetch more history solely to paint the TTL meter.
            Basis::Activity(session.updated_at_ms)
        } else {
            Basis::Unknown
        };
        let remaining_ms = basis
            .timestamp()
            .zip(clock::now_ms())
            .and_then(|(at, now)| now.checked_sub(at))
            .map(|elapsed| ESTIMATED_TTL_MS.saturating_sub(elapsed));
        Self {
            basis,
            remaining_ms,
        }
    }
    pub fn meter(&self) -> (Option<f32>, u32) {
        match self.remaining_ms {
            Some(ms) => (
                Some(ms.div_ceil(60_000) as f32 / 60.),
                if ms <= 300_000 { 0xfbbf24 } else { 0x67d4ff },
            ),
            None => (
                None,
                if matches!(self.basis, Basis::Working) { 0x67d4ff } else { 0x82909f },
            ),
        }
    }
    pub fn details(&self) -> Content {
        let mut content = Content::default();
        match self.remaining_ms {
            Some(ms) => {
                content.dim("TTL ").strong(format!("~{}m", ms.div_ceil(60_000)),
                    if ms <= 300_000 { WARNING } else { ACCENT }).dim(" remaining");
            }
            None => match self.basis {
                Basis::Working => { content.strong("Working...", ACCENT); }
                Basis::NoReply => { content.push("No reply yet", false, INK); }
                _ => { content.dim("TTL unavailable"); }
            },
        }
        content
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(status: SessionStatus, updated_at_ms: u64) -> SessionSummary {
        SessionSummary {
            id: "chat".into(),
            project_id: "general".into(),
            title: "Chat".into(),
            starter: false,
            status,
            detail: None,
            context_usage: None,
            model: None,
            parent_id: None,
            created_at_ms: 0,
            updated_at_ms,
        }
    }

    #[test]
    fn tooltip_is_one_line_with_rounded_minutes() {
        for basis in [Basis::Reply(1), Basis::Activity(1)] {
            for (ms, minutes) in [
                (0, 0), (1, 1), (60_000, 1), (60_001, 2), (480_000, 8), (ESTIMATED_TTL_MS, 60),
            ] {
                let estimate = Estimate { basis, remaining_ms: Some(ms) };
                assert_eq!(estimate.details().text, format!("TTL ~{minutes}m remaining"));
            }
        }
    }

    #[test]
    fn working_has_no_countdown_even_without_a_reply_or_after_expiry() {
        let now = clock::now_ms().unwrap();
        let mut feed = Feed::default();
        feed.synchronized = true;
        for timestamp in [0, now, now - 2 * ESTIMATED_TTL_MS] {
            let mut session = session(SessionStatus::Running, timestamp);
            for starter in [false, true] {
                session.starter = starter;
                for feed in [None, Some(&feed)] {
                    let estimate = Estimate::from_session(&session, feed);
                    assert_eq!(estimate.details().text, "Working...");
                    assert_eq!(estimate.remaining_ms, None);
                    assert_eq!(estimate.meter(), (None, 0x67d4ff));
                }
            }
        }
    }

    #[test]
    fn countdown_returns_when_work_stops() {
        let mut session = session(SessionStatus::Running, clock::now_ms().unwrap() - 52 * 60_000);
        assert_eq!(Estimate::from_session(&session, None).details().text, "Working...");
        for status in [SessionStatus::Idle, SessionStatus::Sleeping, SessionStatus::Error] {
            session.status = status;
            assert_eq!(Estimate::from_session(&session, None).details().text, "TTL ~8m remaining");
        }
        session.updated_at_ms -= ESTIMATED_TTL_MS;
        assert_eq!(Estimate::from_session(&session, None).details().text, "TTL ~0m remaining");
    }

    #[test]
    fn missing_timestamps_and_empty_chats_stay_compact() {
        let mut session = session(SessionStatus::Idle, 0);
        assert_eq!(Estimate::from_session(&session, None).details().text, "TTL unavailable");
        session.updated_at_ms = clock::now_ms().unwrap() + ESTIMATED_TTL_MS;
        assert_eq!(Estimate::from_session(&session, None).details().text, "TTL unavailable");
        session.starter = true;
        assert_eq!(Estimate::from_session(&session, None).details().text, "No reply yet");
        session.starter = false;
        let mut feed = Feed::default();
        feed.synchronized = true;
        assert_eq!(Estimate::from_session(&session, Some(&feed)).details().text, "No reply yet");
    }
}
