//! Authenticated control requests and the daemon native-stream backend.
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, ensure};
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
use tau_net::*;
use tau_net::blocks::*;
use crate::state::{StateStore, UncertainOutcome, StoredSession};
use rusqlite::params;
use futures_util::FutureExt;

const WS_PING_INTERVAL: Duration = Duration::from_secs(30);

fn admission_timing(start: Option<std::time::Instant>, outcome: &'static str) {
    if let Some(start)=start {
        tracing::debug!(target:"taud::control_admission",outcome,
            wait_us=start.elapsed().as_micros() as u64,"control admission timing");
    }
}

#[derive(Clone)]
struct AppState {
    config: Config,
    manager: AgentManager,
    telemetry_gate: Arc<Mutex<()>>,
    transfers: Arc<tau_net::native::Server>,
    requests: Arc<tokio::sync::Semaphore>,
    admissions: Arc<tokio::sync::Semaphore>,
}

pub async fn serve(config: Config, manager: AgentManager, listener: tokio::net::TcpListener) -> Result<()> {
    let transfers = Arc::new(tau_net::native::Server::bind_with_ipv6(config.transfer_bind, config.transfer_bind_v6, Arc::new(manager.clone())).await?);
    let state = AppState {
        config: config.clone(),
        manager: manager.clone(),
        telemetry_gate: Arc::new(Mutex::new(())),
        transfers: transfers.clone(),
        requests: Arc::new(tokio::sync::Semaphore::new(32)),
        admissions: Arc::new(tokio::sync::Semaphore::new(128)),
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
    // Bound waiting + running requests separately from execution concurrency.
    // A normal reconnect/UI burst waits here rather than failing at request #9.
    let socket_admissions = Arc::new(tokio::sync::Semaphore::new(32));
    let socket_closed = tokio_util::sync::CancellationToken::new();
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
                let admission_started=tracing::enabled!(target:"taud::control_admission",tracing::Level::DEBUG).then(std::time::Instant::now);
                let admission = state.admissions.clone().try_acquire_owned().and_then(|global|
                    socket_admissions.clone().try_acquire_owned().map(|local|(global,local)));
                let Ok(admission) = admission else {
                    admission_timing(admission_started,"overflow");
                    // Real overload remains a failure, but does not block pings
                    // behind a full response queue or create unbounded tasks.
                    let response = ServerMessage::failure(request.id, "Too many requests are waiting. Try again shortly.");
                    let Ok(encoded) = serde_json::to_string(&response) else { break; };
                    if outbound_tx.tx.try_send(Message::Text(encoded.into())).is_err() { break; }
                    continue;
                };
                let global = state.requests.clone();
                let local = socket_requests.clone();
                let disconnected = socket_closed.clone();
                tokio::spawn(async move {
                    let _admission = admission;
                    // Waiting never stalls the socket reader. A dead socket
                    // cancels only requests that have not started; admitted
                    // effects still finish and record their durable receipts.
                    let _permits = tokio::select! { biased;
                        _ = disconnected.cancelled() => {admission_timing(admission_started,"cancelled");return;},
                        _ = tokio::time::sleep(Duration::from_secs(2)) => {
                            admission_timing(admission_started,"timeout");
                            queue_server(&response_outbound, &ServerMessage::failure(request.id,
                                "The server could not start this request in time. Please try again.")).await;
                            return;
                        }
                        permits = async {
                            let local = local.acquire_owned().await?;
                            let global = global.acquire_owned().await?;
                            Ok::<_, tokio::sync::AcquireError>((local,global))
                        } => match permits { Ok(permits) => permits, Err(_) => return },
                    };
                    admission_timing(admission_started,"started");
                    let request_id = request.id.clone();
                    let request = match manager.inner.state.resolve_input(request).await {
                        Ok(request) => request,
                        Err(error) => {queue_server(&response_outbound,&command_failure(request_id,error)).await;return;}
                    };
                    // Creation is also callable from a pipelined prompt. Its manager
                    // boundary journals it under the same gate as standalone creation.
                    let journalled=request.command.journalled_control() && !matches!(request.command, ClientCommand::CreateSession { .. });
                    if journalled {
                        match manager.inner.state.reserve_operation(&request).await {
                            Ok(Some(response))=>{queue_server(&response_outbound,&response).await;return;}
                            Err(error)=>{queue_server(&response_outbound,&command_failure(request_id,error)).await;return;}
                            Ok(None)=>{queue_server(&response_outbound,&ServerMessage::Accepted {request_id:request_id.clone()}).await;}
                        }
                    }
                    let mut response = match dispatch(&manager,&transfers,request,&response_outbound).await {
                        Ok(Some(response)) => response,
                        Ok(None) => return,
                        Err(error) => command_failure(request_id.clone(),error),
                    };
                    if journalled {
                        // Some operations include file cleanup or cancellation
                        // after a DB effect. An error is not proof of no effect.
                        if let ServerMessage::Response {ok:false,uncertain,..}=&mut response {*uncertain=true;}
                        if let Err(error)=manager.inner.state.finish_operation(request_id,&response).await {
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

    socket_closed.cancel();
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
    let valid_frame = |frame: &tau_net::CrashFrame| {
        frame.class_name.chars().count() <= 192
            && frame.method_name.chars().count() <= 192
            && frame.file_name.as_ref().is_none_or(|name| name.chars().count() <= 192)
    };
    let valid_range = |range: &Option<tau_net::CrashRange>| {
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
#[path = "../../tests/unit/server.rs"]
mod tests;

impl StateStore {
    pub(crate) async fn resolve_input(&self, request: ClientRequest) -> Result<ClientRequest> {
        let ClientCommand::Input {content} = request.command else {return Ok(request);};
        let bytes = self.read(move |db|tau_block_store::uploaded_input(db,&content)).await?;
        let decoded: ClientRequest = serde_json::from_slice(&bytes)?;
        ensure!(decoded.id == request.id,"Input request ID does not match its control descriptor");
        ensure!(!matches!(decoded.command,ClientCommand::Input {..} | ClientCommand::ConnectBlocks {..}),"Invalid nested input");
        Ok(decoded)
    }
    pub(crate) async fn control_frame(&self, message: &ServerMessage) -> Result<String> {
        let bytes = serde_json::to_vec(message)?;
        if bytes.len() <= MAX_CONTROL_BYTES {return Ok(String::from_utf8(bytes)?);}
        let mut payload=message.clone();
        let route=match &mut payload {ServerMessage::SessionPage {catalog_id,..}|ServerMessage::ProjectPage {catalog_id,..}=>Some(std::mem::take(catalog_id)),_=>None};
        let bytes=serde_json::to_vec(&payload)?;
        ensure!(bytes.len() as u64 <= MAX_BLOCK_BYTES,"Descriptor exceeds its content limit");
        let hash = blake3::hash(&bytes).to_hex().to_string();
        let id = hash.clone();
        let length = bytes.len() as u64;
        let lineage = self.access(move |db| {
            let tx = db.transaction()?;
            let now=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs();
            tx.execute("DELETE FROM blocks WHERE scope=?1 AND position<?2",rusqlite::params![CONTROL_SCOPE,now.saturating_sub(24*3600)])?;
            tx.execute("DELETE FROM block_changes WHERE scope=?1 AND id NOT IN (SELECT id FROM blocks WHERE scope=?1)",[CONTROL_SCOPE])?;
            if tau_block_store::header(&tx,CONTROL_SCOPE,&id)?.is_none() {
                let (count,size):(u64,u64)=tx.query_row("SELECT count(*),coalesce(sum(json_extract(header,'$.length')),0) FROM blocks WHERE scope=?1",[CONTROL_SCOPE],|r|Ok((r.get(0)?,r.get(1)?)))?;
                ensure!(count<4096 && size.saturating_add(length)<=256*1024*1024,"Descriptor storage quota is full");
                let h = BlockHeader {id:id.clone(),parent:None,order:now,kind:BlockKind::State,meta:serde_json::json!({"descriptor":true}),version:0,length:0,sealed:true,revision:0};
                tau_block_store::put(&tx,CONTROL_SCOPE,h,&bytes)?;
            }
            // Renew the immutable descriptor's lease without changing bytes.
            tx.execute("UPDATE blocks SET position=?3 WHERE scope=?1 AND id=?2",rusqlite::params![CONTROL_SCOPE,id,now])?;
            let lineage = tau_block_store::cursor(&tx)?.lineage;
            tx.commit()?; Ok(lineage)
        }).await?;
        let short = |s: &Option<String>| s.as_ref().map(|s| {
            let mut n = s.len().min(32);while !s.is_char_boundary(n) {n-=1;}
            if n < s.len() {format!("{}… (full details downloading)",&s[..n])} else {s.clone()}
        });
        let (session_id,reports) = match message {
            ServerMessage::Response {request_id,ok,session_id,error,notice,uncertain,..} if !uncertain => (session_id.clone(),vec![OperationReceipt {id:request_id.clone(),accepted:*ok,complete:true,error:short(error),notice:short(notice)}]),
            ServerMessage::Receipts {session_id,reports} => (Some(session_id.clone()),reports.iter().map(|r|OperationReceipt {id:r.id.clone(),accepted:r.accepted,complete:r.complete,error:short(&r.error),notice:short(&r.notice)}).collect()),
            _ => (None,vec![]),
        };
        let operation_id=if let ServerMessage::Operation {operation_id,registered:true,..}=message {Some(operation_id.clone())} else {None};
        let descriptor = ServerMessage::Data {operation_id,route,key:message.replication_key().unwrap_or_else(||format!("receipt:{hash}")),content:ContentRef {lineage,scope:CONTROL_SCOPE.into(),id:hash.clone(),length,hash},session_id,reports};
        let encoded = serde_json::to_string(&descriptor)?;
        ensure!(encoded.len() <= MAX_CONTROL_BYTES,"Receipt summary exceeds the control limit");
        Ok(encoded)
    }
}

impl AgentManager {
    pub(crate) async fn list_page(&self,catalog_id:String,projects:bool,after:Option<String>,revision:u64)->Result<ServerMessage> {
        let (revision,after,rows)=self.inner.state.read(move |db| {
            let current:u64=db.query_row("SELECT revision FROM catalogue_clock",[],|r|r.get(0))?;
            let after=if current==revision {after} else {None};
            let rows=if projects {
                db.prepare("SELECT id,json_object('id',id,'name',name,'prompt',prompt,'revision',revision) FROM projects WHERE ?1 IS NULL OR id>?1 ORDER BY id LIMIT 9")?
                    .query_map([&after],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?
            } else {
                // Never copy a captured project prompt (up to 256 KiB per chat)
                // into a list query, nor load history or construct runtimes.
                db.prepare("SELECT id,json_remove(data,'$.project_prompt') FROM sessions WHERE ?1 IS NULL OR id>?1 ORDER BY id LIMIT 65")?
                    .query_map(params![after],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?
            };
            Ok((current,after,rows))
        }).await?;
        let limit=if projects {8} else {64};
        let next=(rows.len()>limit).then(||rows[limit-1].0.clone());
        if projects {
            let projects=rows.into_iter().take(limit).map(|(_,data)|serde_json::from_str::<Project>(&data)).collect::<serde_json::Result<_>>()?;
            Ok(ServerMessage::ProjectPage {catalog_id,revision,after,next,projects})
        } else {
            let settings=self.inner.settings.get();let runtimes=self.inner.runtimes.lock().await;
            let cold_revision=self.inner.state_clock.load(std::sync::atomic::Ordering::Acquire);
            let mut providers=std::collections::HashSet::new();
            let mut sessions=Vec::new();let mut states=std::collections::BTreeMap::new();
            for (id,data) in rows.into_iter().take(limit) {
                let stored:StoredSession=serde_json::from_str(&data)?;
                if providers.contains(&stored.model.provider) || providers.len()<8 {
                    providers.insert(stored.model.provider.clone()); self.schedule_catalog(&stored.model);
                }
                let state=runtimes.get(&id).map(|runtime|runtime.snapshot());
                states.insert(id.clone(),state.as_ref().map_or(cold_revision,|s|s.revision));
                let tokens=state.as_ref().filter(|s|s.status!=SessionStatus::Sleeping).and_then(|s|s.context_usage).and_then(|u|u.tokens).or(stored.tokens);
                sessions.push(SessionSummary {id,title:stored.title,project_id:stored.project_id,starter:stored.starter,parent_id:stored.parent_id,model:Some(stored.model.clone()),thinking_level:Some(stored.thinking),
                    status:state.as_ref().map(|s|s.status).unwrap_or(SessionStatus::Sleeping),detail:state.as_ref().and_then(|s|s.detail.clone()),
                    context_usage:self.context_usage(&settings,&stored.model,tokens),created_at_ms:stored.created_at_ms,updated_at_ms:stored.updated_at_ms});
            }
            Ok(ServerMessage::SessionPage {catalog_id,revision,after,next,sessions,states})
        }
    }
}

impl tau_net::native::Backend for AgentManager {
    fn feed(&self, request: tau_net::blocks::FeedRequest) -> futures_util::future::BoxFuture<'static,Result<tau_net::blocks::FeedPage>> {
        let state = self.inner.state.clone();
        async move {
            state.read(move |db| {
                ensure!(db.query_row("SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1)",[&request.scope],|r|r.get::<_,bool>(0))?,"Chat no longer exists");
                tau_block_store::feed(db,&request)
            }).await
        }.boxed()
    }
    fn read(&self, request: tau_net::blocks::BlockRequest) -> futures_util::future::BoxFuture<'static,Result<tau_net::blocks::ContentRange>> {
        let manager = self.clone();
        async move {
            let req=request.clone();
            let ready=manager.inner.state.read(move |db| {
                ensure!(req.scope == tau_net::blocks::CONTROL_SCOPE || db.query_row("SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1)",[&req.scope],|r|r.get::<_,bool>(0))?,"Chat no longer exists");
                let h=tau_block_store::header(db,&req.scope,&req.id)?.context("Unknown block")?;
                if h.meta.get("materialized")==Some(&json!(false)) {Ok(None)} else {tau_block_store::read(db,&req).map(Some)}
            }).await?;
            if let Some(range)=ready {return Ok(range);}
            manager.materialize_file(&request.scope,&request.id).await?;
            manager.inner.state.read(move |db| tau_block_store::read(db,&request)).await
        }.boxed()
    }
    fn files(&self, request: tau_net::files::FileRequest) -> futures_util::future::BoxFuture<'static, Result<tau_net::files::FileReply>> {
        let manager = self.clone();
        async move {
            let session = request.session_id.clone();
            manager.inner.state.read(move |db| {
                ensure!(db.query_row("SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1)", [&session], |r| r.get::<_, bool>(0))?, "Chat no longer exists");
                Ok(())
            }).await?;
            // This is the single cwd resolution point. Today's daemon uses one
            // cwd; future per-chat roots do not alter transport or UI lifetimes.
            manager.inner.files.request(manager.inner.config.cwd.clone(), request).await
        }.boxed()
    }
    fn changes(&self) -> tokio::sync::watch::Receiver<u64> { self.inner.state.block_changes.subscribe() }
    fn upload_begin(&self, spec: tau_net::blocks::UploadSpec) -> futures_util::future::BoxFuture<'static,Result<tau_net::blocks::UploadStatus>> {
        let manager = self.clone(); async move {manager.begin_upload(spec).await}.boxed()
    }
    fn upload_write(&self, spec: tau_net::blocks::UploadSpec, offset: u64, bytes: Vec<u8>) -> futures_util::future::BoxFuture<'static,Result<()>> {
        let manager = self.clone(); async move {manager.write_upload(spec,offset,bytes).await}.boxed()
    }
    fn upload_finish(&self, spec: tau_net::blocks::UploadSpec) -> futures_util::future::BoxFuture<'static,Result<tau_net::blocks::UploadStatus>> {
        let manager = self.clone(); async move {manager.finish_upload(spec).await}.boxed()
    }
}

fn command_failure(request_id: String, error: anyhow::Error) -> ServerMessage {
    let uncertain = error.is::<UncertainOutcome>();
    let mut response = ServerMessage::failure(request_id, error.to_string());
    if let ServerMessage::Response { uncertain: field, .. } = &mut response { *field = uncertain; }
    response
}

/// Execute an admitted request. Error conversion and journal completion happen
/// once at the socket boundary, not independently in every command arm.
async fn dispatch(manager: &AgentManager, transfers: &tau_net::native::Server,
    request: ClientRequest, outbound: &Outbound) -> Result<Option<ServerMessage>> {
    let ClientRequest { id: request_id, command } = request;
    let (mut session_id, mut draft, mut notice, mut outcome) = (None, None, None, None);
    match command {
        ClientCommand::ConnectBlocks { node_id } => {
            let cursor = manager.inner.state.block_cursor().await?;
            let offer = transfers.authorize(&node_id, cursor.lineage)?;
            queue_server(outbound, &ServerMessage::BlockConnection { offer }).await;
        }
        ClientCommand::GetSession { session_id: id } => {
            queue_server(outbound, &manager.session_state_message(&id).await?).await;
            session_id = Some(id);
        }
        ClientCommand::GetReceipts { session_id: id, requests } => {
            queue_server(outbound, &manager.receipt_message(&id, &requests).await?).await;
            session_id = Some(id);
        }
        ClientCommand::GetOperation { operation_id } => {
            queue_server(outbound, &manager.inner.state.operation_outcome(&operation_id).await?).await;
        }
        ClientCommand::ReviewRestore { session_id: id } => {
            manager.inner.state.review_restore(&id).await?;
            manager.broadcast_sessions().await;
            session_id = Some(id);
        }
        ClientCommand::ListSessions => {
            for projects in [false, true] {
                let page = manager.list_page(request_id.clone(), projects, None, 0).await?;
                if !queue_server(outbound, &page).await { return Ok(None); }
            }
        }
        ClientCommand::ListPage { catalog_id, projects, after, revision } => {
            let page = manager.list_page(catalog_id, projects, after, revision).await?;
            if !queue_server(outbound, &page).await { return Ok(None); }
        }
        ClientCommand::GetModelCatalog => {
            manager.schedule_model_catalog();
            return Ok(Some(ServerMessage::ModelCatalog { catalog: manager.model_catalog() }));
        }
        ClientCommand::GetCodexUsage { force } => {
            let result = manager.codex_usage(force).await;
            return Ok(Some(ServerMessage::CodexUsage { request_id, report: result.report, error: result.error }));
        }
        ClientCommand::RefreshModelCatalog { provider } => {
            notice = Some(manager.refresh_model_catalog(&provider).await?);
        }
        command @ (ClientCommand::GetSettings | ClientCommand::SetSettings { .. }) => {
            let settings = match command {
                ClientCommand::SetSettings { revision, settings } => manager.set_settings(revision, *settings).await?,
                _ => manager.inner.settings.get(),
            };
            queue_server(outbound, &ServerMessage::Settings { request_id: request_id.clone(), settings: Box::new(settings) }).await;
        }
        ClientCommand::CreateSession { keep_session_id, project_id } => {
            session_id = Some(manager.create_session_operation(&request_id, &ChatCreation { keep_session_id, project_id }).await?);
        }
        ClientCommand::CreateProject { project_id, name, prompt } => manager.create_project(project_id, name, prompt).await?,
        ClientCommand::UpdateProject { project_id, revision, name, prompt } => manager.update_project(project_id, revision, name, prompt).await?,
        ClientCommand::DeleteProject { project_id, revision, mode } => manager.delete_project(project_id, revision, mode).await?,
        ClientCommand::MoveSession { session_id: id, project_id } => {
            manager.move_session(&id, project_id).await?;
            session_id = Some(id);
        }
        ClientCommand::Input { .. } => anyhow::bail!("Unsupported control command"),
        ClientCommand::Prompt { session_id, text, model, create } => {
            ensure!(text.chars().count() <= MAX_PROMPT_CHARS, "message is too large");
            if let Some(create) = create {
                ensure!(uuid::Uuid::parse_str(&session_id).is_ok(), "Invalid named chat");
                let id = manager.create_session_operation(&session_id, &create).await?;
                ensure!(id == session_id, "Creation resolved to a different chat");
            }
            let result = manager.prompt_with_model(&session_id, &text, &request_id, model.as_ref()).await?;
            return Ok(Some(ServerMessage::prompt_success(request_id, session_id, result.disposition, result.notice)));
        }
        ClientCommand::QueueControl { session_id: id, generation, operation } => {
            outcome = Some(manager.queue_control(&id, &generation, &request_id, operation).await?);
            session_id = Some(id);
        }
        ClientCommand::Abort { session_id: id } => {
            manager.abort(&id, &request_id).await?;
            session_id = Some(id);
        }
        ClientCommand::CloseSession { session_id: id } => {
            manager.close_session(&id).await?;
            session_id = Some(id);
        }
        ClientCommand::DeleteSession { session_id: id } => {
            manager.delete_session(&id).await?;
            session_id = Some(id);
        }
        ClientCommand::RenameSession { session_id: id, title } => {
            manager.rename_session(&id, &title).await?;
            session_id = Some(id);
        }
        ClientCommand::ForkSession { session_id: id, entry_id } => {
            let (child, text) = manager.fork_session(&id, &entry_id).await?;
            session_id = Some(child); draft = text;
        }
        ClientCommand::CloneSession { session_id: id } => session_id = Some(manager.clone_session(&id).await?),
    }
    Ok(Some(ServerMessage::Response { request_id, ok: true, session_id, draft, disposition: None,
        uncertain: false, outcome, notice, error: None }))
}
