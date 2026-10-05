use std::path::PathBuf;
use std::sync::Arc;
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::sync::Mutex;
use tokio::io::AsyncReadExt;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};

const ISSUER: &str = "https://auth.openai.com";
#[derive(Debug)]
pub struct SignInRequired;
impl std::fmt::Display for SignInRequired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(tau_net::CODEX_SIGN_IN_REQUIRED) }
}
impl std::error::Error for SignInRequired {}

struct LoginAttempt {
    id: String,
    state: tau_net::CodexLogin,
    cancel: tokio_util::sync::CancellationToken,
}

const CLIENT: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
#[derive(Clone)]
pub struct AuthStore { path: PathBuf, shared_codex:Option<PathBuf>, gate: Arc<Mutex<()>>, http: reqwest::Client, issuer: Arc<str>, login: Arc<Mutex<Option<LoginAttempt>>> }
impl AuthStore {
    pub fn new(path: PathBuf, http: reqwest::Client) -> Self { Self { path, shared_codex:None, gate:Arc::new(Mutex::new(())), http, issuer:ISSUER.into(), login:Arc::new(Mutex::new(None)) } }
    pub fn shared_codex(mut self, path: Option<PathBuf>) -> Self { self.shared_codex=path; self }
    #[cfg(test)]
    pub(crate) fn with_issuer(mut self, issuer: String) -> Self { self.issuer = issuer.into(); self }
    async fn read(&self) -> Result<Value> { Self::read_path(&self.path).await }
    async fn read_path(path: &std::path::Path) -> Result<Value> {
        let mut options = tokio::fs::OpenOptions::new(); options.read(true);
        #[cfg(unix)] options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        let file = options.open(path).await.context("Credential file is unavailable")?;
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
            let own_lock = super::auth_lock::AuthLock::acquire(&this.path).await?;
            let mut root = if this.path.try_exists()? { this.read().await? } else { json!({}) };
            let mut path = &this.path;
            let mut shared_lock = None;
            if provider == "openai-codex" && root.get(&provider).is_none() && let Some(source)=&this.shared_codex {
                // Participate in the primary's existing lock, then reread. A
                // concurrent stable/beta refresh is reused, never duplicated.
                shared_lock = Some(super::auth_lock::AuthLock::acquire(source).await?);
                root = match Self::read_path(source).await {
                    Err(error) if error.downcast_ref::<std::io::Error>().is_some_and(|e| e.kind()==std::io::ErrorKind::NotFound) => return Err(SignInRequired.into()),
                    other => other?,
                };
                path = source;
            }
            if provider == "openai-codex" && root.get(&provider).is_none() { return Err(SignInRequired.into()); }
            let record = root.get_mut(&provider).context("Provider has no daemon credentials; configure an API key")?;
            if record["type"] == "api_key" {
                return Ok((record["key"].as_str().filter(|v| !v.is_empty()).context("API key is empty")?.to_owned(), None));
            }
            if record["type"] != "oauth" || provider != "openai-codex" { bail!("Unsupported credential type"); }
            let lease = shared_lock.as_ref().unwrap_or(&own_lock);
            if record["expires"].as_u64().unwrap_or(0) <= super::now_ms() + 60_000 || rejected.as_deref() == record["access"].as_str() {
                lease.check()?;
                let refresh = record["refresh"].as_str().filter(|s| !s.is_empty()).ok_or(SignInRequired)?;
                let (status, value) = auth_json(this.http.post(format!("{}/oauth/token", this.issuer))
                    .form(&[("grant_type", "refresh_token"), ("refresh_token", refresh), ("client_id", CLIENT)])).await?;
                if matches!(status.as_u16(), 400 | 401 | 403) { return Err(SignInRequired.into()); }
                if !status.is_success() { bail!("Codex token refresh returned HTTP {}; try again shortly", status.as_u16()); }
                *record = credentials(&value, Some(refresh))?;
                lease.check()?;
                crate::settings::atomic_write(path, &serde_json::to_vec_pretty(&root)?).await?;
            }
            let record = &root[&provider];
            Ok((record["access"].as_str().filter(|v| !v.is_empty()).context("Missing access token")?.to_owned(),
                Some(record["accountId"].as_str().filter(|v| !v.is_empty()).context("Missing account ID")?.to_owned())))
        }).await.context("Credential refresh task stopped")?
    }
    /// One bounded device flow per daemon. Never hold the credential lock while
    /// waiting for a human; refreshes and other providers remain available.
    pub async fn start_login(&self) -> Result<tau_net::CodexLogin> {
        use tau_net::CodexLogin;
        let mut slot = self.login.lock().await;
        if let Some(attempt) = slot.as_ref() && matches!(attempt.state, CodexLogin::Pending { .. }) {
            return Ok(attempt.state.clone());
        }
        let (status, device) = auth_json(self.http.post(format!("{}/api/accounts/deviceauth/usercode", self.issuer))
            .json(&json!({"client_id":CLIENT}))).await?;
        if !status.is_success() {
            bail!("Codex sign-in could not start (HTTP {}). Enable device code authorization in ChatGPT Settings → Security, then try again.", status.as_u16());
        }
        let device_id = device["device_auth_id"].as_str().filter(|s| !s.is_empty()).context("Device login has no ID")?.to_owned();
        let code = device["user_code"].as_str().filter(|s| !s.is_empty() && s.len() <= 128).context("Device login has no code")?.to_owned();
        let interval = device["interval"].as_u64().or_else(|| device["interval"].as_str().and_then(|v| v.parse().ok())).unwrap_or(5).clamp(1,900);
        let lifetime = device["expires_in"].as_u64().or_else(|| device["expires_in"].as_str().and_then(|v| v.parse().ok())).unwrap_or(900).clamp(1,900);
        let id = uuid::Uuid::new_v4().to_string();
        let state = CodexLogin::Pending { login_id:id.clone(), user_code:code.clone(), verification_uri:tau_net::CODEX_LOGIN_URL.into(), expires_at_ms:super::now_ms().saturating_add(lifetime*1000) };
        let cancel = tokio_util::sync::CancellationToken::new();
        *slot = Some(LoginAttempt { id:id.clone(), state:state.clone(), cancel:cancel.clone() });
        let this = self.clone();
        tokio::spawn(async move {
            let result = tokio::select! {
                biased;
                _ = cancel.cancelled() => return,
                result = tokio::time::timeout(std::time::Duration::from_secs(lifetime), this.approve_device(&device_id, &code, interval)) =>
                    result.context("Codex sign-in expired. Try again for a new code.").and_then(|r| r),
            };
            // Cancellation and credential publication have one ordering. An old
            // or cancelled flow can never overwrite a later authorization.
            let mut slot = this.login.lock().await;
            let Some(attempt) = slot.as_mut().filter(|a| a.id == id && !a.cancel.is_cancelled()) else { return; };
            let result = match result {
                Ok(record) => this.save_login(record).await,
                Err(error) => Err(error),
            };
            attempt.state = match result { Ok(()) => CodexLogin::Complete, Err(error) => CodexLogin::Failed { message:error.to_string() } };
        });
        Ok(state)
    }
    pub async fn login_status(&self, id: &str, cancel: bool) -> Result<tau_net::CodexLogin> {
        let mut slot = self.login.lock().await;
        let attempt = slot.as_mut().filter(|a| a.id == id).context("This sign-in is no longer available. Start again for a new code.")?;
        if cancel && matches!(attempt.state, tau_net::CodexLogin::Pending { .. }) {
            attempt.cancel.cancel();
            attempt.state = tau_net::CodexLogin::Cancelled;
        }
        Ok(attempt.state.clone())
    }
    async fn save_login(&self, record: Value) -> Result<()> {
        let _guard = self.gate.lock().await;
        let lease = super::auth_lock::AuthLock::acquire(&self.path).await?;
        // Reread at commit time to preserve credentials changed during approval.
        let mut root = if self.path.try_exists()? { self.read().await? } else { json!({}) };
        root["openai-codex"] = record;
        lease.check()?;
        crate::settings::atomic_write(&self.path, &serde_json::to_vec_pretty(&root)?).await
    }
    async fn approve_device(&self, id: &str, code: &str, mut interval: u64) -> Result<Value> {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
            let (status, result) = auth_json(self.http.post(format!("{}/api/accounts/deviceauth/token", self.issuer))
                .json(&json!({"device_auth_id":id,"user_code":code}))).await?;
            if status.is_success() {
                let (status, value) = auth_json(self.http.post(format!("{}/oauth/token", self.issuer)).form(&[
                    ("grant_type","authorization_code"), ("client_id",CLIENT),
                    ("code",result["authorization_code"].as_str().context("Device approval has no authorization code")?),
                    ("code_verifier",result["code_verifier"].as_str().context("Device approval has no verifier")?),
                    ("redirect_uri","https://auth.openai.com/deviceauth/callback")])).await?;
                if !status.is_success() { bail!("Codex token exchange returned HTTP {}; try signing in again",status.as_u16()); }
                return credentials(&value, None);
            }
            if matches!(result["error"].as_str(), Some("access_denied" | "expired_token" | "deviceauth_expired")) {
                bail!("Codex sign-in was declined or expired. Try again for a new code.");
            }
            if result["error"] == "slow_down" { interval = (interval + 5).min(900); continue; }
            if matches!(status.as_u16(), 403 | 404) || result["error"] == "deviceauth_authorization_pending" { continue; }
            bail!("Codex device approval returned HTTP {}; try signing in again",status.as_u16());
        }
    }
    pub async fn login(&self) -> Result<()> {
        let tau_net::CodexLogin::Pending { login_id, user_code, verification_uri, .. } = self.start_login().await? else { unreachable!() };
        println!("Open {verification_uri} and enter: {user_code}\nDo not share this code. Waiting up to 15 minutes.");
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            match self.login_status(&login_id, false).await? {
                tau_net::CodexLogin::Pending { .. } => {},
                tau_net::CodexLogin::Complete => { println!("Codex credentials saved privately to {}", self.path.display()); return Ok(()); },
                tau_net::CodexLogin::Failed { message } => bail!(message),
                tau_net::CodexLogin::Cancelled => bail!("Sign-in cancelled"),
            }
        }
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

#[cfg(test)]
#[path = "../../tests/unit/agent/auth.rs"]
mod tests;

#[cfg(test)]
#[path = "../../tests/unit/agent/device_login.rs"]
pub(crate) mod device_login_tests;

#[cfg(test)]
#[path = "../../tests/unit/agent/shared_refresh.rs"]
mod shared_refresh_tests;
