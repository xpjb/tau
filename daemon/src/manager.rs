use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock as StdRwLock};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::fs;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncSeekExt, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex, broadcast};
use tracing::{debug, error, warn};

use crate::config::Config;
use crate::pi::RpcProcess;
use crate::protocol::{
    AttachmentKind, ChatAttachment, ChatMessage, ChatRole, ExtensionUiRequest, ServerMessage,
    SessionStatus, SessionSummary, SlashCommand, SlashCommandArgument, SlashCommandSource,
    UploadedFile, MAX_PROMPT_CHARS, MAX_TITLE_CHARS, MAX_UPLOAD_BYTES,
};
use crate::state::{SessionModel, StateStore};

const EVENT_BUFFER: usize = 2048;
const IDLE_TIMEOUT: Duration = Duration::from_secs(60 * 60);
const IMAGE_LIMIT: u64 = 10_000_000;
const FILE_LIMIT: u64 = 50_000_000;
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

struct AttachmentRequest {
    kind: AttachmentKind,
    path: PathBuf,
    caption: Option<String>,
    size: Option<u64>,
}

pub struct PromptOutcome {
    pub command_handled: bool,
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
    process: Mutex<Option<Arc<RpcProcess>>>,
    commands: StdRwLock<Option<Vec<SlashCommand>>>,
    state: StdRwLock<RuntimeState>,
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
            process: Mutex::new(None),
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
        let (entries, leaf_id) = self.entries_for_read(id, &runtime).await?;
        let mut messages = active_chat_messages(&entries, leaf_id.as_deref())?;
        self.populate_attachment_sizes(&entries, &mut messages).await;
        let _ = self.inner.events.send(ServerMessage::History {
            session_id: id.to_owned(),
            messages,
        });
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

    pub async fn prompt(&self, id: &str, text: &str) -> Result<PromptOutcome> {
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
                        self.inner.state.rename(id, title.to_owned()).await?;
                        Ok(format!("Chat renamed to {title}."))
                    }
                    _ => bail!("unsupported Tau command /{name}"),
                }
            }
            .await;
            let notice = match result {
                Ok(notice) => notice,
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
            self.persist_session_file(id, &process).await?;
            if let Err(error) = self.sync_history(id, &process).await {
                debug!(session = id, %error, "history refresh after command was delayed");
            }
            self.refresh_runtime_status(id, &runtime, &process).await?;
            self.broadcast_sessions().await;
            return Ok(PromptOutcome {
                command_handled: true,
                notice: Some(notice),
            });
        }

        if let Some((name, _)) = slash
            && slash_command.is_none()
            && TUI_ONLY_COMMANDS.contains(&name)
        {
            bail!("Pi /{name} is available only in the interactive terminal");
        }

        let command_handled = slash_command.as_ref().is_some_and(|command| {
            if command.source != SlashCommandSource::Extension {
                return false;
            }
            let rest = text.strip_prefix('/').expect("slash command starts with slash");
            let end = rest.find(' ').unwrap_or(rest.len());
            rest[..end] == command.name
        });
        let previous_status = runtime.snapshot().status;
        self.set_runtime_state(id, &runtime, SessionStatus::Running, None);
        let command = json!({
            "type": "prompt",
            "message": text,
            "streamingBehavior": "steer"
        });
        if let Err(error) = process.request_unbounded(command).await {
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
        if let Err(error) = self.sync_history(id, &process).await {
            debug!(session = id, %error, "history refresh after prompt acceptance was delayed");
        }
        if command_handled {
            self.refresh_runtime_status(id, &runtime, &process).await?;
        }
        self.broadcast_sessions().await;
        Ok(PromptOutcome {
            command_handled,
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
        if !self.runtime_owns(&runtime, &pending.process).await || !pending.process.is_alive() {
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

    pub async fn abort(&self, id: &str) -> Result<()> {
        let runtime = self.runtime(id).await?;
        let process = runtime.process.lock().await.clone();
        if runtime.snapshot().status != SessionStatus::Running {
            return Ok(());
        }
        if let Some(process) = process.filter(|process| process.is_alive()) {
            self.cancel_extension_ui(&process).await;
            process.notify(json!({ "type": "abort" })).await?;
        }
        Ok(())
    }

    pub async fn close_session(&self, id: &str) -> Result<()> {
        let runtime = self.runtime(id).await?;
        if let Some(process) = runtime.process.lock().await.clone() {
            self.cancel_extension_ui(&process).await;
        }
        let _guard = runtime.operation.lock().await;
        let process = runtime.process.lock().await.take();
        *runtime
            .commands
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        self.set_runtime_state(id, &runtime, SessionStatus::Sleeping, None);
        if let Some(process) = process {
            process.shutdown().await;
        }
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
        let process = runtime.process.lock().await.take();
        *runtime
            .commands
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        self.set_runtime_state(id, &runtime, SessionStatus::Sleeping, None);
        if let Some(process) = process {
            process.shutdown().await;
        }
        debug!(session = id, "put idle Pi process to sleep");
        self.broadcast_sessions().await;
    }

    pub async fn delete_session(&self, id: &str) -> Result<()> {
        let runtime = self.runtime(id).await?;
        if let Some(process) = runtime.process.lock().await.clone() {
            self.cancel_extension_ui(&process).await;
        }
        let _guard = runtime.operation.lock().await;
        if let Some(process) = runtime.process.lock().await.take() {
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
        if let Some(process) = runtime
            .process
            .lock()
            .await
            .as_ref()
            .filter(|process| process.is_alive())
            .cloned()
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

    async fn populate_attachment_sizes(&self, entries: &[Value], messages: &mut [ChatMessage]) {
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
            let Some(request) = by_id.get(message.entry_id.as_str()).and_then(|entry| {
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
        let (entries, _) = self.entries_for_read(id, &runtime).await?;
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

        runtime.process.lock().await.take();
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
            if let Some(process) = runtime.process.lock().await.clone() {
                self.cancel_extension_ui(&process).await;
            }
            let _guard = runtime.operation.lock().await;
            if let Some(process) = runtime.process.lock().await.take() {
                process.shutdown().await;
            }
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
        runtime: &SessionRuntime,
    ) -> Result<(Vec<Value>, Option<String>)> {
        if let Some(process) = runtime
            .process
            .lock()
            .await
            .as_ref()
            .filter(|process| process.is_alive())
            .cloned()
        {
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
        let mut slot = runtime.process.lock().await;
        if let Some(process) = slot.as_ref().filter(|process| process.is_alive()) {
            return Ok(process.clone());
        }
        if let Some(process) = slot.take() {
            process.shutdown().await;
        }
        *runtime
            .commands
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;

        self.set_runtime_state(id, runtime, SessionStatus::Starting, None);
        let stored = self.inner.state.get(id).context("session disappeared")?;
        let session = stored
            .session_file
            .as_deref()
            .filter(|path| Path::new(path).is_file());
        if stored.session_file.is_some() && session.is_none() {
            warn!(session = id, "stored Pi session file is missing; starting fresh");
        }
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
        let state = match process.request(json!({ "type": "get_state" })).await {
            Ok(state) => state,
            Err(error) => {
                process.shutdown().await;
                self.set_runtime_state(
                    id,
                    runtime,
                    SessionStatus::Error,
                    Some(bounded(&error.to_string(), 240)),
                );
                return Err(error);
            }
        };
        let data = state.get("data").context("Pi state response had no data")?;
        if let Some(model) = data.get("model").and_then(session_model_from_pi_model) {
            self.inner.state.set_model(id, model).await?;
        }
        let session_file = data
            .get("sessionFile")
            .and_then(Value::as_str)
            .context("Pi did not create a persistent session")?;
        self.inner
            .state
            .set_session_file(id, session_file.to_owned())
            .await?;
        let streaming = data
            .get("isStreaming")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        *slot = Some(process.clone());
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
                    error!(session = %id, skipped, "Pi event stream lagged; closing the process");
                    if let Ok(runtime) = self.runtime(&id).await
                        && self.runtime_owns(&runtime, &process).await
                    {
                        runtime.process.lock().await.take();
                        *runtime
                            .commands
                            .write()
                            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
                        self.set_runtime_state(
                            &id,
                            &runtime,
                            SessionStatus::Error,
                            Some(format!("Pi event stream skipped {skipped} events")),
                        );
                        self.cancel_extension_ui(&process).await;
                        process.shutdown().await;
                    }
                    break;
                }
                Err(broadcast::error::RecvError::Closed) => break,
            };
            let Ok(runtime) = self.runtime(&id).await else {
                break;
            };
            if !self.runtime_owns(&runtime, &process).await {
                break;
            }

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
                Some("message_start") => {
                    if event
                        .get("message")
                        .and_then(|message| message.get("role"))
                        .and_then(Value::as_str)
                        == Some("assistant")
                    {
                        if let Err(error) = self.sync_history(&id, &process).await {
                            debug!(session = %id, %error, "history was not ready at assistant start");
                        }
                        let _ = self.inner.events.send(ServerMessage::StreamReset {
                            session_id: id.clone(),
                        });
                    }
                }
                Some("message_update") => {
                    let update = event.get("assistantMessageEvent");
                    if update.and_then(|update| update.get("type")).and_then(Value::as_str)
                        == Some("text_delta")
                        && let Some(delta) = update
                            .and_then(|update| update.get("delta"))
                            .and_then(Value::as_str)
                        && !delta.is_empty()
                    {
                        let _ = self.inner.events.send(ServerMessage::StreamDelta {
                            session_id: id.clone(),
                            delta: delta.to_owned(),
                        });
                    }
                }
                Some("message_end") => {
                    if event
                        .get("message")
                        .and_then(|message| message.get("role"))
                        .and_then(Value::as_str)
                        == Some("assistant")
                    {
                        let _ = self.inner.events.send(ServerMessage::StreamEnd {
                            session_id: id.clone(),
                        });
                        if let Err(error) = self.sync_history(&id, &process).await {
                            debug!(session = %id, %error, "history was not ready at message completion");
                        }
                    }
                }
                Some("tool_execution_start") => {
                    let detail = event
                        .get("toolName")
                        .and_then(Value::as_str)
                        .map(|name| format!("Running {name}"));
                    self.set_runtime_state(&id, &runtime, SessionStatus::Running, detail);
                }
                Some("tool_execution_end") => {
                    self.set_runtime_state(&id, &runtime, SessionStatus::Running, None);
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
                        Some("Waiting to retry".to_owned()),
                    );
                }
                Some("agent_settled") => {
                    let _guard = runtime.operation.lock().await;
                    if !self.runtime_owns(&runtime, &process).await {
                        break;
                    }
                    self.set_runtime_state(&id, &runtime, SessionStatus::Idle, None);
                    if let Err(error) = self.persist_session_file(&id, &process).await {
                        warn!(session = %id, %error, "failed to persist settled Pi session path");
                    }
                    if let Err(error) = self.sync_history(&id, &process).await {
                        warn!(session = %id, %error, "failed to refresh settled Pi history");
                    }
                    self.broadcast_sessions().await;
                }
                Some("rpc_closed") => {
                    runtime.process.lock().await.take();
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
                    break;
                }
                _ => {}
            }
        }
    }

    async fn runtime_owns(&self, runtime: &SessionRuntime, process: &Arc<RpcProcess>) -> bool {
        runtime
            .process
            .lock()
            .await
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, process))
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

    async fn sync_history(&self, id: &str, process: &RpcProcess) -> Result<()> {
        let response = process.request(json!({ "type": "get_entries" })).await?;
        let data = response
            .get("data")
            .context("Pi entries response had no data")?;
        let entries = data
            .get("entries")
            .and_then(Value::as_array)
            .context("Pi entries response had no entries")?;
        let leaf_id = data.get("leafId").and_then(Value::as_str);
        let mut messages = active_chat_messages(entries, leaf_id)?;
        self.populate_attachment_sizes(entries, &mut messages).await;
        let _ = self.inner.events.send(ServerMessage::History {
            session_id: id.to_owned(),
            messages,
        });
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

fn active_chat_messages(entries: &[Value], leaf_id: Option<&str>) -> Result<Vec<ChatMessage>> {
    let Some(mut current) = leaf_id else {
        return Ok(Vec::new());
    };
    let by_id = entries
        .iter()
        .filter_map(|entry| Some((entry.get("id")?.as_str()?, entry)))
        .collect::<HashMap<_, _>>();
    let mut visited = HashSet::new();
    let mut branch = Vec::new();
    loop {
        if !visited.insert(current) {
            bail!("Pi session branch contains a cycle at {current}");
        }
        let entry = by_id
            .get(current)
            .copied()
            .with_context(|| format!("Pi session branch references missing entry {current}"))?;
        branch.push(entry);
        let Some(parent) = entry.get("parentId").and_then(Value::as_str) else {
            break;
        };
        current = parent;
    }
    branch.reverse();

    Ok(branch
        .into_iter()
        .filter_map(|entry| {
            let entry_id = entry.get("id")?.as_str()?.to_owned();
            let entry_type = entry.get("type")?.as_str()?;
            if entry_type == "custom_message"
                && entry.get("display").and_then(Value::as_bool) == Some(true)
            {
                let text = content_text(entry.get("content")?);
                return (!text.is_empty()).then_some(ChatMessage {
                    entry_id,
                    role: ChatRole::System,
                    text,
                    timestamp_ms: None,
                    attachment: None,
                });
            }
            if entry_type != "message" {
                return None;
            }
            let message = entry.get("message")?;
            if message.get("role").and_then(Value::as_str) == Some("toolResult") {
                let request = attachment_request(entry)?;
                let file_name = request
                    .path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .filter(|name| !name.is_empty())?;
                let text = request.caption.clone().unwrap_or_else(|| match request.kind {
                    AttachmentKind::Image => format!("Image: {file_name}"),
                    AttachmentKind::File => format!("File: {file_name}"),
                });
                return Some(ChatMessage {
                    entry_id,
                    role: ChatRole::System,
                    text,
                    timestamp_ms: message.get("timestamp").and_then(Value::as_u64),
                    attachment: Some(ChatAttachment {
                        kind: request.kind,
                        file_name,
                        caption: request.caption,
                        size: request.size,
                    }),
                });
            }
            let role = match message.get("role")?.as_str()? {
                "user" => ChatRole::User,
                "assistant" => ChatRole::Assistant,
                _ => return None,
            };
            let mut text = content_text(message.get("content")?);
            if text.is_empty() && role == ChatRole::Assistant {
                text = message
                    .get("errorMessage")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
            }
            (!text.is_empty()).then_some(ChatMessage {
                entry_id,
                role,
                text,
                timestamp_ms: message.get("timestamp").and_then(Value::as_u64),
                attachment: None,
            })
        })
        .collect())
}

fn attachment_request(entry: &Value) -> Option<AttachmentRequest> {
    if entry.get("type").and_then(Value::as_str) != Some("message") {
        return None;
    }
    let message = entry.get("message")?;
    if message.get("role").and_then(Value::as_str) != Some("toolResult") {
        return None;
    }
    let attachment = message.get("details")?.get("tauAttachment")?;
    if attachment.get("version").and_then(Value::as_u64) != Some(1) {
        return None;
    }
    let kind = match attachment.get("kind").and_then(Value::as_str)? {
        "image" => AttachmentKind::Image,
        "file" => AttachmentKind::File,
        _ => return None,
    };
    let caption = attachment
        .get("caption")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|caption| !caption.is_empty())
        .map(|caption| bounded(caption, 1024));
    let limit = match kind {
        AttachmentKind::Image => IMAGE_LIMIT,
        AttachmentKind::File => FILE_LIMIT,
    };
    let size = attachment
        .get("size")
        .and_then(Value::as_u64)
        .filter(|size| *size <= limit);
    Some(AttachmentRequest {
        kind,
        path: PathBuf::from(attachment.get("path")?.as_str()?),
        caption,
        size,
    })
}

fn content_text(content: &Value) -> String {
    if let Some(text) = content.as_str() {
        return text.to_owned();
    }
    content
        .as_array()
        .into_iter()
        .flatten()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn title_from_prompt(prompt: &str) -> String {
    let first_line = prompt.lines().find(|line| !line.trim().is_empty()).unwrap_or("New chat");
    bounded(first_line.trim(), MAX_TITLE_CHARS)
}

fn bounded(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconstructs_only_the_active_pi_branch() {
        let entries = vec![
            json!({"type":"message","id":"u1","parentId":null,"message":{"role":"user","content":"Start","timestamp":1}}),
            json!({"type":"message","id":"a1","parentId":"u1","message":{"role":"assistant","content":[{"type":"text","text":"First"}],"timestamp":2}}),
            json!({"type":"message","id":"old","parentId":"a1","message":{"role":"user","content":"Old branch","timestamp":3}}),
            json!({"type":"message","id":"u2","parentId":"a1","message":{"role":"user","content":[{"type":"text","text":"New branch"}],"timestamp":4}}),
            json!({"type":"message","id":"a2","parentId":"u2","message":{"role":"assistant","content":[{"type":"thinking","thinking":"hidden"},{"type":"text","text":"Result"},{"type":"toolCall","name":"send_file"}],"timestamp":5}}),
            json!({"type":"message","id":"t1","parentId":"a2","message":{"role":"toolResult","content":[{"type":"text","text":"queued"}],"details":{"tauAttachment":{"version":1,"kind":"file","path":"/tmp/outbox/result.zip","caption":"Build","size":123}},"timestamp":6}}),
        ];
        let messages = active_chat_messages(&entries, Some("t1")).unwrap();
        assert_eq!(
            messages,
            vec![
                ChatMessage { entry_id: "u1".to_owned(), role: ChatRole::User, text: "Start".to_owned(), timestamp_ms: Some(1), attachment: None },
                ChatMessage { entry_id: "a1".to_owned(), role: ChatRole::Assistant, text: "First".to_owned(), timestamp_ms: Some(2), attachment: None },
                ChatMessage { entry_id: "u2".to_owned(), role: ChatRole::User, text: "New branch".to_owned(), timestamp_ms: Some(4), attachment: None },
                ChatMessage { entry_id: "a2".to_owned(), role: ChatRole::Assistant, text: "Result".to_owned(), timestamp_ms: Some(5), attachment: None },
                ChatMessage { entry_id: "t1".to_owned(), role: ChatRole::System, text: "Build".to_owned(), timestamp_ms: Some(6), attachment: Some(ChatAttachment { kind: AttachmentKind::File, file_name: "result.zip".to_owned(), caption: Some("Build".to_owned()), size: Some(123) }) },
            ]
        );
    }

    #[test]
    fn rejects_broken_or_cyclic_pi_branches() {
        let broken = vec![json!({"type":"message","id":"a","parentId":"missing","message":{"role":"user","content":"x"}})];
        assert!(active_chat_messages(&broken, Some("a")).is_err());
        let cyclic = vec![
            json!({"type":"message","id":"a","parentId":"b","message":{"role":"user","content":"x"}}),
            json!({"type":"message","id":"b","parentId":"a","message":{"role":"assistant","content":"y"}}),
        ];
        assert!(active_chat_messages(&cyclic, Some("a")).is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn drives_a_persistent_streaming_chat_through_pi_rpc() {
        use std::os::unix::fs::PermissionsExt;
        use std::time::Duration;

        use tokio::fs;
        use uuid::Uuid;

        let root = std::env::temp_dir().join(format!("tau-manager-{}", Uuid::new_v4()));
        let session_dir = root.join("pi-sessions");
        fs::create_dir_all(&session_dir).await.unwrap();
        let mock = root.join("mock-pi.py");
        fs::write(
            &mock,
            r#"#!/usr/bin/env python3
import json, os, sys, threading, time
session_dir = sys.argv[sys.argv.index("--session-dir") + 1]
os.makedirs(session_dir, exist_ok=True)
with open(os.path.join(session_dir, "spawn-args"), "a") as marker:
    marker.write(json.dumps(sys.argv[1:]) + "\n")
selected_model = sys.argv[sys.argv.index("--model") + 1] if "--model" in sys.argv else "test/model"
current_provider, current_model = selected_model.split("/", 1)
session_file = os.path.join(session_dir, "mock.jsonl")
if not os.path.exists(session_file) or os.path.getsize(session_file) == 0:
    with open(session_file, "w") as session:
        session.write(json.dumps({"type":"session","version":3,"id":"mock","timestamp":"2025-01-01T00:00:00Z","cwd":os.getcwd()}) + "\n")
entries = []
with open(session_file) as session:
    for record in session:
        entry = json.loads(record)
        if entry.get("type") != "session":
            entries.append(entry)
pending_prompt = None
def append(entry):
    entries.append(entry)
    with open(session_file, "a") as session:
        session.write(json.dumps(entry) + "\n")
for line in sys.stdin:
    command = json.loads(line)
    ident = command.get("id")
    kind = command.get("type")
    response = {"id": ident, "type": "response", "command": kind, "success": True}
    if kind == "get_state":
        response["data"] = {"sessionFile": session_file, "isStreaming": False, "isCompacting": False, "model":{"provider":current_provider,"id":current_model,"name":"Test Model"}}
        print(json.dumps(response), flush=True)
    elif kind == "get_commands":
        response["data"] = {"commands": [
            {"name":"tau-fork-at","description":"private","source":"extension"},
            {"name":"choose","description":"Choose a value","source":"extension"},
            {"name":"review","description":"Review this change","source":"prompt"},
            {"name":"skill:search","description":"Search","source":"skill"}
        ]}
        print(json.dumps(response), flush=True)
    elif kind == "get_available_models":
        response["data"] = {"models": [{"provider":"test","id":"model","name":"Test Model"}]}
        print(json.dumps(response), flush=True)
    elif kind == "get_available_thinking_levels":
        response["data"] = {"levels": ["low", "high"]}
        print(json.dumps(response), flush=True)
    elif kind == "set_model":
        current_provider = command["provider"]
        current_model = command["modelId"]
        response["data"] = {"provider":current_provider,"id":current_model,"name":"Selected Model"}
        print(json.dumps(response), flush=True)
    elif kind == "get_entries":
        response["data"] = {"entries": entries, "leafId": entries[-1]["id"] if entries else None}
        print(json.dumps(response), flush=True)
    elif kind == "prompt":
        if command.get("streamingBehavior") != "steer":
            response["success"] = False
            response["error"] = "prompt was not race-safe"
            print(json.dumps(response), flush=True)
            continue
        if command["message"].split(" ", 1)[0] == "/choose":
            pending_prompt = ident
            print(json.dumps({"type":"extension_ui_request","id":"dialog-1","method":"select","title":"Choose","options":["One","Two"]}), flush=True)
            continue
        append({"type":"message","id":"u1","parentId":None,"message":{"role":"user","content":command["message"],"timestamp":1}})
        print(json.dumps({"type":"agent_start"}), flush=True)
        print(json.dumps({"type":"message_start","message":{"role":"assistant","content":[]}}), flush=True)
        print(json.dumps(response), flush=True)
        def answer():
            time.sleep(0.05)
            print(json.dumps({"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"Hello "}}), flush=True)
            print(json.dumps({"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"from Tau"}}), flush=True)
            assistant = {"role":"assistant","content":[{"type":"text","text":"Hello from Tau"}],"timestamp":2}
            append({"type":"message","id":"a1","parentId":"u1","message":assistant})
            print(json.dumps({"type":"message_end","message":assistant}), flush=True)
            print(json.dumps({"type":"agent_settled"}), flush=True)
        threading.Thread(target=answer, daemon=True).start()
    elif kind == "extension_ui_response":
        with open(os.path.join(session_dir, "extension-response"), "w") as marker:
            marker.write(command.get("value", "cancelled"))
        if pending_prompt:
            print(json.dumps({"id":pending_prompt,"type":"response","command":"prompt","success":True}), flush=True)
            pending_prompt = None
    elif kind == "abort":
        with open(os.path.join(session_dir, "abort-notified"), "w") as marker:
            marker.write("abort")
    else:
        print(json.dumps(response), flush=True)
"#,
        )
        .await
        .unwrap();
        let mut permissions = fs::metadata(&mock).await.unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&mock, permissions).await.unwrap();

        let config = Config {
            bind: "127.0.0.1:0".parse().unwrap(),
            token: Arc::from("test-token-with-at-least-thirty-two-characters"),
            pi_command: mock,
            default_model: "test/model".to_owned(),
            default_thinking_level: "high".to_owned(),
            cwd: root.clone(),
            state_path: root.join("state.json"),
            session_dir,
            telemetry_path: root.join("crashes.jsonl"),
            pi_extension_path: root.join("unused-extension.ts"),
            attachment_root: root.join("outbox"),
            upload_root: root.join("uploads"),
        };
        let state = StateStore::load(config.state_path.clone()).await.unwrap();
        let manager = AgentManager::new(config, state);
        fs::create_dir_all(root.join("outbox")).await.unwrap();
        let artifact = root.join("outbox/result.zip");
        fs::write(&artifact, b"artifact").await.unwrap();
        let attachment_entries = vec![json!({
            "type": "message",
            "id": "attachment",
            "parentId": null,
            "message": {
                "role": "toolResult",
                "content": [{"type": "text", "text": "queued"}],
                "details": {"tauAttachment": {
                    "version": 1,
                    "kind": "file",
                    "path": artifact,
                    "caption": "Build",
                }},
                "timestamp": 1,
            },
        })];
        let mut attachment_messages =
            active_chat_messages(&attachment_entries, Some("attachment")).unwrap();
        manager
            .populate_attachment_sizes(&attachment_entries, &mut attachment_messages)
            .await;
        assert_eq!(attachment_messages[0].attachment.as_ref().unwrap().size, Some(8));

        let mut events = manager.subscribe();
        let session_id = manager.create_session().await.unwrap();
        assert_eq!(
            manager.inner.state.get(&session_id).unwrap().model,
            Some(SessionModel {
                provider: "test".to_owned(),
                model_id: "model".to_owned(),
            }),
        );
        let uploaded = manager
            .store_upload(&session_id, "../source file.rs", b"fn main() {}\n")
            .await
            .unwrap();
        assert_eq!(uploaded.name, "source_file.rs");
        assert!(fs::try_exists(&uploaded.path).await.unwrap());
        manager.open_session(&session_id).await.unwrap();
        let runtime = manager
            .inner
            .runtimes
            .lock()
            .await
            .get(&session_id)
            .unwrap()
            .clone();
        assert_eq!(runtime.snapshot().status, SessionStatus::Sleeping);
        assert!(runtime.process.lock().await.is_none());
        manager.prompt(&session_id, "Say hello").await.unwrap();
        let spawn_arguments = fs::read_to_string(root.join("pi-sessions/spawn-args"))
            .await
            .unwrap();
        let spawn_arguments =
            serde_json::from_str::<Vec<String>>(spawn_arguments.lines().next().unwrap()).unwrap();
        assert!(
            spawn_arguments
                .windows(2)
                .any(|arguments| arguments == ["--model", "test/model"])
        );
        assert!(
            spawn_arguments
                .windows(2)
                .any(|arguments| arguments == ["--thinking", "high"])
        );

        let mut streamed = String::new();
        let history = tokio::time::timeout(Duration::from_secs(5), async {
            let mut user_history_seen = false;
            loop {
                match events.recv().await.unwrap() {
                    ServerMessage::History { session_id: event_session, messages }
                        if event_session == session_id => {
                            user_history_seen |= messages.iter().any(|message| {
                                message.role == ChatRole::User && message.text == "Say hello"
                            });
                            if messages.last().is_some_and(|message| message.role == ChatRole::Assistant) {
                                break messages;
                            }
                        }
                    ServerMessage::StreamReset { session_id: event_session }
                        if event_session == session_id => assert!(
                            user_history_seen,
                            "assistant stream was exposed before its canonical user message",
                        ),
                    ServerMessage::StreamDelta { session_id: event_session, delta }
                        if event_session == session_id => streamed.push_str(&delta),
                    _ => {}
                }
            }
        })
        .await
        .expect("mock Pi chat did not settle");
        assert_eq!(streamed, "Hello from Tau");
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].text, "Say hello");
        assert_eq!(history[1].text, "Hello from Tau");

        let commands = manager.commands(&session_id).await.unwrap();
        assert!(commands.iter().any(|command| {
            command.name == "choose" && command.source == SlashCommandSource::Extension
        }));
        assert!(commands.iter().any(|command| {
            command.name == "review" && command.source == SlashCommandSource::Prompt
        }));
        assert!(commands.iter().any(|command| {
            command.name == "model"
                && command.source == SlashCommandSource::Builtin
                && command.arguments.iter().any(|argument| argument.value == "test/model")
        }));
        assert!(commands.iter().all(|command| command.name != INTERNAL_FORK_COMMAND));

        let command_manager = manager.clone();
        let command_session = session_id.clone();
        let command_task = tokio::spawn(async move {
            command_manager.prompt(&command_session, "/choose").await
        });
        let dialog = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if let ServerMessage::ExtensionUi { session_id: event_session, request } =
                    events.recv().await.unwrap()
                    && event_session == session_id
                    && request.method == "select"
                {
                    break request;
                }
            }
        })
        .await
        .expect("extension dialog was not forwarded");
        assert_eq!(dialog.options, ["One", "Two"]);
        manager
            .extension_ui_response(
                &session_id,
                &dialog.id,
                Some("Two".to_owned()),
                None,
                false,
            )
            .await
            .unwrap();
        let outcome = command_task.await.unwrap().unwrap();
        assert!(outcome.command_handled);
        assert_eq!(runtime.snapshot().status, SessionStatus::Idle);
        assert_eq!(
            fs::read_to_string(root.join("pi-sessions/extension-response"))
                .await
                .unwrap(),
            "Two",
        );

        let thinking = manager.prompt(&session_id, "/thinking high").await.unwrap();
        assert!(thinking.command_handled);
        assert_eq!(thinking.notice.as_deref(), Some("Thinking level set to high."));
        let model = manager.prompt(&session_id, "/model test/other").await.unwrap();
        assert!(model.command_handled);
        assert_eq!(model.notice.as_deref(), Some("Model set to test/other."));
        assert_eq!(
            manager.inner.state.get(&session_id).unwrap().model,
            Some(SessionModel {
                provider: "test".to_owned(),
                model_id: "other".to_owned(),
            }),
        );
        assert!(manager.prompt(&session_id, "/settings").await.is_err());

        let session_file = manager
            .inner
            .state
            .get(&session_id)
            .unwrap()
            .session_file
            .unwrap();

        for _ in 0..100 {
            if runtime.snapshot().status == SessionStatus::Idle {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(runtime.snapshot().status, SessionStatus::Idle);
        manager.set_runtime_state(&session_id, &runtime, SessionStatus::Running, None);
        tokio::time::timeout(Duration::from_secs(1), manager.abort(&session_id))
            .await
            .expect("abort waited for Pi to settle")
            .unwrap();
        let abort_marker = root.join("pi-sessions/abort-notified");
        for _ in 0..100 {
            if fs::try_exists(&abort_marker).await.unwrap() {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(fs::try_exists(abort_marker).await.unwrap());
        manager.set_runtime_state(&session_id, &runtime, SessionStatus::Idle, None);
        let idle_since = runtime.snapshot().idle_since.unwrap();
        manager
            .sleep_if_idle(
                &session_id,
                idle_since.checked_sub(Duration::from_secs(1)).unwrap(),
            )
            .await;
        assert_eq!(runtime.snapshot().status, SessionStatus::Idle);
        manager.sleep_if_idle(&session_id, idle_since).await;
        assert_eq!(runtime.snapshot().status, SessionStatus::Sleeping);
        assert!(runtime.process.lock().await.is_none());

        let mut reopened_events = manager.subscribe();
        manager.open_session(&session_id).await.unwrap();
        assert_eq!(runtime.snapshot().status, SessionStatus::Sleeping);
        assert!(runtime.process.lock().await.is_none());
        let reopened_history = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if let ServerMessage::History { session_id: event_session, messages } =
                    reopened_events.recv().await.unwrap()
                    && event_session == session_id
                {
                    break messages;
                }
            }
        })
        .await
        .expect("cold chat history was not loaded");
        assert_eq!(reopened_history.len(), 2);
        assert_eq!(reopened_history[0].text, "Say hello");
        assert_eq!(reopened_history[1].text, "Hello from Tau");

        manager.delete_session(&session_id).await.unwrap();
        assert!(manager.inner.state.get(&session_id).is_none());
        assert!(!fs::try_exists(session_file).await.unwrap());
        assert!(!fs::try_exists(uploaded.path).await.unwrap());

        manager.shutdown().await;
        fs::remove_dir_all(root).await.unwrap();
    }
}
