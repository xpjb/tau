//! WebSocket heartbeat measurements. Times are monotonic; no saved settings are displayed.
use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

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
    Reconnecting,
    Blocked,
}
#[derive(Default)]
pub struct Health {
    pub phase: Phase,
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
    pub fn connected(&mut self) {
        self.phase = Phase::Connected;
        self.pending_since = None;
        // Retain the ten recent attempts and last received time for the same
        // configured server across automatic reconnects; configure() resets us.
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
        if self.phase == Phase::Connected
            && let Some(sent) = self.pending_since
        {
            return Some(("waiting", now.saturating_duration_since(sent).as_millis()));
        }
        self.last_reply.map(|received| {
            (
                "received",
                now.saturating_duration_since(received).as_millis(),
            )
        })
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
            Phase::Offline | Phase::Blocked | Phase::Reconnecting => RED,
            Phase::Connecting => ORANGE,
            Phase::Connected => {
                let recent = self.latest();
                let pending = self
                    .pending_since
                    .map(|sent| now.saturating_duration_since(sent));
                match (recent, pending) {
                    (Some(last), Some(waiting)) => latency_color(last.max(waiting)),
                    (Some(last), None) => latency_color(last),
                    (None, Some(waiting)) => {
                        // A fresh socket is unverified until its first pong.
                        if waiting.as_millis() <= 250 {
                            YELLOW
                        } else {
                            latency_color(waiting)
                        }
                    }
                    (None, None) => YELLOW,
                }
            }
        }
    }
    /// When the card is hidden, wake only as a pending ping crosses a color
    /// boundary. The network worker handles the independent 5s timeout.
    pub fn next_color_wake(&self, now: Instant) -> Option<Duration> {
        let sent = self
            .pending_since
            .filter(|_| self.phase == Phase::Connected)?;
        let waited = now.saturating_duration_since(sent).as_millis();
        [251u64, 1001, 3001]
            .into_iter()
            .find(|edge| u128::from(*edge) > waited)
            .map(|edge| Duration::from_millis((u128::from(edge) - waited) as u64))
    }
    /// Pure snapshot: `now` is injectable in tests and previews.
    pub fn details(&self, reason: &str, now: Instant) -> String {
        let title = match self.phase {
            Phase::Offline => Some("Offline"),
            Phase::Connecting => Some("Connecting…"),
            Phase::Connected => None, // The live reply/wait timer says more than "Connected".
            Phase::Reconnecting => Some("Reconnecting…"),
            Phase::Blocked => Some("Connection blocked"),
        };
        let (min, max) = match self.min_max() {
            Some((min, max)) => (
                format!("{}ms", min.as_millis()),
                format!("{}ms", max.as_millis()),
            ),
            None => ("—".into(), "—".into()),
        };
        let mut lines = Vec::new();
        if let Some(title) = title {
            lines.push(title.into());
        }
        let latest = self.latest().map_or_else(|| "—".into(), |rtt| format!("{}ms", rtt.as_millis()));
        lines.extend([format!("min: {min}"), format!("max: {max}"), format!("latest: {latest}")]);
        lines.push(match self.counter(now) {
            Some((label, ms)) => format!("{label}: {ms}ms"),
            None => "received: —".into(),
        });
        if matches!(self.phase, Phase::Blocked | Phase::Reconnecting) && !reason.is_empty() {
            lines.push(
                reason
                    .trim_end_matches(" Reconnecting…")
                    .trim_end_matches('.')
                    .into(),
            );
        }
        lines.join("\n")
    }
}
fn latency_color(rtt: Duration) -> u32 {
    match rtt.as_millis() {
        0..=250 => GREEN,
        251..=1000 => YELLOW,
        1001..=3000 => ORANGE,
        _ => RED,
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
    fn last_ten_attempts_exclude_unanswered_rtts_and_keep_last_ack_clock() {
        let now = Instant::now();
        let mut health = Health::connecting();
        health.connected();
        assert_eq!(health.details("", now), "min: —\nmax: —\nlatest: —\nreceived: —");
        for ms in 100..110 {
            ack(&mut health, now, ms);
        }
        assert_eq!(
            health.min_max(),
            Some((Duration::from_millis(100), Duration::from_millis(109)))
        );
        health.sent(now);
        assert_eq!(
            health.min_max(),
            Some((Duration::from_millis(101), Duration::from_millis(109)))
        );
        assert_eq!(
            health.details("", now + Duration::from_millis(347)),
            "min: 101ms\nmax: 109ms\nlatest: 109ms\nwaiting: 347ms"
        );
        health.disconnected(false); // The unanswered attempt is not a fabricated 5s RTT.
        assert_eq!(
            health.details(
                "Ping timed out. Reconnecting…",
                now + Duration::from_secs(5)
            ),
            "Reconnecting…\nmin: 101ms\nmax: 109ms\nlatest: 109ms\nreceived: 5000ms\nPing timed out"
        );
        health.connected();
        assert_eq!(
            health.counter(now + Duration::from_secs(6)),
            Some(("received", 6000))
        );
        for _ in 0..10 {
            health.sent(now);
            health.disconnected(false);
            health.connected();
        }
        assert_eq!(health.min_max(), None, "only the last ten attempts count");
        assert_eq!(health.latest(), None, "expired acknowledgements cannot masquerade as latest");
        assert_eq!(
            health.counter(now + Duration::from_secs(6)),
            Some(("received", 6000))
        );
    }

    #[test]
    fn latest_is_the_last_acknowledged_rtt_not_an_extreme_or_pending_wait() {
        let now = Instant::now();
        let mut health = Health::connecting();
        health.connected();
        for ms in [120, 400, 210] {
            ack(&mut health, now, ms);
        }
        assert_eq!(health.min_max(), Some((Duration::from_millis(120), Duration::from_millis(400))));
        assert_eq!(health.latest(), Some(Duration::from_millis(210)));
        health.sent(now);
        assert_eq!(health.latest(), Some(Duration::from_millis(210)));
        assert_eq!(health.details("", now + Duration::from_millis(50)),
            "min: 120ms\nmax: 400ms\nlatest: 210ms\nwaiting: 50ms");
        health.disconnected(false);
        health.connected();
        assert_eq!(health.latest(), Some(Duration::from_millis(210)));
    }

    #[test]
    fn waiting_and_ack_counters_use_monotonic_send_and_receive_instants() {
        let now = Instant::now();
        let mut health = Health::connecting();
        health.connected();
        health.sent(now);
        assert_eq!(
            health.counter(now + Duration::from_millis(4321)),
            Some(("waiting", 4321))
        );
        health.reply(
            Duration::from_millis(4321),
            now + Duration::from_millis(4321),
        );
        assert_eq!(
            health.counter(now + Duration::from_millis(5555)),
            Some(("received", 1234))
        );
        assert_eq!(
            health.details("", now + Duration::from_millis(5555)),
            "min: 4321ms\nmax: 4321ms\nlatest: 4321ms\nreceived: 1234ms"
        );
        health.sent(now + Duration::from_millis(5555));
        assert_eq!(
            health.counter(now + Duration::from_millis(5555)),
            Some(("waiting", 0))
        );
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
            (250, YELLOW),
            (251, YELLOW),
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
            Some(Duration::from_millis(251))
        );
        assert_eq!(
            health.next_color_wake(now + Duration::from_millis(251)),
            Some(Duration::from_millis(750))
        );
        health.reply(Duration::from_millis(32), now);
        for (ms, expected) in [
            (250, GREEN),
            (251, YELLOW),
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
        assert_eq!(health.color(now), YELLOW);
        ack(&mut health, now, 1500);
        assert_eq!(health.color(now), ORANGE);
        ack(&mut health, now, 3100);
        assert_eq!(health.color(now), RED);
        health.disconnected(false);
        assert_eq!(health.color(now), RED);
    }
}
