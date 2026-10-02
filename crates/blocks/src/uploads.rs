//! Durable unpublished client-originated blocks. No upload is readable as an
//! input until its complete raw digest has been checked and it has been sealed.
use super::*;

pub const UPLOAD_QUOTA: u64 = 1024 * 1024 * 1024;
pub const MAX_UPLOADS: usize = 4096;

pub fn validate(spec: &UploadSpec) -> Result<()> {
    ensure!(!spec.id.is_empty() && spec.id.len() <= 128 && spec.id.bytes().all(|b|b.is_ascii_alphanumeric() || b == b'-' || b == b'_'), "Invalid upload ID");
    ensure!(spec.hash.len() == 64 && spec.hash.bytes().all(|b|b.is_ascii_hexdigit() && !b.is_ascii_uppercase()), "Invalid upload hash");
    let limit = match &spec.purpose {
        UploadPurpose::Command => MAX_COMMAND_BYTES,
        UploadPurpose::File { session_id, file_name } => {
            ensure!(!session_id.is_empty() && session_id.len() <= 128 && session_id.bytes().all(|b|b.is_ascii_alphanumeric() || b == b'-' || b == b'_'), "Invalid upload session");
            ensure!(!file_name.trim().is_empty() && file_name.len() <= 1024, "Invalid upload file name");
            tau_protocol::MAX_UPLOAD_BYTES as u64
        }
    };
    ensure!(spec.length > 0 && spec.length <= limit, "Upload exceeds its byte limit");
    Ok(())
}

pub fn begin(db: &Connection, spec: &UploadSpec) -> Result<UploadStatus> {
    writing(db)?; validate(spec)?;
    let now=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs();
    // Pure data leases may expire; operation receipts never do. Retrying an
    // expired upload re-verifies bytes and cannot itself repeat an effect.
    db.execute("DELETE FROM blocks WHERE scope=?1 AND position<?2",params![UPLOAD_SCOPE,now.saturating_sub(7*24*3600)])?;
    if header(db, UPLOAD_SCOPE, &spec.id)?.is_none() {
        let (count, bytes): (usize,u64) = db.query_row("SELECT count(*),coalesce(sum(CASE WHEN json_extract(header,'$.sealed')=1 AND json_extract(header,'$.meta.file') IS NOT NULL THEN 0 ELSE json_extract(header,'$.meta.upload.length') END),0) FROM blocks WHERE scope=?1", [UPLOAD_SCOPE], |r| Ok((r.get(0)?,r.get(1)?)))?;
        ensure!(count < MAX_UPLOADS && bytes.saturating_add(spec.length) <= UPLOAD_QUOTA, "Durable upload quota is full");
        // Upload progress is not display state: do not journal every chunk or
        // wake unrelated feeds. Publication is the final immutable seal.
        store_header(db, UPLOAD_SCOPE, &BlockHeader { id:spec.id.clone(),parent:None,order:now,kind:BlockKind::File,
            meta:serde_json::json!({"upload":spec}),version:1,length:0,sealed:false,revision:1 })?;
    }
    let result=status(db,spec)?;
    let mut h=header(db,UPLOAD_SCOPE,&spec.id)?.unwrap();h.order=now;store_header(db,UPLOAD_SCOPE,&h)?;
    Ok(result)
}

pub fn status(db: &Connection, spec: &UploadSpec) -> Result<UploadStatus> {
    let h = header(db, UPLOAD_SCOPE, &spec.id)?.context("Upload not found")?;
    ensure!(h.meta.get("upload") == Some(&serde_json::to_value(spec)?), "Upload ID was already used for different content");
    let file = h.meta.get("file").map(|v|serde_json::from_value(v.clone())).transpose()?;
    Ok(UploadStatus { offset:h.length,sealed:h.sealed,file })
}

pub fn write(db: &Connection, spec: &UploadSpec, offset: u64, bytes: &[u8]) -> Result<()> {
    writing(db)?;
    let current = status(db, spec)?;
    ensure!(!bytes.is_empty() && bytes.len() <= BLOCK_CHUNK_BYTES && offset.saturating_add(bytes.len() as u64) <= spec.length, "Invalid upload range");
    ensure!(offset <= current.offset, "Upload range has a gap");
    // Duplicated in-flight chunks are allowed only when identical. A competing
    // stream can never replace verified bytes under the same upload identity.
    if offset < current.offset || current.sealed {
        ensure!(offset + bytes.len() as u64 <= current.offset, "Upload overlap crosses its durable prefix");
        let mut at = offset;
        while at < offset + bytes.len() as u64 {
            let range = read(db, &BlockRequest { scope:UPLOAD_SCOPE.into(),id:spec.id.clone(),version:1,offset:at,follow:false })?;
            let start = (at-offset) as usize;
            let n = range.bytes.len().min(bytes.len()-start);
            ensure!(n > 0 && range.bytes[..n] == bytes[start..start+n], "Conflicting upload bytes");
            at += n as u64;
        }
        return Ok(());
    }
    let mut h = header(db, UPLOAD_SCOPE, &spec.id)?.unwrap();
    install(db, UPLOAD_SCOPE, &spec.id, 1, offset, bytes)?;
    h.length += bytes.len() as u64;
    store_header(db, UPLOAD_SCOPE, &h)
}

/// The caller hashes bounded reads outside its database lock, then seals only
/// after the declared length/digest match. Full-length inputs cannot be mutated.
pub fn seal(db: &Connection, spec: &UploadSpec, verified_hash: &str, file: Option<tau_protocol::UploadedFile>) -> Result<UploadStatus> {
    writing(db)?;
    let current = status(db, spec)?;
    ensure!(current.offset == spec.length && verified_hash == spec.hash, "Upload integrity check failed");
    ensure!(matches!(spec.purpose,UploadPurpose::File {..}) == file.is_some(), "Upload publication is incomplete");
    let mut h = header(db, UPLOAD_SCOPE, &spec.id)?.unwrap();
    h.sealed = true;
    if let Some(file) = file {
        h.meta["file"] = serde_json::to_value(file)?;
        // The fsynced atomic export owns attachment bytes after publication.
        db.execute("DELETE FROM block_parts WHERE scope=?1 AND id=?2",params![UPLOAD_SCOPE,spec.id])?;
    }
    store_header(db, UPLOAD_SCOPE, &h)?;
    status(db,spec)
}

pub fn input(db: &Connection, reference: &ContentRef) -> Result<Vec<u8>> {
    ensure!(reference.scope == UPLOAD_SCOPE && reference.lineage == cursor(db)?.lineage, "Input belongs to a different source");
    let h = header(db, UPLOAD_SCOPE, &reference.id)?.context("Input is unavailable; upload it first")?;
    let spec: UploadSpec = serde_json::from_value(h.meta["upload"].clone())?;
    ensure!(h.sealed && spec.purpose == UploadPurpose::Command && reference.length == h.length && reference.hash == spec.hash, "Input is incomplete or does not match its reference");
    let bytes = cached_content(db, UPLOAD_SCOPE, &reference.id)?;
    ensure!(bytes.len() as u64 == reference.length && blake3::hash(&bytes).to_hex().as_str() == reference.hash, "Input integrity check failed");
    Ok(bytes)
}
