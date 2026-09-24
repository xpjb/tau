//! Authenticated HTTP/WebSocket + direct Rust Iroh transfers. Requests carry a
//! connection epoch: commands queued for a dead socket can never run on its successor.
use crate::{
    connection::{HEARTBEAT_INTERVAL, HEARTBEAT_TIMEOUT},
    store::{LocalFile, Settings},
};
use anyhow::{Context, Result, bail, ensure};
use futures_util::{SinkExt, StreamExt};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use tau_protocol::*;
use tokio::sync::mpsc;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};

pub type Wake = Arc<dyn Fn() + Send + Sync>;
pub enum Command {
    Request {
        epoch: u64,
        request: ClientRequest,
    },
    Upload {
        epoch: u64,
        id: String,
        session: String,
        text: String,
        files: Vec<LocalFile>,
    },
    Download {
        key: String,
        session: String,
        entry: String,
        target: PathBuf,
        limit: u64,
    },
    CancelDownload(String),
}
pub enum Event {
    Ready(u64),
    HeartbeatSent { epoch: u64, at: Instant },
    HeartbeatReply { epoch: u64, rtt: Duration },
    Message(u64, Box<ServerMessage>),
    Disconnected(String),
    Fatal(String),
    NotSent(String, String),
    Prepared {
        epoch: u64,
        id: String,
        result: Result<String, String>,
    },
    Download {
        key: String,
        status: tau_transfer::TransferStatus,
        path: PathBuf,
    },
}
#[derive(Clone)]
struct Events {
    tx: mpsc::Sender<Event>,
    wake: Wake,
}
impl Events {
    async fn send(&self, event: Event) -> bool {
        if self.tx.send(event).await.is_err() {
            return false;
        }
        (self.wake)();
        true
    }
}
pub struct Network {
    tx: mpsc::Sender<Command>,
    pub events: mpsc::Receiver<Event>,
}
impl Network {
    pub fn start(settings: Settings, wake: Wake) -> Self {
        let (tx, rx) = mpsc::channel(64);
        let (events, incoming) = mpsc::channel(256);
        std::thread::Builder::new()
            .name("tau-network".into())
            .spawn(move || {
                let sink = Events { tx: events, wake };
                match tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt.block_on(run(settings, rx, sink)),
                    Err(_) => {
                        let _ = sink
                            .tx
                            .blocking_send(Event::Fatal("Cannot start network runtime".into()));
                        (sink.wake)();
                    }
                }
            })
            .expect("start network thread");
        Self {
            tx,
            events: incoming,
        }
    }
    pub fn send(&self, command: Command) -> Result<()> {
        self.tx
            .try_send(command)
            .map_err(|_| anyhow::anyhow!("Connection is busy or closed"))
    }
}

pub fn endpoint(settings: &Settings, parts: &[&str]) -> Result<url::Url> {
    let mut url = settings.url()?;
    url.path_segments_mut()
        .map_err(|_| anyhow::anyhow!("Invalid server URL"))?
        .pop_if_empty()
        .extend(parts.iter().copied());
    Ok(url)
}
fn http() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(90))
        .build()?)
}
async fn bounded_body(response: reqwest::Response, limit: usize) -> Result<Vec<u8>> {
    let response = response.error_for_status()?;
    ensure!(
        response.content_length().is_none_or(|n| n <= limit as u64),
        "Response is too large"
    );
    let mut result = Vec::new();
    let mut chunks = response.bytes_stream();
    while let Some(chunk) = chunks.next().await {
        let chunk = chunk?;
        ensure!(
            result.len().saturating_add(chunk.len()) <= limit,
            "Response is too large"
        );
        result.extend_from_slice(&chunk);
    }
    Ok(result)
}
async fn upload(
    client: reqwest::Client,
    settings: Settings,
    session: String,
    text: String,
    files: Vec<LocalFile>,
) -> Result<String> {
    let mut text = if text.trim().is_empty() {
        "Please inspect the attached files.".into()
    } else {
        text
    };
    if !files.is_empty() {
        text.push_str("\n\nAttached files are available at:\n");
    }
    for file in files {
        let mut url = endpoint(&settings, &["v1", "sessions", &session, "uploads"])?;
        url.query_pairs_mut().append_pair("fileName", &file.name);
        let meta = tokio::fs::metadata(&file.path).await?;
        ensure!(
            meta.is_file() && meta.len() == file.size && file.size <= MAX_UPLOAD_BYTES as u64,
            "Attachment changed or exceeds 50 MB"
        );
        let bytes = tokio::fs::read(&file.path).await?;
        ensure!(bytes.len() as u64 == file.size, "Attachment changed");
        let response = client
            .post(url)
            .bearer_auth(&settings.token)
            .header("Content-Type", "application/octet-stream")
            .body(bytes)
            .send()
            .await?;
        let uploaded: UploadedFile = serde_json::from_slice(&bounded_body(response, 16384).await?)?;
        text.push_str(&format!("- {}: {}\n", uploaded.name, uploaded.path));
    }
    ensure!(
        text.chars().count() <= MAX_PROMPT_CHARS,
        "Prompt and attachment paths are too long"
    );
    Ok(text)
}

#[derive(Debug)]
struct HeartbeatTimeout;
impl std::fmt::Display for HeartbeatTimeout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WebSocket ping timed out")
    }
}
impl std::error::Error for HeartbeatTimeout {}

async fn run(settings: Settings, mut commands: mpsc::Receiver<Command>, events: Events) {
    let setup = (|| -> Result<_> {
        let mut url = endpoint(&settings, &["v1", "ws"])?;
        let scheme = if url.scheme() == "https" { "wss" } else { "ws" };
        url.set_scheme(scheme)
            .map_err(|_| anyhow::anyhow!("Invalid WebSocket URL"))?;
        let mut request = url.as_str().into_client_request()?;
        request.headers_mut().insert(
            "Authorization",
            format!("Bearer {}", settings.token).parse()?,
        );
        Ok((request, http()?))
    })();
    let (request, client) = match setup {
        Ok(setup) => setup,
        Err(_) => {
            events
                .send(Event::Fatal("Invalid connection settings".into()))
                .await;
            return;
        }
    };
    let mut epoch = 0u64;
    let mut delay = 1;
    let mut jobs = tokio::task::JoinSet::new();
    let mut downloads = HashMap::<String, Arc<tau_transfer::TransferDownload>>::new();
    loop {
        let connection =
            tokio::time::timeout(Duration::from_secs(20), connect_async(request.clone()));
        tokio::pin!(connection);
        let socket = loop {
            tokio::select! {
                result = &mut connection => break result,
                command = commands.recv() => match command {
                    None => return,
                    Some(Command::CancelDownload(key)) => { if let Some(d) = downloads.remove(&key) { d.cancel(); } }
                    Some(command) => { reject_offline(command, &events).await; }
                },
                _ = jobs.join_next(), if !jobs.is_empty() => {},
            }
        };
        let outcome: Result<()> = async {
            let (mut socket, _) = socket.context("Connection timed out")?.context("Cannot reach Tau")?;
            let hello = tokio::time::timeout(Duration::from_secs(15), socket.next()).await?.context("No hello")??;
            let Message::Text(hello) = hello else { bail!("Expected Tau hello"); };
            let ServerMessage::Hello { protocol_version, .. } = serde_json::from_str(&hello)? else { bail!("Expected Tau hello"); };
            if protocol_version != PROTOCOL_VERSION {
                events.send(Event::Fatal(format!("Protocol {protocol_version} requires a matching client (this client uses {PROTOCOL_VERSION})"))).await;
                commands.close();
                return Ok(());
            }
            epoch += 1;
            delay = 1;
            if !events.send(Event::Ready(epoch)).await { return Ok(()); }
            let mut heartbeat = tokio::time::interval_at(
                tokio::time::Instant::now() + HEARTBEAT_INTERVAL, HEARTBEAT_INTERVAL);
            heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let mut waiting: Option<(Vec<u8>, Instant)> = None;
            loop {
                let deadline = waiting.as_ref().map(|(_, at)|
                    tokio::time::Instant::from_std(*at + HEARTBEAT_TIMEOUT));
                tokio::select! {
                    _ = async {
                        if let Some(deadline) = deadline { tokio::time::sleep_until(deadline).await; }
                        else { std::future::pending::<()>().await; }
                    } => return Err(HeartbeatTimeout.into()),
                    command = commands.recv() => match command {
                        None => return Ok(()),
                        Some(Command::Request { epoch: requested, request }) => {
                            if requested != epoch { events.send(Event::NotSent(request.id, "Connection changed; not sent".into())).await; continue; }
                            let encoded = serde_json::to_string(&request)?;
                            if encoded.len() > MAX_REQUEST_BYTES { events.send(Event::NotSent(request.id, "Request is too large".into())).await; continue; }
                            tokio::time::timeout(Duration::from_secs(15), socket.send(Message::Text(encoded.into()))).await??;
                        }
                        Some(Command::Upload { epoch: requested, id, session, text, files }) => {
                            if requested != epoch { events.send(Event::NotSent(id, "Connection changed before upload".into())).await; continue; }
                            if jobs.len() >= 4 { events.send(Event::NotSent(id, "Too many transfers; try again".into())).await; continue; }
                            let (settings, client, events) = (settings.clone(), client.clone(), events.clone());
                            jobs.spawn(async move {
                                let result = upload(client, settings, session, text, files).await.map_err(|_| "Attachment upload failed; prompt was not sent".into());
                                events.send(Event::Prepared { epoch: requested, id, result }).await;
                            });
                        }
                        Some(Command::CancelDownload(key)) => { if let Some(d) = downloads.remove(&key) { d.cancel(); } }
                        Some(Command::Download { key, session, entry, target, limit }) => {
                            downloads.retain(|_, d| !d.status().done);
                            if downloads.contains_key(&key) { continue; }
                            let d = Arc::new(tau_transfer::TransferDownload::new());
                            let capacity = downloads.len() < 2;
                            if capacity { downloads.insert(key.clone(), d.clone()); }
                            let (settings, client, events) = (settings.clone(), client.clone(), events.clone());
                            jobs.spawn(async move {
                                let result: Result<()> = async {
                                    ensure!(capacity, "Only two downloads can run at once");
                                    tokio::fs::create_dir_all(target.parent().context("Invalid download path")?).await?;
                                    let mut url = endpoint(&settings, &["v1", "sessions", &session, "attachments", &entry])?;
                                    url.query_pairs_mut().append_pair("transferNode", &d.node_id());
                                    let response = client.get(url).bearer_auth(&settings.token).send().await?;
                                    let offer = String::from_utf8(bounded_body(response, 4096).await?)?;
                                    let host = settings.url()?.host_str().context("Missing host")?.to_owned();
                                    d.start(offer, host, target.to_string_lossy().into_owned(), limit)?;
                                    loop {
                                        let status = d.status(); let done = status.done;
                                        if !events.send(Event::Download { key: key.clone(), status, path: target.clone() }).await { d.cancel(); return Ok(()); }
                                        if done { return Ok(()); }
                                        tokio::time::sleep(Duration::from_millis(100)).await;
                                    }
                                }.await;
                                if let Err(error) = result {
                                    let status = tau_transfer::TransferStatus { transferred: 0, total: 0, network_bytes: 0, done: true, failure: Some(format!("Download failed: {}", error.without_url())) };
                                    events.send(Event::Download { key, status, path: target }).await;
                                }
                            });
                        }
                    },
                    frame = socket.next() => {
                        match frame.context("Connection closed")?? {
                            Message::Text(text) => {
                                ensure!(text.len() <= 16 * 1024 * 1024, "Tau frame is too large");
                                let message: ServerMessage = serde_json::from_str(&text).context("Invalid Tau message")?;
                                if !events.send(Event::Message(epoch, Box::new(message))).await { return Ok(()); }
                            }
                            Message::Ping(payload) => { tokio::time::timeout(Duration::from_secs(15), socket.send(Message::Pong(payload))).await??; }
                            Message::Pong(payload) => {
                                if waiting.as_ref().is_some_and(|(bytes, _)| bytes.as_slice() == payload.as_ref()) {
                                    let (_, sent) = waiting.take().unwrap();
                                    if !events.send(Event::HeartbeatReply { epoch, rtt: sent.elapsed() }).await { return Ok(()); }
                                }
                            }
                            Message::Close(_) => bail!("Connection closed"),
                            _ => {},
                        }
                    },
                    _ = heartbeat.tick() => {
                        if waiting.is_some() { continue; } // Timeout is independent of the probe cadence.
                        let payload = uuid::Uuid::new_v4().as_bytes().to_vec();
                        tokio::time::timeout(Duration::from_secs(15), socket.send(Message::Ping(payload.clone().into()))).await??;
                        let sent = Instant::now();
                        waiting = Some((payload, sent));
                        if !events.send(Event::HeartbeatSent { epoch, at: sent }).await { return Ok(()); }
                    },
                    _ = jobs.join_next(), if !jobs.is_empty() => {},
                }
            }
        }.await;
        if outcome.is_ok() {
            break;
        }
        // Classify without printing headers, request bodies, or token-bearing URLs.
        let error = outcome.unwrap_err();
        let detail = if error.is::<HeartbeatTimeout>() {
            "Ping timed out. Reconnecting…"
        } else if let Some(error) = error.downcast_ref::<tokio_tungstenite::tungstenite::Error>() {
            use tokio_tungstenite::tungstenite::Error;
            match error {
                Error::Http(response) => match response.status().as_u16() {
                    401 | 403 => "Access denied. Check the access token in Settings.",
                    404 => "Tau's WebSocket endpoint was not found. Check the daemon URL.",
                    _ => {
                        "The server refused the connection. Check the daemon URL and server status."
                    }
                },
                Error::Tls(_) => {
                    "TLS connection failed. Check the server certificate and HTTPS URL."
                }
                Error::Io(_) => {
                    "Cannot reach the daemon. Check the URL, port, and Tailscale connection. Retrying…"
                }
                _ => {
                    "Connection to the daemon was lost or it returned an invalid response. Retrying…"
                }
            }
        } else if error
            .downcast_ref::<tokio::time::error::Elapsed>()
            .is_some()
        {
            "Connection timed out. Check the daemon URL and Tailscale connection. Retrying…"
        } else {
            "The server did not return a valid Tau response. Check the URL and server version. Retrying…"
        };
        if !events.send(Event::Disconnected(detail.into())).await {
            break;
        }
        let wait = tokio::time::sleep(Duration::from_secs(delay));
        tokio::pin!(wait);
        delay = (delay * 2).min(15);
        loop {
            tokio::select! {
                _ = &mut wait => break,
                command = commands.recv() => match command {
                    None => return,
                    Some(Command::CancelDownload(key)) => { if let Some(d) = downloads.remove(&key) { d.cancel(); } }
                    Some(command) => { reject_offline(command, &events).await; }
                },
                _ = jobs.join_next(), if !jobs.is_empty() => {},
            }
        }
    }
    for d in downloads.values() {
        d.cancel();
    }
}
async fn reject_offline(command: Command, events: &Events) {
    match command {
        Command::Request { request, .. } => {
            events
                .send(Event::NotSent(
                    request.id,
                    "Not connected; request was not sent".into(),
                ))
                .await;
        }
        Command::Upload { id, .. } => {
            events
                .send(Event::NotSent(
                    id,
                    "Not connected; prompt was not sent".into(),
                ))
                .await;
        }
        Command::Download { key, target, .. } => {
            events
                .send(Event::Download {
                    key,
                    path: target,
                    status: tau_transfer::TransferStatus {
                        transferred: 0,
                        total: 0,
                        network_bytes: 0,
                        done: true,
                        failure: Some("Connect to download this file".into()),
                    },
                })
                .await;
        }
        Command::CancelDownload(_) => {}
    }
}
// Request errors may embed URLs; only static, non-sensitive details leave this layer.
trait SafeError {
    fn without_url(&self) -> &str;
}
impl SafeError for anyhow::Error {
    fn without_url(&self) -> &str {
        "check the connection and retry (partial data is retained)"
    }
}
