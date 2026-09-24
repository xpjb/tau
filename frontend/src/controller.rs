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
}
pub struct Chat {
    pub local: LocalChat,
    pub feed: Feed,
    pub commands: Vec<SlashCommand>,
    pub commands_loaded: bool,
    pub model_request: Option<(String, String)>,
}
pub struct Controller {
    pub store: Store,
    pub settings: Settings,
    pub identity: String,
    pub account: Account,
    pub model_preferences: crate::models::Preferences,
    pub chats: HashMap<String, Chat>,
    pub downloads: HashMap<String, Download>,
    pub viewing_chat: bool,
    pub project_result: Option<(String, bool)>,
    pub settings_result: Option<(String, bool)>,
    pub daemon_settings: Option<tau_protocol::settings::Settings>,
    pub connection: String,
    pub health: crate::connection::Health,
    pub epoch: Option<u64>,
    pub notice: Option<String>,
    remote: crate::blocks::Cache,
    block_plan: Option<crate::blocks::Plan>,
    viewport:Option<(String,std::collections::BTreeSet<String>)>,
    copy:Option<(String,Vec<String>)>,
    pub copied:Option<String>,
    network: Option<Network>,
    requests: HashMap<String, ClientCommand>,
    project_deletions: HashMap<String, Vec<String>>,
    create_failed_epoch: Option<u64>,
    control_check:Option<std::time::Instant>,
    control_cursor:usize,
    wake: Wake,
}
impl Controller {
    pub fn new(store: Store, wake: Wake) -> Result<Self> {
        let settings: Settings = store.get("", "settings")?;
        let identity = settings.identity();
        let account = store.get(&identity, "account")?;
        let model_preferences = store.get(&identity, "quick-models")?;
        let remote = store.block_cache(&identity)?;
        let mut c = Self {
            store,
            settings,
            identity,
            account,
            model_preferences,
            chats: HashMap::new(),
            downloads: HashMap::new(),
            viewing_chat: true,
            project_result: None,
            settings_result: None,
            daemon_settings: None,
            connection: "Not connected".into(),
            health: crate::connection::Health::default(),
            epoch: None,
            notice: None,
            remote,
            block_plan: None,
            viewport:None,
            copy:None, copied:None,
            network: None,
            requests: HashMap::new(),
            project_deletions: HashMap::new(),
            create_failed_epoch: None,
            control_check:None,control_cursor:0,
            wake,
        };
        if let Some(id) = c.account.selected.clone() {
            c.ensure_chat(&id)?;
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
        self.block_plan = None;
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
        self.block_plan = None;
        self.viewport=None;
        self.copy=None;self.copied=None;
        self.account = self.store.get(&self.identity, "account")?;
        self.model_preferences = self.store.get(&self.identity, "quick-models")?;
        self.chats.clear();
        self.downloads.clear();
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
            if let Some(view) = self.remote.snapshot(id)? {feed.snapshot(view.snapshot)?;feed.block_lengths=view.lengths;feed.incomplete=view.incomplete;feed.block_states=view.states;feed.synchronized=false;}
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
        if let Some(chat) = self.chats.get(id) {
            self.store.save_chat(&self.identity, id, &chat.local)?;
        }
        Ok(())
    }
    pub fn select(&mut self, id: &str) -> Result<()> {
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
        if self.epoch.is_some() && !self.is_creating(id) {
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
            if self.epoch.is_some() && !self.is_creating(&chat) {
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
        self.ensure_chat(session)?;
        let id = session.to_owned();
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
        self.chats.get_mut(&id).unwrap().local.files.push(file);
        self.save_chat(&id)
    }
    pub fn remove_file(&mut self, file: &str) -> Result<()> {
        let id = self
            .account
            .selected
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Select a chat"))?;
        self.chats
            .get_mut(&id)
            .unwrap()
            .local
            .files
            .retain(|f| f.id != file);
        self.save_chat(&id)
    }
    pub fn send_prompt(&mut self) -> Result<()> {
        let session = self.account.selected.clone().ok_or_else(|| anyhow::anyhow!("Select a chat"))?;
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
        let durable=command.journalled_control();
        let encoded=serde_json::to_vec(&command)?;
        let retry=durable.then(||self.account.pending_controls.values().find(|saved|
            serde_json::to_vec(&saved.request.command).is_ok_and(|bytes|bytes==encoded)).map(|saved|saved.request.id.clone())).flatten();
        let request=ClientRequest {id:retry.unwrap_or_else(||uuid::Uuid::new_v4().to_string()),command};
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
        self.requests.insert(request.id.clone(),request.command.clone());
        self.network.as_ref().unwrap().send(Command::Request {epoch,request:request.clone()})?;
        Ok(request.id)
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
        self.chats
            .get_mut(&session)
            .unwrap()
            .local
            .pending
            .retain(|p| p.request.id != id);
        self.save_chat(&session)
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
        ensure!(self.account.pending_create.is_none(), "The previous new chat is still awaiting confirmation");
        let keep_session_id = self.account.selected.clone()
            .filter(|id| self.chats.get(id).is_some_and(|c| c.local.has_work()));
        let request = ClientRequest { id:uuid::Uuid::new_v4().to_string(), command:ClientCommand::CreateSession { keep_session_id, project_id:self.account.selected_project.clone() } };
        let now = crate::clock::now_ms().unwrap_or(0);
        let mut account = self.account.clone();
        account.pending_create = Some(request.clone());
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
    pub fn retry_create_manually(&mut self) -> Result<()> {
        self.create_failed_epoch = None;
        self.retry_create()
    }
    pub fn retry_create(&mut self) -> Result<()> {
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
        let pending = self.chats.get(session).map(|chat| chat.local.pending.iter()
            .filter(|p| matches!(p.status, Delivery::WaitingForChat | Delivery::WaitingForConnection)
                || model_confirmed && p.status == Delivery::WaitingForModel)
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
    pub fn open(&mut self, id: &str) -> Result<()> {
        self.ensure_chat(id)?;
        if self.is_creating(id) { return self.retry_create(); }
        if self.chats[id].feed.opening {
            return Ok(());
        }
        let requests = self.chats[id].local.pending.iter().map(|p|p.request.id.clone()).collect::<Vec<_>>();
        self.request(ClientCommand::GetSession { session_id:id.into() })?;
        for batch in requests.chunks(4) { self.request(ClientCommand::GetReceipts {session_id:id.into(),requests:batch.to_vec()})?; }
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
    /// The data endpoint is authorized independently of control readiness.
    pub fn content_authorized(&self) -> bool { self.remote.authorized() }
    pub fn attachment_path(&self, session:&str, entry:&str) -> PathBuf {
        let source=self.remote.lineage().unwrap_or_default();
        self.store.attachment_path(&format!("{}:{source}",self.identity),session,entry)
    }
    pub fn download(&mut self, session: &str, entry: &str, limit: u64) -> Result<PathBuf> {
        let path = self.attachment_path(session,entry);
        if self.remote.file_ready(session,&format!("file:{entry}"),&path,limit)? {return Ok(path);}
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
            Download {
                path: path.clone(),
                status: tau_transfer::TransferStatus {
                    transferred: 0,
                    total: 0,
                    network_bytes: 0,
                    done: false,
                    failure: None,
                },
            },
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
            if matches!(command, ClientCommand::CreateProject { .. } | ClientCommand::UpdateProject { .. } | ClientCommand::DeleteProject { .. }) {
                self.project_result = Some((id.into(), false));
            }
            self.notice = Some(detail.into());
        }
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
        changed
    }
    fn refresh_blocks(&mut self, scope: &str) -> Result<()> {
        if let Some(view) = self.remote.snapshot(scope)? {
            let snapshot=view.snapshot;
            let chat = self.chats.get_mut(scope).context("Unknown cached chat")?;
            let delivered = snapshot.events.iter().filter(|e|e.phase == EventPhase::Saved).filter_map(|e|e.origin.request_id.clone()).collect::<Vec<_>>();
            chat.feed.snapshot(snapshot)?;
            chat.feed.block_lengths = view.lengths;
            chat.feed.incomplete=view.incomplete;
            chat.feed.block_states=view.states;
            chat.feed.synchronized = chat.feed.queue.available;
            chat.feed.opening = false;
            let before = chat.local.pending.len();
            chat.local.reconcile_complete(&chat.feed.queue, &delivered,&chat.feed.incomplete);
            if chat.local.pending.len() != before { self.store.save_chat(&self.identity,scope,&chat.local)?; }
        }
        Ok(())
    }
    pub fn cancel_copy(&mut self) {self.copy=None;self.copied=None;}
    pub fn copy_details(&mut self, scope:&str, ids:Vec<String>) -> Result<()> {
        self.cancel_copy();
        // Local demo/renderer fixtures have complete events without a remote
        // cache. Native views fetch missing bytes as an explicit copy interest.
        if self.remote.snapshot(scope)?.is_none() {
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
        self.viewport=Some((scope.into(),ids));
    }
    fn watch_blocks(&mut self) -> Result<()> {
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
            transport::Event::Ready(epoch) => {
                self.epoch = Some(epoch);
                self.create_failed_epoch = None;
                self.control_check=None;
                for saved in self.account.pending_controls.values_mut() {saved.blocked=false;}
                self.connection = "Connected".into();
                self.health.connected();
                self.request(ClientCommand::ListSessions)?;
                for id in self.chats.keys().cloned().collect::<Vec<_>>() {
                    if !self.is_creating(&id) { self.open(&id)?; self.send_waiting(&id, false)?; }
                }
                if let Some(id) = self.account.selected.clone().filter(|id| !self.is_creating(id)) {
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
                self.downloads.insert(key, Download { status, path });
            }
            transport::Event::Message(epoch, message) if self.epoch == Some(epoch) => {
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
            ServerMessage::Projects { projects } => {
                self.account.projects = projects;
                if !self.account.projects.iter().any(|p| p.id == self.account.selected_project) {
                    self.account.selected_project = general_project_id();
                }
                self.store.put(&self.identity, "account", &self.account)?;
            }
            ServerMessage::Sessions { sessions } => {
                // Preserve a provisional local chat across server list refreshes.
                // Remote deletion still clears a genuinely missing selection.
                let pending = self.account.pending_create.clone();
                let confirmed = pending.as_ref().and_then(|request| {
                    sessions.iter().any(|s| s.id == request.id).then(|| request.id.clone())
                });
                if self.account.selected.as_ref().is_some_and(|id| !sessions.iter().any(|s| &s.id == id)
                    && !pending.as_ref().is_some_and(|request| &request.id == id)) {
                    self.account.selected = None;
                }
                for s in &sessions {
                    if s.starter { self.account.read_at.insert(s.id.clone(), s.updated_at_ms); }
                }
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
                session_id,
                status,
                context_usage,
                detail,
            } => {
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
        Ok(())
    }
}
