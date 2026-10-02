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
mod tests {
    use super::*;
    use axum::{Router, routing::get, extract::State as AxumState, http::{HeaderMap, StatusCode}};
    use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
    use serde_json::json;

    #[test]
    fn maps_taus_existing_quota_windows_and_handles_missing_fields() {
        let report = parse(&json!({"plan_type":"pro", "rate_limit":{"limit_reached":true,
            "primary_window":{"limit_window_seconds":18000,"used_percent":26,"reset_at":1800000000},
            "secondary_window":{"limit_window_seconds":604800,"used_percent":46.5,"reset_at":1800600000}}})).unwrap();
        assert_eq!(report.plan.as_deref(),Some("pro")); assert!(report.limit_reached);
        assert_eq!(report.windows[0].label,"5-hour"); assert_eq!(report.windows[0].remaining_percent,Some(74.));
        assert_eq!(report.windows[0].resets_at_ms,Some(1_800_000_000_000));
        assert_eq!(report.windows[1].label,"Weekly"); assert_eq!(report.windows[1].remaining_percent,Some(53.5));
        let sparse=parse(&json!({"plan_type":"<unsafe>","rate_limit":{"primary_window":{"used_percent":null,"reset_at":-1},
            "secondary_window":{"used_percent":150}}})).unwrap();
        assert_eq!(sparse.plan,None);assert_eq!(sparse.windows[0].label,"Primary window");
        assert_eq!(sparse.windows[0].remaining_percent,None);assert_eq!(sparse.windows[0].resets_at_ms,None);
        assert_eq!(sparse.windows[1].remaining_percent,Some(0.));
        assert!(parse(&json!({})).unwrap().windows.is_empty());
        let changed=parse(&json!({"rate_limit":{"secondary_window":{"limit_window_seconds":86400,"used_percent":0}}})).unwrap();
        assert_eq!(changed.windows.len(),1);assert_eq!(changed.windows[0].label,"1-day");
        assert_eq!(changed.windows[0].remaining_percent,Some(100.));
        assert!(parse(&json!([])).is_err());
    }

    #[tokio::test]
    async fn cache_is_account_scoped_refresh_bounded_and_keeps_old_values_only_for_same_login() {
        let root=tempfile::tempdir().unwrap();let path=root.path().join("auth.json");
        let save=|key:&str|serde_json::to_vec(&json!({"openai-codex":{"type":"oauth","access":key,"refresh":"unused","accountId":"fixture-account","expires":u64::MAX}})).unwrap();
        crate::settings::atomic_write(&path,&save("fixture-access")).await.unwrap();
        let calls=Arc::new(AtomicUsize::new(0));let count=calls.clone();
        let fail=Arc::new(std::sync::atomic::AtomicBool::new(false));let failure=fail.clone();
        let app=Router::new().route("/usage",get(move |headers:HeaderMap| {
            let count=count.clone();let failure=failure.clone();async move {
                count.fetch_add(1,Ordering::SeqCst);
                assert_eq!(headers["chatgpt-account-id"],"fixture-account");
                if failure.load(Ordering::SeqCst) {(StatusCode::SERVICE_UNAVAILABLE,axum::Json(json!({}))) }
                else {(StatusCode::OK,axum::Json(json!({"rate_limit":{"primary_window":{"used_percent":20},"secondary_window":{"used_percent":30}}}))) }
            }
        }));
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let addr=listener.local_addr().unwrap();
        let server=tokio::spawn(async move {axum::serve(listener,app).await.unwrap();});
        let http=reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap();
        let auth=AuthStore::new(path.clone(),http.clone());let reader=UsageReader::default();let url=format!("http://{addr}/usage");
        let first=reader.read_endpoint(&auth,&http,None,false,&url).await;
        assert!(first.error.is_none());assert_eq!(first.report.unwrap().windows.len(),2);
        reader.read_endpoint(&auth,&http,None,true,&url).await;
        assert_eq!(calls.load(Ordering::SeqCst),1,"Forced refresh is bounded too");
        reader.state.lock().await.attempted=Some(Instant::now()-MIN_REFRESH);
        fail.store(true,Ordering::SeqCst);
        let stale=reader.read_endpoint(&auth,&http,None,true,&url).await;
        assert_eq!(stale.error.as_deref(),Some("Codex quota unavailable (HTTP 503)"));
        assert!(stale.report.is_some(),"A transient error preserves same-account last known data");
        assert_eq!(calls.load(Ordering::SeqCst),2);
        reader.read_endpoint(&auth,&http,None,false,&url).await;
        assert_eq!(calls.load(Ordering::SeqCst),2,"Failed lookups have a cooldown");
        crate::settings::atomic_write(&path,&save("replacement-login")).await.unwrap();
        let changed=reader.read_endpoint(&auth,&http,None,false,&url).await;
        assert!(changed.report.is_none(),"Old account quota must not survive a credential change");
        assert!(changed.error.is_some());assert_eq!(calls.load(Ordering::SeqCst),3);
        tokio::fs::remove_file(&path).await.unwrap();
        let missing=reader.read_endpoint(&auth,&http,None,false,&url).await;
        assert!(missing.report.is_none());assert!(missing.error.unwrap().contains("sign in"));
        let shared=root.path().join("shared.json");
        crate::settings::atomic_write(&shared,&serde_json::to_vec(&json!({"openai-codex":{
            "type":"oauth","access":"stale-shared-token","refresh":"never-use","accountId":"fixture-account","expires":1}})).unwrap()).await.unwrap();
        let readonly=AuthStore::new(root.path().join("beta.json"),http.clone()).shared_codex(Some(shared));
        let expired=reader.read_endpoint(&readonly,&http,None,true,&url).await;
        assert!(expired.report.is_none());assert!(expired.error.unwrap().contains("renew"));
        assert_eq!(calls.load(Ordering::SeqCst),3,"Expiry must not query quota or rotate the shared login");
        server.abort();
    }

    #[tokio::test]
    async fn reads_only_quota_with_headers_and_bounds_errors_without_exposing_credentials() {
        let calls=Arc::new(AtomicUsize::new(0));let counter=calls.clone();
        let app=Router::new().route("/usage",get(move |AxumState(c):AxumState<Arc<AtomicUsize>>,headers:HeaderMap| async move {
            c.fetch_add(1,Ordering::SeqCst);
            assert_eq!(headers["authorization"],"Bearer fixture-access");
            assert_eq!(headers["chatgpt-account-id"],"fixture-account");
            (StatusCode::OK, axum::Json(json!({"rate_limit":{"primary_window":{"used_percent":25}}})))
        })).with_state(counter);
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let addr=listener.local_addr().unwrap();
        let server=tokio::spawn(async move {axum::serve(listener,app).await.unwrap();});
        let http=reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap();
        let url=format!("http://{addr}/usage");
        let report=fetch(&http,&url,"fixture-access","fixture-account").await.unwrap();
        assert_eq!(report.windows[0].remaining_percent,Some(75.));assert_eq!(calls.load(Ordering::SeqCst),1);
        server.abort();
        let app=Router::new().route("/usage",get(||async {(StatusCode::UNAUTHORIZED,"fixture-access must never be reflected")}));
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let addr=listener.local_addr().unwrap();
        let server=tokio::spawn(async move {axum::serve(listener,app).await.unwrap();});
        let error=fetch(&http,&format!("http://{addr}/usage"),"fixture-access","fixture-account").await.unwrap_err().to_string();
        assert_eq!(error,"Codex quota unavailable (HTTP 401)");server.abort();
    }
}
