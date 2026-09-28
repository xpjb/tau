//! Finite filesystem RPCs multiplexed with the existing native streams. Bodies
//! use the same compression, chunk hashes, byte-credit and cancellation machinery.
use super::*;
use tau_protocol::files::*;

pub(super) async fn serve(send: &mut SendStream, recv: &mut RecvStream, backend: Arc<dyn Backend>, grants: &Grants, node: NodeId, request: FileRequest, mut credit: u32) -> Result<()> {
    ensure!(credit == BLOCK_WINDOW_BYTES, "Invalid filesystem byte credit");
    ensure!(authorized(grants, &node), "Block authorization expired");
    ensure!(!request.session_id.is_empty() && request.session_id.len() <= 128, "Invalid chat identity");
    send.set_priority(3)?;
    let reply = tokio::select! {
        _ = send.stopped() => return Ok(()),
        reply = backend.files(request) => reply?,
    };
    let bytes = serde_json::to_vec(&reply)?;
    ensure!(bytes.len() <= MAX_FILE_REPLY_BYTES, "Filesystem response exceeds limit");
    send_credited(send, recv, &mut credit, Frame::metadata(Header::Browsed { length: bytes.len() as u64, hash: blake3::hash(&bytes).to_hex().to_string() }), None).await?;
    for (i, chunk) in bytes.chunks(BLOCK_CHUNK_BYTES).enumerate() {
        ensure!(authorized(grants, &node), "Block authorization expired");
        let compressed = if chunk.len() >= 1024 { zstd::bulk::compress(chunk, 1)? } else { vec![] };
        let (codec, data) = if !compressed.is_empty() && compressed.len()+16 < chunk.len() { (Codec::Zstd, compressed) } else { (Codec::Raw, chunk.to_vec()) };
        let frame = Frame { header: Header::Data { version: 1, offset: (i*BLOCK_CHUNK_BYTES) as u64, hash: blake3::hash(chunk).to_hex().to_string(), length: chunk.len() as u32, codec }, data };
        send_credited(send, recv, &mut credit, frame, None).await?;
    }
    send_credited(send, recv, &mut credit, Frame::metadata(Header::End), None).await
}
impl Client {
    /// Dropping this future resets only its own stream. It shares the foreground
    /// admission budget and authenticated endpoint, never opens another client.
    pub async fn files(&self, request: FileRequest) -> Result<FileReply> {
        let class = self.foreground.clone().acquire_owned().await?;
        let permit = self.streams.clone().acquire_owned().await?;
        let connection = self.connection().await?;
        let (mut send, recv) = connection.open_bi().await?;
        send.set_priority(3)?;
        let bytes = encode(&Frame::metadata(Header::Browse { request, credit: BLOCK_WINDOW_BYTES }))?;
        send.write_all(&bytes).await?;
        self.stats.tx.fetch_add(bytes.len() as u64, Ordering::Relaxed);
        self.stats.opened();
        let mut stream = Watcher { stats: self.stats.clone(), complete: false, send, recv, _permit: permit, _bulk: Some(class) };
        let mut expected = None; let mut body = Vec::new();
        loop {
            let (frame, wire_bytes) = tokio::time::timeout(IO_TIMEOUT, stream.next()).await??;
            match &frame.header {
                Header::Browsed { length, hash } => {
                    ensure!(expected.is_none() && *length <= MAX_FILE_REPLY_BYTES as u64 && hash.len() == 64, "Invalid filesystem response header");
                    expected = Some((*length, hash.clone()));
                }
                Header::Data { version, offset, .. } => {
                    let (length, _) = expected.as_ref().context("Filesystem data without header")?;
                    let bytes = frame.decoded()?;
                    ensure!(*version == 1 && *offset == body.len() as u64 && body.len() as u64+bytes.len() as u64 <= *length, "Invalid filesystem range");
                    body.extend(bytes);
                }
                Header::End => {
                    let (length, hash) = expected.context("Filesystem response without header")?;
                    ensure!(length == body.len() as u64 && blake3::hash(&body).to_hex().as_str() == hash, "Filesystem response integrity failure");
                    return Ok(serde_json::from_slice(&body)?);
                }
                Header::Error { message } => bail!("{message}"),
                _ => bail!("Unexpected filesystem response"),
            }
            let _ = stream.consumed(wire_bytes).await;
        }
    }
}
