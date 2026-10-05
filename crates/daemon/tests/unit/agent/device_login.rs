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
fn pending_id(state: &tau_net::CodexLogin) -> String {
    let tau_net::CodexLogin::Pending { login_id, verification_uri, user_code, .. } = state else { panic!("not pending") };
    assert_eq!(verification_uri, tau_net::CODEX_LOGIN_URL); assert_eq!(user_code, "ABCD-EFGH"); login_id.clone()
}
async fn finished(auth: &AuthStore, id: &str) -> tau_net::CodexLogin {
    tokio::time::timeout(std::time::Duration::from_secs(4), async {
        loop { let state=auth.login_status(id,false).await.unwrap(); if !matches!(state,tau_net::CodexLogin::Pending {..}) { return state; }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await; }
    }).await.unwrap()
}
#[tokio::test]
async fn device_signin_coalesces_saves_only_beta_and_preserves_concurrent_other_credentials() {
    let (url, server, starts) = fixture(false, 30).await;
    let dir=tempfile::tempdir().unwrap();let own=dir.path().join("beta.json");let primary=dir.path().join("primary.json");
    let shared=json!({}).to_string();
    crate::settings::atomic_write(&primary,shared.as_bytes()).await.unwrap();
    let auth=AuthStore::new(own.clone(),reqwest::Client::new()).shared_codex(Some(primary.clone())).with_issuer(url);
    assert!(auth.authorization("openai-codex",None,None).await.unwrap_err().is::<SignInRequired>());
    let (first, second)=tokio::join!(auth.start_login(),auth.start_login());let first=first.unwrap();assert_eq!(first,second.unwrap());
    let id=pending_id(&first);assert_eq!(starts.load(Ordering::SeqCst),1);
    let wire=serde_json::to_string(&first).unwrap();for secret in ["private-device-id","private-new-refresh","private-verifier"] {assert!(!wire.contains(secret));}
    // A different provider changed while the browser was open; do not restore an old snapshot.
    crate::settings::atomic_write(&own,json!({"openrouter":{"type":"api_key","key":"concurrent-provider-key"}}).to_string().as_bytes()).await.unwrap();
    assert_eq!(tokio::time::timeout(std::time::Duration::from_millis(300),auth.authorization("openrouter",None,None)).await.unwrap().unwrap().0,"concurrent-provider-key");
    assert_eq!(finished(&auth,&id).await,tau_net::CodexLogin::Complete);
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
    assert_eq!(auth.login_status(&id,true).await.unwrap(),tau_net::CodexLogin::Cancelled);
    let next=pending_id(&auth.start_login().await.unwrap());assert_ne!(id,next);assert!(auth.login_status(&id,true).await.is_err());
    let state=finished(&auth,&next).await;assert!(matches!(state,tau_net::CodexLogin::Failed {message} if message.contains("expired")));
    assert_eq!(starts.load(Ordering::SeqCst),2);assert!(!own.exists());server.abort();
}
