//! Native display projection. Tools are small blocks with separately addressed
//! input/output children. Source events remain provider history, never the wire.
use anyhow::{Context, Result, ensure};
use futures_util::FutureExt;
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use tau_blocks::{BlockHeader, BlockKind};
use tau_protocol::{Event, EventKind, EventPhase, EventRole, QueueState};
use crate::{manager::AgentManager, state::StateStore};

pub const QUEUE: &str = "@queue";
pub fn input_id(event: &str) -> String { format!("{event}/input") }
pub fn file_id(entry: &str) -> String { format!("file:{entry}") }

fn header(id: String, parent: Option<String>, order: u64, kind: BlockKind, meta: Value, sealed: bool) -> BlockHeader {
    BlockHeader { id,parent,order,kind,meta,version:0,length:0,sealed,revision:0 }
}

pub fn event(db: &Connection, session: &str, value: &Event) -> Result<()> {
    let mut meta = value.clone(); meta.text.clear();
    // Tool result children link to the original call. Imported orphan results
    // remain root blocks; no missing parent is fabricated from provider IDs.
    let parent = if value.role == EventRole::Tool {
        value.tool_call_id.as_ref().map(|call| {
            db.query_row("SELECT id FROM blocks WHERE scope=?1 AND json_extract(header,'$.kind')='tool'
                AND json_extract(header,'$.meta.event.toolCallId')=?2 ORDER BY position DESC LIMIT 1",params![session,call],|r|r.get::<_,String>(0))
                .optional()
        }).transpose()?.flatten()
    } else { None };
    let kind = match value.kind {
        EventKind::Tool => BlockKind::Tool,
        EventKind::Thinking => BlockKind::Thinking,
        EventKind::Image => BlockKind::Image,
        _ if value.role == EventRole::Tool => BlockKind::Code,
        _ => BlockKind::Text,
    };
    let sealed = value.phase != EventPhase::Live;
    let h = header(value.id.clone(),parent.clone(),value.order*2,kind,json!({"event":meta}),sealed);
    if value.kind == EventKind::Tool {
        // The card itself carries no argument bytes, even while they stream.
        tau_blocks::put(db,session,h,b"")?;
        let input = header(input_id(&value.id),Some(value.id.clone()),0,BlockKind::Code,
            json!({"inputFor":value.id,"language":"json","label":"Input"}),sealed);
        tau_blocks::put(db,session,input,value.text.as_bytes())?;
    } else {
        tau_blocks::put(db,session,h,value.text.as_bytes())?;
    }
    if let Some(parent) = parent {
        // A collapsed tool can show completion/error without downloading output.
        if let Some(mut call) = tau_blocks::header(db,session,&parent)? {
            call.meta["event"]["isError"] = json!(value.is_error);
            call.meta["toolState"] = json!(if value.is_error { "failed" } else { "completed" });
            tau_blocks::put(db,session,call,b"")?;
        }
    }
    if let Some(attachment) = &value.attachment {
        let id = file_id(&value.entry_id);
        if tau_blocks::header(db,session,&id)?.is_none() {
            let h = header(id,Some(value.id.clone()),1,match attachment.kind {
                tau_protocol::AttachmentKind::Image => BlockKind::Image,
                tau_protocol::AttachmentKind::File => BlockKind::File,
            },json!({"attachment":attachment,"entry":value.entry_id,"materialized":false}),false);
            tau_blocks::put(db,session,h,b"")?;
        }
    }
    Ok(())
}
use rusqlite::OptionalExtension;

pub fn queue(db: &Connection, session: &str, value: &QueueState) -> Result<()> {
    let mut state = value.clone(); state.requests.clear();
    let root = header(QUEUE.into(),None,i64::MAX as u64-1,BlockKind::Queue,json!({"queue":true}),true);
    tau_blocks::put(db,session,root,&serde_json::to_vec(&state)?)?;
    let ids = value.requests.iter().map(|r|format!("queued:{}",r.request_id)).collect::<Vec<_>>();
    for old in tau_blocks::children(db,session,Some(QUEUE))? {
        if !ids.contains(&old.id) { tau_blocks::remove(db,session,&old.id)?; }
    }
    for (i,request) in value.requests.iter().enumerate() {
        let mut meta = request.clone(); meta.text.clear();
        let h = header(ids[i].clone(),Some(QUEUE.into()),i as u64,BlockKind::Text,json!({"request":meta}),true);
        tau_blocks::put(db,session,h,request.text.as_bytes())?;
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
    tau_blocks::discard_staging(db)?;
    let mut q = db.prepare("SELECT scope,header FROM blocks WHERE json_extract(header,'$.meta.event.phase')='live'
        OR (json_extract(header,'$.kind')='tool' AND json_extract(header,'$.meta.toolState') IS NULL)")?;
    let rows = q.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
    for (scope,raw) in rows {
        let mut h: BlockHeader = serde_json::from_str(&raw)?;
        h.meta["event"]["phase"] = json!("interrupted"); h.sealed = true;
        if h.kind == BlockKind::Tool {
            h.meta["toolState"] = json!("interrupted");
            h.meta["event"]["errorMessage"] = json!("Interrupted; execution outcome may be unknown");
            if let Some(mut input) = tau_blocks::header(db,&scope,&input_id(&h.id))? { input.sealed = true; tau_blocks::set_header(db,&scope,input)?; }
        }
        tau_blocks::set_header(db,&scope,h)?;
    }
    let mut q = db.prepare("SELECT id,data,queue FROM sessions")?;
    let rows = q.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
    for (id,raw,state) in rows {
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
    pub async fn block_cursor(&self) -> Result<tau_blocks::FeedCursor> { self.access(|db|tau_blocks::cursor(db)).await }
    pub async fn project_live(&self, session: &str, values: Vec<Event>, removed: Vec<String>) -> Result<()> {
        let session = session.to_owned();
        self.access(move |db| {
            let tx = db.transaction()?;
            for id in removed { tau_blocks::remove(&tx,&session,&id)?; }
            let next = values.iter().map(|e|e.order+1).max().unwrap_or(0);
            for value in values { event(&tx,&session,&value)?; }
            // Reserving display positions also survives a crash before the
            // provider's final history entry has been committed.
            tx.execute("UPDATE sessions SET data=json_set(data,'$.next_order',?2) WHERE id=?1 AND json_extract(data,'$.next_order')<?2",params![session,next])?;
            tx.commit()?; Ok(())
        }).await
    }
}

struct StagedFile { state:StateStore, id:String }
impl Drop for StagedFile {
    fn drop(&mut self) {
        // Cancellation may happen while the QUIC stream is being dropped. Any
        // interrupted cleanup is also performed transactionally at startup.
        let state=self.state.clone(); let id=self.id.clone();
        tokio::spawn(async move { let _=state.access(move |db| {let tx=db.transaction()?; tau_blocks::discard_stage(&tx,&id)?;tx.commit()?;Ok(())}).await; });
    }
}
impl AgentManager {
    async fn materialize_file(&self, scope: &str, id: &str) -> Result<()> {
        let scope=scope.to_owned(); let id=id.to_owned();
        let read_header = || {let scope=scope.clone(); let id=id.clone(); self.inner.state.access(move |db|tau_blocks::header(db,&scope,&id))};
        let Some(h)=read_header().await? else {return Ok(());};
        if h.meta.get("materialized") != Some(&json!(false)) {return Ok(());}
        let _permit=self.inner.block_imports.acquire().await?;
        let Some(h)=read_header().await? else {return Ok(());};
        if h.meta.get("materialized") != Some(&json!(false)) {return Ok(());}
        let entry=h.meta["entry"].as_str().context("File has no source reference")?;
        let attachment=self.resolve_attachment(&scope,entry).await?;
        let mut file=attachment.file;
        let before=file.metadata().await?;
        ensure!(before.len() <= tau_blocks::MAX_BLOCK_BYTES,"File is too large");
        let staging=StagedFile {state:self.inner.state.clone(),id:uuid::Uuid::new_v4().to_string()};
        let mut offset=0; let mut buffer=vec![0; tau_blocks::BLOCK_CHUNK_BYTES*8];
        use sha2::Digest;
        use tokio::io::AsyncReadExt;
        let mut hash=sha2::Sha256::new();
        loop {
            let n=file.read(&mut buffer).await?;
            let bytes=buffer[..n].to_vec(); let stage=staging.id.clone();
            self.inner.state.access(move |db| {let tx=db.transaction()?;tau_blocks::stage_append(&tx,&stage,offset,&bytes)?;tx.commit()?;Ok(())}).await?;
            hash.update(&buffer[..n]); offset+=n as u64;
            if n==0 {break;}
        }
        let after=file.metadata().await?;
        ensure!(offset==before.len() && before.len()==after.len() && before.modified()?==after.modified()?,"File changed while importing");
        let mut meta=h.meta.clone(); meta["materialized"]=json!(true);meta["sha256"]=json!(format!("{:x}",hash.finalize()));
        let stage=staging.id.clone();
        self.inner.state.access(move |db| {
            let tx=db.transaction()?;
            let current=tau_blocks::header(&tx,&scope,&id)?.context("File no longer exists")?;
            if current.meta.get("materialized") == Some(&json!(false)) {tau_blocks::publish_stage(&tx,&stage,&scope,&id,meta)?;}
            else {tau_blocks::discard_stage(&tx,&stage)?;}
            tx.commit()?;Ok(())
        }).await
    }
}

impl tau_transfer::blocks::Backend for AgentManager {
    fn feed(&self, request: tau_blocks::FeedRequest) -> futures_util::future::BoxFuture<'static,Result<tau_blocks::FeedPage>> {
        let state = self.inner.state.clone();
        async move {
            state.access(move |db| {
                ensure!(db.query_row("SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1)",[&request.scope],|r|r.get::<_,bool>(0))?,"Chat no longer exists");
                tau_blocks::feed(db,&request)
            }).await
        }.boxed()
    }
    fn read(&self, request: tau_blocks::BlockRequest) -> futures_util::future::BoxFuture<'static,Result<tau_blocks::ContentRange>> {
        let manager = self.clone();
        async move {
            let req=request.clone();
            let ready=manager.inner.state.access(move |db| {
                ensure!(db.query_row("SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1)",[&req.scope],|r|r.get::<_,bool>(0))?,"Chat no longer exists");
                let h=tau_blocks::header(db,&req.scope,&req.id)?.context("Unknown block")?;
                if h.meta.get("materialized")==Some(&json!(false)) {Ok(None)} else {tau_blocks::read(db,&req).map(Some)}
            }).await?;
            if let Some(range)=ready {return Ok(range);}
            manager.materialize_file(&request.scope,&request.id).await?;
            manager.inner.state.access(move |db| tau_blocks::read(db,&request)).await
        }.boxed()
    }
    fn changes(&self) -> tokio::sync::watch::Receiver<u64> { self.inner.state.block_changes.subscribe() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::EventProjection;
    #[test]
    fn tool_projection_keeps_raw_streamed_input_and_seals_without_replacement() {
        let mut db=Connection::open_in_memory().unwrap();db.execute_batch("PRAGMA foreign_keys=ON").unwrap();tau_blocks::initialize(&db).unwrap();
        let raw=format!("{{\"command\":\"{}\"}}","x".repeat(40000));
        let stream="stable-stream";
        let project=|raw:&str,saved:bool| {
            let message=json!({"role":"assistant","content":[{"type":"toolCall","id":"call","name":"bash","partialArguments":raw,"arguments":serde_json::from_str::<Value>(raw).unwrap_or(Value::Null)}]});
            let entry=if saved {json!({"type":"message","id":"saved-entry","origin":{"streamId":stream},"message":message})}
                else {json!({"streamId":stream,"message":message})};
            crate::transcript::Event::from_entry(&entry,!saved).unwrap().remove(0)
        };
        for prefix in [100,10000,raw.len()] {let tx=db.transaction().unwrap();event(&tx,"chat",&project(&raw[..prefix],false)).unwrap();tx.commit().unwrap();}
        let finished=project(&raw,true);let input=input_id(&finished.id);let before=tau_blocks::header(&db,"chat",&input).unwrap().unwrap();
        let tx=db.transaction().unwrap();event(&tx,"chat",&finished).unwrap();tx.commit().unwrap();
        let range=tau_blocks::read(&db,&tau_blocks::BlockRequest {scope:"chat".into(),id:input,version:before.version,offset:before.length,follow:false}).unwrap();
        assert!(range.bytes.is_empty());assert!(range.header.sealed);assert_eq!(range.header.version,before.version);
        let card=tau_blocks::header(&db,"chat",&finished.id).unwrap().unwrap();assert_eq!(card.length,0);
        assert!(serde_json::to_vec(&card).unwrap().len()<2048);assert_eq!(tau_blocks::children(&db,"chat",Some(&card.id)).unwrap().len(),1);
    }
}
