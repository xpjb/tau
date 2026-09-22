use std::path::PathBuf;
use std::sync::Arc;
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::sync::Mutex;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};

#[derive(Clone)]
pub struct AuthStore { path: PathBuf, gate: Arc<Mutex<()>>, http: reqwest::Client }
impl AuthStore {
    pub fn new(path: PathBuf, http: reqwest::Client) -> Self { Self { path, gate: Arc::new(Mutex::new(())), http } }
    pub async fn authorization(&self, provider: &str, env: Option<&str>, rejected: Option<&str>) -> Result<(String, Option<String>)> {
        if let Some(key) = env.and_then(|name| std::env::var(name).ok()).filter(|key| !key.is_empty()) { return Ok((key, None)); }
        let this = self.clone(); let provider = provider.to_owned(); let rejected = rejected.map(str::to_owned);
        // A cancelled model call must not interrupt a rotating refresh-token write.
        tokio::spawn(async move {
            let _guard = this.gate.lock().await;
            let bytes = tokio::fs::read(&this.path).await.context("No daemon credentials; import Pi auth.json or configure an API key environment variable")?;
            if bytes.len() > 1024 * 1024 { bail!("Credential file is too large"); }
            let mut root: Value = serde_json::from_slice(&bytes).map_err(|_| anyhow::anyhow!("Invalid credential file (values withheld)"))?;
            let record = root.get_mut(&provider).context("Provider has no daemon credentials")?;
            if record["type"] == "api_key" {
                return Ok((record["key"].as_str().filter(|v| !v.is_empty()).context("API key is empty")?.to_owned(), None));
            }
            if record["type"] != "oauth" || provider != "openai-codex" { bail!("Unsupported credential type"); }
            let now = super::now_ms();
            if record["expires"].as_u64().unwrap_or(0) <= now + 60_000 || rejected.as_deref() == record["access"].as_str() {
                let refresh = record["refresh"].as_str().context("Missing Codex refresh token")?;
                let mut response = this.http.post("https://auth.openai.com/oauth/token")
                    .timeout(std::time::Duration::from_secs(30)).form(&[("grant_type", "refresh_token"), ("refresh_token", refresh),
                        ("client_id", "app_EMoamEEZ73f0CkXaXp7hrann")]).send().await.context("Codex token refresh failed")?;
                if !response.status().is_success() { bail!("Codex token refresh returned HTTP {}; sign in again", response.status().as_u16()); }
                let mut bytes = Vec::new();
                while let Some(chunk) = response.chunk().await? {
                    if bytes.len() + chunk.len() > 64 * 1024 { bail!("OAuth response too large"); }
                    bytes.extend_from_slice(&chunk);
                }
                let value: Value = serde_json::from_slice(&bytes).map_err(|_| anyhow::anyhow!("Invalid OAuth response (values withheld)"))?;
                let access = value["access_token"].as_str().context("OAuth returned no access token")?;
                let claims: Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(access.split('.').nth(1).context("Token has no account claims")?)
                    .map_err(|_| anyhow::anyhow!("Invalid token claims"))?).map_err(|_| anyhow::anyhow!("Invalid account claims"))?;
                let account = claims.pointer("/https:~1~1api.openai.com~1auth/chatgpt_account_id").and_then(Value::as_str).context("Token has no account ID")?;
                *record = json!({"type":"oauth", "access":access, "refresh":value["refresh_token"].as_str().unwrap_or(refresh),
                    "expires":now.saturating_add(value["expires_in"].as_u64().context("OAuth returned no lifetime")?.saturating_mul(1000)), "accountId":account});
                crate::settings::atomic_write(&this.path, &serde_json::to_vec_pretty(&root)?).await?;
            }
            let record = &root[&provider];
            Ok((record["access"].as_str().filter(|v| !v.is_empty()).context("Missing access token")?.to_owned(),
                Some(record["accountId"].as_str().filter(|v| !v.is_empty()).context("Missing account ID")?.to_owned())))
        }).await.context("Credential refresh task stopped")?
    }
}
