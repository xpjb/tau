use super::*;
use axum::{Router, Json, routing::post};
use std::sync::atomic::{AtomicUsize, Ordering};
#[tokio::test]
async fn shared_codex_refresh_rotates_once_under_the_primary_lock_without_a_beta_copy() {
    let calls=Arc::new(AtomicUsize::new(0));let count=calls.clone();
    let access=format!("header.{}.signature",URL_SAFE_NO_PAD.encode(json!({"https://api.openai.com/auth":{"chatgpt_account_id":"fixture-account"}}).to_string()));
    let expected=access.clone();
    let app=Router::new().route("/oauth/token",post(move |axum::extract::Form(form):axum::extract::Form<std::collections::HashMap<String,String>>| {let count=count.clone();let access=access.clone();async move {
        assert_eq!(form["grant_type"],"refresh_token");assert_eq!(form["refresh_token"],"rotate-only-once");
        assert_eq!(count.fetch_add(1,Ordering::SeqCst),0);
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        Json(json!({"access_token":access,"refresh_token":"new-refresh","expires_in":3600}))
    }}));
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let issuer=format!("http://{}",listener.local_addr().unwrap());
    let server=tokio::spawn(async move {axum::serve(listener,app).await.unwrap()});
    let dir=tempfile::tempdir().unwrap();let primary=dir.path().join("primary.json");
    let original=json!({"openai-codex":{"type":"oauth","access":"expired-access","refresh":"rotate-only-once","accountId":"fixture-account","expires":0},"other-provider":{"type":"api_key","key":"keep-other-provider"}});
    crate::settings::atomic_write(&primary,original.to_string().as_bytes()).await.unwrap();
    let a=AuthStore::new(dir.path().join("beta-a.json"),reqwest::Client::new()).shared_codex(Some(primary.clone())).with_issuer(issuer.clone());
    let b=AuthStore::new(dir.path().join("beta-b.json"),reqwest::Client::new()).shared_codex(Some(primary.clone())).with_issuer(issuer);
    let (a_result,b_result)=tokio::join!(a.authorization("openai-codex",None,None),b.authorization("openai-codex",None,Some("expired-access")));
    assert_eq!(a_result.unwrap().0,expected);assert_eq!(b_result.unwrap().0,expected);assert_eq!(calls.load(Ordering::SeqCst),1);
    let saved=AuthStore::read_path(&primary).await.unwrap();assert_eq!(saved["other-provider"],original["other-provider"]);assert_eq!(saved["openai-codex"]["refresh"],"new-refresh");
    assert!(!a.path.exists() && !b.path.exists());assert!(!dir.path().join("primary.json.lock").exists());
    server.abort();
}
#[tokio::test]
async fn shared_refresh_invalid_grant_requests_signin_without_destroying_the_login() {
    let app=Router::new().route("/oauth/token",post(||async{(axum::http::StatusCode::BAD_REQUEST,Json(json!({"error":"invalid_grant","details":"do-not-leak-provider-body"})))}));
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let issuer=format!("http://{}",listener.local_addr().unwrap());
    let server=tokio::spawn(async move{axum::serve(listener,app).await.unwrap()});
    let dir=tempfile::tempdir().unwrap();let primary=dir.path().join("primary.json");
    let original=json!({"openai-codex":{"type":"oauth","access":"expired","refresh":"revoked","accountId":"fixture-account","expires":0}}).to_string();
    crate::settings::atomic_write(&primary,original.as_bytes()).await.unwrap();
    let auth=AuthStore::new(dir.path().join("beta.json"),reqwest::Client::new()).shared_codex(Some(primary.clone())).with_issuer(issuer);
    let error=auth.authorization("openai-codex",None,None).await.unwrap_err();assert!(error.is::<SignInRequired>());assert!(!error.to_string().contains("do-not-leak"));
    assert_eq!(tokio::fs::read_to_string(primary).await.unwrap(),original);assert!(!auth.path.exists());server.abort();
}
