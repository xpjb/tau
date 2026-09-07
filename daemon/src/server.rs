use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use axum::body::{Body, Bytes};
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{DefaultBodyLimit, Path as AxumPath, Query, State, WebSocketUpgrade};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncWriteExt;
use tokio::sync::{Mutex, mpsc};
use tokio_util::io::ReaderStream;
use tracing::{info, warn};

use crate::config::Config;
use crate::manager::{AgentManager, safe_file_name};
use crate::protocol::{
    ClientCommand, ClientRequest, CrashReport, MAX_CRASH_BYTES, MAX_PROMPT_CHARS,
    MAX_REQUEST_BYTES, MAX_UPLOAD_BYTES, PROTOCOL_VERSION, ServerMessage,
};

const WS_PING_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Clone)]
struct AppState {
    config: Config,
    manager: AgentManager,
    telemetry_gate: Arc<Mutex<()>>,
}

pub async fn serve(config: Config, manager: AgentManager) -> Result<()> {
    let state = AppState {
        config: config.clone(),
        manager: manager.clone(),
        telemetry_gate: Arc::new(Mutex::new(())),
    };
    let app = Router::new()
        .route("/v1/health", get(|| async { Json(json!({
            "name": "Tau",
            "version": env!("CARGO_PKG_VERSION"),
            "protocolVersion": PROTOCOL_VERSION
        })) }))
        .route("/v1/ws", get(websocket))
        .route(
            "/v1/sessions/{session_id}/attachments/{entry_id}",
            get(download_attachment),
        )
        .route(
            "/v1/sessions/{session_id}/uploads",
            post(upload_file).layer(DefaultBodyLimit::max(MAX_UPLOAD_BYTES)),
        )
        .route(
            "/v1/telemetry/crash",
            post(crash_report).layer(DefaultBodyLimit::max(MAX_CRASH_BYTES)),
        )
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(config.bind)
        .await
        .with_context(|| format!("failed to bind {}", config.bind))?;
    info!(address = %config.bind, "Tau daemon is listening");

    let result = axum::serve(listener, app)
        .with_graceful_shutdown(async {
            #[cfg(unix)]
            {
                match tokio::signal::unix::signal(
                    tokio::signal::unix::SignalKind::terminate(),
                ) {
                    Ok(mut terminate) => {
                        tokio::select! {
                            _ = tokio::signal::ctrl_c() => {}
                            _ = terminate.recv() => {}
                        }
                    }
                    Err(error) => {
                        warn!(%error, "failed to install Tau SIGTERM listener");
                        let _ = tokio::signal::ctrl_c().await;
                    }
                }
            }
            #[cfg(not(unix))]
            {
                let _ = tokio::signal::ctrl_c().await;
            }
            info!("Tau shutdown requested");
        })
        .await
        .context("Tau HTTP server stopped unexpectedly");
    manager.shutdown().await;
    result
}

async fn websocket(
    State(state): State<AppState>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Response {
    if !authorized(&headers, &state.config.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    upgrade
        .max_message_size(MAX_REQUEST_BYTES)
        .on_upgrade(move |socket| serve_socket(socket, state))
        .into_response()
}

async fn serve_socket(socket: WebSocket, state: AppState) {
    let (mut socket_tx, mut socket_rx) = socket.split();
    let (outbound_tx, mut outbound_rx) = mpsc::channel::<Message>(512);
    let writer = tokio::spawn(async move {
        while let Some(message) = outbound_rx.recv().await {
            if socket_tx.send(message).await.is_err() {
                break;
            }
        }
    });

    queue_server(
        &outbound_tx,
        &ServerMessage::Hello {
            protocol_version: PROTOCOL_VERSION,
            daemon_version: env!("CARGO_PKG_VERSION"),
        },
    )
    .await;
    queue_server(&outbound_tx, &state.manager.sessions_message().await).await;

    let mut events = state.manager.subscribe();
    let event_outbound = outbound_tx.clone();
    let event_forwarder = tokio::spawn(async move {
        loop {
            let message = match events.recv().await {
                Ok(message) => message,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    ServerMessage::ResyncRequired { session_id: None }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };
            if !queue_server(&event_outbound, &message).await {
                break;
            }
        }
    });

    let mut subscriptions = HashMap::<String, tokio::task::JoinHandle<()>>::new();
    let mut heartbeat = tokio::time::interval_at(tokio::time::Instant::now() + WS_PING_INTERVAL, WS_PING_INTERVAL);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut pending_ping = None;
    loop {
        let incoming = tokio::select! {
            incoming = socket_rx.next() => incoming,
            _ = heartbeat.tick() => {
                if pending_ping.is_some() { break; }
                let payload = Bytes::copy_from_slice(uuid::Uuid::new_v4().as_bytes());
                if outbound_tx.try_send(Message::Ping(payload.clone())).is_err() { break; }
                pending_ping = Some(payload);
                continue;
            }
        };
        let Some(incoming) = incoming else { break; };
        let message = match incoming {
            Ok(message) => message,
            Err(error) => {
                warn!(%error, "Tau WebSocket client failed");
                break;
            }
        };
        match message {
            Message::Text(text) => {
                if text.len() > MAX_REQUEST_BYTES {
                    break;
                }
                let request = match serde_json::from_str::<ClientRequest>(&text) {
                    Ok(request) if !request.id.is_empty() && request.id.len() <= 128 => Ok(request),
                    Ok(request) => Err(ServerMessage::failure(request.id, "invalid request id")),
                    Err(error) => Err(ServerMessage::failure("invalid".to_owned(), format!("invalid request: {error}"))),
                };
                let request = match request {
                    Ok(request) => request,
                    Err(response) => {
                        let Ok(encoded) = serde_json::to_string(&response) else { break; };
                        if outbound_tx.try_send(Message::Text(encoded.into())).is_err() { break; }
                        continue;
                    }
                };
                let manager = state.manager.clone();
                let response_outbound = outbound_tx.clone();
                if let ClientCommand::OpenSession { session_id, requests, streams } = &request.command {
                    for (_, task) in subscriptions.drain() { task.abort(); let _ = task.await; }
                    let session_id = session_id.clone();
                    let requests = requests.clone();
                    let streams = streams.clone();
                    subscriptions.insert(session_id.clone(), tokio::spawn(async move {
                        let mut feed = match manager.open_session(&session_id, &requests, &streams).await {
                            Ok(feed) => feed,
                            Err(error) => {
                                let mut response = ServerMessage::command_failure(request.id, error);
                                if let ServerMessage::Response { session_id: field, .. } = &mut response { *field = Some(session_id); }
                                queue_server(&response_outbound, &response).await;
                                return;
                            }
                        };
                        for message in feed.initial {
                            if !queue_server(&response_outbound, &message).await { return; }
                        }
                        if !queue_server(&response_outbound, &ServerMessage::success(request.id, Some(session_id.clone()), None)).await { return; }
                        loop {
                            match feed.events.recv().await {
                                Ok(message) => if !queue_server(&response_outbound, &message).await { break; },
                                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                                    queue_server(&response_outbound, &ServerMessage::ResyncRequired { session_id: Some(session_id) }).await;
                                    break;
                                }
                                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                            }
                        }
                    }));
                    continue;
                }
                tokio::spawn(async move {
                    let request_id = request.id;
                    let response = match request.command {
                        ClientCommand::ListSessions => {
                            if !queue_server(&response_outbound, &manager.sessions_message().await).await {
                                return;
                            }
                            ServerMessage::success(request_id, None, None)
                        }
                        ClientCommand::CreateSession => match manager.create_session().await {
                            Ok(session_id) => ServerMessage::success(
                                request_id,
                                Some(session_id),
                                None,
                            ),
                            Err(error) => ServerMessage::command_failure(request_id, error),
                        },
                        ClientCommand::OpenSession { .. } => unreachable!("open requests own their transcript feed"),
                        ClientCommand::GetHistory { session_id, generation, before } => {
                            match manager.history_page(&session_id, &generation, &before).await {
                                Ok(page) => {
                                    if !queue_server(&response_outbound, &ServerMessage::TranscriptPage {
                                        request_id: request_id.clone(), session_id: session_id.clone(), generation, cursor: before, page,
                                    }).await { return; }
                                    ServerMessage::success(request_id, Some(session_id), None)
                                }
                                Err(error) => {
                                    let mut response = ServerMessage::command_failure(request_id, error);
                                    if let ServerMessage::Response { session_id: field, .. } = &mut response { *field = Some(session_id); }
                                    response
                                }
                            }
                        }
                        ClientCommand::GetCommands { session_id } => {
                            match manager.commands(&session_id).await {
                                Ok(commands) => {
                                    if !queue_server(
                                        &response_outbound,
                                        &ServerMessage::Commands {
                                            session_id: session_id.clone(),
                                            commands,
                                        },
                                    ).await {
                                        return;
                                    }
                                    ServerMessage::success(
                                        request_id,
                                        Some(session_id),
                                        None,
                                    )
                                }
                                Err(error) => ServerMessage::command_failure(request_id, error),
                            }
                        }
                        ClientCommand::Prompt { session_id, text } => {
                            if text.chars().count() > MAX_PROMPT_CHARS {
                                ServerMessage::failure(request_id, "message is too large")
                            } else {
                                match manager.prompt(&session_id, &text, &request_id).await {
                                    Ok(outcome) => ServerMessage::prompt_success(
                                        request_id,
                                        session_id,
                                        outcome.disposition,
                                        outcome.notice,
                                    ),
                                    Err(error) => {
                                        ServerMessage::command_failure(request_id, error)
                                    }
                                }
                            }
                        }
                        ClientCommand::ExtensionUiResponse {
                            session_id,
                            request_id: extension_request_id,
                            value,
                            confirmed,
                            cancelled,
                        } => {
                            match manager.extension_ui_response(
                                &session_id,
                                &extension_request_id,
                                value,
                                confirmed,
                                cancelled,
                            ).await {
                                Ok(()) => ServerMessage::success(
                                    request_id,
                                    Some(session_id),
                                    None,
                                ),
                                Err(error) => ServerMessage::command_failure(request_id, error),
                            }
                        }
                        ClientCommand::QueueControl { session_id, generation, operation } => {
                            match manager.queue_control(&session_id, &generation, &request_id, operation).await {
                                Ok(outcome) => {
                                    let mut response = ServerMessage::success(request_id, Some(session_id), None);
                                    if let ServerMessage::Response { outcome: field, .. } = &mut response { *field = Some(outcome); }
                                    response
                                }
                                Err(error) => ServerMessage::command_failure(request_id, error),
                            }
                        }
                        ClientCommand::Abort { session_id } => {
                            match manager.abort(&session_id).await {
                                Ok(()) => ServerMessage::success(
                                    request_id,
                                    Some(session_id),
                                    None,
                                ),
                                Err(error) => ServerMessage::command_failure(request_id, error),
                            }
                        }
                        ClientCommand::CloseSession { session_id } => {
                            match manager.close_session(&session_id).await {
                                Ok(()) => ServerMessage::success(
                                    request_id,
                                    Some(session_id),
                                    None,
                                ),
                                Err(error) => ServerMessage::command_failure(request_id, error),
                            }
                        }
                        ClientCommand::DeleteSession { session_id } => {
                            match manager.delete_session(&session_id).await {
                                Ok(()) => ServerMessage::success(
                                    request_id,
                                    Some(session_id),
                                    None,
                                ),
                                Err(error) => ServerMessage::command_failure(request_id, error),
                            }
                        }
                        ClientCommand::RenameSession { session_id, title } => {
                            match manager.rename_session(&session_id, &title).await {
                                Ok(()) => ServerMessage::success(
                                    request_id,
                                    Some(session_id),
                                    None,
                                ),
                                Err(error) => ServerMessage::command_failure(request_id, error),
                            }
                        }
                        ClientCommand::ForkSession {
                            session_id,
                            entry_id,
                        } => match manager.fork_session(&session_id, &entry_id).await {
                            Ok((child, draft)) => ServerMessage::success(
                                request_id,
                                Some(child),
                                draft,
                            ),
                            Err(error) => ServerMessage::command_failure(request_id, error),
                        },
                        ClientCommand::CloneSession { session_id } => {
                            match manager.clone_session(&session_id).await {
                                Ok(child) => ServerMessage::success(
                                    request_id,
                                    Some(child),
                                    None,
                                ),
                                Err(error) => ServerMessage::command_failure(request_id, error),
                            }
                        }
                    };
                    queue_server(&response_outbound, &response).await;
                });
            }
            Message::Pong(bytes) => {
                if pending_ping.as_ref() == Some(&bytes) { pending_ping = None; }
            }
            Message::Close(_) => break,
            Message::Binary(_) | Message::Ping(_) => {}
        }
    }

    event_forwarder.abort();
    writer.abort();
    for (_, task) in subscriptions { task.abort(); let _ = task.await; }
    drop(outbound_tx);
    let _ = event_forwarder.await;
    let _ = writer.await;
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UploadQuery {
    file_name: String,
}

async fn upload_file(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(session_id): AxumPath<String>,
    Query(query): Query<UploadQuery>,
    body: Bytes,
) -> Response {
    if !authorized(&headers, &state.config.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if !valid_resource_key(&session_id)
        || query.file_name.trim().is_empty()
        || query.file_name.chars().count() > 256
        || body.is_empty()
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    match state
        .manager
        .store_upload(&session_id, &query.file_name, &body)
        .await
    {
        Ok(file) => (StatusCode::CREATED, Json(file)).into_response(),
        Err(error) => {
            warn!(session = %session_id, %error, "Tau file upload failed");
            StatusCode::BAD_REQUEST.into_response()
        }
    }
}

fn valid_resource_key(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
}

async fn download_attachment(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath((session_id, entry_id)): AxumPath<(String, String)>,
) -> Response {
    if !authorized(&headers, &state.config.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if !valid_resource_key(&session_id) || !valid_resource_key(&entry_id) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let attachment = match state
        .manager
        .resolve_attachment(&session_id, &entry_id)
        .await
    {
        Ok(attachment) => attachment,
        Err(error) => {
            warn!(session = %session_id, entry = %entry_id, %error, "Tau attachment was not available");
            return StatusCode::NOT_FOUND.into_response();
        }
    };
    let disposition = match HeaderValue::from_str(&format!(
        "attachment; filename=\"{}\"",
        safe_file_name(&attachment.file_name)
    )) {
        Ok(value) => value,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    let mut response = Response::new(Body::from_stream(ReaderStream::new(attachment.file)));
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(attachment.mime_type),
    );
    response.headers_mut().insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&attachment.size.to_string())
            .expect("attachment length is a valid header"),
    );
    response
        .headers_mut()
        .insert(header::CONTENT_DISPOSITION, disposition);
    response
}

async fn crash_report(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !authorized(&headers, &state.config.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let report = match serde_json::from_slice::<CrashReport>(&body) {
        Ok(report) => report,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let valid = report.schema == 1
        && !report.report_id.is_empty()
        && report.report_id.len() <= 128
        && !report.platform.is_empty()
        && report.platform.len() <= 64
        && !report.app_version.is_empty()
        && report.app_version.len() <= 64
        && report.os_version.len() <= 192
        && report.thread.len() <= 128
        && !report.exception_class.is_empty()
        && report.exception_class.len() <= 192
        && report.stack.len() <= 64
        && report.stack.iter().all(|frame| {
            frame.class_name.len() <= 192
                && frame.method_name.len() <= 192
                && frame
                    .file_name
                    .as_ref()
                    .is_none_or(|file_name| file_name.len() <= 192)
        });
    if !valid {
        return StatusCode::BAD_REQUEST.into_response();
    }

    let mut encoded = match serde_json::to_vec(&report) {
        Ok(encoded) if encoded.len() < MAX_CRASH_BYTES => encoded,
        _ => return StatusCode::BAD_REQUEST.into_response(),
    };
    encoded.push(b'\n');
    let _guard = state.telemetry_gate.lock().await;
    let path = &state.config.telemetry_path;
    if let Some(parent) = path.parent()
        && fs::create_dir_all(parent).await.is_err()
    {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    let mut file = match OpenOptions::new().create(true).append(true).open(path).await {
        Ok(file) => file,
        Err(error) => {
            warn!(%error, path = %path.display(), "failed to open Tau crash log");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    if let Err(error) = file.write_all(&encoded).await {
        warn!(%error, path = %path.display(), "failed to append Tau crash report");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    if let Err(error) = file.sync_data().await {
        warn!(%error, path = %path.display(), "failed to sync Tau crash report");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    warn!(
        report_id = %report.report_id,
        platform = %report.platform,
        app_version = %report.app_version,
        exception = %report.exception_class,
        "Tau client crash report received"
    );
    StatusCode::NO_CONTENT.into_response()
}

async fn queue_server(outbound: &mpsc::Sender<Message>, message: &ServerMessage) -> bool {
    let Ok(encoded) = serde_json::to_string(message) else {
        return false;
    };
    outbound.send(Message::Text(encoded.into())).await.is_ok()
}

fn authorized(headers: &HeaderMap, expected: &str) -> bool {
    let Some(actual) = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
    else {
        return false;
    };
    let actual = actual.as_bytes();
    let expected = expected.as_bytes();
    let mut difference = actual.len() ^ expected.len();
    let length = actual.len().max(expected.len());
    for index in 0..length {
        difference |= usize::from(
            actual.get(index).copied().unwrap_or_default()
                ^ expected.get(index).copied().unwrap_or_default(),
        );
    }
    difference == 0
}

#[cfg(test)]
mod tests {
    use axum::http::{HeaderMap, HeaderValue, header::AUTHORIZATION};

    use super::{authorized, safe_file_name, valid_resource_key};

    #[cfg(unix)]
    #[tokio::test]
    async fn pings_clients_and_reaps_missing_pongs_without_waiting_for_commands() {
        use std::os::unix::fs::PermissionsExt;
        use super::*;
        use crate::state::StateStore;
        use tokio_tungstenite::{connect_async, tungstenite::Message as ClientMessage};

        let root = std::env::temp_dir().join(format!("tau-ws-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).await.unwrap();
        let mock = root.join("pi.py");
        fs::write(&mock, include_str!("../tests/fixtures/pi.py")).await.unwrap();
        fs::set_permissions(&mock, std::fs::Permissions::from_mode(0o700)).await.unwrap();
        let config = Config {
            bind: "127.0.0.1:0".parse().unwrap(), token: Arc::from("test-token"),
            pi_command: mock, default_thinking_level: "high".to_owned(),
            cwd: root.clone(), state_path: root.join("state.json"), session_dir: root.join("pi-sessions"),
            telemetry_path: root.join("crashes.jsonl"), pi_extension_path: root.join("extension.ts"),
            attachment_root: root.join("outbox"), upload_root: root.join("uploads"),
        };
        let manager = AgentManager::new(config.clone(), StateStore::load(config.state_path.clone()).await.unwrap());
        let id = manager.create_session().await.unwrap();
        let state = AppState { config, manager: manager.clone(), telemetry_gate: Arc::new(Mutex::new(())) };
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
            socket.send(ClientMessage::Text(json!({"id":"open", "type":"open_session", "sessionId":id}).to_string().into())).await.unwrap();
            tokio::time::timeout(Duration::from_secs(5), async {
                let mut hello = false;
                let mut snapshot = false;
                loop {
                    let message = socket.next().await.unwrap().unwrap();
                    let message: serde_json::Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
                    match message["type"].as_str().unwrap() {
                        "hello" => { assert_eq!(message["protocolVersion"], PROTOCOL_VERSION); hello = true; }
                        "transcript_snapshot" => snapshot = true,
                        "response" => { assert_eq!(message["ok"], true); break; }
                        _ => {}
                    }
                }
                assert!(hello && snapshot);
            }).await.unwrap();
        }
        healthy.send(ClientMessage::Ping(Bytes::from_static(b"client-ping"))).await.unwrap();
        assert_eq!(tokio::time::timeout(Duration::from_secs(5), healthy.next()).await.unwrap().unwrap().unwrap(),
            ClientMessage::Pong(Bytes::from_static(b"client-ping")));
        for (text, request_id) in [("{", "invalid"), ("{\"id\":\"\",\"type\":\"list_sessions\"}", "")] {
            healthy.send(ClientMessage::Text(text.into())).await.unwrap();
            let response = tokio::time::timeout(Duration::from_secs(5), healthy.next()).await.unwrap().unwrap().unwrap();
            let response: serde_json::Value = serde_json::from_str(response.to_text().unwrap()).unwrap();
            assert_eq!(response["type"], "response");
            assert_eq!(response["requestId"], request_id);
            assert_eq!(response["ok"], false);
        }
        wrong.send(ClientMessage::Text(json!({"id":"dialog", "type":"prompt", "sessionId":id, "text":"/choose"}).to_string().into())).await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let message = wrong.next().await.unwrap().unwrap();
                let message: serde_json::Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
                if message["type"] == "extension_ui" { break; }
            }
        }).await.unwrap();
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
        manager.extension_ui_response(&id, "dialog-1", None, None, true).await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), manager.commands(&id)).await.unwrap().unwrap();
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
        assert!(valid_resource_key("Ab_01-xy"));
        assert!(valid_resource_key(&"a".repeat(128)));
        for key in ["", ".", "../chat", "a/b", "a\\b", "a b", "é", &"a".repeat(129)] {
            assert!(!valid_resource_key(key));
        }
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
}
