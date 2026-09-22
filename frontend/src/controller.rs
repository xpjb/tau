use crate::{
    feed::Feed,
    store::*,
    transport::{self, Command, Network, Wake},
};
use anyhow::{Result, ensure};
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
    pub settings_result: Option<(String, bool)>,
    pub daemon_settings: Option<(tau_protocol::settings::Settings, String)>,
    pub connection: String,
    pub health: crate::connection::Health,
    pub epoch: Option<u64>,
    pub notice: Option<String>,
    network: Option<Network>,
    requests: HashMap<String, ClientCommand>,
    wake: Wake,
}
impl Controller {
    pub fn new(store: Store, wake: Wake) -> Result<Self> {
        let settings: Settings = store.get("", "settings")?;
        let identity = settings.identity();
        let account = store.get(&identity, "account")?;
        let model_preferences = store.get(&identity, "quick-models")?;
        let mut c = Self {
            store,
            settings,
            identity,
            account,
            model_preferences,
            chats: HashMap::new(),
            downloads: HashMap::new(),
            settings_result: None,
            daemon_settings: None,
            connection: "Not connected".into(),
            health: crate::connection::Health::default(),
            epoch: None,
            notice: None,
            network: None,
            requests: HashMap::new(),
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
        self.network = Some(Network::start(self.settings.clone(), self.wake.clone()));
    }
    pub fn configure(&mut self, settings: Settings) -> Result<()> {
        let settings = settings.normalized();
        settings.url()?;
        self.store.put("", "settings", &settings)?;
        self.network = None;
        self.settings = settings;
        self.identity = self.settings.identity();
        self.account = self.store.get(&self.identity, "account")?;
        self.model_preferences = self.store.get(&self.identity, "quick-models")?;
        self.chats.clear();
        self.downloads.clear();
        self.requests.clear();
        self.daemon_settings = None;
        self.settings_result = None;
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
            self.chats.insert(
                id.to_owned(),
                Chat {
                    local,
                    feed: Feed::default(),
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
            self.account
                .read_at
                .insert(id.into(), session.updated_at_ms);
        }
        self.store.put(&self.identity, "account", &self.account)?;
        if self.epoch.is_some() {
            self.open(id)?;
            self.request(ClientCommand::GetCommands {
                session_id: id.into(),
            })?;
        }
        Ok(())
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
        let epoch = self
            .epoch
            .ok_or_else(|| anyhow::anyhow!("Connect before sending; your draft is saved"))?;
        let session = self
            .account
            .selected
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Select a chat"))?;
        let chat = self.chats.get_mut(&session).unwrap();
        ensure!(
            chat.model_request.is_none(),
            "Wait for model selection to finish; your draft is saved"
        );
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
            status: if files.is_empty() {
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
        let epoch = self.epoch.ok_or_else(|| anyhow::anyhow!("Not connected"))?;
        let request = ClientRequest {
            id: uuid::Uuid::new_v4().to_string(),
            command,
        };
        self.network.as_ref().unwrap().send(Command::Request {
            epoch,
            request: request.clone(),
        })?;
        self.requests.insert(request.id.clone(), request.command);
        Ok(request.id)
    }
    pub fn control(&mut self, command: ClientCommand) -> Result<()> {
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
                _ => "Control requested".into(),
            },
            files: vec![],
            status: Delivery::Sending,
            detail: None,
        });
        self.store.save_chat(&self.identity, &session, &local)?;
        chat.local = local;
        if let Err(error) = self.network.as_ref().unwrap().send(Command::Request {
            epoch,
            request: request.clone(),
        }) {
            self.not_sent(&request.id, &error.to_string())?;
        }
        Ok(())
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
            chat.commands_loaded && chat.model_request.is_none(),
            "Wait for model selection/catalog to finish"
        );
        let slug = crate::models::resolve(selector, &chat.commands)
            .ok_or_else(|| anyhow::anyhow!("That model is not offered by this daemon"))?
            .to_owned();
        // /model already persists the last chosen model in the daemon.
        // Send even when it matches this chat: another chat may have changed
        // the remembered default. Only an explicit tile click reaches here.
        // Preserve drafts/attachments and never replay after reconnect.
        let id = self.request(ClientCommand::Prompt {
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
    pub fn new_chat(&mut self) -> Result<()> {
        let keep_session_id = self
            .account
            .selected
            .clone()
            .filter(|id| self.chats.get(id).is_some_and(|c| c.local.has_work()));
        self.request(ClientCommand::CreateSession { keep_session_id })?;
        Ok(())
    }
    pub fn open(&mut self, id: &str) -> Result<()> {
        self.ensure_chat(id)?;
        if self.chats[id].feed.opening {
            return Ok(());
        }
        let requests = self.chats[id]
            .local
            .pending
            .iter()
            .map(|p| p.request.id.clone())
            .collect();
        self.request(ClientCommand::OpenSession {
            session_id: id.into(),
            requests,
        })?;
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
        if let Some(before) = feed.before {
            self.request(ClientCommand::GetHistory {
                session_id: id.clone(),
                generation: feed.generation.clone(),
                before,
            })?;
            self.chats.get_mut(&id).unwrap().feed.loading = true;
        }
        Ok(())
    }
    pub fn download_key(session: &str, entry: &str) -> String {
        format!("{}:{}", session.len(), session) + entry
    }
    pub fn download(&mut self, session: &str, entry: &str, limit: u64) -> Result<PathBuf> {
        let path = self.store.attachment_path(&self.identity, session, entry);
        if path
            .metadata()
            .is_ok_and(|m| m.is_file() && m.len() <= limit)
        {
            return Ok(path);
        }
        ensure!(self.epoch.is_some(), "Connect to download this file");
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
        if self.requests.remove(id).is_some() {
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
        changed
    }
    fn network_event(&mut self, event: transport::Event) -> Result<()> {
        let fatal = matches!(&event, transport::Event::Fatal(_));
        match event {
            transport::Event::Ready(epoch) => {
                self.epoch = Some(epoch);
                self.connection = "Connected".into();
                self.health.connected();
                self.request(ClientCommand::ListSessions)?;
                for id in self.chats.keys().cloned().collect::<Vec<_>>() {
                    self.open(&id)?;
                }
                if let Some(id) = self.account.selected.clone() {
                    self.request(ClientCommand::GetCommands { session_id: id })?;
                }
            }
            transport::Event::Disconnected(detail) | transport::Event::Fatal(detail) => {
                self.epoch = None;
                self.connection = detail;
                self.health.disconnected(fatal);
                self.requests.clear();
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
                        }
                    }
                    self.store.save_chat(&self.identity, session, &chat.local)?;
                }
            }
            transport::Event::HeartbeatSent { epoch, at } if self.epoch == Some(epoch) => {
                self.health.sent(at)
            }
            transport::Event::HeartbeatReply { epoch, at, rtt, ok }
                if self.epoch == Some(epoch) =>
            {
                self.health.reply(at, rtt, ok)
            }
            transport::Event::NotSent(id, detail) => self.not_sent(&id, &detail)?,
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
            ServerMessage::Sessions { sessions } => {
                self.account.sessions = sessions;
                if let Some(id) = &self.account.selected
                    && let Some(s) = self.account.sessions.iter().find(|s| &s.id == id)
                {
                    self.account.read_at.insert(id.clone(), s.updated_at_ms);
                }
                self.store.put(&self.identity, "account", &self.account)?;
                // Bounded recent/unread/running warming, never starts a worker.
                let warm = self
                    .account
                    .sessions
                    .iter()
                    .filter(|s| !self.chats.contains_key(&s.id))
                    .take(8usize.saturating_sub(self.chats.len()))
                    .map(|s| s.id.clone())
                    .collect::<Vec<_>>();
                if self.epoch.is_some() {
                    for id in warm {
                        self.open(&id)?;
                    }
                }
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
                    chat.local.reconcile(&chat.feed.queue, &delivered);
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
                        chat.local.reconcile(&chat.feed.queue, &delivered);
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
                default_system_prompt,
                ..
            } => {
                self.daemon_settings = Some((*settings, default_system_prompt));
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
                                || !matches!(p.request.command, ClientCommand::Prompt { .. }))
                        {
                            chat.local.pending.retain(|p| p.request.id != request_id);
                        }
                        self.store.save_chat(&self.identity, id, &chat.local)?;
                    }
                }
                let command = self.requests.remove(&request_id);
                if matches!(
                    command,
                    Some(ClientCommand::SetSettings { .. } | ClientCommand::GetSettings)
                ) {
                    self.settings_result = Some((request_id.clone(), ok && !uncertain));
                }
                if model_changed {
                    self.request(ClientCommand::ListSessions)?;
                }
                if !ok {
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
                        Some(
                            ClientCommand::CreateSession { .. }
                            | ClientCommand::ForkSession { .. }
                            | ClientCommand::CloneSession { .. },
                        ) => {
                            if let Some(id) = session_id {
                                self.select(&id)?;
                                if let Some(draft) = draft {
                                    self.draft(draft)?;
                                }
                            }
                        }
                        Some(ClientCommand::DeleteSession { session_id }) => {
                            self.store.delete_chat(&self.identity, &session_id)?;
                            self.chats.remove(&session_id);
                            if self.account.selected.as_ref() == Some(&session_id) {
                                self.account.selected = None;
                            }
                            self.store.put(&self.identity, "account", &self.account)?;
                        }
                        _ => {}
                    }
                }
            }
            ServerMessage::Hello { .. } => {}
        }
        Ok(())
    }
}
