use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use crate::protocol::PromptDisposition;
use crate::transcript::{EventProjection, Event, HistoryPage, QueueState, PAGE_EVENTS, PAGE_BYTES};

pub(crate) use tau_protocol::settings::DEFAULT_TITLE_PROMPT;
pub(crate) const MAX_FLAG_CHARS: usize = 4096;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Flag { pub id: String, pub timestamp_ms: u64, pub session_id: String, pub session_title: String, pub text: String }

pub use tau_protocol::SessionModel;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct StoredSession {
    pub title: String,
    #[serde(default = "tau_protocol::general_project_id")]
    pub project_id: String,
    #[serde(default)]
    pub project_prompt: String,
    pub starter: bool,
    pub parent_id: Option<String>,
    pub model: SessionModel,
    pub thinking: String,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub tokens: Option<u64>,
    pub needs_turn: bool,
    pub head: Option<String>,
    pub next_order: u64,
    pub revision: u64,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Receipt {
    pub id: String,
    #[serde(default)] pub command: Option<String>,
    pub text: String,
    pub disposition: PromptDisposition,
    pub finished: bool,
    pub notice: Option<String>,
    pub error: Option<String>,
}
#[derive(Clone)]
pub struct StateStore { connection: Arc<Mutex<Connection>>, path: PathBuf, flag_gate: Arc<tokio::sync::Mutex<()>>, pub(crate) block_changes: tokio::sync::watch::Sender<u64> }

impl StateStore {
    pub async fn load(path: PathBuf) -> Result<Self> {
        let location = path.clone();
        let connection = tokio::task::spawn_blocking(move || -> Result<Connection> {
            if let Some(parent) = location.parent() { std::fs::create_dir_all(parent)?; }
            // SQLite inherits the database's private permissions for its WAL/SHM files.
            let mut options = std::fs::OpenOptions::new(); options.create(true).truncate(false).write(true);
            #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; options.mode(0o600).custom_flags(libc::O_NOFOLLOW); }
            let file = options.open(&location)?;
            if !file.metadata()?.is_file() { bail!("Database is not a regular file"); }
            #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; file.set_permissions(std::fs::Permissions::from_mode(0o600))?; }
            let mut db = Connection::open(&location).context("Could not open Tau SQLite database")?;
            db.busy_timeout(std::time::Duration::from_secs(5))?;
            db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")?;
            let version: u32 = db.pragma_query_value(None, "user_version", |row| row.get(0))?;
            if version > 4 { bail!("Unsupported Tau database version {version}"); }
            if version == 0 {
                let tx = db.transaction()?;
                tx.execute_batch(include_str!("schema.sql"))?;
                tx.commit()?;
            }
            if version < 2 {
                let tx = db.transaction()?;
                tx.execute_batch(include_str!("schema_projects.sql"))?;
                tx.commit()?;
            }
            if version < 3 {
                let tx = db.transaction()?;
                tau_blocks::initialize(&tx)?;
                crate::blocks::project_existing(&tx)?;
                tx.execute_batch("PRAGMA user_version=3")?;
                tx.commit()?;
            }
            if version < 4 {
                let tx=db.transaction()?;
                tx.execute_batch("CREATE TABLE operations(id TEXT PRIMARY KEY,payload TEXT NOT NULL,response TEXT); CREATE INDEX receipts_request ON receipts(request_id); PRAGMA user_version=4;")?;
                tx.commit()?;
            }
            if let Some(parent) = location.parent() { std::fs::File::open(parent)?.sync_all()?; }
            Ok(db)
        }).await??;
        Ok(Self { connection:Arc::new(Mutex::new(connection)), path, flag_gate:Arc::new(tokio::sync::Mutex::new(())), block_changes:tokio::sync::watch::channel(0).0 })
    }
    pub(crate) async fn access<T: Send + 'static>(&self, action: impl FnOnce(&mut Connection) -> Result<T> + Send + 'static) -> Result<T> {
        // Wait asynchronously; don't fill the blocking pool with database lock waiters.
        let mut connection = self.connection.clone().lock_owned().await;
        let changes = self.block_changes.clone();
        tokio::task::spawn_blocking(move || {
            let before = tau_blocks::cursor(&connection)?.sequence;
            let result = action(&mut connection);
            let after = tau_blocks::cursor(&connection)?.sequence;
            if after != before { changes.send_replace(after); }
            result
        }).await?
    }
    pub async fn get(&self, id: &str) -> Result<Option<StoredSession>> {
        let id = id.to_owned();
        self.access(move |db| db.query_row("SELECT data FROM sessions WHERE id=?1", [&id], |row| row.get::<_,String>(0)).optional()?
            .map(|data| serde_json::from_str(&data).map_err(Into::into)).transpose()).await
    }
    pub async fn list(&self) -> Result<Vec<(String, StoredSession)>> {
        self.access(|db| {
            let mut query = db.prepare("SELECT id,data FROM sessions ORDER BY activity DESC,id")?;
            query.query_map([], |row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?)))?
                .map(|row| { let (id,data) = row?; Ok((id,serde_json::from_str(&data)?)) }).collect()
        }).await
    }
    #[cfg(test)]
    pub async fn create(&self, model: SessionModel, thinking: String, keep: Option<String>, project_id: String) -> Result<String> {
        self.create_requested(model, thinking, keep, project_id, None).await
    }
    pub async fn created_session(&self, request: &str, keep: Option<&str>, project: &str) -> Result<Option<String>> {
        let request = request.to_owned();
        let payload = json!({"keepSessionId":keep,"projectId":project}).to_string();
        self.access(move |db| find_created_session(db, &request, &payload)).await
    }
    pub async fn create_requested(&self, model: SessionModel, thinking: String, keep: Option<String>, project_id: String, requested_id: Option<String>) -> Result<String> {
        self.access(move |db| {
            let tx = db.transaction()?;
            let keep_receipt = json!({"keepSessionId":keep,"projectId":project_id}).to_string();
            if let Some(id) = &requested_id {
                uuid::Uuid::parse_str(id).context("Invalid create ID")?;
                if let Some(session) = find_created_session(&tx, id, &keep_receipt)? {
                    tx.commit()?; return Ok(session);
                }
                if tx.query_row("SELECT id FROM sessions WHERE id=?1",[id],|row|row.get::<_,String>(0)).optional()?.is_some() {
                    bail!("Create request ID is already in use");
                }
            }
            let project_prompt: String = tx.query_row("SELECT prompt FROM projects WHERE id=?1", [&project_id], |r| r.get(0)).context("Unknown topic")?;
            if let Some(id) = keep {
                let data: String = tx.query_row("SELECT data FROM sessions WHERE id=?1", [&id], |row| row.get(0)).context("Unknown session")?;
                let mut session: StoredSession = serde_json::from_str(&data)?; session.starter = false;
                tx.execute("UPDATE sessions SET starter=0,data=?2 WHERE id=?1", params![id,serde_json::to_string(&session)?])?;
            }
            // Reuse only a starter in the requested topic with the same captured
            // instructions; never replace an older chat or its local draft.
            tx.execute("UPDATE sessions SET starter=0,data=json_set(data,'$.starter',json('false')) WHERE starter=1 AND json_extract(data,'$.project_id')=?1
                AND coalesce(json_extract(data,'$.project_prompt'),'')!=?2", params![project_id,project_prompt])?;
            if let Some(id) = tx.query_row("SELECT id FROM sessions WHERE starter=1 AND json_extract(data,'$.project_id')=?1", [&project_id], |row| row.get::<_,String>(0)).optional()? {
                if requested_id.is_none() {tx.commit()?;return Ok(id);}
                // A client-named chat keeps its exact identity. Retire the old
                // empty tile, never alias/migrate another client's local work.
                tx.execute("UPDATE sessions SET starter=0,data=json_set(data,'$.starter',json('false')) WHERE id=?1",[id])?;
            }
            let id = requested_id.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            let now = activity(&tx)?;
            let mut session = StoredSession { title:"New chat".into(), project_id, project_prompt, starter:true, parent_id:None, model:model.clone(), thinking:thinking.clone(),
                created_at_ms:now, updated_at_ms:now, tokens:None, needs_turn:false, head:None, next_order:0, revision:0 };
            let entry = json!({"id":uuid::Uuid::new_v4().to_string(),"type":"model_change","parentId":null,
                "timestamp":chrono::Utc::now().to_rfc3339(),"provider":model.provider,"modelId":model.model_id,"thinkingLevel":thinking});
            session.head = entry["id"].as_str().map(str::to_owned);
            tx.execute("INSERT INTO sessions(id,starter,activity,data,queue) VALUES(?1,1,?2,?3,?4)",
                params![id,now,serde_json::to_string(&session)?,serde_json::to_string(&QueueState::native())?])?;
            tx.execute("INSERT INTO entries(session_id,id,kind,data) VALUES(?1,?2,'model_change',?3)",params![id,entry["id"].as_str(),entry.to_string()])?;
            if let Some(request) = requested_id {
                let receipt = Receipt { id:request.clone(), command:Some("create_session".into()), text:keep_receipt,
                    disposition:PromptDisposition::Handled, finished:true, notice:None, error:None };
                tx.execute("INSERT INTO receipts(session_id,request_id,data) VALUES(?1,?2,?3)",params![id,request,serde_json::to_string(&receipt)?])?;
            }
            crate::blocks::queue(&tx,&id,&QueueState::native())?;
            tx.commit()?; Ok(id)
        }).await
    }
    pub async fn queue(&self, id: &str) -> Result<QueueState> {
        let id = id.to_owned();
        self.access(move |db| {
            let raw: String = db.query_row("SELECT queue FROM sessions WHERE id=?1", [&id], |row| row.get(0))?;
            let mut queue: QueueState = serde_json::from_str(&raw)?;
            let mut query = db.prepare("SELECT data FROM queue WHERE session_id=?1 ORDER BY position")?;
            queue.requests = query.query_map([&id], |row| row.get::<_,String>(0))?.map(|raw| Ok(serde_json::from_str(&raw?)?)).collect::<Result<_>>()?;
            Ok(queue)
        }).await
    }
    pub async fn receipt(&self, id: &str, request: &str) -> Result<Option<Receipt>> {
        let id = id.to_owned(); let request = request.to_owned();
        self.access(move |db| db.query_row("SELECT data FROM receipts WHERE session_id=?1 AND request_id=?2", params![id,request], |row| row.get::<_,String>(0)).optional()?
            .map(|data| serde_json::from_str(&data).map_err(Into::into)).transpose()).await
    }
    // Commit before publishing any saved transcript/queue change. The revision is
    // a database CAS, not another transcript event sequence.
    pub async fn commit(&self, id: &str, revision: u64, entries: Vec<Value>, events: Vec<Event>, queue: Option<QueueState>, receipt: Option<Receipt>) -> Result<StoredSession> {
        let id = id.to_owned();
        self.access(move |db| {
            let tx = db.transaction()?;
            let raw: String = tx.query_row("SELECT data FROM sessions WHERE id=?1", [&id], |row| row.get(0)).context("Unknown session")?;
            let mut session: StoredSession = serde_json::from_str(&raw)?;
            if session.revision != revision { bail!("Session changed in another writer; close and reopen it"); }
            if let Some(receipt) = receipt {
                anyhow::ensure!(!tx.query_row("SELECT EXISTS(SELECT 1 FROM operations WHERE id=?1)",[&receipt.id],|r|r.get::<_,bool>(0))?,"Operation ID belongs to another control mutation");
                tx.execute("INSERT INTO receipts(session_id,request_id,data) VALUES(?1,?2,?3)",params![id,receipt.id,serde_json::to_string(&receipt)?])?;
                if !matches!(receipt.disposition, PromptDisposition::Handled) { session.starter = false; }
            }
            if let Some(mut queue) = queue {
                crate::blocks::queue(&tx,&id,&queue)?;
                if queue.requests.len() > 256 || queue.requests.iter().map(|r| r.text.len()).sum::<usize>() > 4 * 1024 * 1024 { bail!("Pending queue exceeds its limit"); }
                let ids = queue.requests.iter().map(|r| &r.request_id).collect::<Vec<_>>();
                tx.execute("DELETE FROM queue WHERE session_id=?1 AND request_id NOT IN (SELECT value FROM json_each(?2))",params![id,serde_json::to_string(&ids)?])?;
                for (position, request) in queue.requests.drain(..).enumerate() {
                    tx.execute("INSERT INTO queue(session_id,request_id,position,data) VALUES(?1,?2,?3,?4)
                        ON CONFLICT(session_id,request_id) DO UPDATE SET position=excluded.position,data=excluded.data
                        WHERE position!=excluded.position OR data!=excluded.data",params![id,request.request_id,position,serde_json::to_string(&request)?])?;
                }
                tx.execute("UPDATE sessions SET queue=?2 WHERE id=?1",params![id,serde_json::to_string(&queue)?])?;
            }
            for entry in entries {
                if entry["parentId"].as_str() != session.head.as_deref() { bail!("History head changed"); }
                let entry_id = entry["id"].as_str().context("History entry has no ID")?;
                let kind = entry["type"].as_str().context("History entry has no type")?;
                apply_entry(&mut session,&entry)?;
                tx.execute("INSERT INTO entries(session_id,id,kind,data) VALUES(?1,?2,?3,?4)",params![id,entry_id,kind,entry.to_string()])?;
                session.head = Some(entry_id.into());
            }
            for event in events {
                crate::blocks::event(&tx,&id,&event)?;
                session.next_order = session.next_order.max(event.order + 1);
                tx.execute("INSERT INTO events(session_id,position,id,entry_id,data) VALUES(?1,?2,?3,?4,?5)",
                    params![id,event.order,event.id,event.entry_id,serde_json::to_string(&event)?])?;
            }
            session.revision += 1;
            session.updated_at_ms = activity(&tx)?;
            tx.execute("UPDATE sessions SET starter=?2,activity=?3,data=?4 WHERE id=?1",params![id,session.starter,session.updated_at_ms,serde_json::to_string(&session)?])?;
            tx.commit()?; Ok(session)
        }).await
    }
    pub async fn finish_command(&self, id: &str, mut receipt: Receipt, result: &Result<crate::manager::PromptOutcome>) -> Result<()> {
        receipt.finished = true;
        match result { Ok(outcome) => receipt.notice = outcome.notice.clone(), Err(error) => receipt.error = Some(error.to_string()) }
        let id = id.to_owned();
        self.access(move |db| {
            if db.execute("UPDATE receipts SET data=?3 WHERE session_id=?1 AND request_id=?2",params![id,receipt.id,serde_json::to_string(&receipt)?])? != 1 { bail!("Command receipt disappeared"); }
            Ok(())
        }).await
    }
    pub async fn rename(&self, id: &str, title: String, only_untitled: bool) -> Result<()> {
        let id = id.to_owned();
        self.access(move |db| {
            let tx = db.transaction()?;
            let data: String = tx.query_row("SELECT data FROM sessions WHERE id=?1",[&id],|row| row.get(0)).context("Unknown session")?;
            let mut session: StoredSession = serde_json::from_str(&data)?;
            if !only_untitled || session.title == "New chat" {
                session.title = title; session.starter = false; session.updated_at_ms = activity(&tx)?;
                tx.execute("UPDATE sessions SET starter=0,activity=?2,data=?3 WHERE id=?1",params![id,session.updated_at_ms,serde_json::to_string(&session)?])?;
            }
            tx.commit()?; Ok(())
        }).await
    }
    pub async fn entry(&self, id: &str, entry_id: &str) -> Result<Value> {
        let id = id.to_owned(); let entry_id = entry_id.to_owned();
        self.access(move |db| {
            let data: String = db.query_row("SELECT data FROM entries WHERE session_id=?1 AND id=?2",params![id,entry_id],|row| row.get(0)).context("History entry does not exist")?;
            Ok(serde_json::from_str(&data)?)
        }).await
    }
    pub async fn page(&self, id: &str, before: Option<u64>) -> Result<HistoryPage> {
        let id = id.to_owned();
        self.access(move |db| {
            let mut query = db.prepare("SELECT data FROM events WHERE session_id=?1 AND position<?2 ORDER BY position DESC LIMIT ?3")?;
            let mut rows = query.query(params![id,before.unwrap_or(i64::MAX as u64),PAGE_EVENTS+1])?;
            let mut events = Vec::<Event>::new(); let mut bytes = 0; let mut more = false;
            while let Some(row) = rows.next()? {
                let raw: String = row.get(0)?;
                if !events.is_empty() && (events.len() >= PAGE_EVENTS || bytes + raw.len() > PAGE_BYTES) { more = true; break; }
                bytes += raw.len(); events.push(serde_json::from_str(&raw)?);
            }
            events.reverse(); Ok(HistoryPage { before:if more { events.first().map(|event| event.order) } else {None}, events })
        }).await
    }
    // Only the latest applicable checkpoint and its retained suffix enter memory.
    // Display paging and attachment lookup never load provider history.
    pub async fn context(&self, id: &str, selected: &SessionModel) -> Result<Vec<Value>> {
        let id = id.to_owned(); let selected = selected.clone();
        self.access(move |db| {
            let checkpoint: Option<String> = db.query_row("SELECT data FROM entries WHERE session_id=?1 AND kind='compaction'
                AND (coalesce(json_extract(data,'$.details.kind'),'')!='codex-native-compaction'
                    OR (json_extract(data,'$.details.model')=?2 AND json_extract(data,'$.details.provider')=?3)) ORDER BY position DESC LIMIT 1",
                params![id,selected.model_id,selected.provider],|row| row.get(0)).optional()?;
            let start = if let Some(raw) = checkpoint {
                let entry: Value = serde_json::from_str(&raw)?;
                db.query_row("SELECT position FROM entries WHERE session_id=?1 AND id=?2",params![id,entry["firstKeptEntryId"].as_str().context("Compaction has no boundary")?],|row| row.get::<_,i64>(0)).context("Compaction boundary is missing")?
            } else { 0 };
            let mut query = db.prepare("SELECT data FROM entries WHERE session_id=?1 AND position>=?2 ORDER BY position")?;
            query.query_map(params![id,start],|row| row.get::<_,String>(0))?.map(|raw| Ok(serde_json::from_str(&raw?)?)).collect()
        }).await
    }
    pub async fn branch(&self, id: &str, entry_id: Option<&str>) -> Result<(String, Option<String>)> {
        let id = id.to_owned(); let entry_id = entry_id.map(str::to_owned);
        self.access(move |db| {
            let tx = db.transaction()?;
            let data: String = tx.query_row("SELECT data FROM sessions WHERE id=?1",[&id],|row| row.get(0))?;
            let mut session: StoredSession = serde_json::from_str(&data)?;
            let mut draft = None;
            let cut = if let Some(entry_id) = &entry_id {
                let (position,raw): (i64,String) = tx.query_row("SELECT position,data FROM entries WHERE session_id=?1 AND id=?2",params![id,entry_id],|row| Ok((row.get(0)?,row.get(1)?))).context("Fork entry does not exist")?;
                let entry: Value = serde_json::from_str(&raw)?;
                if entry["message"]["role"] == "user" { draft = entry["message"]["content"].as_str().map(str::to_owned); position-1 } else { position }
            } else { i64::MAX };
            let child = uuid::Uuid::new_v4().to_string();
            session.title = crate::manager::bounded(&format!("{}{}",if entry_id.is_some() {"Fork of "} else {"Copy of "},session.title),crate::protocol::MAX_TITLE_CHARS);
            session.parent_id = Some(id.clone()); session.starter = false; session.revision = 0;
            session.created_at_ms = activity(&tx)?; session.updated_at_ms = session.created_at_ms;
            tx.execute("INSERT INTO sessions(id,starter,activity,data,queue) VALUES(?1,0,?2,?3,?4)",params![child,session.updated_at_ms,serde_json::to_string(&session)?,serde_json::to_string(&QueueState::native())?])?;
            tx.execute("INSERT INTO entries(session_id,id,kind,data) SELECT ?1,id,kind,data FROM entries WHERE session_id=?2 AND position<=?3 ORDER BY position",params![child,id,cut])?;
            tx.execute("INSERT INTO events(session_id,position,id,entry_id,data) SELECT ?1,position,id,entry_id,data FROM events WHERE session_id=?2
                AND entry_id IN (SELECT id FROM entries WHERE session_id=?1)",params![child,id])?;
            // Only already-consumed user receipts belong to the copied history.
            tx.execute("INSERT INTO receipts(session_id,request_id,data) SELECT ?1,request_id,data FROM receipts WHERE session_id=?2
                AND request_id IN (SELECT json_extract(data,'$.origin.requestId') FROM entries WHERE session_id=?1 AND json_extract(data,'$.message.role')='user')",params![child,id])?;
            session.head = tx.query_row("SELECT id FROM entries WHERE session_id=?1 ORDER BY position DESC LIMIT 1",[&child],|row| row.get(0)).optional()?;
            session.next_order = tx.query_row("SELECT coalesce(max(position)+1,0) FROM events WHERE session_id=?1",[&child],|row| row.get(0))?;
            let model: String = tx.query_row("SELECT data FROM entries WHERE session_id=?1 AND kind='model_change' ORDER BY position DESC LIMIT 1",[&child],|row| row.get(0))?;
            let model: Value = serde_json::from_str(&model)?;
            session.model = SessionModel { provider:model["provider"].as_str().context("No fork provider")?.into(),model_id:model["modelId"].as_str().context("No fork model")?.into() };
            let thinking: String = tx.query_row("SELECT json_extract(data,'$.thinkingLevel') FROM entries WHERE session_id=?1 AND kind IN ('model_change','thinking_level_change') ORDER BY position DESC LIMIT 1",[&child],|row| row.get(0))?;
            session.thinking = thinking; session.tokens = None;
            let message: Option<String> = tx.query_row("SELECT json_extract(data,'$.message') FROM entries WHERE session_id=?1 AND kind='message' ORDER BY position DESC LIMIT 1",[&child],|row| row.get(0)).optional()?;
            session.needs_turn = false;
            if let Some(raw) = message { apply_entry(&mut session,&json!({"type":"message","message":serde_json::from_str::<Value>(&raw)?}))?; }
            tx.execute("UPDATE sessions SET data=?2 WHERE id=?1",params![child,serde_json::to_string(&session)?])?;
            let mut projected = tx.prepare("SELECT data FROM events WHERE session_id=?1 ORDER BY position")?;
            for raw in projected.query_map([&child],|r|r.get::<_,String>(0))? {
                crate::blocks::event(&tx,&child,&serde_json::from_str(&raw?)?)?;
            }
            drop(projected);
            crate::blocks::queue(&tx,&child,&QueueState::native())?;
            tx.commit()?; Ok((child,draft))
        }).await
    }
    pub async fn remove(&self, id: &str) -> Result<()> {
        let id = id.to_owned();
        self.access(move |db| {
            let tx = db.transaction()?;
            tau_blocks::remove_scope(&tx,&id)?;
            if tx.execute("DELETE FROM sessions WHERE id=?1",[&id])? != 1 { bail!("Unknown session"); }
            tx.execute("UPDATE sessions SET data=json_set(data,'$.parent_id',NULL) WHERE json_extract(data,'$.parent_id')=?1",[&id])?;
            tx.commit()?; Ok(())
        }).await
    }
    // Explicit, offline Tau 1/Pi import into an empty database. Native Tau 2
    // never writes JSONL, and there is no migration for its unreleased old format.
    pub async fn import_legacy(&self, state: Value, settings: crate::settings::Settings) -> Result<usize> {
        self.access(move |db| {
            use std::collections::{HashMap, HashSet};
            use std::io::BufRead;
            if state["schema"] != 1 { bail!("Unsupported legacy Tau state"); }
            let sessions = state["sessions"].as_object().context("Legacy state has no sessions")?;
            let tx = db.transaction()?;
            if tx.query_row("SELECT count(*) FROM sessions",[],|row| row.get::<_,u64>(0))? != 0 { bail!("Import requires an empty database"); }
            for (id,old) in sessions {
                uuid::Uuid::parse_str(id).context("Legacy session ID is invalid")?;
                let model = old.get("model").filter(|model| !model.is_null()).map(|model| serde_json::from_value(model.clone())).transpose()?.unwrap_or_else(|| settings.agent.model.clone());
                let thinking = settings.agent.model_thinking_levels.get(&format!("{}/{}",model.provider,model.model_id)).unwrap_or(&settings.agent.thinking_level).clone();
                let mut session = StoredSession { title:old["title"].as_str().unwrap_or("Unnamed chat").into(),project_id:tau_protocol::general_project_id(),project_prompt:String::new(),starter:false,
                    parent_id:old["parent_id"].as_str().filter(|parent| sessions.contains_key(*parent)).map(str::to_owned),model,thinking,
                    created_at_ms:old["created_at_ms"].as_u64().unwrap_or(0),updated_at_ms:old["updated_at_ms"].as_u64().unwrap_or(0),tokens:None,needs_turn:false,head:None,next_order:0,revision:0 };
                let mut records = HashMap::new(); let mut head = None;
                if let Some(path) = old["session_file"].as_str() {
                    let mut options = std::fs::OpenOptions::new(); options.read(true);
                    #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; options.custom_flags(libc::O_NONBLOCK); }
                    let file = options.open(path).context("Legacy history is unavailable")?;
                    if !file.metadata()?.is_file() { bail!("Legacy history is not a regular file"); }
                    for line in std::io::BufReader::new(file).lines() {
                        let line = line?;
                        let Ok(entry) = serde_json::from_str::<Value>(&line) else { continue; };
                        if entry["type"] == "session" { continue; }
                        if entry["type"] == "tau_queue" { bail!("This importer accepts Tau 1/Pi history, not the unreleased Tau 2 JSONL format"); }
                        let Some(id) = entry["id"].as_str() else { continue; }; let id = id.to_owned();
                        if records.insert(id.clone(),entry).is_some() { bail!("Duplicate legacy history entry ID"); }
                        head = Some(id);
                    }
                }
                let mut entries = Vec::new(); let mut seen = HashSet::new();
                while let Some(id) = head {
                    if !seen.insert(id.clone()) { bail!("Legacy branch contains a cycle"); }
                    let Some(entry) = records.remove(&id) else { break; };
                    head = entry["parentId"].as_str().map(str::to_owned); entries.push(entry);
                }
                entries.push(json!({"id":uuid::Uuid::new_v4().to_string(),"type":"model_change","provider":session.model.provider,
                    "modelId":session.model.model_id,"thinkingLevel":session.thinking}));
                entries.reverse();
                tx.execute("INSERT INTO sessions(id,starter,activity,data,queue) VALUES(?1,0,?2,?3,?4)",params![id,session.updated_at_ms,serde_json::to_string(&session)?,serde_json::to_string(&QueueState::native())?])?;
                for mut entry in entries {
                    entry["parentId"] = json!(session.head);
                    if entry["type"] == "model_change" && entry["thinkingLevel"].is_null() { entry["thinkingLevel"] = json!(session.thinking); }
                    apply_entry(&mut session,&entry)?;
                    session.head = entry["id"].as_str().map(str::to_owned);
                    tx.execute("INSERT INTO entries(session_id,id,kind,data) VALUES(?1,?2,?3,?4)",params![id,entry["id"].as_str(),entry["type"].as_str().context("Legacy entry has no type")?,entry.to_string()])?;
                    for mut event in Event::from_entry(&entry,false)? {
                        event.order = session.next_order; session.next_order += 1;
                        tx.execute("INSERT INTO events(session_id,position,id,entry_id,data) VALUES(?1,?2,?3,?4,?5)",params![id,event.order,event.id,event.entry_id,serde_json::to_string(&event)?])?;
                    }
                    if entry["message"]["role"] == "user" && let Some(request) = entry["origin"]["requestId"].as_str() {
                        let receipt = Receipt { id:request.into(),command:None,text:entry["message"]["content"].as_str().unwrap_or_default().into(),disposition:PromptDisposition::Submitted,finished:true,notice:None,error:None };
                        tx.execute("INSERT INTO receipts(session_id,request_id,data) VALUES(?1,?2,?3)",params![id,request,serde_json::to_string(&receipt)?])?;
                    }
                }
                tx.execute("UPDATE sessions SET data=?2 WHERE id=?1",params![id,serde_json::to_string(&session)?])?;
            }
            crate::blocks::project_existing(&tx)?;
            tx.commit()?; Ok(sessions.len())
        }).await
    }
    /// Portable conversation export, not a backup of pending jobs or receipts.
    pub async fn export_history(&self, id: &str, destination: &std::path::Path) -> Result<()> {
        let id=id.to_owned();
        let bytes=self.access(move |db| {
            let tx=db.transaction()?;
            let session:String=tx.query_row("SELECT data FROM sessions WHERE id=?1",[&id],|row|row.get(0)).context("Unknown session")?;
            let mut query=tx.prepare("SELECT data FROM entries WHERE session_id=?1 ORDER BY position")?;
            let entries=query.query_map([&id],|row|row.get::<_,String>(0))?.map(|raw|Ok(serde_json::from_str::<Value>(&raw?)?)).collect::<Result<Vec<_>>>()?;
            Ok(serde_json::to_vec_pretty(&json!({"format":"tau-history","version":1,"sessionId":id,"session":serde_json::from_str::<Value>(&session)?,"entries":entries}))?)
        }).await?;
        crate::settings::atomic_write(destination,&bytes).await
    }
    pub async fn flag(&self, id: &str, text: &str) -> Result<Flag> {
        use tokio::io::AsyncWriteExt;
        if text.trim().is_empty() || text.chars().count() > MAX_FLAG_CHARS { bail!("Flag text must contain 1–{MAX_FLAG_CHARS} characters"); }
        let _guard = self.flag_gate.lock().await;
        let session = self.get(id).await?.context("Unknown session")?;
        let flag = Flag { id:uuid::Uuid::new_v4().to_string(),timestamp_ms:crate::agent::now_ms(),session_id:id.into(),session_title:session.title,text:text.into() };
        let mut bytes = serde_json::to_vec(&flag)?; bytes.push(b'\n');
        let path = self.path.with_file_name("flags.jsonl");
        if path == self.path { bail!("Database and flag paths must differ"); }
        let mut options = tokio::fs::OpenOptions::new(); options.create(true).append(true);
        #[cfg(unix)] options.mode(0o600);
        let mut file = options.open(path).await.context("Could not open Tau flag log")?;
        let length = file.metadata().await?.len();
        if let Err(error) = async { file.write_all(&bytes).await?; file.sync_all().await }.await {
            file.set_len(length).await?; file.sync_all().await?; return Err(error.into());
        }
        Ok(flag)
    }
}

fn find_created_session(db: &Connection, request: &str, payload: &str) -> Result<Option<String>> {
    let previous: Option<(String, String)> = db.query_row(
        "SELECT session_id,data FROM receipts WHERE request_id=?1 AND json_extract(data,'$.command')='create_session' LIMIT 1",
        [request], |row| Ok((row.get(0)?,row.get(1)?))).optional()?;
    let Some((session, raw)) = previous else { return Ok(None); };
    let receipt: Receipt = serde_json::from_str(&raw)?;
    if receipt.text != payload { bail!("Create request ID was used for another chat or topic"); }
    Ok(Some(session))
}

fn apply_entry(session: &mut StoredSession, entry: &Value) -> Result<()> {
    match entry["type"].as_str().context("History entry has no type")? {
        "model_change" => {
            session.model = SessionModel { provider:entry["provider"].as_str().context("No provider")?.into(), model_id:entry["modelId"].as_str().context("No model")?.into() };
            if let Some(level) = entry["thinkingLevel"].as_str() { session.thinking = level.into(); }
            session.tokens = None;
        }
        "thinking_level_change" => session.thinking = entry["thinkingLevel"].as_str().context("No thinking level")?.into(),
        "compaction" => session.tokens = None,
        "message" => {
            let message = &entry["message"];
            match message["role"].as_str() {
                Some("user" | "toolResult") => session.needs_turn = true,
                Some("assistant") => {
                    session.needs_turn = matches!(message["stopReason"].as_str(),Some("error" | "aborted" | "toolUse"))
                        || message["content"].as_array().is_some_and(|parts| parts.iter().any(|part| part["type"] == "toolCall"));
                    session.tokens = message["tauModelMessage"]["usage"]["total_tokens"].as_u64();
                }
                _ => {},
            }
        }
        _ => {},
    }
    Ok(())
}

fn activity(db: &Connection) -> Result<u64> {
    let last: Option<u64> = db.query_row("SELECT max(activity) FROM sessions",[],|row| row.get(0))?;
    Ok(crate::agent::now_ms().max(last.unwrap_or(0).saturating_add(1)))
}
