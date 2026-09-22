use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock as StdRwLock};
use std::time::{Duration, Instant};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::fs;
use tokio::sync::{Mutex, broadcast};
use tracing::warn;

use crate::agent::{AgentSession, auth::AuthStore, journal::Journal};
use crate::config::Config;
use crate::protocol::{ContextUsage, PromptDisposition, QueueOperation, ServerMessage, SessionStatus, SessionSummary, MAX_PROMPT_CHARS, MAX_TITLE_CHARS};
use crate::settings::{SettingsStore, Settings};
use crate::state::StateStore;
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
        let settings = SettingsStore::load(&config, state.legacy_title_prompt()).await?;
        state.clear_legacy_title_prompt().await?;
        let http = reqwest::Client::builder().connect_timeout(Duration::from_secs(30)).redirect(reqwest::redirect::Policy::none()).build()?;
        let auth = AuthStore::new(config.settings_path.with_file_name("auth.json"), http.clone());
        Ok(Self { inner: Arc::new(ManagerInner { config, state, settings, http, auth,
            runtimes: Mutex::new(HashMap::new()), events: broadcast::channel(EVENT_BUFFER).0, shutting_down: AtomicBool::new(false) }) })
    }
    pub fn subscribe(&self) -> broadcast::Receiver<ServerMessage> { self.inner.events.subscribe() }
    pub async fn sessions_message(&self) -> ServerMessage {
        let runtimes = self.inner.runtimes.lock().await;
        ServerMessage::Sessions { sessions: self.inner.state.list().into_iter().filter(|(_, s)| !s.starter || s.model.is_some()).map(|(id, s)| {
            let runtime = runtimes.get(&id).map(|r| r.snapshot()).unwrap_or_default();
            SessionSummary { id, title:s.title, starter:s.starter, status:runtime.status, detail:runtime.detail, context_usage:runtime.context_usage,
                model:s.model, parent_id:s.parent_id, created_at_ms:s.created_at_ms, updated_at_ms:s.updated_at_ms }
        }).collect() }
    }
    pub async fn create_session(&self, keep_session_id: Option<&str>) -> Result<String> {
        if let Some(id) = keep_session_id { self.inner.state.retain(id).await?; }
        loop {
            if self.inner.shutting_down.load(Ordering::Acquire) { bail!("Tau is shutting down"); }
            let id = self.inner.state.create("New chat".into(), None, None, None, true).await?;
            let runtime = self.runtime(&id).await?;
            let _guard = runtime.operation.lock().await;
            let mut content = runtime.content.lock().await;
            self.ensure_loaded(&id, &runtime, &mut content).await?;
            let used = content.agent.as_ref().is_some_and(|agent| !agent.receipts.is_empty())
                || content.transcript.as_mut().is_some_and(|t| t.events_mut().next().is_some());
            if used { self.inner.state.retain(&id).await?; continue; }
            if !self.inner.state.get(&id).is_some_and(|s| s.starter) { continue; }
            drop(content); self.broadcast_sessions().await; return Ok(id);
        }
    }
    pub async fn open_session(&self, id: &str, requests: &[String]) -> Result<SessionFeed> {
        let runtime = self.runtime(id).await?;
        let mut content = runtime.content.lock().await;
        self.ensure_loaded(id, &runtime, &mut content).await?;
        let mut snapshot = content.transcript.as_ref().unwrap().snapshot(requests);
        snapshot.delivered = requests.iter().filter(|id| content.agent.as_ref().unwrap().receipts.contains_key(*id)).cloned().collect();
        let state = runtime.snapshot();
        Ok(SessionFeed { initial: vec![ServerMessage::TranscriptSnapshot { session_id:id.into(), snapshot },
            ServerMessage::SessionState { session_id:id.into(), status:state.status, detail:state.detail, context_usage:state.context_usage }], events:content.events.subscribe() })
    }
    pub async fn history_page(&self, id: &str, generation: &str, before: u64) -> Result<crate::transcript::HistoryPage> {
        let runtime = self.runtime(id).await?; let content = runtime.content.lock().await;
        let transcript = content.transcript.as_ref().context("Open this chat before reading history")?;
        if transcript.generation != generation { bail!("History changed; reopen this chat"); }
        Ok(transcript.page(Some(before)))
    }
    pub async fn prompt(&self, id: &str, text: &str, request_id: &str) -> Result<PromptOutcome> {
        if text.trim().is_empty() || text.chars().count() > MAX_PROMPT_CHARS { bail!("Message must contain 1–{MAX_PROMPT_CHARS} characters"); }
        if request_id.is_empty() || request_id.len() > 128 { bail!("Invalid request ID"); }
        let runtime = self.runtime(id).await?;
        let _guard = runtime.operation.lock().await;
        let mut content = runtime.content.lock().await;
        self.ensure_loaded(id, &runtime, &mut content).await?;
        if let Some((original, disposition)) = content.agent.as_ref().unwrap().receipts.get(request_id) {
            if original != text { bail!("Request ID was already used for different text"); }
            return Ok(PromptOutcome { disposition:*disposition, notice:None });
        }
        if let Some(rest) = text.strip_prefix('/') {
            let (name, args) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
            if ["compact", "model", "thinking", "name", "fast"].contains(&name) {
                drop(content);
                let outcome = self.run_builtin_command(id, &runtime, name, args.trim()).await?;
                let mut content = runtime.content.lock().await;
                let queue = content.transcript.as_ref().unwrap().queue.clone();
                content.save_queue(id, queue, Some(json!({"id":request_id,"text":text,"disposition":outcome.disposition}))).await?;
                content.agent.as_mut().unwrap().receipts.insert(request_id.into(), (text.into(), outcome.disposition));
                return Ok(outcome);
            }
            if ["settings", "login", "logout", "reload", "tau-fork-at", "tree", "new", "resume", "fork", "clone", "scoped-models", "export", "import", "share", "copy", "session", "changelog", "hotkeys", "trust", "quit"].contains(&name) {
                bail!("Use Tau's menu for /{name}; Pi terminal extensions are not loaded");
            }
        }
        let disposition = if content.agent.as_ref().unwrap().running || content.transcript.as_ref().unwrap().queue.paused { PromptDisposition::Queued } else { PromptDisposition::Submitted };
        let mut queue = content.transcript.as_ref().unwrap().queue.clone();
        if queue.requests.len() >= 256 { bail!("Queue is full (256 messages)"); }
        queue.requests.push(QueuedRequest { request_id:request_id.into(), revision:0, kind:"steer".into(), text:text.into(), images:0, timestamp_ms:Some(crate::agent::now_ms()) });
        content.save_queue(id, queue, Some(json!({"id":request_id,"text":text,"disposition":disposition}))).await?;
        content.agent.as_mut().unwrap().receipts.insert(request_id.into(), (text.into(), disposition));
        // Acceptance is the synced queue record, not provider availability or title generation.
        if let Err(error) = self.inner.state.retain(id).await { warn!(session=id, %error, "Accepted prompt; metadata will recover from journal"); }
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
        content.save_queue(id, queue, None).await?;
        self.start_run(id, &runtime, &mut content);
        Ok("accepted".into())
    }
    pub async fn abort(&self, id: &str) -> Result<()> {
        let runtime = self.runtime(id).await?;
        let content = runtime.content.lock().await;
        if let Some(agent) = &content.agent { agent.cancel.cancel(); }
        Ok(())
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
        let stored = self.inner.state.get(id).context("Unknown session")?;
        let path = if let Some(path) = &stored.session_file {
            match fs::canonicalize(path).await {
                Ok(path) if path.starts_with(fs::canonicalize(&self.inner.config.session_dir).await?) => Some(path),
                Ok(_) => bail!("Session history is outside the session directory"),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(error.into()),
            }
        } else { None };
        self.inner.state.remove(id).await?;
        self.inner.runtimes.lock().await.remove(id);
        if let Some(path) = path { fs::remove_file(path).await?; }
        match fs::remove_dir_all(self.inner.config.upload_root.join(id)).await { Ok(()) => {}, Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}, Err(e) => return Err(e.into()) }
        self.broadcast_sessions().await; Ok(())
    }
    pub async fn rename_session(&self, id: &str, title: &str) -> Result<()> {
        let title = title.trim();
        if title.is_empty() || title.contains(['\n','\r']) || title.chars().count() > MAX_TITLE_CHARS { bail!("Invalid session title"); }
        let runtime = self.runtime(id).await?; let _guard = runtime.operation.lock().await;
        self.inner.state.rename(id, title.into()).await?;
        self.broadcast_sessions().await; Ok(())
    }
    pub async fn fork_session(&self, id: &str, entry_id: &str) -> Result<(String, Option<String>)> { self.branch_session(id, Some(entry_id)).await }
    pub async fn clone_session(&self, id: &str) -> Result<String> { Ok(self.branch_session(id, None).await?.0) }
    async fn branch_session(&self, id: &str, entry_id: Option<&str>) -> Result<(String, Option<String>)> {
        let runtime = self.runtime(id).await?; let _guard = runtime.operation.lock().await;
        let mut content = runtime.content.lock().await; self.ensure_loaded(id, &runtime, &mut content).await?;
        let agent = content.agent.as_ref().unwrap();
        let mut entries = agent.journal.entries.clone(); let mut draft = None;
        if let Some(entry_id) = entry_id {
            let index = entries.iter().position(|entry| entry["id"] == entry_id).context("Fork entry does not exist")?;
            let user = entries[index]["message"]["role"] == "user";
            if user { draft = entries[index]["message"]["content"].as_str().map(str::to_owned); }
            entries.truncate(index + usize::from(!user));
        }
        // A child owns history, never its parent's pending work or request receipts.
        entries.retain(|entry| entry["type"] != "tau_queue");
        let mut parent = None;
        for entry in &mut entries { entry["parentId"] = json!(parent); parent = entry["id"].as_str().map(str::to_owned); }
        let file = self.inner.config.session_dir.join(format!("{}.jsonl", uuid::Uuid::new_v4()));
        let mut bytes = format!("{}\n", json!({"type":"session","version":3,"tauVersion":1})).into_bytes();
        for entry in &entries { bytes.extend(serde_json::to_vec(entry)?); bytes.push(b'\n'); }
        crate::settings::atomic_write(&file, &bytes).await?;
        let stored = self.inner.state.get(id).context("Session disappeared")?;
        let model = crate::transcript::session_model_from_file(&file).await?.or(stored.model);
        let child = self.inner.state.create(bounded(&format!("{}{}", if entry_id.is_some() {"Fork of "} else {"Copy of "}, stored.title), MAX_TITLE_CHARS),
            Some(id.into()), Some(file.to_string_lossy().into_owned()), model, false).await?;
        drop(content); self.broadcast_sessions().await; Ok((child, draft))
    }
    pub async fn shutdown(&self) {
        if self.inner.shutting_down.swap(true, Ordering::AcqRel) { return; }
        let runtimes = self.inner.runtimes.lock().await.iter().map(|(id,r)| (id.clone(),r.clone())).collect::<Vec<_>>();
        for (_,runtime) in &runtimes { if let Some(agent) = &runtime.content.lock().await.agent { agent.cancel.cancel(); } }
        for (id,runtime) in runtimes { let _guard = runtime.operation.lock().await; self.retire_session(&id, &runtime).await; }
    }
    pub(crate) async fn runtime(&self, id: &str) -> Result<Arc<SessionRuntime>> {
        if self.inner.shutting_down.load(Ordering::Acquire) { bail!("Tau is shutting down"); }
        if self.inner.state.get(id).is_none() { bail!("Unknown session {id}"); }
        Ok(self.inner.runtimes.lock().await.entry(id.into()).or_insert_with(|| Arc::new(SessionRuntime::new())).clone())
    }
    pub(crate) async fn ensure_loaded(&self, id: &str, runtime: &Arc<SessionRuntime>, content: &mut SessionContent) -> Result<()> {
        if self.inner.shutting_down.load(Ordering::Acquire) { bail!("Tau is shutting down"); }
        let stored = self.inner.state.get(id).context("Unknown session")?;
        if content.agent.is_some() { return Ok(()); }
        let settings = self.inner.settings.get();
        let journal = Journal::open(&self.inner.config.session_dir, stored.session_file.as_deref(), &settings).await?;
        let (model, thinking, mut queue, receipts) = journal.restored(&settings)?;
        let mut tokens = None;
        for entry in journal.entries.iter().rev() {
            if entry["type"] == "compaction" || entry["type"] == "model_change" { break; }
            if entry["message"]["role"] == "assistant" {
                let usage = &entry["message"]["tauModelMessage"]["usage"];
                tokens = usage["total_tokens"].as_u64();
                break;
            }
        }
        let context_usage = settings.model(&model).ok().map(|model| ContextUsage { tokens, context_window:model.context_window });
        let needs_turn = journal.entries.iter().rev().find(|entry| entry["type"] == "message").is_some_and(|entry| {
            let message = &entry["message"];
            matches!(message["role"].as_str(), Some("user" | "toolResult"))
                || message["role"] == "assistant" && (matches!(message["stopReason"].as_str(), Some("error" | "aborted" | "toolUse"))
                    || message["content"].as_array().is_some_and(|parts| parts.iter().any(|part| part["type"] == "toolCall")))
        });
        if needs_turn { queue.paused = true; }
        let detail = (queue.paused && (needs_turn || !queue.requests.is_empty())).then(|| "Pending work is paused; resume when ready".to_owned());
        self.inner.state.set_session_file(id, journal.path.to_string_lossy().into_owned()).await?;
        self.inner.state.set_model(id, model.clone()).await?;
        let mut transcript = journal.transcript(queue)?;
        self.populate_attachment_sizes(transcript.events_mut()).await;
        content.transcript = Some(transcript);
        content.agent = Some(AgentSession { journal, model, thinking, receipts, running:false, cancel:tokio_util::sync::CancellationToken::new(), task:None, tokens, needs_turn });
        let _ = content.events.send(Arc::new(ServerMessage::ResyncRequired { session_id:Some(id.into()) }));
        self.set_runtime_state(id, runtime, SessionStatus::Idle, detail, Some(context_usage)); Ok(())
    }
    pub(crate) async fn entries_for_read(&self, id: &str) -> Result<(Vec<Value>, Option<String>)> {
        let runtime = self.runtime(id).await?; let mut content = runtime.content.lock().await;
        self.ensure_loaded(id, &runtime, &mut content).await?;
        let journal = &content.agent.as_ref().unwrap().journal; Ok((journal.entries.clone(), journal.head()))
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
                if runtime.snapshot().idle_since != idle_since || content.agent.as_ref().is_some_and(|a| a.running)
                    || content.transcript.as_ref().is_some_and(|t| t.queue.paused || !t.queue.requests.is_empty()) { return; }
                drop(content); manager.retire_session(&id, &runtime).await;
            });
        }
    }
    pub(crate) async fn broadcast_sessions(&self) { let _ = self.inner.events.send(self.sessions_message().await); }
    pub async fn set_settings(&self, revision: u64, settings: Settings) -> Result<Settings> { self.inner.settings.set(revision, settings).await }
}
pub(crate) fn safe_file_name(file_name: &str) -> String {
    let safe = file_name.rsplit(['/', '\\']).next().unwrap_or_default().chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' }).take(160).collect::<String>().trim_matches('.').to_owned();
    if safe.is_empty() { "attachment".into() } else { safe }
}
pub(crate) fn bounded(value: &str, max: usize) -> String { value.chars().take(max).collect() }
