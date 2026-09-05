use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock as StdRwLock};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::fs;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncSeekExt, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex, broadcast};
use tracing::{debug, warn};

use crate::config::Config;
use crate::pi::RpcProcess;
use crate::protocol::{
    AttachmentKind, ExtensionUiRequest, PromptDisposition, QueueOperation, ServerMessage,
    SessionStatus, SessionSummary, SlashCommand, SlashCommandArgument, SlashCommandSource,
    UploadedFile, MAX_PROMPT_CHARS, MAX_TITLE_CHARS, MAX_UPLOAD_BYTES,
};
use crate::transcript::{Entry, PiPosition, QueueState, Transcript, TranscriptChange, attachment_request, IMAGE_LIMIT, FILE_LIMIT};
use crate::state::{SessionModel, StateStore};

const EVENT_BUFFER: usize = 2048;
const IDLE_TIMEOUT: Duration = Duration::from_secs(60 * 60);
const INTERNAL_FORK_COMMAND: &str = "tau-fork-at";
const TUI_ONLY_COMMANDS: &[&str] = &[
    "settings",
    "tree",
    "scoped-models",
    "export",
    "import",
    "share",
    "copy",
    "session",
    "changelog",
    "hotkeys",
    "fork",
    "clone",
    "trust",
    "login",
    "logout",
    "new",
    "resume",
    "reload",
    "quit",
];

pub struct ResolvedAttachment {
    pub file: fs::File,
    pub file_name: String,
    pub mime_type: &'static str,
    pub size: u64,
}

pub struct PromptOutcome {
    pub disposition: PromptDisposition,
    pub notice: Option<String>,
}

struct PendingExtensionUi {
    process: Arc<RpcProcess>,
    request: ExtensionUiRequest,
}

#[derive(Clone)]
pub struct AgentManager {
    inner: Arc<ManagerInner>,
}

struct ManagerInner {
    config: Config,
    state: StateStore,
    runtimes: Mutex<HashMap<String, Arc<SessionRuntime>>>,
    pending_extension_ui: Mutex<HashMap<(String, String), PendingExtensionUi>>,
    events: broadcast::Sender<ServerMessage>,
    shutting_down: AtomicBool,
}

struct SessionRuntime {
    operation: Mutex<()>,
    content: Mutex<SessionContent>,
    commands: StdRwLock<Option<Vec<SlashCommand>>>,
    state: StdRwLock<RuntimeState>,
}

#[derive(Default)]
struct SessionContent {
    process: Option<Arc<RpcProcess>>,
    transcript: Option<Transcript>,
    recovering: bool,
}

#[derive(Clone, Default)]
struct RuntimeState {
    status: SessionStatus,
    detail: Option<String>,
    idle_since: Option<Instant>,
}

enum BranchOperation<'a> {
    Fork(&'a str),
    Clone,
}

impl SessionRuntime {
    fn new() -> Self {
        Self {
            operation: Mutex::new(()),
            content: Mutex::new(SessionContent::default()),
            commands: StdRwLock::new(None),
            state: StdRwLock::new(RuntimeState::default()),
        }
    }

    fn snapshot(&self) -> RuntimeState {
        self.state
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

impl AgentManager {
    pub fn new(config: Config, state: StateStore) -> Self {
        let (events, _) = broadcast::channel(EVENT_BUFFER);
        Self {
            inner: Arc::new(ManagerInner {
                config,
                state,
                runtimes: Mutex::new(HashMap::new()),
                pending_extension_ui: Mutex::new(HashMap::new()),
                events,
                shutting_down: AtomicBool::new(false),
            }),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ServerMessage> {
        self.inner.events.subscribe()
    }

    pub async fn sessions_message(&self) -> ServerMessage {
        let runtimes = self.inner.runtimes.lock().await;
        let sessions = self
            .inner
            .state
            .list()
            .into_iter()
            .map(|(id, stored)| {
                let runtime = runtimes
                    .get(&id)
                    .map(|runtime| runtime.snapshot())
                    .unwrap_or_default();
                SessionSummary {
                    id,
                    title: stored.title,
                    status: runtime.status,
                    detail: runtime.detail,
                    model: stored.model,
                    parent_id: stored.parent_id,
                    created_at_ms: stored.created_at_ms,
                    updated_at_ms: stored.updated_at_ms,
                }
            })
            .collect();
        ServerMessage::Sessions { sessions }
    }

    pub async fn create_session(&self) -> Result<String> {
        self.ensure_running()?;
        let (provider, model_id) = self
            .inner
            .config
            .default_model
            .split_once('/')
            .expect("validated Tau default model");
        let id = self
            .inner
            .state
            .create(
                "New chat".to_owned(),
                None,
                None,
                Some(SessionModel {
                    provider: provider.to_owned(),
                    model_id: model_id.to_owned(),
                }),
            )
            .await?;
        self.broadcast_sessions().await;
        Ok(id)
    }

    pub async fn open_session(&self, id: &str) -> Result<()> {
        let runtime = self.runtime(id).await?;
        let pending = self
            .inner
            .pending_extension_ui
            .lock()
            .await
            .iter()
            .filter(|((session_id, _), _)| session_id == id)
            .map(|(_, pending)| pending.request.clone())
            .collect::<Vec<_>>();
        for request in pending {
            let _ = self.inner.events.send(ServerMessage::ExtensionUi {
                session_id: id.to_owned(),
                request: Box::new(request),
            });
        }
        let _guard = runtime.operation.lock().await;
        let mut content = runtime.content.lock().await;
        if content.transcript.is_none() {
            let (entries, head) = self.entries_for_read(id, content.process.as_ref()).await?;
            let mut retained = entries.iter().map(|entry| Entry::from_pi(entry, false)).collect::<Result<Vec<_>>>()?;
            self.populate_attachment_sizes(&entries, &mut retained).await;
            content.transcript = Some(Transcript::new(retained, head, None, QueueState::default())?);
        }
        if content.recovering && let Some(process) = content.process.as_ref() {
            process.notify(json!({ "type": "get_transcript" })).await?;
        }
        let _ = self.inner.events.send(ServerMessage::TranscriptSnapshot {
            session_id: id.to_owned(),
            snapshot: content.transcript.as_ref().expect("transcript was loaded").snapshot(),
        });
        drop(content);
        let state = runtime.snapshot();
        let _ = self.inner.events.send(ServerMessage::SessionState {
            session_id: id.to_owned(),
            status: state.status,
            detail: state.detail,
        });
        Ok(())
    }

    pub async fn commands(&self, id: &str) -> Result<Vec<SlashCommand>> {
        let runtime = self.runtime(id).await?;
        let _guard = runtime.operation.lock().await;
        let process = self.ensure_process(id, &runtime).await?;
        self.load_slash_commands(&runtime, &process, true).await
    }

    pub async fn prompt(&self, id: &str, text: &str, request_id: &str) -> Result<PromptOutcome> {
        if text.trim().is_empty() {
            bail!("message cannot be empty");
        }
        let runtime = self.runtime(id).await?;
        let _guard = runtime.operation.lock().await;
        let process = self.ensure_process(id, &runtime).await?;

        let slash = if let Some(rest) = text.strip_prefix('/') {
            let name_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            let name = &rest[..name_end];
            (!name.is_empty()).then(|| (name, rest[name_end..].trim()))
        } else {
            None
        };
        if slash.is_some_and(|(name, _)| name == INTERNAL_FORK_COMMAND) {
            bail!("that command is reserved for Tau's fork operation");
        }

        let slash_command = if let Some((name, _)) = slash {
            let mut commands = self.load_slash_commands(&runtime, &process, false).await?;
            let found = commands.iter().find(|command| command.name == name).cloned();
            if found.is_none() {
                commands = self.load_slash_commands(&runtime, &process, true).await?;
            }
            commands.into_iter().find(|command| command.name == name)
        } else {
            None
        };

        if let Some((name, arguments)) = slash
            && slash_command.as_ref().is_some_and(|command| {
                command.source == SlashCommandSource::Builtin
            })
        {
            let previous_status = runtime.snapshot().status;
            self.set_runtime_state(id, &runtime, SessionStatus::Running, None);
            let mut accepted = false;
            let result: Result<String> = async {
                match name {
                    "compact" => {
                        let mut command = json!({ "type": "compact" });
                        if !arguments.is_empty() {
                            command.as_object_mut().expect("command is an object").insert(
                                "customInstructions".to_owned(),
                                Value::String(arguments.to_owned()),
                            );
                        }
                        process.request_unbounded(command).await?;
                        accepted = true;
                        self.inner.state.touch(id).await?;
                        Ok("Context compacted.".to_owned())
                    }
                    "model" => {
                        let (provider, model_id) = arguments.split_once('/').filter(
                            |(provider, model_id)| !provider.is_empty() && !model_id.is_empty(),
                        ).context("Usage: /model <provider/model>")?;
                        let response = process.request(json!({
                            "type": "set_model",
                            "provider": provider,
                            "modelId": model_id,
                        })).await?;
                        accepted = true;
                        let model = response
                            .get("data")
                            .and_then(session_model_from_pi_model)
                            .unwrap_or_else(|| SessionModel {
                                provider: provider.to_owned(),
                                model_id: model_id.to_owned(),
                            });
                        self.inner.state.set_model(id, model).await?;
                        self.inner.state.touch(id).await?;
                        Ok(format!("Model set to {provider}/{model_id}."))
                    }
                    "thinking" => {
                        if arguments.is_empty() || arguments.chars().any(char::is_whitespace) {
                            bail!("Usage: /thinking <level>");
                        }
                        process.request(json!({
                            "type": "set_thinking_level",
                            "level": arguments,
                        })).await?;
                        accepted = true;
                        self.inner.state.touch(id).await?;
                        Ok(format!("Thinking level set to {arguments}."))
                    }
                    "name" => {
                        let title = arguments.trim();
                        if title.is_empty() {
                            bail!("Usage: /name <title>");
                        }
                        if title.contains('\n') || title.contains('\r') {
                            bail!("session title must be one line");
                        }
                        if title.chars().count() > MAX_TITLE_CHARS {
                            bail!("session title is too long");
                        }
                        process.request(json!({
                            "type": "set_session_name",
                            "name": title,
                        })).await?;
                        accepted = true;
                        self.inner.state.rename(id, title.to_owned()).await?;
                        Ok(format!("Chat renamed to {title}."))
                    }
                    _ => bail!("unsupported Tau command /{name}"),
                }
            }
            .await;
            let notice = match result {
                Ok(notice) => notice,
                Err(error) if accepted => {
                    warn!(session = id, %error, "command accepted; session metadata refresh was delayed");
                    format!("/{name} accepted; metadata refresh is delayed.")
                }
                Err(error) => {
                    let status = if process.is_alive() {
                        if previous_status == SessionStatus::Running {
                            SessionStatus::Running
                        } else {
                            SessionStatus::Idle
                        }
                    } else {
                        SessionStatus::Error
                    };
                    self.set_runtime_state(
                        id,
                        &runtime,
                        status,
                        (status == SessionStatus::Error)
                            .then(|| bounded(&error.to_string(), 240)),
                    );
                    return Err(error);
                }
            };
            if let Err(error) = self.persist_session_file(id, &process).await {
                warn!(session = id, %error, "command accepted; session path refresh was delayed");
            }
            if let Err(error) = self.refresh_runtime_status(id, &runtime, &process).await {
                warn!(session = id, %error, "command accepted; status refresh was delayed");
            }
            self.broadcast_sessions().await;
            return Ok(PromptOutcome {
                disposition: PromptDisposition::Handled,
                notice: Some(notice),
            });
        }

        if let Some((name, _)) = slash
            && slash_command.is_none()
            && TUI_ONLY_COMMANDS.contains(&name)
        {
            bail!("Pi /{name} is available only in the interactive terminal");
        }

        let previous_status = runtime.snapshot().status;
        self.set_runtime_state(id, &runtime, SessionStatus::Running, None);
        let command = json!({
            "type": "prompt",
            "message": text,
            "requestId": request_id,
            "streamingBehavior": "steer"
        });
        let response = match process.request_unbounded(command).await {
            Ok(response) => response,
            Err(error) => {
            let status = if process.is_alive() {
                if previous_status == SessionStatus::Running {
                    SessionStatus::Running
                } else {
                    SessionStatus::Idle
                }
            } else {
                SessionStatus::Error
            };
            self.set_runtime_state(
                id,
                &runtime,
                status,
                (status == SessionStatus::Error).then(|| bounded(&error.to_string(), 240)),
            );
                return Err(error);
            }
        };
        let disposition: PromptDisposition = serde_json::from_value(
            response.pointer("/data/disposition").context("Pi prompt acceptance has no disposition")?.clone(),
        )?;
        let command_handled = matches!(disposition, PromptDisposition::Handled);

        let metadata: Result<()> = async {

        if command_handled {
            self.inner.state.touch(id).await?;
        } else if let Some(stored) = self.inner.state.get(id)
            && stored.title == "New chat"
        {
            let title = title_from_prompt(text);
            self.inner.state.rename(id, title.clone()).await?;
            if let Err(error) = process
                .request(json!({ "type": "set_session_name", "name": title }))
                .await
            {
                debug!(session = id, %error, "Pi did not accept the Tau session title");
            }
        } else {
            self.inner.state.touch(id).await?;
        }
        self.persist_session_file(id, &process).await?;
        if command_handled {
            self.refresh_runtime_status(id, &runtime, &process).await?;
        }
            Ok(())
        }.await;
        if let Err(error) = metadata {
            warn!(session = id, %error, "prompt accepted; session metadata refresh was delayed");
        }
        self.broadcast_sessions().await;
        Ok(PromptOutcome {
            disposition,
            notice: None,
        })
    }

    pub async fn extension_ui_response(
        &self,
        id: &str,
        request_id: &str,
        value: Option<String>,
        confirmed: Option<bool>,
        cancelled: bool,
    ) -> Result<()> {
        if request_id.is_empty() || request_id.len() > 128 {
            bail!("invalid extension dialog ID");
        }
        if value.as_deref().is_some_and(|value| value.chars().count() > MAX_PROMPT_CHARS) {
            bail!("extension dialog response is too large");
        }
        let key = (id.to_owned(), request_id.to_owned());
        let pending = self
            .inner
            .pending_extension_ui
            .lock()
            .await
            .remove(&key)
            .context("that extension dialog is no longer active")?;
        let runtime = self.runtime(id).await?;
        let owned = runtime.content.lock().await.process.as_ref().is_some_and(|current| Arc::ptr_eq(current, &pending.process));
        if !owned || !pending.process.is_alive() {
            bail!("the Pi process for that extension dialog is no longer active");
        }
        let invalid = if cancelled {
            None
        } else if pending.request.method == "confirm" && confirmed.is_none() {
            Some("confirmation response is missing")
        } else if pending.request.method != "confirm" && value.is_none() {
            Some("extension dialog response is missing")
        } else if pending.request.method == "select"
            && value.as_ref().is_some_and(|value| !pending.request.options.contains(value))
        {
            Some("extension selection is invalid")
        } else {
            None
        };
        if let Some(error) = invalid {
            self.inner.pending_extension_ui.lock().await.insert(key, pending);
            bail!(error);
        }
        let response = if cancelled {
            json!({ "type": "extension_ui_response", "id": request_id, "cancelled": true })
        } else if pending.request.method == "confirm" {
            json!({
                "type": "extension_ui_response",
                "id": request_id,
                "confirmed": confirmed.expect("confirmation was validated"),
            })
        } else {
            json!({
                "type": "extension_ui_response",
                "id": request_id,
                "value": value.expect("dialog value was validated"),
            })
        };
        pending.process.notify(response).await
    }

    pub async fn queue_control(&self, id: &str, generation: &str, command_id: &str, operation: QueueOperation) -> Result<String> {
        let runtime = self.runtime(id).await?;
        let _operation = runtime.operation.lock().await;
        let process = {
            let content = runtime.content.lock().await;
            let transcript = content.transcript.as_ref().context("Synchronize this chat before changing its queue")?;
            if content.recovering || transcript.generation != generation || !transcript.queue.available {
                bail!("Queue changed; synchronize this chat before trying again");
            }
            content.process.as_ref().filter(|process| process.is_alive()).context("Pi queue is no longer active")?.clone()
        };
        let command = match operation {
            QueueOperation::Edit { request_id, revision, text } => {
                if text.chars().count() > MAX_PROMPT_CHARS { bail!("message is too large"); }
                json!({"type":"edit_queued_message", "requestId":request_id, "revision":revision, "message":text})
            }
            QueueOperation::Delete { request_id, revision } => json!({"type":"delete_queued_message", "requestId":request_id, "revision":revision}),
            QueueOperation::Prefix { run_id, requests, boundary } => json!({"type":"run_queue_prefix", "controlId":command_id, "runId":run_id, "requests":requests, "boundary":boundary}),
            QueueOperation::Pause { run_id, boundary } => json!({"type":"pause_queue", "controlId":command_id, "runId":run_id, "boundary":boundary}),
            QueueOperation::Resume { run_id } => json!({"type":"resume_queue", "controlId":command_id, "runId":run_id}),
            QueueOperation::Cancel { control_id } => json!({"type":"cancel_queue_control", "controlId":control_id}),
        };
        let response = process.request(command).await?;
        Ok(response.pointer("/data/outcome").and_then(Value::as_str).unwrap_or("accepted").to_owned())
    }

    pub async fn abort(&self, id: &str) -> Result<()> {
        let runtime = self.runtime(id).await?;
        let process = runtime.content.lock().await.process.clone();
        if runtime.snapshot().status != SessionStatus::Running {
            return Ok(());
        }
        if let Some(process) = process.filter(|process| process.is_alive()) {
            self.cancel_extension_ui(&process).await;
            process.request(json!({ "type": "abort" })).await?;
        }
        Ok(())
    }

    pub async fn close_session(&self, id: &str) -> Result<()> {
        let runtime = self.runtime(id).await?;
        let process = runtime.content.lock().await.process.clone();
        if let Some(process) = process { self.cancel_extension_ui(&process).await; }
        let _guard = runtime.operation.lock().await;
        let process = runtime.content.lock().await.process.take();
        *runtime
            .commands
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        self.set_runtime_state(id, &runtime, SessionStatus::Sleeping, None);
        if let Some(process) = process {
            process.shutdown().await;
        }
        self.interrupt_transcript(id, &mut *runtime.content.lock().await);
        self.broadcast_sessions().await;
        Ok(())
    }

    async fn sleep_if_idle(&self, id: &str, expected_idle_since: Instant) {
        let runtime = self.inner.runtimes.lock().await.get(id).cloned();
        let Some(runtime) = runtime else {
            return;
        };
        let _guard = runtime.operation.lock().await;
        let state = runtime.snapshot();
        if state.status != SessionStatus::Idle
            || state.idle_since != Some(expected_idle_since)
        {
            return;
        }
        let process = {
            let mut content = runtime.content.lock().await;
            if content.transcript.as_ref().is_some_and(|transcript| transcript.queue.paused || !transcript.queue.requests.is_empty()) { return; }
            content.process.take()
        };
        *runtime
            .commands
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        self.set_runtime_state(id, &runtime, SessionStatus::Sleeping, None);
        if let Some(process) = process {
            process.shutdown().await;
        }
        self.interrupt_transcript(id, &mut *runtime.content.lock().await);
        debug!(session = id, "put idle Pi process to sleep");
        self.broadcast_sessions().await;
    }

    pub async fn delete_session(&self, id: &str) -> Result<()> {
        let runtime = self.runtime(id).await?;
        let process = runtime.content.lock().await.process.clone();
        if let Some(process) = process { self.cancel_extension_ui(&process).await; }
        let _guard = runtime.operation.lock().await;
        let process = runtime.content.lock().await.process.take();
        if let Some(process) = process {
            *runtime
                .commands
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
            self.set_runtime_state(id, &runtime, SessionStatus::Sleeping, None);
            process.shutdown().await;
        }

        let stored = self
            .inner
            .state
            .get(id)
            .context("session disappeared while deleting")?;
        let session_file = if let Some(path) = stored.session_file.as_deref() {
            let root = fs::canonicalize(&self.inner.config.session_dir)
                .await
                .context("Tau session directory is unavailable")?;
            match fs::canonicalize(path).await {
                Ok(path) if path.starts_with(&root) && path != root => Some(path),
                Ok(_) => bail!("Pi session file is outside Tau's session directory"),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(error).context("could not inspect the Pi session file"),
            }
        } else {
            None
        };

        self.inner.state.remove(id).await?;
        let mut runtimes = self.inner.runtimes.lock().await;
        if runtimes
            .get(id)
            .is_some_and(|current| Arc::ptr_eq(current, &runtime))
        {
            runtimes.remove(id);
        }
        drop(runtimes);

        let session_removal = if let Some(path) = session_file {
            fs::remove_file(&path)
                .await
                .with_context(|| format!("failed to delete {}", path.display()))
        } else {
            Ok(())
        };
        let upload_removal = match fs::remove_dir_all(self.inner.config.upload_root.join(id)).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error).context("failed to delete chat attachments"),
        };
        self.broadcast_sessions().await;
        session_removal.and(upload_removal)
    }

    pub async fn store_upload(
        &self,
        id: &str,
        file_name: &str,
        bytes: &[u8],
    ) -> Result<UploadedFile> {
        if bytes.is_empty() {
            bail!("attached file is empty");
        }
        if bytes.len() > MAX_UPLOAD_BYTES {
            bail!("attached file exceeds Tau's upload limit");
        }
        let runtime = self.runtime(id).await?;
        let _guard = runtime.operation.lock().await;
        if self.inner.state.get(id).is_none() {
            bail!("unknown session {id}");
        }

        let base_name = file_name
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or_default();
        let safe_name = base_name
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                    character
                } else {
                    '_'
                }
            })
            .take(160)
            .collect::<String>()
            .trim_matches('.')
            .to_owned();
        let safe_name = if safe_name.is_empty() {
            "attachment".to_owned()
        } else {
            safe_name
        };

        fs::create_dir_all(&self.inner.config.upload_root).await?;
        let root = fs::canonicalize(&self.inner.config.upload_root).await?;
        let directory = root.join(id);
        fs::create_dir_all(&directory).await?;
        let directory = fs::canonicalize(directory).await?;
        if !directory.starts_with(&root) || directory == root {
            bail!("unsafe Tau upload directory");
        }
        let path = directory.join(format!("{}-{safe_name}", uuid::Uuid::new_v4()));
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .await?;
        file.write_all(bytes).await?;
        file.flush().await?;
        file.sync_all().await?;
        Ok(UploadedFile {
            name: safe_name,
            path: path.to_string_lossy().into_owned(),
            size: bytes.len().try_into().unwrap_or(u64::MAX),
        })
    }

    pub async fn rename_session(&self, id: &str, title: &str) -> Result<()> {
        let title = title.trim();
        if title.is_empty() {
            bail!("title cannot be empty");
        }
        let title = bounded(title, MAX_TITLE_CHARS);
        let runtime = self.runtime(id).await?;
        let _guard = runtime.operation.lock().await;
        self.inner.state.rename(id, title.clone()).await?;
        let process = runtime.content.lock().await.process.clone();
        if let Some(process) = process.filter(|process| process.is_alive())
            && let Err(error) = process
                .request(json!({ "type": "set_session_name", "name": title }))
                .await
        {
            debug!(session = id, %error, "Pi did not accept the renamed session");
        }
        self.broadcast_sessions().await;
        Ok(())
    }

    pub async fn fork_session(
        &self,
        id: &str,
        entry_id: &str,
    ) -> Result<(String, Option<String>)> {
        self.branch_session(id, BranchOperation::Fork(entry_id)).await
    }

    pub async fn clone_session(&self, id: &str) -> Result<String> {
        self.branch_session(id, BranchOperation::Clone)
            .await
            .map(|(child, _)| child)
    }

    async fn populate_attachment_sizes(&self, entries: &[Value], messages: &mut [Entry]) {
        let Ok(root) = fs::canonicalize(&self.inner.config.attachment_root).await else {
            return;
        };
        let by_id = entries
            .iter()
            .filter_map(|entry| Some((entry.get("id")?.as_str()?, entry)))
            .collect::<HashMap<_, _>>();
        for message in messages {
            let Some(attachment) = message.attachment.as_mut() else {
                continue;
            };
            if attachment.size.is_some() {
                continue;
            }
            let Some(request) = by_id.get(message.id.as_str()).and_then(|entry| {
                attachment_request(entry)
            }) else {
                continue;
            };
            let Ok(path) = fs::canonicalize(&request.path).await else {
                continue;
            };
            if !path.starts_with(&root) {
                continue;
            }
            let Ok(metadata) = fs::metadata(path).await else {
                continue;
            };
            let limit = match request.kind {
                AttachmentKind::Image => IMAGE_LIMIT,
                AttachmentKind::File => FILE_LIMIT,
            };
            if metadata.is_file() && metadata.len() <= limit {
                attachment.size = Some(metadata.len());
            }
        }
    }

    pub async fn resolve_attachment(
        &self,
        id: &str,
        entry_id: &str,
    ) -> Result<ResolvedAttachment> {
        let runtime = self.runtime(id).await?;
        let _guard = runtime.operation.lock().await;
        let process = runtime.content.lock().await.process.clone();
        let (entries, _) = self.entries_for_read(id, process.as_ref()).await?;
        let entry = entries
            .iter()
            .find(|entry| entry.get("id").and_then(Value::as_str) == Some(entry_id))
            .with_context(|| format!("attachment entry {entry_id} does not exist"))?;
        let request = attachment_request(entry).context("entry has no Tau attachment")?;
        let root = fs::canonicalize(&self.inner.config.attachment_root)
            .await
            .context("Tau attachment root is unavailable")?;
        let path = fs::canonicalize(&request.path)
            .await
            .with_context(|| format!("attachment {} is unavailable", request.path.display()))?;
        if !path.starts_with(&root) {
            bail!("attachment is outside the Tau outbox");
        }
        let mut file = fs::File::open(&path).await?;
        let metadata = file.metadata().await?;
        if !metadata.is_file() {
            bail!("attachment is not a regular file");
        }
        let limit = match request.kind {
            AttachmentKind::Image => IMAGE_LIMIT,
            AttachmentKind::File => FILE_LIMIT,
        };
        if metadata.len() > limit {
            bail!("attachment exceeds the {} byte limit", limit);
        }
        let mime_type = match request.kind {
            AttachmentKind::File => "application/octet-stream",
            AttachmentKind::Image => {
                let mut header = [0_u8; 12];
                let length = file.read(&mut header).await?;
                file.seek(std::io::SeekFrom::Start(0)).await?;
                if length >= 8 && header[..8] == [137, 80, 78, 71, 13, 10, 26, 10] {
                    "image/png"
                } else if length >= 3 && header[..3] == [0xff, 0xd8, 0xff] {
                    "image/jpeg"
                } else if length >= 12
                    && &header[..4] == b"RIFF"
                    && &header[8..12] == b"WEBP"
                {
                    "image/webp"
                } else {
                    bail!("attachment is not a supported image");
                }
            }
        };
        let file_name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .filter(|name| !name.is_empty())
            .context("attachment has no file name")?;
        Ok(ResolvedAttachment {
            file,
            file_name,
            mime_type,
            size: metadata.len(),
        })
    }

    async fn branch_session(
        &self,
        id: &str,
        operation: BranchOperation<'_>,
    ) -> Result<(String, Option<String>)> {
        let runtime = self.runtime(id).await?;
        let _guard = runtime.operation.lock().await;
        let process = self.ensure_process(id, &runtime).await?;
        let state = process.request(json!({ "type": "get_state" })).await?;
        if state
            .get("data")
            .and_then(|data| data.get("isStreaming"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            bail!("wait for Pi to become idle before forking");
        }
        let fork_at_entry = if let BranchOperation::Fork(entry_id) = &operation {
            let response = process.request(json!({ "type": "get_entries" })).await?;
            let entries = response
                .get("data")
                .and_then(|data| data.get("entries"))
                .and_then(Value::as_array)
                .context("Pi entries response had no entries")?;
            let entry = entries
                .iter()
                .find(|entry| entry.get("id").and_then(Value::as_str) == Some(*entry_id))
                .with_context(|| format!("fork entry {entry_id} does not exist"))?;
            entry
                .get("message")
                .and_then(|message| message.get("role"))
                .and_then(Value::as_str)
                != Some("user")
        } else {
            false
        };
        if runtime.content.lock().await.transcript.as_ref().is_some_and(|transcript| transcript.queue.paused || !transcript.queue.requests.is_empty()) {
            bail!("Resume or clear the pending queue before forking");
        }
        self.persist_session_file(id, &process).await?;
        let parent = self
            .inner
            .state
            .get(id)
            .context("session disappeared while forking")?;
        let parent_file = parent
            .session_file
            .as_deref()
            .context("Pi session file is not available")?
            .to_owned();

        {
            let mut content = runtime.content.lock().await;
            content.process.take();
            self.interrupt_transcript(id, &mut content);
        }
        self.set_runtime_state(id, &runtime, SessionStatus::Sleeping, None);
        process.shutdown().await;

        let temporary = RpcProcess::spawn(&self.inner.config, Some(&parent_file))?;
        let result = async {
            let response = match operation {
                BranchOperation::Fork(entry_id) if fork_at_entry => {
                    temporary
                        .request(json!({
                            "type": "prompt",
                            "message": format!("/tau-fork-at {entry_id}"),
                            "streamingBehavior": "steer"
                        }))
                        .await?
                }
                BranchOperation::Fork(entry_id) => {
                    temporary
                        .request(json!({ "type": "fork", "entryId": entry_id }))
                        .await?
                }
                BranchOperation::Clone => temporary.request(json!({ "type": "clone" })).await?,
            };
            if response
                .get("data")
                .and_then(|data| data.get("cancelled"))
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                bail!("Pi cancelled the fork");
            }
            let draft = response
                .get("data")
                .and_then(|data| data.get("text"))
                .and_then(Value::as_str)
                .map(str::to_owned);
            let state = temporary.request(json!({ "type": "get_state" })).await?;
            let data = state
                .get("data")
                .context("forked Pi state response had no data")?;
            let child_file = data
                .get("sessionFile")
                .and_then(Value::as_str)
                .context("forked Pi session has no session file")?
                .to_owned();
            if child_file == parent_file {
                bail!("Pi did not create an independent session file");
            }
            let model = data
                .get("model")
                .and_then(session_model_from_pi_model)
                .or_else(|| parent.model.clone());
            Ok::<_, anyhow::Error>((child_file, draft, model))
        }
        .await;
        temporary.shutdown().await;
        let (child_file, draft, model) = result?;

        let prefix = match operation {
            BranchOperation::Fork(_) => "Fork of ",
            BranchOperation::Clone => "Copy of ",
        };
        let child = self
            .inner
            .state
            .create(
                bounded(&format!("{prefix}{}", parent.title), MAX_TITLE_CHARS),
                Some(id.to_owned()),
                Some(child_file),
                model,
            )
            .await?;
        self.broadcast_sessions().await;
        Ok((child, draft))
    }

    pub async fn shutdown(&self) {
        if self.inner.shutting_down.swap(true, Ordering::AcqRel) {
            return;
        }
        let runtimes = self
            .inner
            .runtimes
            .lock()
            .await
            .drain()
            .map(|(_, runtime)| runtime)
            .collect::<Vec<_>>();
        for runtime in runtimes {
            let process = runtime.content.lock().await.process.clone();
            if let Some(process) = process { self.cancel_extension_ui(&process).await; }
            let _guard = runtime.operation.lock().await;
            let process = runtime.content.lock().await.process.take();
            if let Some(process) = process { process.shutdown().await; }
        }
    }

    async fn cancel_extension_ui(&self, process: &Arc<RpcProcess>) {
        let pending_ids = {
            let mut pending = self.inner.pending_extension_ui.lock().await;
            let keys = pending
                .iter()
                .filter(|(_, request)| Arc::ptr_eq(&request.process, process))
                .map(|(key, _)| key.clone())
                .collect::<Vec<_>>();
            for key in &keys {
                pending.remove(key);
            }
            keys.into_iter().map(|(_, request_id)| request_id).collect::<Vec<_>>()
        };
        for request_id in pending_ids {
            if let Err(error) = process
                .notify(json!({
                    "type": "extension_ui_response",
                    "id": request_id,
                    "cancelled": true,
                }))
                .await
            {
                debug!(%error, "could not cancel a Pi extension dialog");
            }
        }
    }

    async fn runtime(&self, id: &str) -> Result<Arc<SessionRuntime>> {
        self.ensure_running()?;
        if self.inner.state.get(id).is_none() {
            bail!("unknown session {id}");
        }
        let mut runtimes = self.inner.runtimes.lock().await;
        self.ensure_running()?;
        Ok(runtimes
            .entry(id.to_owned())
            .or_insert_with(|| Arc::new(SessionRuntime::new()))
            .clone())
    }

    fn ensure_running(&self) -> Result<()> {
        if self.inner.shutting_down.load(Ordering::Acquire) {
            bail!("Tau is shutting down");
        }
        Ok(())
    }

    async fn entries_for_read(
        &self,
        id: &str,
        process: Option<&Arc<RpcProcess>>,
    ) -> Result<(Vec<Value>, Option<String>)> {
        if let Some(process) = process.filter(|process| process.is_alive()) {
            let response = process.request(json!({ "type": "get_entries" })).await?;
            let data = response
                .get("data")
                .context("Pi entries response had no data")?;
            let entries = data
                .get("entries")
                .and_then(Value::as_array)
                .context("Pi entries response had no entries")?
                .clone();
            let leaf_id = data
                .get("leafId")
                .and_then(Value::as_str)
                .map(str::to_owned);
            return Ok((entries, leaf_id));
        }

        let stored = self.inner.state.get(id).context("session disappeared")?;
        let Some(session_file) = stored.session_file else {
            return Ok((Vec::new(), None));
        };
        let root = fs::canonicalize(&self.inner.config.session_dir)
            .await
            .context("Tau session directory is unavailable")?;
        let path = fs::canonicalize(&session_file)
            .await
            .with_context(|| format!("Pi session history is unavailable: {session_file}"))?;
        if !path.starts_with(&root) || path == root {
            bail!("Pi session file is outside Tau's session directory");
        }
        let file = fs::File::open(&path).await?;
        if !file.metadata().await?.is_file() {
            bail!("Pi session history is not a regular file");
        }

        let mut lines = BufReader::new(file).lines();
        let mut entries = Vec::new();
        let mut leaf_id = None;
        let mut line_number = 0_usize;
        while let Some(line) = lines.next_line().await? {
            line_number += 1;
            if line.trim().is_empty() {
                continue;
            }
            let entry = serde_json::from_str::<Value>(&line).with_context(|| {
                format!("failed to parse {} at line {line_number}", path.display())
            })?;
            if entry.get("type").and_then(Value::as_str) == Some("session") {
                continue;
            }
            if let Some(entry_id) = entry.get("id").and_then(Value::as_str) {
                leaf_id = Some(entry_id.to_owned());
            }
            entries.push(entry);
        }
        Ok((entries, leaf_id))
    }

    async fn ensure_process(
        &self,
        id: &str,
        runtime: &Arc<SessionRuntime>,
    ) -> Result<Arc<RpcProcess>> {
        self.ensure_running()?;
        let mut slot = runtime.content.lock().await;
        if let Some(process) = slot.process.as_ref().filter(|process| process.is_alive()) {
            return Ok(process.clone());
        }
        if let Some(process) = slot.process.take() {
            self.interrupt_transcript(id, &mut slot);
            drop(slot);
            process.shutdown().await;
            slot = runtime.content.lock().await;
        }
        *runtime
            .commands
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;

        let stored = self.inner.state.get(id).context("session disappeared")?;
        let session = stored.session_file.as_deref();
        if let Some(path) = session {
            let root = fs::canonicalize(&self.inner.config.session_dir).await?;
            let path = fs::canonicalize(path).await.context("stored Pi session file is unavailable")?;
            if !path.starts_with(&root) || path == root || !fs::metadata(&path).await?.is_file() {
                bail!("stored Pi session file is outside Tau's session directory or is not a file");
            }
        }
        self.set_runtime_state(id, runtime, SessionStatus::Starting, None);
        let process = match RpcProcess::spawn(&self.inner.config, session) {
            Ok(process) => process,
            Err(error) => {
                self.set_runtime_state(
                    id,
                    runtime,
                    SessionStatus::Error,
                    Some(bounded(&error.to_string(), 240)),
                );
                return Err(error);
            }
        };
        let events = process.subscribe();
        let started: Result<bool> = async {
            let state = process.request(json!({ "type": "get_state" })).await?;
            let data = state.get("data").context("Pi state response had no data")?;
            if let Some(model) = data.get("model").and_then(session_model_from_pi_model) {
                self.inner.state.set_model(id, model).await?;
            }
            let session_file = data.get("sessionFile").and_then(Value::as_str)
                .context("Pi did not create a persistent session")?;
            self.inner.state.set_session_file(id, session_file.to_owned()).await?;
            let streaming = data.get("isStreaming").and_then(Value::as_bool).unwrap_or(false);
            let snapshot = process.request(json!({ "type": "get_transcript" })).await
                .context("Pi does not provide the identified transcript protocol")?;
            self.replace_transcript(id, &mut slot, snapshot.get("data").context("Pi transcript has no data")?).await?;
            Ok(streaming)
        }.await;
        let streaming = match started {
            Ok(streaming) => streaming,
            Err(error) => {
                self.interrupt_transcript(id, &mut slot);
                drop(slot);
                process.shutdown().await;
                self.set_runtime_state(id, runtime, SessionStatus::Error, Some(bounded(&error.to_string(), 240)));
                return Err(error);
            }
        };
        slot.process = Some(process.clone());
        self.set_runtime_state(
            id,
            runtime,
            if streaming {
                SessionStatus::Running
            } else {
                SessionStatus::Idle
            },
            None,
        );

        let manager = self.clone();
        let session_id = id.to_owned();
        let monitored = process.clone();
        tokio::spawn(async move {
            manager.monitor_process(session_id, monitored, events).await;
        });
        Ok(process)
    }

    async fn load_slash_commands(
        &self,
        runtime: &SessionRuntime,
        process: &RpcProcess,
        refresh: bool,
    ) -> Result<Vec<SlashCommand>> {
        if !refresh
            && let Some(commands) = runtime
                .commands
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone()
        {
            return Ok(commands);
        }

        let response = process
            .request(json!({ "type": "get_commands" }))
            .await
            .context("Pi could not list slash commands")?;
        let records = response
            .get("data")
            .and_then(|data| data.get("commands"))
            .and_then(Value::as_array)
            .context("Pi command response had no commands")?;
        let mut commands = records
            .iter()
            .filter_map(|record| {
                let name = record.get("name")?.as_str()?.trim();
                if name.is_empty()
                    || name == INTERNAL_FORK_COMMAND
                    || name.chars().count() > 128
                    || name.chars().any(char::is_whitespace)
                {
                    return None;
                }
                let source = match record.get("source")?.as_str()? {
                    "extension" => SlashCommandSource::Extension,
                    "prompt" => SlashCommandSource::Prompt,
                    "skill" => SlashCommandSource::Skill,
                    _ => return None,
                };
                Some(SlashCommand {
                    name: name.to_owned(),
                    description: record
                        .get("description")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|description| !description.is_empty())
                        .map(|description| bounded(description, 240)),
                    source,
                    argument_hint: None,
                    arguments: Vec::new(),
                })
            })
            .collect::<Vec<_>>();
        let names = commands
            .iter()
            .map(|command| command.name.as_str())
            .collect::<HashSet<_>>();

        let model_arguments = if names.contains("model") {
            Vec::new()
        } else {
            match process
                .request(json!({ "type": "get_available_models" }))
                .await
            {
                Ok(response) => {
                    let mut arguments = response
                        .get("data")
                        .and_then(|data| data.get("models"))
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(|model| {
                            let provider = model.get("provider")?.as_str()?;
                            let model_id = model.get("id")?.as_str()?;
                            if provider.is_empty() || model_id.is_empty() {
                                return None;
                            }
                            Some(SlashCommandArgument {
                                value: bounded(&format!("{provider}/{model_id}"), 240),
                                description: model
                                    .get("name")
                                    .and_then(Value::as_str)
                                    .filter(|name| !name.is_empty())
                                    .map(|name| bounded(name, 160)),
                            })
                        })
                        .collect::<Vec<_>>();
                    arguments.sort_by(|left, right| left.value.cmp(&right.value));
                    arguments.dedup_by(|left, right| left.value == right.value);
                    arguments
                }
                Err(error) => {
                    debug!(%error, "Pi model completion is unavailable");
                    Vec::new()
                }
            }
        };
        let thinking_arguments = if names.contains("thinking") {
            Vec::new()
        } else {
            match process
                .request(json!({ "type": "get_available_thinking_levels" }))
                .await
            {
                Ok(response) => response
                    .get("data")
                    .and_then(|data| data.get("levels"))
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .filter(|level| !level.is_empty() && !level.chars().any(char::is_whitespace))
                    .map(|level| SlashCommandArgument {
                        value: level.to_owned(),
                        description: None,
                    })
                    .collect(),
                Err(error) => {
                    debug!(%error, "Pi thinking-level completion is unavailable");
                    Vec::new()
                }
            }
        };
        drop(names);

        for command in [
            SlashCommand {
                name: "compact".to_owned(),
                description: Some("Manually compact the session context".to_owned()),
                source: SlashCommandSource::Builtin,
                argument_hint: Some("[instructions]".to_owned()),
                arguments: Vec::new(),
            },
            SlashCommand {
                name: "model".to_owned(),
                description: Some("Select the Pi model".to_owned()),
                source: SlashCommandSource::Builtin,
                argument_hint: Some("<provider/model>".to_owned()),
                arguments: model_arguments,
            },
            SlashCommand {
                name: "thinking".to_owned(),
                description: Some("Set the Pi thinking level".to_owned()),
                source: SlashCommandSource::Builtin,
                argument_hint: Some("<level>".to_owned()),
                arguments: thinking_arguments,
            },
            SlashCommand {
                name: "name".to_owned(),
                description: Some("Set the Tau and Pi session name".to_owned()),
                source: SlashCommandSource::Builtin,
                argument_hint: Some("<title>".to_owned()),
                arguments: Vec::new(),
            },
        ] {
            if commands.iter().all(|existing| existing.name != command.name) {
                commands.push(command);
            }
        }
        commands.sort_by(|left, right| left.name.cmp(&right.name));
        *runtime
            .commands
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(commands.clone());
        Ok(commands)
    }

    async fn monitor_process(
        &self,
        id: String,
        process: Arc<RpcProcess>,
        mut events: broadcast::Receiver<Value>,
    ) {
        loop {
            let event = match events.recv().await {
                Ok(event) => event,
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    warn!(session = %id, skipped, "Pi event gap; requesting a transcript snapshot");
                    if let Ok(runtime) = self.runtime(&id).await {
                        let mut content = runtime.content.lock().await;
                        if content.process.as_ref().is_none_or(|current| !Arc::ptr_eq(current, &process)) { break; }
                        content.recovering = true;
                        if let Err(error) = process.notify(json!({ "type": "get_transcript" })).await {
                            warn!(session = %id, %error, "Pi transcript recovery request failed");
                        }
                    }
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => break,
            };
            let Ok(runtime) = self.runtime(&id).await else {
                break;
            };
            let mut content = runtime.content.lock().await;
            if content.process.as_ref().is_none_or(|current| !Arc::ptr_eq(current, &process)) { break; }

            match event.get("type").and_then(Value::as_str) {
                Some("extension_ui_request") => {
                    match serde_json::from_value::<ExtensionUiRequest>(event.clone()) {
                        Ok(request) if !request.id.is_empty() && request.id.len() <= 128 => {
                            if matches!(
                                request.method.as_str(),
                                "select" | "confirm" | "input" | "editor"
                            ) {
                                let key = (id.clone(), request.id.clone());
                                self.inner.pending_extension_ui.lock().await.insert(
                                    key.clone(),
                                    PendingExtensionUi {
                                        process: process.clone(),
                                        request: request.clone(),
                                    },
                                );
                                if let Some(timeout_ms) = request.timeout {
                                    let inner = self.inner.clone();
                                    let timed_process = process.clone();
                                    tokio::spawn(async move {
                                        tokio::time::sleep(Duration::from_millis(
                                            timeout_ms.saturating_add(1_000),
                                        )).await;
                                        let mut pending = inner.pending_extension_ui.lock().await;
                                        if pending.get(&key).is_some_and(|pending| {
                                            Arc::ptr_eq(&pending.process, &timed_process)
                                        }) {
                                            pending.remove(&key);
                                        }
                                    });
                                }
                            }
                            let _ = self.inner.events.send(ServerMessage::ExtensionUi {
                                session_id: id.clone(),
                                request: Box::new(request),
                            });
                        }
                        Ok(_) => warn!(session = %id, "Pi sent an invalid extension UI request ID"),
                        Err(error) => {
                            warn!(session = %id, %error, "Pi sent an invalid extension UI request")
                        }
                    }
                }
                Some("extension_error") => {
                    let message = event
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("Pi extension failed");
                    let _ = self.inner.events.send(ServerMessage::ExtensionError {
                        session_id: id.clone(),
                        error: bounded(message, 480),
                    });
                }
                Some("agent_start") => {
                    self.set_runtime_state(&id, &runtime, SessionStatus::Running, None);
                }
                Some("transcript_update") => {
                    if content.recovering { continue; }
                    let Some(transcript) = content.transcript.as_mut() else { continue; };
                    let result: Result<Option<TranscriptChange>> = async {
                        let position = PiPosition::from_pi(&event)?;
                        if !transcript.check_position(&position)? { return Ok(None); }
                        let raw = event.get("change").context("Pi update has no change")?;
                        let mut change = TranscriptChange::from_pi(raw)?;
                        if let TranscriptChange::Entry { entry } = &mut change {
                            self.populate_attachment_sizes(
                                std::slice::from_ref(raw.get("entry").context("Pi change has no entry")?),
                                std::slice::from_mut(entry.as_mut()),
                            ).await;
                        }
                        transcript.apply(&change, Some(position))?;
                        Ok(Some(change))
                    }.await;
                    match result {
                        Ok(Some(change)) => {
                            let _ = self.inner.events.send(ServerMessage::TranscriptUpdate {
                                session_id: id.clone(), generation: transcript.generation.clone(),
                                sequence: transcript.sequence, change,
                            });
                        }
                        Ok(None) => {}
                        Err(error) => {
                            warn!(session = %id, %error, "Pi transcript needs a snapshot");
                            content.recovering = true;
                            if let Err(error) = process.notify(json!({ "type": "get_transcript" })).await {
                                warn!(session = %id, %error, "Pi transcript recovery request failed");
                            }
                        }
                    }
                }
                Some("transcript_snapshot") | Some("response")
                    if event.get("type").and_then(Value::as_str) == Some("transcript_snapshot")
                        || event.get("command").and_then(Value::as_str) == Some("get_transcript") =>
                {
                    let data = event.get("snapshot").or_else(|| event.get("data"));
                    let result = match data {
                        Some(data) => self.replace_transcript(&id, &mut content, data).await,
                        None => Err(anyhow::anyhow!("Pi transcript snapshot has no data")),
                    };
                    content.recovering = result.is_err();
                    if let Err(error) = result {
                        warn!(session = %id, %error, "Pi transcript snapshot was rejected");
                        self.set_runtime_state(&id, &runtime, SessionStatus::Error, Some("Transcript synchronization failed".to_owned()));
                    }
                }
                Some("compaction_start") => {
                    self.set_runtime_state(
                        &id,
                        &runtime,
                        SessionStatus::Running,
                        Some("Compacting context".to_owned()),
                    );
                }
                Some("auto_retry_start") => {
                    self.set_runtime_state(
                        &id,
                        &runtime,
                        SessionStatus::Running,
                        Some("Retrying".to_owned()),
                    );
                }
                Some("agent_settled") => {
                    if let Err(error) = self.refresh_runtime_status(&id, &runtime, &process).await {
                        warn!(session = %id, %error, "failed to refresh settled Pi state");
                    }
                    if let Err(error) = self.persist_session_file(&id, &process).await {
                        warn!(session = %id, %error, "failed to persist settled Pi session path");
                    }
                    self.broadcast_sessions().await;
                }
                Some("rpc_closed") => {
                    self.interrupt_transcript(&id, &mut content);
                    content.process.take();
                    *runtime
                        .commands
                        .write()
                        .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
                    self.inner
                        .pending_extension_ui
                        .lock()
                        .await
                        .retain(|_, pending| !Arc::ptr_eq(&pending.process, &process));
                    let detail = event
                        .get("error")
                        .and_then(Value::as_str)
                        .map(|error| bounded(error, 240));
                    self.set_runtime_state(&id, &runtime, SessionStatus::Error, detail);
                    self.broadcast_sessions().await;
                    drop(content);
                    process.shutdown().await;
                    break;
                }
                _ => {}
            }
        }
    }

    async fn refresh_runtime_status(
        &self,
        id: &str,
        runtime: &SessionRuntime,
        process: &RpcProcess,
    ) -> Result<()> {
        let state = process.request(json!({ "type": "get_state" })).await?;
        let data = state.get("data").context("Pi state response had no data")?;
        let running = data
            .get("isStreaming")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            || data
                .get("isCompacting")
                .and_then(Value::as_bool)
                .unwrap_or(false);
        self.set_runtime_state(
            id,
            runtime,
            if running {
                SessionStatus::Running
            } else {
                SessionStatus::Idle
            },
            None,
        );
        Ok(())
    }

    async fn persist_session_file(&self, id: &str, process: &RpcProcess) -> Result<()> {
        let state = process.request(json!({ "type": "get_state" })).await?;
        let session_file = state
            .get("data")
            .and_then(|data| data.get("sessionFile"))
            .and_then(Value::as_str)
            .context("Pi state had no session file")?;
        self.inner
            .state
            .set_session_file(id, session_file.to_owned())
            .await
    }

    fn interrupt_transcript(&self, id: &str, content: &mut SessionContent) {
        if let Some(transcript) = content.transcript.as_mut() {
            let change = TranscriptChange::Interrupted;
            transcript.apply(&change, None).expect("interrupting entries is infallible");
            transcript.source = None;
            let _ = self.inner.events.send(ServerMessage::TranscriptUpdate {
                session_id: id.to_owned(), generation: transcript.generation.clone(),
                sequence: transcript.sequence, change,
            });
        }
    }

    async fn replace_transcript(&self, id: &str, content: &mut SessionContent, data: &Value) -> Result<()> {
        let raw = data.get("entries").and_then(Value::as_array).context("Pi transcript has no entries")?;
        let mut entries = raw.iter().map(|entry| Entry::from_pi(entry, false)).collect::<Result<Vec<_>>>()?;
        self.populate_attachment_sizes(raw, &mut entries).await;
        for live in data.get("live").and_then(Value::as_array).context("Pi transcript has no live entries")? {
            entries.push(Entry::from_pi(live, true)?);
        }
        let source = PiPosition::from_pi(data)?;
        if let Some(previous) = content.transcript.as_ref().and_then(|transcript| transcript.source.as_ref())
            && source.generation == previous.generation
        {
            if source.session_id != previous.session_id { bail!("Pi snapshot changed session within one generation"); }
            if source.sequence < previous.sequence { bail!("Pi snapshot precedes retained transcript state"); }
        }
        let mut next = Transcript::new(
            entries,
            data.get("leafId").and_then(Value::as_str).map(str::to_owned),
            Some(source),
            QueueState::from_pi(data)?,
        )?;
        if let Some(previous) = content.transcript.as_ref() { next.retain_interrupted(previous); }
        let message = ServerMessage::TranscriptSnapshot { session_id: id.to_owned(), snapshot: next.snapshot() };
        content.transcript = Some(next);
        content.recovering = false;
        let _ = self.inner.events.send(message);
        Ok(())
    }

    fn set_runtime_state(
        &self,
        id: &str,
        runtime: &SessionRuntime,
        status: SessionStatus,
        detail: Option<String>,
    ) {
        let mut current = runtime
            .state
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if current.status == status && current.detail == detail {
            return;
        }
        let schedule_sleep = status == SessionStatus::Idle && current.status != SessionStatus::Idle;
        let idle_since = if status == SessionStatus::Idle {
            current.idle_since.or_else(|| Some(Instant::now()))
        } else {
            None
        };
        let next = RuntimeState {
            status,
            detail,
            idle_since,
        };
        *current = next.clone();
        drop(current);
        let _ = self.inner.events.send(ServerMessage::SessionState {
            session_id: id.to_owned(),
            status: next.status,
            detail: next.detail,
        });

        if schedule_sleep {
            let manager = self.clone();
            let session_id = id.to_owned();
            let expected_idle_since = next.idle_since.expect("idle sessions have an idle time");
            tokio::spawn(async move {
                tokio::time::sleep(IDLE_TIMEOUT).await;
                manager
                    .sleep_if_idle(&session_id, expected_idle_since)
                    .await;
            });
        }
    }

    async fn broadcast_sessions(&self) {
        let message = self.sessions_message().await;
        let _ = self.inner.events.send(message);
    }
}

fn session_model_from_pi_model(model: &Value) -> Option<SessionModel> {
    let provider = model.get("provider")?.as_str()?.trim();
    let model_id = model.get("id")?.as_str()?.trim();
    if provider.is_empty() || model_id.is_empty() {
        return None;
    }
    Some(SessionModel {
        provider: bounded(provider, 120),
        model_id: bounded(model_id, 240),
    })
}

fn title_from_prompt(prompt: &str) -> String {
    let first_line = prompt.lines().find(|line| !line.trim().is_empty()).unwrap_or("New chat");
    bounded(first_line.trim(), MAX_TITLE_CHARS)
}

fn bounded(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

#[cfg(test)]
#[path = "manager_test.rs"]
mod tests;
