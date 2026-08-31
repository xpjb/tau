use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock as StdRwLock};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::sync::{Mutex, broadcast};
use tracing::{debug, error, warn};

use crate::config::Config;
use crate::pi::RpcProcess;
use crate::protocol::{
    ChatMessage, ChatRole, ServerMessage, SessionStatus, SessionSummary, MAX_TITLE_CHARS,
};
use crate::state::StateStore;

const EVENT_BUFFER: usize = 2048;

#[derive(Clone)]
pub struct AgentManager {
    inner: Arc<ManagerInner>,
}

struct ManagerInner {
    config: Config,
    state: StateStore,
    runtimes: Mutex<HashMap<String, Arc<SessionRuntime>>>,
    events: broadcast::Sender<ServerMessage>,
    shutting_down: AtomicBool,
}

struct SessionRuntime {
    operation: Mutex<()>,
    process: Mutex<Option<Arc<RpcProcess>>>,
    state: StdRwLock<RuntimeState>,
}

#[derive(Clone, Default)]
struct RuntimeState {
    status: SessionStatus,
    detail: Option<String>,
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
        let id = self
            .inner
            .state
            .create("New chat".to_owned(), None, None)
            .await?;
        self.broadcast_sessions().await;
        Ok(id)
    }

    pub async fn open_session(&self, id: &str) -> Result<()> {
        let runtime = self.runtime(id).await?;
        let _guard = runtime.operation.lock().await;
        let process = self.ensure_process(id, &runtime).await?;
        self.sync_history(id, &process).await?;
        let state = runtime.snapshot();
        let _ = self.inner.events.send(ServerMessage::SessionState {
            session_id: id.to_owned(),
            status: state.status,
            detail: state.detail,
        });
        Ok(())
    }

    pub async fn prompt(&self, id: &str, text: &str) -> Result<()> {
        if text.trim().is_empty() {
            bail!("message cannot be empty");
        }
        let runtime = self.runtime(id).await?;
        let _guard = runtime.operation.lock().await;
        let process = self.ensure_process(id, &runtime).await?;
        let state = process.request(json!({ "type": "get_state" })).await?;
        let streaming = state
            .get("data")
            .and_then(|data| data.get("isStreaming"))
            .and_then(Value::as_bool)
            .unwrap_or(false);

        self.set_runtime_state(id, &runtime, SessionStatus::Running, None);
        let command = if streaming {
            json!({
                "type": "prompt",
                "message": text,
                "streamingBehavior": "steer"
            })
        } else {
            json!({ "type": "prompt", "message": text })
        };
        if let Err(error) = process.request_unbounded(command).await {
            let status = if process.is_alive() {
                SessionStatus::Idle
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

        if let Some(stored) = self.inner.state.get(id)
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
        self.broadcast_sessions().await;
        Ok(())
    }

    pub async fn abort(&self, id: &str) -> Result<()> {
        let runtime = self.runtime(id).await?;
        let _guard = runtime.operation.lock().await;
        let process = runtime.process.lock().await.clone();
        if runtime.snapshot().status != SessionStatus::Running {
            return Ok(());
        }
        if let Some(process) = process.filter(|process| process.is_alive()) {
            process.request(json!({ "type": "abort" })).await?;
        }
        Ok(())
    }

    pub async fn close_session(&self, id: &str) -> Result<()> {
        let runtime = self.runtime(id).await?;
        let _guard = runtime.operation.lock().await;
        let process = runtime.process.lock().await.take();
        self.set_runtime_state(id, &runtime, SessionStatus::Sleeping, None);
        if let Some(process) = process {
            process.shutdown().await;
        }
        self.broadcast_sessions().await;
        Ok(())
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

    pub async fn fork_session(&self, id: &str, entry_id: &str) -> Result<(String, String)> {
        let (child, draft) = self
            .branch_session(id, BranchOperation::Fork(entry_id))
            .await?;
        Ok((child, draft.context("Pi fork returned no draft")?))
    }

    pub async fn clone_session(&self, id: &str) -> Result<String> {
        self.branch_session(id, BranchOperation::Clone)
            .await
            .map(|(child, _)| child)
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
            let child_file = state
                .get("data")
                .and_then(|data| data.get("sessionFile"))
                .and_then(Value::as_str)
                .context("forked Pi session has no session file")?
                .to_owned();
            if child_file == parent_file {
                bail!("Pi did not create an independent session file");
            }
            Ok::<_, anyhow::Error>((child_file, draft))
        }
        .await;
        temporary.shutdown().await;
        let (child_file, draft) = result?;

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
            let _guard = runtime.operation.lock().await;
            if let Some(process) = runtime.process.lock().await.take() {
                process.shutdown().await;
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
            manager.monitor_process(session_id, monitored).await;
        });
        Ok(process)
    }

    async fn monitor_process(&self, id: String, process: Arc<RpcProcess>) {
        let mut events = process.subscribe();
        loop {
            let event = match events.recv().await {
                Ok(event) => event,
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    error!(session = %id, skipped, "Pi event stream lagged; closing the process");
                    if let Ok(runtime) = self.runtime(&id).await
                        && self.runtime_owns(&runtime, &process).await
                    {
                        runtime.process.lock().await.take();
                        self.set_runtime_state(
                            &id,
                            &runtime,
                            SessionStatus::Error,
                            Some(format!("Pi event stream skipped {skipped} events")),
                        );
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
        let messages = active_chat_messages(entries, leaf_id)?;
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
        let next = RuntimeState { status, detail };
        let mut current = runtime
            .state
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if current.status == next.status && current.detail == next.detail {
            return;
        }
        *current = next.clone();
        drop(current);
        let _ = self.inner.events.send(ServerMessage::SessionState {
            session_id: id.to_owned(),
            status: next.status,
            detail: next.detail,
        });
    }

    async fn broadcast_sessions(&self) {
        let message = self.sessions_message().await;
        let _ = self.inner.events.send(message);
    }
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
            if entry_type == "custom_message" && entry.get("display").and_then(Value::as_bool) == Some(true) {
                let text = content_text(entry.get("content")?);
                return (!text.is_empty()).then_some(ChatMessage {
                    entry_id,
                    role: ChatRole::System,
                    text,
                    timestamp_ms: None,
                });
            }
            if entry_type != "message" {
                return None;
            }
            let message = entry.get("message")?;
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
            })
        })
        .collect())
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
            json!({"type":"message","id":"a2","parentId":"u2","message":{"role":"assistant","content":[{"type":"thinking","thinking":"hidden"},{"type":"text","text":"Result"},{"type":"toolCall","name":"bash"}],"timestamp":5}}),
        ];
        let messages = active_chat_messages(&entries, Some("a2")).unwrap();
        assert_eq!(
            messages,
            vec![
                ChatMessage { entry_id: "u1".to_owned(), role: ChatRole::User, text: "Start".to_owned(), timestamp_ms: Some(1) },
                ChatMessage { entry_id: "a1".to_owned(), role: ChatRole::Assistant, text: "First".to_owned(), timestamp_ms: Some(2) },
                ChatMessage { entry_id: "u2".to_owned(), role: ChatRole::User, text: "New branch".to_owned(), timestamp_ms: Some(4) },
                ChatMessage { entry_id: "a2".to_owned(), role: ChatRole::Assistant, text: "Result".to_owned(), timestamp_ms: Some(5) },
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
import json, os, sys
session_dir = sys.argv[sys.argv.index("--session-dir") + 1]
os.makedirs(session_dir, exist_ok=True)
session_file = os.path.join(session_dir, "mock.jsonl")
open(session_file, "a").close()
entries = []
for line in sys.stdin:
    command = json.loads(line)
    ident = command.get("id")
    kind = command.get("type")
    response = {"id": ident, "type": "response", "command": kind, "success": True}
    if kind == "get_state":
        response["data"] = {"sessionFile": session_file, "isStreaming": False}
        print(json.dumps(response), flush=True)
    elif kind == "get_entries":
        response["data"] = {"entries": entries, "leafId": entries[-1]["id"] if entries else None}
        print(json.dumps(response), flush=True)
    elif kind == "prompt":
        entries.append({"type":"message","id":"u1","parentId":None,"message":{"role":"user","content":command["message"],"timestamp":1}})
        print(json.dumps(response), flush=True)
        print(json.dumps({"type":"agent_start"}), flush=True)
        print(json.dumps({"type":"message_start","message":{"role":"assistant","content":[]}}), flush=True)
        print(json.dumps({"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"Hello "}}), flush=True)
        print(json.dumps({"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"from Tau"}}), flush=True)
        assistant = {"role":"assistant","content":[{"type":"text","text":"Hello from Tau"}],"timestamp":2}
        entries.append({"type":"message","id":"a1","parentId":"u1","message":assistant})
        print(json.dumps({"type":"message_end","message":assistant}), flush=True)
        print(json.dumps({"type":"agent_settled"}), flush=True)
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
            cwd: root.clone(),
            state_path: root.join("state.json"),
            session_dir,
            telemetry_path: root.join("crashes.jsonl"),
        };
        let state = StateStore::load(config.state_path.clone()).await.unwrap();
        let manager = AgentManager::new(config, state);
        let mut events = manager.subscribe();
        let session_id = manager.create_session().await.unwrap();
        manager.open_session(&session_id).await.unwrap();
        manager.prompt(&session_id, "Say hello").await.unwrap();

        let mut streamed = String::new();
        let history = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match events.recv().await.unwrap() {
                    ServerMessage::StreamDelta { session_id: event_session, delta }
                        if event_session == session_id => streamed.push_str(&delta),
                    ServerMessage::History { session_id: event_session, messages }
                        if event_session == session_id
                            && messages.last().is_some_and(|message| message.role == ChatRole::Assistant) => break messages,
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
        assert!(manager.inner.state.get(&session_id).unwrap().session_file.is_some());

        manager.shutdown().await;
        fs::remove_dir_all(root).await.unwrap();
    }
}
