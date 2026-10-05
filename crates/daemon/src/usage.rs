//! Read-only Codex account quota, separate from per-chat context-token usage.
use std::time::{Duration, Instant};
use anyhow::{Context, Result, bail};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;
use tau_net::{CodexUsage, CodexUsageWindow};

use crate::agent::auth::AuthStore;

const ENDPOINT: &str = "https://chatgpt.com/backend-api/wham/usage";
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_BYTES: usize = 64 * 1024;
const FRESH: Duration = Duration::from_secs(300);
const FAILURE_RETRY: Duration = Duration::from_secs(30);
const MIN_REFRESH: Duration = Duration::from_secs(10);
const MAX_STALE: Duration = Duration::from_secs(30 * 60);

#[derive(Default)]
struct State {
    identity: Option<[u8; 32]>,
    report: Option<(CodexUsage, Instant)>,
    attempted: Option<Instant>,
    error: Option<String>,
}

#[derive(Default)]
pub(crate) struct UsageReader { state: Mutex<State> }

pub(crate) struct UsageResult { pub report: Option<CodexUsage>, pub error: Option<String> }
impl UsageReader {
    /// Serialize lookups and bound refreshes across all clients, including force.
    /// Credentials are resolved before consulting a cached report, so a different
    /// login or an expired read-only shared credential cannot expose the old quota.
    pub(crate) async fn read(&self, auth: &AuthStore, http: &reqwest::Client, env: Option<&str>, force: bool) -> UsageResult {
        self.read_endpoint(auth,http,env,force,ENDPOINT).await
    }
    async fn read_endpoint(&self, auth: &AuthStore, http: &reqwest::Client, env: Option<&str>, force: bool, endpoint: &str) -> UsageResult {
        let (key, account) = match auth.authorization("openai-codex", env, None).await {
            Ok((key, Some(account))) => (key, account),
            _ => return UsageResult { report: None, error: Some("Codex quota unavailable: sign in to Codex or renew its credentials.".into()) },
        };
        let mut hash = Sha256::new();
        hash.update(key.as_bytes()); hash.update([0]); hash.update(account.as_bytes());
        let identity: [u8; 32] = hash.finalize().into();
        let mut state = self.state.lock().await;
        if state.identity != Some(identity) {
            *state = State { identity: Some(identity), ..Default::default() };
        }
        let now = Instant::now();
        let recent = state.attempted.is_some_and(|at| now.duration_since(at) < if force { MIN_REFRESH } else if state.error.is_some() { FAILURE_RETRY } else { FRESH });
        if !recent {
            state.attempted = Some(now);
            match fetch(http, endpoint, &key, &account).await {
                Ok(report) => { state.report = Some((report, Instant::now())); state.error = None; }
                Err(error) => { state.error = Some(error.to_string()); }
            }
        }
        UsageResult {
            report: state.report.as_ref().filter(|(_,at)| at.elapsed() < MAX_STALE).map(|(report,at)|{
                let mut report=report.clone();report.age_ms=at.elapsed().as_millis().try_into().unwrap_or(u64::MAX);report
            }),
            error: state.error.clone(),
        }
    }
}

async fn fetch(http: &reqwest::Client, endpoint: &str, key: &str, account: &str) -> Result<CodexUsage> {
    let mut response = http.get(endpoint).bearer_auth(key).header("chatgpt-account-id", account)
        .header("Accept", "application/json").header("User-Agent", concat!("Tau/", env!("CARGO_PKG_VERSION")))
        .timeout(FETCH_TIMEOUT).send().await.context("Codex quota request failed")?;
    if !response.status().is_success() { bail!("Codex quota unavailable (HTTP {})", response.status().as_u16()); }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.context("Codex quota request disconnected")? {
        if bytes.len().saturating_add(chunk.len()) > MAX_BYTES { bail!("Codex quota response is too large"); }
        bytes.extend_from_slice(&chunk);
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|_|anyhow::anyhow!("Codex quota response is invalid"))?;
    parse(&value).context("Codex quota response is invalid")
}

fn parse(value: &Value) -> Result<CodexUsage> {
    let data = value.as_object().context("not an object")?;
    let limit = data.get("rate_limit").and_then(Value::as_object);
    let mut windows = Vec::new();
    for (id, fallback) in [("primary_window", "Primary window"), ("secondary_window", "Secondary window")] {
        let Some(window) = limit.and_then(|rate|rate.get(id)).and_then(Value::as_object) else { continue; };
        let duration = window.get("limit_window_seconds").and_then(Value::as_u64).filter(|n| *n > 0 && *n <= 10 * 365 * 86400);
        let label = match duration {
            Some(604800) => "Weekly".into(),
            Some(n) if n % 86400 == 0 => format!("{}-day", n / 86400),
            Some(n) if n % 3600 == 0 => format!("{}-hour", n / 3600),
            Some(n) => format!("{}-minute", n.div_ceil(60)),
            None => fallback.into(),
        };
        let remaining_percent = window.get("used_percent").and_then(Value::as_f64)
            .filter(|n| n.is_finite() && *n >= 0.).map(|n| (100. - n).clamp(0., 100.));
        let resets_at_ms = window.get("reset_at").and_then(Value::as_f64)
            .filter(|n| n.is_finite() && *n > 0. && *n <= 8.64e12)
            .map(|n| (n * 1000.) as u64);
        windows.push(CodexUsageWindow { id:id.into(), label, duration_seconds:duration, remaining_percent, resets_at_ms });
    }
    let plan = data.get("plan_type").and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.len() <= 40 && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'))
        .map(str::to_owned);
    Ok(CodexUsage { provider:"openai-codex".into(), fetched_at_ms:crate::agent::now_ms(), age_ms:0, plan,
        limit_reached:limit.and_then(|rate|rate.get("limit_reached")).and_then(Value::as_bool)==Some(true), windows })
}

#[cfg(test)]
#[path = "../tests/unit/usage.rs"]
mod tests;
