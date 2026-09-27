//! Live control lifecycle and heartbeat measurements. All clocks are monotonic.
use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
pub const MIN_CONNECT_INTERVAL: Duration = Duration::from_secs(1);
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(2);
pub const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(5);
pub(crate) const COUNTER_REFRESH: Duration = Duration::from_millis(50);
const RECENT_ATTEMPTS: usize = 10;
const GREEN: u32 = 0x4ade80;
const YELLOW: u32 = 0xfbbf24;
const ORANGE: u32 = 0xfb923c;
const RED: u32 = 0xff5a5f;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Offline,
    Connecting,
    Connected,
    Blocked,
}
#[derive(Default)]
pub struct Health {
    pub phase: Phase,
    attempt: u64,
    attempt_at: Option<Instant>,
    deadline: Option<Instant>,
    retry_at: Option<Instant>,
    next_ping: Option<Instant>,
    last_ping: Option<Instant>,
    // None represents an unanswered attempt. It still uses one of the ten
    // slots, but must never be presented as an RTT sample.
    recent: VecDeque<Option<Duration>>,
    pending_since: Option<Instant>,
    last_reply: Option<Instant>,
}
impl Health {
    pub fn connecting() -> Self {
        Self {
            phase: Phase::Connecting,
            ..Self::default()
        }
    }
    pub fn attempt(&mut self, number: u64, at: Instant) {
        self.phase = Phase::Connecting;
        self.attempt = number;
        self.attempt_at = Some(at);
        self.deadline = Some(at + CONNECT_TIMEOUT);
        self.retry_at = None;
        self.pending_since = None;
    }
    pub fn retry_scheduled(&mut self, at: Instant) {
        self.phase = Phase::Connecting;
        self.retry_at = Some(at);
    }
    pub fn connected(&mut self) { self.connected_at(Instant::now()); }
    pub fn connected_at(&mut self, at: Instant) {
        self.phase = Phase::Connected;
        self.next_ping = Some(at + HEARTBEAT_INTERVAL);
        self.last_ping = None;
        self.last_reply = None;
        self.deadline = None;
        self.retry_at = None;
        self.pending_since = None;
        // RTT history survives automatic reacquisition, but a previous socket's
        // pong is never presented as evidence that this socket is responding.
    }
    pub fn disconnected(&mut self, fatal: bool) {
        self.phase = if fatal {
            Phase::Blocked
        } else {
            Phase::Connecting
        };
        self.pending_since = None;
        self.deadline = None;
        self.retry_at = None;
    }
    pub fn sent(&mut self, at: Instant) {
        self.last_ping = Some(at);
        self.next_ping = Some(at + HEARTBEAT_INTERVAL);
        self.pending_since = Some(at);
        if self.recent.len() == RECENT_ATTEMPTS {
            self.recent.pop_front();
        }
        self.recent.push_back(None);
    }
    pub fn reply(&mut self, rtt: Duration, received: Instant) {
        if self.pending_since.take().is_some() {
            *self.recent.back_mut().expect("pending attempt") = Some(rtt);
            self.last_reply = Some(received);
        }
    }
    pub fn counter(&self, now: Instant) -> Option<(&'static str, u128)> {
        let age = |at| now.saturating_duration_since(at).as_millis();
        match self.phase {
            Phase::Connecting => self.attempt_at.map(|at| ("Last attempt", age(at))),
            Phase::Connected => self.last_ping.map(|at| ("Last ping", age(at)))
                .or_else(|| self.next_ping.map(|at| ("Next ping in", at.saturating_duration_since(now).as_millis()))),
            _ => None,
        }
    }
    pub fn min_max(&self) -> Option<(Duration, Duration)> {
        let mut samples = self.recent.iter().filter_map(|sample| *sample);
        let first = samples.next()?;
        Some(samples.fold((first, first), |(min, max), rtt| {
            (min.min(rtt), max.max(rtt))
        }))
    }
    pub fn latest(&self) -> Option<Duration> {
        self.recent.iter().rev().find_map(|sample| *sample)
    }
    pub fn color(&self, now: Instant) -> u32 {
        match self.phase {
            Phase::Offline | Phase::Blocked => RED,
            Phase::Connecting => ORANGE,
            Phase::Connected => {
                let pending = self.pending_since.map(|sent| now.saturating_duration_since(sent));
                let latest = self.last_reply.and_then(|_| self.latest()).unwrap_or_default();
                let latency = latest.max(pending.unwrap_or_default());
                if latency > Duration::from_millis(3000) { return RED; }
                if latency > Duration::from_millis(1000) { return ORANGE; }
                let missed = self.recent.iter().take(self.recent.len().saturating_sub(usize::from(self.pending_since.is_some())))
                    .any(Option::is_none);
                let jitter = self.min_max().is_some_and(|(min, max)| max - min > Duration::from_millis(400));
                if self.last_reply.is_none() || latency > Duration::from_millis(800) || missed || jitter {
                    YELLOW
                } else { GREEN }
            }
        }
    }
    pub fn next_color_wake(&self, now: Instant) -> Option<Duration> {
        if self.phase != Phase::Connected { return None; }
        let pending = self.pending_since.and_then(|sent| {
            [801, 1001, 3001].into_iter().map(|ms| sent + Duration::from_millis(ms))
                .find(|at| *at > now).map(|at| at.duration_since(now))
        });
        pending
    }
    /// Pure snapshot: `now` is injectable in tests and previews.
    pub fn details(&self, reason: &str, now: Instant) -> String {
        let mut lines = vec![match self.phase {
            Phase::Offline => "No WebSocket · not configured",
            Phase::Connecting => "No WebSocket · acquiring",
            Phase::Connected => "WebSocket · connected",
            Phase::Blocked => "No WebSocket · needs attention",
        }.to_owned()];
        let remaining = |at: Instant| at.saturating_duration_since(now).as_millis();
        if self.phase == Phase::Connecting {
            if let Some(at) = self.attempt_at {
                lines.push(format!("Attempt #{} started: {}ms ago", self.attempt, now.saturating_duration_since(at).as_millis()));
            }
            if let Some(at) = self.retry_at {
                lines.push(format!("Next attempt in: {}ms", remaining(at)));
            } else if let Some(at) = self.deadline {
                lines.push(format!("Waiting · timeout in: {}ms", remaining(at)));
            }
        }
        if self.phase == Phase::Connected {
            if let Some(at) = self.last_ping {
                lines.push(format!("Last ping: {}ms ago", now.saturating_duration_since(at).as_millis()));
            }
            if let Some(at) = self.pending_since {
                lines.push(format!("Waiting for pong · timeout in: {}ms", remaining(at + HEARTBEAT_TIMEOUT)));
            } else if let Some(at) = self.next_ping {
                lines.push(format!("Next ping in: {}ms", remaining(at)));
            }
            if let Some((min, max)) = self.min_max() {
                lines.push(format!("RTT · latest {}ms · min {}ms · max {}ms{}", self.latest().unwrap().as_millis(), min.as_millis(), max.as_millis(),
                    if self.last_reply.is_none() { " (previous socket)" } else { "" }));
            } else { lines.push("RTT: awaiting first pong".into()); }
        } else if !reason.is_empty() && !matches!(reason, "Connecting…" | "Not connected" | "Connected") {
            lines.push(format!("Last failure: {}", reason.trim_end_matches('.')));
        }

        lines.join("\n")
    }
}
/// A single on-demand worker for the visible 50ms timer *or* the next hidden
/// dot color boundary. It sleeps when neither is needed and stops on drop.
pub struct CounterTicker {
    tx: mpsc::Sender<Option<Duration>>,
    scheduled: Option<Duration>,
    fired: Arc<AtomicBool>,
}
impl CounterTicker {
    pub fn new(wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        let (tx, rx) = mpsc::channel();
        let fired = Arc::new(AtomicBool::new(false));
        let worker_fired = fired.clone();
        std::thread::Builder::new()
            .name("tau-connection-counter".into())
            .spawn(move || {
                let mut delay = None;
                loop {
                    let message = if let Some(duration) = delay {
                        rx.recv_timeout(duration)
                    } else {
                        rx.recv().map_err(|_| mpsc::RecvTimeoutError::Disconnected)
                    };
                    match message {
                        Ok(next) => delay = next,
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            delay = None;
                            worker_fired.store(true, Ordering::Release);
                            wake();
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }
            })
            .expect("start connection counter");
        Self {
            tx,
            scheduled: None,
            fired,
        }
    }
    pub fn sync(&mut self, next: Option<Duration>) {
        let fired = self.fired.swap(false, Ordering::AcqRel);
        if self.scheduled != next || (fired && next.is_some()) {
            self.scheduled = next;
            let _ = self.tx.send(next);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ack(health: &mut Health, at: Instant, ms: u64) {
        health.sent(at - Duration::from_millis(ms));
        health.reply(Duration::from_millis(ms), at);
    }

    #[test]
    fn acquiring_and_pinging_share_attempt_age_and_deadline_semantics() {
        let now = Instant::now();
        let mut health = Health::default();
        health.attempt(1, now);
        let detail = health.details("", now + Duration::from_secs(1));
        assert!(detail.contains("Attempt #1 started: 1000ms ago"));
        assert!(detail.contains("Waiting · timeout in: 4000ms"));
        health.disconnected(false);
        health.retry_scheduled(now + Duration::from_secs(1));
        let detail = health.details("Connection refused", now + Duration::from_millis(100));
        assert!(detail.contains("Next attempt in: 900ms"));
        assert!(detail.contains("started: 100ms ago"));
        assert!(!detail.contains("pong") && !detail.contains("received"));
        health.attempt(2, now + Duration::from_secs(1));
        assert_eq!(health.counter(now + Duration::from_millis(1200)), Some(("Last attempt", 200)));
        health.connected_at(now);
        assert!(health.details("", now).contains("Next ping in: 2000ms"));
        for ms in 100..110 { ack(&mut health, now, ms); }
        health.sent(now);
        assert!(health.details("", now + Duration::from_secs(1)).contains("Waiting for pong · timeout in: 4000ms"));
        assert_eq!(health.min_max(), Some((Duration::from_millis(101), Duration::from_millis(109))));
        health.reply(Duration::from_secs(1), now + Duration::from_secs(1));
        let detail = health.details("", now + Duration::from_millis(1200));
        assert!(detail.contains("Last ping: 1200ms ago"));
        assert!(detail.contains("Next ping in: 800ms"));
        health.disconnected(false);
        health.connected_at(now + Duration::from_secs(2));
        assert!(!health.details("", now + Duration::from_secs(2)).contains("Last ping"));
        assert!(health.details("", now + Duration::from_secs(2)).contains("previous socket"));
        for _ in 0..10 { health.sent(now); health.disconnected(false); }
        assert_eq!(health.latest(), None, "unanswered pings never become RTT samples");
    }

    #[test]
    fn latency_bands_escalate_while_waiting_and_fail_closed_without_a_socket() {
        let now = Instant::now();
        let mut health = Health::default();
        assert_eq!(health.color(now), RED);
        health.phase = Phase::Blocked;
        assert_eq!(health.color(now), RED);
        health.connected();
        assert_eq!(health.color(now), YELLOW); // No ack yet, not proven healthy.
        health.sent(now);
        for (ms, expected) in [
            (0, YELLOW),
            (800, YELLOW),
            (801, YELLOW),
            (1000, YELLOW),
            (1001, ORANGE),
            (3000, ORANGE),
            (3001, RED),
        ] {
            assert_eq!(
                health.color(now + Duration::from_millis(ms)),
                expected,
                "{ms}ms"
            );
        }
        assert_eq!(
            health.next_color_wake(now),
            Some(Duration::from_millis(801))
        );
        assert_eq!(
            health.next_color_wake(now + Duration::from_millis(801)),
            Some(Duration::from_millis(200))
        );
        health.reply(Duration::from_millis(32), now);
        for (ms, expected) in [
            (800, GREEN),
            (801, YELLOW),
            (1000, YELLOW),
            (1001, ORANGE),
            (3000, ORANGE),
            (3001, RED),
        ] {
            health.sent(now);
            assert_eq!(
                health.color(now + Duration::from_millis(ms)),
                expected,
                "{ms}ms"
            );
            health.reply(Duration::from_millis(32), now);
        }
        ack(&mut health, now, 420);
        assert_eq!(health.color(now), GREEN);
        ack(&mut health, now, 1500);
        assert_eq!(health.color(now), ORANGE);
        ack(&mut health, now, 3100);
        assert_eq!(health.color(now), RED);
        health.disconnected(false);
        assert_eq!(health.color(now), ORANGE);
    }
    #[test]
    fn stable_latency_missed_pings_and_jitter_use_recent_windows() {
        let now = Instant::now();
        let mut health = Health::default();
        health.connected();
        for ms in [250, 500, 270, 480, 300, 450, 310, 470, 340, 420] { ack(&mut health, now, ms); }
        assert_eq!(health.color(now), GREEN);
        health.sent(now);
        assert_eq!(health.color(now + Duration::from_millis(500)), GREEN, "an in-flight probe is not loss");
        health.disconnected(false);
        health.connected();
        ack(&mut health, now, 350);
        assert_eq!(health.color(now), YELLOW, "missed probe stays in the window after reconnect");
        for _ in 0..10 { ack(&mut health, now, 350); }
        assert_eq!(health.color(now), GREEN);
        ack(&mut health, now, 799);
        assert_eq!(health.color(now), YELLOW, "large jitter below the latency limit");
        for _ in 0..10 { ack(&mut health, now, 799); }
        assert_eq!(health.color(now), GREEN);

    }

}
