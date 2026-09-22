use crate::settings::SettingsExt;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock as StdRwLock};
use std::time::{Duration, Instant};
use anyhow::{Context, Result, bail};
use serde_json::json;
use tokio::fs;
use tokio::sync::{Mutex, broadcast};
use tracing::warn;

use crate::agent::{AgentSession, auth::AuthStore};
use crate::config::Config;
use crate::protocol::{ContextUsage, PromptDisposition, QueueOperation, ServerMessage, SessionStatus, SessionSummary, MAX_PROMPT_CHARS, MAX_TITLE_CHARS};
use crate::settings::{SettingsStore, Settings};
use crate::state::{StateStore, Receipt};
use crate::transcript::{QueuedRequest, QueueControl, Transcript};

const EVENT_BUFFER: usize = 2048;
pub struct PromptOutcome { pub disposition: PromptDisposition, pub notice: Option<String> }
#[derive(Clone)]
pub struct AgentManager { pub(crate) inner: Arc<ManagerInner> }
pub(crate) struct ManagerInner {
    pub config: Config,
    pub state: StateStore,
    pub settings: SettingsStore,
    pub http: reqwest::Client,
    pub auth: AuthStore,
    pub runtimes: Mutex<HashMap<String, Arc<SessionRuntime>>>,
    pub events: broadcast::Sender<ServerMessage>,
    pub shutting_down: AtomicBool,
}
pub(crate) struct SessionRuntime {
    pub operation: Mutex<()>,
    pub content: Mutex<SessionContent>,
    pub state: StdRwLock<RuntimeState>,
}
pub(crate) struct SessionContent {
    pub agent: Option<AgentSession>,
    pub transcript: Option<Transcript>,
    pub events: broadcast::Sender<Arc<ServerMessage>>,
}
impl Default for SessionContent {
    fn default() -> Self { Self { agent: None, transcript: None, events: broadcast::channel(EVENT_BUFFER).0 } }
}
pub struct SessionFeed { pub initial: Vec<ServerMessage>, pub events: broadcast::Receiver<Arc<ServerMessage>> }
#[derive(Clone, Default)]
pub(crate) struct RuntimeState { pub status: SessionStatus, pub detail: Option<String>, pub idle_since: Option<Instant>, pub context_usage: Option<ContextUsage> }
impl SessionRuntime {
    fn new() -> Self { Self { operation: Mutex::new(()), content: Mutex::new(SessionContent::default()), state: StdRwLock::new(RuntimeState::default()) } }
    pub fn snapshot(&self) -> RuntimeState { self.state.read().unwrap_or_else(|e| e.into_inner()).clone() }
}
impl AgentManager {
    pub async fn new(config: Config, state: StateStore) -> Result<Self> {
        let settings = SettingsStore::load(&config, crate::state::DEFAULT_TITLE_PROMPT.into()).await?;
        let http = reqwest::Client::builder().connect_timeout(Duration::from_secs(30)).redirect(reqwest::redirect::Policy::none()).build()?;
        let auth = AuthStore::new(config.settings_path.with_file_name("auth.json"), http.clone()).shared_codex(config.codex_auth_source.clone());
        Ok(Self { inner: Arc::new(ManagerInner { config, state, settings, http, auth,
            runtimes: Mutex::new(HashMap::new()), events: broadcast::channel(EVENT_BUFFER).0, shutting_down: AtomicBool::new(false) }) })
    }
    pub fn subscribe(&self) -> broadcast::Receiver<ServerMessage> { self.inner.events.subscribe() }
    pub async fn sessions_message(&self) -> Result<ServerMessage> {
        let runtimes = self.inner.runtimes.lock().await;
        Ok(ServerMessage::Sessions { sessions: self.inner.state.list().await?.into_iter().map(|(id, s)| {
            let runtime = runtimes.get(&id).map(|r| r.snapshot()).unwrap_or_default();
            SessionSummary { id, title:s.title, starter:s.starter, status:runtime.status, detail:runtime.detail, context_usage:runtime.context_usage,
                model:Some(s.model), parent_id:s.parent_id, created_at_ms:s.created_at_ms, updated_at_ms:s.updated_at_ms }
        }).collect() })
    }
    pub async fn create_session(&self, keep_session_id: Option<&str>) -> Result<String> {
        if self.inner.shutting_down.load(Ordering::Acquire) { bail!("Tau is shutting down"); }
        let settings = self.inner.settings.get(); let model = settings.agent.model.clone();
        let thinking = settings.agent.model_thinking_levels.get(&format!("{}/{}",model.provider,model.model_id)).unwrap_or(&settings.agent.thinking_level).clone();
        let mut keep=keep_session_id.map(str::to_owned);
        loop {
            let id = self.inner.state.create(model.clone(),thinking.clone(),keep.take()).await?;
            let runtime = self.runtime(&id).await?; let _guard = runtime.operation.lock().await;
            let mut content=runtime.content.lock().await;
            self.ensure_loaded(&id,&runtime,&mut content).await?;
            if !self.inner.state.get(&id).await?.is_some_and(|s|s.starter) { continue; }
            if content.agent.as_ref().is_some_and(|agent|agent.model != model || agent.thinking != thinking) {
                content.append(&id,json!({"type":"model_change","provider":model.provider,"modelId":model.model_id,"thinkingLevel":thinking})).await?;
            }
            drop(content); self.broadcast_sessions().await; return Ok(id);
        }
    }
    pub async fn open_session(&self, id: &str, requests: &[String]) -> Result<SessionFeed> {
        let runtime = self.runtime(id).await?;
        let mut content = runtime.content.lock().await;
        self.ensure_loaded(id, &runtime, &mut content).await?;
        let mut snapshot = content.transcript.as_ref().unwrap().snapshot();
        snapshot.delivered = self.inner.state.delivered(id,requests).await?;
        let state = runtime.snapshot();
        Ok(SessionFeed { initial: vec![ServerMessage::TranscriptSnapshot { session_id:id.into(), snapshot },
            ServerMessage::SessionState { session_id:id.into(), status:state.status, detail:state.detail, context_usage:state.context_usage }], events:content.events.subscribe() })
    }
    pub async fn history_page(&self, id: &str, generation: &str, before: u64) -> Result<crate::transcript::HistoryPage> {
        let runtime = self.runtime(id).await?; let content = runtime.content.lock().await;
        let transcript = content.transcript.as_ref().context("Open this chat before reading history")?;
        if transcript.generation != generation { bail!("History changed; reopen this chat"); }
        self.inner.state.page(id,Some(before)).await
    }
    pub async fn prompt(&self, id: &str, text: &str, request_id: &str) -> Result<PromptOutcome> {
        if text.trim().is_empty() || text.chars().count() > MAX_PROMPT_CHARS { bail!("Message must contain 1–{MAX_PROMPT_CHARS} characters"); }
        if request_id.is_empty() || request_id.len() > 128 { bail!("Invalid request ID"); }
        let runtime = self.runtime(id).await?;
        let _guard = runtime.operation.lock().await;
        let mut content = runtime.content.lock().await;
        self.ensure_loaded(id, &runtime, &mut content).await?;
        if let Some(receipt) = self.inner.state.receipt(id,request_id).await? {
            if receipt.command.as_deref().is_some_and(|kind|kind != "builtin") { bail!("Request ID was already used for another operation"); }
            if receipt.text != text { bail!("Request ID was already used for different text"); }
            if !receipt.finished { bail!("This command was interrupted; inspect its effects before issuing a new request ID"); }
            if let Some(error) = receipt.error { bail!("{error}"); }
            return Ok(PromptOutcome { disposition:receipt.disposition,notice:receipt.notice });
        }
        if let Some(rest) = text.strip_prefix('/') {
            let (name, args) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
            if ["compact", "model", "thinking", "name", "fast"].contains(&name) {
                let receipt = Receipt { id:request_id.into(),command:Some("builtin".into()),text:text.into(),disposition:PromptDisposition::Handled,finished:false,notice:None,error:None };
                content.commit(id,Vec::new(),None,Some(receipt.clone())).await?;
                drop(content);
                let result = self.run_builtin_command(id, &runtime, name, args.trim()).await;
                self.inner.state.finish_command(id,receipt,&result).await?;
                return result;
            }
            if ["settings", "login", "logout", "reload", "tau-fork-at", "tree", "new", "resume", "fork", "clone", "scoped-models", "export", "import", "share", "copy", "session", "changelog", "hotkeys", "trust", "quit"].contains(&name) {
                bail!("Use Tau's menu for /{name}; Pi terminal extensions are not loaded");
            }
        }
        let disposition = if content.agent.as_ref().unwrap().running || content.transcript.as_ref().unwrap().queue.paused { PromptDisposition::Queued } else { PromptDisposition::Submitted };
        let mut queue = content.transcript.as_ref().unwrap().queue.clone();
        if queue.requests.len() >= 256 { bail!("Queue is full (256 messages)"); }
        queue.requests.push(QueuedRequest { request_id:request_id.into(), revision:0, kind:"steer".into(), text:text.into(), images:0, timestamp_ms:Some(crate::agent::now_ms()) });
        content.save_queue(id,queue,Some(Receipt { id:request_id.into(),command:None,text:text.into(),disposition,finished:true,notice:None,error:None })).await?;
        // Queue, receipt and retained session metadata commit together before acknowledgement.
        self.start_run(id, &runtime, &mut content);
        drop(content);
        let manager = self.clone(); let session = id.to_owned(); let text = text.to_owned();
        tokio::spawn(async move { manager.title_after_prompt(&session, &text).await; });
        Ok(PromptOutcome { disposition, notice:None })
    }
    pub async fn queue_control(&self, id: &str, generation: &str, command_id: &str, operation: QueueOperation) -> Result<String> {
        let runtime = self.runtime(id).await?; let _guard = runtime.operation.lock().await;
        let mut content = runtime.content.lock().await;
        self.ensure_loaded(id, &runtime, &mut content).await?;
        let payload=json!({"generation":generation,"operation":operation}).to_string();
        if let Some(receipt)=self.inner.state.receipt(id,command_id).await? {
            if receipt.command.as_deref() != Some("queue_control") || receipt.text != payload { bail!("Request ID was already used for another control"); }
            return Ok("accepted".into());
        }
        let transcript = content.transcript.as_ref().unwrap();
        if transcript.generation != generation { bail!("Queue changed; reopen this chat"); }
        let mut queue = transcript.queue.clone();
        if matches!(operation, QueueOperation::Pause { .. } | QueueOperation::Prefix { .. } | QueueOperation::Resume { .. })
            && queue.control.as_ref().is_some_and(|control| control.status == "waiting") { bail!("Cancel the pending queue control first"); }
        match operation {
            QueueOperation::Edit { request_id, revision, text } => {
                if text.trim().is_empty() || text.chars().count() > MAX_PROMPT_CHARS { bail!("Invalid queue text"); }
                let request = queue.requests.iter_mut().find(|r| r.request_id == request_id && r.revision == revision).context("Queued message changed")?;
                request.text = text; request.revision = revision.checked_add(1).context("Queue revision exhausted")?;
            }
            QueueOperation::Delete { request_id, revision } => {
                let index = queue.requests.iter().position(|r| r.request_id == request_id && r.revision == revision).context("Queued message changed")?;
                queue.requests.remove(index);
            }
            QueueOperation::Pause { run_id, boundary } | QueueOperation::Prefix { run_id, boundary, requests: _ } if boundary != "turn" || run_id != queue.run_id => {
                bail!("Unsupported boundary or stale run ID");
            }
            QueueOperation::Pause { run_id, boundary } => {
                queue.control = Some(QueueControl { command_id:command_id.into(), run_id, action:"pause".into(), boundary:Some(boundary), requests:vec![], status:"waiting".into(), detail:None });
                if !content.agent.as_ref().unwrap().running { queue.paused = true; queue.control.as_mut().unwrap().status = "applied".into(); }
            }
            QueueOperation::Prefix { run_id, boundary, requests } => {
                if requests.is_empty() || requests.len() > queue.requests.len() || requests.iter().zip(&queue.requests).any(|(a,b)| a.request_id != b.request_id || a.revision != b.revision) { bail!("Queue prefix changed"); }
                queue.control = Some(QueueControl { command_id:command_id.into(), run_id, action:"prefix".into(), boundary:Some(boundary), requests, status:"waiting".into(), detail:None });
                queue.paused = false;
            }
            QueueOperation::Resume { run_id } => {
                if run_id != queue.run_id { bail!("Run changed"); }
                queue.paused = false; queue.control = None;
            }
            QueueOperation::Cancel { control_id } => {
                if !queue.control.as_ref().is_some_and(|control| control.command_id == control_id && control.status == "waiting") { bail!("Control is no longer pending"); }
                queue.control = None;
            }
        }
        if let Some(control) = &queue.control && control.action == "prefix" && control.status == "waiting"
            && (control.requests.len() > queue.requests.len() || control.requests.iter().zip(&queue.requests).any(|(a,b)| a.request_id != b.request_id || a.revision != b.revision)) {
            bail!("Cancel the pending prefix before editing its messages");
        }
        content.save_queue(id, queue, Some(Receipt { id:command_id.into(),command:Some("queue_control".into()),text:payload,disposition:PromptDisposition::Handled,finished:true,notice:None,error:None })).await?;
        self.start_run(id, &runtime, &mut content);
        Ok("accepted".into())
    }
    pub async fn abort(&self, id: &str, request_id: &str) -> Result<()> {
        let runtime = self.runtime(id).await?;
        let mut content = runtime.content.lock().await;
        self.ensure_loaded(id,&runtime,&mut content).await?;
        if let Some(receipt)=self.inner.state.receipt(id,request_id).await? {
            if receipt.command.as_deref() != Some("abort") { bail!("Request ID was already used for another operation"); }
            return Ok(());
        }
        let mut queue=content.transcript.as_ref().unwrap().queue.clone();
        if let Some(agent)=&content.agent {
            agent.cancel.cancel();
            if agent.running || !queue.requests.is_empty() { queue.paused=true; }
        }
        content.save_queue(id,queue,Some(Receipt { id:request_id.into(),command:Some("abort".into()),text:String::new(),disposition:PromptDisposition::Handled,finished:true,notice:None,error:None })).await
    }
    pub async fn close_session(&self, id: &str) -> Result<()> {
        let runtime = self.runtime(id).await?;
        if let Some(agent) = &runtime.content.lock().await.agent { agent.cancel.cancel(); }
        let _guard = runtime.operation.lock().await;
        self.retire_session(id, &runtime).await;
        self.broadcast_sessions().await; Ok(())
    }
    async fn retire_session(&self, id: &str, runtime: &Arc<SessionRuntime>) {
        let task = {
            let mut content = runtime.content.lock().await;
            content.agent.as_mut().and_then(|agent| { agent.cancel.cancel(); agent.task.take() })
        };
        if let Some(task) = task && let Err(error) = task.await { warn!(%error, "Agent task stopped unexpectedly"); }
        let mut content = runtime.content.lock().await;
        content.agent = None; content.transcript = None;
        let _ = content.events.send(Arc::new(ServerMessage::ResyncRequired { session_id:Some(id.into()) }));
        self.set_runtime_state(id, runtime, SessionStatus::Sleeping, None, Some(None));
    }
    pub async fn delete_session(&self, id: &str) -> Result<()> {
        let runtime = self.runtime(id).await?;
        if let Some(agent) = &runtime.content.lock().await.agent { agent.cancel.cancel(); }
        let _guard = runtime.operation.lock().await;
        self.retire_session(id, &runtime).await;
        self.inner.state.remove(id).await?;
        self.inner.runtimes.lock().await.remove(id);
        match fs::remove_dir_all(self.inner.config.upload_root.join(id)).await { Ok(()) => {}, Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}, Err(e) => return Err(e.into()) }
        self.broadcast_sessions().await; Ok(())
    }
    pub async fn rename_session(&self, id: &str, title: &str) -> Result<()> {
        let title = title.trim();
        if title.is_empty() || title.contains(['\n','\r']) || title.chars().count() > MAX_TITLE_CHARS { bail!("Invalid session title"); }
        let runtime = self.runtime(id).await?; let _guard = runtime.operation.lock().await;
        self.inner.state.rename(id, title.into(), false).await?;
        self.broadcast_sessions().await; Ok(())
    }
    pub async fn fork_session(&self, id: &str, entry_id: &str) -> Result<(String, Option<String>)> { self.branch_session(id, Some(entry_id)).await }
    pub async fn clone_session(&self, id: &str) -> Result<String> { Ok(self.branch_session(id, None).await?.0) }
    async fn branch_session(&self, id: &str, entry_id: Option<&str>) -> Result<(String, Option<String>)> {
        let runtime = self.runtime(id).await?; let _guard = runtime.operation.lock().await;
        let _content = runtime.content.lock().await;
        let result = self.inner.state.branch(id,entry_id).await?;
        self.broadcast_sessions().await; Ok(result)
    }
    pub async fn shutdown(&self) {
        if self.inner.shutting_down.swap(true, Ordering::AcqRel) { return; }
        let runtimes = self.inner.runtimes.lock().await.iter().map(|(id,r)| (id.clone(),r.clone())).collect::<Vec<_>>();
        for (_,runtime) in &runtimes { if let Some(agent) = &runtime.content.lock().await.agent { agent.cancel.cancel(); } }
        for (id,runtime) in runtimes { let _guard = runtime.operation.lock().await; self.retire_session(&id, &runtime).await; }
    }
    pub(crate) async fn runtime(&self, id: &str) -> Result<Arc<SessionRuntime>> {
        if self.inner.shutting_down.load(Ordering::Acquire) { bail!("Tau is shutting down"); }
        if self.inner.state.get(id).await?.is_none() { bail!("Unknown session {id}"); }
        Ok(self.inner.runtimes.lock().await.entry(id.into()).or_insert_with(|| Arc::new(SessionRuntime::new())).clone())
    }
    pub(crate) async fn ensure_loaded(&self, id: &str, runtime: &Arc<SessionRuntime>, content: &mut SessionContent) -> Result<()> {
        if self.inner.shutting_down.load(Ordering::Acquire) { bail!("Tau is shutting down"); }
        if content.agent.is_some() { return Ok(()); }
        let stored = self.inner.state.get(id).await?.context("Unknown session")?;
        let settings = self.inner.settings.get();
        let mut queue = self.inner.state.queue(id).await?;
        queue.run_id = None; queue.control = None;
        if !queue.requests.is_empty() || stored.needs_turn { queue.paused = true; }
        let detail = (queue.paused && (stored.needs_turn || !queue.requests.is_empty())).then(|| "Pending work is paused; resume when ready".to_owned());
        let usage = settings.model(&stored.model).ok().map(|model| ContextUsage { tokens:stored.tokens,context_window:model.context_window });
        let page = self.inner.state.page(id,None).await?;
        content.transcript = Some(Transcript::new(page,stored.head,stored.next_order,queue));
        content.agent = Some(AgentSession { store:self.inner.state.clone(), revision:stored.revision, model:stored.model, thinking:stored.thinking,
            running:false, cancel:tokio_util::sync::CancellationToken::new(), task:None, tokens:stored.tokens, needs_turn:stored.needs_turn });
        let _ = content.events.send(Arc::new(ServerMessage::ResyncRequired { session_id:Some(id.into()) }));
        self.set_runtime_state(id,runtime,SessionStatus::Idle,detail,Some(usage)); Ok(())
    }
    pub(crate) fn set_runtime_state(&self, id: &str, runtime: &Arc<SessionRuntime>, status: SessionStatus, detail: Option<String>, usage: Option<Option<ContextUsage>>) {
        let mut current = runtime.state.write().unwrap_or_else(|e| e.into_inner());
        let usage = usage.unwrap_or(current.context_usage);
        if current.status == status && current.detail == detail && current.context_usage == usage { return; }
        let schedule = status == SessionStatus::Idle && current.status != SessionStatus::Idle;
        let idle_since = if status == SessionStatus::Idle { current.idle_since.or(Some(Instant::now())) } else { None };
        *current = RuntimeState { status, detail:detail.clone(), context_usage:usage, idle_since }; drop(current);
        let _ = self.inner.events.send(ServerMessage::SessionState { session_id:id.into(), status, detail, context_usage:usage });
        let timeout = self.inner.settings.get().daemon.idle_timeout_seconds;
        if schedule && timeout > 0 {
            let manager = self.clone(); let id = id.to_owned(); let runtime = runtime.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(timeout)).await;
                let _guard = runtime.operation.lock().await;
                let content = runtime.content.lock().await;
                // Unlike a Pi worker, the native runtime owns no unsaved queue.
                // Paused/pending work survives eviction and reopens paused; retaining
                // it here would abandon the one-shot timer and leak idle runtimes.
                if runtime.snapshot().idle_since != idle_since || content.agent.as_ref().is_some_and(|a| a.running) { return; }
                drop(content); manager.retire_session(&id, &runtime).await;
            });
        }
    }
    pub(crate) async fn broadcast_sessions(&self) {
        match self.sessions_message().await { Ok(message) => { let _ = self.inner.events.send(message); }, Err(error) => warn!(%error,"Could not read session list") }
    }
    pub async fn set_settings(&self, revision: u64, settings: Settings) -> Result<Settings> { self.inner.settings.set(revision, settings).await }
}
pub(crate) fn safe_file_name(file_name: &str) -> String {
    let safe = file_name.rsplit(['/', '\\']).next().unwrap_or_default().chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' }).take(160).collect::<String>().trim_matches('.').to_owned();
    if safe.is_empty() { "attachment".into() } else { safe }
}
pub(crate) fn bounded(value: &str, max: usize) -> String { value.chars().take(max).collect() }
