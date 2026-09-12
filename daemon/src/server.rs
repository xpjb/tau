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

pub async fn serve(config: Config, manager: AgentManager, listener: tokio::net::TcpListener) -> Result<()> {
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
        .route("/v1/sessions/{session_id}/flags", post(flag_it).layer(DefaultBodyLimit::max(32 * 1024)))
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

#[derive(Deserialize)]
struct FlagInput {
    text: String,
}

async fn flag_it(
    State(state): State<AppState>,
    AxumPath(session_id): AxumPath<String>,
    headers: HeaderMap,
    Json(input): Json<FlagInput>,
) -> Response {
    let Some(token) = headers.get(header::AUTHORIZATION).and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer ")) else {
        return (StatusCode::UNAUTHORIZED, Json(json!({"error":"Tau flag capability is required"}))).into_response();
    };
    if input.text.trim().is_empty() || input.text.chars().count() > crate::state::MAX_FLAG_CHARS {
        return (StatusCode::BAD_REQUEST, Json(json!({"error":"Flag text must contain 1–4096 characters"}))).into_response();
    }
    match state.manager.flag(&session_id, token, &input.text).await {
        Ok(Some(flag)) => Json(flag).into_response(),
        Ok(None) => (StatusCode::UNAUTHORIZED, Json(json!({"error":"Tau flag capability is invalid or expired"}))).into_response(),
        Err(error) => {
            warn!(session = %session_id, %error, "Tau flag save failed");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error":"Tau could not confirm the flag save; check flags.jsonl before retrying"}))).into_response()
        }
    }
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
                if let ClientCommand::OpenSession { session_id, requests } = &request.command {
                    if let Some(task) = subscriptions.remove(session_id) { task.abort(); let _ = task.await; }
                    let session_id = session_id.clone();
                    let requests = requests.clone();
                    subscriptions.insert(session_id.clone(), tokio::spawn(async move {
                        let mut feed = match manager.open_session(&session_id, &requests).await {
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
                        ClientCommand::SetTitlePrompt { prompt } if prompt.chars().count() > MAX_PROMPT_CHARS => {
                            ServerMessage::failure(request_id, "Title prompt is too long")
                        }
                        command @ (ClientCommand::GetTitlePrompt | ClientCommand::SetTitlePrompt { .. }) => {
                            let replacement = match command {
                                ClientCommand::SetTitlePrompt { prompt } => Some(prompt),
                                _ => None,
                            };
                            match manager.inner.state.title_prompt(replacement).await {
                                Ok(prompt) => {
                                    queue_server(&response_outbound, &ServerMessage::TitlePrompt {
                                        request_id: request_id.clone(), prompt,
                                        default_prompt: crate::state::DEFAULT_TITLE_PROMPT,
                                    }).await;
                                    ServerMessage::success(request_id, None, None)
                                }
                                Err(error) => ServerMessage::command_failure(request_id, error),
                            }
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
                            match manager.history_page(&session_id, &generation, before).await {
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
    async fn saves_flags_and_notifies_all_clients_with_worker_scoped_access() {
        use std::os::unix::fs::{PermissionsExt, MetadataExt};
        use super::*;
        use crate::state::StateStore;
        use tokio::io::AsyncReadExt;
        use tokio_tungstenite::{connect_async, tungstenite::client::IntoClientRequest};

        async fn submit(address: std::net::SocketAddr, id: &str, token: &str, text: &str) -> (u16, serde_json::Value) {
            tokio::time::timeout(Duration::from_secs(5), async {
                let body = json!({"text":text, "sessionId":"cannot-spoof-source"}).to_string();
                let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
                stream.write_all(format!("POST /v1/sessions/{id}/flags HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
                let mut bytes = Vec::new();
                stream.read_to_end(&mut bytes).await.unwrap();
                let reply = String::from_utf8(bytes).unwrap();
                let (header, body) = reply.split_once("\r\n\r\n").unwrap();
                (header.split_whitespace().nth(1).unwrap().parse().unwrap(), serde_json::from_str(body).unwrap())
            }).await.unwrap()
        }

        let root = std::env::temp_dir().join(format!("tau-flags-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).await.unwrap();
        let pi = root.join("pi.py");
        fs::write(&pi, include_str!("../tests/fixtures/pi.py")).await.unwrap();
        fs::set_permissions(&pi, std::fs::Permissions::from_mode(0o700)).await.unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let config = Config {
            bind: address, token: Arc::from("full-client-token"), pi_command: pi,
            default_thinking_level: "high".to_owned(), cwd: root.clone(), state_path: root.join("state.json"),
            session_dir: root.join("pi-sessions"), telemetry_path: root.join("crashes.jsonl"),
            pi_extension_path: root.join("extension.ts"), attachment_root: root.join("outbox"),
            upload_root: root.join("uploads"), title_command: None,
        };
        let manager = AgentManager::new(config.clone(), StateStore::load(config.state_path.clone()).await.unwrap());
        let first = manager.create_session().await.unwrap();
        let second = manager.create_session().await.unwrap();
        manager.rename_session(&first, "Flag source").await.unwrap();
        let mut tokens = Vec::new();
        for id in [&first, &second] {
            manager.commands(id).await.unwrap();
            let session = manager.inner.state.get(id).unwrap().session_file.unwrap();
            let capability: serde_json::Value = serde_json::from_slice(&fs::read(format!("{session}.flag-capability")).await.unwrap()).unwrap();
            assert_eq!(capability["url"], format!("http://{address}/v1/sessions/{id}/flags"));
            assert_eq!(capability["clientTokenPresent"], false);
            tokens.push(capability["token"].as_str().unwrap().to_owned());
        }
        assert_ne!(tokens[0], tokens[1]);
        let session = manager.inner.state.get(&first).unwrap();
        let history = fs::read(session.session_file.as_ref().unwrap()).await.unwrap();
        let state = AppState { config: config.clone(), manager: manager.clone(), telemetry_gate: Arc::new(Mutex::new(())) };
        let app = Router::new().route("/v1/ws", get(websocket))
            .route("/v1/sessions/{session_id}/flags", post(flag_it).layer(DefaultBodyLimit::max(32 * 1024))).with_state(state);
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
        let mut request = format!("ws://{address}/v1/ws").into_client_request().unwrap();
        request.headers_mut().insert(AUTHORIZATION, format!("Bearer {}", tokens[0]).parse().unwrap());
        assert!(connect_async(request.clone()).await.is_err(), "Flag token granted client access");
        request.headers_mut().insert(AUTHORIZATION, "Bearer full-client-token".parse().unwrap());
        let mut clients = Vec::new();
        for _ in 0..2 {
            let (mut socket, _) = connect_async(request.clone()).await.unwrap();
            for _ in 0..2 { socket.next().await.unwrap().unwrap(); }
            clients.push(socket);
        }
        let log = root.join("flags.jsonl");
        for (id, token, text, status) in [
            (first.as_str(), "full-client-token", "Denied", 401),
            (second.as_str(), tokens[0].as_str(), "Wrong source", 401),
            ("missing", tokens[0].as_str(), "Missing source", 401),
            (first.as_str(), tokens[0].as_str(), " \n ", 400),
            (first.as_str(), tokens[0].as_str(), &"x".repeat(4097), 400),
        ] {
            assert_eq!(submit(address, id, token, text).await.0, status);
            assert!(!log.exists());
        }
        let texts = ["Private cache 🔧\nQuote: \"build-dir\"".to_owned(), "x".repeat(4096)];
        let mut flags = Vec::new();
        for text in &texts {
            let (status, saved) = submit(address, &first, &tokens[0], text).await;
            assert_eq!(status, 200);
            assert_eq!(saved["text"], *text);
            assert_eq!(saved["sessionId"], first);
            assert_eq!(saved["sessionTitle"], "Flag source");
            assert!(saved["timestampMs"].as_u64().unwrap() > 0);
            assert!(uuid::Uuid::parse_str(saved["id"].as_str().unwrap()).is_ok());
            flags.push(saved.clone());
            let persisted: Vec<serde_json::Value> = fs::read_to_string(&log).await.unwrap().lines().map(|line| serde_json::from_str(line).unwrap()).collect();
            assert_eq!(persisted, flags, "Success preceded a complete durable log entry");
            for socket in &mut clients {
                let frame = tokio::time::timeout(Duration::from_secs(5), socket.next()).await.unwrap().unwrap().unwrap();
                let message: serde_json::Value = serde_json::from_str(frame.to_text().unwrap()).unwrap();
                assert_eq!(message["type"], "extension_ui");
                assert_eq!(message["sessionId"], first);
                assert_eq!(message["request"]["method"], "notify");
                assert_eq!(message["request"]["notifyType"], "info");
                assert!(message["request"]["message"].as_str().unwrap().starts_with(&format!("Flagged {}", saved["id"].as_str().unwrap())));
                assert!(message["request"]["message"].as_str().unwrap().chars().count() < 480);
            }
        }
        assert_eq!(fs::metadata(&log).await.unwrap().mode() & 0o777, 0o600);
        {
            let mut cancelled = Box::pin(manager.flag(&first, &tokens[0], "Caller disconnected"));
            assert!(futures_util::poll!(&mut cancelled).is_pending());
        }
        for socket in &mut clients {
            let frame = tokio::time::timeout(Duration::from_secs(5), socket.next()).await.unwrap().unwrap().unwrap();
            let message: serde_json::Value = serde_json::from_str(frame.to_text().unwrap()).unwrap();
            assert_eq!(message["request"]["notifyType"], "info");
            assert!(message["request"]["message"].as_str().unwrap().contains("Caller disconnected"));
        }
        let completed: Vec<serde_json::Value> = fs::read_to_string(&log).await.unwrap().lines().map(|line| serde_json::from_str(line).unwrap()).collect();
        assert_eq!(completed.len(), 3);
        assert_eq!(completed.last().unwrap()["text"], "Caller disconnected");
        let before_failure = fs::read(&log).await.unwrap();
        fs::rename(&log, root.join("flags-backup")).await.unwrap();
        fs::create_dir(&log).await.unwrap();
        assert_eq!(submit(address, &first, &tokens[0], "Unsaved").await.0, 500);
        for socket in &mut clients {
            let frame = tokio::time::timeout(Duration::from_secs(5), socket.next()).await.unwrap().unwrap().unwrap();
            let message: serde_json::Value = serde_json::from_str(frame.to_text().unwrap()).unwrap();
            assert_eq!(message["request"]["notifyType"], "error");
            assert!(message["request"]["message"].as_str().unwrap().starts_with("Flag save failed"));
        }
        assert_eq!(fs::read(root.join("flags-backup")).await.unwrap(), before_failure);
        fs::remove_dir(&log).await.unwrap();
        fs::rename(root.join("flags-backup"), &log).await.unwrap();
        let (left, right) = tokio::join!(submit(address, &first, &tokens[0], "Concurrent first"), submit(address, &second, &tokens[1], "Concurrent second"));
        assert_eq!((left.0, right.0), (200, 200));
        let rows: Vec<serde_json::Value> = fs::read_to_string(&log).await.unwrap().lines().map(|line| serde_json::from_str(line).unwrap()).collect();
        assert_eq!(rows.len(), 5);
        assert!(rows.contains(&left.1) && rows.contains(&right.1));
        assert_eq!(manager.inner.state.get(&first).unwrap().updated_at_ms, session.updated_at_ms);
        assert_eq!(fs::read(session.session_file.as_ref().unwrap()).await.unwrap(), history);
        manager.close_session(&first).await.unwrap();
        assert_eq!(submit(address, &first, &tokens[0], "Retired token").await.0, 401);
        manager.commands(&first).await.unwrap();
        assert_eq!(submit(address, &first, &tokens[0], "Old worker token").await.0, 401);
        for socket in &mut clients { socket.close(None).await.unwrap(); }
        manager.shutdown().await;
        server.abort();
        let restored = StateStore::load(config.state_path.clone()).await.unwrap();
        let saved = restored.flag(&first, "After restart").await.unwrap();
        assert_eq!(saved.session_id, first);
        let rows: Vec<serde_json::Value> = fs::read_to_string(&log).await.unwrap().lines().map(|line| serde_json::from_str(line).unwrap()).collect();
        assert_eq!(rows.len(), 6);
        assert_eq!(rows.last().unwrap()["id"], saved.id);
        fs::create_dir(root.join("collision")).await.unwrap();
        let collision = root.join("collision/flags.jsonl");
        fs::copy(config.state_path, &collision).await.unwrap();
        let before = fs::read(&collision).await.unwrap();
        let colliding = StateStore::load(collision.clone()).await.unwrap();
        assert!(colliding.flag(&first, "Must not overwrite state").await.is_err());
        assert_eq!(fs::read(collision).await.unwrap(), before);
        fs::remove_dir_all(root).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn saves_the_full_title_prompt_and_passes_it_to_the_generator() {
        use std::os::unix::fs::PermissionsExt;
        use super::*;
        use crate::state::{StateStore, DEFAULT_TITLE_PROMPT};
        use tokio_tungstenite::{connect_async, tungstenite::{client::IntoClientRequest, Message as ClientMessage}};

        let root = std::env::temp_dir().join(format!("tau-title-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).await.unwrap();
        let pi = root.join("pi.py");
        fs::write(&pi, include_str!("../tests/fixtures/pi.py")).await.unwrap();
        fs::set_permissions(&pi, std::fs::Permissions::from_mode(0o700)).await.unwrap();
        fs::write(root.join("title_gen.py"), include_str!("../../scripts/title_gen.py")).await.unwrap();
        fs::write(root.join("title_prompt.txt"), DEFAULT_TITLE_PROMPT).await.unwrap();
        fs::write(root.join("llama_cpp.py"), r#"import json
from pathlib import Path
class Llama:
    def __init__(self, model_path, n_ctx, n_threads, verbose):
        assert (n_ctx, n_threads, verbose) == (2048, 4, False)
        self.path = Path(model_path)
    def __call__(self, prompt, **options):
        assert options == dict(max_tokens=25, temperature=0.2, stop=["\n", "Session:", "Title:"], echo=False)
        self.path.write_text(json.dumps({"prompt": prompt}))
        return {"choices": [{"text": ' "Literal title" '}]}
"#).await.unwrap();
        let config = Config {
            bind: "127.0.0.1:0".parse().unwrap(), token: Arc::from("test-token"),
            pi_command: pi, default_thinking_level: "high".to_owned(), cwd: root.clone(),
            state_path: root.join("state.json"), session_dir: root.join("pi-sessions"),
            telemetry_path: root.join("crashes.jsonl"), pi_extension_path: root.join("extension.ts"),
            attachment_root: root.join("outbox"), upload_root: root.join("uploads"),
            title_command: Some(format!("python3 {} --model {}", root.join("title_gen.py").display(), root.join("model-call.json").display())),
        };
        fs::write(&config.state_path, r#"{"schema":1,"sessions":{}}"#).await.unwrap();
        let manager = AgentManager::new(config.clone(), StateStore::load(config.state_path.clone()).await.unwrap());
        let existing = manager.create_session().await.unwrap();
        manager.rename_session(&existing, "Existing title").await.unwrap();
        let state = AppState { config: config.clone(), manager: manager.clone(), telemetry_gate: Arc::new(Mutex::new(())) };
        let app = Router::new().route("/v1/ws", get(websocket)).with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}/v1/ws", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
        assert!(connect_async(&url).await.is_err());
        let mut request = url.into_client_request().unwrap();
        request.headers_mut().insert(AUTHORIZATION, "Bearer test-token".parse().unwrap());
        let (mut first, _) = connect_async(request.clone()).await.unwrap();
        let (mut second, _) = connect_async(request).await.unwrap();
        let custom = "  Literal \"rules\" 🔧\n{text}\nAgain: {text}\n  ";
        for (index, (command, expected)) in [
            (json!({"type":"get_title_prompt"}), Some(DEFAULT_TITLE_PROMPT)),
            (json!({"type":"set_title_prompt", "prompt":custom}), Some(custom)),
            (json!({"type":"get_title_prompt"}), Some(custom)),
            (json!({"type":"set_title_prompt", "prompt":""}), Some("")),
            (json!({"type":"get_title_prompt"}), Some("")),
            (json!({"type":"set_title_prompt", "prompt":custom}), Some(custom)),
            (json!({"type":"set_title_prompt", "prompt":"x".repeat(MAX_PROMPT_CHARS + 1)}), None),
            (json!({"type":"set_title_prompt", "prompt":"Unsaved"}), None),
        ].into_iter().enumerate() {
            if index == 7 {
                fs::rename(&config.state_path, root.join("backup.json")).await.unwrap();
                fs::create_dir(&config.state_path).await.unwrap();
            }
            let mut command = command;
            command["id"] = json!(format!("setting-{index}"));
            let socket = if index % 2 == 0 { &mut first } else { &mut second };
            socket.send(ClientMessage::Text(command.to_string().into())).await.unwrap();
            tokio::time::timeout(Duration::from_secs(5), async {
                let mut prompt = None;
                loop {
                    let frame = socket.next().await.unwrap().unwrap();
                    let message: serde_json::Value = serde_json::from_str(frame.to_text().unwrap()).unwrap();
                    if message["type"] == "title_prompt" {
                        assert_eq!(message["requestId"], command["id"]);
                        assert_eq!(message["defaultPrompt"], DEFAULT_TITLE_PROMPT);
                        prompt = Some(message["prompt"].as_str().unwrap().to_owned());
                    }
                    if message["type"] == "response" && message["requestId"] == command["id"] {
                        assert_eq!(message["ok"], expected.is_some());
                        assert_eq!(prompt.as_deref(), expected);
                        break;
                    }
                }
            }).await.unwrap();
        }
        fs::remove_dir(&config.state_path).await.unwrap();
        fs::rename(root.join("backup.json"), &config.state_path).await.unwrap();
        assert_eq!(manager.inner.state.title_prompt(None).await.unwrap(), custom);
        assert_eq!(manager.inner.state.get(&existing).unwrap().title, "Existing title");
        assert!(!root.join("pi-sessions/spawn-args").exists());
        assert!(!root.join("model-call.json").exists());
        first.close(None).await.unwrap();
        second.close(None).await.unwrap();
        server.abort();
        manager.shutdown().await;

        let restored = StateStore::load(config.state_path.clone()).await.unwrap();
        assert_eq!(restored.title_prompt(None).await.unwrap(), custom);
        let manager = AgentManager::new(config, restored);
        let id = manager.create_session().await.unwrap();
        let text = format!("First {{text}} 🔧 {}", "z".repeat(900));
        manager.prompt(&id, &text, "title-test").await.unwrap();
        let call: serde_json::Value = serde_json::from_slice(&fs::read(root.join("model-call.json")).await.unwrap()).unwrap();
        assert_eq!(call["prompt"], custom.replace("{text}", &text.chars().take(600).collect::<String>()));
        assert_eq!(manager.inner.state.get(&id).unwrap().title, "Literal title");
        manager.shutdown().await;
        for (template, cli, expected) in [
            (None, None, DEFAULT_TITLE_PROMPT),
            (None, Some("CLI {text}"), "CLI {text}"),
            (Some(""), Some("CLI {text}"), ""),
            (Some(custom), Some("CLI {text}"), custom),
        ] {
            let mut command = tokio::process::Command::new("python3");
            command.arg(root.join("title_gen.py")).arg("--model").arg(root.join("model-call.json"));
            if let Some(cli) = cli { command.arg("--prompt-template").arg(cli); }
            let mut child = command.stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).spawn().unwrap();
            let mut input = json!({"text":"Message {text}"});
            if let Some(template) = template { input["promptTemplate"] = json!(template); }
            child.stdin.take().unwrap().write_all(input.to_string().as_bytes()).await.unwrap();
            let output = child.wait_with_output().await.unwrap();
            assert!(output.status.success());
            let call: serde_json::Value = serde_json::from_slice(&fs::read(root.join("model-call.json")).await.unwrap()).unwrap();
            assert_eq!(call["prompt"], expected.replace("{text}", "Message {text}"));
        }
        fs::remove_dir_all(root).await.unwrap();
    }

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
        title_command: None,
        };
        let manager = AgentManager::new(config.clone(), StateStore::load(config.state_path.clone()).await.unwrap());
        let id = manager.create_session().await.unwrap();
        let other = manager.create_session().await.unwrap();
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
        healthy.send(ClientMessage::Text(json!({"id":"open-other", "type":"open_session", "sessionId":other}).to_string().into())).await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let message = healthy.next().await.unwrap().unwrap();
                let message: serde_json::Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
                if message["requestId"] == "open-other" { assert_eq!(message["ok"], true); break; }
            }
        }).await.unwrap();
        manager.close_session(&id).await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let message = healthy.next().await.unwrap().unwrap();
                let message: serde_json::Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
                if message["type"] == "transcript_update" { assert_eq!(message["sessionId"], id); break; }
            }
        }).await.expect("opening another chat must retain the first feed");
        assert!(!root.join("pi-sessions/spawn-args").exists(), "reading chats must not start Pi");
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
