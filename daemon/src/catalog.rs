//! Tau-owned, private provider model catalog. Never read Pi metadata for limits.
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use anyhow::{Context, Result, bail};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::sync::Mutex as AsyncMutex;
use crate::agent::auth::AuthStore;
use crate::settings::{Api, ProviderSettings, Settings};
use crate::state::SessionModel;

// The Codex backend gates /models by Codex's client version, not Tau's version.
// 0.156.1 is the verified release on September 24, 2026. The actual model
// windows still come exclusively from the authenticated endpoint response.
const CODEX_CATALOG_CLIENT_VERSION: &str = "0.156.1";
const RETRY_DELAY: Duration = Duration::from_secs(60);
const FETCH_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_CACHED_PROVIDERS: usize = 64;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SavedProvider {
    provider: String,
    base_url: String,
    api: Api,
    identity: String, // SHA-256 of the credential and account, never the secret itself.
    fetched_at_ms: u64,
    windows: HashMap<String, u64>,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SavedFile { schema: u32, providers: Vec<SavedProvider> }
struct Record {
    saved: Option<SavedProvider>,
    ready: bool,
    fetching: bool,
    retry_after: Option<Instant>,
}
#[derive(Default)]
struct State { records: HashMap<String, Record> }
pub(crate) struct ModelCatalog { path: PathBuf, state: Mutex<State>, write_gate: AsyncMutex<()> }

impl ModelCatalog {
    pub(crate) async fn load(path: PathBuf) -> Self {
        let mut records = HashMap::new();
        match tokio::fs::read(&path).await {
            Ok(bytes) if bytes.len() <= MAX_BYTES => match serde_json::from_slice::<SavedFile>(&bytes) {
                Ok(file) if file.schema == 1 && file.providers.len() <= MAX_CACHED_PROVIDERS => {
                    for provider in file.providers {
                        if provider.windows.len() > 20_000 || provider.windows.values().any(|w| !(1024..=100_000_000).contains(w)) { continue; }
                        records.insert(provider.provider.clone(), Record { saved:Some(provider), ready:false, fetching:false, retry_after:None });
                    }
                }
                _ => tracing::warn!("Invalid model catalog file; fetching from provider instead"),
            },
            Ok(_) => tracing::warn!("Model catalog file is too large; fetching from provider instead"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
            Err(error) => tracing::warn!(%error, "Could not read model catalog; fetching from provider instead"),
        }
        Self { path, state:Mutex::new(State { records }), write_gate:AsyncMutex::new(()) }
    }
    /// Schedule one check for an absent/changed cache; manual refresh bypasses it.
    pub(crate) fn begin(&self, provider: &str, config: &ProviderSettings, force: bool) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(record) = state.records.get_mut(provider) {
            if record.fetching || !force && (record.ready && record.saved.as_ref().is_some_and(|s| s.base_url == config.base_url && s.api == config.api)
                || record.retry_after.is_some_and(|at| at > Instant::now())) { return false; }
            record.fetching = true; return true;
        }
        if state.records.len() >= MAX_CACHED_PROVIDERS {
            // Do not fan out or retain an unbounded number of remote catalogs.
            return false;
        }
        state.records.insert(provider.into(), Record { saved:None, ready:false, fetching:true, retry_after:None });
        true
    }
    pub(crate) fn restore(&self, provider: &str, config: &ProviderSettings, identity: &str) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(record) = state.records.get_mut(provider) else { return false; };
        if record.saved.as_ref().is_some_and(|s| s.api == config.api && s.base_url == config.base_url && s.identity == identity) {
            record.ready = true;
            return true;
        }
        record.ready = false;
        false
    }
    pub(crate) fn restored(&self, provider: &str) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(record) = state.records.get_mut(provider) { record.fetching = false; record.retry_after = None; }
    }
    pub(crate) async fn save(&self, provider: &str, config: &ProviderSettings, identity: String, windows: HashMap<String, u64>) -> Result<usize> {
        let _gate = self.write_gate.lock().await;
        let saved = SavedProvider { provider:provider.into(), base_url:config.base_url.clone(), api:config.api,
            identity, fetched_at_ms:crate::agent::now_ms(), windows };
        let mut providers = {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.records.values().filter_map(|r| r.saved.clone()).filter(|s| s.provider != provider).collect::<Vec<_>>()
        };
        providers.push(saved.clone());
        let bytes = serde_json::to_vec(&SavedFile { schema:1, providers })?;
        if bytes.len() > MAX_BYTES { bail!("Model catalog file is too large"); }
        crate::settings::atomic_write(&self.path, &bytes).await?;
        let count = saved.windows.len();
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(record) = state.records.get_mut(provider) {
            record.saved = Some(saved); record.ready = true; record.fetching = false; record.retry_after = None;
        }
        Ok(count)
    }
    pub(crate) fn failed(&self, provider: &str) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(record) = state.records.get_mut(provider) {
            record.fetching = false; record.retry_after = Some(Instant::now() + RETRY_DELAY);
        }
    }
    pub(crate) fn capacity(&self, settings: &Settings, model: &SessionModel) -> Option<u64> {
        let config = settings.providers.get(&model.provider)?;
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let record = state.records.get(&model.provider)?;
        let saved = record.saved.as_ref().filter(|s| record.ready && s.api == config.api && s.base_url == config.base_url)?;
        saved.windows.get(&model.model_id).copied()
    }
    pub(crate) fn models(&self, settings: &Settings, provider: &str) -> Vec<(String, u64)> {
        let Some(config) = settings.providers.get(provider) else { return vec![]; };
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(saved) = state.records.get(provider).filter(|r| r.ready).and_then(|r| r.saved.as_ref())
            .filter(|s| s.api == config.api && s.base_url == config.base_url) else { return vec![]; };
        let mut models = saved.windows.iter().map(|(id,window)| (id.clone(), *window)).collect::<Vec<_>>();
        models.sort_by(|a,b| a.0.cmp(&b.0)); models
    }
}

pub(crate) async fn authorize(auth: &AuthStore, provider: &str, config: &ProviderSettings) -> Result<(String, Option<String>, String)> {
    let (key, account) = auth.authorization(provider, config.api_key_env.as_deref(), None).await?;
    let mut hash = Sha256::new();
    hash.update(key.as_bytes()); hash.update([0]); hash.update(account.as_deref().unwrap_or_default().as_bytes());
    let identity = URL_SAFE_NO_PAD.encode(hash.finalize());
    Ok((key, account, identity))
}
/// Bounded read-only request to the configured inference endpoint, using the same
/// credential and Codex originator. Public OpenAI /v1/models has no context size.
pub(crate) async fn fetch(http: &reqwest::Client, config: &ProviderSettings, key: &str, account: Option<&str>) -> Result<HashMap<String, u64>> {
    let url = format!("{}/models", config.base_url.trim_end_matches('/'));
    let mut request = http.get(url).bearer_auth(key).header("User-Agent", concat!("Tau/", env!("CARGO_PKG_VERSION")));
    if config.api == Api::Codex {
        request = request.query(&[("client_version", CODEX_CATALOG_CLIENT_VERSION)])
            .header("chatgpt-account-id", account.context("Codex catalog requires an account ID")?)
            .header("originator", "tau").header("OpenAI-Beta", "responses=experimental");
    }
    let mut response = request.timeout(FETCH_TIMEOUT).send().await.context("Model catalog request failed")?;
    if !response.status().is_success() { bail!("Model catalog returned HTTP {}", response.status().as_u16()); }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.context("Model catalog disconnected")? {
        if bytes.len().saturating_add(chunk.len()) > MAX_BYTES { bail!("Model catalog too large"); }
        bytes.extend_from_slice(&chunk);
    }
    let data: Value = serde_json::from_slice(&bytes).context("Invalid model catalog")?;
    let models = match config.api {
        Api::Codex => data.get("models").and_then(Value::as_array),
        Api::ChatCompletions => data.get("data").and_then(Value::as_array),
    }.context("Model catalog has no models")?;
    if models.is_empty() { bail!("Model catalog is empty"); }
    if models.len() > 20_000 { bail!("Model catalog has too many models"); }
    let mut windows = HashMap::new();
    let mut seen = HashSet::new();
    let mut duplicates = HashSet::new();
    for model in models {
        let (id, window) = match config.api {
            Api::Codex => (model.get("slug"), model.get("context_window").filter(|v| !v.is_null()).or_else(|| model.get("max_context_window"))),
            Api::ChatCompletions => (model.get("id"), model.get("context_length")),
        };
        if let Some(id) = id.and_then(Value::as_str).filter(|id| !id.is_empty()) {
            if !seen.insert(id.to_owned()) { duplicates.insert(id.to_owned()); windows.remove(id); continue; }
            if let Some(window) = window.and_then(Value::as_u64).filter(|w| (1024..=100_000_000).contains(w)) {
                windows.insert(id.to_owned(), window);
            }
        }
    }
    for id in duplicates { windows.remove(&id); }
    if windows.is_empty() { bail!("Model catalog provides no context limits"); }
    Ok(windows)
}
