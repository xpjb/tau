//! One authenticated Iroh connection, many explicitly requested feeds/ranges.
//! Each stream has byte credit, cancellation and a hard frame/decompression bound.
//! Nothing about this protocol knows whether a block is a collapsed tool.
use anyhow::{Context, Result, bail, ensure};
use futures_util::future::BoxFuture;
use iroh::{Endpoint, NodeAddr, NodeId, RelayMode};
use iroh::endpoint::{Connection, RecvStream, SendStream, TransportConfig};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, net::{Ipv6Addr, SocketAddrV4, SocketAddrV6}, sync::{Arc, Mutex}, time::{Duration, Instant}};
use tau_blocks::*;
use tokio::{sync::{Semaphore, watch}, task::JoinSet};

pub const ALPN: &[u8] = b"tau/blocks/1";
const MAX_WIRE_HEADER: usize = MAX_BLOCK_HEADER_BYTES + 1024;
const LEASE: Duration = Duration::from_secs(3600);
const IO_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_STREAMS: usize = 16;

pub trait Backend: Send + Sync + 'static {
    fn feed(&self, request: FeedRequest) -> BoxFuture<'static, Result<FeedPage>>;
    fn read(&self, request: BlockRequest) -> BoxFuture<'static, Result<ContentRange>>;
    /// Hints only: lag/coalescing cannot lose data, which is read by durable cursor.
    fn changes(&self) -> watch::Receiver<u64>;
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Codec { Raw, Zstd }

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum Header {
    Watch { request: BlockWatch, credit: u32 },
    Credit { bytes: u32 },
    Record { watch:usize, record: BlockRecord },
    Page { watch:usize, reset: bool, cursor: FeedCursor, floor: u64, before: Option<FeedPosition>, more: bool },
    Block { block: BlockHeader },
    Data { version: u64, offset: u64, hash: String, length: u32, codec: Codec },
    End,
    Error { message: String },
}

#[derive(Debug)]
pub struct Frame { pub header: Header, pub data: Vec<u8> }
impl Frame {
    pub fn metadata(header: Header) -> Self { Self { header, data:vec![] } }
    fn content(range: &ContentRange) -> Result<Self> {
        ensure!(range.bytes.len() <= BLOCK_CHUNK_BYTES, "Block backend exceeded range limit");
        let compressed = if range.bytes.len() >= 1024 { zstd::bulk::compress(&range.bytes,1)? } else { vec![] };
        let (codec, data) = if !compressed.is_empty() && compressed.len()+16 < range.bytes.len() {
            (Codec::Zstd, compressed)
        } else { (Codec::Raw,range.bytes.clone()) };
        Ok(Self { header:Header::Data { version:range.header.version, offset:range.offset,
            hash:range.hash.clone(), length:range.bytes.len() as u32, codec }, data })
    }
    pub fn decoded(&self) -> Result<Vec<u8>> {
        let Header::Data { hash, length, codec, .. } = &self.header else { ensure!(self.data.is_empty(),"Metadata carried content"); return Ok(vec![]); };
        ensure!(*length as usize <= BLOCK_CHUNK_BYTES && self.data.len() <= BLOCK_CHUNK_BYTES,"Oversized content frame");
        let bytes = match codec {
            Codec::Raw => self.data.clone(),
            Codec::Zstd => zstd::bulk::decompress(&self.data,*length as usize).context("Invalid compressed block chunk")?,
        };
        ensure!(bytes.len() == *length as usize && blake3::hash(&bytes).to_hex().as_str() == hash,"Block content integrity check failed");
        Ok(bytes)
    }
}

fn encode(frame: &Frame) -> Result<Vec<u8>> {
    let head = serde_json::to_vec(&frame.header)?;
    ensure!(head.len() <= MAX_WIRE_HEADER && frame.data.len() <= BLOCK_CHUNK_BYTES,"Frame exceeds byte budget");
    ensure!(matches!(frame.header,Header::Data { .. }) || frame.data.is_empty(),"Unexpected content payload");
    let mut bytes = Vec::with_capacity(8+head.len()+frame.data.len());
    bytes.extend_from_slice(&(head.len() as u32).to_be_bytes());
    bytes.extend_from_slice(&(frame.data.len() as u32).to_be_bytes());
    bytes.extend(head); bytes.extend_from_slice(&frame.data); Ok(bytes)
}

async fn receive(recv: &mut RecvStream) -> Result<(Frame,u32)> {
    let mut lengths = [0;8]; recv.read_exact(&mut lengths).await?;
    let h = u32::from_be_bytes(lengths[..4].try_into().unwrap()) as usize;
    let n = u32::from_be_bytes(lengths[4..].try_into().unwrap()) as usize;
    ensure!(h > 0 && h <= MAX_WIRE_HEADER && n <= BLOCK_CHUNK_BYTES,"Peer exceeded frame budget");
    let mut head = vec![0;h]; recv.read_exact(&mut head).await?;
    let header: Header = serde_json::from_slice(&head)?;
    ensure!(matches!(header,Header::Data { .. }) || n == 0,"Metadata carried content bytes");
    let mut data = vec![0;n]; recv.read_exact(&mut data).await?;
    Ok((Frame { header,data },(8+h+n) as u32))
}

fn config() -> TransportConfig {
    let mut c = TransportConfig::default();
    c.initial_mtu(1200).min_mtu(1200).mtu_discovery_config(None);
    c.max_concurrent_bidi_streams((MAX_STREAMS as u32).into()).max_concurrent_uni_streams(0u32.into());
    c.max_idle_timeout(Some(Duration::from_secs(40).try_into().unwrap()));
    c.keep_alive_interval(Some(Duration::from_secs(5)));
    c
}

type Grants = Arc<Mutex<HashMap<NodeId,Instant>>>;
fn authorized(grants: &Grants, node: &NodeId) -> bool {
    grants.lock().unwrap().get(node).is_some_and(|deadline| *deadline > Instant::now())
}

#[derive(Clone)]
pub struct Acceptor { backend: Arc<dyn Backend>, grants: Grants }
impl Acceptor {
    pub fn new(backend: Arc<dyn Backend>) -> Self { Self { backend,grants:Arc::new(Mutex::new(HashMap::new())) } }
    pub fn authorize(&self, endpoint: &Endpoint, node: &str, lineage: String) -> Result<BulkOffer> {
        let node: NodeId = node.parse().context("Invalid block client identity")?;
        let mut grants = self.grants.lock().unwrap();
        grants.retain(|_,deadline| *deadline > Instant::now());
        ensure!(grants.len() < 128 || grants.contains_key(&node),"Too many block clients");
        grants.insert(node,Instant::now()+LEASE);
        Ok(BulkOffer { node_id:endpoint.node_id().to_string(),port:endpoint.bound_sockets().0.port(),lineage })
    }
    pub async fn accept(&self, connection: Connection) { accept_connection(connection,self.backend.clone(),self.grants.clone()).await; }
}

async fn accept_connection(conn: Connection, backend: Arc<dyn Backend>, allowed: Grants) {
let Ok(node) = conn.remote_node_id() else { return; };
if !authorized(&allowed,&node) { conn.close(1u32.into(),b"Block authorization required"); return; }
let mut streams = JoinSet::new();
loop {
    tokio::select! {
        stream = conn.accept_bi() => {
            let Ok((mut send,mut recv)) = stream else { break; };
            if streams.len() >= MAX_STREAMS { let _ = send.reset(1u32.into()); let _ = recv.stop(1u32.into()); continue; }
            let backend = backend.clone(); let allowed = allowed.clone();
            streams.spawn(async move {
                let result = serve_stream(&mut send,&mut recv,backend,&allowed,node).await;
                if let Err(error) = result {
                    // Bounded diagnostic; no body, credentials or source file paths.
                    let message = error.to_string().chars().take(240).collect();
                    if let Ok(bytes) = encode(&Frame::metadata(Header::Error { message })) {
                        let _ = tokio::time::timeout(Duration::from_secs(1),send.write_all(&bytes)).await;
                    }
                }
                let _ = send.finish();
            });
        }
        _ = streams.join_next(), if !streams.is_empty() => {}
        _ = tokio::time::sleep(Duration::from_secs(1)) => {
            if !authorized(&allowed,&node) { conn.close(1u32.into(),b"Block authorization expired"); break; }
        }
    }
}
streams.abort_all();
}

pub struct Server {
    endpoint: Endpoint,
    grants: Grants,
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}
impl Server {
    pub async fn bind(address: SocketAddrV4, backend: Arc<dyn Backend>) -> Result<Self> {
        let endpoint = Endpoint::builder().bind_addr_v4(address)
            .bind_addr_v6(SocketAddrV6::new(Ipv6Addr::LOCALHOST,0,0,0)).relay_mode(RelayMode::Disabled)
            .alpns(vec![ALPN.to_vec()]).transport_config(config()).bind().await?;
        if address.port() != 0 && endpoint.bound_sockets().0.port() != address.port() {
            endpoint.close().await; bail!("Tau block UDP port is already in use");
        }
        let grants = Arc::new(Mutex::new(HashMap::new()));
        let acceptor = endpoint.clone(); let allowed = grants.clone();
        let task = tokio::spawn(async move {
            let mut jobs = JoinSet::new(); let permits = Arc::new(Semaphore::new(16));
            loop {
                tokio::select! {
                    incoming = acceptor.accept() => {
                        let Some(incoming) = incoming else { break; };
                        let Ok(permit) = permits.clone().try_acquire_owned() else { incoming.refuse(); continue; };
                        let allowed = allowed.clone(); let backend = backend.clone();
                        jobs.spawn(async move {
                            let _permit = permit;
                            let Ok(Ok(conn)) = tokio::time::timeout(Duration::from_secs(10),incoming).await else { return; };
                            accept_connection(conn,backend,allowed).await;
                        });
                    }
                    _ = jobs.join_next(), if !jobs.is_empty() => {}
                }
            }
            jobs.abort_all();
        });
        Ok(Self { endpoint,grants,task:Mutex::new(Some(task)) })
    }
    /// Called only from authenticated control. Renewal does not restart streams.
    pub fn authorize(&self, node: &str, lineage: String) -> Result<BulkOffer> {
        let node: NodeId = node.parse().context("Invalid block client identity")?;
        let mut grants = self.grants.lock().unwrap();
        grants.retain(|_,deadline| *deadline > Instant::now());
        ensure!(grants.len() < 128 || grants.contains_key(&node),"Too many block clients");
        grants.insert(node,Instant::now()+LEASE);
        Ok(BulkOffer { node_id:self.endpoint.node_id().to_string(),port:self.endpoint.bound_sockets().0.port(),lineage })
    }
    pub fn revoke(&self, node: &str) {
        if let Ok(node) = node.parse::<NodeId>() { self.grants.lock().unwrap().remove(&node); }
    }
    pub async fn shutdown(&self) {
        self.endpoint.close().await;
        let task = self.task.lock().unwrap().take();
        if let Some(task) = task { let _ = task.await; }
    }
}
impl Drop for Server {
    fn drop(&mut self) { if let Some(task) = self.task.get_mut().unwrap().take() { task.abort(); } }
}

async fn send_credited(send: &mut SendStream, recv: &mut RecvStream, credit: &mut u32, frame: Frame) -> Result<()> {
    let bytes = encode(&frame)?;
    while (*credit as usize) < bytes.len() {
        let (frame,_) = tokio::time::timeout(IO_TIMEOUT,receive(recv)).await.context("Block consumer stopped granting credit")??;
        let Header::Credit { bytes } = frame.header else { bail!("Expected block byte credit"); };
        ensure!(bytes > 0 && bytes <= BLOCK_WINDOW_BYTES && credit.saturating_add(bytes) <= BLOCK_WINDOW_BYTES,"Invalid block credit");
        *credit += bytes;
    }
    tokio::time::timeout(IO_TIMEOUT,send.write_all(&bytes)).await.context("Block writer stalled")??;
    *credit -= bytes.len() as u32;
    Ok(())
}

async fn serve_stream(send: &mut SendStream, recv: &mut RecvStream, backend: Arc<dyn Backend>, grants: &Grants, node: NodeId) -> Result<()> {
    let (frame,_) = tokio::time::timeout(Duration::from_secs(10),receive(recv)).await??;
    let Header::Watch { mut request, mut credit } = frame.header else { bail!("Expected block watch"); };
    ensure!(credit == BLOCK_WINDOW_BYTES,"Invalid initial block window");
    let priority = match &request { BlockWatch::Feed(_) | BlockWatch::Feeds {..} => 10, BlockWatch::Block(_) => 0 };
    send.set_priority(priority)?;
    let mut changes = backend.changes();
    let mut sent_revision = None;
    let mut initial = true;
    let mut finite_done = std::collections::HashSet::new();
    loop {
        ensure!(authorized(grants,&node),"Block authorization expired");
        match &mut request {
            feeds @ (BlockWatch::Feed(_) | BlockWatch::Feeds {..}) => {
                let requests=match feeds {BlockWatch::Feed(req)=>std::slice::from_mut(req),BlockWatch::Feeds {requests}=>requests.as_mut_slice(),_=>unreachable!()};
                ensure!(!requests.is_empty() && requests.len()<=16,"Invalid feed batch");
                let mut more=false;
                for (watch,req) in requests.iter_mut().enumerate() {
                    if finite_done.contains(&watch) {continue;}
                    let page=backend.feed(req.clone()).await?;
                    if initial || page.reset || !page.records.is_empty() {
                        for record in page.records {send_credited(send,recv,&mut credit,Frame::metadata(Header::Record {watch,record})).await?;}
                        send_credited(send,recv,&mut credit,Frame::metadata(Header::Page {watch,reset:page.reset,cursor:page.cursor.clone(),floor:page.floor,before:page.before,more:page.more})).await?;
                    }
                    if req.before.is_some() {finite_done.insert(watch);}
                    req.cursor=Some(page.cursor);req.floor=page.floor;more|=page.more;
                }
                initial=false;
                if finite_done.len()==requests.len() {break;}
                if more {continue;}
            }
            BlockWatch::Block(req) => {
                let range = tokio::select! {
                    _ = send.stopped() => return Ok(()),
                    range = backend.read(req.clone()) => range?,
                };
                if sent_revision != Some(range.header.revision) {
                    send.set_priority(if matches!(range.header.kind,BlockKind::File | BlockKind::Image) { -10 } else { 5 })?;
                    send_credited(send,recv,&mut credit,Frame::metadata(Header::Block { block:range.header.clone() })).await?;
                    sent_revision = Some(range.header.revision);
                }
                if !range.bytes.is_empty() {
                    send_credited(send,recv,&mut credit,Frame::content(&range)?).await?;
                }
                req.version = range.header.version; req.offset = range.offset + range.bytes.len() as u64;
                if req.offset < range.header.length { continue; }
                if range.header.sealed || !req.follow { break; }
            }
        }
        // Subscribe before reading. Notifications are only wakeups; loss or
        // coalescing never determines the next sequence or byte offset.
        tokio::select! {
            _ = send.stopped() => return Ok(()),
            result = changes.changed() => {
                if result.is_err() { break; }
                // Metadata does not get to monopolize a weak link with a full
                // header for every token. Body tails have their own cadence.
                let delay=if matches!(request,BlockWatch::Block(_)) {50} else {100};
                tokio::time::sleep(Duration::from_millis(delay)).await;
                let _=changes.borrow_and_update();
            }
            _ = tokio::time::sleep(Duration::from_secs(1)) => {}
        }
    }
    send_credited(send,recv,&mut credit,Frame::metadata(Header::End)).await
}

/// Reused for every stream/file in this client/daemon context.
pub struct Client {
    endpoint: Endpoint,
    peer: tokio::sync::Mutex<Option<NodeAddr>>,
    connection: tokio::sync::Mutex<Option<Connection>>,
    streams: Arc<Semaphore>,
    bulk: Arc<Semaphore>,
}
impl Client {
    pub async fn bind() -> Result<Self> {
        let endpoint = Endpoint::builder().bind_addr_v6(SocketAddrV6::new(Ipv6Addr::LOCALHOST,0,0,0))
            .relay_mode(RelayMode::Disabled).transport_config(config()).bind().await?;
        Ok(Self { endpoint,peer:tokio::sync::Mutex::new(None),connection:tokio::sync::Mutex::new(None),streams:Arc::new(Semaphore::new(MAX_STREAMS-2)),bulk:Arc::new(Semaphore::new(6)) })
    }
    pub fn node_id(&self) -> String { self.endpoint.node_id().to_string() }
    pub async fn configure(&self, offer: &BulkOffer, host: &str) -> Result<()> {
        let node: NodeId = offer.node_id.parse().context("Invalid block server identity")?;
        let addresses = tokio::time::timeout(IO_TIMEOUT,tokio::net::lookup_host((host,offer.port))).await??.filter(|a|a.is_ipv4()).collect::<Vec<_>>();
        ensure!(!addresses.is_empty(),"Block server has no IPv4 address");
        let address = NodeAddr::from_parts(node,None,addresses);
        // Same peer/grant renewal preserves the connection and all active streams.
        let mut connection = self.connection.lock().await;
        let mut peer = self.peer.lock().await;
        if peer.as_ref().is_some_and(|old| old.node_id != address.node_id)
            && let Some(old) = connection.take() { old.close(0u32.into(),b"Block peer changed"); }
        *peer = Some(address);
        Ok(())
    }
    async fn connection(&self) -> Result<Connection> {
        let mut slot = self.connection.lock().await;
        if let Some(conn) = slot.as_ref().filter(|c|c.close_reason().is_none()) { return Ok(conn.clone()); }
        let peer = self.peer.lock().await.clone().context("Block connection not authorized yet")?;
        let connection = tokio::time::timeout(IO_TIMEOUT,self.endpoint.connect(peer,ALPN)).await??;
        *slot = Some(connection.clone()); Ok(connection)
    }
    pub async fn watch(&self, request: BlockWatch) -> Result<Watcher> { self.open_watch(request,None).await }
    pub async fn watch_bulk(&self, request: BlockWatch) -> Result<Watcher> {
        let bulk=self.bulk.clone().acquire_owned().await?;
        self.open_watch(request,Some(bulk)).await
    }
    async fn open_watch(&self, request: BlockWatch, bulk:Option<tokio::sync::OwnedSemaphorePermit>) -> Result<Watcher> {
        let permit = self.streams.clone().acquire_owned().await?;
        let connection = self.connection().await?;
        let (mut send,recv) = connection.open_bi().await?;
        send.write_all(&encode(&Frame::metadata(Header::Watch { request,credit:BLOCK_WINDOW_BYTES }))?).await?;
        Ok(Watcher { send,recv,_permit:permit,_bulk:bulk })
    }
    pub async fn shutdown(&self) { self.endpoint.close().await; }
}

pub struct Watcher { send: SendStream, recv: RecvStream, _permit:tokio::sync::OwnedSemaphorePermit, _bulk:Option<tokio::sync::OwnedSemaphorePermit> }
impl Watcher {
    pub async fn next(&mut self) -> Result<(Frame,u32)> { receive(&mut self.recv).await }
    /// Return credit only after consuming/persisting the preceding bounded frame.
    pub async fn consumed(&mut self, bytes: u32) -> Result<()> {
        ensure!(bytes > 0 && bytes <= BLOCK_WINDOW_BYTES,"Invalid consumed byte count");
        self.send.write_all(&encode(&Frame::metadata(Header::Credit { bytes }))?).await?;
        Ok(())
    }
}
impl Drop for Watcher {
    fn drop(&mut self) { let _ = self.send.reset(0u32.into()); let _ = self.recv.stop(0u32.into()); }
}

#[cfg(test)]
mod tests;
