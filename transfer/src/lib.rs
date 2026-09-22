use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::net::{Ipv6Addr, SocketAddrV4, SocketAddrV6};
use std::path::PathBuf;
use std::ops::{Bound, RangeBounds};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use bao_tree::io::outboard::PreOrderMemOutboard;
use bao_tree::BaoTree;
use bytes::Bytes;
use iroh::{Endpoint, NodeAddr, NodeId, RelayMode, SecretKey};
use iroh::endpoint::TransportConfig;
use iroh_blobs::{BlobFormat, Hash, HashAndFormat, IROH_BLOCK_SIZE};
use iroh_blobs::get::db::{DownloadProgress, get_to_db};
use iroh_blobs::provider::{EventSender, handle_connection};
use iroh_blobs::store::{BaoBlobSize, ExportMode, Map, MapEntry, ReadableStore, Store as _};
use iroh_blobs::store::fs::Store;
use iroh_blobs::util::local_pool::LocalPool;
use iroh_blobs::util::progress::{IdGenerator, ProgressSendError, ProgressSendResult, ProgressSender};
use iroh_io::AsyncSliceReader;
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;


const GRANT_LIFETIME: Duration = Duration::from_secs(3600);
const STALL_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_GRANTS: usize = 128;
pub const MAX_FILE_BYTES: u64 = 50_000_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferOffer {
    pub node_id: String,
    pub port: u16,
    pub hash: String,
    pub size: u64,
}

#[derive(Debug, Clone)]
struct Source {
    file: Arc<Mutex<File>>,
    outboard: PreOrderMemOutboard<Bytes>,
    expires: Instant,
}

impl Map for Source {
    type Entry = Self;

    async fn get(&self, hash: &Hash) -> io::Result<Option<Self>> {
        Ok((Instant::now() < self.expires && *hash == self.hash()).then(|| self.clone()))
    }
}

impl MapEntry for Source {
    fn hash(&self) -> Hash { self.outboard.root.into() }
    fn size(&self) -> BaoBlobSize { BaoBlobSize::Verified(self.outboard.tree.size()) }
    fn is_complete(&self) -> bool { true }
    async fn outboard(&self) -> io::Result<impl bao_tree::io::fsm::Outboard> { Ok(self.outboard.clone()) }
    async fn data_reader(&self) -> io::Result<impl AsyncSliceReader> { Ok(self.clone()) }
}

impl AsyncSliceReader for Source {
    async fn read_at(&mut self, offset: u64, len: usize) -> io::Result<Bytes> {
        let file = self.file.clone();
        let len = len.min(self.outboard.tree.size().saturating_sub(offset) as usize);
        tokio::task::spawn_blocking(move || {
            let mut file = file.lock().map_err(|_| io::Error::other("source lock failed"))?;
            file.seek(SeekFrom::Start(offset))?;
            let mut bytes = vec![0; len];
            file.read_exact(&mut bytes)?;
            Ok(bytes.into())
        }).await.map_err(io::Error::other)?
    }

    async fn size(&mut self) -> io::Result<u64> { Ok(self.outboard.tree.size()) }
}

pub struct TransferProvider {
    endpoint: Endpoint,
    grants: Arc<Mutex<HashMap<NodeId, Source>>>,
    imports: Semaphore,
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl TransferProvider {
    pub async fn bind(address: SocketAddrV4) -> Result<Self> {
        let endpoint = Endpoint::builder()
            .bind_addr_v4(address)
            .bind_addr_v6(SocketAddrV6::new(Ipv6Addr::LOCALHOST, 0, 0, 0))
            .relay_mode(RelayMode::Disabled)
            .alpns(vec![iroh_blobs::ALPN.to_vec()])
            .transport_config(transport_config())
            .bind().await?;
        if address.port() != 0 && endpoint.bound_sockets().0.port() != address.port() {
            endpoint.close().await;
            bail!("Tau transfer UDP port is already in use");
        }
        let grants = Arc::new(Mutex::new(HashMap::<NodeId, Source>::new()));
        let acceptor = endpoint.clone();
        let allowed = grants.clone();
        let task = tokio::spawn(async move {
            let pool = LocalPool::single();
            let connections = Arc::new(Semaphore::new(16));
            let mut tasks = JoinSet::new();
            let mut cleanup = tokio::time::interval(Duration::from_secs(60));
            loop {
                tokio::select! {
                    incoming = acceptor.accept() => {
                        let Some(incoming) = incoming else { break; };
                        let Ok(permit) = connections.clone().try_acquire_owned() else {
                            incoming.refuse();
                            continue;
                        };
                        let allowed = allowed.clone();
                        let pool = pool.handle().clone();
                        tasks.spawn(async move {
                            let _permit = permit;
                            let Ok(Ok(connection)) = tokio::time::timeout(Duration::from_secs(10), incoming).await else { return; };
                            let source = connection.remote_node_id().ok().and_then(|id| {
                                allowed.lock().unwrap().get(&id).filter(|s| s.expires > Instant::now()).cloned()
                            });
                            let Some(source) = source else {
                                connection.close(1u32.into(), b"Transfer authorization required");
                                return;
                            };
                            let deadline = tokio::time::Instant::from_std(source.expires);
                            let _ = tokio::time::timeout_at(deadline,
                                handle_connection(connection.clone(), source, EventSender::new(None), pool),
                            ).await;
                            connection.close(0u32.into(), b"Transfer closed");
                        });
                    }
                    _ = cleanup.tick() => allowed.lock().unwrap().retain(|_, s| s.expires > Instant::now()),
                    _ = tasks.join_next(), if !tasks.is_empty() => {}
                }
            }
            tasks.abort_all();
            while tasks.join_next().await.is_some() {}
            pool.shutdown().await;
        });
        Ok(Self { endpoint, grants, imports: Semaphore::new(2), task: Mutex::new(Some(task)) })
    }

    pub async fn offer(&self, mut file: File, client_id: &str, limit: u64) -> Result<TransferOffer> {
        let client: NodeId = client_id.parse().context("invalid transfer client identity")?;
        let _permit = self.imports.acquire().await?;
        let source = tokio::task::spawn_blocking(move || {
            let before = file.metadata()?;
            ensure!(before.is_file() && before.len() <= limit.min(MAX_FILE_BYTES), "attachment exceeds transfer limit");
            file.rewind()?;
            let tree = BaoTree::new(before.len(), IROH_BLOCK_SIZE);
            let mut outboard = PreOrderMemOutboard {
                root: blake3::Hash::from([0; 32]), tree, data: vec![0; tree.outboard_size() as usize],
            };
            outboard.root = bao_tree::io::sync::outboard(BufReader::new(&mut file), tree, &mut outboard)?;
            let after = file.metadata()?;
            ensure!(after.len() == before.len() && after.modified()? == before.modified()?, "attachment changed while hashing");
            Ok::<_, anyhow::Error>(Source {
                file: Arc::new(Mutex::new(file)),
                outboard: outboard.map_data(Bytes::from),
                expires: Instant::now() + GRANT_LIFETIME,
            })
        }).await??;
        let offer = TransferOffer {
            node_id: self.endpoint.node_id().to_string(),
            port: self.endpoint.bound_sockets().0.port(),
            hash: source.hash().to_string(),
            size: source.outboard.tree.size(),
        };
        let mut grants = self.grants.lock().unwrap();
        grants.retain(|_, source| source.expires > Instant::now());
        if grants.len() >= MAX_GRANTS && !grants.contains_key(&client) {
            let oldest = grants.iter().min_by_key(|(_, s)| s.expires).map(|(id, _)| *id).unwrap();
            grants.remove(&oldest);
        }
        grants.insert(client, source);
        Ok(offer)
    }

    pub async fn shutdown(&self) {
        self.endpoint.close().await;
        let task = self.task.lock().unwrap().take();
        if let Some(task) = task { let _ = task.await; }
        self.grants.lock().unwrap().clear();
    }
}

impl Drop for TransferProvider {
    fn drop(&mut self) {
        if let Some(task) = self.task.get_mut().unwrap().take() { task.abort(); }
    }
}

fn transport_config() -> TransportConfig {
    let mut config = TransportConfig::default();
    config.initial_mtu(1200).min_mtu(1200).mtu_discovery_config(None);
    config.max_concurrent_bidi_streams(4u32.into()).max_concurrent_uni_streams(0u32.into());
    config.max_idle_timeout(Some(STALL_TIMEOUT.try_into().unwrap()));
    config.keep_alive_interval(Some(Duration::from_secs(5)));
    config
}

#[derive(Debug, Clone)]
pub struct TransferStatus {
    pub transferred: u64,
    pub total: u64,
    pub network_bytes: u64,
    pub done: bool,
    pub failure: Option<String>,
}

#[derive(Debug)]
struct DownloadState {
    status: TransferStatus,
    last_data: Instant,
}

#[derive(Debug, thiserror::Error)]
pub enum TransferError {
    #[error("{reason}")]
    Failed { reason: String },
}

pub struct TransferDownload {
    key: SecretKey,
    state: Arc<Mutex<DownloadState>>,
    cancel: CancellationToken,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl TransferDownload {
    pub fn new() -> Self {
        Self {
            key: SecretKey::generate(rand::rngs::OsRng),
            state: Arc::new(Mutex::new(DownloadState {
                status: TransferStatus { transferred: 0, total: 0, network_bytes: 0, done: false, failure: None },
                last_data: Instant::now(),
            })),
            cancel: CancellationToken::new(),
            thread: Mutex::new(None),
        }
    }

    pub fn node_id(&self) -> String { self.key.public().to_string() }

    pub fn start(&self, offer_json: String, host: String, target: String, limit: u64) -> Result<(), TransferError> {
        if offer_json.len() > 4096 {
            return Err(TransferError::Failed { reason: "Transfer metadata is too large".into() });
        }
        let offer: TransferOffer = serde_json::from_str(&offer_json)
            .map_err(|_| TransferError::Failed { reason: "Invalid transfer metadata".into() })?;
        let mut thread = self.thread.lock().unwrap();
        if thread.is_some() || self.state.lock().unwrap().status.done {
            return Err(TransferError::Failed { reason: "Transfer already started".into() });
        }
        if offer.size > limit.min(MAX_FILE_BYTES) {
            return Err(TransferError::Failed { reason: "too_large".into() });
        }
        let state = self.state.clone();
        state.lock().unwrap().status.total = offer.size;
        let cancel = self.cancel.clone();
        let key = self.key.clone();
        *thread = Some(std::thread::Builder::new().name("tau-download".into()).spawn(move || {
            let result = (|| {
                let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
                runtime.block_on(async {
                    let hash: Hash = offer.hash.parse().context("invalid transfer hash")?;
                    let node: NodeId = offer.node_id.parse().context("invalid transfer server identity")?;
                    let target = PathBuf::from(target);
                    ensure!(target.is_absolute(), "transfer target must be absolute");
                    let parent = target.parent().context("transfer target has no parent")?;
                    let name = target.file_name().context("transfer target has no name")?.to_string_lossy();
                    let partial = parent.join(format!(".{name}.part"));
                    let ready = partial.join("complete");
                    let addresses = tokio::time::timeout(STALL_TIMEOUT, tokio::net::lookup_host((host.as_str(), offer.port)))
                        .await.context("timed_out")??.filter(|a| a.is_ipv4()).collect::<Vec<_>>();
                    ensure!(!addresses.is_empty(), "transfer server has no IPv4 address");
                    let address = NodeAddr::from_parts(node, None, addresses);
                    let endpoint = Endpoint::builder().secret_key(key)
                        .bind_addr_v6(SocketAddrV6::new(Ipv6Addr::LOCALHOST, 0, 0, 0))
                        .relay_mode(RelayMode::Disabled).transport_config(transport_config()).bind().await?;
                    let store = match Store::load(&partial).await {
                        Ok(store) => store,
                        Err(error) => { endpoint.close().await; return Err(error.into()); }
                    };
                    let result = async {
                        let stale = store.blobs().await?.chain(store.partial_blobs().await?)
                            .collect::<io::Result<Vec<_>>>()?.into_iter().filter(|h| *h != hash).collect();
                        store.delete(stale).await?;
                        for attempt in 0..3 {
                            ensure!(!cancel.is_cancelled(), "cancelled");
                            state.lock().unwrap().last_data = Instant::now();
                            let progress = DownloadProgressSink { state: state.clone(), cancel: cancel.clone(), size: offer.size };
                            let request = HashAndFormat::new(hash, BlobFormat::Raw);
                            let transfer = get_to_db(&store, || async {
                                endpoint.connect(address.clone(), iroh_blobs::ALPN).await
                            }, &request, progress);
                            let outcome = {
                                tokio::pin!(transfer);
                                let mut clock = tokio::time::interval(Duration::from_secs(1));
                                loop {
                                    tokio::select! {
                                        result = &mut transfer => break result.map_err(anyhow::Error::from),
                                        _ = cancel.cancelled() => break Err(anyhow::anyhow!("cancelled")),
                                        _ = clock.tick() => {
                                            if state.lock().unwrap().last_data.elapsed() >= STALL_TIMEOUT {
                                                break Err(anyhow::anyhow!("timed_out"));
                                            }
                                        }
                                    }
                                }
                            };
                            match outcome {
                                Ok(stats) => state.lock().unwrap().status.network_bytes += stats.bytes_read,
                                Err(error) => {
                                    if cancel.is_cancelled() { bail!("cancelled"); }
                                    if attempt == 2 { return Err(error); }
                                    tokio::select! {
                                        _ = cancel.cancelled() => bail!("cancelled"),
                                        _ = tokio::time::sleep(Duration::from_secs(1 << attempt)) => {}
                                    }
                                    continue;
                                }
                            }
                            let export_cancel = cancel.clone();
                            store.export(hash, ready.clone(), ExportMode::TryReference, Box::new(move |_| {
                                if export_cancel.is_cancelled() { return Err(io::Error::other("cancelled")); }
                                Ok(())
                            })).await?;
                            let ready_file = ready.clone();
                            let expected_size = offer.size;
                            let verified = tokio::task::spawn_blocking(move || {
                                let mut file = File::options().read(true).write(true).open(ready_file)?;
                                if file.metadata()?.len() != expected_size { return Ok(false); }
                                let mut hasher = blake3::Hasher::new();
                                hasher.update_reader(&mut file)?;
                                file.sync_all()?;
                                Ok::<_, io::Error>(hasher.finalize().as_bytes() == hash.as_bytes())
                            }).await??;
                            if !verified {
                                store.delete(vec![hash]).await?;
                                tokio::fs::remove_file(&ready).await?;
                                if attempt == 2 { bail!("integrity_check_failed"); }
                                state.lock().unwrap().status.transferred = 0;
                                continue;
                            }
                            ensure!(!cancel.is_cancelled(), "cancelled");
                            tokio::fs::rename(&ready, &target).await?;
                            state.lock().unwrap().status.transferred = offer.size;
                            return Ok(());
                        }
                        bail!("interrupted")
                    }.await;
                    let synced = store.sync().await;
                    store.shutdown().await;
                    drop(store);
                    endpoint.close().await;
                    if result.is_ok() {
                        synced?;
                        tokio::fs::remove_dir_all(partial).await?;
                    }
                    result
                })
            })();
            let mut state = state.lock().unwrap();
            state.status.failure = result.err().map(|error: anyhow::Error| format!("{error:#}"));
            state.status.done = true;
        }).map_err(|error| TransferError::Failed { reason: error.to_string() })?);
        Ok(())
    }

    pub fn status(&self) -> TransferStatus { self.state.lock().unwrap().status.clone() }
    pub fn cancel(&self) { self.cancel.cancel(); }

    pub fn join(&self) {
        let thread = self.thread.lock().unwrap().take();
        if let Some(thread) = thread { let _ = thread.join(); }
    }
}

impl Default for TransferDownload {
    fn default() -> Self { Self::new() }
}

impl Drop for TransferDownload {
    fn drop(&mut self) { self.cancel.cancel(); }
}

#[derive(Debug, Clone)]
struct DownloadProgressSink {
    state: Arc<Mutex<DownloadState>>,
    cancel: CancellationToken,
    size: u64,
}

impl IdGenerator for DownloadProgressSink {
    fn new_id(&self) -> u64 { 0 }
}

impl ProgressSender for DownloadProgressSink {
    type Msg = DownloadProgress;

    async fn send(&self, message: DownloadProgress) -> ProgressSendResult<()> { self.try_send(message) }
    fn blocking_send(&self, message: DownloadProgress) -> ProgressSendResult<()> { self.try_send(message) }

    fn try_send(&self, message: DownloadProgress) -> ProgressSendResult<()> {
        if self.cancel.is_cancelled() { return Err(ProgressSendError::ReceiverDropped); }
        let mut state = self.state.lock().unwrap();
        match message {
            DownloadProgress::Found { size, .. } if size != self.size => return Err(ProgressSendError::ReceiverDropped),
            DownloadProgress::FoundLocal { valid_ranges, .. } => {
                let available = valid_ranges.to_chunk_ranges().iter().map(|range| {
                    let start = match range.start_bound() { Bound::Included(n) => n.to_bytes().min(self.size), _ => 0 };
                    let end = match range.end_bound() { Bound::Excluded(n) => n.to_bytes().min(self.size), _ => self.size };
                    end.saturating_sub(start)
                }).sum();
                state.status.transferred = available;
            }
            DownloadProgress::Progress { offset, .. } => {
                state.status.transferred = state.status.transferred.max(offset.min(self.size));
                state.last_data = Instant::now();
            }
            _ => {}
        }
        Ok(())
    }
}
