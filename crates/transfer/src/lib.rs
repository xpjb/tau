//! Tau's native data plane. Chat, descriptor and file reads and resumable uploads
//! share one authenticated Iroh endpoint/connection; no legacy blob ALPN remains.
pub mod blocks;

#[derive(Debug, Clone)]
pub struct TransferStatus {
    pub transferred: u64,
    pub total: u64,
    pub network_bytes: u64,
    pub done: bool,
    pub failure: Option<String>,
}
