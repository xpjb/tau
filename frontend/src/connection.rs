//! WebSocket heartbeat state, kept separately from presentation and wall-clock time.
use crate::store::Settings;
use std::{
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};

pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(2);
pub const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(5);
const COUNTER_REFRESH: Duration = Duration::from_millis(50);

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
    last_rtt: Option<Duration>,
    pending_since: Option<Instant>,
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
        self.last_rtt = None; // A new socket must not display the old socket's RTT.
        self.pending_since = None;
    }
    pub fn disconnected(&mut self, fatal: bool) {
        self.phase = if fatal {
            Phase::Blocked
        } else {
            Phase::Reconnecting
        };
        self.pending_since = None;
    }
    pub fn sent(&mut self, at: Instant) {
        self.pending_since = Some(at);
    }
    pub fn reply(&mut self, rtt: Duration) {
        self.pending_since = None;
        self.last_rtt = Some(rtt);
    }
    pub fn waiting_ms(&self, now: Instant) -> Option<u128> {
        self.pending_since
            .filter(|_| self.phase == Phase::Connected)
            .map(|sent| now.saturating_duration_since(sent).as_millis())
    }
    pub fn color(&self) -> u32 {
        match self.phase {
            Phase::Connected => 0x4ade80,
            Phase::Connecting | Phase::Reconnecting => 0xfbbf24,
            Phase::Blocked => 0xffb4ab,
            Phase::Offline => 0x82909f,
        }
    }
    /// Pure snapshot: callers can inject `now` for tests or a headless preview.
    pub fn details(&self, settings: &Settings, reason: &str, now: Instant) -> String {
        let title = match self.phase {
            Phase::Offline => "Offline",
            Phase::Connecting => "Connecting…",
            Phase::Connected => "Connected",
            Phase::Reconnecting => "Reconnecting…",
            Phase::Blocked => "Connection blocked",
        };
        let mut lines = vec![title.into()];
        // Show only the origin, never a token-bearing path, query, fragment or userinfo.
        if let Some(origin) = url::Url::parse(&settings.server_url)
            .ok()
            .filter(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some())
            .map(|url| url.origin().ascii_serialization())
        {
            lines.push(origin);
        }
        if self.phase == Phase::Connected {
            lines.push(match self.last_rtt {
                Some(rtt) => format!("RTT: {} ms", rtt.as_millis()),
                None => "RTT: —".into(),
            });
            if let Some(sent) = self.pending_since {
                lines.push(format!(
                    "Waiting: {} ms",
                    now.saturating_duration_since(sent).as_millis()
                ));
            }
        } else if matches!(self.phase, Phase::Blocked | Phase::Reconnecting) && !reason.is_empty() {
            lines.push(reason.into());
        }
        lines.join("\n")
    }
}

/// Wake only while an awaiting counter is visible. The on-demand renderer stays
/// asleep when the card is hidden or the ping has completed; dropping the sender
/// stops the worker. No frame-rate redraw loop runs in the normal UI.
pub struct CounterTicker {
    tx: mpsc::Sender<bool>,
    active: bool,
}
impl CounterTicker {
    pub fn new(wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("tau-connection-counter".into())
            .spawn(move || {
                let mut active = false;
                loop {
                    let message = if active {
                        rx.recv_timeout(COUNTER_REFRESH)
                    } else {
                        rx.recv().map_err(|_| mpsc::RecvTimeoutError::Disconnected)
                    };
                    match message {
                        Ok(next) => active = next,
                        Err(mpsc::RecvTimeoutError::Timeout) => wake(),
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }
            })
            .expect("start connection counter");
        Self { tx, active: false }
    }
    pub fn sync(&mut self, active: bool) {
        if self.active != active {
            self.active = active;
            let _ = self.tx.send(active);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshots_use_monotonic_elapsed_time_and_hide_credentials() {
        let now = Instant::now();
        let settings = Settings {
            server_url: "https://user:secret@example.com:8443/private?key=hidden#frag".into(),
            token: "bearer-secret".into(),
        };
        let mut health = Health::connecting();
        health.connected();
        assert!(health.details(&settings, "", now).contains("RTT: —"));
        health.sent(now);
        let waiting = health.details(&settings, "", now + Duration::from_millis(347));
        assert_eq!(
            waiting,
            "Connected\nhttps://example.com:8443\nRTT: —\nWaiting: 347 ms"
        );
        assert!(!waiting.contains("secret"));
        health.reply(Duration::from_millis(32));
        assert_eq!(
            health.details(&settings, "", now),
            "Connected\nhttps://example.com:8443\nRTT: 32 ms"
        );
        health.sent(now);
        health.disconnected(false);
        assert_eq!(health.waiting_ms(now), None);
        assert_eq!(
            health.details(&settings, "Socket closed", now),
            "Reconnecting…\nhttps://example.com:8443\nSocket closed"
        );
        health.connected();
        assert!(health.details(&settings, "", now).contains("RTT: —"));
    }
}
