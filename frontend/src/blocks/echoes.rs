//! Disposable, bounded local body reuse. A receipt or matching request ID alone
//! is NOT proof of bytes: only an authenticated sealed header's digest can adopt
//! these bytes into the verified replica. Entries survive queue -> history moves.
use super::*;
use rusqlite::{OptionalExtension, params};
use tau_protocol::{ClientCommand, QueueOperation};

const MAX_BYTES: usize = 32 * 1024 * 1024;
const MAX_ENTRIES: usize = 256;

pub(super) fn initialize(db: &Connection) -> Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS local_echoes (
        seq INTEGER PRIMARY KEY, scope TEXT NOT NULL, request TEXT NOT NULL,
        hash TEXT NOT NULL, body BLOB NOT NULL, UNIQUE(scope,request,hash));
        CREATE INDEX IF NOT EXISTS block_body_hash ON blocks(scope,json_extract(header,'$.meta.bodyHash'))
        WHERE json_extract(header,'$.meta.bodyHash') IS NOT NULL;")?;
    Ok(())
}

impl Cache {
    pub(crate) fn remember_local(&self, scope: &str, local: &LocalChat, lineage: &str) -> Result<()> {
        if local.pending.is_empty() { return Ok(()); }
        let db = self.db.lock().unwrap();
        // Saved intent remains in the authored store. Optional replica reuse
        // must not turn startup/viewport contention into a lock error popup.
        let Some(tx) = try_replica_maintenance(&db)? else { return Ok(()); };
        // Never lend bytes or retire authored work across a source restore.
        if tau_blocks::cursor(&tx)?.lineage != lineage { return Ok(()); }
        for pending in &local.pending {
            let (request, text) = match &pending.request.command {
                ClientCommand::Prompt { text, .. } => (&pending.request.id, text),
                ClientCommand::QueueControl { operation: QueueOperation::Edit { request_id, text, .. }, .. } => (request_id, text),
                _ => continue,
            };
            if text.len() > MAX_BYTES { continue; }
            tx.execute("INSERT OR IGNORE INTO local_echoes(scope,request,hash,body) VALUES(?1,?2,?3,?4)",
                params![scope, request, blake3::hash(text.as_bytes()).to_hex().as_str(), text.as_bytes()])?;
        }
        loop {
            let (count, bytes): (usize, usize) = tx.query_row("SELECT count(*),coalesce(sum(length(body)),0) FROM local_echoes", [], |r| Ok((r.get(0)?, r.get(1)?)))?;
            if count <= MAX_ENTRIES && bytes <= MAX_BYTES { break; }
            tx.execute("DELETE FROM local_echoes WHERE seq=(SELECT min(seq) FROM local_echoes)", [])?;
        }
        // Header-first and input-first arrival are both valid. This indexed
        // join touches only locally authored candidates, not every old body.
        let headers = tx.prepare("SELECT b.header FROM blocks b JOIN local_echoes e
            ON b.scope=e.scope AND json_extract(b.header,'$.meta.bodyHash')=e.hash
                AND coalesce(json_extract(b.header,'$.meta.request.requestId'),json_extract(b.header,'$.meta.event.origin.requestId'))=e.request
            WHERE b.scope=?1 AND json_extract(b.header,'$.meta.bodyHash') IS NOT NULL")?.query_map([scope], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut changed = Vec::new();
        for raw in headers {
            let h: BlockHeader = serde_json::from_str(&raw)?;
            if adopt(&tx, scope, &h)? { changed.push(h.id); }
        }
        tx.commit()?;
        drop(db);
        for id in changed { self.changed(scope, &id, false); }
        Ok(())
    }
}

pub(super) fn adopt(db: &Connection, scope: &str, h: &BlockHeader) -> Result<bool> {
    if !h.sealed || h.kind != BlockKind::Text || h.length > MAX_BYTES as u64 { return Ok(false); }
    let request = if h.parent.as_deref() == Some(QUEUE) {
        h.meta.pointer("/request/requestId").and_then(|v| v.as_str())
    } else if h.meta.pointer("/event/role").and_then(|v| v.as_str()) == Some("user") {
        h.meta.pointer("/event/origin/requestId").and_then(|v| v.as_str())
    } else { None };
    let (Some(request), Some(hash)) = (request, h.meta.get("bodyHash").and_then(|v| v.as_str())) else { return Ok(false); };
    if tau_blocks::cache_budget::stored_bytes(db, scope, &h.id)? == h.length { return Ok(false); }
    let bytes: Option<Vec<u8>> = db.query_row("SELECT body FROM local_echoes WHERE scope=?1 AND request=?2 AND hash=?3",
        params![scope, request, hash], |r| r.get(0)).optional()?;
    let Some(bytes) = bytes else { return Ok(false); };
    if bytes.len() as u64 != h.length || blake3::hash(&bytes).to_hex().as_str() != hash { return Ok(false); }
    for (i, bytes) in bytes.chunks(BLOCK_CHUNK_BYTES).enumerate() {
        tau_blocks::cache_range(db, scope, &ContentRange { header: h.clone(), offset: (i * BLOCK_CHUNK_BYTES) as u64,
            hash: blake3::hash(bytes).to_hex().to_string(), bytes: bytes.to_vec() })?;
    }
    tau_blocks::cache_budget::enforce(db, scope, &h.id, tau_blocks::cache_budget::DEFAULT_CACHE_BYTES)?;
    Ok(true)
}
