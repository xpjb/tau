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
        model: None, thinking_level: None,
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
