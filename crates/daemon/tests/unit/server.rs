use axum::http::{HeaderMap, HeaderValue, header::AUTHORIZATION};

use super::authorized;
use crate::manager::safe_file_name;

#[tokio::test]
async fn authenticated_quota_control_is_read_only_and_returns_an_explicit_unavailable_state() {
    use super::*;
    use crate::state::StateStore;
    use tokio_tungstenite::{connect_async, tungstenite::{Message as ClientMessage, client::IntoClientRequest}};
    let root=tempfile::tempdir().unwrap();
    let config=Config {bind:"127.0.0.1:0".parse().unwrap(),transfer_bind:"127.0.0.1:0".parse().unwrap(),transfer_bind_v6:None,
        token:Arc::from("fixture-token"),settings_path:root.path().join("settings.json"),import_pi_dir:None,codex_auth_source:None,
        cwd:root.path().into(),database_path:root.path().join("tau.sqlite3"),telemetry_path:root.path().join("crashes.jsonl"),
        attachment_root:root.path().join("outbox"),upload_root:root.path().join("uploads")};
    let manager=AgentManager::new(config.clone(),StateStore::load(config.database_path.clone()).await.unwrap()).await.unwrap();
    let transfers=Arc::new(tau_net::native::Server::bind("127.0.0.1:0".parse().unwrap(),Arc::new(manager.clone())).await.unwrap());
    let state=AppState {config,manager,telemetry_gate:Arc::new(Mutex::new(())),transfers,requests:Arc::new(tokio::sync::Semaphore::new(32)),admissions:Arc::new(tokio::sync::Semaphore::new(128))};
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url=format!("ws://{}/v1/ws",listener.local_addr().unwrap());
    let server=tokio::spawn(async move {axum::serve(listener,Router::new().route("/v1/ws",get(websocket)).with_state(state)).await.unwrap();});
    assert!(connect_async(&url).await.is_err(),"Unauthenticated clients cannot see account quota");
    let mut request=url.into_client_request().unwrap();request.headers_mut().insert("Authorization","Bearer fixture-token".parse().unwrap());
    let (mut client,_)=connect_async(request).await.unwrap();
    client.send(ClientMessage::Text(serde_json::json!({"id":"quota-1","type":"get_codex_usage","force":true}).to_string().into())).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5),async {
        loop {
            let message=client.next().await.unwrap().unwrap();
            let value:serde_json::Value=serde_json::from_str(message.to_text().unwrap()).unwrap();
            if value["requestId"]!="quota-1" {continue;}
            assert_eq!(value["type"],"codex_usage");assert!(value["report"].is_null());
            assert!(value["error"].as_str().unwrap().contains("sign in"));
            assert!(!value.to_string().contains("fixture-token"));break;
        }
    }).await.unwrap();
    server.abort();
}

#[tokio::test]
async fn persists_bounded_crash_reports_with_safe_diagnostics() {
    use std::sync::Arc;
    use std::time::Duration;
    use axum::extract::DefaultBodyLimit;
    use axum::routing::post;
    use axum::Router;
    use serde_json::json;
    use tokio::fs;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::sync::Mutex;
    use crate::config::Config;
    use crate::manager::AgentManager;
    use tau_net::MAX_CRASH_BYTES;
    use crate::state::StateStore;
    use super::{AppState, crash_report};

    let root = std::env::temp_dir().join(format!("tau-crashes-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let log = root.join("crashes.jsonl");
    let config = Config {
        transfer_bind: "127.0.0.1:0".parse().unwrap(), transfer_bind_v6:None,
        bind: address, token: Arc::from("test-token"), settings_path: root.join("settings.json"), import_pi_dir: None, codex_auth_source:None, cwd: root.clone(), database_path: root.join("tau.sqlite3"), telemetry_path: log.clone(), attachment_root: root.join("outbox"),
        upload_root: root.join("uploads"),
    };
    let manager = AgentManager::new(config.clone(), StateStore::load(config.database_path.clone()).await.unwrap()).await.unwrap();
    let app = Router::new().route("/v1/telemetry/crash", post(crash_report).layer(DefaultBodyLimit::max(MAX_CRASH_BYTES)))
        .with_state(AppState { config, manager:manager.clone(), telemetry_gate: Arc::new(Mutex::new(())),
        transfers: Arc::new(tau_net::native::Server::bind("127.0.0.1:0".parse().unwrap(),Arc::new(manager.clone())).await.unwrap()), requests:Arc::new(tokio::sync::Semaphore::new(32)),admissions:Arc::new(tokio::sync::Semaphore::new(128)) });
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
    let frame = json!({"className":"example.Frame", "methodName":"draw", "fileName":"File.kt", "lineNumber":5});
    let legacy = json!({"schema":1, "reportId":"legacy", "platform":"windows", "appVersion":"0.5.12",
        "osVersion":"Windows 11", "thread":"AWT-EventQueue-0", "exceptionClass":"java.lang.IllegalStateException", "stack":[frame]});
    let cause = json!({"exceptionClass":"java.lang.IllegalArgumentException", "stack":[frame],
        "selectionRange":{"start":5, "end":0, "textLength":710}});
    let mut modern = legacy.clone();
    modern["schema"] = json!(2);
    modern["reportId"] = json!("modern");
    modern["causes"] = json!([cause]);
    modern["ignoredMessage"] = json!("private exception message must not be retained");
    let mut cases = vec![(legacy.clone(), "wrong-token", 401), (legacy, "test-token", 204), (modern.clone(), "test-token", 204)];
    for (field, value) in [("schema", json!(3)), ("schema", json!(1)), ("causes", json!(vec![cause.clone(); 4])),
        ("stack", json!(vec![frame.clone(); 65]))] {
        let mut invalid = modern.clone();
        invalid[field] = value;
        cases.push((invalid, "test-token", 400));
    }
    let mut unicode = modern.clone();
    unicode["stack"][0]["fileName"] = json!("界".repeat(192));
    cases.push((unicode.clone(), "test-token", 204));
    unicode["stack"][0]["fileName"] = json!("界".repeat(193));
    cases.push((unicode, "test-token", 400));
    let mut too_many_frames = modern.clone();
    too_many_frames["causes"][0]["stack"] = json!(vec![frame; 13]);
    cases.push((too_many_frames, "test-token", 400));
    for range in [json!({"start":0,"end":10,"textLength":710}), json!({"start":5,"end":0,"textLength":-1})] {
        let mut invalid = modern.clone();
        invalid["causes"][0]["selectionRange"] = range;
        cases.push((invalid, "test-token", 400));
    }
    cases.push((json!("x".repeat(MAX_CRASH_BYTES)), "test-token", 413));
    let mut accepted = Vec::new();
    for (mut body, token, expected) in cases {
        let payload = body.to_string();
        let status = tokio::time::timeout(Duration::from_secs(5), async {
            let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
            stream.write_all(format!("POST /v1/telemetry/crash HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}", payload.len()).as_bytes()).await.unwrap();
            let mut reply = String::new();
            stream.read_to_string(&mut reply).await.unwrap();
            reply.split_whitespace().nth(1).unwrap().parse::<u16>().unwrap()
        }).await.unwrap();
        assert_eq!(status, expected);
        if status == 204 {
            body.as_object_mut().unwrap().remove("ignoredMessage");
            accepted.push(body);
        }
        let saved = if log.exists() { fs::read_to_string(&log).await.unwrap() } else { String::new() };
        let reports: Vec<serde_json::Value> = saved.lines().map(|line| serde_json::from_str(line).unwrap()).collect();
        assert_eq!(reports, accepted, "Success preceded a durable complete report, or an invalid report was saved");
        assert!(!saved.contains("private exception message"));
    }
    server.abort();
    let _ = server.await;
    fs::remove_dir_all(root).await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn pings_clients_and_reaps_missing_pongs_without_waiting_for_commands() {
    use super::*;
    use crate::state::StateStore;
    use tokio_tungstenite::{connect_async, tungstenite::Message as ClientMessage};

    let root = std::env::temp_dir().join(format!("tau-ws-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).await.unwrap();
    let config = Config {
        transfer_bind: "127.0.0.1:0".parse().unwrap(), transfer_bind_v6:None,
        bind: "127.0.0.1:0".parse().unwrap(), token: Arc::from("test-token"),
        settings_path: root.join("settings.json"), import_pi_dir: None, codex_auth_source:None,
        cwd: root.clone(), database_path: root.join("tau.sqlite3"),
        telemetry_path: root.join("crashes.jsonl"),
        attachment_root: root.join("outbox"), upload_root: root.join("uploads"),

    };
    let manager = AgentManager::new(config.clone(), StateStore::load(config.database_path.clone()).await.unwrap()).await.unwrap();
    let id = manager.create_session(None, "general").await.unwrap();
    let other = manager.create_session(Some(&id), "general").await.unwrap();
    let state = AppState { config, manager: manager.clone(), telemetry_gate: Arc::new(Mutex::new(())),
        transfers: Arc::new(tau_net::native::Server::bind("127.0.0.1:0".parse().unwrap(),Arc::new(manager.clone())).await.unwrap()), requests:Arc::new(tokio::sync::Semaphore::new(32)),admissions:Arc::new(tokio::sync::Semaphore::new(128)) };
    let (closed_tx, mut closed_rx) = mpsc::unbounded_channel();
    let app = Router::new().route("/", get(move |upgrade: WebSocketUpgrade| {
        let state = state.clone();
        let closed = closed_tx.clone();
        async move { upgrade.on_upgrade(move |socket| async move {
            serve_socket(socket, state).await;
            let _ = closed.send(());
        }) }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}/", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
    let (mut healthy, _) = connect_async(&url).await.unwrap();
    let (mut quiet, _) = connect_async(&url).await.unwrap();
    let (mut wrong, _) = connect_async(&url).await.unwrap();
    for socket in [&mut healthy, &mut quiet, &mut wrong] {
        socket.send(ClientMessage::Text(json!({"id":"open", "type":"get_session", "sessionId":id}).to_string().into())).await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            let mut hello = false;
            let mut snapshot = false;
            loop {
                let message = socket.next().await.unwrap().unwrap();
                let message: serde_json::Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
                match message["type"].as_str().unwrap() {
                    "hello" => { assert_eq!(message["protocolVersion"], PROTOCOL_VERSION); hello = true; }
                    "session_state" => snapshot = true,
                    "response" => { assert_eq!(message["ok"], true); break; }
                    _ => {}
                }
            }
            assert!(hello && snapshot);
        }).await.unwrap();
    }
    healthy.send(ClientMessage::Text(json!({"id":"open-other", "type":"get_session", "sessionId":other}).to_string().into())).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let message = healthy.next().await.unwrap().unwrap();
            let message: serde_json::Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
            if message["requestId"] == "open-other" { assert_eq!(message["ok"], true); break; }
        }
    }).await.unwrap();
    healthy.send(ClientMessage::Ping(Bytes::from_static(b"client-ping"))).await.unwrap();
    let pong = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let message = healthy.next().await.unwrap().unwrap();
            if !matches!(message, ClientMessage::Text(_)) { break message; }
        }
    }).await.unwrap();
    assert_eq!(pong, ClientMessage::Pong(Bytes::from_static(b"client-ping")));
    for (text, request_id) in [("{", "invalid"), ("{\"id\":\"\",\"type\":\"list_sessions\"}", "")] {
        healthy.send(ClientMessage::Text(text.into())).await.unwrap();
        let response = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let message = healthy.next().await.unwrap().unwrap();
                let message: serde_json::Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
                if message["requestId"] == request_id { break message; }
            }
        }).await.unwrap();
        assert_eq!(response["type"], "response");
        assert_eq!(response["requestId"], request_id);
        assert_eq!(response["ok"], false);
    }
    tokio::time::pause();
    tokio::time::advance(WS_PING_INTERVAL).await;
    tokio::time::resume();
    let mut pings = Vec::new();
    for socket in [&mut healthy, &mut quiet, &mut wrong] {
        pings.push(tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match socket.next().await.unwrap().unwrap() {
                    ClientMessage::Ping(payload) => break payload,
                    ClientMessage::Text(_) => {}
                    message => panic!("unexpected WebSocket frame: {message:?}"),
                }
            }
        }).await.unwrap());
    }
    assert_ne!(pings[0], pings[2]);
    healthy.flush().await.unwrap();
    wrong.send(ClientMessage::Pong(pings[0].clone())).await.unwrap();
    for socket in [&mut healthy, &mut wrong] {
        socket.send(ClientMessage::Text(json!({"id":"traffic", "type":"list_sessions"}).to_string().into())).await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let message = socket.next().await.unwrap().unwrap();
                let message: serde_json::Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
                if message["requestId"] == "traffic" { assert_eq!(message["ok"], true); break; }
            }
        }).await.unwrap();
    }
    tokio::time::pause();
    tokio::time::advance(WS_PING_INTERVAL).await;
    tokio::time::resume();
    for _ in 0..2 {
        tokio::time::timeout(Duration::from_secs(5), closed_rx.recv()).await.unwrap().unwrap();
    }
    let ping = tokio::time::timeout(Duration::from_secs(5), healthy.next()).await.unwrap().unwrap().unwrap();
    let ClientMessage::Ping(payload) = ping else { panic!("expected next ping, got {ping:?}"); };
    assert_ne!(pings[0], payload);
    healthy.flush().await.unwrap();
    healthy.close(None).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), closed_rx.recv()).await.unwrap().unwrap();
    tokio::time::timeout(Duration::from_secs(5), manager.session_state_message(&id)).await.unwrap().unwrap();
    manager.shutdown().await;
    server.abort();
    let _ = server.await;
    fs::remove_dir_all(root).await.unwrap();
}

#[test]
fn bounds_file_names_and_resource_keys() {
    for (name, expected) in [
        ("../source file.rs", "source_file.rs"),
        ("C:\\tmp\\.résumé_1-.txt.", "r_sum__1-.txt"),
        ("bad\r\n\";name.zip", "bad____name.zip"),
        ("...", "attachment"),
        ("", "attachment"),
    ] {
        assert_eq!(safe_file_name(name), expected);
    }
    assert_eq!(safe_file_name(&"a".repeat(200)), "a".repeat(160));
    assert_eq!(safe_file_name(&format!("{}a", ".".repeat(160))), "attachment");

}


#[test]
fn accepts_only_the_complete_bearer_token() {
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer abcdef"));
    assert!(authorized(&headers, "abcdef"));
    assert!(!authorized(&headers, "abcdeg"));
    assert!(!authorized(&headers, "abcdef0"));
    headers.insert(AUTHORIZATION, HeaderValue::from_static("abcdef"));
    assert!(!authorized(&headers, "abcdef"));
}
