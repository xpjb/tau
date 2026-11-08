use super::*;
use tau_net::CodexUsageWindow;
#[test]
fn automatic_refresh_is_bounded_and_timeouts_retry_without_a_button() {
    let mut view = UsageView::default();
    assert!(view.needs_refresh());
    view.attempted = Some(Instant::now());
    assert!(!view.needs_refresh());
    view.attempted = Some(Instant::now() - FRESH);
    assert!(view.needs_refresh());
    view.in_flight = Some(("pending".into(), Instant::now()));
    assert!(!view.needs_refresh(), "Only one read may be in flight");
    let started = Instant::now() - TIMEOUT;
    view.attempted = Some(started);
    view.in_flight = Some(("timeout".into(), started));
    assert!(!view.needs_refresh(), "Timeout keeps the retry cooldown");
    assert!(view.in_flight.is_none());
    assert!(view.error.as_deref().unwrap().contains("retrying automatically"));
    view.attempted = Some(Instant::now() - RETRY);
    assert!(view.needs_refresh());
    view.in_flight = Some(("retry".into(), Instant::now()));
    assert!(!view.complete("timeout", None, Some("late error".into())));
    view.offline();
    assert!(view.in_flight.is_none());
    view.clear();
    assert!(view.report.is_none() && view.attempted.is_none());
}

#[test]
fn quota_is_separate_from_context_and_reports_missing_stale_and_reset_windows() {
    let mut view=UsageView::default();
    assert_eq!(view.content(false).text,"Codex quota\nUnavailable · offline");
    view.in_flight=Some(("first".into(),Instant::now()));
    let report=CodexUsage {provider:"openai-codex".into(),plan:Some("pro".into()),fetched_at_ms:1_800_000_000_000,age_ms:0,limit_reached:false,
        windows:vec![CodexUsageWindow {id:"primary_window".into(),label:"5-hour".into(),duration_seconds:Some(18000),remaining_percent:Some(74.),resets_at_ms:Some(1_800_000_120_000)}]};
    assert!(!view.complete("old",Some(report.clone()),None));
    assert!(view.complete("first",Some(report),None));
    assert_eq!(view.content(true).text,"Codex quota · pro\n5-hour · 74% remaining\nResets in 2m");
    view.in_flight=Some(("second".into(),Instant::now()));
    let stale=view.report.clone().map(|mut report|{report.age_ms=300_000;report});
    assert!(view.complete("second",stale,Some("Codex quota unavailable (HTTP 503)".into())));
    assert!(view.content(true).text.contains("last known"));
    assert!(view.content(true).text.contains("HTTP 503"));
    view.report.as_mut().unwrap().windows.clear();
    assert!(view.content(true).text.contains("Quota windows unavailable"));
    view.received=Some(Instant::now()-MAX_STALE);
    assert_eq!(view.content(true).text,"Codex quota\nCodex quota unavailable (HTTP 503)");
    view.in_flight=Some(("third".into(),Instant::now()));
    assert!(view.complete("third",None,Some("Codex quota unavailable (HTTP 401)".into())));
    assert_eq!(view.content(true).text,"Codex quota\nCodex quota unavailable (HTTP 401)");
}
