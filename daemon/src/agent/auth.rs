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
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(tau_protocol::CODEX_SIGN_IN_REQUIRED) }
}
impl std::error::Error for SignInRequired {}

struct LoginAttempt {
    id: String,
    state: tau_protocol::CodexLogin,
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
            let mut root = if this.path.try_exists()? { this.read().await? } else { json!({}) };
            if provider == "openai-codex" && root.get(&provider).is_none() && let Some(source)=&this.shared_codex {
                // Never rotate or copy the primary daemon's refresh token. Native
                // beta login creates its own record and takes precedence here.
                let shared=match Self::read_path(source).await {
                    Err(error) if error.downcast_ref::<std::io::Error>().is_some_and(|e| e.kind()==std::io::ErrorKind::NotFound) => return Err(SignInRequired.into()),
                    other => other?,
                };
                let record=&shared[&provider];
                if record["type"] != "oauth" { return Err(SignInRequired.into()); }
                let access=record["access"].as_str().filter(|v|!v.is_empty()).context("Shared Codex source has no access token")?;
                if record["expires"].as_u64().unwrap_or(0)<=super::now_ms().saturating_add(30_000) || rejected.as_deref() == Some(access) {
                    return Err(SignInRequired.into());
                }
                return Ok((access.to_owned(),Some(record["accountId"].as_str().filter(|v|!v.is_empty()).context("Shared Codex source has no account ID")?.into())));
            }
            if provider == "openai-codex" && root.get(&provider).is_none() { return Err(SignInRequired.into()); }
            let record = root.get_mut(&provider).context("Provider has no daemon credentials; configure an API key")?;
            if record["type"] == "api_key" {
                return Ok((record["key"].as_str().filter(|v| !v.is_empty()).context("API key is empty")?.to_owned(), None));
            }
            if record["type"] != "oauth" || provider != "openai-codex" { bail!("Unsupported credential type"); }
            if record["expires"].as_u64().unwrap_or(0) <= super::now_ms() + 60_000 || rejected.as_deref() == record["access"].as_str() {
                let refresh = record["refresh"].as_str().filter(|s| !s.is_empty()).ok_or(SignInRequired)?;
                let (status, value) = auth_json(this.http.post(format!("{}/oauth/token", this.issuer))
                    .form(&[("grant_type", "refresh_token"), ("refresh_token", refresh), ("client_id", CLIENT)])).await?;
                if matches!(status.as_u16(), 400 | 401 | 403) { return Err(SignInRequired.into()); }
                if !status.is_success() { bail!("Codex token refresh returned HTTP {}; try again shortly", status.as_u16()); }
                *record = credentials(&value, Some(refresh))?;
                crate::settings::atomic_write(&this.path, &serde_json::to_vec_pretty(&root)?).await?;
            }
            let record = &root[&provider];
            Ok((record["access"].as_str().filter(|v| !v.is_empty()).context("Missing access token")?.to_owned(),
                Some(record["accountId"].as_str().filter(|v| !v.is_empty()).context("Missing account ID")?.to_owned())))
        }).await.context("Credential refresh task stopped")?
    }
    /// One bounded device flow per daemon. Never hold the credential lock while
    /// waiting for a human; refreshes and other providers remain available.
    pub async fn start_login(&self) -> Result<tau_protocol::CodexLogin> {
        use tau_protocol::CodexLogin;
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
        let state = CodexLogin::Pending { login_id:id.clone(), user_code:code.clone(), verification_uri:tau_protocol::CODEX_LOGIN_URL.into(), expires_at_ms:super::now_ms().saturating_add(lifetime*1000) };
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
    pub async fn login_status(&self, id: &str, cancel: bool) -> Result<tau_protocol::CodexLogin> {
        let mut slot = self.login.lock().await;
        let attempt = slot.as_mut().filter(|a| a.id == id).context("This sign-in is no longer available. Start again for a new code.")?;
        if cancel && matches!(attempt.state, tau_protocol::CodexLogin::Pending { .. }) {
            attempt.cancel.cancel();
            attempt.state = tau_protocol::CodexLogin::Cancelled;
        }
        Ok(attempt.state.clone())
    }
    async fn save_login(&self, record: Value) -> Result<()> {
        let _guard = self.gate.lock().await;
        // Reread at commit time to preserve credentials changed during approval.
        let mut root = if self.path.try_exists()? { self.read().await? } else { json!({}) };
        root["openai-codex"] = record;
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
        let tau_protocol::CodexLogin::Pending { login_id, user_code, verification_uri, .. } = self.start_login().await? else { unreachable!() };
        println!("Open {verification_uri} and enter: {user_code}\nDo not share this code. Waiting up to 15 minutes.");
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            match self.login_status(&login_id, false).await? {
                tau_protocol::CodexLogin::Pending { .. } => {},
                tau_protocol::CodexLogin::Complete => { println!("Codex credentials saved privately to {}", self.path.display()); return Ok(()); },
                tau_protocol::CodexLogin::Failed { message } => bail!(message),
                tau_protocol::CodexLogin::Cancelled => bail!("Sign-in cancelled"),
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
mod tests {
    use super::*;
    #[tokio::test]
    async fn side_by_side_auth_reads_rotations_but_never_rotates_or_rewrites_the_primary() {
        let root=tempfile::tempdir().unwrap(); let primary=root.path().join("primary.json"); let own=root.path().join("beta.json");
        let record=|access:&str,expiry:u64|json!({"openai-codex":{"type":"oauth","access":access,"refresh":"never-submit-this-refresh","accountId":"fixture-account","expires":expiry}}).to_string();
        crate::settings::atomic_write(&primary,record("first",u64::MAX).as_bytes()).await.unwrap();
        let auth=AuthStore::new(own.clone(),reqwest::Client::new()).shared_codex(Some(primary.clone()));
        assert_eq!(auth.authorization("openai-codex",None,None).await.unwrap().0,"first");
        crate::settings::atomic_write(&primary,record("rotated",u64::MAX).as_bytes()).await.unwrap();
        assert_eq!(auth.authorization("openai-codex",None,Some("first")).await.unwrap().0,"rotated");
        assert!(auth.authorization("openai-codex",None,Some("rotated")).await.is_err());
        crate::settings::atomic_write(&primary,record("expired",0).as_bytes()).await.unwrap();
        assert!(auth.authorization("openai-codex",None,None).await.is_err());
        assert_eq!(tokio::fs::read_to_string(&primary).await.unwrap(),record("expired",0));
        assert!(!own.exists(),"Reading a shared account must not copy its rotating refresh credential");
        crate::settings::atomic_write(&own,record("independent-beta",u64::MAX).as_bytes()).await.unwrap();
        assert_eq!(auth.authorization("openai-codex",None,None).await.unwrap().0,"independent-beta");
    }
}

#[cfg(test)]
pub(crate) mod device_login_tests {
    use super::*;
    use axum::{Json, Router, routing::post};
    use std::sync::atomic::{AtomicUsize, Ordering};

    pub(crate) async fn fixture(pending: bool, lifetime: u64) -> (String, tokio::task::JoinHandle<()>, Arc<AtomicUsize>) {
        let starts = Arc::new(AtomicUsize::new(0)); let count = starts.clone();
        let token = format!("header.{}.signature", URL_SAFE_NO_PAD.encode(json!({"https://api.openai.com/auth":{"chatgpt_account_id":"fixture-account"}}).to_string()));
        let app = Router::new()
            .route("/api/accounts/deviceauth/usercode", post(move || { let count=count.clone(); async move {
                count.fetch_add(1, Ordering::SeqCst);
                Json(json!({"device_auth_id":"private-device-id","user_code":"ABCD-EFGH","interval":"1","expires_in":lifetime}))
            }}))
            .route("/api/accounts/deviceauth/token", post(move |Json(value): Json<Value>| async move {
                assert_eq!(value["device_auth_id"], "private-device-id"); assert_eq!(value["user_code"], "ABCD-EFGH");
                if pending { (axum::http::StatusCode::FORBIDDEN, Json(json!({"error":"deviceauth_authorization_pending"}))) }
                else { (axum::http::StatusCode::OK, Json(json!({"authorization_code":"private-authorization-code", "code_verifier":"private-verifier"}))) }
            }))
            .route("/oauth/token", post(move |axum::extract::Form(form): axum::extract::Form<std::collections::HashMap<String,String>>| {let token=token.clone(); async move {
                assert_eq!(form["grant_type"], "authorization_code"); assert_eq!(form["code_verifier"], "private-verifier");
                assert_eq!(form["redirect_uri"], "https://auth.openai.com/deviceauth/callback");
                Json(json!({"access_token":token,"refresh_token":"private-new-refresh","expires_in":3600}))
            }}));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        (url, tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); }), starts)
    }
    fn pending_id(state: &tau_protocol::CodexLogin) -> String {
        let tau_protocol::CodexLogin::Pending { login_id, verification_uri, user_code, .. } = state else { panic!("not pending") };
        assert_eq!(verification_uri, tau_protocol::CODEX_LOGIN_URL); assert_eq!(user_code, "ABCD-EFGH"); login_id.clone()
    }
    async fn finished(auth: &AuthStore, id: &str) -> tau_protocol::CodexLogin {
        tokio::time::timeout(std::time::Duration::from_secs(4), async {
            loop { let state=auth.login_status(id,false).await.unwrap(); if !matches!(state,tau_protocol::CodexLogin::Pending {..}) { return state; }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await; }
        }).await.unwrap()
    }
    #[tokio::test]
    async fn device_signin_coalesces_saves_only_beta_and_preserves_concurrent_other_credentials() {
        let (url, server, starts) = fixture(false, 30).await;
        let dir=tempfile::tempdir().unwrap();let own=dir.path().join("beta.json");let primary=dir.path().join("primary.json");
        let shared=json!({"openai-codex":{"type":"oauth","access":"old-shared","refresh":"never-rotate-primary","accountId":"fixture-account","expires":0}}).to_string();
        crate::settings::atomic_write(&primary,shared.as_bytes()).await.unwrap();
        let auth=AuthStore::new(own.clone(),reqwest::Client::new()).shared_codex(Some(primary.clone())).with_issuer(url);
        assert!(auth.authorization("openai-codex",None,None).await.unwrap_err().is::<SignInRequired>());
        let (first, second)=tokio::join!(auth.start_login(),auth.start_login());let first=first.unwrap();assert_eq!(first,second.unwrap());
        let id=pending_id(&first);assert_eq!(starts.load(Ordering::SeqCst),1);
        let wire=serde_json::to_string(&first).unwrap();for secret in ["private-device-id","private-new-refresh","private-verifier"] {assert!(!wire.contains(secret));}
        // A different provider changed while the browser was open; do not restore an old snapshot.
        crate::settings::atomic_write(&own,json!({"openrouter":{"type":"api_key","key":"concurrent-provider-key"}}).to_string().as_bytes()).await.unwrap();
        assert_eq!(tokio::time::timeout(std::time::Duration::from_millis(300),auth.authorization("openrouter",None,None)).await.unwrap().unwrap().0,"concurrent-provider-key");
        assert_eq!(finished(&auth,&id).await,tau_protocol::CodexLogin::Complete);
        assert_eq!(tokio::fs::read_to_string(&primary).await.unwrap(),shared);
        let saved=auth.read().await.unwrap();assert_eq!(saved["openrouter"]["key"],"concurrent-provider-key");assert_eq!(saved["openai-codex"]["refresh"],"private-new-refresh");
        assert_ne!(auth.authorization("openai-codex",None,None).await.unwrap().0,"old-shared");
        #[cfg(unix)] {use std::os::unix::fs::PermissionsExt; assert_eq!(std::fs::metadata(own).unwrap().permissions().mode() & 0o777,0o600);}
        server.abort();
    }
    #[tokio::test]
    async fn device_signin_cancellation_expiry_and_stale_ids_never_publish_credentials() {
        let (url,server,starts)=fixture(true,1).await;let dir=tempfile::tempdir().unwrap();let own=dir.path().join("auth.json");
        let auth=AuthStore::new(own.clone(),reqwest::Client::new()).with_issuer(url);
        let id=pending_id(&auth.start_login().await.unwrap());
        assert!(auth.login_status("other-flow",true).await.is_err());
        assert_eq!(auth.login_status(&id,true).await.unwrap(),tau_protocol::CodexLogin::Cancelled);
        let next=pending_id(&auth.start_login().await.unwrap());assert_ne!(id,next);assert!(auth.login_status(&id,true).await.is_err());
        let state=finished(&auth,&next).await;assert!(matches!(state,tau_protocol::CodexLogin::Failed {message} if message.contains("expired")));
        assert_eq!(starts.load(Ordering::SeqCst),2);assert!(!own.exists());server.abort();
    }
}
