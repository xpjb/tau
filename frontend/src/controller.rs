use crate::{
    feed::Feed,
    store::*,
    transport::{self, Command, Network, Wake},
};
use anyhow::{Context, Result, ensure};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};
use tau_protocol::*;

pub struct Download {
    pub status: tau_transfer::TransferStatus,
    pub path: PathBuf,
    pub bytes_per_second: Option<u64>,
    last_progress: Option<(std::time::Instant, u64)>,
}
impl Download {
    pub fn new(status: tau_transfer::TransferStatus, path: PathBuf) -> Self {
        let last_progress = (!status.done).then(|| (std::time::Instant::now(), status.transferred));
        Self { status, path, bytes_per_second: None, last_progress }
    }
    fn update(&mut self, status: tau_transfer::TransferStatus, path: PathBuf) {
        let now = std::time::Instant::now();
        if status.done || self.path != path || status.transferred < self.status.transferred {
            self.bytes_per_second = None;
            self.last_progress = None;
        } else if let Some((at, bytes)) = self.last_progress {
            let elapsed = now.duration_since(at);
            if elapsed >= std::time::Duration::from_millis(100) {
                self.bytes_per_second = (status.transferred > bytes).then(||
                    (u128::from(status.transferred - bytes) * 1000 / elapsed.as_millis().max(1)).min(u128::from(u64::MAX)) as u64);
            }
        }
        if !status.done && (self.last_progress.is_none() || now.duration_since(self.last_progress.unwrap().0) >= std::time::Duration::from_millis(100)) {
            self.last_progress = Some((now, status.transferred));
        }
        self.status = status;
        self.path = path;
    }
}
pub struct Chat {
    pub local: LocalChat,
    pub feed: Feed,
    pub commands: Vec<SlashCommand>,
    pub commands_loaded: bool,
    pub model_request: Option<(String, String)>,
}
#[derive(Default)]
struct Catalog {
    id:String,revision:Option<u64>,session_after:Option<String>,project_after:Option<String>,
    sessions_done:bool,projects_done:bool,refresh:bool,
    sessions:Vec<SessionSummary>,projects:Vec<Project>,
}
pub struct Controller {
    pub store: Store,
    pub settings: Settings,
    pub identity: String,
    pub account: Account,
    pub model_preferences: crate::models::Preferences,
    pub chats: HashMap<String, Chat>,
    pub downloads: HashMap<String, Download>,
    saved_downloads: HashMap<String, Option<SavedDownload>>,
    pub viewing_chat: bool,
    pub project_result: Option<(String, bool)>,
    pub settings_result: Option<(String, bool)>,
    pub daemon_settings: Option<tau_protocol::settings::Settings>,
    pub connection: String,
    pub health: crate::connection::Health,
    pub native_metrics:tau_transfer::blocks::Stats,
    pub epoch: Option<u64>,
    pub notice: Option<String>,
    remote: crate::blocks::Cache,
    block_plan: Option<crate::blocks::Plan>,
    plan_dirty:std::cell::Cell<bool>,
    viewport:Option<(String,std::collections::BTreeSet<String>)>,
    copy:Option<(String,Vec<String>)>,
    pub copied:Option<String>,
    network: Option<Network>,
    requests: HashMap<String, ClientCommand>,
    project_deletions: HashMap<String, Vec<String>>,
    create_failed_epoch: Option<u64>,
    control_check:Option<std::time::Instant>,
    control_cursor:usize,
    catalog:Option<Catalog>,state_versions:HashMap<String,(u64,SessionStatus,Option<String>,Option<ContextUsage>)>,
    receipt_queue:std::collections::VecDeque<(String,Vec<String>)>,receipt_inflight:Option<(String,String,Vec<String>,std::time::Instant)>,
    source_guard:Option<(u64,bool)>, pub restore_reviews:std::collections::HashSet<String>,
    wake: Wake,
}
impl Controller {
    pub fn new(store: Store, wake: Wake) -> Result<Self> {
        let settings: Settings = store.get("", "settings")?;
        let identity = settings.identity();
        let mut account:Account = store.get(&identity, "account")?;
        let model_preferences = store.get(&identity, "quick-models")?;
        let remote = store.block_cache(&identity)?;
        if account.source_lineage.is_none() && let Some(previous)=remote.previous_source()? {store.bind_source(&identity,&previous)?;account=store.get(&identity,"account")?;}
        let mut c = Self {
            store,
            settings,
            identity,
            account,
            model_preferences,
            chats: HashMap::new(),
            downloads: HashMap::new(),
            saved_downloads: HashMap::new(),
            viewing_chat: true,
            project_result: None,
            settings_result: None,
            daemon_settings: None,
            connection: "Not connected".into(),
            health: crate::connection::Health::default(),native_metrics:Default::default(),
            epoch: None,
            notice: None,
            remote,
            block_plan: None,plan_dirty:std::cell::Cell::new(true),
            viewport:None,
            copy:None, copied:None,
            network: None,
            requests: HashMap::new(),
            project_deletions: HashMap::new(),
            create_failed_epoch: None,
            receipt_queue:Default::default(),receipt_inflight:None,control_check:None,control_cursor:0,catalog:None,state_versions:HashMap::new(),source_guard:None,restore_reviews:Default::default(),
            wake,
        };
        if let Some(id) = c.account.selected.clone() {
            c.ensure_chat(&id)?;
        }
        if let Some(provisional)=c.account.pending_create.as_ref().map(|p|p.id.clone()) {
            let resolved=c.store.resolve_chat(&c.identity,&provisional)?;
            if resolved!=provisional {c.ensure_chat(&provisional)?;c.finish_create(&provisional,&resolved)?;}
        }
        if c.settings.url().is_ok() {
            c.connect();
        }
        Ok(c)
    }
    pub fn connect(&mut self) {
        self.epoch = None;
        self.connection = "Connecting…".into();
        self.health = crate::connection::Health::connecting();
        self.block_plan = None;self.plan_dirty.set(true);
        self.requests.clear();
        self.project_deletions.clear();
        self.network = Some(Network::start_cached(self.settings.clone(), self.wake.clone(),self.remote.clone()));
    }
    pub fn configure(&mut self, settings: Settings) -> Result<()> {
        let settings = settings.normalized();
        settings.url()?;
        self.store.put("", "settings", &settings)?;
        self.network = None;
        self.settings = settings;
        self.identity = self.settings.identity();
        self.remote = self.store.block_cache(&self.identity)?;
        self.block_plan = None;self.plan_dirty.set(true);
        self.viewport=None;
        self.copy=None;self.copied=None;
        self.account = self.store.get(&self.identity, "account")?;
        self.model_preferences = self.store.get(&self.identity, "quick-models")?;
        self.chats.clear();
        self.downloads.clear();
        self.saved_downloads.clear();
        self.requests.clear();
        self.project_deletions.clear();
        self.create_failed_epoch = None;
        self.daemon_settings = None;
        self.settings_result = None;
        self.project_result = None;
        self.notice = None;
        if let Some(id) = self.account.selected.clone() {
            self.ensure_chat(&id)?;
        }
        self.connect();
        Ok(())
    }
    pub fn selected(&self) -> Option<&Chat> {
        self.account
            .selected
            .as_ref()
            .and_then(|id| self.chats.get(id))
    }
    pub fn ensure_chat(&mut self, id: &str) -> Result<()> {
        if !self.chats.contains_key(id) {
            let local = self.store.load_chat(&self.identity, id)?;
            let mut feed = Feed::default();
            if let Some(view) = self.remote.preview(id,None)? {feed.snapshot(view.snapshot)?;feed.block_lengths=view.lengths;feed.incomplete=view.incomplete;feed.block_states=view.states;feed.synchronized=false;}
            self.chats.insert(
                id.to_owned(),
                Chat {
                    local,
                    feed,
                    commands: vec![],
                    commands_loaded: false,
                    model_request: None,
                },
            );
        }
        Ok(())
    }
    pub fn save_chat(&self, id: &str) -> Result<()> {
        self.plan_dirty.set(true);
        if let Some(chat) = self.chats.get(id) {
            self.store.save_chat(&self.identity, id, &chat.local)?;
        }
        Ok(())
    }
    pub fn select(&mut self, id: &str) -> Result<()> {
        self.plan_dirty.set(true);
        let native=self.chats.keys().filter_map(|old|self.remote.has_snapshot(old).ok().filter(|yes|*yes).map(|_|old.clone())).collect::<std::collections::HashSet<_>>();
        self.chats.retain(|old,chat|!native.contains(old) || old==id || chat.local.has_work());
        for (old,chat) in &mut self.chats {if old!=id && native.contains(old) {chat.feed=Feed::default();chat.commands.clear();chat.commands_loaded=false;}}
        self.ensure_chat(id)?;
        self.account.selected = Some(id.into());
        if let Some(session) = self.account.sessions.iter().find(|s| s.id == id) {
            self.account.selected_project = session.project_id.clone();
            self.account.last_chat_by_project.insert(session.project_id.clone(), id.into());
            self.account
                .read_at
                .insert(id.into(), session.updated_at_ms);
        }
        self.store.put(&self.identity, "account", &self.account)?;
        if self.epoch.is_some() && !self.is_creating(id) && !self.account.missing_chats.contains(id) {
            self.open(id)?;
            self.request(ClientCommand::GetCommands {
                session_id: id.into(),
            })?;
        }
        Ok(())
    }
    pub fn viewing(&mut self, visible: bool) -> Result<()> {
        let changed = visible && !self.viewing_chat;
        self.viewing_chat = visible;
        if changed && let Some(id) = &self.account.selected
            && let Some(s) = self.account.sessions.iter().find(|s| &s.id == id) {
            self.account.read_at.insert(id.clone(), s.updated_at_ms);
            self.store.put(&self.identity, "account", &self.account)?;
        }
        Ok(())
    }
    fn last_chat_in_project(&self, project: &str) -> Option<String> {
        self.account.last_chat_by_project.get(project)
            .filter(|id| self.account.sessions.iter().any(|s| s.project_id == project && s.id == id.as_str()))
            .cloned()
            .or_else(|| self.account.sessions.iter().filter(|s| s.project_id == project)
                .max_by_key(|s| (s.updated_at_ms, &s.id)).map(|s| s.id.clone()))
    }
    pub fn select_project(&mut self, id: &str) -> Result<()> {
        self.plan_dirty.set(true);
        // Older local accounts remember only a single selected chat; capture it
        // before replacing the global selection with this topic's resume target.
        if let Some(current) = &self.account.selected
            && let Some(session) = self.account.sessions.iter().find(|s| &s.id == current) {
            self.account.last_chat_by_project.insert(session.project_id.clone(), current.clone());
        }
        self.account.selected_project = id.into();
        self.account.selected = self.last_chat_in_project(id);
        // Tab selection itself is not a read receipt. The app marks the chat
        // read only when its pane is actually visible (including after restart).
        self.viewing_chat = false;
        self.store.put(&self.identity, "account", &self.account)?;
        if let Some(chat) = self.account.selected.clone() {
            self.ensure_chat(&chat)?;
            if self.epoch.is_some() && !self.is_creating(&chat) && !self.account.missing_chats.contains(&chat) {
                self.open(&chat)?;
                self.request(ClientCommand::GetCommands { session_id: chat })?;
            }
        }
        Ok(())
    }
    pub fn unread(&self, session: &SessionSummary) -> bool {
        !session.starter && self.account.read_at.get(&session.id).copied().unwrap_or(0) < session.updated_at_ms
    }
    pub fn project_unread(&self, id: &str) -> bool {
        self.account.sessions.iter().any(|s| s.project_id == id && self.unread(s))
    }
    pub fn draft(&mut self, value: String) -> Result<()> {
        let id = self
            .account
            .selected
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Select a chat"))?;
        ensure!(value.len() <= MAX_REQUEST_BYTES, "Draft is too large");
        self.chats.get_mut(&id).unwrap().local.draft = value;
        self.save_chat(&id)
    }
    pub fn attach(&mut self, source: &Path, name: Option<&str>) -> Result<()> {
        let id = self
            .account
            .selected
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Select a chat first"))?;
        self.attach_to(&self.identity.clone(), &id, source, name)
    }
    pub fn attach_to(
        &mut self,
        identity: &str,
        session: &str,
        source: &Path,
        name: Option<&str>,
    ) -> Result<()> {
        ensure!(
            identity == self.identity,
            "Connection changed while picking a file; choose it again"
        );
        let id=self.store.resolve_chat(identity,session)?;
        self.ensure_chat(&id)?;
        let chat = &self.chats[&id];
        ensure!(
            chat.local.files.len() < 8,
            "At most eight files per message"
        );
        ensure!(
            chat.local
                .files
                .iter()
                .map(|f| f.size)
                .sum::<u64>()
                .saturating_add(source.metadata()?.len())
                <= MAX_UPLOAD_BYTES as u64,
            "Attachments exceed 50 MB total"
        );
        let file = self.store.import(&self.identity, &id, source, name)?;
        if chat.local.files.iter().map(|f| f.size).sum::<u64>() + file.size
            > MAX_UPLOAD_BYTES as u64
        {
            let _ = std::fs::remove_file(&file.path);
            anyhow::bail!("Attachments exceed 50 MB total");
        }
        let mut replacement=chat.local.clone();replacement.files.push(file.clone());
        if let Err(error)=self.store.save_chat(&self.identity,&id,&replacement) {let _=self.store.discard_import(&self.identity,&id,&file);return Err(error);}
        self.chats.get_mut(&id).unwrap().local=replacement;self.plan_dirty.set(true);Ok(())
    }
    pub fn remove_file(&mut self, file: &str) -> Result<()> {
        let id = self
            .account
            .selected
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Select a chat"))?;
        let mut replacement=self.chats[&id].local.clone();
        let removed=replacement.files.iter().find(|f|f.id==file).cloned();
        replacement.files.retain(|f|f.id!=file);
        self.store.save_chat(&self.identity,&id,&replacement)?;
        self.chats.get_mut(&id).unwrap().local=replacement;
        if let Some(file)=removed && !self.chats[&id].local.pending.iter().any(|p|p.files.iter().any(|f|f.id==file.id)) {self.store.discard_import(&self.identity,&id,&file)?;}
        self.plan_dirty.set(true);Ok(())
    }
    pub fn send_prompt(&mut self) -> Result<()> {
        let session = self.account.selected.clone().ok_or_else(|| anyhow::anyhow!("Select a chat"))?;
        ensure!(!self.account.missing_chats.contains(&session),"This source chat is missing. Copy the draft to a new chat; old intents are not resent");
        let provisional = self.is_creating(&session);
        let epoch = self.epoch;
        let chat = self.chats.get_mut(&session).unwrap();
        let choosing = chat.model_request.is_some();
        ensure!(
            !chat.local.draft.trim().is_empty() || !chat.local.files.is_empty(),
            "Write a message or attach a file"
        );
        ensure!(
            chat.local.draft.chars().count() <= MAX_PROMPT_CHARS,
            "Message is too large"
        );
        let text = chat.local.draft.clone();
        let files = chat.local.files.clone();
        let request = ClientRequest {
            id: uuid::Uuid::new_v4().to_string(),
            command: ClientCommand::Prompt {
                session_id: session.clone(),
                text: text.clone(),
            },
        };
        let pending = Pending {
            request: request.clone(),
            started_at_ms: crate::clock::now_ms(),
            text: text.clone(),
            files: files.clone(),
            status: if provisional { Delivery::WaitingForChat }
                else if choosing { Delivery::WaitingForModel }
                else if epoch.is_none() { Delivery::WaitingForConnection }
                else if files.is_empty() {
                Delivery::Sending
            } else {
                Delivery::Preparing
            },
            detail: None,
        };
        let mut replacement = chat.local.clone();
        replacement.pending.push(pending);
        replacement.draft.clear();
        replacement.files.clear();
        replacement.position.follow = true;
        // Commit local work before attempting any network effect. A disk failure
        // leaves both the composer and server untouched.
        self.store
            .save_chat(&self.identity, &session, &replacement)?;
        chat.local = replacement;
        if provisional || choosing || epoch.is_none() { return Ok(()); }
        let epoch = epoch.unwrap();
        let command = if files.is_empty() {
            Command::Request {
                epoch,
                request: request.clone(),
            }
        } else {
            Command::Upload {
                epoch,
                id: request.id.clone(),
                session,
                text,
                files,
            }
        };
        if let Err(error) = self.network.as_ref().unwrap().send(command) {
            self.not_sent(&request.id, &error.to_string())?;
        }
        Ok(())
    }
    pub fn request(&mut self, command: ClientCommand) -> Result<String> {
        let epoch=self.epoch.ok_or_else(||anyhow::anyhow!("Not connected"))?;
        if matches!(command,ClientCommand::ListSessions) && let Some(catalog)=&mut self.catalog && !(catalog.sessions_done && catalog.projects_done) {catalog.refresh=true;return Ok(catalog.id.clone());}
        let durable=command.journalled_control();
        let encoded=serde_json::to_vec(&command)?;
        let retry=durable.then(||self.account.pending_controls.values().find(|saved|
            serde_json::to_vec(&saved.request.command).is_ok_and(|bytes|bytes==encoded)).map(|saved|saved.request.id.clone())).flatten();
        let request=ClientRequest {id:retry.unwrap_or_else(||uuid::Uuid::new_v4().to_string()),command};
        if matches!(request.command,ClientCommand::ListSessions) {self.catalog=Some(Catalog {id:request.id.clone(),..Default::default()});}
        let deleted=if let ClientCommand::DeleteProject {project_id,mode:DeleteProjectMode::DeleteChats,..}=&request.command {
            self.account.sessions.iter().filter(|s|&s.project_id==project_id).map(|s|s.id.clone()).collect()
        } else {vec![]};
        if durable {
            ensure!(self.account.pending_controls.len()<32 || self.account.pending_controls.contains_key(&request.id),"Reconcile outstanding actions before submitting more");
            let mut account=self.account.clone();
            account.pending_controls.insert(request.id.clone(),PendingControl {request:request.clone(),deleted_chats:deleted.clone(),blocked:false,accepted:false});
            ensure!(serde_json::to_vec(&account.pending_controls)?.len()<=16*1024*1024,"Saved action inputs exceed the 16 MiB outbox limit; reconcile them before submitting more");
            self.store.put(&self.identity,"account",&account)?;self.account=account;
        }
        if !deleted.is_empty() {self.project_deletions.insert(request.id.clone(),deleted);}
        if durable || matches!(request.command,ClientCommand::GetSession {..}|ClientCommand::GetSettings|ClientCommand::RefreshModelCatalog {..}) {self.requests.insert(request.id.clone(),request.command.clone());}
        self.network.as_ref().unwrap().send(Command::Request {epoch,request:request.clone()})?;
        Ok(request.id)
    }
    pub fn diagnostics(&self)->String {
        let n=&self.native_metrics;
        format!("{}\n\nNative: {} connects / {} attempts; {} streams ({} active).\nSlots: metadata {}, foreground {}, bulk {}, descriptors {}.\nContent bytes ↑{} ↓{}; Tau frame bytes ↑{} ↓{}.\nResume offsets requested: {}; stream cancels: {}; integrity failures: {}.\nCurrent QUIC: UDP bytes ↑{} ↓{}; lost sent packets {}; RTT {} ms.\nControl RTT includes writer queue time; QUIC counters include retransmissions. Native samples update every five seconds.",self.health.details(&self.connection,std::time::Instant::now()),n.connections,n.connection_attempts,n.streams,n.active_streams,n.metadata_slots,n.foreground_slots,n.bulk_slots,n.descriptor_slots,n.content_tx_bytes,n.content_rx_bytes,n.frame_tx_bytes,n.frame_rx_bytes,n.resumed_bytes,n.cancelled_streams,n.integrity_failures,n.quic_tx_bytes,n.quic_rx_bytes,n.quic_lost_packets,n.quic_rtt_ms)
    }
    pub fn clear_replica(&mut self)->Result<()> {
        self.remote.clear()?;self.block_plan=None;self.plan_dirty.set(true);self.copy=None;
        if let Some(network)=&self.network {network.send(Command::Blocks(crate::blocks::Command::Reset))?;}
        for chat in self.chats.values_mut() {chat.feed=Feed::default();}
        self.watch_blocks()?;Ok(())
    }
    pub fn check_control(&mut self,id:&str)->Result<()> {
        ensure!(self.account.pending_controls.contains_key(id),"Saved action no longer exists");
        self.request(ClientCommand::GetOperation {operation_id:id.into()})?;Ok(())
    }
    pub fn retry_control(&mut self,id:&str)->Result<()> {
        let command=self.account.pending_controls.get(id).context("Saved action no longer exists")?.request.command.clone();
        self.request(command)?;Ok(())
    }
    pub fn forget_control(&mut self,id:&str)->Result<()> {
        let mut account=self.account.clone();account.pending_controls.remove(id);
        self.store.put(&self.identity,"account",&account)?;self.account=account;self.requests.remove(id);self.project_deletions.remove(id);Ok(())
    }
    fn reconcile_controls(&mut self)->Result<()> {
        if self.epoch.is_none() || self.control_check.is_some_and(|at|at.elapsed()<std::time::Duration::from_secs(2)) {return Ok(());}
        self.control_check=Some(std::time::Instant::now());
        let ids=self.account.pending_controls.iter().filter(|(id,p)|!p.blocked && !self.requests.contains_key(*id)).map(|(id,_)|id.clone()).collect::<Vec<_>>();
        if ids.is_empty() {return Ok(());}
        let start=self.control_cursor%ids.len();
        for id in ids.iter().cycle().skip(start).take(4.min(ids.len())) {
            self.request(ClientCommand::GetOperation {operation_id:id.clone()})?;
        }
        self.control_cursor=(start+4)%ids.len();Ok(())
    }
    pub fn control(&mut self,command:ClientCommand)->Result<()> {self.control_id(command).map(|_|())}
    fn control_id(&mut self, command: ClientCommand) -> Result<String> {
        let epoch = self.epoch.ok_or_else(|| anyhow::anyhow!("Not connected"))?;
        let session = match &command {
            ClientCommand::Prompt { session_id, text } if text.starts_with('/') => {
                session_id.clone()
            }
            ClientCommand::QueueControl { session_id, .. }
            | ClientCommand::Abort { session_id } => session_id.clone(),
            _ => anyhow::bail!("Not a durable control"),
        };
        let request = ClientRequest {
            id: uuid::Uuid::new_v4().to_string(),
            command,
        };
        let chat = self
            .chats
            .get_mut(&session)
            .ok_or_else(|| anyhow::anyhow!("Chat has not loaded"))?;
        let mut local = chat.local.clone();
        local.pending.push(Pending {
            request: request.clone(),
            started_at_ms: crate::clock::now_ms(),
            text: match &request.command {
                ClientCommand::Prompt { text, .. } => text.clone(),
                ClientCommand::QueueControl { operation: QueueOperation::Edit { text, .. }, .. } => text.clone(),
                ClientCommand::QueueControl { operation: QueueOperation::Delete { .. }, .. } => "Delete queued message".into(),
                ClientCommand::QueueControl { .. } => "Queue action".into(),
                ClientCommand::Abort { .. } => "Stop requested".into(),
                _ => unreachable!("Only durable controls reach this path"),
            },
            files: vec![],
            status: Delivery::Sending,
            detail: None,
        });
        self.store.save_chat(&self.identity, &session, &local)?;
        chat.local = local;
        self.requests.insert(request.id.clone(),request.command.clone());
        if let Err(error) = self.network.as_ref().unwrap().send(Command::Request {
            epoch,
            request: request.clone(),
        }) {
            self.not_sent(&request.id, &error.to_string())?;
        }
        Ok(request.id)
    }
    pub fn restore_pending(&mut self, id: &str) -> Result<()> {
        let session = self
            .account
            .selected
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Select a chat"))?;
        let chat = self.chats.get_mut(&session).unwrap();
        ensure!(
            chat.local.draft.is_empty() && chat.local.files.is_empty(),
            "Clear the current draft first"
        );
        if let Some(p) = chat.local.pending.iter().find(|p| p.request.id == id) {
            ensure!(
                matches!(p.request.command, ClientCommand::Prompt { .. }),
                "Controls cannot become drafts"
            );
            chat.local.draft = p.text.clone();
            chat.local.files = p.files.clone();
        }
        // Keep the uncertain receipt until the user explicitly dismisses it.
        // Restoring text is not evidence that the original was not delivered.
        self.save_chat(&session)
    }
    pub fn dismiss_pending(&mut self, id: &str) -> Result<()> {
        let session = self
            .account
            .selected
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Select a chat"))?;
        let mut replacement=self.chats[&session].local.clone();
        let files=replacement.pending.iter().find(|p|p.request.id==id).map(|p|p.files.clone()).unwrap_or_default();
        replacement.pending.retain(|p|p.request.id!=id);
        self.store.save_chat(&self.identity,&session,&replacement)?;
        self.chats.get_mut(&session).unwrap().local=replacement;
        for file in files {let chat=&self.chats[&session].local;if !chat.files.iter().chain(chat.pending.iter().flat_map(|p|&p.files)).any(|f|f.id==file.id) {self.store.discard_import(&self.identity,&session,&file)?;}}
        Ok(())
    }
    pub fn quick_start(&self, id: &str) -> bool {
        self.account
            .sessions
            .iter()
            .any(|s| s.id == id && s.starter)
            && self.chats.get(id).is_some_and(|c| {
                c.feed.synchronized
                    && c.feed.before.is_none()
                    && c.feed.events.values().all(|e| e.role == EventRole::System)
                    && c.feed.queue.requests.is_empty()
                    && c.local.pending.is_empty()
            })
    }
    pub fn save_model_preferences(
        &mut self,
        preferences: crate::models::Preferences,
    ) -> Result<()> {
        self.store
            .put(&self.identity, "quick-models", &preferences)?;
        self.model_preferences = preferences;
        Ok(())
    }
    pub fn choose_model(&mut self, session: &str, selector: &str) -> Result<()> {
        ensure!(
            self.epoch.is_some() && self.quick_start(session),
            "Model tiles are for an untouched, connected new chat"
        );
        let chat = &self.chats[session];
        ensure!(
            chat.model_request.is_none(),
            "Wait for model selection to finish"
        );
        let _: tau_protocol::SessionModel = selector.parse().map_err(anyhow::Error::msg)?;
        let slug = selector.to_owned();
        // /model already persists the last chosen model in the daemon.
        // Send even when it matches this chat: another chat may have changed
        // the remembered default. Only an explicit tile click reaches here.
        // Preserve drafts/attachments and never replay after reconnect.
        let id = self.control_id(ClientCommand::Prompt {
            session_id: session.into(),
            text: format!("/model {slug}"),
        })?;
        self.chats.get_mut(session).unwrap().model_request = Some((id, slug));
        Ok(())
    }
    pub fn cache_ttl(&self, session: &SessionSummary) -> crate::cache_ttl::Estimate {
        crate::cache_ttl::Estimate::from_session(
            session,
            self.chats.get(&session.id).map(|c| &c.feed),
        )
    }
    pub fn is_creating(&self, id: &str) -> bool {
        self.account.pending_create.as_ref().is_some_and(|request| request.id == id)
    }
    pub fn new_chat(&mut self) -> Result<()> {
        self.plan_dirty.set(true);
        ensure!(self.account.pending_create.is_none(), "The previous new chat is still awaiting confirmation");
        let keep_session_id = self.account.selected.clone()
            .filter(|id| self.chats.get(id).is_some_and(|c| c.local.has_work()));
        let request = ClientRequest { id:uuid::Uuid::new_v4().to_string(), command:ClientCommand::CreateSession { keep_session_id, project_id:self.account.selected_project.clone() } };
        let now = crate::clock::now_ms().unwrap_or(0);
        let mut account = self.account.clone();
        account.pending_create = Some(request.clone());account.create_blocked=false;
        account.selected = Some(request.id.clone());
        account.sessions.insert(0, Self::creating_summary(&request.id, &account.selected_project, now));
        account.last_chat_by_project.insert(account.selected_project.clone(), request.id.clone());
        self.store.put(&self.identity, "account", &account)?;
        self.account = account;
        self.ensure_chat(&request.id)?;
        self.retry_create()?;
        Ok(())
    }
    fn creating_summary(id: &str, project: &str, at: u64) -> SessionSummary {
        SessionSummary { id:id.into(), project_id:project.into(), title:"Creating chat…".into(), starter:false, status:SessionStatus::Sleeping,
            detail:Some("Waiting for daemon confirmation".into()), context_usage:None, model:None,
            parent_id:None, created_at_ms:at, updated_at_ms:at }
    }
    pub fn copy_missing_draft(&mut self,id:&str)->Result<()> {
        ensure!(self.account.missing_chats.contains(id),"Chat is not a local recovery");self.ensure_chat(id)?;
        let draft=self.chats[id].local.draft.clone();let files=self.chats[id].local.files.clone();
        ensure!(!draft.is_empty() || !files.is_empty(),"Restore an intended pending message to the draft first; nothing is automatically resent");
        self.account.selected_project=GENERAL_PROJECT_ID.into();self.new_chat()?;
        self.draft(draft)?;let target=self.account.selected.clone().unwrap();
        for original in files {
            let file=self.store.import(&self.identity,&target,&original.path,Some(&original.name))?;
            if original.hash.as_ref().is_some_and(|hash|file.hash.as_ref()!=Some(hash)) {let _=self.store.discard_import(&self.identity,&target,&file);anyhow::bail!("Original attachment changed; recovery draft was not sent");}
            let mut chat=self.chats[&target].local.clone();chat.files.push(file.clone());
            if let Err(e)=self.store.save_chat(&self.identity,&target,&chat) {let _=self.store.discard_import(&self.identity,&target,&file);return Err(e);}
            self.chats.get_mut(&target).unwrap().local=chat;
        }
        self.notice=Some("Copied only the draft and its files. Original intents remain in local recovery. Inspect before explicitly sending.".into());Ok(())
    }
    pub fn forget_missing_chat(&mut self,id:&str)->Result<()> {
        ensure!(self.account.missing_chats.contains(id),"Refusing to forget a live source chat locally");
        self.store.delete_chat(&self.identity,id)?;self.chats.remove(id);self.account.missing_chats.remove(id);self.account.sessions.retain(|s|s.id!=id);
        if self.account.selected.as_deref()==Some(id) {self.account.selected=None;}
        self.store.put(&self.identity,"account",&self.account)?;self.plan_dirty.set(true);Ok(())
    }
    pub fn retry_create_manually(&mut self) -> Result<()> {
        self.account.create_blocked=false;self.store.put(&self.identity,"account",&self.account)?;
        self.create_failed_epoch = None;
        self.retry_create()
    }
    pub fn retry_create(&mut self) -> Result<()> {
        if self.account.create_blocked {return Ok(());}
        let Some(request) = self.account.pending_create.clone() else { return Ok(()); };
        if self.create_failed_epoch == self.epoch && self.epoch.is_some() { return Ok(()); }
        let Some(epoch) = self.epoch else { return Ok(()); };
        if self.requests.contains_key(&request.id) { return Ok(()); }
        if let Err(error) = self.network.as_ref().unwrap().send(Command::Request {epoch, request:request.clone()}) {
            self.notice = Some(format!("New chat saved locally; will retry after reconnect: {error}"));
        } else { self.requests.insert(request.id, request.command); }
        Ok(())
    }
    fn finish_create(&mut self, provisional: &str, confirmed: &str) -> Result<()> {
        self.plan_dirty.set(true);
        if !self.is_creating(provisional) { return Ok(()); }
        let project = match &self.account.pending_create.as_ref().unwrap().command {
            ClientCommand::CreateSession { project_id, .. } => project_id.clone(),
            _ => unreachable!("pending create must be a create request"),
        };
        if provisional != confirmed {
            let source = &self.chats.get(provisional).ok_or_else(|| anyhow::anyhow!("Local new chat is missing"))?.local;
            let target = self.chats.get(confirmed).map(|chat| &chat.local);
            let merged = self.store.merge_chat(&self.identity, provisional, confirmed, source, target)?;
            if let Some(chat) = self.chats.get_mut(confirmed) { chat.local = merged; }
            else {
                let mut chat = self.chats.remove(provisional).unwrap();
                chat.local = merged;
                chat.feed = Feed::default();
                self.chats.insert(confirmed.into(), chat);
            }
            self.chats.remove(provisional);
            self.account.sessions.retain(|s| s.id != provisional);
            if !self.account.sessions.iter().any(|s| s.id == confirmed) {
                self.account.sessions.insert(0, Self::creating_summary(confirmed, &project, crate::clock::now_ms().unwrap_or(0)));
            }
            if self.account.selected.as_deref() == Some(provisional) { self.account.selected = Some(confirmed.into()); }
            for chat in self.account.last_chat_by_project.values_mut() {
                if chat == provisional { *chat = confirmed.into(); }
            }
            if let Some(at) = self.account.read_at.remove(provisional) {
                let current = self.account.read_at.entry(confirmed.into()).or_default();
                *current = (*current).max(at);
            }
        }
        self.account.pending_create = None;
        self.create_failed_epoch = None;
        self.requests.remove(provisional);
        self.store.put(&self.identity, "account", &self.account)?;
        if self.epoch.is_some() { self.open(confirmed)?; self.send_waiting(confirmed, false)?; }
        Ok(())
    }
    fn send_waiting(&mut self, session: &str, model_confirmed: bool) -> Result<()> {
        let Some(epoch) = self.epoch else { return Ok(()); };
        if self.is_creating(session) || self.account.missing_chats.contains(session) {return Ok(());}
        if model_confirmed && let Some(chat)=self.chats.get_mut(session) {
            for p in &mut chat.local.pending {if p.status==Delivery::WaitingForModel {p.status=Delivery::WaitingForConnection;}}
            self.store.save_chat(&self.identity,session,&chat.local)?;
        }
        let active=self.chats.values().flat_map(|c|&c.local.pending).filter(|p|matches!(p.status,Delivery::Sending|Delivery::Preparing)).take(4).count();
        let pending = self.chats.get(session).map(|chat| chat.local.pending.iter()
            .filter(|p|matches!(p.status,Delivery::WaitingForChat|Delivery::WaitingForConnection)).take(4-active)
            .cloned().collect::<Vec<_>>()).unwrap_or_default();
        for p in pending {
            let chat = self.chats.get_mut(session).unwrap();
            if let Some(saved) = chat.local.pending.iter_mut().find(|saved| saved.request.id == p.request.id) {
                saved.status = if p.files.is_empty() {Delivery::Sending} else {Delivery::Preparing};
            }
            self.save_chat(session)?;
            let command = if p.files.is_empty() { Command::Request {epoch, request:p.request.clone()} }
                else { Command::Upload {epoch, id:p.request.id.clone(), session:session.into(), text:p.text.clone(), files:p.files.clone()} };
            if let Err(error) = self.network.as_ref().unwrap().send(command) { self.not_sent(&p.request.id, &error.to_string())?; }
        }
        Ok(())
    }
    fn queue_receipts(&mut self,id:&str) {
        self.receipt_queue.retain(|(old,_)|old!=id);
        if let Some(chat)=self.chats.get(id) {
            let ids=chat.local.pending.iter().map(|p|p.request.id.clone()).collect::<Vec<_>>();
            for batch in ids.chunks(4).rev() {self.receipt_queue.push_front((id.into(),batch.to_vec()));}
        }
    }
    fn reconcile_receipts(&mut self)->Result<()> {
        if self.epoch.is_none() {return Ok(());}
        if self.receipt_inflight.as_ref().is_some_and(|(_,_,_,at)|at.elapsed()>std::time::Duration::from_secs(120)) {
            let (_,session,ids,_)=self.receipt_inflight.take().unwrap();self.receipt_queue.push_back((session,ids));
        }
        if self.receipt_inflight.is_some() {return Ok(());}
        if let Some((session,ids))=self.receipt_queue.front().cloned() {
            let request=self.request(ClientCommand::GetReceipts {session_id:session.clone(),requests:ids.clone()})?;
            self.receipt_queue.pop_front();self.receipt_inflight=Some((request,session,ids,std::time::Instant::now()));
        }
        Ok(())
    }
    pub fn open(&mut self, id: &str) -> Result<()> {
        self.ensure_chat(id)?;
        if self.account.missing_chats.contains(id) {return Ok(());}
        if self.is_creating(id) { return self.retry_create(); }
        if self.chats[id].feed.opening {
            return Ok(());
        }
        self.request(ClientCommand::GetSession { session_id:id.into() })?;
        self.queue_receipts(id);
        self.chats.get_mut(id).unwrap().feed.opening = true;
        Ok(())
    }
    pub fn history(&mut self) -> Result<()> {
        let Some(id) = self.account.selected.clone() else {
            return Ok(());
        };
        let feed = &self.chats[&id].feed;
        if feed.loading || !feed.synchronized {
            return Ok(());
        }
        if let Some(before) = self.remote.history_cursor(&id)? {
            self.network.as_ref().ok_or_else(||anyhow::anyhow!("Not connected"))?.send(transport::Command::Blocks(crate::blocks::Command::History { scope:id.clone(),before }))?;
            self.chats.get_mut(&id).unwrap().feed.loading = true;
        }
        Ok(())
    }
    pub fn download_key(session: &str, entry: &str) -> String {
        format!("{}:{}", session.len(), session) + entry
    }
    pub fn saved_download(&mut self, session: &str, entry: &str) -> Option<SavedDownload> {
        let key=Self::download_key(session,entry);
        if !self.saved_downloads.contains_key(&key) {
            let lineage=self.account.source_lineage.as_deref().unwrap_or_default();
            match self.store.saved_download(&self.identity,lineage,session,entry) {
                Ok(saved) => {self.saved_downloads.insert(key.clone(),saved);}
                Err(error) => {self.notice=Some(error.to_string());return None;}
            }
        }
        self.saved_downloads.get(&key).cloned().flatten()
    }
    pub fn record_download(&mut self, identity:&str, lineage:&str, session:&str, entry:&str, saved:SavedDownload) -> Result<()> {
        self.store.record_download(identity,lineage,session,entry,&saved)?;
        if identity==self.identity && self.account.source_lineage.as_deref().unwrap_or_default()==lineage {
            self.saved_downloads.insert(Self::download_key(session,entry),Some(saved));
        }
        Ok(())
    }
    pub fn forget_download(&mut self, session:&str, entry:&str) -> Result<()> {
        self.forget_download_for(&self.identity.clone(),&self.account.source_lineage.clone().unwrap_or_default(),session,entry)
    }
    pub fn forget_download_for(&mut self, identity:&str, lineage:&str, session:&str, entry:&str) -> Result<()> {
        self.store.forget_download(identity,lineage,session,entry)?;
        if identity==self.identity && self.account.source_lineage.as_deref().unwrap_or_default()==lineage {
            self.saved_downloads.insert(Self::download_key(session,entry),None);
        }
        Ok(())
    }
    /// The data endpoint is authorized independently of control readiness.
    pub fn content_authorized(&self) -> bool { self.remote.authorized() }
    pub fn attachment_path(&self, session:&str, entry:&str) -> PathBuf {
        let source=self.remote.lineage().unwrap_or_default();
        self.store.attachment_path(&format!("{}:{source}",self.identity),session,entry)
    }
    pub fn download(&mut self, session: &str, entry: &str, limit: u64) -> Result<PathBuf> {
        let path = self.attachment_path(session,entry);
        if self.remote.file_ready(session,&format!("file:{entry}"),&path,limit)? {
            let key=Self::download_key(session,entry);
            if let Some(download)=self.downloads.get_mut(&key)
                && download.path==path && (!download.status.done || download.status.failure.is_some()) {
                let size=path.metadata()?.len();
                download.update(tau_transfer::TransferStatus {transferred:size,total:size,
                    network_bytes:download.status.network_bytes,done:true,failure:None},path.clone());
            }
            return Ok(path);
        }
        ensure!(self.network.is_some() && (self.remote.authorized() || self.remote.has_file(session,&format!("file:{entry}"))), "Content connection is not authorized yet");
        let key = Self::download_key(session, entry);
        if self.downloads.get(&key).is_some_and(|d| !d.status.done) {
            return Ok(path);
        }
        self.network.as_ref().unwrap().send(Command::Download {
            key: key.clone(),
            session: session.into(),
            entry: entry.into(),
            target: path.clone(),
            limit,
        })?;
        self.downloads.insert(
            key,
            Download::new(tau_transfer::TransferStatus {
                transferred: 0,
                total: 0,
                network_bytes: 0,
                done: false,
                failure: None,
            }, path.clone()),
        );
        Ok(path)
    }
    pub fn cancel_download(&self, key: &str) -> Result<()> {
        self.network
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Not connected"))?
            .send(Command::CancelDownload(key.into()))
    }
    fn not_sent(&mut self, id: &str, detail: &str) -> Result<()> {
        self.project_deletions.remove(id);
        if self.receipt_inflight.as_ref().is_some_and(|(request,_,_,_)|request==id) {let (_,session,ids,_)=self.receipt_inflight.take().unwrap();self.receipt_queue.push_back((session,ids));}
        for (session, chat) in &mut self.chats {
            if chat
                .model_request
                .as_ref()
                .is_some_and(|(request, _)| request == id)
            {
                chat.model_request = None;
            }
            if let Some(p) = chat.local.pending.iter_mut().find(|p| p.request.id == id) {
                p.status = Delivery::Rejected;
                p.detail = Some(detail.into());
                self.store.save_chat(&self.identity, session, &chat.local)?;
            }
        }
        if let Some(command) = self.requests.remove(id) {
            if let ClientCommand::GetSession {session_id}=&command && let Some(chat)=self.chats.get_mut(session_id) {chat.feed.opening=false;}
            if matches!(command, ClientCommand::CreateProject { .. } | ClientCommand::UpdateProject { .. } | ClientCommand::DeleteProject { .. }) {
                self.project_result = Some((id.into(), false));
            }
        }
        self.notice = Some(detail.into());
        Ok(())
    }
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        for _ in 0..256 {
            let Some(event) = self.network.as_mut().and_then(|n| n.events.try_recv().ok()) else {
                break;
            };
            changed = true;
            if let Err(error) = self.network_event(event) {
                self.notice = Some(error.to_string());
            }
        }
        let mut scopes = std::collections::HashSet::new();
        for _ in 0..256 {
            let Some(notice) = self.network.as_mut().and_then(|n|n.blocks.try_recv().ok()) else { break; };
            changed = true;
            if let Some((key,path,status))=notice.transfer {
                if let Err(error)=self.network_event(transport::Event::Download {key,path,status}) {self.notice=Some(error.to_string());}
            } else if let Some(error) = notice.error { if let Some(chat)=self.chats.get_mut(&notice.scope) {chat.feed.loading=false;} self.notice = Some(error); }
            else { scopes.insert(notice.scope); }
        }
        for scope in scopes {
            if self.chats.contains_key(&scope) && let Err(error) = self.refresh_blocks(&scope) { self.notice = Some(error.to_string()); }
        }
        if let Err(error) = self.watch_blocks() { self.notice = Some(error.to_string()); }
        if let Err(error) = self.reconcile_controls() {self.notice=Some(error.to_string());}
        if let Err(error) = self.reconcile_receipts() {self.notice=Some(error.to_string());}
        let mut waiting=self.chats.keys().cloned().collect::<Vec<_>>();waiting.sort_by_key(|id|self.account.selected.as_ref()!=Some(id));
        for id in waiting {if let Err(e)=self.send_waiting(&id,false) {self.notice=Some(e.to_string());}}
        changed
    }
    fn refresh_blocks(&mut self, scope: &str) -> Result<()> {
        self.plan_dirty.set(true);
        let full=self.chats.get(scope).is_none_or(|chat|chat.feed.generation.is_empty());
        if let Some(view) = self.remote.changes(scope,self.viewport.as_ref().filter(|(id,_)|id==scope).map(|(_,ids)|ids),full)? {
            let chat = self.chats.get_mut(scope).context("Unknown cached chat")?;
            let delivered=chat.feed.native_view(view)?;
            chat.feed.synchronized = chat.feed.queue.available;
            chat.feed.opening = false;
            let before = chat.local.pending.len();
            chat.local.reconcile_complete(&chat.feed.queue, &delivered,&chat.feed.incomplete);
            if chat.local.pending.len() != before { self.store.save_chat(&self.identity,scope,&chat.local)?; }
        }
        Ok(())
    }
    pub fn cancel_copy(&mut self) {self.copy=None;self.copied=None;self.plan_dirty.set(true);}
    pub fn copy_details(&mut self, scope:&str, ids:Vec<String>) -> Result<()> {
        self.cancel_copy();
        // Local demo/renderer fixtures have complete events without a remote
        // cache. Native views fetch missing bytes as an explicit copy interest.
        if !self.remote.has_snapshot(scope)? {
            let chat=self.chats.get(scope).context("Unknown chat")?;
            let tools=crate::details::Tools::new(chat.feed.events.values());
            self.copied=Some(tools.copy(&ids.iter().filter_map(|id|chat.feed.event(id)).collect::<Vec<_>>()));
        } else {
            self.copy=Some((scope.into(),ids));self.notice=Some("Fetching details to copy…".into());
            self.watch_blocks()?;
        }
        Ok(())
    }
    pub fn viewport(&mut self,scope:&str,ids:std::collections::BTreeSet<String>) {
        if self.viewport.as_ref().is_some_and(|(id,old)|id==scope && old==&ids) {return;}
        let previous=self.viewport.as_ref().filter(|(id,_)|id==scope).map(|(_,ids)|ids.clone()).unwrap_or_default();
        self.remote.viewport_changed(scope,ids.iter().cloned().chain(previous));
        self.viewport=Some((scope.into(),ids));self.plan_dirty.set(true);
        if let Err(error)=self.refresh_blocks(scope) {self.notice=Some(error.to_string());}
    }
    fn watch_blocks(&mut self) -> Result<()> {
        if !self.plan_dirty.replace(false) {return Ok(());}
        if self.copy.as_ref().is_some_and(|(scope,_)|self.account.selected.as_ref()!=Some(scope)) {self.cancel_copy();}
        if let Some((scope,ids))=&self.copy {
            match self.remote.copy_ready(scope,ids) {
                Ok(Some(text))=>{self.copied=Some(text);self.copy=None;self.notice=Some("Details copied".into());}
                Ok(None)=>{},
                Err(error)=>{self.copy=None;return Err(error);}
            }
        }
        let next = self.account.selected.as_ref().filter(|id|!self.is_creating(id)).and_then(|id|self.chats.get(id).map(|chat|(id,chat)))
            .map(|(id,chat)|self.remote.plan_visible(id,&chat.local,self.copy.as_ref().map_or(&[],|(_,ids)|ids.as_slice()),self.viewport.as_ref().filter(|(scope,_)|scope==id).map(|(_,ids)|ids))).transpose()?;
        if next != self.block_plan && let Some(network) = &self.network {
            network.send(transport::Command::Blocks(crate::blocks::Command::Plan(next.clone())))?;
            self.block_plan = next;
        }
        Ok(())
    }
    fn network_event(&mut self, event: transport::Event) -> Result<()> {
        let fatal = matches!(&event, transport::Event::Fatal(_));
        match event {
            transport::Event::Source(epoch,lineage)=>{
                self.source_guard=Some((epoch,false));
                if self.store.bind_source(&self.identity,&lineage)? {
                    self.saved_downloads.clear();
                    self.account=self.store.get(&self.identity,"account")?;
                    for (id,chat) in &mut self.chats {chat.local=self.store.load_chat(&self.identity,id)?;chat.feed=Feed::default();}
                    self.notice=Some("Source lineage changed. Saved work was preserved, but old intents will not be automatically executed. Review any effects after the restored snapshot before retrying.".into());
                } else {self.account.source_lineage=Some(lineage);}
                self.source_guard=Some((epoch,true));
            }
            transport::Event::Ready(epoch) => {
                ensure!(self.source_guard==Some((epoch,true)),"Source identity was not durably recorded; automatic submission is disabled. Repair local storage and reconnect.");
                self.state_versions.clear();self.catalog=None;self.receipt_queue.clear();self.receipt_inflight=None;
                self.epoch = Some(epoch);
                self.create_failed_epoch = None;
                self.control_check=None;
                for saved in self.account.pending_controls.values_mut() {saved.blocked=false;}
                self.connection = "Connected".into();
                self.health.connected();
                self.request(ClientCommand::ListSessions)?;
                for id in self.chats.keys().cloned().collect::<Vec<_>>() {
                    if !self.is_creating(&id) {self.queue_receipts(&id);}
                }
                if let Some(id) = self.account.selected.clone().filter(|id| !self.is_creating(id) && !self.account.missing_chats.contains(id)) {
                    self.open(&id)?;self.send_waiting(&id,false)?;
                    self.request(ClientCommand::GetCommands { session_id: id })?;
                }
            }
            transport::Event::Disconnected(detail) | transport::Event::Fatal(detail) => {
                self.epoch = None;
                self.connection = detail;
                self.health.disconnected(fatal);
                self.requests.clear();
                self.project_deletions.clear();
                for (session, chat) in &mut self.chats {
                    if chat.model_request.take().is_some() {
                        self.notice = Some("Model change unconfirmed; check the current model after reconnecting. It was not resent.".into());
                    }
                    chat.commands_loaded = false;
                    chat.feed.synchronized = false;
                    chat.feed.loading = false;
                    chat.feed.opening = false;
                    for p in &mut chat.local.pending {
                        if matches!(p.status, Delivery::Sending | Delivery::Preparing) {
                            p.status = Delivery::Unconfirmed;
                        } else if p.status == Delivery::WaitingForModel {
                            p.status = Delivery::Rejected;
                            p.detail = Some("Model selection unconfirmed; check the current model, then restore this draft".into());
                        }
                    }
                    self.store.save_chat(&self.identity, session, &chat.local)?;
                }
            }
            transport::Event::HeartbeatSent { epoch, at } if self.epoch == Some(epoch) => {
                self.health.sent(at)
            }
            transport::Event::HeartbeatReply { epoch, at, rtt } if self.epoch == Some(epoch) => {
                self.health.reply(rtt, at)
            }
            transport::Event::NotSent(id, detail) => {
                if self.is_creating(&id) {
                    self.requests.remove(&id);
                    self.notice = Some(format!("New chat saved locally, not yet confirmed: {detail}"));
                } else { self.not_sent(&id, &detail)?; }
            },
            transport::Event::Prepared { epoch, id, result } => {
                let session = self
                    .chats
                    .iter()
                    .find(|(_, c)| c.local.pending.iter().any(|p| p.request.id == id))
                    .map(|(s, _)| s.clone());
                if let Some(session) = session {
                    if self.epoch != Some(epoch) {
                        self.not_sent(
                            &id,
                            "Connection changed during upload; prompt was not sent",
                        )?;
                    } else {
                        match result {
                            Err(detail) => self.not_sent(&id, &detail)?,
                            Ok(text) => {
                                let p = self
                                    .chats
                                    .get_mut(&session)
                                    .unwrap()
                                    .local
                                    .pending
                                    .iter_mut()
                                    .find(|p| p.request.id == id)
                                    .unwrap();
                                p.request.command = ClientCommand::Prompt {
                                    session_id: session.clone(),
                                    text,
                                };
                                p.status = Delivery::Sending;
                                let request = p.request.clone();
                                self.save_chat(&session)?;
                                if let Err(error) = self
                                    .network
                                    .as_ref()
                                    .unwrap()
                                    .send(Command::Request { epoch, request })
                                {
                                    self.not_sent(&id, &error.to_string())?;
                                }
                            }
                        }
                    }
                }
            }
            transport::Event::Download { key, status, path } => {
                if let Some(download) = self.downloads.get_mut(&key) {
                    download.update(status, path);
                } else {
                    self.downloads.insert(key, Download::new(status, path));
                }
            }
            transport::Event::Metrics(stats)=>self.native_metrics=stats,
            transport::Event::Message(epoch, message)|transport::Event::SizedMessage(epoch,message,_) if self.epoch == Some(epoch) => {
                self.message(*message)?
            }
            _ => {}
        }
        Ok(())
    }
    pub fn message(&mut self, message: ServerMessage) -> Result<()> {
        match message {
            ServerMessage::BlockConnection { .. } | ServerMessage::Data {..} => {},
            ServerMessage::Operation {operation_id,registered,response} => {
                if let Some(saved)=self.account.pending_controls.get(&operation_id).cloned() {
                    if registered && !saved.accepted {
                        self.account.pending_controls.get_mut(&operation_id).unwrap().accepted=true;
                        self.store.put(&self.identity,"account",&self.account)?;
                    }
                    if let Some(response)=response {
                        ensure!(matches!(response.as_ref(),ServerMessage::Response {request_id,..} if request_id==&operation_id),"Operation receipt has a different identity");
                        self.requests.insert(operation_id.clone(),saved.request.command);
                        if !saved.deleted_chats.is_empty() {self.project_deletions.insert(operation_id.clone(),saved.deleted_chats);}
                        self.message(*response)?;
                    } else if !registered {
                        self.account.pending_controls.get_mut(&operation_id).unwrap().blocked=true;
                        self.store.put(&self.identity,"account",&self.account)?;
                        self.notice=Some("An action has no daemon receipt. Its intent is saved locally; repeat the same action explicitly to retry its original ID.".into());
                    }
                }
            }
            ServerMessage::Accepted {request_id} => {
                if let Some(saved)=self.account.pending_controls.get_mut(&request_id) {saved.accepted=true;self.store.put(&self.identity,"account",&self.account)?;}
                for (session,chat) in &mut self.chats {
                    if let Some(p)=chat.local.pending.iter_mut().find(|p|p.request.id==request_id) {
                        p.status=Delivery::Accepted;p.detail=Some("Accepted; awaiting outcome".into());
                        self.store.save_chat(&self.identity,session,&chat.local)?;
                    }
                }
            }, // Owned by the network block service.
            ServerMessage::Receipts {session_id,reports} => {
                if self.receipt_inflight.as_ref().is_some_and(|(_,session,ids,_)|session==&session_id && ids.iter().all(|id|reports.iter().any(|r|&r.id==id))) {self.receipt_inflight=None;}
                self.ensure_chat(&session_id)?;
                let chat = self.chats.get_mut(&session_id).unwrap();
                for report in reports {
                    if let Some(notice)=&report.notice {self.notice=Some(notice.clone());}
                    if let Some(pending) = chat.local.pending.iter_mut().find(|p|p.request.id == report.id) {
                        if let Some(error) = report.error { pending.status = Delivery::Rejected; pending.detail = Some(error); }
                        else if report.accepted && !report.complete { pending.status = Delivery::Accepted; pending.detail = Some("Accepted; awaiting outcome".into()); }
                        else if report.accepted {
                            if matches!(pending.request.command,ClientCommand::QueueControl {operation:QueueOperation::Edit {..}|QueueOperation::Delete {..},..}) {
                                pending.status=Delivery::Accepted;pending.detail=Some("Accepted; synchronizing queue".into());
                            } else {chat.local.pending.retain(|p|p.request.id!=report.id);}
                        }
                    }
                }
                chat.local.reconcile_complete(&chat.feed.queue,&[],&chat.feed.incomplete);
                self.store.save_chat(&self.identity,&session_id,&chat.local)?;
            }
            ServerMessage::SessionPage {catalog_id,revision,after,next,mut sessions,states}=>{
                let Some(catalog)=&mut self.catalog else {return Ok(());};
                if catalog.id!=catalog_id {return Ok(());}
                if catalog.revision.is_some_and(|r|r!=revision) {self.catalog=None;self.request(ClientCommand::ListSessions)?;return Ok(());}
                if catalog.session_after!=after {return Ok(());}
                catalog.revision=Some(revision);catalog.session_after=next.clone();
                for session in &mut sessions {
                    let revision=states.get(&session.id).copied().unwrap_or(0);
                    if let Some((old,status,detail,usage))=self.state_versions.get(&session.id) && *old>revision {
                        session.status=*status;session.detail=detail.clone();session.context_usage=*usage;
                    } else {self.state_versions.insert(session.id.clone(),(revision,session.status,session.detail.clone(),session.context_usage));}
                }
                ensure!(catalog.sessions.len()+sessions.len()<=20000,"Session catalogue exceeds its bounded working set");
                catalog.sessions.extend(sessions);
                if let Some(after)=next {self.request(ClientCommand::ListPage {catalog_id,projects:false,after:Some(after),revision})?;}
                else {catalog.sessions_done=true;let sessions=std::mem::take(&mut catalog.sessions);self.message(ServerMessage::Sessions {sessions})?;}
            }
            ServerMessage::ProjectPage {catalog_id,revision,after,next,projects}=>{
                let Some(catalog)=&mut self.catalog else {return Ok(());};
                if catalog.id!=catalog_id {return Ok(());}
                if catalog.revision.is_some_and(|r|r!=revision) {self.catalog=None;self.request(ClientCommand::ListSessions)?;return Ok(());}
                if catalog.project_after!=after {return Ok(());}
                catalog.revision=Some(revision);catalog.project_after=next.clone();
                ensure!(catalog.projects.len()+projects.len()<=128,"Topic catalogue exceeds its limit");
                catalog.projects.extend(projects);
                if let Some(after)=next {self.request(ClientCommand::ListPage {catalog_id,projects:true,after:Some(after),revision})?;}
                else {catalog.projects_done=true;let mut projects=std::mem::take(&mut catalog.projects);projects.sort_by_key(|p|p.id!=GENERAL_PROJECT_ID);self.message(ServerMessage::Projects {projects})?;}
            }
            ServerMessage::Projects { projects } => {
                self.account.projects = projects;
                if !self.account.projects.iter().any(|p| p.id == self.account.selected_project) {
                    self.account.selected_project = general_project_id();
                }
                self.store.put(&self.identity, "account", &self.account)?;
            }
            ServerMessage::Sessions { mut sessions } => {
                self.plan_dirty.set(true);
                // Preserve a provisional local chat across server list refreshes.
                // Remote deletion still clears a genuinely missing selection.
                let pending = self.account.pending_create.clone();
                let confirmed = pending.as_ref().and_then(|request| {
                    sessions.iter().any(|s| s.id == request.id).then(|| request.id.clone())
                });
                let present=sessions.iter().map(|s|s.id.clone()).collect::<std::collections::HashSet<_>>();
                self.account.missing_chats.clear();
                for id in self.store.work_chats(&self.identity)? {
                    if present.contains(&id) || pending.as_ref().is_some_and(|p|p.id==id) {continue;}
                    let mut summary=self.account.sessions.iter().find(|s|s.id==id).cloned().unwrap_or_else(||Self::creating_summary(&id,GENERAL_PROJECT_ID,0));
                    summary.title=format!("Local recovery: {}",summary.title.trim_start_matches("Local recovery: "));
                    summary.project_id=GENERAL_PROJECT_ID.into();summary.starter=false;summary.status=SessionStatus::Sleeping;
                    summary.detail=Some("Source chat missing. Drafts, files and original intents are retained locally; nothing is resent.".into());
                    self.account.missing_chats.insert(id);sessions.push(summary);
                }
                if self.account.selected.as_ref().is_some_and(|id| !sessions.iter().any(|s| &s.id == id)
                    && !pending.as_ref().is_some_and(|request| &request.id == id)) {
                    self.account.selected = None;
                }
                for s in &sessions {
                    if s.starter { self.account.read_at.insert(s.id.clone(), s.updated_at_ms); }
                }
                for session in &mut sessions {if !self.account.missing_chats.contains(&session.id) && let Some((_,status,detail,usage))=self.state_versions.get(&session.id) {session.status=*status;session.detail=detail.clone();session.context_usage=*usage;}}
                self.account.sessions = sessions;
                if let Some(request) = pending {
                    if let Some(confirmed) = confirmed { self.finish_create(&request.id, &confirmed)?; }
                    else {
                        let project = match &request.command {
                            ClientCommand::CreateSession { project_id, .. } => project_id.as_str(),
                            _ => unreachable!("pending create must be a create request"),
                        };
                        self.account.sessions.insert(0, Self::creating_summary(&request.id, project, crate::clock::now_ms().unwrap_or(0)));
                        self.retry_create()?;
                    }
                }
                self.account.last_chat_by_project.retain(|project, chat| self.account.sessions.iter()
                    .any(|s| s.project_id == *project && s.id == *chat));
                if let Some(id) = &self.account.selected
                    && let Some(s) = self.account.sessions.iter().find(|s| &s.id == id)
                {
                    self.account.selected_project = s.project_id.clone();
                    self.account.last_chat_by_project.insert(s.project_id.clone(), id.clone());
                    if self.viewing_chat { self.account.read_at.insert(id.clone(), s.updated_at_ms); }
                }
                self.store.put(&self.identity, "account", &self.account)?;
            }
            ServerMessage::TranscriptSnapshot {
                session_id,
                snapshot,
            } => {
                self.ensure_chat(&session_id)?;
                let mut delivered = snapshot.delivered.clone();
                delivered.extend(
                    snapshot
                        .events
                        .iter()
                        .filter(|e| e.phase == EventPhase::Saved)
                        .filter_map(|e| e.origin.request_id.clone()),
                );
                let chat = self.chats.get_mut(&session_id).unwrap();
                chat.feed.opening = false;
                if chat.feed.snapshot(snapshot)? {
                    chat.local.reconcile_complete(&chat.feed.queue, &delivered,&chat.feed.incomplete);
                    self.save_chat(&session_id)?;
                }
            }
            ServerMessage::TranscriptUpdate {
                session_id,
                generation,
                sequence,
                change,
            } => {
                self.ensure_chat(&session_id)?;
                let mut delivered = change.delivered.clone();
                delivered.extend(
                    change
                        .events
                        .iter()
                        .filter(|e| e.phase == EventPhase::Saved)
                        .filter_map(|e| e.origin.request_id.clone()),
                );
                let chat = self.chats.get_mut(&session_id).unwrap();
                match chat.feed.update(&generation, sequence, change) {
                    Ok(true) => {
                        chat.local.reconcile_complete(&chat.feed.queue, &delivered,&chat.feed.incomplete);
                        self.save_chat(&session_id)?;
                    }
                    Ok(false) => {}
                    Err(_) => {
                        if self.epoch.is_some() {
                            self.open(&session_id)?;
                        }
                    }
                }
            }
            ServerMessage::TranscriptPage {
                session_id,
                generation,
                cursor,
                page,
                ..
            } => {
                if let Some(chat) = self.chats.get_mut(&session_id) {
                    chat.feed.loading = false;
                    chat.feed.page(&generation, cursor, page)?;
                }
            }
            ServerMessage::SessionState {
                session_id,revision,restore_review,
                status,
                context_usage,
                detail,
            } => {
                if self.state_versions.get(&session_id).is_some_and(|old|old.0>revision) {return Ok(());}
                self.state_versions.insert(session_id.clone(),(revision,status,detail.clone(),context_usage));
                if let Some(review)=restore_review {if review {self.restore_reviews.insert(session_id.clone());} else {self.restore_reviews.remove(&session_id);}}
                if let Some(s) = self
                    .account
                    .sessions
                    .iter_mut()
                    .find(|s| s.id == session_id)
                {
                    s.status = status;
                    s.context_usage = context_usage;
                    s.detail = detail;
                }
            }
            ServerMessage::Commands {
                session_id,
                commands,
            } => {
                self.ensure_chat(&session_id)?;
                let chat = self.chats.get_mut(&session_id).unwrap();
                chat.commands = commands;
                chat.commands_loaded = true;
            }
            ServerMessage::Settings {
                settings,
                ..
            } => {
                if self.daemon_settings.as_ref().is_none_or(|old|old.revision<=settings.revision) {self.daemon_settings = Some(*settings);}
            }
            ServerMessage::Notice { message, .. } => self.notice = Some(message),
            ServerMessage::ResyncRequired { session_id } => {
                if self.epoch.is_some() {
                    if let Some(id) = session_id {
                        self.open(&id)?;
                    } else {
                        self.request(ClientCommand::ListSessions)?;
                        for id in self.chats.keys().cloned().collect::<Vec<_>>() {
                            self.open(&id)?;
                        }
                    }
                }
            }
            ServerMessage::Response {
                request_id,
                ok,
                session_id,
                draft,
                disposition,
                uncertain,
                notice,
                error,
                ..
            } => {
                if let Some(notice) = notice {
                    self.notice = Some(notice);
                }
                let mut matched = false;
                let mut model_changed = false;
                for (id, chat) in &mut self.chats {
                    if chat
                        .model_request
                        .as_ref()
                        .is_some_and(|(id, _)| id == &request_id)
                    {
                        chat.model_request = None;
                        model_changed = ok && !uncertain;
                        if uncertain {
                            self.notice = Some(
                                "Model change unconfirmed; check the current model before sending."
                                    .into(),
                            );
                        }
                    }
                    if let Some(p) = chat
                        .local
                        .pending
                        .iter_mut()
                        .find(|p| p.request.id == request_id)
                    {
                        matched = true;
                        p.status = if uncertain {
                            Delivery::Unconfirmed
                        } else if ok {
                            Delivery::Accepted
                        } else {
                            Delivery::Rejected
                        };
                        p.detail = error.clone();
                        if ok
                            && !uncertain
                            && (matches!(disposition, Some(PromptDisposition::Handled))
                                || !matches!(p.request.command, ClientCommand::Prompt { .. } | ClientCommand::QueueControl {operation:QueueOperation::Edit {..}|QueueOperation::Delete {..},..}))
                        {
                            chat.local.pending.retain(|p| p.request.id != request_id);
                        }
                        chat.local.reconcile_complete(&chat.feed.queue,&[],&chat.feed.incomplete);
                        self.store.save_chat(&self.identity, id, &chat.local)?;
                    }
                }
                let create_reply = self.is_creating(&request_id);
                let command = self.requests.remove(&request_id).or_else(||self.account.pending_controls.get(&request_id).map(|saved|saved.request.command.clone()));
                if let Some(ClientCommand::GetSession {session_id}) = &command && let Some(chat) = self.chats.get_mut(session_id) { chat.feed.opening = false; }
                if let Some(deleted) = self.project_deletions.remove(&request_id) && ok && !uncertain {
                    for id in deleted {
                        self.store.delete_chat(&self.identity, &id)?;
                        self.chats.remove(&id);
                        self.account.read_at.remove(&id);
                        self.account.sessions.retain(|s| s.id != id);
                        if self.account.selected.as_ref() == Some(&id) { self.account.selected = None; }
                    }
                    self.store.put(&self.identity, "account", &self.account)?;
                }
                if matches!(command, Some(ClientCommand::CreateProject { .. } | ClientCommand::UpdateProject { .. } | ClientCommand::DeleteProject { .. })) {
                    self.project_result = Some((request_id.clone(), ok && !uncertain));
                }
                if ok && !uncertain && create_reply && let Some(id) = &session_id { self.finish_create(&request_id, id)?; }
                if matches!(
                    command,
                    Some(ClientCommand::SetSettings { .. } | ClientCommand::GetSettings)
                ) {
                    self.settings_result = Some((request_id.clone(), ok && !uncertain));
                }
                if model_changed {
                    if let Some(id) = session_id.as_deref() { self.send_waiting(id, true)?; }
                    self.request(ClientCommand::ListSessions)?;
                }
                if !ok {
                    if create_reply || matches!(command, Some(ClientCommand::CreateSession { .. })) {
                        self.create_failed_epoch = self.epoch;
                        self.notice = Some(error.clone().unwrap_or_else(|| "New chat is saved locally but was not confirmed; retry when connected".into()));
                    }
                    if matches!(&command, Some(ClientCommand::Prompt {text,..}) if text.starts_with("/model "))
                        && let Some(id) = session_id.as_deref() && let Some(chat) = self.chats.get_mut(id)
                        && chat.local.pending.iter().any(|p| p.status == Delivery::WaitingForModel) {
                        for p in &mut chat.local.pending {
                            if p.status == Delivery::WaitingForModel {
                                p.status = Delivery::Rejected;
                                p.detail = Some("Model selection failed; restore the draft and choose a model".into());
                            }
                        }
                        self.store.save_chat(&self.identity, id, &chat.local)?;
                    }
                    if let Some(ClientCommand::GetHistory { session_id, .. }) = &command
                        && let Some(chat) = self.chats.get_mut(session_id)
                    {
                        chat.feed.loading = false;
                    }
                    if !matched {
                        self.notice = Some(error.unwrap_or_else(|| "Request failed".into()));
                    }
                } else {
                    match command {
                        Some(ClientCommand::CreateProject { project_id, .. }) => {
                            self.select_project(&project_id)?;
                        }
                        Some(ClientCommand::MoveSession { session_id, project_id }) => {
                            // The clicked chat, not the selected chat, owns this action.
                            if let Some(session) = self.account.sessions.iter_mut().find(|s| s.id == session_id) {
                                session.project_id = project_id;
                            }
                            self.store.put(&self.identity, "account", &self.account)?;
                        }
                        Some(ClientCommand::DeleteProject { project_id, mode, .. }) => {
                            if self.account.selected_project == project_id {
                                self.account.selected_project = general_project_id();
                            }
                            if mode == DeleteProjectMode::MoveToGeneral {
                                for s in &mut self.account.sessions {
                                    if s.project_id == project_id { s.project_id = general_project_id(); }
                                }
                            }
                            self.store.put(&self.identity, "account", &self.account)?;
                        }
                        Some(ClientCommand::CreateSession { .. }) => {
                            if let Some(id) = session_id { self.finish_create(&request_id, &id)?; }
                        }
                        Some(ClientCommand::ForkSession { .. } | ClientCommand::CloneSession { .. }) => {
                            if let Some(id) = session_id {
                                self.select(&id)?;
                                if let Some(draft) = draft {
                                    if self.chats[&id].local.draft.is_empty() || self.chats[&id].local.draft==draft {self.draft(draft)?;}
                                    else {
                                        let chat=self.chats.get_mut(&id).unwrap();
                                        if !chat.local.pending.iter().any(|p|p.text==draft) {
                                            chat.local.pending.push(Pending {request:ClientRequest {id:uuid::Uuid::new_v4().to_string(),command:ClientCommand::Prompt {session_id:id.clone(),text:draft.clone()}},started_at_ms:None,text:draft,files:vec![],status:Delivery::Rejected,detail:Some("Recovered fork draft; existing draft was preserved".into())});
                                            self.save_chat(&id)?;
                                        }
                                    }
                                }
                            }
                        }
                        Some(ClientCommand::RefreshModelCatalog { .. }) => {
                            if let Some(id) = self.account.selected.clone() {
                                self.request(ClientCommand::GetCommands { session_id:id })?;
                            }
                        }
                        Some(ClientCommand::DeleteSession { session_id }) => {
                            self.store.delete_chat(&self.identity, &session_id)?;
                            self.chats.remove(&session_id);
                            if self.account.selected.as_ref() == Some(&session_id) {
                                self.account.selected = None;
                            }
                            self.account.last_chat_by_project.retain(|_, chat| chat != &session_id);
                            self.store.put(&self.identity, "account", &self.account)?;
                        }
                        _ => {}
                    }
                }
                if uncertain {
                    if let Some(saved)=self.account.pending_controls.get_mut(&request_id) {saved.blocked=true;self.store.put(&self.identity,"account",&self.account)?;}
                } else if self.account.pending_controls.remove(&request_id).is_some() {
                    self.store.put(&self.identity,"account",&self.account)?;
                }
            }
            ServerMessage::Hello { .. } => {}
        }
        if self.catalog.as_ref().is_some_and(|c|c.sessions_done && c.projects_done && c.refresh) {self.catalog=None;self.request(ClientCommand::ListSessions)?;}
        Ok(())
    }
}

#[cfg(test)]
mod safety_tests {
use super::*;
use std::sync::Arc;
use crate::transport::Event as NetworkEvent;
#[test]
fn lineage_fence_rolls_back_atomically_and_missing_source_work_stays_reachable() {
    use crate::store::{LocalChat,Pending};
    let root=tempfile::tempdir().unwrap();let mut c=Controller::new(Store::open(root.path().into()).unwrap(),Arc::new(||{})).unwrap();c.select("missing").unwrap();
    c.store.bind_source(&c.identity,"before").unwrap();c.account=c.store.get(&c.identity,"account").unwrap();
    let local=LocalChat {draft:"keep me".into(),pending:vec![Pending {request:ClientRequest {id:"original".into(),command:ClientCommand::Prompt {session_id:"missing".into(),text:"possibly paid".into()}},text:"possibly paid".into(),files:vec![],status:Delivery::WaitingForConnection,started_at_ms:None,detail:None}],..Default::default()};
    c.store.save_chat(&c.identity,"missing",&local).unwrap();
    let db=rusqlite::Connection::open(root.path().join("client.sqlite3")).unwrap();db.execute_batch("CREATE TRIGGER fail_fence BEFORE UPDATE ON local WHEN NEW.key='account' BEGIN SELECT RAISE(ABORT,'fence full');END").unwrap();
    assert!(c.network_event(NetworkEvent::Source(1,"after".into())).is_err());assert!(c.network_event(NetworkEvent::Ready(1)).is_err());assert!(c.epoch.is_none());
    assert_eq!(c.store.load_chat(&c.identity,"missing").unwrap().pending[0].status,Delivery::WaitingForConnection);
    assert_eq!(c.store.get::<crate::store::Account>(&c.identity,"account").unwrap().source_lineage.as_deref(),Some("before"));
    db.execute_batch("DROP TRIGGER fail_fence").unwrap();c.network_event(NetworkEvent::Source(1,"after".into())).unwrap();
    assert_eq!(c.selected().unwrap().local.pending[0].status,Delivery::Unconfirmed);
    c.message(ServerMessage::Sessions {sessions:vec![]}).unwrap();assert!(c.account.missing_chats.contains("missing"));assert_eq!(c.account.selected.as_deref(),Some("missing"));assert_eq!(c.selected().unwrap().local.draft,"keep me");
    assert!(c.send_prompt().is_err());c.copy_missing_draft("missing").unwrap();assert_eq!(c.selected().unwrap().local.draft,"keep me");assert!(c.selected().unwrap().local.pending.is_empty());assert_eq!(c.chats["missing"].local.pending[0].request.id,"original");
}


#[test]
fn delayed_catalogue_pages_do_not_overwrite_newer_status_even_before_membership_arrives() {
    let root=tempfile::tempdir().unwrap();let mut c=Controller::new(Store::open(root.path().into()).unwrap(),Arc::new(||{})).unwrap();
    c.catalog=Some(Catalog {id:"walk".into(),..Default::default()});
    c.message(ServerMessage::SessionState {session_id:"a".into(),revision:10,restore_review:None,status:SessionStatus::Running,detail:Some("new".into()),context_usage:None}).unwrap();
    let a=Controller::creating_summary("a",GENERAL_PROJECT_ID,0);
    // The first page is staged before its continuation can be sent offline.
    assert!(c.message(ServerMessage::SessionPage {catalog_id:"walk".into(),revision:3,after:None,next:Some("a".into()),sessions:vec![a],states:std::collections::BTreeMap::from([("a".into(),9)])}).is_err());
    c.message(ServerMessage::SessionState {session_id:"a".into(),revision:11,restore_review:None,status:SessionStatus::Idle,detail:Some("latest".into()),context_usage:None}).unwrap();
    c.message(ServerMessage::SessionPage {catalog_id:"walk".into(),revision:3,after:Some("a".into()),next:None,sessions:vec![Controller::creating_summary("b",GENERAL_PROJECT_ID,0)],states:Default::default()}).unwrap();
    assert_eq!(c.account.sessions[0].status,SessionStatus::Idle);assert_eq!(c.account.sessions[0].detail.as_deref(),Some("latest"));
    c.message(ServerMessage::SessionState {session_id:"a".into(),revision:8,restore_review:None,status:SessionStatus::Running,detail:None,context_usage:None}).unwrap();
    c.message(ServerMessage::SessionPage {catalog_id:"old-walk".into(),revision:1,after:None,next:None,sessions:vec![],states:Default::default()}).unwrap();
    assert_eq!(c.account.sessions.len(),2);assert_eq!(c.account.sessions[0].status,SessionStatus::Idle);
}

#[test]
fn list_resyncs_coalesce_without_starving_an_in_progress_traversal() {
    let root=tempfile::tempdir().unwrap();let mut c=Controller::new(Store::open(root.path().into()).unwrap(),Arc::new(||{})).unwrap();
    c.catalog=Some(Catalog {id:"in-progress".into(),..Default::default()});c.epoch=Some(1);
    for _ in 0..100 {assert_eq!(c.request(ClientCommand::ListSessions).unwrap(),"in-progress");}
    assert!(c.catalog.as_ref().unwrap().refresh);assert!(c.requests.is_empty());
    c.epoch=None;c.message(ServerMessage::SessionPage {catalog_id:"in-progress".into(),revision:1,after:None,next:None,sessions:vec![],states:Default::default()}).unwrap();assert!(c.catalog.is_some());
    assert!(c.message(ServerMessage::ProjectPage {catalog_id:"in-progress".into(),revision:1,after:None,next:None,projects:vec![Project::general()]}).is_err());assert!(c.catalog.is_none());
}

}
