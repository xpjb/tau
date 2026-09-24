//! Flat, durable block storage and a resumable client cache.
//!
//! All writes run in the caller's transaction: a daemon can commit its source
//! entry, projected blocks and receipt together; a client commits received data
//! before advancing a feed cursor. No function publishes network events.
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};
pub use tau_protocol::blocks::*;
pub mod uploads;
pub mod cache_budget;


pub fn initialize(db: &Connection) -> Result<()> {
    db.execute_batch(include_str!("schema.sql"))?;
    db.execute("INSERT OR IGNORE INTO block_state(singleton,lineage,sequence) VALUES(1,?1,0)", [uuid::Uuid::new_v4().to_string()])?;
    Ok(())
}

pub fn cursor(db: &Connection) -> Result<FeedCursor> {
    Ok(db.query_row("SELECT lineage,sequence FROM block_state WHERE singleton=1", [], |r|
        Ok(FeedCursor { lineage:r.get(0)?, sequence:r.get(1)? }))?)
}

fn writing(db: &Connection) -> Result<()> {
    ensure!(!db.is_autocommit(), "Block writes require a transaction");
    Ok(())
}

pub fn validate_header(h: &BlockHeader) -> Result<()> {
    ensure!(!h.id.is_empty() && h.id.len() <= 256, "Invalid block ID");
    ensure!(h.parent.as_ref().is_none_or(|p| !p.is_empty() && p.len() <= 256 && p != &h.id), "Invalid block parent");
    ensure!(h.length <= MAX_BLOCK_BYTES && h.order <= i64::MAX as u64 && h.version <= i64::MAX as u64 && h.revision <= i64::MAX as u64, "Block bounds exceeded");
    ensure!(serde_json::to_vec(h)?.len() <= MAX_BLOCK_HEADER_BYTES, "Block metadata exceeds its hard byte budget");
    Ok(())
}

fn scope_ok(scope: &str) -> Result<()> {
    ensure!(!scope.is_empty() && scope.len() <= 128, "Invalid block scope");
    Ok(())
}

pub fn header(db: &Connection, scope: &str, id: &str) -> Result<Option<BlockHeader>> {
    let raw: Option<String> = db.query_row("SELECT header FROM blocks WHERE scope=?1 AND id=?2", params![scope,id], |r| r.get(0)).optional()?;
    raw.map(|s| serde_json::from_str(&s).map_err(Into::into)).transpose()
}

pub fn children(db: &Connection, scope: &str, parent: Option<&str>) -> Result<Vec<BlockHeader>> {
    let mut q = db.prepare("SELECT header FROM blocks WHERE scope=?1 AND parent=?2 ORDER BY position,id")?;
    q.query_map(params![scope,parent.unwrap_or_default()], |r| r.get::<_,String>(0))?
        .map(|r| Ok(serde_json::from_str(&r?)?)).collect()
}

fn store_header(db: &Connection, scope: &str, h: &BlockHeader) -> Result<()> {
    validate_header(h)?;
    db.execute("INSERT INTO blocks(scope,id,parent,position,header) VALUES(?1,?2,?3,?4,?5)
        ON CONFLICT(scope,id) DO UPDATE SET parent=excluded.parent,position=excluded.position,header=excluded.header",
        params![scope,h.id,h.parent.as_deref().unwrap_or_default(),h.order,serde_json::to_string(h)?])?;
    Ok(())
}

fn journal(db: &Connection, scope: &str, parent: Option<&str>, order: u64, record: &mut BlockRecord) -> Result<()> {
    let sequence: u64 = db.query_row("UPDATE block_state SET sequence=sequence+1 WHERE singleton=1 RETURNING sequence", [], |r| r.get(0))?;
    match record { BlockRecord::Put { block } => block.revision = sequence, BlockRecord::Remove { revision, .. } => *revision = sequence }
    let id = match &*record { BlockRecord::Put { block } => block.id.as_str(), BlockRecord::Remove { id, .. } => id.as_str() };
    // This is a state-change index, not an event log. Only the latest header (or
    // tombstone) for each direct child matters. Growing content stays in chunks;
    // 100,000 appends do not create 100,000 metadata snapshots to replay or prune.
    db.execute("INSERT INTO block_changes(sequence,scope,parent,id,position,record) VALUES(?1,?2,?3,?4,?5,?6)
        ON CONFLICT(scope,parent,id) DO UPDATE SET sequence=excluded.sequence,position=excluded.position,record=excluded.record",
        params![sequence,scope,parent.unwrap_or_default(),id,order,serde_json::to_string(record)?])?;
    Ok(())
}

fn validate_parent(db: &Connection, scope: &str, h: &BlockHeader) -> Result<()> {
    let mut parent = h.parent.clone();
    for _ in 0..32 {
        let Some(id) = parent else { return Ok(()); };
        ensure!(id != h.id, "Block parent cycle");
        parent = header(db,scope,&id)?.context("Block parent does not exist")?.parent;
    }
    anyhow::bail!("Block nesting exceeds 32 levels")
}

fn part(db: &Connection, scope: &str, id: &str, version: u64, offset: u64) -> Result<Option<(String, Vec<u8>)>> {
    Ok(db.query_row("SELECT c.hash,c.data FROM block_parts p JOIN block_chunks c ON c.hash=p.hash
        WHERE p.scope=?1 AND p.id=?2 AND p.version=?3 AND p.offset=?4",params![scope,id,version,offset],|r| Ok((r.get(0)?,r.get(1)?))).optional()?)
}

fn put_part(db: &Connection, scope: &str, id: &str, version: u64, offset: u64, data: &[u8]) -> Result<()> {
    ensure!(!data.is_empty() && data.len() <= BLOCK_CHUNK_BYTES && offset % BLOCK_CHUNK_BYTES as u64 == 0, "Invalid content chunk");
    let hash = blake3::hash(data).to_hex().to_string();
    db.execute("INSERT INTO block_chunks(hash,data) VALUES(?1,?2) ON CONFLICT(hash) DO UPDATE SET data=excluded.data WHERE block_chunks.data!=excluded.data",params![hash,data])?;
    db.execute("INSERT INTO block_parts VALUES(?1,?2,?3,?4,?5) ON CONFLICT(scope,id,version,offset) DO UPDATE SET hash=excluded.hash",
        params![scope,id,version,offset,hash])?;
    Ok(())
}

/// Install an authenticated range. Overlap must match; later verified chunks can
/// survive a missing/corrupt earlier chunk. At most one 16 KiB tail is rewritten.
fn install(db: &Connection, scope: &str, id: &str, version: u64, mut offset: u64, mut data: &[u8]) -> Result<()> {
    while !data.is_empty() {
        let start = offset / BLOCK_CHUNK_BYTES as u64 * BLOCK_CHUNK_BYTES as u64;
        let at = (offset - start) as usize;
        let mut bytes = part(db,scope,id,version,start)?.map(|(_,b)| b).unwrap_or_default();
        ensure!(at <= bytes.len(), "Content range has a gap within a chunk");
        let n = data.len().min(BLOCK_CHUNK_BYTES-at);
        let overlap = n.min(bytes.len()-at);
        ensure!(bytes[at..at+overlap] == data[..overlap], "Conflicting bytes within a content version");
        if at+n > bytes.len() { bytes.extend_from_slice(&data[overlap..n]); }
        put_part(db,scope,id,version,start,&bytes)?;
        data = &data[n..]; offset += n as u64;
    }
    Ok(())
}

fn is_prefix(db: &Connection, scope: &str, old: &BlockHeader, bytes: &[u8]) -> Result<bool> {
    if bytes.len() < old.length as usize { return Ok(false); }
    let mut offset = 0;
    while offset < old.length {
        let (_, part) = part(db,scope,&old.id,old.version,offset)?.context("Source block has missing content")?;
        if part.is_empty() || offset as usize + part.len() > old.length as usize || bytes[offset as usize..offset as usize+part.len()] != part { return Ok(false); }
        offset += part.len() as u64;
    }
    Ok(true)
}

/// Project a complete value without changing its identity. Unchanged content and
/// sealing do not get a new version. Appending only installs the new suffix.
pub fn put(db: &Connection, scope: &str, mut h: BlockHeader, bytes: &[u8]) -> Result<bool> {
    writing(db)?; scope_ok(scope)?; validate_parent(db,scope,&h)?;
    ensure!(bytes.len() as u64 <= MAX_BLOCK_BYTES, "Block is too large");
    let old = header(db,scope,&h.id)?;
    let extends = if let Some(old) = &old {
        is_prefix(db,scope,old,bytes)? && (!old.sealed || old.length == bytes.len() as u64 && h.sealed)
    } else { false };
    h.version = match &old { Some(old) if extends => old.version, Some(old) => old.version.checked_add(1).context("Block version exhausted")?, None => 1 };
    h.length = bytes.len() as u64;
    h.revision = old.as_ref().map_or(0, |old| old.revision);
    if old.as_ref() == Some(&h) { return Ok(false); }
    validate_header(&h)?;
    if let Some(old) = &old && old.parent != h.parent {
        journal(db,scope,old.parent.as_deref(),old.order,&mut BlockRecord::Remove { id:old.id.clone(), revision:0 })?;
    }
    let mut record = BlockRecord::Put { block:h.clone() };
    journal(db,scope,h.parent.as_deref(),h.order,&mut record)?;
    let BlockRecord::Put { block:h } = record else { unreachable!() };
    store_header(db,scope,&h)?;
    let start = if extends { old.unwrap().length } else {
        db.execute("DELETE FROM block_parts WHERE scope=?1 AND id=?2",params![scope,h.id])?;
        0
    };
    install(db,scope,&h.id,h.version,start,&bytes[start as usize..])?;
    Ok(true)
}

/// Change presentation/sealing without rereading or rewriting content.
pub fn set_header(db: &Connection, scope: &str, mut h: BlockHeader) -> Result<()> {
    writing(db)?; scope_ok(scope)?; validate_parent(db,scope,&h)?;
    let old = header(db,scope,&h.id)?.context("Unknown block")?;
    ensure!(old.version == h.version && old.length == h.length && (!old.sealed || h.sealed),"Metadata update changed content");
    h.revision = old.revision;
    if h == old { return Ok(()); }
    if old.parent != h.parent { journal(db,scope,old.parent.as_deref(),old.order,&mut BlockRecord::Remove {id:h.id.clone(),revision:0})?; }
    let mut record = BlockRecord::Put {block:h.clone()};
    journal(db,scope,h.parent.as_deref(),h.order,&mut record)?;
    let BlockRecord::Put {block:h} = record else {unreachable!()};
    store_header(db,scope,&h)
}

/// Native streaming producers can append directly, without rebuilding prefixes.
pub fn append(db: &Connection, scope: &str, id: &str, version: u64, offset: u64, bytes: &[u8], sealed: bool) -> Result<BlockHeader> {
    writing(db)?;
    let mut h = header(db,scope,id)?.context("Unknown block")?;
    ensure!(h.version == version && h.length == offset && !h.sealed, "Stale block append");
    ensure!(offset.saturating_add(bytes.len() as u64) <= MAX_BLOCK_BYTES, "Block is too large");
    install(db,scope,id,version,offset,bytes)?;
    h.length += bytes.len() as u64; h.sealed = sealed;
    let mut record = BlockRecord::Put { block:h.clone() };
    journal(db,scope,h.parent.as_deref(),h.order,&mut record)?;
    let BlockRecord::Put { block:h } = record else { unreachable!() };
    store_header(db,scope,&h)?;
    Ok(h)
}

pub fn remove(db: &Connection, scope: &str, id: &str) -> Result<()> {
    writing(db)?;
    let Some(h) = header(db,scope,id)? else { return Ok(()); };
    for child in children(db,scope,Some(id))? { remove(db,scope,&child.id)?; }
    journal(db,scope,h.parent.as_deref(),h.order,&mut BlockRecord::Remove { id:id.into(), revision:0 })?;
    db.execute("DELETE FROM blocks WHERE scope=?1 AND id=?2",params![scope,id])?;
    Ok(())
}

pub fn remove_scope(db: &Connection, scope: &str) -> Result<()> {
    for root in children(db,scope,None)? { remove(db,scope,&root.id)?; }
    Ok(())
}

/// Returns only requested direct-child metadata. Never reads block content.
pub fn feed(db: &Connection, request: &FeedRequest) -> Result<FeedPage> {
    scope_ok(&request.scope)?;
    let head = cursor(db)?;
    let retained: u64 = db.query_row("SELECT retained_from FROM block_state WHERE singleton=1",[],|r| r.get(0))?;
    let continuing = request.before.is_none() && request.cursor.as_ref().is_some_and(|c|
        c.lineage == head.lineage && c.sequence >= retained && c.sequence <= head.sequence);
    if continuing {
        let mut q = db.prepare("SELECT record FROM block_changes WHERE scope=?1 AND parent=?2 AND sequence>?3 ORDER BY sequence LIMIT ?4")?;
        let records = q.query_map(params![request.scope,request.parent.as_deref().unwrap_or_default(),request.cursor.as_ref().unwrap().sequence,MAX_FEED_PAGE+1],|r| r.get::<_,String>(0))?
            .map(|r| Ok(serde_json::from_str::<BlockRecord>(&r?)?)).collect::<Result<Vec<_>>>()?;
        let more = records.len() > MAX_FEED_PAGE;
        let records: Vec<_> = records.into_iter().take(MAX_FEED_PAGE).collect();
        let sequence = if more { records.last().unwrap().revision() } else { head.sequence };
        return Ok(FeedPage { reset:false, records, cursor:FeedCursor { sequence, ..head }, floor:request.floor, before:None, more });
    }
    let mut q = db.prepare("SELECT header FROM blocks WHERE scope=?1 AND parent=?2 AND (?3 IS NULL OR position<?3 OR (position=?3 AND id<?4)) ORDER BY position DESC,id DESC LIMIT ?5")?;
    let mut blocks = q.query_map(params![request.scope,request.parent.as_deref().unwrap_or_default(),request.before.as_ref().map(|p|p.order),request.before.as_ref().map(|p|&p.id),MAX_FEED_PAGE+1], |r| r.get::<_,String>(0))?
        .map(|r| Ok(serde_json::from_str::<BlockHeader>(&r?)?)).collect::<Result<Vec<_>>>()?;
    let more = blocks.len() > MAX_FEED_PAGE;
    blocks.truncate(MAX_FEED_PAGE); blocks.reverse();
    let floor = blocks.first().map_or(0,|h|h.order);
    Ok(FeedPage { reset:true, before:more.then(||FeedPosition {order:floor,id:blocks.first().unwrap().id.clone()}), floor, cursor:head,
        records:blocks.into_iter().map(|block|BlockRecord::Put { block }).collect(), more:false })
}

#[derive(Clone, Debug)]
pub struct ContentRange { pub header: BlockHeader, pub offset: u64, pub hash: String, pub bytes: Vec<u8> }

/// A range never crosses a chunk boundary and is always hard byte-bounded.
pub fn read(db: &Connection, request: &BlockRequest) -> Result<ContentRange> {
    scope_ok(&request.scope)?;
    let h = header(db,&request.scope,&request.id)?.context("Unknown block")?;
    let offset = if request.version == h.version { request.offset } else { 0 };
    ensure!(offset <= h.length, "Block offset is beyond the durable head");
    let bytes = if offset == h.length { vec![] } else {
        let start = offset / BLOCK_CHUNK_BYTES as u64 * BLOCK_CHUNK_BYTES as u64;
        let (hash, bytes) = part(db,&request.scope,&request.id,h.version,start)?.context("Source content is missing")?;
        ensure!(blake3::hash(&bytes).to_hex().as_str() == hash, "Source content is corrupt");
        bytes.get((offset-start) as usize..).context("Source content has a gap")?.to_vec()
    };
    ensure!(offset + bytes.len() as u64 <= h.length && bytes.len() <= BLOCK_CHUNK_BYTES, "Source content exceeds its head");
    Ok(ContentRange { header:h, offset, hash:blake3::hash(&bytes).to_hex().to_string(), bytes })
}

// Client cache operations. The source journal cursor and verified content prefix
// are separate: knowing a block exists does not claim its bytes were downloaded.
pub fn cache_page(db: &Connection, request: &FeedRequest, page: &FeedPage) -> Result<()> {
    writing(db)?;
    ensure!(page.records.len() <= MAX_FEED_PAGE && !page.cursor.lineage.is_empty() && page.cursor.lineage.len() <= 128, "Invalid feed page");
    cache_lineage(db,&page.cursor.lineage)?;
    if request.before.is_none() && let Some(old) = cached_feed(db,&request.scope,request.parent.as_deref())?
        && old.cursor.lineage == page.cursor.lineage && old.cursor.sequence > page.cursor.sequence { return Ok(()); }
    if page.reset {
        let ids = page.records.iter().filter_map(|r| match r { BlockRecord::Put { block } => Some(block.id.as_str()), _ => None }).collect::<Vec<_>>();
        let first = page.records.iter().filter_map(|r|match r {BlockRecord::Put {block}=>Some((block.order,block.id.as_str())),_=>None}).min();
        for old in children(db,&request.scope,request.parent.as_deref())? {
            let position = (old.order,old.id.as_str());
            if first.is_none_or(|first|position >= first) && request.before.as_ref().is_none_or(|p|position < (p.order,p.id.as_str()))
                && old.revision <= page.cursor.sequence && !ids.contains(&old.id.as_str()) {
                cache_remove(db,&request.scope,&old.id,page.cursor.sequence)?;
            }
        }
    }
    for record in &page.records {
        ensure!(record.revision() <= page.cursor.sequence,"Record exceeds feed cursor");
        if let BlockRecord::Put { block } = record { ensure!(block.parent == request.parent,"Block belongs to another feed"); }
        match record {
            BlockRecord::Put { block } => { cache_header(db,&request.scope,block)?; }
            BlockRecord::Remove { id, revision } => {
                cache_remove(db,&request.scope,id,*revision)?;
            }
        }
    }
    // A normal journal update must preserve the older-history boundary.
    let mut saved = page.clone(); saved.records.clear();
    if let Some(old) = cached_feed(db,&request.scope,request.parent.as_deref())? {
        if request.before.is_some() {
            // An older page is not proof that newer changes were received. Never
            // advance the live cursor using a history request's later DB head.
            saved.cursor = old.cursor;
            saved.floor = saved.floor.min(old.floor);
        } else if !page.reset {
            saved.before = old.before;
            saved.floor = saved.floor.min(old.floor);
        }
    }
    db.execute("INSERT INTO block_cache_feeds VALUES(?1,?2,?3) ON CONFLICT(scope,parent) DO UPDATE SET page=excluded.page",
        params![request.scope,request.parent.as_deref().unwrap_or_default(),serde_json::to_string(&saved)?])?;
    Ok(())
}

pub fn cached_feed(db: &Connection, scope: &str, parent: Option<&str>) -> Result<Option<FeedPage>> {
    let raw: Option<String> = db.query_row("SELECT page FROM block_cache_feeds WHERE scope=?1 AND parent=?2",params![scope,parent.unwrap_or_default()],|r| r.get(0)).optional()?;
    raw.map(|raw| serde_json::from_str(&raw).map_err(Into::into)).transpose()
}

fn cache_remove(db: &Connection, scope: &str, id: &str, revision: u64) -> Result<()> {
    db.execute("INSERT INTO block_cache_tombstones VALUES(?1,?2,?3) ON CONFLICT(scope,id) DO UPDATE SET revision=max(revision,excluded.revision)",params![scope,id,revision])?;
    if header(db,scope,id)?.is_some_and(|h| h.revision <= revision) {
        db.execute("DELETE FROM blocks WHERE scope=?1 AND id=?2",params![scope,id])?;
    }
    Ok(())
}

pub fn cache_header(db: &Connection, scope: &str, h: &BlockHeader) -> Result<bool> {
    writing(db)?; scope_ok(scope)?; validate_header(h)?;
    let tombstone: Option<u64> = db.query_row("SELECT revision FROM block_cache_tombstones WHERE scope=?1 AND id=?2",params![scope,h.id],|r|r.get(0)).optional()?;
    if tombstone.is_some_and(|r| r >= h.revision) { return Ok(false); }
    if let Some(old) = header(db,scope,&h.id)? {
        if old.revision > h.revision { return Ok(false); }
        if old.revision == h.revision { ensure!(old == *h,"Conflicting block metadata revision"); return Ok(false); }
        if old.version == h.version {
            ensure!(h.length >= old.length && (!old.sealed || h.length == old.length && h.sealed), "Content changed without a new version");
        } else {
            ensure!(h.version > old.version,"Content version moved backwards");
            db.execute("DELETE FROM block_parts WHERE scope=?1 AND id=?2",params![scope,h.id])?;
        }
    }
    store_header(db,scope,h)?;
    Ok(true)
}

/// Bind the cache to the authenticated daemon before starting any data watches.
pub fn cache_lineage(db: &Connection, lineage: &str) -> Result<()> {
    writing(db)?;
    ensure!(!lineage.is_empty() && lineage.len() <= 128,"Invalid data lineage");
    if cursor(db)?.lineage != lineage {
        db.execute("DELETE FROM blocks",[])?;
        db.execute("DELETE FROM block_cache_feeds",[])?;
        db.execute("DELETE FROM block_cache_tombstones",[])?;
        db.execute("UPDATE block_state SET lineage=?1,sequence=0,retained_from=0 WHERE singleton=1",[lineage])?;
    }
    Ok(())
}

pub fn cache_range(db: &Connection, scope: &str, range: &ContentRange) -> Result<()> {
    writing(db)?;
    ensure!(range.bytes.len() <= BLOCK_CHUNK_BYTES && range.offset.saturating_add(range.bytes.len() as u64) <= range.header.length, "Content exceeds its bounds");
    ensure!(blake3::hash(&range.bytes).to_hex().as_str() == range.hash,"Content hash mismatch");
    cache_header(db,scope,&range.header)?;
    let Some(current) = header(db,scope,&range.header.id)? else { return Ok(()); };
    if current.version != range.header.version { return Ok(()); }
    install(db,scope,&current.id,current.version,range.offset,&range.bytes)
}

/// Validate retained bytes before advertising an offset. Corruption invalidates
/// that chunk, not the operation journal or unrelated content.
pub fn cached_prefix(db: &Connection, scope: &str, id: &str) -> Result<u64> {
    writing(db)?;
    let Some(h) = header(db,scope,id)? else { return Ok(0); };
    let mut offset = 0;
    while offset < h.length {
        let Some((hash, bytes)) = part(db,scope,id,h.version,offset)? else { break; };
        if bytes.is_empty() || bytes.len() > BLOCK_CHUNK_BYTES || offset+bytes.len() as u64 > h.length || blake3::hash(&bytes).to_hex().as_str() != hash {
            db.execute("DELETE FROM block_parts WHERE scope=?1 AND id=?2 AND version=?3 AND offset=?4",params![scope,id,h.version,offset])?;
            break;
        }
        offset += bytes.len() as u64;
        if bytes.len() < BLOCK_CHUNK_BYTES { break; }
    }
    Ok(offset)
}

pub fn cached_content(db: &Connection, scope: &str, id: &str) -> Result<Vec<u8>> {
    let Some(h) = header(db,scope,id)? else { return Ok(vec![]); };
    let mut result = Vec::new();
    while (result.len() as u64) < h.length {
        let Some((hash,bytes)) = part(db,scope,id,h.version,result.len() as u64)? else { break; };
        ensure!(!bytes.is_empty() && bytes.len() <= BLOCK_CHUNK_BYTES && blake3::hash(&bytes).to_hex().as_str() == hash,"Cached content is corrupt");
        let n = bytes.len(); result.extend(bytes);
        if n < BLOCK_CHUNK_BYTES { break; }
    }
    Ok(result)
}

/// Unpublished, bounded-batch imports. Staging does not enter any feed or wake
/// viewers. Publishing moves verified chunk references, not a whole-file buffer.
pub const STAGING_SCOPE: &str = "@staging";
pub fn stage_append(db: &Connection, id: &str, offset: u64, bytes: &[u8]) -> Result<()> {
    writing(db)?;
    ensure!(bytes.len() <= BLOCK_CHUNK_BYTES*8,"Staging batch exceeds its bound");
    let mut h = header(db,STAGING_SCOPE,id)?.unwrap_or(BlockHeader {id:id.into(),parent:None,order:0,kind:BlockKind::File,
        meta:serde_json::Value::Null,version:1,length:0,sealed:false,revision:0});
    ensure!(h.length == offset && h.revision == 0 && offset.saturating_add(bytes.len() as u64) <= MAX_BLOCK_BYTES,"Invalid staging offset");
    h.length += bytes.len() as u64;
    store_header(db,STAGING_SCOPE,&h)?;
    install(db,STAGING_SCOPE,id,h.version,offset,bytes)
}
pub fn discard_stage(db: &Connection, id: &str) -> Result<()> {
    writing(db)?;
    db.execute("DELETE FROM blocks WHERE scope=?1 AND id=?2",params![STAGING_SCOPE,id])?;
    Ok(())
}
pub fn discard_staging(db: &Connection) -> Result<()> {
    writing(db)?; db.execute("DELETE FROM blocks WHERE scope=?1",[STAGING_SCOPE])?; Ok(())
}
pub fn publish_stage(db: &Connection, stage: &str, scope: &str, id: &str, meta: serde_json::Value) -> Result<BlockHeader> {
    writing(db)?;
    let staged = header(db,STAGING_SCOPE,stage)?.context("Staging block is missing")?;
    let mut h = header(db,scope,id)?.context("Target block no longer exists")?;
    ensure!(!h.sealed && h.length == 0,"Target block was already published");
    h.length = staged.length; h.sealed = true; h.meta = meta;
    validate_header(&h)?;
    db.execute("INSERT INTO block_parts(scope,id,version,offset,hash) SELECT ?1,?2,?3,offset,hash FROM block_parts WHERE scope=?4 AND id=?5",
        params![scope,id,h.version,STAGING_SCOPE,stage])?;
    let mut record = BlockRecord::Put {block:h.clone()};
    journal(db,scope,h.parent.as_deref(),h.order,&mut record)?;
    let BlockRecord::Put {block:h} = record else {unreachable!()};
    store_header(db,scope,&h)?;
    discard_stage(db,stage)?;
    Ok(h)
}

#[cfg(test)]
mod tests;
