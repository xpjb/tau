//! Native display projection. Tools are small blocks with separately addressed
//! input/output children. Source events remain provider history, never the wire.
use anyhow::Result;
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use tau_net::blocks::{BlockHeader, BlockKind, ToolBody, ToolState};
use tau_net::{Event, EventKind, EventPhase, EventRole, QueueState};
use crate::state::StateStore;

pub const QUEUE: &str = "@queue";
pub use tau_net::blocks::tool_input_id as input_id;
pub fn file_id(entry: &str) -> String { format!("file:{entry}") }

fn header(id: String, parent: Option<String>, order: u64, kind: BlockKind, meta: Value, sealed: bool) -> BlockHeader {
    BlockHeader { id,parent,order,kind,meta,version:0,length:0,sealed,revision:0 }
}

pub fn event(db: &Connection, session: &str, value: &Event) -> Result<()> {event_inner(db,session,value,None)}
fn event_inner(db:&Connection,session:&str,value:&Event,append_from:Option<usize>)->Result<()> {
    let mut meta = value.clone(); meta.text.clear();
    let full_meta = serde_json::to_vec(&meta)?;
    let overflow = full_meta.len() > 2048;
    let call_key = |call: &str| if call.len()>128 {format!("hash:{}",blake3::hash(call.as_bytes()).to_hex())} else {call.into()};
    meta.tool_call_id = meta.tool_call_id.as_deref().map(call_key);
    if overflow {
        let short=|value:&mut Option<String>| {if let Some(s)=value {let mut n=s.len().min(64);while !s.is_char_boundary(n) {n-=1;}s.truncate(n);}};
        short(&mut meta.tool_name);short(&mut meta.error_message);short(&mut meta.stop_reason);short(&mut meta.timestamp);
        if let Some(attachment)=&mut meta.attachment {
            short(&mut attachment.caption);attachment.source_path=None;
            let mut n=attachment.file_name.len().min(64);while !attachment.file_name.is_char_boundary(n) {n-=1;}attachment.file_name.truncate(n);
        }
    }
    // Tool result children link to the original call. Imported orphan results
    // remain root blocks; no missing parent is fabricated from provider IDs.
    // Delivered attachment cards stay in the root feed even when their tool is
    // collapsed; their binary payload remains a separately requested child.
    let call_parent = if value.role == EventRole::Tool {
        value.tool_call_id.as_ref().map(|call| {
            db.query_row("SELECT id FROM blocks WHERE scope=?1 AND json_extract(header,'$.kind')='tool'
                AND json_extract(header,'$.meta.event.toolCallId')=?2 AND position<=?3 ORDER BY position DESC LIMIT 1",params![session,call_key(call),value.order*2],|r|r.get::<_,String>(0))
                .optional()
        }).transpose()?.flatten()
    } else { None };
    let parent=if value.attachment.is_none() {call_parent.clone()} else {None};
    let kind = match value.kind {
        EventKind::Tool => BlockKind::Tool,
        EventKind::Thinking => BlockKind::Thinking,
        EventKind::Image => BlockKind::Image,
        _ if value.role == EventRole::Tool => BlockKind::Code,
        _ => BlockKind::Text,
    };
    let sealed = value.phase != EventPhase::Live;
    let write=|mut h:BlockHeader,bytes:&[u8]|->Result<()> {
        if let Some(offset)=append_from && let Some(old)=tau_block_store::header(db,session,&h.id)?
            && !old.sealed && old.length==offset as u64 && bytes.len()>=offset {
            let appended=tau_block_store::append(db,session,&h.id,old.version,old.length,&bytes[offset..],h.sealed)?;
            h.version=appended.version;h.length=appended.length;h.revision=appended.revision;
            tau_block_store::set_header(db,session,h)?;
        } else {tau_block_store::put(db,session,h,bytes)?;}
        Ok(())
    };
    let mut attributes=json!({"event":meta});
    // Authenticated metadata certifies an exact body without echoing that body
    // back to its author. Additive metadata: older replicas may still fetch it.
    if value.role == EventRole::User && value.kind == EventKind::Text && sealed {
        attributes["bodyHash"] = json!(blake3::hash(value.text.as_bytes()).to_hex().to_string());
    }
    if overflow {attributes["fullEvent"]=json!({"id":format!("{}/meta",value.id),"hash":blake3::hash(&full_meta).to_hex().to_string(),"length":full_meta.len()});}
    let h = header(value.id.clone(),parent.clone(),value.order*2,kind,attributes,sealed);
    if value.kind == EventKind::Tool {
        // The card itself carries no argument bytes, even while they stream.
        tau_block_store::put(db,session,h,b"")?;
        let input = header(input_id(&value.id),Some(value.id.clone()),0,BlockKind::Code,
            json!({"inputFor":value.id,"language":"json","label":ToolBody::Input.label()}),sealed);
        write(input,value.text.as_bytes())?;
    } else {
        write(h,value.text.as_bytes())?;
    }
    if overflow {
        let metadata=header(format!("{}/meta",value.id),Some(value.id.clone()),2,BlockKind::State,json!({"eventMetadata":true}),true);
        tau_block_store::put(db,session,metadata,&full_meta)?;
    }
    if let Some(parent) = call_parent {
        // A collapsed tool can show completion/error without downloading output.
        if let Some(mut call) = tau_block_store::header(db,session,&parent)? {
            call.meta["event"]["isError"] = json!(value.is_error);
            call.meta["toolState"] = json!(if value.is_error { ToolState::Failed } else { ToolState::Completed });
            tau_block_store::put(db,session,call,b"")?;
        }
    }
    if let Some(attachment) = &value.attachment {
        let id = file_id(&value.entry_id);
        if tau_block_store::header(db,session,&id)?.is_none() {
            let h = header(id,Some(value.id.clone()),1,match attachment.kind {
                tau_net::AttachmentKind::Image => BlockKind::Image,
                tau_net::AttachmentKind::File => BlockKind::File,
            },json!({"attachment":{"kind":attachment.kind,"size":attachment.size},"entry":value.entry_id,"materialized":false}),false);
            tau_block_store::put(db,session,h,b"")?;
        }
    }
    Ok(())
}
use rusqlite::OptionalExtension;

pub fn queue(db: &Connection, session: &str, value: &QueueState) -> Result<()> {
    let mut state = value.clone(); state.requests.clear();
    // A queue membership change must also advance its root directory. This
    // lets replicas order a queue removal against the corresponding history
    // insertion, even when those two directory pages arrive out of order.
    let members = value.requests.iter().map(|r| (&r.request_id, r.revision)).collect::<Vec<_>>();
    let membership = blake3::hash(&serde_json::to_vec(&members)?).to_hex().to_string();
    let root = header(QUEUE.into(),None,i64::MAX as u64-1,BlockKind::Queue,json!({"queue":true,"membershipHash":membership}),true);
    tau_block_store::put(db,session,root,&serde_json::to_vec(&state)?)?;
    let ids = value.requests.iter().map(|r|format!("queued:{}",r.request_id)).collect::<Vec<_>>();
    for old in tau_block_store::children(db,session,Some(QUEUE))? {
        if !ids.contains(&old.id) { tau_block_store::remove(db,session,&old.id)?; }
    }
    for (i,request) in value.requests.iter().enumerate() {
        let mut meta = request.clone(); meta.text.clear();
        let h = header(ids[i].clone(),Some(QUEUE.into()),i as u64,BlockKind::Text,json!({"request":meta,"bodyHash":blake3::hash(request.text.as_bytes()).to_hex().to_string()}),true);
        tau_block_store::put(db,session,h,request.text.as_bytes())?;
    }
    Ok(())
}

pub fn project_existing(db: &Connection) -> Result<()> {
    // Bounded per-entry migration; provider-private messages/image bytes are not
    // in this table and can never leak into the block projection.
    let mut events = db.prepare("SELECT session_id,data FROM events ORDER BY session_id,position")?;
    let mut rows = events.query([])?;
    while let Some(row) = rows.next()? {
        let session: String = row.get(0)?; let raw: String = row.get(1)?;
        event(db,&session,&serde_json::from_str(&raw)?)?;
    }
    let mut sessions = db.prepare("SELECT id,queue FROM sessions")?;
    let mut rows = sessions.query([])?;
    while let Some(row) = rows.next()? {
        let id: String = row.get(0)?; let raw: String = row.get(1)?;
        let mut value: QueueState = serde_json::from_str(&raw)?;
        let mut requests = db.prepare("SELECT data FROM queue WHERE session_id=?1 ORDER BY position")?;
        value.requests = requests.query_map([&id],|r|r.get::<_,String>(0))?
            .map(|raw| Ok(serde_json::from_str(&raw?)?)).collect::<Result<_>>()?;
        queue(db,&id,&value)?;
    }
    Ok(())
}

/// Recovery is a source transaction, not a per-viewer reset. Interrupted content
/// keeps its IDs, chunks and offsets, and no paid work is restarted by viewing it.
fn recover(db: &Connection) -> Result<()> {
    tau_block_store::discard_staging(db)?;
    let mut after=(String::new(),String::new());
    loop {
        let mut q=db.prepare("SELECT scope,id,header FROM blocks WHERE (scope,id)>(?1,?2) AND (json_extract(header,'$.meta.event.phase')='live' OR (json_extract(header,'$.kind')='tool' AND json_extract(header,'$.meta.toolState') IS NULL)) ORDER BY scope,id LIMIT 64")?;
        let rows=q.query_map(params![after.0,after.1],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        if rows.is_empty() {break;}
        for (scope,id,raw) in rows {after=(scope.clone(),id);
        let mut h: BlockHeader = serde_json::from_str(&raw)?;
        h.meta["event"]["phase"] = json!("interrupted"); h.sealed = true;
        if h.kind == BlockKind::Tool {
            h.meta["toolState"] = json!(ToolState::Interrupted);
            h.meta["event"]["errorMessage"] = json!("Interrupted; execution outcome may be unknown");
            if let Some(mut input) = tau_block_store::header(db,&scope,&input_id(&h.id))? { input.sealed = true; tau_block_store::set_header(db,&scope,input)?; }
        }
        tau_block_store::set_header(db,&scope,h)?;
        }
    }
    let mut after=String::new();
    loop {
        let mut q=db.prepare("SELECT id,data,queue FROM sessions WHERE id>?1 ORDER BY id LIMIT 32")?;
        let rows=q.query_map([&after],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        if rows.is_empty() {break;}
        for (id,raw,state) in rows {after=id.clone();
        let stored: crate::state::StoredSession = serde_json::from_str(&raw)?;
        let mut value: QueueState = serde_json::from_str(&state)?;
        value.run_id = None;
        if let Some(control) = &mut value.control && control.status == "waiting" {
            control.status = "failed".into(); control.detail = Some("Interrupted by daemon restart".into());
        }
        let mut requests = db.prepare("SELECT data FROM queue WHERE session_id=?1 ORDER BY position")?;
        value.requests = requests.query_map([&id],|r|r.get::<_,String>(0))?
            .map(|raw|Ok(serde_json::from_str(&raw?)?)).collect::<Result<_>>()?;
        if stored.needs_turn || !value.requests.is_empty() { value.paused = true; }
        queue(db,&id,&value)?;
        value.requests.clear();
        let encoded = serde_json::to_string(&value)?;
        if encoded != state { db.execute("UPDATE sessions SET queue=?2 WHERE id=?1",params![id,encoded])?; }
        }
    }
    // An accepted built-in is never replayed after a crash. Preserve its outcome
    // explicitly so reconnect can distinguish it from an absent receipt.
    db.execute("UPDATE receipts SET data=json_set(data,'$.finished',json('true'),'$.error',
        'Command interrupted by daemon restart; inspect effects before issuing a new operation') WHERE json_extract(data,'$.finished')=0",[])?;
    Ok(())
}

impl StateStore {
    pub(crate) async fn recover_blocks(&self) -> Result<()> {
        self.access(|db| {let tx=db.transaction()?; recover(&tx)?; tx.commit()?; Ok(())}).await
    }
    pub async fn block_cursor(&self) -> Result<tau_net::blocks::FeedCursor> { self.read(|db|tau_block_store::cursor(db)).await }
    pub async fn project_live(&self, session: &str, values: Vec<(Event,Option<usize>)>, removed: Vec<String>) -> Result<()> {
        let session = session.to_owned();
        self.access(move |db| {
            let tx = db.transaction()?;
            for id in removed { tau_block_store::remove(&tx,&session,&id)?; }
            let next = values.iter().map(|(e,_)|e.order+1).max().unwrap_or(0);
            for (value,append_from) in values { event_inner(&tx,&session,&value,append_from)?; }
            // Reserving display positions also survives a crash before the
            // provider's final history entry has been committed.
            tx.execute("UPDATE sessions SET data=json_set(data,'$.next_order',?2) WHERE id=?1 AND json_extract(data,'$.next_order')<?2",params![session,next])?;
            tx.commit()?; Ok(())
        }).await
    }
}


#[cfg(test)]
#[path = "../tests/unit/blocks.rs"]
mod tests;
