//! Diagnostics from the existing application heartbeat, not a synthetic quality score.
use crate::{clock, store::Settings};
use std::{collections::VecDeque, time::Duration};

pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(20);
const RECENT_SAMPLES: usize = 8;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Offline,
    Connecting,
    Connected,
    Reconnecting,
    Blocked,
}
#[derive(Default)]
pub struct Health {
    pub phase: Phase,
    connected_at: Option<u64>,
    last_reply: Option<u64>,
    pending_at: Option<u64>,
    pending: bool,
    last_ok: Option<bool>,
    connections: u64,
    recent: VecDeque<Duration>,
}
impl Health {
    pub fn connecting() -> Self {
        Self {
            phase: Phase::Connecting,
            ..Self::default()
        }
    }
    pub fn connected(&mut self) {
        self.phase = Phase::Connected;
        self.connections = self.connections.saturating_add(1);
        self.connected_at = clock::now_ms();
        self.last_reply = None;
        self.pending_at = None;
        self.pending = false;
        self.last_ok = None;
        self.recent.clear(); // Never present old-epoch RTTs as current measurements.
    }
    pub fn disconnected(&mut self, fatal: bool) {
        self.phase = if fatal {
            Phase::Blocked
        } else {
            Phase::Reconnecting
        };
        self.pending = false;
        self.pending_at = None;
    }
    pub fn sent(&mut self, at: Option<u64>) {
        self.pending = true;
        self.pending_at = at;
    }
    pub fn reply(&mut self, at: Option<u64>, rtt: Duration, ok: bool) {
        self.last_reply = at;
        self.pending_at = None;
        self.pending = false;
        self.last_ok = Some(ok);
        if self.recent.len() == RECENT_SAMPLES {
            self.recent.pop_front();
        }
        self.recent.push_back(rtt);
    }
    pub fn color(&self) -> u32 {
        match self.phase {
            Phase::Connected if self.last_ok != Some(false) => 0x4ade80,
            Phase::Connected | Phase::Connecting | Phase::Reconnecting => 0xfbbf24,
            Phase::Blocked => 0xffb4ab,
            Phase::Offline => 0x82909f,
        }
    }
    pub fn details(&self, settings: &Settings, reason: &str) -> String {
        let title = match self.phase {
            Phase::Offline => "Not connected",
            Phase::Connecting => "Connecting…",
            Phase::Connected if self.last_ok == Some(false) => {
                "Connected · heartbeat request failed"
            }
            Phase::Connected => "Connected",
            Phase::Reconnecting => "Reconnecting…",
            Phase::Blocked => "Connection blocked",
        };
        // Display only the origin, never credentials, query strings, fragments or
        // potentially token-bearing paths (even if saved settings are invalid).
        let endpoint = url::Url::parse(&settings.server_url)
            .ok()
            .filter(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some());
        let mut lines = vec![title.into()];
        if let Some(url) = endpoint {
            lines.push(url.origin().ascii_serialization());
            lines.push(
                if url.scheme() == "https" {
                    "WebSocket · TLS"
                } else {
                    "WebSocket · no TLS (HTTP)"
                }
                .into(),
            );
        }
        if self.phase != Phase::Connected {
            if matches!(self.phase, Phase::Blocked | Phase::Reconnecting) {
                lines.push(reason.into());
            }
            if self.last_reply.is_some() {
                lines.push(format!(
                    "Last confirmed reply: {}",
                    clock::label(self.last_reply)
                ));
            }
            return lines.join("\n");
        }
        if let Some(last) = self.recent.back() {
            lines.push(format!(
                "Heartbeat RTT: {}",
                millis(last.as_secs_f64() * 1000.)
            ));
            if self.recent.len() > 1 {
                let avg = self.recent.iter().map(Duration::as_secs_f64).sum::<f64>() * 1000.
                    / self.recent.len() as f64;
                let min = self.recent.iter().min().unwrap().as_secs_f64() * 1000.;
                let max = self.recent.iter().max().unwrap().as_secs_f64() * 1000.;
                lines.push(format!(
                    "Recent {}: avg {} · {}–{}",
                    self.recent.len(),
                    millis(avg),
                    millis(min),
                    millis(max)
                ));
            }
            // Absolute times remain accurate while the on-demand UI is asleep;
            // no once-per-second redraw timer is needed for an aging counter.
            lines.push(format!("Last reply: {}", clock::label(self.last_reply)));
        } else {
            lines.push("Heartbeat RTT: not measured yet".into());
        }
        if self.pending {
            lines.push(format!(
                "Awaiting reply · sent {}",
                clock::label(self.pending_at)
            ));
        }
        lines.push(format!(
            "Connected since: {}",
            clock::label(self.connected_at)
        ));
        lines.push(format!(
            "Reconnects: {} · probe every {}s",
            self.connections.saturating_sub(1),
            HEARTBEAT_INTERVAL.as_secs()
        ));
        lines.push(
            "RTT includes daemon/client processing.\nNot transfer speed or packet loss.".into(),
        );
        lines.join("\n")
    }
}
fn millis(ms: f64) -> String {
    if ms < 10. {
        format!("{ms:.1} ms")
    } else {
        format!("{ms:.0} ms")
    }
}
