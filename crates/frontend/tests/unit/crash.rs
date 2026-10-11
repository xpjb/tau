use super::*;
use axum::{Router, body::Bytes, http::{HeaderMap, StatusCode}, routing::post};
use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};

#[tokio::test]
async fn private_diagnostics_survive_bounded_restart_upload_retries() {
    let root = tempfile::tempdir().unwrap();
    let first_secret = "private chat and bearer token";
    record(root.path(), "rust.panic", first_secret, Some(("src/main.rs", 42)));
    let pending = root.path().join(PENDING_REPORT);
    let local = root.path().join(LOCAL_TRACE);
    let first = std::fs::read(&pending).unwrap();
    let report: CrashReport = serde_json::from_slice(&first).unwrap();
    assert_eq!(report.schema, 2);
    assert_eq!(report.exception_class, "rust.panic");
    assert_eq!(report.stack[0].file_name.as_deref(), Some("src/main.rs"));
    assert_eq!(report.stack[0].line_number, 42);
    assert!(first.len() < MAX_CRASH_BYTES);
    assert!(!String::from_utf8_lossy(&first).contains(first_secret));
    assert!(std::fs::read_to_string(&local).unwrap().contains(first_secret));

    let second_secret = "new private failure";
    record(root.path(), "app.tau.FatalError", second_secret, None);
    assert_eq!(std::fs::read(&pending).unwrap(), first, "The first unsent report must remain pending");
    assert!(std::fs::read_to_string(&local).unwrap().contains(second_secret), "The latest full local trace must remain available");
    let parsed = crash_frames("stack backtrace:\n   0: app::ui::paint\n             at C:\\src\\ui.rs:91:7\n", None);
    assert_eq!(parsed[0].class_name, "app::ui");
    assert_eq!(parsed[0].method_name, "paint");
    assert_eq!(parsed[0].file_name.as_deref(), Some("C:\\src\\ui.rs"));
    assert_eq!(parsed[0].line_number, 91);

    let attempts = Arc::new(AtomicUsize::new(0));
    let received = Arc::new(Mutex::new(Vec::<(Option<String>, Vec<u8>)>::new()));
    let route_attempts = attempts.clone();
    let route_received = received.clone();
    let app = Router::new().route("/v1/telemetry/crash", post(move |headers: HeaderMap, body: Bytes| {
        let attempts = route_attempts.clone();
        let received = route_received.clone();
        async move {
            received.lock().unwrap().push((headers.get("authorization").and_then(|value| value.to_str().ok()).map(str::to_owned), body.to_vec()));
            if attempts.fetch_add(1, Ordering::SeqCst) == 0 { StatusCode::SERVICE_UNAVAILABLE } else { StatusCode::NO_CONTENT }
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
    let settings = Settings { server_url:format!("http://{address}"), token:"test-token".into() };

    assert!(upload_pending(root.path(), &settings).await.is_err());
    assert_eq!(std::fs::read(&pending).unwrap(), first, "A failed upload must remain pending");
    upload_pending(root.path(), &settings).await.unwrap();
    assert!(!pending.exists());
    assert!(local.exists(), "Uploading telemetry must not erase the private local trace");
    let received = received.lock().unwrap();
    assert_eq!(received.len(), 2);
    for (authorization, body) in received.iter() {
        assert_eq!(authorization.as_deref(), Some("Bearer test-token"));
        let uploaded: CrashReport = serde_json::from_slice(body).unwrap();
        assert_eq!(uploaded.report_id, report.report_id);
        assert!(!String::from_utf8_lossy(body).contains(first_secret));
    }
    drop(received);
    server.abort();

    let hook_root = tempfile::tempdir().unwrap();
    install(hook_root.path().to_owned());
    let hook_secret = "uncaught private diagnostics probe";
    assert!(std::panic::catch_unwind(|| panic!("{hook_secret}")).is_err());
    assert!(std::fs::read_to_string(hook_root.path().join(LOCAL_TRACE)).unwrap().contains(hook_secret));
    assert!(!std::fs::read_to_string(hook_root.path().join(PENDING_REPORT)).unwrap().contains(hook_secret));
}
