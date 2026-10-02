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
#[path = "../tests/unit/codex_usage.rs"]
mod tests;
