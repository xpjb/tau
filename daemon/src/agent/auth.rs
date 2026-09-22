use std::path::PathBuf;
use std::sync::Arc;
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::sync::Mutex;
use tokio::io::AsyncReadExt;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};

const ISSUER: &str = "https://auth.openai.com";
const CLIENT: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
#[derive(Clone)]
pub struct AuthStore { path: PathBuf, gate: Arc<Mutex<()>>, http: reqwest::Client }
impl AuthStore {
    pub fn new(path: PathBuf, http: reqwest::Client) -> Self { Self { path, gate:Arc::new(Mutex::new(())), http } }
    async fn read(&self) -> Result<Value> {
        let mut options = tokio::fs::OpenOptions::new(); options.read(true);
        #[cfg(unix)] options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        let file = options.open(&self.path).await.context("No daemon credentials; use taud --login-codex, import Pi auth.json, or configure an API key environment variable")?;
        let metadata = file.metadata().await?;
        if !metadata.is_file() { bail!("Credentials must be a regular file"); }
        #[cfg(unix)] {
            use std::os::unix::fs::MetadataExt;
            if metadata.mode() & 0o077 != 0 || metadata.uid() != unsafe {libc::geteuid()} { bail!("Credentials must be owned by the daemon user and private (mode 0600)"); }
        }
        let mut bytes = Vec::new(); file.take(1024 * 1024 + 1).read_to_end(&mut bytes).await?;
        if bytes.len() > 1024 * 1024 { bail!("Credential file is too large"); }
        serde_json::from_slice(&bytes).map_err(|_| anyhow::anyhow!("Invalid credential file (values withheld)"))
    }
    pub async fn authorization(&self, provider: &str, env: Option<&str>, rejected: Option<&str>) -> Result<(String, Option<String>)> {
        if let Some(key) = env.and_then(|name| std::env::var(name).ok()).filter(|key| !key.is_empty()) { return Ok((key, None)); }
        let this = self.clone(); let provider = provider.to_owned(); let rejected = rejected.map(str::to_owned);
        // A cancelled model call must not interrupt a rotating refresh-token write.
        tokio::spawn(async move {
            let _guard = this.gate.lock().await;
            let mut root = this.read().await?;
            let record = root.get_mut(&provider).context("Provider has no daemon credentials")?;
            if record["type"] == "api_key" {
                return Ok((record["key"].as_str().filter(|v| !v.is_empty()).context("API key is empty")?.to_owned(), None));
            }
            if record["type"] != "oauth" || provider != "openai-codex" { bail!("Unsupported credential type"); }
            if record["expires"].as_u64().unwrap_or(0) <= super::now_ms() + 60_000 || rejected.as_deref() == record["access"].as_str() {
                let refresh = record["refresh"].as_str().context("Missing Codex refresh token")?;
                let (status, value) = auth_json(this.http.post(format!("{ISSUER}/oauth/token"))
                    .form(&[("grant_type", "refresh_token"), ("refresh_token", refresh), ("client_id", CLIENT)])).await?;
                if !status.is_success() { bail!("Codex token refresh returned HTTP {}; sign in again", status.as_u16()); }
                *record = credentials(&value, Some(refresh))?;
                crate::settings::atomic_write(&this.path, &serde_json::to_vec_pretty(&root)?).await?;
            }
            let record = &root[&provider];
            Ok((record["access"].as_str().filter(|v| !v.is_empty()).context("Missing access token")?.to_owned(),
                Some(record["accountId"].as_str().filter(|v| !v.is_empty()).context("Missing account ID")?.to_owned())))
        }).await.context("Credential refresh task stopped")?
    }
    pub async fn login(&self) -> Result<()> {
        let _guard = self.gate.lock().await;
        let mut root = if self.path.try_exists()? { self.read().await? } else { json!({}) };
        let (status, device) = auth_json(self.http.post(format!("{ISSUER}/api/accounts/deviceauth/usercode")).json(&json!({"client_id":CLIENT}))).await?;
        if !status.is_success() { bail!("Codex device login returned HTTP {}; enable device login for this account", status.as_u16()); }
        let id = device["device_auth_id"].as_str().context("Device login has no ID")?;
        let code = device["user_code"].as_str().context("Device login has no code")?;
        let mut interval = device["interval"].as_u64().or_else(|| device["interval"].as_str().and_then(|v| v.parse().ok())).unwrap_or(5).clamp(1,900);
        println!("Open {ISSUER}/codex/device and enter: {code}\nDo not share this code. Waiting up to 15 minutes.");
        let record = tokio::time::timeout(std::time::Duration::from_secs(900), async {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
                let (status, result) = auth_json(self.http.post(format!("{ISSUER}/api/accounts/deviceauth/token")).json(&json!({"device_auth_id":id,"user_code":code}))).await?;
                if status.is_success() {
                    let (status, value) = auth_json(self.http.post(format!("{ISSUER}/oauth/token")).form(&[
                        ("grant_type","authorization_code"), ("client_id",CLIENT),
                        ("code",result["authorization_code"].as_str().context("Device approval has no authorization code")?),
                        ("code_verifier",result["code_verifier"].as_str().context("Device approval has no verifier")?),
                        ("redirect_uri","https://auth.openai.com/deviceauth/callback")])).await?;
                    if !status.is_success() { bail!("Codex token exchange returned HTTP {}",status.as_u16()); }
                    return credentials(&value, None);
                }
                if matches!(status.as_u16(), 403 | 404) || result["error"] == "deviceauth_authorization_pending" { continue; }
                if result["error"] == "slow_down" { interval = (interval + 5).min(900); continue; }
                bail!("Codex device approval returned HTTP {}",status.as_u16());
            }
        }).await.context("Codex device login timed out")??;
        root["openai-codex"] = record;
        crate::settings::atomic_write(&self.path, &serde_json::to_vec_pretty(&root)?).await?;
        println!("Codex credentials saved privately to {}", self.path.display());
        Ok(())
    }
}
async fn auth_json(request: reqwest::RequestBuilder) -> Result<(reqwest::StatusCode, Value)> {
    let mut response = request.timeout(std::time::Duration::from_secs(30)).send().await.context("OAuth request failed")?;
    let status = response.status(); let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if bytes.len() + chunk.len() > 64 * 1024 { bail!("OAuth response too large"); }
        bytes.extend_from_slice(&chunk);
    }
    let value = serde_json::from_slice(&bytes).map_err(|_| anyhow::anyhow!("Invalid OAuth response (values withheld)"))?;
    Ok((status, value))
}
fn credentials(value: &Value, previous_refresh: Option<&str>) -> Result<Value> {
    let access = value["access_token"].as_str().filter(|v| !v.is_empty()).context("OAuth returned no access token")?;
    let claims: Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(access.split('.').nth(1).context("Token has no account claims")?)
        .map_err(|_| anyhow::anyhow!("Invalid token claims"))?).map_err(|_| anyhow::anyhow!("Invalid account claims"))?;
    let account = claims.pointer("/https:~1~1api.openai.com~1auth/chatgpt_account_id").and_then(Value::as_str).filter(|v| !v.is_empty()).context("Token has no account ID")?;
    Ok(json!({"type":"oauth", "access":access, "refresh":value["refresh_token"].as_str().or(previous_refresh).filter(|v| !v.is_empty()).context("OAuth returned no refresh token")?,
        "expires":super::now_ms().saturating_add(value["expires_in"].as_u64().filter(|v| *v > 0).context("OAuth returned no lifetime")?.saturating_mul(1000)), "accountId":account}))
}
