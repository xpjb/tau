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
