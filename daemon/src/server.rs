use crate::protocol::ResponseError;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{DefaultBodyLimit, State, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncWriteExt;
use tokio::sync::{Mutex, mpsc};
use tracing::{info, warn};

use crate::config::Config;
use crate::manager::AgentManager;
use crate::protocol::{
    ClientCommand, ClientRequest, CrashReport, MAX_CRASH_BYTES, MAX_PROMPT_CHARS,
    MAX_CONTROL_BYTES, PROTOCOL_VERSION, ServerMessage,
};

const WS_PING_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Clone)]
struct AppState {
    config: Config,
    manager: AgentManager,
    telemetry_gate: Arc<Mutex<()>>,
    transfers: Arc<tau_transfer::blocks::Server>,
    requests: Arc<tokio::sync::Semaphore>,
}

pub async fn serve(config: Config, manager: AgentManager, listener: tokio::net::TcpListener) -> Result<()> {
    let transfers = Arc::new(tau_transfer::blocks::Server::bind(config.transfer_bind, Arc::new(manager.clone())).await?);
    let state = AppState {
        config: config.clone(),
        manager: manager.clone(),
        telemetry_gate: Arc::new(Mutex::new(())),
        transfers: transfers.clone(),
        requests: Arc::new(tokio::sync::Semaphore::new(32)),
    };
    let app = Router::new()
        .route("/v1/health", get(|| async { Json(json!({
            "name": "Tau",
            "version": env!("CARGO_PKG_VERSION"),
            "protocolVersion": PROTOCOL_VERSION
        })) }))
        .route("/v1/ws", get(websocket))
        .route(
            "/v1/telemetry/crash",
            post(crash_report).layer(DefaultBodyLimit::max(MAX_CRASH_BYTES)),
        )
        .with_state(state);
    info!(address = %config.bind, "Tau daemon is listening");

    manager.maintain_uploads().await?;
    let maintenance=manager.clone();let mut maintenance_task=tokio::task::JoinSet::new();
    maintenance_task.spawn(async move {loop {tokio::time::sleep(Duration::from_secs(600)).await;if let Err(error)=maintenance.maintain_uploads().await {warn!(%error,"Upload maintenance requires attention");}}});
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
    transfers.shutdown().await;
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
        .max_message_size(MAX_CONTROL_BYTES).max_frame_size(MAX_CONTROL_BYTES)
        .on_upgrade(move |socket| serve_socket(socket, state))
        .into_response()
}

async fn serve_socket(socket: WebSocket, state: AppState) {
    let (mut socket_tx, mut socket_rx) = socket.split();
    let (tx, mut outbound_rx) = mpsc::channel::<Message>(32);
    let (health, mut health_rx) = mpsc::channel::<Message>(8);
    let outbound_tx = Outbound {tx,state:state.manager.inner.state.clone()};
    let writer = tokio::spawn(async move {
        loop {
            let message = tokio::select! {biased;
                message = health_rx.recv() => message,
                message = outbound_rx.recv() => message,
            };
            let Some(message) = message else {break;};
            if !matches!(tokio::time::timeout(Duration::from_secs(10),socket_tx.send(message)).await,Ok(Ok(()))) {break;}
        }
    });

    let Ok(cursor)=state.manager.inner.state.block_cursor().await else {writer.abort();return;};
    queue_server(
        &outbound_tx,
        &ServerMessage::Hello {
            protocol_version: PROTOCOL_VERSION,
            daemon_version: env!("CARGO_PKG_VERSION").into(),lineage:Some(cursor.lineage),
        },
    )
    .await;
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

    let socket_requests = Arc::new(tokio::sync::Semaphore::new(8));
    let mut heartbeat = tokio::time::interval_at(tokio::time::Instant::now() + WS_PING_INTERVAL, WS_PING_INTERVAL);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut pending_ping = None;
    loop {
        let incoming = tokio::select! {
            incoming = socket_rx.next() => incoming,
            _ = heartbeat.tick() => {
                if pending_ping.is_some() { break; }
                let payload = Bytes::copy_from_slice(uuid::Uuid::new_v4().as_bytes());
                if health.try_send(Message::Ping(payload.clone())).is_err() { break; }
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
                if text.len() > MAX_CONTROL_BYTES {
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
                        if outbound_tx.tx.try_send(Message::Text(encoded.into())).is_err() { break; }
                        continue;
                    }
                };
                let manager = state.manager.clone();
                let transfers = state.transfers.clone();
                let response_outbound = outbound_tx.clone();
                let permits = state.requests.clone().try_acquire_owned().and_then(|global|socket_requests.clone().try_acquire_owned().map(|local|(global,local)));
                let Ok(permits) = permits else {
                    queue_server(&outbound_tx,&ServerMessage::failure(request.id,"Control is busy; retry the same operation ID")).await;
                    continue;
                };
                tokio::spawn(async move {
                    let _permits = permits;
                    let request_id = request.id.clone();
                    let request = match manager.inner.state.resolve_input(request).await {
                        Ok(request) => request,
                        Err(error) => {queue_server(&response_outbound,&ServerMessage::command_failure(request_id,error)).await;return;}
                    };
                    let journalled=request.command.journalled_control();
                    if journalled {
                        match manager.inner.state.reserve_operation(&request).await {
                            Ok(Some(response))=>{queue_server(&response_outbound,&response).await;return;}
                            Err(error)=>{queue_server(&response_outbound,&ServerMessage::command_failure(request_id,error)).await;return;}
                            Ok(None)=>{queue_server(&response_outbound,&ServerMessage::Accepted {request_id:request_id.clone()}).await;}
                        }
                    }
                    let operation_id=request_id.clone();
                    let mut response = match request.command {
                        ClientCommand::ConnectBlocks { node_id } => {
                            match manager.inner.state.block_cursor().await.and_then(|cursor| transfers.authorize(&node_id,cursor.lineage)) {
                                Ok(offer) => {
                                    queue_server(&response_outbound,&ServerMessage::BlockConnection { offer }).await;
                                    ServerMessage::success(request_id,None,None)
                                }
                                Err(error) => ServerMessage::command_failure(request_id,error),
                            }
                        }
                        ClientCommand::GetSession { session_id } => match manager.session_state_message(&session_id).await {
                            Ok(message) => { queue_server(&response_outbound,&message).await; ServerMessage::success(request_id,Some(session_id),None) }
                            Err(error) => ServerMessage::command_failure(request_id,error),
                        },
                        ClientCommand::GetReceipts { session_id,requests } => match manager.receipt_message(&session_id,&requests).await {
                            Ok(message) => { queue_server(&response_outbound,&message).await; ServerMessage::success(request_id,Some(session_id),None) }
                            Err(error) => ServerMessage::command_failure(request_id,error),
                        },
                        ClientCommand::GetOperation {operation_id} => match manager.inner.state.operation_outcome(&operation_id).await {
                            Ok(message)=>{queue_server(&response_outbound,&message).await;ServerMessage::success(request_id,None,None)}
                            Err(error)=>ServerMessage::command_failure(request_id,error),
                        },
                        ClientCommand::ReviewRestore {session_id}=>match manager.inner.state.review_restore(&session_id).await {
                            Ok(())=>{manager.broadcast_sessions().await;ServerMessage::success(request_id,Some(session_id),None)},
                            Err(error)=>ServerMessage::command_failure(request_id,error)
                        },
                        ClientCommand::ListSessions => {
                            let mut result=ServerMessage::success(request_id.clone(),None,None);
                            for projects in [false,true] {
                                match manager.list_page(request_id.clone(),projects,None,0).await {
                                    Ok(page)=>{if !queue_server(&response_outbound,&page).await {return;}},
                                    Err(error)=>{result=ServerMessage::command_failure(request_id.clone(),error);break;}
                                }
                            }result
                        }
                        ClientCommand::ListPage {catalog_id,projects,after,revision}=>match manager.list_page(catalog_id,projects,after,revision).await {
                            Ok(page)=>{if !queue_server(&response_outbound,&page).await {return;}ServerMessage::success(request_id,None,None)},
                            Err(error)=>ServerMessage::command_failure(request_id,error)
                        },
                        ClientCommand::RefreshModelCatalog { provider } => match manager.refresh_model_catalog(&provider).await {
                            Ok(notice) => {
                                let mut response = ServerMessage::success(request_id, None, None);
                                if let ServerMessage::Response { notice: field, .. } = &mut response { *field = Some(notice); }
                                response
                            }
                            Err(error) => ServerMessage::command_failure(request_id, error),
                        },
                        command @ (ClientCommand::GetSettings | ClientCommand::SetSettings { .. }) => {
                            let result = match command {
                                ClientCommand::SetSettings { revision, settings } => manager.set_settings(revision, *settings).await,
                                _ => Ok(manager.inner.settings.get()),
                            };
                            match result {
                                Ok(settings) => {
                                    queue_server(&response_outbound, &ServerMessage::Settings { request_id:request_id.clone(), settings:Box::new(settings) }).await;
                                    ServerMessage::success(request_id, None, None)
                                }
                                Err(error) => ServerMessage::command_failure(request_id, error),
                            }
                        }
                        ClientCommand::CreateSession { keep_session_id, project_id } => match manager.create_session_requested(
                            keep_session_id.as_deref(), &project_id,
                            uuid::Uuid::parse_str(&request_id).ok().as_ref().map(|_| request_id.as_str()),
                        ).await {
                            Ok(session_id) => ServerMessage::success(
                                request_id,
                                Some(session_id),
                                None,
                            ),
                            Err(error) => ServerMessage::command_failure(request_id, error),
                        },
                        ClientCommand::CreateProject { project_id, name, prompt } => match manager.create_project(project_id,name,prompt).await {
                            Ok(()) => ServerMessage::success(request_id,None,None),
                            Err(error) => ServerMessage::command_failure(request_id,error),
                        },
                        ClientCommand::UpdateProject { project_id, revision, name, prompt } => match manager.update_project(project_id,revision,name,prompt).await {
                            Ok(()) => ServerMessage::success(request_id,None,None),
                            Err(error) => ServerMessage::command_failure(request_id,error),
                        },
                        ClientCommand::DeleteProject { project_id, revision, mode } => match manager.delete_project(project_id,revision,mode).await {
                            Ok(()) => ServerMessage::success(request_id,None,None),
                            Err(error) => ServerMessage::command_failure(request_id,error),
                        },
                        ClientCommand::MoveSession { session_id, project_id } => match manager.move_session(&session_id,project_id).await {
                            Ok(()) => ServerMessage::success(request_id,Some(session_id),None),
                            Err(error) => ServerMessage::command_failure(request_id,error),
                        },
                        ClientCommand::Input {..} | ClientCommand::OpenSession {..} | ClientCommand::GetHistory {..} =>
                            ServerMessage::failure(request_id,"Unsupported control command"),
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
                            match manager.abort(&session_id, &request_id).await {
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
                    if journalled {
                        // Some operations include file cleanup or cancellation
                        // after a DB effect. An error is not proof of no effect.
                        if let ServerMessage::Response {ok:false,uncertain,..}=&mut response {*uncertain=true;}
                        if let Err(error)=manager.inner.state.finish_operation(operation_id,&response).await {
                            warn!(%error,"Could not persist operation outcome");
                            if let ServerMessage::Response {ok,uncertain,error,..}=&mut response {*ok=false;*uncertain=true;*error=Some("Operation outcome could not be committed; reconcile before retrying".into());}
                        }
                    }
                    queue_server(&response_outbound, &response).await;
                });
            }
            Message::Pong(bytes) => {
                if pending_ping.as_ref() == Some(&bytes) { pending_ping = None; }
            }
            Message::Close(_) => break,
            Message::Ping(_) => {}, // Tungstenite queues and flushes the matching pong.
            Message::Binary(_) => break,
        }
    }

    event_forwarder.abort();
    writer.abort();
    drop(outbound_tx);
    let _ = event_forwarder.await;
    let _ = writer.await;
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
    let valid_frame = |frame: &crate::protocol::CrashFrame| {
        frame.class_name.chars().count() <= 192
            && frame.method_name.chars().count() <= 192
            && frame.file_name.as_ref().is_none_or(|name| name.chars().count() <= 192)
    };
    let valid_range = |range: &Option<crate::protocol::CrashRange>| {
        range.as_ref().is_none_or(|range| range.text_length >= 0
            && (range.start < 0 || range.start > range.end || range.end > range.text_length))
    };
    let valid = matches!(report.schema, 1 | 2)
        && (report.schema == 2 || (report.selection_range.is_none() && report.causes.is_empty()))
        && !report.report_id.is_empty()
        && report.report_id.chars().count() <= 128
        && !report.platform.is_empty()
        && report.platform.chars().count() <= 64
        && !report.app_version.is_empty()
        && report.app_version.chars().count() <= 64
        && report.os_version.chars().count() <= 192
        && report.thread.chars().count() <= 128
        && !report.exception_class.is_empty()
        && report.exception_class.chars().count() <= 192
        && report.stack.len() <= 64
        && report.stack.iter().all(valid_frame)
        && valid_range(&report.selection_range)
        && report.causes.len() <= 3
        && report.causes.iter().all(|cause| {
            !cause.exception_class.is_empty()
                && cause.exception_class.chars().count() <= 192
                && cause.stack.len() <= 12
                && cause.stack.iter().all(valid_frame)
                && valid_range(&cause.selection_range)
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
        selection_range = ?report.selection_range,
        causes = report.causes.len(),
        "Tau client crash report received"
    );
    StatusCode::NO_CONTENT.into_response()
}

#[derive(Clone)]
struct Outbound {tx:mpsc::Sender<Message>,state:crate::state::StateStore}
async fn queue_server(outbound: &Outbound, message: &ServerMessage) -> bool {
    use std::borrow::Cow;
    let messages=if let ServerMessage::Receipts {session_id,reports}=message && reports.len()>1 {
        reports.iter().map(|report|Cow::Owned(ServerMessage::Receipts {session_id:session_id.clone(),reports:vec![report.clone()]})).collect::<Vec<_>>()
    } else {vec![Cow::Borrowed(message)]};
    for message in messages {
        let encoded = match outbound.state.control_frame(&message).await {
            Ok(encoded) => encoded,
            Err(error) => {
                warn!(%error,"Could not encode bounded control descriptor");
                let message=if let ServerMessage::Response {request_id,..}=message.as_ref() {
                    let mut failure=ServerMessage::failure(request_id.clone(),"Outcome details are unavailable; reconcile this operation before retrying");
                    if let ServerMessage::Response {uncertain,..}=&mut failure {*uncertain=true;}failure
                } else {ServerMessage::Notice {session_id:String::new(),message:"Content metadata is unavailable; refresh after checking daemon storage".into()}};
                serde_json::to_string(&message).expect("bounded control failure")
            }
        };
        if outbound.tx.send(Message::Text(encoded.into())).await.is_err() {return false;}
    }
    true
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

    use super::authorized;
    use crate::manager::safe_file_name;

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
        use crate::protocol::MAX_CRASH_BYTES;
        use crate::state::StateStore;
        use super::{AppState, crash_report};

        let root = std::env::temp_dir().join(format!("tau-crashes-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).await.unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let log = root.join("crashes.jsonl");
        let config = Config {
            transfer_bind: "127.0.0.1:0".parse().unwrap(),
            bind: address, token: Arc::from("test-token"), settings_path: root.join("settings.json"), import_pi_dir: None, codex_auth_source:None, cwd: root.clone(), database_path: root.join("tau.sqlite3"), telemetry_path: log.clone(), attachment_root: root.join("outbox"),
            upload_root: root.join("uploads"),
        };
        let manager = AgentManager::new(config.clone(), StateStore::load(config.database_path.clone()).await.unwrap()).await.unwrap();
        let app = Router::new().route("/v1/telemetry/crash", post(crash_report).layer(DefaultBodyLimit::max(MAX_CRASH_BYTES)))
            .with_state(AppState { config, manager:manager.clone(), telemetry_gate: Arc::new(Mutex::new(())),
            transfers: Arc::new(tau_transfer::blocks::Server::bind("127.0.0.1:0".parse().unwrap(),Arc::new(manager.clone())).await.unwrap()), requests:Arc::new(tokio::sync::Semaphore::new(32)) });
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
            transfer_bind: "127.0.0.1:0".parse().unwrap(),
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
            transfers: Arc::new(tau_transfer::blocks::Server::bind("127.0.0.1:0".parse().unwrap(),Arc::new(manager.clone())).await.unwrap()), requests:Arc::new(tokio::sync::Semaphore::new(32)) };
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
