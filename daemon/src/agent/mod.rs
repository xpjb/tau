pub mod auth;
pub mod history;
mod provider;
mod tools;

use std::sync::Arc;
use std::collections::HashMap;
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use crate::manager::{AgentManager, SessionContent, SessionRuntime, bounded};
use crate::protocol::{ContextUsage, ServerMessage, SessionStatus};
use crate::state::SessionModel;
use crate::transcript::{QueueState, TranscriptChange};
use crate::settings::SteeringMode;
use crate::state::{StateStore, Receipt};

pub struct AgentSession {
    pub store: StateStore,
    pub revision: u64,
    pub model: SessionModel,
    pub thinking: String,
    pub running: bool,
    pub cancel: CancellationToken,
    pub task: Option<tokio::task::JoinHandle<()>>,
    pub tokens: Option<u64>,
    pub needs_turn: bool,
}
pub fn now_ms() -> u64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis().try_into().unwrap_or(u64::MAX) }

impl SessionContent {
    pub async fn append(&mut self, id: &str, value: Value) -> Result<()> { self.commit(id, vec![value], None, None).await }
    pub async fn commit(&mut self, id: &str, values: Vec<Value>, queue: Option<QueueState>, receipt: Option<Receipt>) -> Result<()> {
        let agent = self.agent.as_mut().context("Session is not loaded")?;
        let mut projected = self.transcript.as_ref().unwrap().clone();
        let mut change = TranscriptChange::default(); let mut entries = Vec::new();
        for mut entry in values {
            entry["id"] = json!(uuid::Uuid::new_v4().to_string()); entry["parentId"] = json!(projected.head);
            entry["timestamp"] = json!(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis,true));
            let next = projected.project(&entry,false)?; projected.apply(&next)?;
            change.events.extend(next.events); change.removed.extend(next.removed); change.delivered.extend(next.delivered);
            change.head = next.head; change.bumps_chat |= next.bumps_chat; entries.push(entry);
        }
        change.queue = queue.clone();
        let saved = agent.store.commit(id,agent.revision,entries,change.events.clone(),queue,receipt).await?;
        agent.revision = saved.revision; agent.model = saved.model; agent.thinking = saved.thinking;
        agent.tokens = saved.tokens; agent.needs_turn = saved.needs_turn;
        self.publish(id,change)
    }
    pub fn publish(&mut self, id: &str, change: TranscriptChange) -> Result<()> {
        let transcript = self.transcript.as_mut().unwrap();
        transcript.apply(&change)?;
        let _ = self.events.send(Arc::new(ServerMessage::TranscriptUpdate { session_id:id.into(), generation:transcript.generation.clone(), sequence:transcript.sequence, change }));
        Ok(())
    }
    pub async fn save_queue(&mut self, id: &str, queue: QueueState, receipt: Option<Receipt>) -> Result<()> {
        self.commit(id,Vec::new(),Some(queue),receipt).await
    }
    pub fn live(&mut self, id: &str, stream: &str, message: Value) -> Result<()> {
        let transcript = self.transcript.as_mut().unwrap();
        let mut change = transcript.project(&json!({"streamId":stream,"parentId":transcript.head,"message":message}), true)?;
        // Emit a delta for the common single growing block; avoid resending the whole response.
        change.events.retain(|event| transcript.event(&event.id).is_none_or(|old| old != event));
        if change.events.len() == 1 && let Some(old) = transcript.event(&change.events[0].id) {
            let event = &change.events[0];
            if old.kind == event.kind && matches!(event.kind, crate::transcript::EventKind::Text | crate::transcript::EventKind::Thinking)
                && let Some(delta) = event.text.strip_prefix(&old.text) {
                change.delta = Some(crate::transcript::TextDelta { event_id:event.id.clone(), text:delta.into() }); change.events.clear();
            }
        }
        if change.events.is_empty() && change.delta.is_none() && change.removed.is_empty() { return Ok(()); }
        self.publish(id, change)
    }
}

fn assistant_message(message: &Value, model: &SessionModel, stop: &str, started: &mut HashMap<String, u64>) -> Value {
    let mut content = Vec::new();
    if let Some(reasoning) = message.get("reasoning").or_else(|| message.get("reasoning_content")).and_then(Value::as_str).filter(|v| !v.is_empty()) {
        content.push(json!({"type":"thinking","thinking":reasoning}));
    }
    if let Some(text) = message["content"].as_str().filter(|v| !v.is_empty()) { content.push(json!({"type":"text","text":text})); }
    for call in message["tool_calls"].as_array().into_iter().flatten() {
        let arguments = call["function"]["arguments"].as_str().unwrap_or_default();
        content.push(json!({"type":"toolCall","id":call["id"],"name":call["function"]["name"],
            "arguments":serde_json::from_str::<Value>(arguments).unwrap_or(Value::Null),"partialArguments":arguments}));
    }
    // First observation by the daemon, not the final entry's save time or a
    // fabricated provider timestamp. Keep it unchanged across deltas and restart.
    for block in &mut content {
        let key = if block["type"] == "toolCall" { format!("tool:{}", block["id"].as_str().unwrap_or_default()) }
            else { block["type"].as_str().unwrap().to_owned() };
        block["timestamp"] = json!(*started.entry(key).or_insert_with(now_ms));
    }
    json!({"role":"assistant","content":content,"provider":model.provider,"model":model.model_id,"stopReason":stop})
}
impl AgentManager {
    pub(crate) fn start_run(&self, id: &str, runtime: &Arc<SessionRuntime>, content: &mut SessionContent) {
        let agent = content.agent.as_mut().unwrap();
        let queue = &content.transcript.as_ref().unwrap().queue;
        if agent.running || queue.paused || queue.requests.is_empty() && !agent.needs_turn { return; }
        agent.running = true; agent.cancel = CancellationToken::new();
        let manager = self.clone(); let id = id.to_owned(); let runtime = runtime.clone();
        self.set_runtime_state(&id, &runtime, SessionStatus::Running, None, None);
        agent.task = Some(tokio::spawn(async move {
            loop {
            let mut result = manager.run_agent(&id, &runtime).await;
            let mut content = runtime.content.lock().await;
            let Some(agent) = &mut content.agent else { return; };
            let cancelled = agent.cancel.is_cancelled();
            if result.is_ok() && !cancelled && content.transcript.as_ref().is_some_and(|t| !t.queue.paused && !t.queue.requests.is_empty()) { drop(content); continue; }
            let agent = content.agent.as_mut().unwrap();
            agent.running = false;
            let settings = manager.inner.settings.get();
            let usage = settings.model(&agent.model).ok().map(|model| ContextUsage { tokens:agent.tokens, context_window:model.context_window });
            let mut queue = content.transcript.as_ref().unwrap().queue.clone();
            queue.run_id = None;
            if cancelled || result.is_err() { queue.paused = true; }
            if let Err(error) = content.save_queue(&id, queue, None).await {
                tracing::error!(session=%id, %error, "Could not save settled queue");
                result = Err(error.context("Could not save settled queue"));
            }
            let mut interrupted = TranscriptChange::default();
            for event in content.transcript.as_mut().unwrap().events_mut().filter(|event| event.phase == crate::transcript::EventPhase::Live) {
                let mut event = event.clone(); event.phase = crate::transcript::EventPhase::Interrupted;
                interrupted.events.push(event);
            }
            if !interrupted.events.is_empty() { let _ = content.publish(&id, interrupted); }
            manager.set_runtime_state(&id, &runtime, if result.is_err() && !cancelled { SessionStatus::Error } else { SessionStatus::Idle },
                result.err().map(|e| bounded(&e.to_string(), 480)), Some(usage));
            drop(content);
            manager.broadcast_sessions().await;
            break;
            }
        }));
    }

    async fn run_agent(&self, id: &str, runtime: &Arc<SessionRuntime>) -> Result<()> {
        let mut compacted = false;
        loop {
            let settings = self.inner.settings.get();
            let (selected, thinking, cancel, messages, tokens) = {
                let mut content = runtime.content.lock().await;
                if content.agent.as_ref().unwrap().cancel.is_cancelled() { return Ok(()); }
                let mut queue = content.transcript.as_ref().unwrap().queue.clone();
                if let Some(control) = &mut queue.control && control.action == "pause" && control.status == "waiting" {
                    queue.paused = true; control.status = "applied".into();
                }
                if queue.paused && !content.agent.as_ref().unwrap().needs_turn {
                    content.save_queue(id, queue, None).await?; return Ok(());
                }
                if queue.control.as_ref().is_some_and(|c| c.action == "pause" && c.status == "applied") {
                    content.save_queue(id, queue, None).await?; return Ok(());
                }
                let count = if queue.paused { 0 } else if let Some(control) = queue.control.as_ref().filter(|c| c.action == "prefix" && c.status == "waiting") {
                    if control.requests.len() > queue.requests.len() || control.requests.iter().zip(&queue.requests).any(|(a,b)| a.request_id != b.request_id || a.revision != b.revision) { bail!("Queue prefix changed before it could run"); }
                    control.requests.len()
                } else if settings.agent.steering_mode == SteeringMode::All { queue.requests.len() } else { queue.requests.len().min(1) };
                let requests = queue.requests.drain(..count).collect::<Vec<_>>();
                let entries = requests.into_iter().map(|request| json!({"type":"message","origin":{"requestId":request.request_id,"requestRevision":request.revision},
                    "message":{"role":"user","content":request.text,"timestamp":request.timestamp_ms}})).collect::<Vec<_>>();
                if !entries.is_empty() {
                    if let Some(control) = &mut queue.control && control.action == "prefix" && control.status == "waiting" { control.status = "applied".into(); queue.paused = true; }
                } else if !content.agent.as_ref().unwrap().needs_turn { return Ok(()); }
                queue.run_id.get_or_insert_with(|| uuid::Uuid::new_v4().to_string());
                // Consuming queued messages and saving their user entries is one transaction.
                content.commit(id,entries,Some(queue),None).await?;
                let agent = content.agent.as_ref().unwrap();
                let entries = agent.store.context(id,&agent.model).await?;
                let messages = history::messages(&entries,settings.system_prompt(&self.inner.config.cwd).await?, &agent.model, &self.inner.config.attachment_root).await?;
                (agent.model.clone(), agent.thinking.clone(), agent.cancel.clone(), messages, agent.tokens)
            };
            let context_window = settings.model(&selected)?.context_window;
            let estimated = messages.iter().map(history::estimate_tokens).sum::<u64>();
            if settings.agent.compaction.enabled && tokens.unwrap_or(estimated).max(estimated) > context_window.saturating_sub(settings.agent.compaction.reserve_tokens) {
                if compacted { bail!("Context is still too large after compaction; reduce the queued input or fork an earlier turn"); }
                self.compact(id, runtime, "").await?;
                compacted = true;
                continue;
            }
            let stream = uuid::Uuid::new_v4().to_string();
            let (updates, mut receiver) = mpsc::channel(8);
            let generation = provider::generate(&self.inner.http, &self.inner.auth, provider::Request {
                settings:&settings, selected:&selected, thinking:&thinking, messages:&messages, session_id:id,
                definitions:tools::definitions(), mode:provider::Mode::Chat,
            }, updates);
            tokio::pin!(generation);
            let mut partial = json!({"role":"assistant","content":""});
            let mut started = HashMap::new();
            let completion = loop {
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => break Err(anyhow::anyhow!("Aborted")),
                    Some(message) = receiver.recv() => {
                        partial = message;
                        runtime.content.lock().await.live(id, &stream, assistant_message(&partial, &selected, "", &mut started))?;
                    }
                    result = &mut generation => break result,
                }
            };
            let completion = match completion {
                Ok(completion) => completion,
                Err(error) => {
                    let mut content = runtime.content.lock().await;
                    let mut message = assistant_message(&partial, &selected, if cancel.is_cancelled() {"aborted"} else {"error"}, &mut started);
                    message["errorMessage"] = json!(bounded(&error.to_string(), 480));
                    message["timestamp"] = json!(now_ms());
                    content.append(id, json!({"type":"message","origin":{"streamId":stream},"message":message})).await?;
                    content.agent.as_mut().unwrap().needs_turn = true;
                    return Err(error);
                }
            };
            compacted = false;
            let calls = completion.message["tool_calls"].as_array().cloned().unwrap_or_default();
            let mut attachments = Vec::new();
            // Decode/validate the entire response before staging or publishing any files.
            // Native images are deliveries, not fabricated local function calls.
            for image in &completion.images {
                let staged = crate::attachments::generated_image(&self.inner.config.attachment_root, &image.bytes, &cancel).await?;
                attachments.push(json!({"type":"tau_attachment","message":{"role":"assistant","content":[{"type":"image","mimeType":"image/png"}],
                    "details":staged["details"],"timestamp":now_ms()}}));
            }
            {
                let mut message = assistant_message(&completion.message, &selected, if completion.limited {"length"} else if calls.is_empty() {"stop"} else {"toolUse"}, &mut started);
                message["tauModelMessage"] = completion.message.clone();
                message["timestamp"] = json!(now_ms());
                let mut content = runtime.content.lock().await;
                content.append(id, json!({"type":"message","origin":{"streamId":stream},"message":message})).await?;
                let agent = content.agent.as_mut().unwrap(); agent.tokens = completion.tokens; agent.needs_turn = !calls.is_empty();
            }
            for call in calls {
                let name = call["function"]["name"].as_str().context("Tool call has no name")?;
                let args = serde_json::from_str::<Value>(call["function"]["arguments"].as_str().context("Tool call has no arguments")?);
                let result: Result<Value> = match args {
                    Err(error) => Err(error).context("Tool arguments are invalid JSON"),
                    Ok(args) if name == "web_search" => {
                        let query = args["query"].as_str().unwrap_or_default();
                        let messages = vec![json!({"role":"system","content":"Search the web and return concise findings with source URLs."}),json!({"role":"user","content":query})];
                        let (updates, _receiver) = mpsc::channel(1);
                        tokio::select! {
                            _ = cancel.cancelled() => Err(anyhow::anyhow!("Tool cancelled")),
                            result = provider::generate(&self.inner.http, &self.inner.auth, provider::Request {
                                settings:&settings, selected:&selected, thinking:"low", messages:&messages, session_id:id, definitions:vec![], mode:provider::Mode::Search,
                            }, updates) =>
                                result.map(|completion| tools::text_result(format!("{}\n{}", completion.message["content"].as_str().unwrap_or_default(), completion.message["annotations"]))),
                        }
                    }
                    Ok(args) => tools::execute(&self.inner.config, &settings, &self.inner.state, id, name, &args, &cancel).await,
                };
                let mut result = match result { Ok(result) => result, Err(error) => { let mut result = tools::text_result(bounded(&error.to_string(), 2000)); result["isError"] = json!(true); result } };
                result["role"] = json!("toolResult"); result["toolCallId"] = call["id"].clone(); result["toolName"] = json!(name); result["timestamp"] = json!(now_ms());
                if name == "flag_it" && result["isError"] != true {
                    let _ = self.inner.events.send(ServerMessage::ExtensionUi { session_id:id.into(), request:Box::new(crate::protocol::ExtensionUiRequest {
                        id:uuid::Uuid::new_v4().to_string(), method:"notify".into(), notify_type:Some("info".into()), message:result["content"][0]["text"].as_str().map(str::to_owned), ..Default::default()
                    }) });
                }
                runtime.content.lock().await.append(id, json!({"type":"message","message":result})).await?;
            }
            for attachment in attachments { runtime.content.lock().await.append(id, attachment).await?; }
            self.set_runtime_state(id, runtime, SessionStatus::Running, None, Some(Some(ContextUsage { tokens:completion.tokens, context_window })));
        }
    }

    pub(crate) async fn compact(&self, id: &str, runtime: &Arc<SessionRuntime>, instructions: &str) -> Result<()> {
        let settings = self.inner.settings.get();
        let (selected, thinking, cancel, first_kept, prefix, before_tokens) = {
            let content = runtime.content.lock().await; let agent = content.agent.as_ref().unwrap();
            let entries = agent.store.context(id,&agent.model).await?;
            let users = entries.iter().enumerate().filter(|(_, entry)| entry["message"]["role"] == "user").map(|(index,_)| index).collect::<Vec<_>>();
            if users.len() < 2 { bail!("Not enough completed turns to compact safely"); }
            let mut cut = *users.last().unwrap(); let mut size = 0;
            for (index, entry) in entries.iter().enumerate().rev() {
                if matches!(entry["type"].as_str(), Some("message" | "tau_attachment")) { size += history::estimate_tokens(&entry["message"]); }
                if entry["message"]["role"] == "user" && size <= settings.agent.compaction.keep_recent_tokens { cut = index; }
            }
            if cut <= users[0] { cut = users[1]; }
            let mut prefix = entries[..cut].to_vec();
            // An earlier checkpoint can sit after its retained boundary in append order.
            prefix.extend(entries[cut..].iter().filter(|entry| entry["type"] == "compaction").cloned());
            (agent.model.clone(), agent.thinking.clone(), agent.cancel.clone(), entries[cut]["id"].clone(), prefix, agent.tokens)
        };
        settings.model(&selected)?;
        self.set_runtime_state(id, runtime, SessionStatus::Running, Some("Compacting context".into()), None);
        let native = settings.agent.compaction.native_codex && settings.providers[&selected.provider].api == crate::settings::Api::Codex;
        let mut messages = history::messages(&prefix,settings.system_prompt(&self.inner.config.cwd).await?, &selected, &self.inner.config.attachment_root).await?;
        if !native { messages.push(json!({"role":"user","content":format!("Summarize this conversation for another coding agent. Preserve goals, decisions, files changed, commands run, pending work, and important constraints. Do not continue the task. {instructions}")})); }
        else if !instructions.is_empty() { messages.push(json!({"role":"user","content":format!("Compaction instructions: {instructions}")})); }
        let (updates, _receiver) = mpsc::channel(1);
        let generation = provider::generate(&self.inner.http, &self.inner.auth, provider::Request {
            settings:&settings, selected:&selected, thinking:&thinking, messages:&messages, session_id:id, definitions:vec![],
            mode:if native {provider::Mode::Compact} else {provider::Mode::Summary},
        }, updates);
        let result = tokio::select! {
            _ = cancel.cancelled() => bail!("Compaction cancelled; history is unchanged"),
            result = tokio::time::timeout(std::time::Duration::from_secs(settings.agent.compaction.timeout_seconds), generation) => result.context("Compaction timed out; history is unchanged")??,
        };
        let (summary, details) = if native {
            let item = result.message["codex_output"].as_array().and_then(|items| items.iter().find(|item| item["type"] == "compaction" && item["encrypted_content"].as_str().is_some_and(|v| !v.is_empty())))
                .context("Codex returned no native checkpoint; history is unchanged")?;
            ("Codex native compaction. Encrypted history is stored in this session.".to_owned(), json!({"kind":"codex-native-compaction","version":1,
                "provider":selected.provider,"model":selected.model_id,"api":"openai-codex-responses","baseUrl":settings.providers[&selected.provider].base_url,"accountId":result.account,"item":item}))
        } else {
            (result.message["content"].as_str().filter(|text| !text.trim().is_empty()).context("Compaction returned an empty summary")?.to_owned(), Value::Null)
        };
        let mut content = runtime.content.lock().await;
        content.append(id, json!({"type":"compaction","summary":summary,"firstKeptEntryId":first_kept,"tokensBefore":before_tokens,"details":details})).await?;
        content.agent.as_mut().unwrap().tokens = None;
        Ok(())
    }

    pub(crate) async fn title_after_prompt(&self, id: &str, text: &str) {
        let Ok(Some(stored)) = self.inner.state.get(id).await else { return; };
        if stored.title != "New chat" { self.broadcast_sessions().await; return; }
        let title = async {
            use tokio::io::AsyncWriteExt;
            let command = self.inner.config.title_command.as_ref()?;
            let template = self.inner.settings.get().daemon.title_prompt;
            tokio::time::timeout(std::time::Duration::from_secs(30), async {
                let mut child = tokio::process::Command::new("sh").arg("-c").arg(command)
                    .stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).kill_on_drop(true).spawn().ok()?;
                let mut stdin = child.stdin.take()?;
                stdin.write_all(json!({"text":text,"promptTemplate":template}).to_string().as_bytes()).await.ok()?;
                stdin.write_all(b"\n").await.ok()?; drop(stdin);
                let output = child.wait_with_output().await.ok()?;
                let raw: Value = serde_json::from_slice(&output.stdout).ok()?;
                let title = raw["title"].as_str()?.trim();
                (title.chars().count() > 1 && title.chars().count() <= crate::protocol::MAX_TITLE_CHARS && !title.contains(['\n','\r'])).then(|| title.to_owned())
            }).await.ok().flatten()
        }.await.unwrap_or_else(|| bounded(text.lines().find(|line| !line.trim().is_empty()).unwrap_or("Unnamed chat").trim(), crate::protocol::MAX_TITLE_CHARS));
        if let Err(error) = self.inner.state.rename(id, title, true).await { tracing::warn!(%error, "Could not save generated title"); }
        self.broadcast_sessions().await;
    }
}
