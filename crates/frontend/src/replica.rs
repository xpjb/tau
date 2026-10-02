//! Verified SQLite replica and display projection. Network workers only commit
//! validated headers/ranges here; local outbox intent remains in `store`.
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use std::{collections::{BTreeSet, HashMap}, path::Path, sync::{Arc, Mutex}, time::Duration};
use tau_net::blocks::*;
use tau_net::{ClientCommand, Event, EventKind, QueueOperation, QueueState};
use crate::store::LocalChat;
use std::io::{Read, Write};
use tokio::sync::watch;

pub const QUEUE: &str = "@queue";
const DB_BUSY_TIMEOUT: Duration = Duration::from_secs(5);
pub struct View { pub queue_removals:HashMap<String,u64>,pub previews:Vec<(String,Vec<String>,usize)>,pub partial:bool,pub queue_changed:bool,pub generation:String,pub sequence:u64,pub events:Vec<Event>,pub queue:QueueState,pub before:Option<u64>,pub delivered:Vec<String>, pub bodies:HashMap<String,Body>, pub incomplete:std::collections::HashSet<String>, pub states:HashMap<String,ToolState>, pub parents:HashMap<String,String> }
/// Verified preview state, separate from both authored bytes and finality.
/// None means the body directory/header has not arrived, not a completed empty body.
#[derive(Clone, Debug, Hash)]
pub struct Body {
    pub reference: Option<BodyRef>,
    pub resident: u64,
    pub limited: bool,
}
impl Body {
    pub fn length(&self) -> u64 { self.reference.as_ref().map_or(0, |r| r.length) }
    pub fn complete(&self) -> bool { self.reference.as_ref().is_some_and(|r| self.resident == r.length) }
    pub fn missing(&self) -> bool { self.resident == 0 && !self.complete() }
}
#[derive(Default)]
struct Dirty {full:bool,viewing:bool,ids:BTreeSet<String>}
#[derive(Default)]
struct Prefetched {
    binding: Option<(String,u64)>,
    bodies: HashMap<(String,String),(u64,u64)>,
}
#[derive(Clone)]
pub struct Cache { prefetched:Arc<Mutex<Prefetched>>,exports:Option<std::path::PathBuf>,dirty:Arc<Mutex<HashMap<String,Dirty>>>, db: Arc<Mutex<Connection>>, bound:Arc<std::sync::atomic::AtomicBool>, _lease:Arc<std::fs::File> }
impl Cache {
    pub(crate) fn needs_body(&self, scope: &str, id: &str, background: bool) -> bool {
        let db = self.db.lock().unwrap();
        tau_block_store::header(&db,scope,id).ok().flatten().is_none_or(|h|
            if background {tau_block_store::cache_budget::stored_bytes(&db,scope,id).map_or(true,|bytes|bytes!=h.length)}
            else {!h.sealed || tau_block_store::cached_content(&db,scope,id).map_or(true,|bytes|bytes.len() as u64!=h.length)})
    }
    pub(crate) fn body_checkpoint(&self, scope: &str, id: &str) -> Option<((u64,u64,bool),u64)> {
        let db = self.db.lock().ok()?;
        let h = tau_block_store::header(&db,scope,id).ok()??;
        Some(((h.version,h.length,h.sealed),tau_block_store::cached_prefix(&db,scope,id).ok()?))
    }
    pub(crate) fn descriptor(&self, reference: &ContentRef, epoch: u64) -> Result<tau_net::ServerMessage> {
        let db = self.db.lock().unwrap();
        ensure!(replica_epoch(&db)?==epoch && tau_block_store::cursor(&db)?.lineage==reference.lineage,"Descriptor source changed");
        let bytes = tau_block_store::cached_content(&db,&reference.scope,&reference.id)?;
        drop(db);
        ensure!(bytes.len() as u64==reference.length && blake3::hash(&bytes).to_hex().as_str()==reference.hash,"Descriptor integrity check failed");
        let message=serde_json::from_slice(&bytes)?;
        ensure!(!matches!(message,tau_net::ServerMessage::Data {..}|tau_net::ServerMessage::BlockConnection {..}),"Recursive descriptor");
        Ok(message)
    }

    #[cfg(not(target_os = "android"))]
    pub(crate) fn seed_preview(&self, scope: &str, events: Vec<Event>, mut queue: QueueState, before: Option<u64>) -> Result<View> {
        let ids = events.iter().map(|e| e.id.clone()).collect::<BTreeSet<_>>();
        {
            let mut db = self.db.lock().unwrap(); let tx = db.transaction()?;
            let mut wanted = BTreeSet::from([QUEUE.to_owned()]);
            let mut parents = BTreeSet::from([None, Some(QUEUE.to_owned())]);
            let mut put = |id: String, parent: Option<String>, order, kind, meta, sealed, bytes: &[u8]| -> Result<()> {
                wanted.insert(id.clone());
                tau_block_store::put(&tx, scope, BlockHeader { id, parent, order, kind, meta, sealed, version: 0, length: 0, revision: 0 }, bytes)?;
                Ok(())
            };
            for mut event in events {
                let text = std::mem::take(&mut event.text);
                let kind = match event.kind { EventKind::Tool => BlockKind::Tool, EventKind::Thinking => BlockKind::Thinking, _ => BlockKind::Text };
                let sealed = event.phase != tau_net::EventPhase::Live;
                let mut meta = serde_json::json!({"event": event});
                if kind == BlockKind::Tool {
                    meta["toolState"] = serde_json::to_value(if sealed { ToolState::Completed } else { ToolState::Writing })?;
                }
                put(event.id.clone(), None, event.order, kind, meta, sealed, if kind == BlockKind::Tool { b"" } else { text.as_bytes() })?;
                if kind == BlockKind::Tool {
                    parents.insert(Some(event.id.clone()));
                    put(tool_input_id(&event.id), Some(event.id.clone()), 0, BlockKind::Code, serde_json::json!({"inputFor": event.id}), sealed, text.as_bytes())?;
                }
            }
            let requests = std::mem::take(&mut queue.requests);
            put(QUEUE.into(), None, i64::MAX as u64, BlockKind::Queue, serde_json::json!({}), true, &serde_json::to_vec(&queue)?)?;
            for (order, mut q) in requests.into_iter().enumerate() {
                let text = std::mem::take(&mut q.text);
                put(format!("queued:{}", q.request_id), Some(QUEUE.into()), order as u64, BlockKind::Text, serde_json::json!({"request": q}), true, text.as_bytes())?;
            }
            let old = tx.prepare("SELECT id FROM blocks WHERE scope=?1")?.query_map([scope], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            for id in old { if !wanted.contains(&id) { tau_block_store::remove(&tx, scope, &id)?; } }
            tx.execute("DELETE FROM block_cache_feeds WHERE scope=?1", [scope])?;
            for parent in parents {
                let request = FeedRequest { scope: scope.into(), parent: parent.clone(), cursor: None, floor: 0, before: None };
                // All scripted records above are already committed bodies. This
                // declares their directory coverage, not a fake transport delta.
                tau_block_store::cache_page(&tx, &request, &FeedPage { reset: false, records: vec![], cursor: tau_block_store::cursor(&tx)?, floor: 0,
                    before: if parent.is_none() { before.map(|order| FeedPosition { order, id: String::new() }) } else { None }, more: false })?;
            }
            tx.commit()?;
        }
        self.snapshot_inner(scope, Some(&ids), &BTreeSet::new(), true, None, true, true)?.context("Preview has no native directory")
    }
    pub fn open(path: &Path) -> Result<Self> {
        std::fs::create_dir_all(path.parent().context("Cache path has no parent")?)?;
        let directory=crate::disk::replica_directory_lease(path.parent().unwrap())?;
        let lease=crate::disk::replica_lease(path)?;lease.try_lock_shared()?;
        crate::disk::collect_replicas(path)?;
        // Publish the file under the quota/GC lock so other openers count even
        // a not-yet-initialized live database. Never hold the directory lock
        // through SQLite's journal/schema setup or another writer's wait.
        let mut db = Connection::open(path)?;
        drop(directory);
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path,std::fs::Permissions::from_mode(0o600))?;
        }
        db.busy_timeout(DB_BUSY_TIMEOUT)?;
        db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL")?;
        let version:u32=db.query_row("PRAGMA user_version",[],|r|r.get(0))?;
        ensure!(version<=2,"Replica cache requires a newer client");
        // Echo reuse was added without changing user_version. Older version-2
        // caches still need that additive migration; current caches need no writes.
        let echoes_ready: bool = db.query_row("SELECT count(*)=2 FROM sqlite_schema WHERE (type='table' AND name='local_echoes') OR (type='index' AND name='block_body_hash')", [], |r| r.get(0))?;
        if version < 2 || !echoes_ready {
            let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            tau_block_store::initialize(&tx)?;
            initialize_echoes(&tx)?;
            tx.execute_batch("UPDATE block_limits SET headers=100000,bytes=134217728; CREATE TABLE IF NOT EXISTS replica_epoch(singleton INTEGER PRIMARY KEY,epoch INTEGER NOT NULL); INSERT OR IGNORE INTO replica_epoch VALUES(1,0); PRAGMA user_version=2")?;
            tx.commit()?;
        }
        let page_size:u64=db.query_row("PRAGMA page_size",[],|r|r.get(0))?;
        db.pragma_update(None,"max_page_count",1024u64*1024*1024/page_size)?;
        let exports=path.parent().filter(|p|p.file_name().is_some_and(|n|n=="blocks")).and_then(Path::parent).map(|root|root.join("downloads"));
        Ok(Self { prefetched:Default::default(),exports,dirty:Default::default(),db:Arc::new(Mutex::new(db)),bound:Arc::new(std::sync::atomic::AtomicBool::new(false)),_lease:Arc::new(lease) })
    }
    fn changed(&self,scope:&str,id:&str,full:bool) {
        let mut dirty=self.dirty.lock().unwrap();let d=dirty.entry(scope.into()).or_default();d.full|=full;
        if !d.full {d.ids.insert(id.into());d.full=d.ids.len()>128;}
        if d.full {d.ids.clear();}
    }
    pub fn viewport_changed(&self,scope:&str,ids:impl Iterator<Item=String>) {
        let mut dirty=self.dirty.lock().unwrap();let d=dirty.entry(scope.into()).or_default();
        d.viewing=true;
        if !d.full {d.ids.extend(ids);d.full=d.ids.len()>128;}
        if d.full {d.ids.clear();}
    }
    pub fn changes(&self,scope:&str,visible:Option<&BTreeSet<String>>,full:bool)->Result<Option<View>> {
        self.changes_retaining(scope, visible, &BTreeSet::new(), full)
    }
    pub(crate) fn changes_retaining(&self, scope: &str, visible: Option<&BTreeSet<String>>, retained: &BTreeSet<String>, full: bool) -> Result<Option<View>> {
        let dirty=self.dirty.lock().unwrap().remove(scope).unwrap_or_default();
        if full || dirty.full {return self.snapshot_inner(scope,visible,retained,true,None,true,full || dirty.viewing);}
        if dirty.ids.is_empty() {return Ok(None);}
        let (ids,queue)={let db=self.db.lock().unwrap();let mut ids=BTreeSet::new();let mut queue=false;
            for id in dirty.ids {
                if let Some(h)=tau_block_store::header(&db,scope,&id)? {
                    queue|=h.id==QUEUE || h.parent.as_deref()==Some(QUEUE);
                    ids.insert(h.parent.filter(|p|p!=QUEUE).unwrap_or(h.id));
                }
            }(ids.into_iter().collect::<Vec<_>>(),queue)};
        self.snapshot_inner(scope,visible,retained,true,Some(&ids),queue,dirty.viewing)
    }
    pub(crate) fn epoch(&self)->u64 {replica_epoch(&self.db.lock().unwrap()).unwrap_or(u64::MAX)}
    pub fn clear(&self)->Result<()> {
        let mut db=self.db.lock().unwrap();let tx=db.transaction()?;
        tx.execute_batch("DELETE FROM local_echoes; DELETE FROM blocks; DELETE FROM block_cache_feeds; DELETE FROM block_cache_tombstones; UPDATE replica_epoch SET epoch=epoch+1;")?;tx.commit()?;Ok(())
    }
    pub(crate) fn previous_source(&self)->Result<Option<String>> {
        let db=self.db.lock().unwrap();let known:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM blocks) OR EXISTS(SELECT 1 FROM block_cache_feeds)",[],|r|r.get(0))?;
        Ok(if known {Some(tau_block_store::cursor(&db)?.lineage)} else {None})
    }
    pub fn authorized(&self) -> bool { self.bound.load(std::sync::atomic::Ordering::Acquire) }
    pub fn has_file(&self, scope:&str, id:&str) -> bool {
        tau_block_store::header(&self.db.lock().unwrap(),scope,id).ok().flatten().is_some_and(|h|matches!(h.kind,BlockKind::File|BlockKind::Image))
    }
    pub(crate) fn feed_request(&self, scope: &str, parent: Option<&str>, before: Option<FeedPosition>) -> Result<FeedRequest> {
        let db = self.db.lock().unwrap();
        let saved = tau_block_store::cached_feed(&db,scope,parent)?;
        Ok(FeedRequest { scope:scope.into(),parent:parent.map(str::to_owned),
            cursor:if before.is_some() {None} else {saved.as_ref().map(|p|p.cursor.clone())},
            floor:saved.map_or(0,|p|p.floor),before })
    }
    pub(crate) fn block_request(&self, scope: &str, id: &str) -> Result<BlockRequest> {
        let mut db = self.db.lock().unwrap(); let tx = db.transaction()?;
        let h = tau_block_store::header(&tx,scope,id)?;
        let offset = tau_block_store::cached_prefix(&tx,scope,id)?; tx.commit()?;
        Ok(BlockRequest { scope:scope.into(),id:id.into(),version:h.map_or(0,|h|h.version),offset,follow:true })
    }
    pub(crate) fn configure(&self, lineage: &str) -> Result<()> {
        let mut db = self.db.lock().unwrap(); let tx = db.transaction()?;
        if tau_block_store::cursor(&tx)?.lineage != lineage { tx.execute("DELETE FROM local_echoes", [])?; }
        tau_block_store::cache_lineage(&tx,lineage)?; tx.commit()?;
        self.bound.store(true,std::sync::atomic::Ordering::Release); Ok(())
    }
    #[cfg(test)]
    fn page(&self,lineage:&str,request:&FeedRequest,page:&FeedPage)->Result<()> {self.page_at(lineage,request,page,self.epoch())}
    pub(crate) fn page_at(&self,lineage:&str,request:&FeedRequest,page:&FeedPage,epoch:u64) -> Result<()> {
        let mut db = self.db.lock().unwrap(); let tx = db.transaction()?;
        ensure!(replica_epoch(&tx)?==epoch,"Replica window changed; retry from verified state");
        ensure!(page.cursor.lineage == lineage && tau_block_store::cursor(&tx)?.lineage == lineage,"Stale data connection");
        let reset=page.reset && request.cursor.is_some() && request.before.is_none()
            && tau_block_store::cached_feed(&tx,&request.scope,request.parent.as_deref())?.is_none_or(|old|old.cursor.sequence<=page.cursor.sequence);
        tau_block_store::cache_page(&tx,request,page)?;
        for record in &page.records {
            if let BlockRecord::Put { block } = record
                && let Some(current) = tau_block_store::header(&tx, &request.scope, &block.id)? {
                adopt_echo(&tx, &request.scope, &current)?;
            }
        }
        if reset {tx.execute("UPDATE replica_epoch SET epoch=epoch+1",[])?;}
        tx.commit()?;
        if request.parent.is_none() && request.before.is_none() { self.changed(&request.scope,QUEUE,false); }
        if page.reset {self.changed(&request.scope,QUEUE,true);}
        if let Some(parent)=&request.parent {self.changed(&request.scope,parent,false);}
        for record in &page.records {match record {BlockRecord::Put {block}=>self.changed(&request.scope,&block.id,false),BlockRecord::Remove {id,..}=>self.changed(&request.scope,id,true)}}Ok(())
    }
    #[cfg(test)]
    fn range(&self,lineage:&str,scope:&str,range:&ContentRange)->Result<()> {self.range_at(lineage,scope,range,self.epoch())}
    pub(crate) fn range_at(&self,lineage:&str,scope:&str,range:&ContentRange,epoch:u64) -> Result<()> {
        let mut db = self.db.lock().unwrap(); let tx = db.transaction()?;
        ensure!(replica_epoch(&tx)?==epoch,"Replica window changed; retry from verified state");
        ensure!(tau_block_store::cursor(&tx)?.lineage == lineage,"Stale data connection");
        tau_block_store::cache_range(&tx,scope,range)?;
        tau_block_store::cache_budget::enforce(&tx,scope,&range.header.id,tau_block_store::cache_budget::DEFAULT_CACHE_BYTES)?;
        tx.commit()?;self.changed(scope,&range.header.id,false); Ok(())
    }
    #[cfg(test)]
    fn header(&self,lineage:&str,scope:&str,header:&BlockHeader)->Result<()> {self.header_at(lineage,scope,header,self.epoch())}
    pub(crate) fn header_at(&self,lineage:&str,scope:&str,header:&BlockHeader,epoch:u64) -> Result<()> {
        let mut db = self.db.lock().unwrap(); let tx = db.transaction()?;
        ensure!(replica_epoch(&tx)?==epoch,"Replica window changed; retry from verified state");
        ensure!(tau_block_store::cursor(&tx)?.lineage == lineage,"Stale data connection");
        tau_block_store::cache_header(&tx,scope,header)?;
        if let Some(current) = tau_block_store::header(&tx,scope,&header.id)? { adopt_echo(&tx,scope,&current)?; }
        tx.commit()?;self.changed(scope,&header.id,false); Ok(())
    }
    #[cfg(test)] pub fn snapshot(&self,scope:&str)->Result<Option<View>> {self.snapshot_inner(scope,None,&BTreeSet::new(),false,None,true,true)}
    pub(crate) fn resume_viewport(&self, scope: &str, local: &LocalChat) -> Result<Option<BTreeSet<String>>> {
        if local.position.follow { return Ok(None); }
        let Some(key) = &local.position.key else { return Ok(None); };
        let db = self.db.lock().unwrap();
        let roots = tau_block_store::children(&db, scope, None)?;
        let anchor = roots.iter().position(|h| {
            key == &format!("{scope}/{}",h.id) || key == &format!("{scope}/details:{}",h.id)
                || key == &format!("{scope}/tool:{}",h.id) || key == &format!("{scope}/thinking:{}",h.id)
                || h.meta.pointer("/event/origin/requestId").and_then(|v|v.as_str())
                    .is_some_and(|request| key == &format!("message:{scope}:{request}"))
        });
        Ok(anchor.map(|index| roots.iter().skip(index.saturating_sub(14)).take(30)
            .filter(|h|h.id!=QUEUE).map(|h|h.id.clone()).collect()))
    }
    pub fn preview(&self,scope:&str,visible:Option<&BTreeSet<String>>)->Result<Option<View>> {self.snapshot_inner(scope,visible,&BTreeSet::new(),true,None,true,true)}
    pub fn has_snapshot(&self,scope:&str)->Result<bool> {Ok(tau_block_store::cached_feed(&self.db.lock().unwrap(),scope,None)?.is_some())}
    fn snapshot_inner(&self, scope: &str, visible:Option<&BTreeSet<String>>,retained:&BTreeSet<String>,preview:bool,only:Option<&[String]>,include_queue:bool,touch:bool) -> Result<Option<View>> {
        let db = self.db.lock().unwrap();
        let Some(page) = tau_block_store::cached_feed(&db,scope,None)? else { return Ok(None); };
        let mut body_budget=8*1024*1024usize;let mut group_budgets=HashMap::new();let mut preview_ids=HashMap::<String,Vec<String>>::new();
        let mut roots = if let Some(ids)=only {ids.iter().filter_map(|id|tau_block_store::header(&db,scope,id).transpose()).collect::<Result<Vec<_>>>()?} else {tau_block_store::children(&db,scope,None)?};
        let recent=roots.iter().rev().filter(|h|h.id!=QUEUE).take(30).map(|h|h.id.clone()).collect::<BTreeSet<_>>();
        let visible=visible.unwrap_or(&recent);
        let resident = |id: &String| visible.contains(id) || retained.contains(id);
        let wanted=|h:&BlockHeader|!preview || resident(&h.id) || h.parent.as_ref().is_some_and(resident);
        // Hydrate the current viewport first, then already-read scrollback. A
        // history page/metadata refresh must not blank the previous screen, but
        // retained bytes must never consume the foreground projection budget.
        roots.sort_by_key(|h| (!visible.contains(&h.id), !retained.contains(&h.id), std::cmp::Reverse(h.order)));
        let mut accessed = BTreeSet::new();
        let mut read = |id: &str, limit: usize| -> Result<Vec<u8>> {
            let bytes = tau_block_store::cached_preview(&db, scope, id, limit)?;
            if !bytes.is_empty() { accessed.insert(id.to_owned()); }
            Ok(bytes)
        };
        let mut all = Vec::new();
        for root in &roots {
            all.push(root.clone());
            if wanted(root) || only.is_some() { all.extend(tau_block_store::children(&db,scope,Some(&root.id))?); }
        }
        let mut delivered = Vec::new();
        let mut events = vec![]; let mut bodies = HashMap::new(); let mut queue = QueueState::default();
        let mut incomplete=std::collections::HashSet::new(); let mut states=HashMap::new(); let mut parents=HashMap::new();
        for h in &all {
            if let Some(value) = h.meta.get("event") {
                let mut event:Event=serde_json::from_value(value.clone())?;
                if let Some(parent)=&h.parent {parents.insert(h.id.clone(),parent.clone());}
                let group=h.parent.as_deref().unwrap_or(&h.id);
                let allowance=group_budgets.entry(group).or_insert(256*1024usize);
                preview_ids.entry(group.into()).or_default().push(h.id.clone());
                if let Some(reference)=h.meta.get("fullEvent") {
                    let id=reference["id"].as_str().context("Invalid metadata reference")?;
                    let length=metadata_length(&db,scope,reference)?.unwrap_or(u64::MAX);
                    let limit=if preview {(*allowance).min(body_budget)} else {MAX_BLOCK_BYTES as usize};
                    let bytes=if wanted(h) && length<=limit as u64 {read(id,limit)?} else {vec![]};
                    if !bytes.is_empty() && blake3::hash(&bytes).to_hex().as_str()==reference["hash"].as_str().unwrap_or("") {
                        let mut full:Event=serde_json::from_slice(&bytes)?;
                        full.phase=event.phase;full.is_error=event.is_error;full.order=event.order;
                        if event.phase==tau_net::EventPhase::Interrupted {full.error_message=event.error_message.clone().or(full.error_message);}
                        event=full;
                        if preview {body_budget-=bytes.len();*allowance-=bytes.len();}
                    } else {incomplete.insert(h.id.clone());}
                }
                ensure!(event.id == h.id, "Event metadata has a different native identity");
                let body = if h.kind == BlockKind::Tool { tool_input_id(&h.id) } else { h.id.clone() };
                let bytes = if wanted(h) {read(&body,if preview {(*allowance).min(body_budget)} else {MAX_BLOCK_BYTES as usize})?} else {vec![]};
                if preview {body_budget=body_budget.saturating_sub(bytes.len());*allowance=allowance.saturating_sub(bytes.len());}
                event.text = text_prefix(&bytes)?;
                let reference = tau_block_store::header(&db,scope,&body)?.map(|b| b.body_ref(&page.cursor.lineage, scope));
                let length = reference.as_ref().map_or(0, |b| b.length);
                // Acceptance/header arrival is not display convergence. Only
                // retire the local message after the canonical body is resident.
                // A bounded preview can still be incomplete while its full body
                // is safely cached, so do not confuse truncation with absence.
                if event.role == tau_net::EventRole::User && event.phase == tau_net::EventPhase::Saved
                    && event.kind == EventKind::Text && wanted(h) && (length == 0 || !bytes.is_empty()) && !incomplete.contains(&event.id)
                    && tau_block_store::cache_budget::stored_bytes(&db,scope,&body)? == length
                    && let Some(request) = &event.origin.request_id {
                    delivered.push(request.clone());
                }
                bodies.insert(event.id.clone(), Body { reference, resident: bytes.len() as u64,
                    limited: preview && wanted(h) && (length>256*1024 || body_budget==0 || *allowance==0) && (bytes.len() as u64)<length });
                if bodies[&event.id].reference.is_none() || bytes.len() as u64 != length {incomplete.insert(event.id.clone());}
                if let Some(state)=h.tool_state() {states.insert(event.id.clone(),state);}
                events.push(event);
            }
        }
        if include_queue && let Some(h)=tau_block_store::header(&db,scope,QUEUE)? {
            let bytes=read(QUEUE,MAX_BLOCK_BYTES as usize)?;
            if bytes.len() as u64==h.length && !bytes.is_empty() {queue=serde_json::from_slice(&bytes)?;}
        }
        for h in if include_queue {tau_block_store::children(&db,scope,Some(QUEUE))?} else {vec![]} {
            if let Some(value) = h.meta.get("request") {
                let mut request: tau_net::QueuedRequest = serde_json::from_value(value.clone())?;
                let bytes=read(&h.id,MAX_BLOCK_BYTES as usize)?;
                if bytes.len() as u64 != h.length {incomplete.insert(h.id.clone());}
                request.text = text_prefix(&bytes)?;
                bodies.insert(h.id.clone(), Body { reference: Some(h.body_ref(&page.cursor.lineage, scope)), resident: bytes.len() as u64, limited: false });
                queue.requests.push(request);
            }
        }
        queue.available &= tau_block_store::cached_feed(&db,scope,Some(QUEUE))?.is_some_and(|page|page.before.is_none());
        // A tombstone can beat the root page containing its replacement user
        // entry. Keep the old display row until that root cursor catches up.
        let queue_removals = if include_queue {
            db.prepare("SELECT substr(id,8),revision FROM block_cache_tombstones WHERE scope=?1 AND id LIKE 'queued:%' AND revision>?2")?
                .query_map(rusqlite::params![scope,page.cursor.sequence], |r| Ok((r.get(0)?,r.get(1)?)))?
                .collect::<rusqlite::Result<HashMap<String,u64>>>()?
        } else { HashMap::new() };
        events.sort_by_key(|e|e.order);
        // Recency is disposable metadata, not a prerequisite for rendering a
        // verified read. Never wait for another writer (or upgrade a stale WAL
        // snapshot) just to refresh it, including the first viewport's visit.
        // Actual content writes retain the normal busy timeout and durability.
        if touch && !accessed.is_empty() && let Some(tx) = try_replica_maintenance(&db)? {
            tau_block_store::cache_budget::touch(&tx, scope, accessed.iter().map(String::as_str))?;
            tx.commit()?;
        }
        let mut previews = preview_ids.into_iter().map(|(root,ids)| {
            let used=256*1024-group_budgets[root.as_str()]; (root,ids,used)
        }).collect::<Vec<_>>();
        // Feed's bounded LRU evicts retained/offscreen groups before the viewport.
        previews.sort_by_key(|(root,_,_)| (visible.contains(root), root.clone()));
        Ok(Some(View { queue_removals,previews,partial:only.is_some(),queue_changed:include_queue,generation:format!("{}:{scope}",page.cursor.lineage),sequence:page.cursor.sequence,
            events,queue,before:page.before.as_ref().map(|p|p.order),delivered,bodies,incomplete,states,parents }))
    }
    pub fn copy_ready(&self, scope:&str, ids:&[String]) -> Result<Option<String>> {
        ensure!(ids.len()<=4096,"Copy fewer than 4097 sections at a time");
        let mut seen=BTreeSet::new();let ids=ids.iter().filter(|id|seen.insert(*id)).cloned().collect::<Vec<_>>();let ids=ids.as_slice();
        if !copy_complete(&self.db.lock().unwrap(),scope,ids)? {return Ok(None);}
        let view=self.snapshot_inner(scope,None,&BTreeSet::new(),false,Some(ids),false,true)?.context("Details are not cached")?;
        let group=ids.iter().filter_map(|id|view.events.iter().find(|e|&e.id==id)).collect::<Vec<_>>();
        let text=crate::details::copy(&group, view.events.iter(), &view.parents);ensure!(text.len() as u64<=MAX_BLOCK_BYTES,"Clipboard output exceeds 64 MiB; copy fewer sections");Ok(Some(text))
    }
    pub fn history_cursor(&self, scope: &str) -> Result<Option<FeedPosition>> {
        Ok(tau_block_store::cached_feed(&self.db.lock().unwrap(),scope,None)?.and_then(|p|p.before))
    }
    #[cfg(test)]
    pub fn plan(&self,scope:&str,local:&LocalChat,copy:&[String])->Result<Plan> {self.plan_visible(scope,local,copy,None)}
    pub(crate) fn plan_active(&self, scope: &str, local: &LocalChat, copy: &[String], viewport: Option<&BTreeSet<String>>) -> Result<Plan> {
        let mut plan = self.plan_visible(scope,local,copy,viewport)?;
        // Scrolling back must not stop the ongoing reply from reaching disk.
        // Tail-only interests remain bulk; visible/copy interests win overlaps.
        if viewport.is_some() || !local.position.follow {
            for head in self.plan_background(scope)?.blocks {
                if !plan.blocks.iter().any(|(id,_)| id == &head.0) { plan.blocks.push(head); }
            }
        }
        Ok(plan)
    }
    pub(crate) fn retain_background(&self, scopes:&[String]) {
        let scopes=scopes.iter().collect::<BTreeSet<_>>();
        self.prefetched.lock().unwrap().bodies.retain(|(scope,_),_|scopes.contains(scope));
    }
    pub(crate) fn plan_background(&self, scope: &str) -> Result<Plan> {
        // A small tail only. Do not inherit open Details/tool preferences from
        // the foreground, walk old history, or download attachment payloads.
        let viewport = {
            let db = self.db.lock().unwrap();
            let mut query = db.prepare("SELECT header FROM blocks WHERE scope=?1 AND parent='' AND id!=?2 ORDER BY position DESC,id DESC LIMIT 32")?;
            let roots = query.query_map(rusqlite::params![scope,QUEUE], |r| r.get::<_,String>(0))?
                .map(|raw| Ok(serde_json::from_str::<BlockHeader>(&raw?)?)).collect::<Result<Vec<_>>>()?;
            roots.into_iter().filter(|h|
                h.kind == BlockKind::Text && h.meta.pointer("/event/role").and_then(|v|v.as_str()) != Some("tool")
                || h.meta.pointer("/event/attachment").is_some_and(|v|v.is_object()))
                .take(8).map(|h| h.id).collect()
        };
        let mut plan = self.plan_visible(scope, &LocalChat::default(), &[], Some(&viewport))?;
        plan.background = true;
        plan.foreground.clear();
        // A byte-bounded disk cache is not an instruction to download the same
        // cold bodies forever. Remember successful prefetches for this native
        // version/prefix; explicit viewing/copy still fetches evicted bytes.
        let mut candidates=viewport.clone();
        candidates.extend(plan.blocks.iter().map(|(id,_)|id.clone()));
        let db=self.db.lock().unwrap();
        for id in &viewport {
            if let Some(h)=tau_block_store::header(&db,scope,id)?
                && let Some(id)=h.meta.pointer("/fullEvent/id").and_then(|v|v.as_str()) {candidates.insert(id.into());}
        }
        let binding=(tau_block_store::cursor(&db)?.lineage,replica_epoch(&db)?);
        let mut fetched=self.prefetched.lock().unwrap();
        if fetched.binding.as_ref()!=Some(&binding) {fetched.bodies.clear();fetched.binding=Some(binding);}
        fetched.bodies.retain(|(old,id),_|old!=scope || candidates.contains(id));
        let required=|id:&str,length:u64| if id==QUEUE || id.starts_with("queued:") {length} else {length.min(256*1024)};
        for id in candidates {
            if let Some(h)=tau_block_store::header(&db,scope,&id)? {
                let length=required(&id,h.length);
                if length>0 && tau_block_store::cache_budget::stored_bytes(&db,scope,&id)? >= length {
                    fetched.bodies.insert((scope.into(),id),(h.version,length));
                }
            }
        }
        plan.blocks.retain(|(id,head)|head.is_none_or(|(version,length,_,stored)| {
            let length=required(id,length);
            stored>=length || fetched.bodies.get(&(scope.into(),id.clone()))!=Some(&(version,length))
        }));
        let mut budget = 8 * 1024 * 1024u64;
        plan.blocks.retain(|(id, head)| {
            let length = head.map_or(0, |(_,length,_,_)|
                if id == QUEUE || id.starts_with("queued:") { length } else { length.min(256*1024) });
            if length > budget { return false; }
            budget -= length;
            true
        });
        Ok(plan)
    }
    pub fn plan_visible(&self, scope: &str, local: &LocalChat, copy:&[String], viewport:Option<&BTreeSet<String>>) -> Result<Plan> {
        let rendered = viewport.is_some();
        let resume = if !rendered { self.resume_viewport(scope,local)? } else { None };
        let viewport = viewport.or(resume.as_ref());
        let db = self.db.lock().unwrap();
        let roots = if let Some(viewport)=viewport {
            let ids=viewport.iter().chain(copy.iter()).collect::<BTreeSet<_>>();
            ids.into_iter().filter_map(|id|tau_block_store::header(&db,scope,id).transpose()).collect::<Result<Vec<_>>>()?.into_iter().filter(|h|h.parent.is_none()).collect()
        } else {tau_block_store::children(&db,scope,None)?};
        let mut parents = BTreeSet::from([None,Some(QUEUE.into())]);
        let mut blocks = BTreeSet::from([QUEUE.into()]);
        let mut metadata = roots.clone();
        for root in &roots {metadata.extend(tau_block_store::children(&db,scope,Some(&root.id))?);}
        for id in copy {
            if let Some(h)=tau_block_store::header(&db,scope,id)? && h.parent.is_some() {
                ensure!(h.meta.get("event").is_some(),"Use Download for non-text payloads");
                blocks.insert(id.clone());metadata.push(h);
            }
        }
        let mut events = roots.iter().filter_map(|h| h.meta.get("event").map(|value|(h,value))).map(|(h,value)| {
            let mut event:Event=serde_json::from_value(value.clone())?;
            if h.length > 0 {event.text = "x".into();}
            Ok(event)
        }).collect::<Result<Vec<_>>>()?;
        events.sort_by_key(|e|e.order);
        let visible = viewport.filter(|_|rendered).map(|ids|ids.iter().cloned().collect()).unwrap_or_else(||crate::details::open_items(events.iter(),local));
        let recent=roots.iter().rev().filter(|h|h.kind==BlockKind::Text && h.meta.pointer("/event/role").and_then(|v|v.as_str())!=Some("tool") && h.length>0)
            .take(2).map(|h|h.id.clone()).collect::<BTreeSet<_>>();
        // Bootstrap without a renderer is a bounded recent window. Once the
        // renderer owns interests, only viewport + two-screen overscan is read.
        let window=viewport.cloned().unwrap_or_else(||roots.iter().rev().filter(|h|h.id!=QUEUE).take(32).map(|h|h.id.clone()).collect());
        let pending_copy=copy.iter().filter(|id|!copy_complete(&db,scope,std::slice::from_ref(*id)).unwrap_or(false)).cloned().collect::<BTreeSet<_>>();
        let metadata_body = |h: &BlockHeader, selected: bool| -> Result<Option<String>> {
            if (selected || copy.contains(&h.id)
                || window.contains(&h.id) && h.meta.pointer("/event/attachment").is_some_and(|v|v.is_object()))
                && let Some(reference)=h.meta.get("fullEvent")
                && (copy.contains(&h.id) || h.parent.as_ref().is_some_and(|id|copy.contains(id)) || metadata_length(&db,scope,reference)?.unwrap_or(0)<=256*1024) {
                return Ok(reference["id"].as_str().map(str::to_owned));
            }
            Ok(None)
        };
        let mut demands = Vec::new();
        for h in roots {
            if h.id == QUEUE { continue; }
            if !pending_copy.contains(&h.id) && (!window.contains(&h.id) || copy.contains(&h.id)) {continue;}
            let mut required = BTreeSet::new();
            let mut parent = None;
            let mut members = vec![h.clone()];
            if h.kind == BlockKind::Tool {
                let call=h.parent.as_deref().unwrap_or(&h.id);
                let key = format!("tool:{call}");
                if copy.contains(&h.id) || visible.contains(&h.id) && local.expansion.get(&key).copied().unwrap_or(false) {
                    parent = Some(h.id.clone());
                    let children = tau_block_store::children(&db,scope,Some(&h.id))?;
                    members.extend(children.iter().cloned());
                    let error = h.meta.pointer("/event/isError").and_then(|v|v.as_bool()) == Some(true)
                        || children.iter().filter(|c|c.meta.get("event").is_some()).next_back()
                            .is_some_and(|c|c.meta.pointer("/event/isError").and_then(|v|v.as_bool()) == Some(true));
                    let output = if error { ToolBody::Error } else { ToolBody::Output };
                    let output_length = children.iter().filter(|c|matches!(c.tool_body(),Some(ToolBody::Output | ToolBody::Error))).map(|c|c.length).sum::<u64>();
                    for child in children {
                        let Some(body) = child.tool_body() else { continue; };
                        // The UI presents one result section. Its disclosure and
                        // aggregate size govern every member, including mixed success/error results.
                        let (section,length) = if body == ToolBody::Input {(body,child.length)} else {(output,output_length)};
                        if copy.contains(&h.id) || length <= 1200 || local.expansion.get(&format!("{key}:{}",section.label())).copied().unwrap_or(false) {
                            required.insert(child.id);
                        }
                    }
                }
            } else if h.meta.get("event").is_some() {
                if let Some(body) = h.tool_body() {
                    let call=h.parent.as_deref().unwrap_or(&h.id);
                    let key = format!("tool:{call}");
                    let label = body.label();
                    if copy.contains(&h.id) || visible.contains(&h.id) && local.expansion.get(&key).copied().unwrap_or(false)
                        && (h.length <= 1200 || local.expansion.get(&format!("{key}:{label}")).copied().unwrap_or(false)) {required.insert(h.id.clone());}
                } else if h.meta.pointer("/event/role").and_then(|v|v.as_str()) != Some("tool")
                    && (copy.contains(&h.id) || h.kind != BlockKind::Thinking || visible.contains(&h.id)) {required.insert(h.id.clone());}
            }
            for member in &members {
                if let Some(id) = metadata_body(member, required.contains(&member.id) || parent.as_ref() == Some(&member.id))? {
                    required.insert(id);
                }
            }
            // Closed tools have no demand. Cached bodies must not repeatedly
            // win the same thirty slots while other viewport rows never start.
            if required.is_empty() && parent.is_none() { continue; }
            let mut pending = if let Some(parent) = &parent {
                tau_block_store::cached_feed(&db,scope,Some(parent))?.is_none_or(|page| page.before.is_some() || page.cursor.sequence < h.revision)
            } else { false };
            for id in &required {
                pending |= match tau_block_store::header(&db,scope,id)? {
                    None => true,
                    Some(head) => tau_block_store::cache_budget::stored_bytes(&db,scope,id)? <
                        if copy.contains(&h.id) { head.length } else { head.length.min(256*1024) },
                };
            }
            demands.push(((!pending_copy.contains(&h.id), !pending, std::cmp::Reverse(h.order)), parent, required));
        }
        demands.sort_by_key(|(priority,_,_)| *priority);
        for (_,parent,required) in demands.into_iter().take(30) {
            if let Some(parent) = parent { parents.insert(Some(parent)); }
            blocks.extend(required);
        }
        for h in tau_block_store::children(&db,scope,Some(QUEUE))? { blocks.insert(h.id); }
        // Full captions/errors/names are explicit metadata-body interests too.
        // Closed tool cards still cause no child or body requests.
        for h in &metadata {
            if let Some(id) = metadata_body(h, blocks.contains(&h.id) || parents.contains(&Some(h.id.clone())))? { blocks.insert(id); }
        }
        // Bound the active body working set independently of watcher count.
        // Downloads (two <=50 MB files) and two <=64 MiB descriptors have room
        // alongside this 128 MiB viewport budget in the 512 MiB body cache.
        let mut candidates=blocks.iter().cloned().collect::<Vec<_>>();
        candidates.sort_by_key(|id|(!(*id==QUEUE || recent.contains(id)),id.clone()));
        let mut budget=128*1024*1024u64;let mut admitted=BTreeSet::new();
        for id in candidates {
            let h=tau_block_store::header(&db,scope,&id)?;
            let complete=copy.contains(&id) || h.as_ref().and_then(|h|h.parent.as_ref()).is_some_and(|parent|copy.contains(parent))
                || id==QUEUE || h.as_ref().and_then(|h|h.parent.as_deref())==Some(QUEUE);
            let mut length=h.map_or(0,|h|h.length);
            if !complete && length>256*1024 {
                if tau_block_store::cache_budget::stored_bytes(&db,scope,&id)?>=256*1024 {continue;}
                length=256*1024;
            }
            if length<=budget {budget-=length;admitted.insert(id);}
        }
        blocks=admitted;
        // Include content heads in the plan so a sealed block's replacement can
        // restart its watch even if the set of IDs did not change.
        let heads = blocks.iter().map(|id| Ok((id.clone(),tau_block_store::header(&db,scope,id)?.map(|h|(h.version,h.length,h.sealed,tau_block_store::cache_budget::stored_bytes(&db,scope,id).unwrap_or(0))))))
            .collect::<Result<Vec<_>>>()?;
        let mut older=vec![];
        for parent in parents.iter().flatten() {
            if let Some(before)=tau_block_store::cached_feed(&db,scope,Some(parent))?.and_then(|p|p.before) {older.push((parent.clone(),before));}
        }
        let foreground=blocks.iter().filter(|id|*id==QUEUE || recent.contains(*id) || tau_block_store::header(&db,scope,id).ok().flatten().is_some_and(|h|!h.sealed && h.kind!=BlockKind::Code))
            .cloned().collect();
        Ok(Plan { scope:scope.into(),parents,blocks:heads,older,foreground,background:false })
    }
}
// Admission only for optional read-recency/local-echo reuse, never authored or
// verified-content commits. The caller holds the cache mutex, so the zero wait
// cannot leak to another job; restore the durable-write timeout on both paths.
fn try_replica_maintenance(db: &Connection) -> Result<Option<rusqlite::Transaction<'_>>> {
    db.busy_timeout(Duration::ZERO)?;
    let transaction = rusqlite::Transaction::new_unchecked(db, rusqlite::TransactionBehavior::Immediate);
    db.busy_timeout(DB_BUSY_TIMEOUT)?;
    match transaction {
        Ok(tx) => Ok(Some(tx)),
        Err(rusqlite::Error::SqliteFailure(error, _)) if error.code == rusqlite::ErrorCode::DatabaseBusy => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn replica_epoch(db:&Connection)->Result<u64> {Ok(db.query_row("SELECT epoch FROM replica_epoch WHERE singleton=1",[],|r|r.get(0))?)}
fn metadata_length(db:&Connection,scope:&str,reference:&serde_json::Value)->Result<Option<u64>> {
    let id=reference["id"].as_str().context("Invalid metadata reference")?;
    Ok(match (reference["length"].as_u64(),tau_block_store::header(db,scope,id)?.map(|h|h.length)) {
        (Some(a),Some(b))=>Some(a.max(b)),(a,b)=>a.or(b),
    })
}
fn copy_complete(db:&Connection,scope:&str,ids:&[String])->Result<bool> {
    let mut ready=true;let mut bodies=std::collections::BTreeMap::new();let mut metadata=std::collections::BTreeMap::new();
    let mut inspect=|h:&BlockHeader|->Result<()> {
        if let Some(reference)=h.meta.get("fullEvent") {metadata.insert(reference["id"].as_str().context("Invalid metadata reference")?.to_owned(),reference.clone());}Ok(())
    };
    for id in ids {
        let h=tau_block_store::header(db,scope,id)?.context("Details block no longer exists")?;
        ready&=h.sealed && (h.kind!=BlockKind::Tool || h.tool_state().is_some_and(ToolState::finished));
        inspect(&h)?;
        if h.kind==BlockKind::Tool {
            ready&=tau_block_store::cached_feed(db,scope,Some(id))?.is_some_and(|p|p.before.is_none() && p.cursor.sequence>=h.revision);
            for child in tau_block_store::children(db,scope,Some(id))? {
                if child.tool_body().is_some() {
                    inspect(&child)?;bodies.insert(child.id.clone(),child);
                }
            }
        } else {bodies.insert(h.id.clone(),h);}
    }
    // Check all declared sizes before reading a single large metadata body, even
    // while another selected body is still incomplete. No cache-thrashing copy.
    let mut total=0u64;
    for h in bodies.values() {total=total.saturating_add(h.length);}
    for reference in metadata.values() {
        if let Some(length)=metadata_length(db,scope,reference)? {total=total.saturating_add(length);} else {ready=false;}
    }
    ensure!(total<=MAX_BLOCK_BYTES,"Details including metadata exceed the 64 MiB clipboard limit");
    for h in bodies.values() {ready&=h.sealed && tau_block_store::cache_budget::stored_bytes(db,scope,&h.id)?==h.length;}
    if !ready {return Ok(false);}
    for (id,reference) in metadata {
        let bytes=tau_block_store::cached_content(db,scope,&id)?;
        if blake3::hash(&bytes).to_hex().as_str()!=reference["hash"].as_str().unwrap_or("") {return Ok(false);}
    }
    Ok(true)
}

fn text_prefix(bytes: &[u8]) -> Result<String> {
    match std::str::from_utf8(bytes) {
        Ok(text) => Ok(text.into()),
        Err(error) if error.error_len().is_none() => Ok(std::str::from_utf8(&bytes[..error.valid_up_to()])?.into()),
        Err(error) => Err(error.into()),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan { pub background: bool, pub scope: String, pub parents: BTreeSet<Option<String>>, pub blocks: Vec<(String,Option<(u64,u64,bool,u64)>)>, pub older:Vec<(String,FeedPosition)>, pub foreground:BTreeSet<String> }

impl Cache {
    pub fn lineage(&self) -> Result<String> { Ok(tau_block_store::cursor(&self.db.lock().unwrap())?.lineage) }
    pub(crate) fn cached_header(&self, scope:&str, id:&str) -> Result<Option<BlockHeader>> { tau_block_store::header(&self.db.lock().unwrap(),scope,id) }
    pub fn file_ready(&self, scope:&str, id:&str, path:&Path, limit:u64) -> Result<bool> {
        let Some(h)=self.cached_header(scope,id)? else {return Ok(false);};
        let Some(expected)=h.meta.get("sha256").and_then(|v|v.as_str()) else {return Ok(false);};
        if !h.sealed || h.length>limit || !path.metadata().is_ok_and(|m|m.is_file() && m.len()==h.length) {return Ok(false);}
        use sha2::Digest;
        let mut file=std::fs::File::open(path)?; let mut buffer=[0;BLOCK_CHUNK_BYTES]; let mut hash=sha2::Sha256::new();
        loop {let n=file.read(&mut buffer)?;if n==0 {break;} hash.update(&buffer[..n]);}
        let valid=format!("{:x}",hash.finalize())==expected;
        if valid && self.exports.as_ref().is_some_and(|root|path.starts_with(root)) && let Ok(file)=std::fs::OpenOptions::new().write(true).open(path) {let _=file.set_times(std::fs::FileTimes::new().set_modified(std::time::SystemTime::now()));}
        Ok(valid)
    }
    pub(crate) fn export(&self, lineage:&str, scope:&str, header:&BlockHeader, target:&Path, cancel:&watch::Receiver<bool>) -> Result<()> {
        use sha2::Digest;
        let epoch=self.epoch();
        let parent=target.parent().context("Invalid cache path")?;
        std::fs::create_dir_all(parent)?;
        let mut temp=tempfile::NamedTempFile::new_in(parent)?;
        let mut hash=sha2::Sha256::new(); let mut offset=0;
        while offset<header.length {
            ensure!(!*cancel.borrow(),"Download cancelled");
            let range={
                let db=self.db.lock().unwrap();
                ensure!(replica_epoch(&db)?==epoch && tau_block_store::cursor(&db)?.lineage==lineage,"Data source changed");
                tau_block_store::read(&db,&BlockRequest {scope:scope.into(),id:header.id.clone(),version:header.version,offset,follow:false})?
            };
            ensure!(range.header.version==header.version && range.offset==offset && !range.bytes.is_empty(),"Cached file changed or is incomplete");
            temp.write_all(&range.bytes)?; hash.update(&range.bytes); offset+=range.bytes.len() as u64;
        }
        if let Some(expected)=header.meta.get("sha256").and_then(|v|v.as_str()) {
            ensure!(format!("{:x}",hash.finalize())==expected,"Cached file checksum mismatch");
        }
        temp.as_file().sync_all()?;
        static EXPORT_GATE:std::sync::Mutex<()>=std::sync::Mutex::new(());
        let _gate=EXPORT_GATE.lock().unwrap();
        if let Some(root)=&self.exports && target.starts_with(root) {crate::disk::collect_downloads(root,target,header.length,1024*1024*1024)?;}
        // Hold the cache fence through the final rename: clear/reset cannot be
        // followed by late publication from a previously verified generation.
        let db=self.db.lock().unwrap();
        ensure!(!*cancel.borrow() && replica_epoch(&db)?==epoch && tau_block_store::cursor(&db)?.lineage==lineage,"Download cancelled or data source changed");
        temp.persist(target)?;
        #[cfg(unix)] std::fs::File::open(parent)?.sync_all()?;
        Ok(())
    }
}
const MAX_ECHO_BYTES: usize = 32 * 1024 * 1024;
const MAX_ECHO_ENTRIES: usize = 256;

fn initialize_echoes(db: &Connection) -> Result<()> {
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
        if tau_block_store::cursor(&tx)?.lineage != lineage { return Ok(()); }
        for pending in &local.pending {
            let (request, text) = match &pending.request.command {
                ClientCommand::Prompt { text, .. } => (&pending.request.id, text),
                ClientCommand::QueueControl { operation: QueueOperation::Edit { request_id, text, .. }, .. } => (request_id, text),
                _ => continue,
            };
            if text.len() > MAX_ECHO_BYTES { continue; }
            tx.execute("INSERT OR IGNORE INTO local_echoes(scope,request,hash,body) VALUES(?1,?2,?3,?4)",
                params![scope, request, blake3::hash(text.as_bytes()).to_hex().as_str(), text.as_bytes()])?;
        }
        loop {
            let (count, bytes): (usize, usize) = tx.query_row("SELECT count(*),coalesce(sum(length(body)),0) FROM local_echoes", [], |r| Ok((r.get(0)?, r.get(1)?)))?;
            if count <= MAX_ECHO_ENTRIES && bytes <= MAX_ECHO_BYTES { break; }
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
            if adopt_echo(&tx, scope, &h)? { changed.push(h.id); }
        }
        tx.commit()?;
        drop(db);
        for id in changed { self.changed(scope, &id, false); }
        Ok(())
    }
}

fn adopt_echo(db: &Connection, scope: &str, h: &BlockHeader) -> Result<bool> {
    if !h.sealed || h.kind != BlockKind::Text || h.length > MAX_ECHO_BYTES as u64 { return Ok(false); }
    let request = if h.parent.as_deref() == Some(QUEUE) {
        h.meta.pointer("/request/requestId").and_then(|v| v.as_str())
    } else if h.meta.pointer("/event/role").and_then(|v| v.as_str()) == Some("user") {
        h.meta.pointer("/event/origin/requestId").and_then(|v| v.as_str())
    } else { None };
    let (Some(request), Some(hash)) = (request, h.meta.get("bodyHash").and_then(|v| v.as_str())) else { return Ok(false); };
    if tau_block_store::cache_budget::stored_bytes(db, scope, &h.id)? == h.length { return Ok(false); }
    let bytes: Option<Vec<u8>> = db.query_row("SELECT body FROM local_echoes WHERE scope=?1 AND request=?2 AND hash=?3",
        params![scope, request, hash], |r| r.get(0)).optional()?;
    let Some(bytes) = bytes else { return Ok(false); };
    if bytes.len() as u64 != h.length || blake3::hash(&bytes).to_hex().as_str() != hash { return Ok(false); }
    for (i, bytes) in bytes.chunks(BLOCK_CHUNK_BYTES).enumerate() {
        tau_block_store::cache_range(db, scope, &ContentRange { header: h.clone(), offset: (i * BLOCK_CHUNK_BYTES) as u64,
            hash: blake3::hash(bytes).to_hex().to_string(), bytes: bytes.to_vec() })?;
    }
    tau_block_store::cache_budget::enforce(db, scope, &h.id, tau_block_store::cache_budget::DEFAULT_CACHE_BYTES)?;
    Ok(true)
}

#[cfg(test)]
#[path = "../tests/unit/replica.rs"]
pub(crate) mod tests;
