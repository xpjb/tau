//! Bounded authenticated WebSocket control + one shared native data connection. Requests carry a
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
    connect_async_with_config,
    tungstenite::{Message, client::IntoClientRequest},
};

pub type Wake = Arc<dyn Fn() + Send + Sync>;
pub enum Command {
    Blocks(crate::blocks::Command),
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
    Source(u64,String),
    HeartbeatSent { epoch: u64, at: Instant },
    HeartbeatReply { epoch: u64, at: Instant, rtt: Duration },
    Message(u64, Box<ServerMessage>),
    SizedMessage(u64,Box<ServerMessage>,usize),
    Metrics(tau_transfer::blocks::Stats),
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
#[path = "transport_events.rs"]
mod event_queue;
use event_queue::Events;
pub use event_queue::EventReceiver;
pub struct Network {
    tx: mpsc::Sender<Command>,
    pub events: EventReceiver,
    pub blocks: mpsc::Receiver<crate::blocks::Notice>,
}
impl Network {
    pub fn start(settings: Settings, wake: Wake) -> Self { Self::start_inner(settings,wake,None) }
    pub fn start_cached(settings: Settings, wake: Wake, cache: crate::blocks::Cache) -> Self { Self::start_inner(settings,wake,Some(cache)) }
    fn start_inner(settings: Settings, wake: Wake, cache: Option<crate::blocks::Cache>) -> Self {
        let (block_notices,blocks) = mpsc::channel(32);
        let (tx, rx) = mpsc::channel(64);
        let (sink, incoming) = event_queue::channel(wake);
        std::thread::Builder::new()
            .name("tau-network".into())
            .spawn(move || {
                match tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt.block_on(run(settings, rx, sink, cache, block_notices)),
                    Err(_) => {
                        sink.send_now(Event::Fatal("Cannot start network runtime".into()));
                    }
                }
            })
            .expect("start network thread");
        Self {
            tx,
            events: incoming,
            blocks,
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
#[derive(Debug)]
struct HeartbeatTimeout;
impl std::fmt::Display for HeartbeatTimeout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WebSocket ping timed out")
    }
}
impl std::error::Error for HeartbeatTimeout {}

async fn run(settings: Settings, mut commands: mpsc::Receiver<Command>, events: Events, cache: Option<crate::blocks::Cache>, block_notices:mpsc::Sender<crate::blocks::Notice>) {
    let block_service = cache.map(|cache|crate::blocks::Service::start(cache,events.wake.clone(),block_notices));
    let mut block_identity = block_service.as_ref().map(|s|s.node.clone());
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
        Ok(request)
    })();
    let request = match setup {
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
    let (prepared_tx,mut prepared_rx) = mpsc::channel::<(u64,Result<ClientRequest,(String,String)>)>(8);
    let (resolved_tx,mut resolved_rx)=mpsc::channel::<(u64,String,u64,Result<ServerMessage>,tokio::sync::OwnedSemaphorePermit,usize)>(8);
    let descriptor_budget=Arc::new(tokio::sync::Semaphore::new(128*1024*1024));
    let mut generations=HashMap::<String,u64>::new();let mut generation=0u64;
    let mut downloads = HashMap::<String, tokio::sync::watch::Sender<bool>>::new();
    loop {
        let connection =
            tokio::time::timeout(Duration::from_secs(20), connect_async_with_config(request.clone(),Some(tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default().max_message_size(Some(MAX_CONTROL_BYTES)).max_frame_size(Some(MAX_CONTROL_BYTES))),false));
        tokio::pin!(connection);
        let socket = loop {
            tokio::select! {
                result = &mut connection => break result,
                command = commands.recv() => match command {
                    None => return,
                    Some(Command::Blocks(command)) => { if let Some(service) = &block_service { service.send(command); } }
                    Some(Command::CancelDownload(key)) => { if let Some(cancel) = downloads.remove(&key) { cancel.send_replace(true); } }
                    Some(Command::Download { key,session,entry,target,limit }) => {
                        start_download(&mut jobs,&mut downloads,block_service.as_ref(),key,session,entry,target,limit,&events).await;
                    }
                    Some(command) => { reject_offline(command, &events).await; }
                },
                _ = jobs.join_next(), if !jobs.is_empty() => {},
            }
        };
        let outcome: Result<()> = async {
            let (mut socket, _) = socket.context("Connection timed out")?.context("Cannot reach Tau")?;
            let hello = tokio::time::timeout(Duration::from_secs(15), socket.next()).await?.context("No hello")??;
            let Message::Text(hello) = hello else { bail!("Expected Tau hello"); };
            ensure!(hello.len() <= MAX_CONTROL_BYTES,"Tau hello exceeds the control limit");
            let ServerMessage::Hello { protocol_version, lineage, .. } = serde_json::from_str(&hello)? else { bail!("Expected Tau hello"); };
            if protocol_version != PROTOCOL_VERSION {
                events.send(Event::Fatal(format!("Protocol {protocol_version} requires a matching client (this client uses {PROTOCOL_VERSION})"))).await;
                commands.close();
                return Ok(());
            }
            let (mut writer,mut reader)=socket.split();
            let (outgoing,mut writes)=mpsc::channel::<Message>(32);
            let (health,mut probes)=mpsc::channel::<Message>(8);
            // Owned by this connection epoch; dropping the scope aborts a stalled
            // writer. No control reader/heartbeat waits for outbound socket IO.
            let mut writer_task=tokio::task::JoinSet::new();
            writer_task.spawn(async move {loop {
                let message=tokio::select! {biased;message=probes.recv()=>message,message=writes.recv()=>message};
                let Some(message)=message else {return Ok::<_,anyhow::Error>(());};
                tokio::time::timeout(Duration::from_secs(5),writer.send(message)).await??;
            }});
            if let Some(identity) = &mut block_identity && let Some(node_id) = identity.borrow_and_update().clone() {
                let request = ClientRequest { id:"block-connection".into(),command:ClientCommand::ConnectBlocks { node_id } };
                outgoing.try_send(Message::Text(serde_json::to_string(&request)?.into())).context("Control writer is full")?;
            }
            epoch += 1;
            generations.clear();
            let connected_at=Instant::now();let mut good_probes=0u32;
            if !events.send(Event::Source(epoch,lineage.context("Missing source lineage")?)).await {return Ok(());}
            if !events.send(Event::Ready(epoch)).await { return Ok(()); }
            let mut heartbeat = tokio::time::interval_at(
                tokio::time::Instant::now() + HEARTBEAT_INTERVAL, HEARTBEAT_INTERVAL);
            heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let mut waiting: Option<(Vec<u8>, Instant)> = None;
            let mut metrics=tokio::time::interval_at(tokio::time::Instant::now()+Duration::from_secs(5),Duration::from_secs(5));
            let mut renew_blocks=Instant::now()+Duration::from_secs(1800);
            loop {
                let deadline = waiting.as_ref().map(|(_, at)|
                    tokio::time::Instant::from_std(*at + HEARTBEAT_TIMEOUT));
                tokio::select! {
                    result=writer_task.join_next()=>{result.context("Control writer stopped")???;bail!("Control writer stopped");}
                    _ = async {
                        if let Some(deadline) = deadline { tokio::time::sleep_until(deadline).await; }
                        else { std::future::pending::<()>().await; }
                    } => return Err(HeartbeatTimeout.into()),
                    _=metrics.tick()=>{if let Some(service)=&block_service {if let Some(stats)=service.stats() {if !events.send(Event::Metrics(stats)).await {return Ok(());}}}}
                    Some((requested,key,serial,result,_budget,bytes)) = resolved_rx.recv() => {
                        if requested==epoch && generations.get(&key)==Some(&serial) {
                            match result {
                                Ok(message)=>{if !events.send(Event::SizedMessage(epoch,Box::new(message),bytes)).await {return Ok(());}}
                                Err(_)=>{events.send(Event::Message(epoch,Box::new(ServerMessage::ResyncRequired {session_id:None}))).await;}
                            }
                            if key.starts_with("receipt:") {generations.remove(&key);}
                        }
                    }
                    Some((requested,result)) = prepared_rx.recv() => {
                        match result {
                            Ok(request) if requested == epoch => {
                                let encoded=serde_json::to_string(&request)?;
                                ensure!(encoded.len()<=MAX_CONTROL_BYTES,"Input reference exceeds the control limit");
                                if outgoing.try_send(Message::Text(encoded.into())).is_err() {events.send(Event::NotSent(request.id,"Control writer is full; intent was not sent".into())).await;}
                            }
                            Ok(request) => {events.send(Event::NotSent(request.id,"Connection changed before input submission; command was not sent".into())).await;}
                            Err((id,error)) => {events.send(Event::NotSent(id,error)).await;}
                        }
                    }
                    command = commands.recv() => match command {
                        None => return Ok(()),
                        Some(Command::Blocks(command)) => { if let Some(service) = &block_service { service.send(command); } }
                        Some(Command::Request { epoch: requested, request }) => {
                            if requested != epoch { events.send(Event::NotSent(request.id, "Connection changed; not sent".into())).await; continue; }
                            let encoded = serde_json::to_string(&request)?;
                            if encoded.len() > MAX_CONTROL_BYTES {
                                let Some(service) = block_service.as_ref().filter(|_| jobs.len() < 8) else {
                                    events.send(Event::NotSent(request.id,"Content service is unavailable or busy".into())).await;continue;
                                };
                                let data = service.downloads(); let tx = prepared_tx.clone();
                                jobs.spawn(async move {
                                    let id = request.id.clone();
                                    let result = data.input(request).await.map_err(|e|(id,format!("Input upload failed; command was not sent: {e}")));
                                    let _ = tx.send((requested,result)).await;
                                });
                                continue;
                            }
                            if outgoing.try_send(Message::Text(encoded.into())).is_err() {events.send(Event::NotSent(request.id,"Control writer is full; intent was not sent".into())).await;}
                        }
                        Some(Command::Upload { epoch: requested, id, session, text, files }) => {
                            if requested != epoch { events.send(Event::NotSent(id, "Connection changed before upload".into())).await; continue; }
                            if jobs.len() >= 4 { events.send(Event::NotSent(id, "Too many transfers; try again".into())).await; continue; }
                            let Some(service) = &block_service else {events.send(Event::NotSent(id,"Content service unavailable".into())).await;continue;};
                            let data=service.downloads();let events=events.clone();
                            jobs.spawn(async move {
                                let result = data.upload(session, text, files).await.map_err(|e| format!("Attachment upload failed; prompt was not sent: {e}"));
                                events.send(Event::Prepared { epoch: requested, id, result }).await;
                            });
                        }
                    Some(Command::CancelDownload(key)) => { if let Some(cancel) = downloads.remove(&key) { cancel.send_replace(true); } }
                        Some(Command::Download { key, session, entry, target, limit }) => {
                            start_download(&mut jobs,&mut downloads,block_service.as_ref(),key,session,entry,target,limit,&events).await;
                        }
                    },
                    identity_changed = async {
                        if let Some(identity) = &mut block_identity { identity.changed().await.is_ok() }
                        else { std::future::pending::<bool>().await }
                    } => {
                        if !identity_changed {block_identity=None;}
                        if let Some(identity) = &mut block_identity && let Some(node_id) = identity.borrow_and_update().clone() {
                            let request = ClientRequest { id:"block-connection".into(),command:ClientCommand::ConnectBlocks { node_id } };
                            outgoing.try_send(Message::Text(serde_json::to_string(&request)?.into())).context("Control writer is full")?;
                        }
                    }
                    frame = reader.next() => {
                        match frame.context("Connection closed")?? {
                            Message::Text(text) => {
                                ensure!(text.len() <= MAX_CONTROL_BYTES, "Tau control frame is too large");
                                let message: ServerMessage = serde_json::from_str(&text).context("Invalid Tau message")?;
                                if let ServerMessage::BlockConnection { offer } = message {
                                    if let Some(service) = &block_service {
                                        let host = settings.url()?.host_str().context("Missing host")?.to_owned();
                                        service.send(crate::blocks::Command::Configure(offer,host));
                                    }
                                    continue;
                                }
                                generation+=1;
                                if let Some(key)=message.replication_key() {generations.insert(key,generation);}
                                if let ServerMessage::Data {content,key,session_id,reports,operation_id,route} = message {
                                    if let Some(operation_id)=operation_id {
                                        if !events.send(Event::Message(epoch,Box::new(ServerMessage::Operation {operation_id,registered:true,response:None}))).await {return Ok(());}
                                    }
                                    if let Some(session_id) = session_id && !reports.is_empty() {
                                        if !events.send(Event::Message(epoch,Box::new(ServerMessage::Receipts {session_id,reports}))).await {return Ok(());}
                                    }
                                    let service=block_service.as_ref().context("Native descriptor without content service")?;
                                    // Descriptor fetches run away from the WebSocket reader/heartbeat.
                                    ensure!(jobs.len() < 64,"Too many unresolved descriptors");
                                    let data=service.downloads();let tx=resolved_tx.clone();let serial=generation;let budget=descriptor_budget.clone();
                                    jobs.spawn(async move {
                                        let Ok(permit)=budget.acquire_many_owned(content.length.min(tau_protocol::blocks::MAX_BLOCK_BYTES).max(1) as u32).await else {return;};
                                        let length=content.length as usize;
                                        let result=data.descriptor(content).await.and_then(|mut message| {
                                            if let Some(route)=route {match &mut message {ServerMessage::SessionPage {catalog_id,..}|ServerMessage::ProjectPage {catalog_id,..}=>*catalog_id=route,_=>bail!("Unexpected descriptor route")}}
                                            Ok(message)
                                        });let _=tx.send((epoch,key,serial,result,permit,length)).await;
                                    });
                                    continue;
                                }
                                if !events.send(Event::Message(epoch, Box::new(message))).await { return Ok(()); }
                            }
                            Message::Ping(payload) => {health.try_send(Message::Pong(payload)).context("Control health writer is full")?;}
                            Message::Pong(payload) => {
                                if waiting.as_ref().is_some_and(|(bytes, _)| bytes.as_slice() == payload.as_ref()) {
                                    let (_, sent) = waiting.take().unwrap();
                                    let at = Instant::now();good_probes+=1;
                                    if healthy_for_backoff(good_probes,connected_at.elapsed()) {delay=1;}
                                    if !events.send(Event::HeartbeatReply { epoch, at, rtt: at.duration_since(sent) }).await { return Ok(()); }
                                }
                            }
                            Message::Close(_) => bail!("Connection closed"),
                            _ => {},
                        }
                    },
                    _ = heartbeat.tick() => {
                        if waiting.is_some() { continue; } // Timeout is independent of the probe cadence.
                        let payload = uuid::Uuid::new_v4().as_bytes().to_vec();
                        health.try_send(Message::Ping(payload.clone().into())).context("Control health writer is full")?;
                        let sent = Instant::now();
                        waiting = Some((payload, sent));
                        if !events.send(Event::HeartbeatSent { epoch, at: sent }).await { return Ok(()); }
                        if sent>=renew_blocks {
                            if let Some(identity)=&block_identity && let Some(node_id)=identity.borrow().clone() {
                                let request=ClientRequest {id:"block-connection".into(),command:ClientCommand::ConnectBlocks {node_id}};
                                outgoing.try_send(Message::Text(serde_json::to_string(&request)?.into())).context("Control writer is full")?;
                            }
                            renew_blocks=sent+Duration::from_secs(1800);
                        }
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
        let jitter=u64::from(uuid::Uuid::new_v4().as_bytes()[0]);
        let wait = tokio::time::sleep(Duration::from_millis(delay*1000+jitter));
        tokio::pin!(wait);
        delay = (delay * 2).min(15);
        loop {
            tokio::select! {
                _ = &mut wait => break,
                command = commands.recv() => match command {
                    None => return,
                    Some(Command::Blocks(command)) => { if let Some(service) = &block_service { service.send(command); } }
                    Some(Command::CancelDownload(key)) => { if let Some(cancel) = downloads.remove(&key) { cancel.send_replace(true); } }
                    Some(Command::Download { key,session,entry,target,limit }) => {
                        start_download(&mut jobs,&mut downloads,block_service.as_ref(),key,session,entry,target,limit,&events).await;
                    }
                    Some(command) => { reject_offline(command, &events).await; }
                },
                _ = jobs.join_next(), if !jobs.is_empty() => {},
            }
        }
    }
    for cancel in downloads.values() { cancel.send_replace(true); }
}
async fn start_download(jobs:&mut tokio::task::JoinSet<()>, downloads:&mut HashMap<String,tokio::sync::watch::Sender<bool>>, service:Option<&crate::blocks::Service>, key:String, session:String, entry:String, target:PathBuf, limit:u64, events:&Events) {
    downloads.retain(|_,cancel|cancel.receiver_count()>0);
    if downloads.contains_key(&key) {return;}
    let Some(service)=service.filter(|_|downloads.len()<2) else {
        events.send(Event::Download {key,path:target,status:tau_transfer::TransferStatus {transferred:0,total:0,network_bytes:0,done:true,
            failure:Some("Content service unavailable or two downloads are already active".into())}}).await;
        return;
    };
    let (cancel,cancellation)=tokio::sync::watch::channel(false); downloads.insert(key.clone(),cancel);
    let context=service.downloads();
    jobs.spawn(context.run(key,session,format!("file:{entry}"),target,limit,cancellation));
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
        Command::CancelDownload(_) | Command::Blocks(_) => {}
    }
}

fn healthy_for_backoff(probes:u32,elapsed:Duration)->bool {probes>=2 && elapsed>=Duration::from_secs(30)}
#[cfg(test)]
mod backoff_tests {
    use super::*;
    #[test] fn flapping_hello_or_one_probe_does_not_reset_retry_backoff() {
        assert!(!healthy_for_backoff(0,Duration::from_secs(60)));
        assert!(!healthy_for_backoff(1,Duration::from_secs(60)));
        assert!(!healthy_for_backoff(20,Duration::from_secs(29)));
        assert!(healthy_for_backoff(2,Duration::from_secs(30)));
    }
}
