//! Account quota UI state. This never changes the session's context-token gauge.
use std::time::{Duration, Instant};
use tau_net::CodexUsage;
use crate::tooltip::{Content, INK, MUTED, GOOD, WARNING, DANGER};

const FRESH: Duration = Duration::from_secs(300);
const TIMEOUT: Duration = Duration::from_secs(15);
const RETRY: Duration = Duration::from_secs(30);
const MAX_STALE: Duration = Duration::from_secs(30 * 60);

#[derive(Default)]
pub struct UsageView {
    pub report: Option<CodexUsage>,
    pub error: Option<String>,
    pub in_flight: Option<(String, Instant)>,
    pub attempted: Option<Instant>,
    pub received: Option<Instant>,
}
impl UsageView {
    pub fn clear(&mut self) { *self=Self::default(); }
    pub fn offline(&mut self) { self.in_flight=None; }
    pub fn needs_refresh(&mut self) -> bool {
        if let Some((_,at))=&self.in_flight {
            if at.elapsed()<TIMEOUT {return false;}
            self.in_flight=None;
            self.error=Some("Codex quota request timed out; retrying automatically.".into());
        }
        if self.attempted.is_some_and(|at|at.elapsed()<if self.error.is_some() {RETRY} else {FRESH}) {return false;}
        true
    }
    pub fn complete(&mut self, id: &str, report: Option<CodexUsage>, error: Option<String>) -> bool {
        if self.in_flight.as_ref().is_none_or(|(expected,_)|expected!=id) {return false;}
        self.in_flight=None;
        if let Some(report)=report.filter(|report|report.provider=="openai-codex") {
            self.received=Some(Instant::now().checked_sub(Duration::from_millis(report.age_ms)).unwrap_or_else(Instant::now));
            self.report=Some(report);
        } else {
            self.report=None;self.received=None;
        }
        self.error=error;
        true
    }
    pub fn content(&self, connected: bool) -> Content {
        let mut content = Content::default();
        content.strong("Codex quota", INK);
        let Some(report)=self.report.as_ref().filter(|_|self.received.is_some_and(|at|at.elapsed()<MAX_STALE)) else {
            content.line();
            if !connected { content.dim("Unavailable · offline"); }
            else if let Some(error) = &self.error { content.push(error, false, WARNING); }
            else { content.dim("Reading…"); }
            return content;
        };
        let stale=!connected || self.error.is_some() || self.received.is_some_and(|at|at.elapsed()>=FRESH);
        if let Some(plan) = &report.plan { content.dim(format!(" · {plan}")); }
        if stale { content.dim(" · last known"); }
        if report.windows.is_empty() { content.line().dim("Quota windows unavailable"); }
        for window in &report.windows {
            content.line().push(&window.label, false, INK).dim(" · ");
            if let Some(value) = window.remaining_percent {
                let tint = if stale { MUTED } else if value <= 10. { DANGER } else if value <= 25. { WARNING } else { GOOD };
                content.strong(percent(value), tint).dim(" remaining");
            } else { content.dim("Usage unavailable"); }
            if let Some(reset)=window.resets_at_ms.and_then(|reset|reset_in(report.fetched_at_ms,self.received.unwrap(),reset)) {
                content.line().dim("Resets in ").strong(reset, MUTED);
            }
        }
        if report.limit_reached { content.line().strong("Codex limit reached", DANGER); }
        if let Some(error)=&self.error { content.line().push(error, false, WARNING); }
        content
    }
}
fn percent(value:f64)->String {format!("{}%",if (value*10.).round()%10.==0. {format!("{value:.0}")} else {format!("{value:.1}")})}

fn reset_in(fetched:u64, received:Instant, reset:u64)->Option<String> {
    let elapsed=u64::try_from(received.elapsed().as_millis()).unwrap_or(u64::MAX);
    let remaining=reset.checked_sub(fetched.saturating_add(elapsed))?;
    if remaining==0 {return None;}
    let minutes=remaining.saturating_add(59_999)/60_000;
    Some(if minutes>=1440 {format!("{}d {}h",minutes/1440,(minutes%1440)/60)}
        else if minutes>=60 {format!("{}h {}m",minutes/60,minutes%60)} else {format!("{minutes}m")})
}

#[cfg(test)]
mod tests {
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
}
