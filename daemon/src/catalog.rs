//! Read-only context limits from the same provider endpoint used for inference.
//! A configured limit is only a visibly unverified fallback when discovery fails;
//! a successful catalog with no exact model/window must not inherit one.
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use anyhow::{Context, Result, bail};
use serde_json::Value;
use crate::agent::auth::AuthStore;
use crate::settings::{Api, ProviderSettings, Settings};
use crate::state::SessionModel;

// The Codex backend gates /models by a Codex client version, not Tau's app version.
// 0.156.1 is the verified Codex release as of September 24, 2026. With the
// same OAuth account and originator as Tau's inference calls, its catalog lists
// GPT-6 Sol/Luna/Astra. Update this protocol version with a verified release;
// it supplies no model IDs or capacities itself.
const CODEX_CATALOG_CLIENT_VERSION: &str = "0.156.1";
const CATALOG_TTL: Duration = Duration::from_secs(3600);
const RETRY_DELAY: Duration = Duration::from_secs(60);
const FETCH_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_CACHED_PROVIDERS: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CapacitySource { Provider, Configured }

#[derive(Default)]
pub(crate) struct ModelCatalog { records: Mutex<HashMap<String, Record>> }
struct Record {
    base_url: String,
    api: Api,
    until: Instant,
    fetching: bool,
    windows: Option<HashMap<String, u64>>,
}
impl ModelCatalog {
    /// Return true exactly once per provider/endpoint and refresh period.
    pub(crate) fn begin(&self, provider: &str, config: &ProviderSettings) -> bool {
        let mut records = self.records.lock().unwrap_or_else(|e| e.into_inner());
        if records.get(provider).is_some_and(|r| r.api == config.api && r.base_url == config.base_url
            && (r.fetching || r.until > Instant::now())) { return false; }
        if !records.contains_key(provider) && records.len() >= MAX_CACHED_PROVIDERS {
            let oldest = records.iter().filter(|(_,r)| !r.fetching)
                .min_by_key(|(_,r)| r.until).map(|(name,_)| name.clone());
            if let Some(oldest) = oldest { records.remove(&oldest); } else { return false; }
        }
        records.insert(provider.into(), Record { base_url:config.base_url.clone(), api:config.api,
            until:Instant::now(), fetching:true, windows:None });
        true
    }
    pub(crate) fn finish(&self, provider: &str, config: &ProviderSettings, windows: Option<HashMap<String, u64>>) {
        let mut records = self.records.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(record) = records.get_mut(provider).filter(|r| r.api == config.api && r.base_url == config.base_url) {
            record.until = Instant::now() + if windows.is_some() { CATALOG_TTL } else { RETRY_DELAY };
            record.fetching = false;
            record.windows = windows;
        }
    }
    pub(crate) fn capacity(&self, settings: &Settings, model: &SessionModel) -> Option<(u64, CapacitySource)> {
        let config = settings.providers.get(&model.provider)?;
        let records = self.records.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(record) = records.get(&model.provider).filter(|r| r.api == config.api && r.base_url == config.base_url
            && r.until > Instant::now() && r.windows.is_some()) {
            // An exact catalog response is authoritative, including the absence of
            // a window or model. Do not borrow limits from a different model.
            return record.windows.as_ref().unwrap().get(&model.model_id).copied().map(|n| (n, CapacitySource::Provider));
        }
        if !settings.agent.allow_configured_context_fallback { return None; }
        settings.models.iter().find(|item| item.provider == model.provider && item.id == model.model_id)
            .and_then(|item| item.context_window).map(|n| (n, CapacitySource::Configured))
    }
}

/// Use only the configured endpoint and the same credential/originator as chat.
/// Neither a public /v1/models listing nor another provider's catalog supplies
/// a Codex model's context window.
pub(crate) async fn fetch(http: &reqwest::Client, auth: &AuthStore, provider: &str, config: &ProviderSettings) -> Result<HashMap<String, u64>> {
    let (key, account) = auth.authorization(provider, config.api_key_env.as_deref(), None).await?;
    let url = format!("{}/models", config.base_url.trim_end_matches('/'));
    let mut request = http.get(url).bearer_auth(&key).header("User-Agent", concat!("Tau/", env!("CARGO_PKG_VERSION")));
    if config.api == Api::Codex {
        request = request.query(&[("client_version", CODEX_CATALOG_CLIENT_VERSION)])
            .header("chatgpt-account-id", account.as_deref().context("Codex catalog requires an account ID")?)
            .header("originator", "tau").header("OpenAI-Beta", "responses=experimental");
    }
    let mut response = request.timeout(FETCH_TIMEOUT).send().await.context("Model catalog request failed")?;
    if !response.status().is_success() { bail!("Model catalog unavailable"); }
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
            // Codex distinguishes effective context_window from an override cap.
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn provider_catalog_is_authoritative_and_settings_fallback_requires_opt_in() {
        let catalog = ModelCatalog::default();
        let mut settings = Settings::default();
        settings.models[0].context_window = Some(272_000); // Deliberate opt-in fixture, not a default.
        let model = settings.agent.model.clone();
        assert!(catalog.capacity(&settings, &model).is_none());
        settings.agent.allow_configured_context_fallback = true;
        assert_eq!(catalog.capacity(&settings, &model), Some((272_000, CapacitySource::Configured)));
        let provider = &settings.providers[&model.provider];
        assert!(catalog.begin(&model.provider, provider));
        assert!(!catalog.begin(&model.provider, provider));
        catalog.finish(&model.provider, provider, Some(HashMap::from([("another-model".into(), 128_000)])));
        assert_eq!(catalog.capacity(&settings, &model), None, "A successful catalog without the exact model must not borrow settings metadata");
        assert!(!catalog.begin(&model.provider, provider), "Fresh catalog is cached");
    }
}
