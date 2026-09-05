use std::io::Cursor;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use axum::body::{Body, Bytes};
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{DefaultBodyLimit, Path as AxumPath, Query, State, WebSocketUpgrade};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::{SinkExt, StreamExt};
use image::codecs::jpeg::JpegEncoder;
use image::metadata::Orientation;
use image::{DynamicImage, ExtendedColorType, ImageDecoder, ImageReader, Limits};
use serde::Deserialize;
use serde_json::json;
use tokio::fs::{self, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{Mutex, Semaphore, mpsc};
use tokio_util::io::ReaderStream;
use tracing::{info, warn};

use crate::config::Config;
use crate::manager::AgentManager;
use crate::protocol::{
    ChatDetails, ClientCommand, ClientRequest, CrashReport, MAX_CRASH_BYTES, MAX_PROMPT_CHARS,
    MAX_REQUEST_BYTES, MAX_UPLOAD_BYTES, PROTOCOL_VERSION, ServerMessage,
};

const THUMBNAIL_MAX_SOURCE_DIMENSION: u32 = 12_000;
const THUMBNAIL_MAX_ALLOCATED_BYTES: u64 = 256 * 1024 * 1024;
const THUMBNAIL_MAX_BYTES: usize = 300_000;

#[derive(Clone)]
struct AppState {
    config: Config,
    manager: AgentManager,
    telemetry_gate: Arc<Mutex<()>>,
    thumbnail_gate: Arc<Semaphore>,
}

#[derive(Default, Deserialize)]
struct WebSocketQuery {
    features: Option<String>,
}

fn thumbnail_bytes(source: &[u8]) -> Result<Vec<u8>> {
    let mut reader = ImageReader::new(Cursor::new(source))
        .with_guessed_format()
        .context("image format was not recognized")?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(THUMBNAIL_MAX_SOURCE_DIMENSION);
    limits.max_image_height = Some(THUMBNAIL_MAX_SOURCE_DIMENSION);
    limits.max_alloc = Some(THUMBNAIL_MAX_ALLOCATED_BYTES);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().context("image decoder was unavailable")?;
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let mut image = DynamicImage::from_decoder(decoder).context("image could not be decoded")?;
    image.apply_orientation(orientation);

    let mut encoded = Vec::new();
    for (dimension, quality) in [(1_024, 76), (768, 68), (512, 60)] {
        let thumbnail = image.thumbnail(dimension, dimension).to_rgb8();
        encoded.clear();
        JpegEncoder::new_with_quality(&mut encoded, quality).encode(
            thumbnail.as_raw(),
            thumbnail.width(),
            thumbnail.height(),
            ExtendedColorType::Rgb8,
        )?;
        if encoded.len() <= THUMBNAIL_MAX_BYTES {
            return Ok(encoded);
        }
    }
    bail!("generated thumbnail exceeds the byte limit")
}

pub async fn serve(config: Config, manager: AgentManager) -> Result<()> {
    let state = AppState {
        config: config.clone(),
        manager: manager.clone(),
        telemetry_gate: Arc::new(Mutex::new(())),
        thumbnail_gate: Arc::new(Semaphore::new(1)),
    };
    let app = Router::new()
        .route("/v1/health", get(|| async { Json(json!({
            "name": "Tau",
            "version": env!("CARGO_PKG_VERSION"),
            "protocolVersion": PROTOCOL_VERSION
        })) }))
        .route("/v1/ws", get(websocket))
        .route(
            "/v1/sessions/{session_id}/messages/{entry_id}/details",
            get(message_details),
        )
        .route(
            "/v1/sessions/{session_id}/attachments/{entry_id}/thumbnail",
            get(download_attachment_thumbnail),
        )
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
    Query(query): Query<WebSocketQuery>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Response {
    if !authorized(&headers, &state.config.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let details = query.features.as_deref().is_some_and(|features| {
        features.split(',').any(|feature| feature == "details")
    });
    upgrade
        .max_message_size(MAX_REQUEST_BYTES)
        .on_upgrade(move |socket| serve_socket(socket, state, details))
        .into_response()
}

async fn serve_socket(socket: WebSocket, state: AppState, details: bool) {
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
            let mut message = match events.recv().await {
                Ok(message) => message,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    ServerMessage::ResyncRequired
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };
            if !details {
                if matches!(&message, ServerMessage::StreamDetailsDelta { .. }) {
                    continue;
                }
                if let ServerMessage::History { messages, .. } = &mut message {
                    messages.retain(|message| {
                        !message.text.is_empty() || message.attachment.is_some()
                    });
                }
            }
            if !queue_server(&event_outbound, &message).await {
                break;
            }
        }
    });

    while let Some(incoming) = socket_rx.next().await {
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
                    Ok(request) if !request.id.is_empty() && request.id.len() <= 128 => request,
                    Ok(request) => {
                        queue_server(
                            &outbound_tx,
                            &ServerMessage::failure(request.id, "invalid request id"),
                        )
                        .await;
                        continue;
                    }
                    Err(error) => {
                        queue_server(
                            &outbound_tx,
                            &ServerMessage::failure("invalid".to_owned(), format!("invalid request: {error}")),
                        )
                        .await;
                        continue;
                    }
                };
                let manager = state.manager.clone();
                let response_outbound = outbound_tx.clone();
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
                            Err(error) => ServerMessage::failure(request_id, error.to_string()),
                        },
                        ClientCommand::OpenSession { session_id } => {
                            match manager.open_session(&session_id).await {
                                Ok(()) => ServerMessage::success(
                                    request_id,
                                    Some(session_id),
                                    None,
                                ),
                                Err(error) => ServerMessage::failure(request_id, error.to_string()),
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
                                Err(error) => ServerMessage::failure(request_id, error.to_string()),
                            }
                        }
                        ClientCommand::Prompt { session_id, text } => {
                            if text.chars().count() > MAX_PROMPT_CHARS {
                                ServerMessage::failure(request_id, "message is too large")
                            } else {
                                match manager.prompt(&session_id, &text).await {
                                    Ok(outcome) => ServerMessage::prompt_success(
                                        request_id,
                                        session_id,
                                        outcome.command_handled,
                                        outcome.notice,
                                    ),
                                    Err(error) => {
                                        ServerMessage::failure(request_id, error.to_string())
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
                                Err(error) => ServerMessage::failure(request_id, error.to_string()),
                            }
                        }
                        ClientCommand::Abort { session_id } => {
                            match manager.abort(&session_id).await {
                                Ok(()) => ServerMessage::success(
                                    request_id,
                                    Some(session_id),
                                    None,
                                ),
                                Err(error) => ServerMessage::failure(request_id, error.to_string()),
                            }
                        }
                        ClientCommand::CloseSession { session_id } => {
                            match manager.close_session(&session_id).await {
                                Ok(()) => ServerMessage::success(
                                    request_id,
                                    Some(session_id),
                                    None,
                                ),
                                Err(error) => ServerMessage::failure(request_id, error.to_string()),
                            }
                        }
                        ClientCommand::DeleteSession { session_id } => {
                            match manager.delete_session(&session_id).await {
                                Ok(()) => ServerMessage::success(
                                    request_id,
                                    Some(session_id),
                                    None,
                                ),
                                Err(error) => ServerMessage::failure(request_id, error.to_string()),
                            }
                        }
                        ClientCommand::RenameSession { session_id, title } => {
                            match manager.rename_session(&session_id, &title).await {
                                Ok(()) => ServerMessage::success(
                                    request_id,
                                    Some(session_id),
                                    None,
                                ),
                                Err(error) => ServerMessage::failure(request_id, error.to_string()),
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
                            Err(error) => ServerMessage::failure(request_id, error.to_string()),
                        },
                        ClientCommand::CloneSession { session_id } => {
                            match manager.clone_session(&session_id).await {
                                Ok(child) => ServerMessage::success(
                                    request_id,
                                    Some(child),
                                    None,
                                ),
                                Err(error) => ServerMessage::failure(request_id, error.to_string()),
                            }
                        }
                    };
                    queue_server(&response_outbound, &response).await;
                });
            }
            Message::Ping(bytes) => {
                if outbound_tx.send(Message::Pong(bytes)).await.is_err() {
                    break;
                }
            }
            Message::Close(_) => break,
            Message::Binary(_) | Message::Pong(_) => {}
        }
    }

    event_forwarder.abort();
    drop(outbound_tx);
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
    if session_id.is_empty()
        || session_id.len() > 128
        || session_id
            .chars()
            .any(|character| !character.is_ascii_alphanumeric() && !matches!(character, '-' | '_'))
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

async fn message_details(
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
    match state.manager.message_details(&session_id, &entry_id).await {
        Ok(details) => (
            [(header::CACHE_CONTROL, HeaderValue::from_static("no-store"))],
            Json(ChatDetails { details }),
        )
            .into_response(),
        Err(error) => {
            warn!(session = %session_id, entry = %entry_id, %error, "Tau message details were not available");
            StatusCode::NOT_FOUND.into_response()
        }
    }
}

async fn download_attachment_thumbnail(
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
    let mut attachment = match state
        .manager
        .resolve_attachment(&session_id, &entry_id)
        .await
    {
        Ok(attachment) if attachment.mime_type.starts_with("image/") => attachment,
        Ok(_) => return StatusCode::NOT_FOUND.into_response(),
        Err(error) => {
            warn!(session = %session_id, entry = %entry_id, %error, "Tau thumbnail source was not available");
            return StatusCode::NOT_FOUND.into_response();
        }
    };
    let _permit = match state.thumbnail_gate.acquire().await {
        Ok(permit) => permit,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let mut source = Vec::with_capacity(attachment.size as usize);
    if let Err(error) = attachment.file.read_to_end(&mut source).await {
        warn!(session = %session_id, entry = %entry_id, %error, "Tau thumbnail source could not be read");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    let thumbnail = match tokio::task::spawn_blocking(move || thumbnail_bytes(&source)).await {
        Ok(Ok(thumbnail)) => thumbnail,
        Ok(Err(error)) => {
            warn!(session = %session_id, entry = %entry_id, %error, "Tau thumbnail could not be generated");
            return StatusCode::NOT_FOUND.into_response();
        }
        Err(error) => {
            warn!(session = %session_id, entry = %entry_id, %error, "Tau thumbnail task failed");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let length = thumbnail.len();
    let mut response = Response::new(Body::from(thumbnail));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("image/jpeg"),
    );
    response.headers_mut().insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&length.to_string()).expect("thumbnail length is a valid header"),
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=31536000, immutable"),
    );
    response
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
    let safe_name = attachment
        .file_name
        .chars()
        .take(160)
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    let disposition = match HeaderValue::from_str(&format!(
        "attachment; filename=\"{}\"",
        if safe_name.is_empty() { "attachment" } else { &safe_name }
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
    use std::io::Cursor;

    use axum::http::{HeaderMap, HeaderValue, header::AUTHORIZATION};
    use image::{DynamicImage, GenericImageView, ImageFormat, Rgb, RgbImage};

    use super::{THUMBNAIL_MAX_BYTES, authorized, thumbnail_bytes};

    #[test]
    fn creates_a_bounded_thumbnail() {
        let image = RgbImage::from_fn(2_048, 1_024, |x, y| {
            Rgb([
                (x.wrapping_mul(31) ^ y.wrapping_mul(17)) as u8,
                (x.wrapping_mul(13) ^ y.wrapping_mul(29)) as u8,
                (x.wrapping_mul(7) ^ y.wrapping_mul(37)) as u8,
            ])
        });
        let mut source = Cursor::new(Vec::new());
        DynamicImage::ImageRgb8(image)
            .write_to(&mut source, ImageFormat::Png)
            .unwrap();

        let thumbnail = thumbnail_bytes(source.get_ref()).unwrap();
        assert!(thumbnail.starts_with(&[0xff, 0xd8, 0xff]));
        assert!(thumbnail.len() <= THUMBNAIL_MAX_BYTES);
        let (width, height) = image::load_from_memory(&thumbnail).unwrap().dimensions();
        assert!(width <= 1_024 && height <= 1_024);
        assert_eq!(width, height * 2);
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
