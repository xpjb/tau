use crate::settings::SettingsExt;
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
use crate::protocol::{ServerMessage, SessionStatus};
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
            change.events.extend(next.wire.events); change.removed.extend(next.wire.removed); change.delivered.extend(next.wire.delivered);
            change.head = next.head; change.bumps_chat |= next.bumps_chat; entries.push(entry);
        }
        change.queue = queue.clone();
        // A receipt and queue become durable in the same SQLite commit. Publish
        // confirmation immediately; neither a model turn nor the WebSocket
        // response frame is needed to settle a pending edit/send on the client.
        let receipt_id = receipt.as_ref().filter(|r| r.finished).map(|r| r.id.clone());
        let saved = agent.store.commit(id,agent.revision,entries,change.events.clone(),queue,receipt).await?;
        if let Some(receipt_id) = receipt_id && !change.delivered.contains(&receipt_id) {
            change.delivered.push(receipt_id);
        }
        agent.revision = saved.revision; agent.model = saved.model; agent.thinking = saved.thinking;
        agent.tokens = saved.tokens; agent.needs_turn = saved.needs_turn;
        self.publish(id,change)
    }
    pub fn publish(&mut self, _id: &str, change: TranscriptChange) -> Result<()> {
        let transcript = self.transcript.as_mut().unwrap();
        transcript.apply(&change)?;
        Ok(())
    }
    pub async fn save_queue(&mut self, id: &str, queue: QueueState, receipt: Option<Receipt>) -> Result<()> {
        self.commit(id,Vec::new(),Some(queue),receipt).await
    }
    pub async fn live(&mut self, id: &str, stream: &str, message: Value) -> Result<()> {
        let transcript = self.transcript.as_mut().unwrap();
        let mut change = transcript.project(&json!({"streamId":stream,"parentId":transcript.head,"message":message}), true)?;
        if let Some(agent) = &self.agent {
            let values=change.events.iter().map(|event| {
                let append=transcript.event(&event.id).filter(|old|old.kind==event.kind && event.text.starts_with(&old.text)).map(|old|old.text.len());
                (event.clone(),append)
            }).collect();
            agent.store.project_live(id,values,change.removed.clone()).await?;
        }
        // Internal runtime updates also append tool input rather than re-copy it.
        change.events.retain(|event| transcript.event(&event.id).is_none_or(|old| old != event));
        if change.events.len() == 1 && let Some(old) = transcript.event(&change.events[0].id) {
            let event = &change.events[0];
            if old.kind == event.kind && matches!(event.kind, crate::transcript::EventKind::Text | crate::transcript::EventKind::Thinking | crate::transcript::EventKind::Tool)
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
            let usage = manager.context_usage(&settings, &agent.model, agent.tokens);
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
            if !interrupted.events.is_empty() {
                if let Some(agent) = &content.agent { let _ = agent.store.project_live(&id,interrupted.events.iter().cloned().map(|e|(e,None)).collect(),vec![]).await; }
                let _ = content.publish(&id, interrupted);
            }
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
        let mut project_prompt = None;
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
                if project_prompt.is_none() || !entries.is_empty() {
                    project_prompt = Some(self.inner.state.project_prompt(id).await?);
                }
                if !entries.is_empty() {
                    if let Some(control) = &mut queue.control && control.action == "prefix" && control.status == "waiting" { control.status = "applied".into(); queue.paused = true; }
                } else if !content.agent.as_ref().unwrap().needs_turn { return Ok(()); }
                queue.run_id.get_or_insert_with(|| uuid::Uuid::new_v4().to_string());
                // Consuming queued messages and saving their user entries is one transaction.
                content.commit(id,entries,Some(queue),None).await?;
                let agent = content.agent.as_ref().unwrap();
                let entries = agent.store.context(id,&agent.model).await?;
                let mut system = settings.system_prompt(&agent.model, &self.inner.config.cwd).await?;
                if let Some(prompt) = project_prompt.as_ref().filter(|p| !p.is_empty()) { system.push_str("\n\n"); system.push_str(prompt); }
                let messages = history::messages(&entries,system, &agent.model, &self.inner.config.attachment_root).await?;
                (agent.model.clone(), agent.thinking.clone(), agent.cancel.clone(), messages, agent.tokens)
            };
            settings.model(&selected)?;
            let context_window = self.context_window(&settings, &selected);
            let estimated = messages.iter().map(history::estimate_tokens).sum::<u64>();
            if settings.agent.compaction.enabled && context_window.is_some_and(|window| tokens.unwrap_or(estimated).max(estimated) > window.saturating_sub(settings.agent.compaction.reserve_tokens)) {
                if compacted { bail!("Context is still too large after compaction; reduce the queued input or fork an earlier turn"); }
                self.compact_with_project(id, runtime, "", project_prompt.as_deref()).await?;
                compacted = true;
                continue;
            }
            let stream = uuid::Uuid::new_v4().to_string();
            let (updates, mut receiver) = mpsc::channel(8);
            let generation = provider::generate(&self.inner.http, &self.inner.auth, provider::Request {
                settings:&settings, selected:&selected, thinking:&thinking, messages:&messages, session_id:id,
                definitions:tools::definitions(settings.providers[&selected.provider].api != crate::settings::Api::Codex && settings.models.iter().any(|m|settings.providers[&m.provider].api == crate::settings::Api::Codex)), mode:provider::Mode::Chat,
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
                        runtime.content.lock().await.live(id, &stream, assistant_message(&partial, &selected, "", &mut started)).await?;
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
                    Ok(args) if name == "generate_image" => {
                        match self.generate_image(id,&settings,&args,&cancel).await {
                            Ok(staged) => {
                                let path=staged["details"]["tauAttachment"]["path"].as_str().unwrap_or_default();
                                let result=tools::text_result(format!("Generated image delivered to the user. Local reference: {path}"));
                                attachments.push(json!({"type":"tau_attachment","message":{"role":"assistant","content":[{"type":"image","mimeType":"image/png"}],"details":staged["details"],"timestamp":now_ms()}}));
                                Ok(result)
                            }
                            Err(error) => Err(error),
                        }
                    }
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
                    let _ = self.inner.events.send(ServerMessage::Notice { session_id:id.into(), message:result["content"][0]["text"].as_str().unwrap_or("Flag recorded").into() });
                }
                runtime.content.lock().await.append(id, json!({"type":"message","message":result})).await?;
            }
            for attachment in attachments { runtime.content.lock().await.append(id, attachment).await?; }
            self.set_runtime_state(id, runtime, SessionStatus::Running, None, Some(self.context_usage(&self.inner.settings.get(), &selected, completion.tokens)));
        }
    }

    async fn generate_image(&self, id: &str, settings: &crate::settings::Settings, args: &Value, cancel: &CancellationToken) -> Result<Value> {
        use tokio::io::AsyncReadExt;
        use base64::Engine as _;
        let prompt=args["prompt"].as_str().filter(|p|!p.trim().is_empty() && p.chars().count()<=32000).context("Image prompt must contain 1–32000 characters")?;
        let selected=settings.models.iter().find(|m|m.provider == settings.agent.model.provider && m.id == settings.agent.model.model_id && settings.providers[&m.provider].api == crate::settings::Api::Codex)
            .or_else(||settings.models.iter().find(|m|settings.providers[&m.provider].api == crate::settings::Api::Codex)).context("Configure a Codex model/account for image generation")?;
        let selected=SessionModel { provider:selected.provider.clone(),model_id:selected.id.clone() };
        let mut parts=vec![json!({"type":"text","text":prompt})];
        if let Some(paths)=args.get("imagePaths") {
            let paths=paths.as_array().filter(|paths|paths.len()<=4).context("imagePaths must be an array of at most four paths")?;
            for path in paths {
                let path=path.as_str().context("Image path must be text")?;
                let file=crate::attachments::regular_file(&self.inner.config.cwd.join(path)).await?;
                let mut bytes=vec![]; file.take(crate::transcript::IMAGE_LIMIT+1).read_to_end(&mut bytes).await?;
                if bytes.len() as u64>crate::transcript::IMAGE_LIMIT { bail!("Reference image exceeds 10 MB"); }
                let mime=crate::attachments::image_mime(&bytes).filter(|mime|matches!(*mime,"image/png" | "image/jpeg" | "image/webp")).context("Reference must be PNG, JPEG or WebP")?;
                parts.push(json!({"type":"image_url","image_url":{"url":format!("data:{mime};base64,{}",base64::engine::general_purpose::STANDARD.encode(bytes))}}));
            }
        }
        let messages=vec![json!({"role":"system","content":"Generate exactly one image using the image generation tool. Do not call other tools."}),json!({"role":"user","content":parts})];
        let (updates,_receiver)=mpsc::channel(1);
        let result=tokio::select! {
            _=cancel.cancelled() => bail!("Image generation cancelled; do not assume no billing or automatically retry"),
            result=tokio::time::timeout(std::time::Duration::from_secs(600),provider::generate(&self.inner.http,&self.inner.auth,provider::Request {
                settings,selected:&selected,thinking:"minimal",messages:&messages,session_id:id,definitions:vec![],mode:provider::Mode::Image,
            },updates)) => result.context("Image generation timed out; outcome may be unknown, do not automatically retry")?
                .context("Image request failed; billing/outcome may be unknown. Do not automatically retry; ask the user before another generation request")?,
        };
        if result.images.len()!=1 || result.message["tool_calls"].as_array().is_some_and(|calls|!calls.is_empty()) { bail!("Image generation did not return exactly one image; do not automatically retry"); }
        crate::attachments::generated_image(&self.inner.config.attachment_root,&result.images[0].bytes,cancel).await
    }

    pub(crate) async fn compact(&self, id: &str, runtime: &Arc<SessionRuntime>, instructions: &str) -> Result<()> {
        self.compact_with_project(id, runtime, instructions, None).await
    }
    async fn compact_with_project(&self, id: &str, runtime: &Arc<SessionRuntime>, instructions: &str, project: Option<&str>) -> Result<()> {
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
        let mut system = settings.system_prompt(&selected, &self.inner.config.cwd).await?;
        let prompt = match project { Some(prompt) => prompt.to_owned(), None => self.inner.state.project_prompt(id).await? };
        if !prompt.is_empty() { system.push_str("\n\n"); system.push_str(&prompt); }
        let mut messages = history::messages(&prefix,system, &selected, &self.inner.config.attachment_root).await?;
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
        let settings=self.inner.settings.get();
        let generated=if settings.daemon.generate_titles {
            let text=bounded(text,8000);
            let messages=vec![json!({"role":"system","content":"Return only a short literal session title, no quotes or commentary."}),
                json!({"role":"user","content":settings.daemon.title_prompt.replace("{text}",&text)})];
            let (updates,_receiver)=mpsc::channel(1);
            tokio::time::timeout(std::time::Duration::from_secs(30),provider::generate(&self.inner.http,&self.inner.auth,provider::Request {
                settings:&settings,selected:settings.daemon.title_model.as_ref().unwrap_or(&stored.model),thinking:"minimal",messages:&messages,session_id:id,definitions:vec![],mode:provider::Mode::Summary,
            },updates)).await.ok().and_then(Result::ok).and_then(|completion|completion.message["content"].as_str().map(str::to_owned))
                .map(|title|title.trim().to_owned()).filter(|title|title.chars().count()>1 && title.chars().count()<=crate::protocol::MAX_TITLE_CHARS && !title.contains(['\n','\r']))
        } else { None };
        let title=generated.unwrap_or_else(|| bounded(text.lines().find(|line| !line.trim().is_empty()).unwrap_or("Unnamed chat").trim(), crate::protocol::MAX_TITLE_CHARS));
        if let Err(error) = self.inner.state.rename(id, title, true).await { tracing::warn!(%error, "Could not save generated title"); }
        self.broadcast_sessions().await;
    }
}
