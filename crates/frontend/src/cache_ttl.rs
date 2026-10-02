//! A one-hour provider-cache *estimate*, not a daemon worker deadline.
//! Use existing source timestamps; receiving history/heartbeats never renews it.
use crate::{clock, feed::Feed, tooltip::{Content, ACCENT, INK, WARNING}};
use tau_net::{EventKind, EventRole, SessionStatus, SessionSummary};

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
#[path = "../tests/unit/cache_ttl.rs"]
mod tests;
