//! Persistent block cache and explicit watch ownership, independent of the UI and
//! the control socket. A collapsed tool never causes a content watch.
mod files;
mod uploads;
use anyhow::{Context, Result, ensure};
use rusqlite::Connection;
use std::{collections::{BTreeSet, HashMap}, path::Path, sync::{Arc, Mutex}, time::Duration};
use tau_blocks::*;
use tau_protocol::{Event, EventKind, QueueState, TranscriptSnapshot};
use tau_transfer::blocks::{Client, Header};
use tokio::sync::{mpsc, watch};
use crate::store::LocalChat;

pub const QUEUE: &str = "@queue";
pub struct View { pub previews:Vec<(String,Vec<String>,usize)>,pub partial:bool,pub queue_changed:bool,pub snapshot:TranscriptSnapshot, pub lengths:HashMap<String,u64>, pub incomplete:std::collections::HashSet<String>, pub states:HashMap<String,String> }
#[derive(Default)]
struct Dirty {full:bool,ids:BTreeSet<String>}
#[derive(Clone)]
pub struct Cache { exports:Option<std::path::PathBuf>,dirty:Arc<Mutex<HashMap<String,Dirty>>>, db: Arc<Mutex<Connection>>, bound:Arc<std::sync::atomic::AtomicBool>, _lease:Arc<std::fs::File> }
impl Cache {
    pub fn open(path: &Path) -> Result<Self> {
        std::fs::create_dir_all(path.parent().context("Cache path has no parent")?)?;
        let _directory=crate::disk::replica_directory_lease(path.parent().unwrap())?;
        let lease=crate::disk::replica_lease(path)?;lease.try_lock_shared()?;
        crate::disk::collect_replicas(path)?;
        let db = Connection::open(path)?;
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path,std::fs::Permissions::from_mode(0o600))?;
        }
        db.busy_timeout(Duration::from_secs(5))?;
        db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL")?;
        let version:u32=db.query_row("PRAGMA user_version",[],|r|r.get(0))?;
        ensure!(version<=2,"Replica cache requires a newer client");
        tau_blocks::initialize(&db)?;
        db.execute_batch("UPDATE block_limits SET headers=100000,bytes=134217728; CREATE TABLE IF NOT EXISTS replica_epoch(singleton INTEGER PRIMARY KEY,epoch INTEGER NOT NULL); INSERT OR IGNORE INTO replica_epoch VALUES(1,0); PRAGMA user_version=2")?;
        let page_size:u64=db.query_row("PRAGMA page_size",[],|r|r.get(0))?;
        db.pragma_update(None,"max_page_count",1024u64*1024*1024/page_size)?;
        let exports=path.parent().filter(|p|p.file_name().is_some_and(|n|n=="blocks")).and_then(Path::parent).map(|root|root.join("downloads"));
        Ok(Self { exports,dirty:Default::default(),db:Arc::new(Mutex::new(db)),bound:Arc::new(std::sync::atomic::AtomicBool::new(false)),_lease:Arc::new(lease) })
    }
    fn changed(&self,scope:&str,id:&str,full:bool) {
        let mut dirty=self.dirty.lock().unwrap();let d=dirty.entry(scope.into()).or_default();d.full|=full;d.ids.insert(id.into());
    }
    pub fn viewport_changed(&self,scope:&str,ids:impl Iterator<Item=String>) {for id in ids {self.changed(scope,&id,false);}}
    pub fn changes(&self,scope:&str,visible:Option<&BTreeSet<String>>,full:bool)->Result<Option<View>> {
        let dirty=self.dirty.lock().unwrap().remove(scope).unwrap_or_default();
        if full || dirty.full {return self.preview(scope,visible);}
        if dirty.ids.is_empty() {return Ok(None);}
        let (ids,queue)={let db=self.db.lock().unwrap();let mut ids=BTreeSet::new();let mut queue=false;
            for id in dirty.ids {
                if let Some(h)=tau_blocks::header(&db,scope,&id)? {
                    queue|=h.id==QUEUE || h.parent.as_deref()==Some(QUEUE);
                    ids.insert(h.parent.filter(|p|p!=QUEUE).unwrap_or(h.id));
                }
            }(ids.into_iter().collect::<Vec<_>>(),queue)};
        self.snapshot_inner(scope,visible,true,Some(&ids),queue)
    }
    fn epoch(&self)->u64 {replica_epoch(&self.db.lock().unwrap()).unwrap_or(u64::MAX)}
    pub fn clear(&self)->Result<()> {
        let mut db=self.db.lock().unwrap();let tx=db.transaction()?;
        tx.execute_batch("DELETE FROM blocks; DELETE FROM block_cache_feeds; DELETE FROM block_cache_tombstones; UPDATE replica_epoch SET epoch=epoch+1;")?;tx.commit()?;Ok(())
    }
    pub(crate) fn previous_source(&self)->Result<Option<String>> {
        let db=self.db.lock().unwrap();let known:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM blocks) OR EXISTS(SELECT 1 FROM block_cache_feeds)",[],|r|r.get(0))?;
        Ok(if known {Some(tau_blocks::cursor(&db)?.lineage)} else {None})
    }
    pub fn authorized(&self) -> bool { self.bound.load(std::sync::atomic::Ordering::Acquire) }
    pub fn has_file(&self, scope:&str, id:&str) -> bool {
        tau_blocks::header(&self.db.lock().unwrap(),scope,id).ok().flatten().is_some_and(|h|matches!(h.kind,BlockKind::File|BlockKind::Image))
    }
    fn feed_request(&self, scope: &str, parent: Option<&str>, before: Option<FeedPosition>) -> Result<FeedRequest> {
        let db = self.db.lock().unwrap();
        let saved = tau_blocks::cached_feed(&db,scope,parent)?;
        Ok(FeedRequest { scope:scope.into(),parent:parent.map(str::to_owned),
            cursor:if before.is_some() {None} else {saved.as_ref().map(|p|p.cursor.clone())},
            floor:saved.map_or(0,|p|p.floor),before })
    }
    fn block_request(&self, scope: &str, id: &str) -> Result<BlockRequest> {
        let mut db = self.db.lock().unwrap(); let tx = db.transaction()?;
        let h = tau_blocks::header(&tx,scope,id)?;
        let offset = tau_blocks::cached_prefix(&tx,scope,id)?; tx.commit()?;
        Ok(BlockRequest { scope:scope.into(),id:id.into(),version:h.map_or(0,|h|h.version),offset,follow:true })
    }
    fn configure(&self, lineage: &str) -> Result<()> {
        let mut db = self.db.lock().unwrap(); let tx = db.transaction()?;
        tau_blocks::cache_lineage(&tx,lineage)?; tx.commit()?;
        self.bound.store(true,std::sync::atomic::Ordering::Release); Ok(())
    }
    #[cfg(test)]
    fn page(&self,lineage:&str,request:&FeedRequest,page:&FeedPage)->Result<()> {self.page_at(lineage,request,page,self.epoch())}
    fn page_at(&self,lineage:&str,request:&FeedRequest,page:&FeedPage,epoch:u64) -> Result<()> {
        let mut db = self.db.lock().unwrap(); let tx = db.transaction()?;
        ensure!(replica_epoch(&tx)?==epoch,"Replica window changed; retry from verified state");
        ensure!(page.cursor.lineage == lineage && tau_blocks::cursor(&tx)?.lineage == lineage,"Stale data connection");
        let reset=page.reset && request.cursor.is_some() && request.before.is_none()
            && tau_blocks::cached_feed(&tx,&request.scope,request.parent.as_deref())?.is_none_or(|old|old.cursor.sequence<=page.cursor.sequence);
        tau_blocks::cache_page(&tx,request,page)?;
        if reset {tx.execute("UPDATE replica_epoch SET epoch=epoch+1",[])?;}
        tx.commit()?;
        if page.reset {self.changed(&request.scope,QUEUE,true);}
        if let Some(parent)=&request.parent {self.changed(&request.scope,parent,false);}
        for record in &page.records {match record {BlockRecord::Put {block}=>self.changed(&request.scope,&block.id,false),BlockRecord::Remove {id,..}=>self.changed(&request.scope,id,true)}}Ok(())
    }
    #[cfg(test)]
    fn range(&self,lineage:&str,scope:&str,range:&ContentRange)->Result<()> {self.range_at(lineage,scope,range,self.epoch())}
    fn range_at(&self,lineage:&str,scope:&str,range:&ContentRange,epoch:u64) -> Result<()> {
        let mut db = self.db.lock().unwrap(); let tx = db.transaction()?;
        ensure!(replica_epoch(&tx)?==epoch,"Replica window changed; retry from verified state");
        ensure!(tau_blocks::cursor(&tx)?.lineage == lineage,"Stale data connection");
        tau_blocks::cache_range(&tx,scope,range)?;
        tau_blocks::cache_budget::enforce(&tx,scope,&range.header.id,tau_blocks::cache_budget::DEFAULT_CACHE_BYTES)?;
        tx.commit()?;self.changed(scope,&range.header.id,false); Ok(())
    }
    #[cfg(test)]
    fn header(&self,lineage:&str,scope:&str,header:&BlockHeader)->Result<()> {self.header_at(lineage,scope,header,self.epoch())}
    fn header_at(&self,lineage:&str,scope:&str,header:&BlockHeader,epoch:u64) -> Result<()> {
        let mut db = self.db.lock().unwrap(); let tx = db.transaction()?;
        ensure!(replica_epoch(&tx)?==epoch,"Replica window changed; retry from verified state");
        ensure!(tau_blocks::cursor(&tx)?.lineage == lineage,"Stale data connection");
        tau_blocks::cache_header(&tx,scope,header)?; tx.commit()?;self.changed(scope,&header.id,false); Ok(())
    }
    #[cfg(test)] pub fn snapshot(&self,scope:&str)->Result<Option<View>> {self.snapshot_inner(scope,None,false,None,true)}
    pub fn preview(&self,scope:&str,visible:Option<&BTreeSet<String>>)->Result<Option<View>> {self.snapshot_inner(scope,visible,true,None,true)}
    pub fn has_snapshot(&self,scope:&str)->Result<bool> {Ok(tau_blocks::cached_feed(&self.db.lock().unwrap(),scope,None)?.is_some())}
    fn snapshot_inner(&self, scope: &str, visible:Option<&BTreeSet<String>>,preview:bool,only:Option<&[String]>,include_queue:bool) -> Result<Option<View>> {
        let db = self.db.lock().unwrap();
        let Some(page) = tau_blocks::cached_feed(&db,scope,None)? else { return Ok(None); };
        let mut body_budget=8*1024*1024usize;let mut group_budgets=HashMap::new();let mut preview_ids=HashMap::<String,Vec<String>>::new();
        let roots = if let Some(ids)=only {ids.iter().filter_map(|id|tau_blocks::header(&db,scope,id).transpose()).collect::<Result<Vec<_>>>()?} else {tau_blocks::children(&db,scope,None)?};
        let recent=roots.iter().rev().filter(|h|h.id!=QUEUE).take(30).map(|h|h.id.clone()).collect::<BTreeSet<_>>();
        let visible=visible.unwrap_or(&recent);
        let wanted=|h:&BlockHeader|!preview || visible.contains(&h.id) || h.parent.as_ref().is_some_and(|id|visible.contains(id));
        let mut all = roots.clone();
        for root in &roots {if wanted(root) || only.is_some() {all.extend(tau_blocks::children(&db,scope,Some(&root.id))?);}}
        let mut events = vec![]; let mut sizes = HashMap::new(); let mut queue = QueueState::default();
        let mut incomplete=std::collections::HashSet::new(); let mut states=HashMap::new();
        for h in &all {
            if let Some(value) = h.meta.get("event") {
                let mut event=native_event(h,value)?;
                let call_key=event.tool_call_id.clone();
                let group=h.parent.as_deref().unwrap_or(&h.id);
                let allowance=group_budgets.entry(group).or_insert(256*1024usize);
                preview_ids.entry(group.into()).or_default().push(h.id.clone());
                if let Some(reference)=h.meta.get("fullEvent") {
                    let id=reference["id"].as_str().context("Invalid metadata reference")?;
                    let length=metadata_length(&db,scope,reference)?.unwrap_or(u64::MAX);
                    let limit=if preview {(*allowance).min(body_budget)} else {MAX_BLOCK_BYTES as usize};
                    let bytes=if wanted(h) && length<=limit as u64 {tau_blocks::cached_preview(&db,scope,id,limit)?} else {vec![]};
                    if !bytes.is_empty() && blake3::hash(&bytes).to_hex().as_str()==reference["hash"].as_str().unwrap_or("") {
                        let mut full:Event=serde_json::from_slice(&bytes)?;
                        full.phase=event.phase;full.is_error=event.is_error;full.order=event.order;
                        if event.phase==tau_protocol::EventPhase::Interrupted {full.error_message=event.error_message.clone().or(full.error_message);}
                        event=full;
                        if preview {body_budget-=bytes.len();*allowance-=bytes.len();}
                    } else {incomplete.insert(h.id.clone());}
                    event.tool_call_id=call_key;
                }
                let body = if h.kind == BlockKind::Tool { format!("{}/input",h.id) } else { h.id.clone() };
                let bytes = if wanted(h) {tau_blocks::cached_preview(&db,scope,&body,if preview {(*allowance).min(body_budget)} else {MAX_BLOCK_BYTES as usize})?} else {vec![]};
                if preview {body_budget=body_budget.saturating_sub(bytes.len());*allowance=allowance.saturating_sub(bytes.len());}
                event.text = text_prefix(&bytes)?;
                let length = tau_blocks::header(&db,scope,&body)?.map_or(0,|b|b.length);
                sizes.insert(event.id.clone(),length);
                if bytes.len() as u64 != length {incomplete.insert(event.id.clone());}
                if preview && wanted(h) && (length>256*1024 || body_budget==0 || *allowance==0) && (bytes.len() as u64)<length {event.text.push_str("\n\n[Preview limited. Fetch the complete message with Copy.]");}
                if h.kind==BlockKind::Tool {
                    states.insert(event.id.clone(),h.meta.get("toolState").and_then(|v|v.as_str()).unwrap_or(if h.sealed {"running"} else {"writing"}).into());
                }
                if event.text.is_empty() && length > 0 && (event.role != tau_protocol::EventRole::Tool || h.parent.is_none())
                    && event.kind != EventKind::Tool { event.text = "Loading…".into(); }
                events.push(event);
            }
        }
        if include_queue && let Some(h)=tau_blocks::header(&db,scope,QUEUE)? {
            let bytes=tau_blocks::cached_content(&db,scope,QUEUE)?;
            if bytes.len() as u64==h.length && !bytes.is_empty() {queue=serde_json::from_slice(&bytes)?;}
        }
        for h in if include_queue {tau_blocks::children(&db,scope,Some(QUEUE))?} else {vec![]} {
            if let Some(value) = h.meta.get("request") {
                let mut request: tau_protocol::QueuedRequest = serde_json::from_value(value.clone())?;
                let bytes=tau_blocks::cached_content(&db,scope,&h.id)?;
                if bytes.len() as u64 != h.length {incomplete.insert(h.id.clone());}
                request.text = text_prefix(&bytes)?;
                if request.text.is_empty() && h.length > 0 { request.text = "Loading…".into(); }
                queue.requests.push(request);
            }
        }
        queue.available &= tau_blocks::cached_feed(&db,scope,Some(QUEUE))?.is_some_and(|page|page.before.is_none());
        events.sort_by_key(|e|e.order);
        Ok(Some(View { previews:preview_ids.into_iter().map(|(root,ids)| {let used=256*1024-group_budgets[root.as_str()];(root,ids,used)}).collect(),partial:only.is_some(),queue_changed:include_queue,snapshot:TranscriptSnapshot { generation:format!("{}:{scope}",page.cursor.lineage),sequence:page.cursor.sequence,
            events,queue,before:page.before.as_ref().map(|p|p.order),delivered:vec![] },lengths:sizes,incomplete,states }))
    }
    pub fn copy_ready(&self, scope:&str, ids:&[String]) -> Result<Option<String>> {
        ensure!(ids.len()<=4096,"Copy fewer than 4097 sections at a time");
        let mut seen=BTreeSet::new();let ids=ids.iter().filter(|id|seen.insert(*id)).cloned().collect::<Vec<_>>();let ids=ids.as_slice();
        if !copy_complete(&self.db.lock().unwrap(),scope,ids)? {return Ok(None);}
        let view=self.snapshot_inner(scope,None,false,Some(ids),false)?.context("Details are not cached")?;
        let tools=crate::details::Tools::new(view.snapshot.events.iter());
        let group=ids.iter().filter_map(|id|view.snapshot.events.iter().find(|e|&e.id==id)).collect::<Vec<_>>();
        let text=tools.copy(&group);ensure!(text.len() as u64<=MAX_BLOCK_BYTES,"Clipboard output exceeds 64 MiB; copy fewer sections");Ok(Some(text))
    }
    pub fn history_cursor(&self, scope: &str) -> Result<Option<FeedPosition>> {
        Ok(tau_blocks::cached_feed(&self.db.lock().unwrap(),scope,None)?.and_then(|p|p.before))
    }
    #[cfg(test)]
    pub fn plan(&self,scope:&str,local:&LocalChat,copy:&[String])->Result<Plan> {self.plan_visible(scope,local,copy,None)}
    pub fn plan_visible(&self, scope: &str, local: &LocalChat, copy:&[String], viewport:Option<&BTreeSet<String>>) -> Result<Plan> {
        let db = self.db.lock().unwrap();
        let roots = if let Some(viewport)=viewport {
            let ids=viewport.iter().chain(copy.iter()).collect::<BTreeSet<_>>();
            ids.into_iter().filter_map(|id|tau_blocks::header(&db,scope,id).transpose()).collect::<Result<Vec<_>>>()?.into_iter().filter(|h|h.parent.is_none()).collect()
        } else {tau_blocks::children(&db,scope,None)?};
        let mut parents = BTreeSet::from([None,Some(QUEUE.into())]);
        let mut blocks = BTreeSet::from([QUEUE.into()]);
        let mut metadata = roots.clone();
        for root in &roots {metadata.extend(tau_blocks::children(&db,scope,Some(&root.id))?);}
        for id in copy {
            if let Some(h)=tau_blocks::header(&db,scope,id)? && h.parent.is_some() {
                ensure!(h.meta.get("event").is_some(),"Use Download for non-text payloads");
                blocks.insert(id.clone());metadata.push(h);
            }
        }
        let mut events = metadata.iter().filter_map(|h| h.meta.get("event").map(|value|(h,value))).map(|(h,value)| {
            let mut event=native_event(h,value)?;
            if h.length > 0 {event.text = "x".into();}
            Ok(event)
        }).collect::<Result<Vec<_>>>()?;
        events.sort_by_key(|e|e.order);
        let visible = viewport.map(|ids|ids.iter().cloned().collect()).unwrap_or_else(||crate::details::open_items(events.iter(),local));
        let recent=roots.iter().rev().filter(|h|h.kind==BlockKind::Text && h.meta.pointer("/event/role").and_then(|v|v.as_str())!=Some("tool") && h.length>0)
            .take(2).map(|h|h.id.clone()).collect::<BTreeSet<_>>();
        // Bootstrap without a renderer is a bounded recent window. Once the
        // renderer owns interests, only viewport + one-screen overscan is read.
        let window=viewport.cloned().unwrap_or_else(||roots.iter().rev().filter(|h|h.id!=QUEUE).take(32).map(|h|h.id.clone()).collect());
        let pending_copy=copy.iter().filter(|id|!copy_complete(&db,scope,std::slice::from_ref(*id)).unwrap_or(false)).cloned().collect::<BTreeSet<_>>();
        let mut roots=roots;
        roots.sort_by_key(|h|(!pending_copy.contains(&h.id),std::cmp::Reverse(h.order)));
        let mut admitted=0;
        for h in roots {
            if h.id == QUEUE { continue; }
            if !pending_copy.contains(&h.id) && (!window.contains(&h.id) || copy.contains(&h.id)) {continue;}
            if admitted>=30 {continue;}admitted+=1;
            if h.kind == BlockKind::Tool {
                let call=h.parent.as_deref().unwrap_or(&h.id);
                let key = format!("tool:{call}");
                if copy.contains(&h.id) || visible.contains(&h.id) && local.expansion.get(&key).copied().unwrap_or(false) {
                    parents.insert(Some(h.id.clone()));
                    for child in tau_blocks::children(&db,scope,Some(&h.id))? {
                        if child.meta.get("attachment").is_some() { continue; }
                        let input = child.meta.get("inputFor").is_some();
                        let label = if input { "Input" } else if child.meta.pointer("/event/isError").and_then(|v|v.as_bool()) == Some(true) { "Error" } else { "Output" };
                        if copy.contains(&h.id) || child.length <= 1200 || local.expansion.get(&format!("{key}:{label}")).copied().unwrap_or(false) {
                            blocks.insert(child.id);
                        }
                    }
                }
            } else if h.meta.get("event").is_some() {
                let is_tool = h.meta.pointer("/event/role").and_then(|v|v.as_str()) == Some("tool");
                if is_tool {
                    let call=h.parent.as_deref().unwrap_or(&h.id);
                    let key = format!("tool:{call}");
                    let label = if h.meta.pointer("/event/isError").and_then(|v|v.as_bool()) == Some(true) {"Error"} else {"Output"};
                    if copy.contains(&h.id) || visible.contains(&h.id) && local.expansion.get(&key).copied().unwrap_or(false)
                        && (h.length <= 1200 || local.expansion.get(&format!("{key}:{label}")).copied().unwrap_or(false)) {blocks.insert(h.id);}
                } else if copy.contains(&h.id) || h.kind != BlockKind::Thinking || visible.contains(&h.id) {blocks.insert(h.id);}
            }
        }
        for h in tau_blocks::children(&db,scope,Some(QUEUE))? { blocks.insert(h.id); }
        // Full captions/errors/names are explicit metadata-body interests too.
        // Closed tool cards still cause no child or body requests.
        for h in &metadata {
            if (blocks.contains(&h.id) || parents.contains(&Some(h.id.clone())) || copy.contains(&h.id)
                || window.contains(&h.id) && h.meta.pointer("/event/attachment").is_some_and(|v|v.is_object()))
                && let Some(reference)=h.meta.get("fullEvent")
                && (copy.contains(&h.id) || h.parent.as_ref().is_some_and(|id|copy.contains(id)) || metadata_length(&db,scope,reference)?.unwrap_or(0)<=256*1024)
                && let Some(id)=reference["id"].as_str() {blocks.insert(id.into());}
        }
        // Bound the active body working set independently of watcher count.
        // Downloads (two <=50 MB files) and two <=64 MiB descriptors have room
        // alongside this 128 MiB viewport budget in the 512 MiB body cache.
        let mut candidates=blocks.iter().cloned().collect::<Vec<_>>();
        candidates.sort_by_key(|id|(!(*id==QUEUE || recent.contains(id)),id.clone()));
        let mut budget=128*1024*1024u64;let mut admitted=BTreeSet::new();
        for id in candidates {
            let h=tau_blocks::header(&db,scope,&id)?;
            let complete=copy.contains(&id) || h.as_ref().and_then(|h|h.parent.as_ref()).is_some_and(|parent|copy.contains(parent))
                || id==QUEUE || h.as_ref().and_then(|h|h.parent.as_deref())==Some(QUEUE);
            let mut length=h.map_or(0,|h|h.length);
            if !complete && length>256*1024 {
                if tau_blocks::cache_budget::stored_bytes(&db,scope,&id)?>=256*1024 {continue;}
                length=256*1024;
            }
            if length<=budget {budget-=length;admitted.insert(id);}
        }
        blocks=admitted;
        // Include content heads in the plan so a sealed block's replacement can
        // restart its watch even if the set of IDs did not change.
        let heads = blocks.iter().map(|id| Ok((id.clone(),tau_blocks::header(&db,scope,id)?.map(|h|(h.version,h.length,h.sealed,tau_blocks::cache_budget::stored_bytes(&db,scope,id).unwrap_or(0))))))
            .collect::<Result<Vec<_>>>()?;
        let mut older=vec![];
        for parent in parents.iter().flatten() {
            if let Some(before)=tau_blocks::cached_feed(&db,scope,Some(parent))?.and_then(|p|p.before) {older.push((parent.clone(),before));}
        }
        let foreground=blocks.iter().filter(|id|*id==QUEUE || recent.contains(*id) || tau_blocks::header(&db,scope,id).ok().flatten().is_some_and(|h|!h.sealed && h.kind!=BlockKind::Code))
            .cloned().collect();
        Ok(Plan { scope:scope.into(),parents,blocks:heads,older,foreground })
    }
}
fn native_event(h:&BlockHeader,value:&serde_json::Value)->Result<Event> {
    let mut event:Event=serde_json::from_value(value.clone())?;
    // Display pairing follows the native tree, not provider IDs that may repeat
    // across turns. Full provider IDs remain untouched in canonical context.
    if event.kind==EventKind::Tool || event.role==tau_protocol::EventRole::Tool {event.tool_call_id=Some(h.parent.as_ref().unwrap_or(&h.id).clone());}
    Ok(event)
}
fn replica_epoch(db:&Connection)->Result<u64> {Ok(db.query_row("SELECT epoch FROM replica_epoch WHERE singleton=1",[],|r|r.get(0))?)}
fn metadata_length(db:&Connection,scope:&str,reference:&serde_json::Value)->Result<Option<u64>> {
    let id=reference["id"].as_str().context("Invalid metadata reference")?;
    Ok(match (reference["length"].as_u64(),tau_blocks::header(db,scope,id)?.map(|h|h.length)) {
        (Some(a),Some(b))=>Some(a.max(b)),(a,b)=>a.or(b),
    })
}
fn copy_complete(db:&Connection,scope:&str,ids:&[String])->Result<bool> {
    let mut ready=true;let mut bodies=std::collections::BTreeMap::new();let mut metadata=std::collections::BTreeMap::new();
    let mut inspect=|h:&BlockHeader|->Result<()> {
        if let Some(reference)=h.meta.get("fullEvent") {metadata.insert(reference["id"].as_str().context("Invalid metadata reference")?.to_owned(),reference.clone());}Ok(())
    };
    for id in ids {
        let h=tau_blocks::header(db,scope,id)?.context("Details block no longer exists")?;
        ready&=h.sealed && (h.kind!=BlockKind::Tool || matches!(h.meta["toolState"].as_str(),Some("completed"|"failed"|"interrupted")));
        inspect(&h)?;
        if h.kind==BlockKind::Tool {
            ready&=tau_blocks::cached_feed(db,scope,Some(id))?.is_some_and(|p|p.before.is_none() && p.cursor.sequence>=h.revision);
            for child in tau_blocks::children(db,scope,Some(id))? {
                if child.meta.get("inputFor").is_some() || child.meta.pointer("/event/kind").and_then(|v|v.as_str())==Some("text") {
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
    for h in bodies.values() {ready&=h.sealed && tau_blocks::cache_budget::stored_bytes(db,scope,&h.id)?==h.length;}
    if !ready {return Ok(false);}
    for (id,reference) in metadata {
        let bytes=tau_blocks::cached_content(db,scope,&id)?;
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
pub struct Plan { pub scope: String, pub parents: BTreeSet<Option<String>>, pub blocks: Vec<(String,Option<(u64,u64,bool,u64)>)>, pub older:Vec<(String,FeedPosition)>, pub foreground:BTreeSet<String> }
pub enum Command { Reset, Configure(BulkOffer,String), Plan(Option<Plan>), History { scope:String,before:FeedPosition } }
pub struct Notice { pub scope: String, pub error: Option<String>, pub transfer:Option<(String,std::path::PathBuf,tau_transfer::TransferStatus)> }
pub struct Service { tx: mpsc::Sender<Command>, plans:watch::Sender<Option<Plan>>, configuration:watch::Sender<Option<(BulkOffer,String)>>, pub node:watch::Receiver<Option<String>>, downloads:files::Downloads, task:tokio::task::JoinHandle<()> }
impl Service {
    pub fn start(cache: Cache, wake: crate::transport::Wake, notices:mpsc::Sender<Notice>) -> Self {
        // Only coalesced plans/configuration and explicit history requests enter
        // this queue; content never passes through it or the control event queue.
        let (tx,rx) = mpsc::channel(8);
        let (plans,plan_rx)=watch::channel(None);let (configuration,config_rx)=watch::channel(None);
        let (node,identity) = watch::channel(None);
        let (endpoint,client)=watch::channel(None); let (ready,lineage)=watch::channel(None);
        let downloads=files::Downloads {cache:cache.clone(),client,ready:lineage,notices:notices.clone(),wake:wake.clone()};
        let task = tokio::spawn(run(cache,wake,rx,plan_rx,config_rx,notices,node,endpoint,ready));
        Self { tx,plans,configuration,node:identity,downloads,task }
    }
    pub fn stats(&self)->Option<tau_transfer::blocks::Stats> {self.downloads.client.borrow().as_ref().map(|client|client.stats())}
    pub(crate) fn downloads(&self) -> files::Downloads { self.downloads.clone() }
    pub fn send(&self, command: Command) {
        match command {
            Command::Plan(plan)=>{self.plans.send_replace(plan);}
            Command::Configure(offer,host)=>{self.configuration.send_replace(Some((offer,host)));}
            history@(Command::History {..}|Command::Reset)=>{
                if let Err(error)=self.tx.try_send(history) && let Command::History {scope,..}=error.into_inner() {
                    let _=self.downloads.notices.try_send(Notice {scope,error:Some("History queue is busy; try again".into()),transfer:None});(self.downloads.wake)();
                }
            }
        }
    }
}
impl Drop for Service { fn drop(&mut self) { self.task.abort(); } }

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Key { Feeds(String,Vec<Option<String>>), Block(String,String,bool), History(String,Option<String>,FeedPosition) }
struct Watches(HashMap<Key,tokio::task::JoinHandle<()>>);
impl std::ops::Deref for Watches { type Target = HashMap<Key,tokio::task::JoinHandle<()>>; fn deref(&self) -> &Self::Target { &self.0 } }
impl std::ops::DerefMut for Watches { fn deref_mut(&mut self) -> &mut Self::Target { &mut self.0 } }
impl Drop for Watches { fn drop(&mut self) { for task in self.0.values() { task.abort(); } } }
async fn run(cache: Cache, wake: crate::transport::Wake, mut commands:mpsc::Receiver<Command>, mut plans:watch::Receiver<Option<Plan>>, mut configuration:watch::Receiver<Option<(BulkOffer,String)>>, notices:mpsc::Sender<Notice>, identity:watch::Sender<Option<String>>, endpoint:watch::Sender<Option<Arc<Client>>>, ready:watch::Sender<Option<String>>) {
    let client = match Client::bind().await { Ok(client) => Arc::new(client), Err(error) => {
        let _ = notices.send(Notice {transfer:None,scope:String::new(),error:Some(error.to_string())}).await; (wake)(); return;
    }};
    endpoint.send_replace(Some(client.clone()));
    identity.send_replace(Some(client.node_id())); (wake)();
    let mut jobs = Watches(HashMap::new());
    let mut plan = None;
    loop {
        let command=tokio::select! {
            command=commands.recv()=>{let Some(command)=command else {break;};command}
            result=plans.changed()=>{if result.is_err() {break;}Command::Plan(plans.borrow_and_update().clone())}
            result=configuration.changed()=>{if result.is_err() {break;}let Some((offer,host))=configuration.borrow_and_update().clone() else {continue;};Command::Configure(offer,host)}
        };
        match command {
            Command::Reset=>{for job in jobs.values() {job.abort();}jobs.clear();}
            Command::Configure(offer,host) => {
                if let Err(error) = client.configure(&offer,&host).await {
                    let _ = notices.send(Notice {transfer:None,scope:String::new(),error:Some(error.to_string())}).await; (wake)();
                } else if let Err(error) = cache.configure(&offer.lineage) {
                    let _ = notices.send(Notice {transfer:None,scope:String::new(),error:Some(error.to_string())}).await; (wake)();
                } else {
                    if ready.borrow().as_ref().is_some_and(|old|old != &offer.lineage) { for task in jobs.values() {task.abort();} jobs.clear(); }
                    ready.send_replace(Some(offer.lineage));
                }
            }
            Command::Plan(next) => { plan = next; }
            Command::History { scope,before } => {
                let key = Key::History(scope,None,before);
                if jobs.get(&key).is_some_and(|task|task.is_finished()) {jobs.remove(&key);}
                if !jobs.contains_key(&key) { jobs.insert(key.clone(),spawn_watch(key,client.clone(),cache.clone(),ready.subscribe(),notices.clone(),wake.clone())); }
            }
        }
        let mut desired=std::collections::HashSet::new();
        if let Some(p)=&plan {
            // Batch headers, with a hard encoded request budget. Many expanded
            // cards must not consume every stream and starve their own bytes.
            let mut parents=vec![]; let mut requests=vec![];
            for parent in &p.parents {
                let Ok(request)=cache.feed_request(&p.scope,parent.as_deref(),None) else {continue;};
                let mut proposed=requests.clone();proposed.push(request.clone());
                if proposed.len()>16 || serde_json::to_vec(&BlockWatch::Feeds {requests:proposed}).map_or(true,|b|b.len()>MAX_BLOCK_HEADER_BYTES) {
                    desired.insert(Key::Feeds(p.scope.clone(),std::mem::take(&mut parents)));requests.clear();
                }
                parents.push(parent.clone());requests.push(request);
            }
            if !parents.is_empty() {desired.insert(Key::Feeds(p.scope.clone(),parents));}
            for (id,_) in &p.blocks {desired.insert(Key::Block(p.scope.clone(),id.clone(),p.foreground.contains(id)));}
            for (parent,before) in &p.older {desired.insert(Key::History(p.scope.clone(),Some(parent.clone()),before.clone()));}
        }
        jobs.retain(|key,task| {
            let keep=if let Key::History(scope,None,_)=key {!task.is_finished() && plan.as_ref().is_some_and(|p|&p.scope==scope)} else {desired.contains(key)};
            if !keep {task.abort();}keep
        });
        for key in desired {
            let completed=jobs.get(&key).is_some_and(|t|t.is_finished());
            let needed=if let Key::Block(scope,id,_)=&key {
                let db=cache.db.lock().unwrap();
                tau_blocks::header(&db,scope,id).ok().flatten().is_none_or(|h|!h.sealed ||
                    tau_blocks::cached_content(&db,scope,id).map_or(true,|bytes|bytes.len() as u64!=h.length))
            } else {true};
            if (!jobs.contains_key(&key) || completed) && needed {
                jobs.insert(key.clone(),spawn_watch(key,client.clone(),cache.clone(),ready.subscribe(),notices.clone(),wake.clone()));
            }
        }
    }
    for task in jobs.values() { task.abort(); }
    client.shutdown().await;
}

fn spawn_watch(key: Key, client: Arc<Client>, cache: Cache, mut ready: watch::Receiver<Option<String>>, notices:mpsc::Sender<Notice>, wake:crate::transport::Wake) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let scope = match &key { Key::Feeds(scope,_) | Key::Block(scope,_,_) | Key::History(scope,_,_) => scope.clone() };
        while ready.borrow().is_none() { if ready.changed().await.is_err() { return; } }
        let mut delay = 100;
        loop {
            let lineage = ready.borrow().clone().unwrap();
            let result = watch_once(&key,&client,&cache,&lineage,&notices,&wake).await;
            match result {
                Ok(true) => return,
                Ok(false) => continue, // Yield the class permit to queued interests.
                Err(error) => {
                    if notices.send(Notice {transfer:None,scope:scope.clone(),error:Some(format!("Content sync: {error}"))}).await.is_err() { return; }
                    (wake)();
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                    delay = (delay*2).min(5000);
                }
            }
        }
    })
}

async fn watch_once(key: &Key, client: &Client, cache: &Cache, lineage:&str, notices: &mpsc::Sender<Notice>, wake: &crate::transport::Wake) -> Result<bool> {
    let epoch=cache.epoch();
    let (scope,request) = match key {
        Key::Feeds(scope,parents) => (scope.clone(),BlockWatch::Feeds {requests:parents.iter().map(|parent|cache.feed_request(scope,parent.as_deref(),None)).collect::<Result<_>>()?}),
        Key::History(scope,parent,before) => (scope.clone(),BlockWatch::Feed(cache.feed_request(scope,parent.as_deref(),Some(before.clone()))?)),
        Key::Block(scope,id,_) => (scope.clone(),BlockWatch::Block(cache.block_request(scope,id)?)),
    };
    let feeds=match &request {BlockWatch::Feed(req)=>vec![req.clone()],BlockWatch::Feeds {requests}=>requests.clone(),BlockWatch::Block(_)=>vec![]};
    let bulk=matches!(key,Key::Block(_,_,false));
    let mut watcher = if bulk {client.watch_bulk(request.clone()).await?} else {client.watch(request.clone()).await?};
    let mut records = vec![vec![];feeds.len()]; let mut head = None;
    // A live watch cannot own a class permit forever under fanout. Renewal uses
    // committed cursors/verified prefixes; no received body prefix is replayed.
    let lease=tokio::time::Instant::now()+Duration::from_secs(5);
    loop {
        let (frame,wire_bytes) = tokio::select! {
            result=watcher.next()=>result?,
            _=tokio::time::sleep_until(lease)=>return Ok(false),
        };
        match &frame.header {
            Header::Record { watch,record } => {
                let records=records.get_mut(*watch).context("Unrequested feed")?;
                ensure!(records.len()<MAX_FEED_PAGE,"Unexpected feed record");records.push(record.clone());
            }
            Header::Page {watch,reset,cursor,floor,before,more} => {
                let req=feeds.get(*watch).context("Unrequested feed")?;
                let page = FeedPage {reset:*reset,records:std::mem::take(&mut records[*watch]),cursor:cursor.clone(),floor:*floor,before:before.clone(),more:*more};
                let cache = cache.clone(); let req = req.clone(); let lineage = lineage.to_owned();
                tokio::task::spawn_blocking(move ||cache.page_at(&lineage,&req,&page,epoch)).await??;
                notices.send(Notice {transfer:None,scope:scope.clone(),error:None}).await?; (wake)();
            }
            Header::Block {block} => {
                let BlockWatch::Block(req) = &request else { anyhow::bail!("Unexpected block header"); };
                ensure!(block.id == req.id,"Peer sent an unrequested block");
                let cache = cache.clone(); let scope2 = scope.clone(); let h = block.clone(); let lineage = lineage.to_owned();
                tokio::task::spawn_blocking(move ||cache.header_at(&lineage,&scope2,&h,epoch)).await??;
                head = Some(block.clone());
                notices.send(Notice {transfer:None,scope:scope.clone(),error:None}).await?; (wake)();
            }
            Header::Data {version,offset,hash,..} => {
                let h: &BlockHeader = head.as_ref().context("Content without a requested header")?;
                ensure!(h.version == *version,"Unexpected content version");
                let range = ContentRange {header:h.clone(),offset:*offset,hash:hash.clone(),bytes:frame.decoded()?};
                let cache = cache.clone(); let scope2 = scope.clone(); let lineage = lineage.to_owned();
                tokio::task::spawn_blocking(move ||cache.range_at(&lineage,&scope2,&range,epoch)).await??;
                notices.send(Notice {transfer:None,scope:scope.clone(),error:None}).await?; (wake)();
            }
            Header::End => return Ok(true),
            Header::Error {message} => anyhow::bail!("{message}"),
            _ => anyhow::bail!("Unexpected block response"),
        }
        // A peer may finish its credit half while the final bounded response is
        // still being consumed. Read-side errors/End determine stream completion.
        let _ = watcher.consumed(wire_bytes).await;
        if cache.epoch()!=epoch {return Ok(false);}
    }
}

#[cfg(test)]
mod tests;
