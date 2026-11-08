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
