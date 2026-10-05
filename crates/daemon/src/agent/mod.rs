use crate::settings::SettingsExt;
pub mod auth;
mod auth_lock;
pub mod history;
mod provider;
mod tools;

use std::sync::Arc;
use std::collections::HashMap;
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use crate::manager::{AgentManager, PromptOutcome, SessionContent, SessionRuntime, bounded};
use std::collections::BTreeMap;
use tau_net::{PromptDisposition, ModelSuggestion};
use tau_net::{ServerMessage, SessionStatus};
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
    pub resume_after_stop: bool,
    pub cancel: CancellationToken,
    pub task: Option<tokio::task::JoinHandle<()>>,
    pub tokens: Option<u64>,
    pub needs_turn: bool,
}
impl AgentSession {
    pub fn stop(&mut self) {
        self.resume_after_stop = false;
        self.cancel.cancel();
    }
}
pub fn now_ms() -> u64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis().try_into().unwrap_or(u64::MAX) }

impl SessionContent {
    pub async fn append(&mut self, id: &str, value: Value) -> Result<()> { self.commit(id, vec![value], None, None).await }
    pub async fn commit(&mut self, id: &str, values: Vec<Value>, queue: Option<QueueState>, receipt: Option<Receipt>) -> Result<()> {
        // Acceptance bumps once, not again when a queued user entry is consumed.
        let bump = receipt.as_ref().is_some_and(|r| r.command.as_deref().is_none_or(|kind| kind == "builtin"));
        self.commit_with_activity(id, values, queue, receipt, bump).await
    }
    async fn commit_with_activity(&mut self, id: &str, values: Vec<Value>, queue: Option<QueueState>, receipt: Option<Receipt>, bump: bool) -> Result<()> {
        let agent = self.agent.as_mut().context("Session is not loaded")?;
        let mut projected = self.transcript.as_ref().unwrap().clone();
        let mut change = TranscriptChange::default(); let mut entries = Vec::new();
        for mut entry in values {
            entry["id"] = json!(uuid::Uuid::new_v4().to_string()); entry["parentId"] = json!(projected.head);
            entry["timestamp"] = json!(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis,true));
            let next = projected.project(&entry,false)?; projected.apply(&next)?;
            change.events.extend(next.events); change.removed.extend(next.removed);
            change.head = next.head; entries.push(entry);
        }
        change.queue = queue.clone();
        let saved = agent.store.commit(id,agent.revision,entries,change.events.clone(),queue,receipt,bump).await?;
        agent.revision = saved.revision; agent.model = saved.model; agent.thinking = saved.thinking;
        agent.tokens = saved.tokens; agent.needs_turn = saved.needs_turn;
        self.transcript.as_mut().unwrap().apply(&change)
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
        self.transcript.as_mut().unwrap().apply(&change)
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
        agent.running = true; agent.resume_after_stop = false; agent.cancel = CancellationToken::new();
        let manager = self.clone(); let id = id.to_owned(); let runtime = runtime.clone();
        let cancel=agent.cancel.clone();
        self.set_runtime_state(&id, &runtime, SessionStatus::Running, Some("Waiting for agent capacity".into()), None);
        agent.task = Some(tokio::spawn(async move {
            let _admission=tokio::select! {_=cancel.cancelled()=>None,permit=manager.inner.agent_runs.acquire()=>permit.ok()};
            if !cancel.is_cancelled() {manager.set_runtime_state(&id,&runtime,SessionStatus::Running,None,None);}
            loop {
            let mut result = manager.run_agent(&id, &runtime).await;
            #[cfg(test)] {
                let gate = manager.inner.settle_gate.lock().unwrap().take();
                if let Some(gate) = gate { gate.notified().await; }
            }
            let mut content = runtime.content.lock().await;
            let Some(agent) = &mut content.agent else { return; };
            let cancelled = agent.cancel.is_cancelled();
            if result.is_ok() && !cancelled && content.transcript.as_ref().is_some_and(|t| !t.queue.paused && !t.queue.requests.is_empty()) { drop(content); continue; }
            let agent = content.agent.as_mut().unwrap();
            // Resume may arrive after Abort has cancelled the task but before
            // its cleanup completes. The old task must not overwrite that newer
            // explicit intent, nor start the replacement before tools stop.
            let mut resume = cancelled && agent.resume_after_stop;
            agent.running = false; agent.resume_after_stop = false;
            let settings = manager.inner.settings.get();
            let usage = manager.context_usage(&settings, &agent.model, agent.tokens);
            let mut queue = content.transcript.as_ref().unwrap().queue.clone();
            queue.run_id = None;
            if (cancelled || result.is_err()) && !resume { queue.paused = true; }
            if !resume && let Some(control) = &mut queue.control && control.status == "waiting" {
                // A boundary action cannot remain pending after its owner ends.
                // Preserve the outcome without automatically retrying failed work.
                control.status = "failed".into();
                control.detail = Some(if cancelled { "Interrupted before the queue boundary".into() }
                    else { result.as_ref().err().map_or_else(|| "Run ended before the queue boundary".into(), |e| bounded(&e.to_string(), 480)) });
            }
            // A run gets one completion bump, including an error/abort that
            // needs attention. Streaming, tools and queued continuations do not.
            if let Err(error) = content.commit_with_activity(&id, Vec::new(), Some(queue), None, true).await {
                tracing::error!(session=%id, %error, "Could not save settled queue");
                resume = false;
                result = Err(error.context("Could not save settled queue"));
            }
            let mut interrupted = TranscriptChange::default();
            for event in content.transcript.as_mut().unwrap().events_mut().filter(|event| event.phase == crate::transcript::EventPhase::Live) {
                let mut event = event.clone(); event.phase = crate::transcript::EventPhase::Interrupted;
                interrupted.events.push(event);
            }
            if !interrupted.events.is_empty() {
                if let Some(agent) = &content.agent { let _ = agent.store.project_live(&id,interrupted.events.iter().cloned().map(|e|(e,None)).collect(),vec![]).await; }
                let _ = content.transcript.as_mut().unwrap().apply(&interrupted);
            }
            if let Err(error) = &result && !cancelled {
                tracing::warn!(session=%id, error=%bounded(&error.to_string(),480), "Agent run failed; waiting for explicit resume");
            }
            manager.set_runtime_state(&id, &runtime, if result.is_err() && !cancelled { SessionStatus::Error } else { SessionStatus::Idle },
                result.err().map(|e| bounded(&e.to_string(), 480)), Some(usage));
            if resume { manager.start_run(&id, &runtime, &mut content); }
            drop(content);
            manager.broadcast_sessions().await;
            break;
            }
        }));
    }

    async fn run_agent(&self, id: &str, runtime: &Arc<SessionRuntime>) -> Result<()> {
        let mut compacted = false;
        let mut project_prompt = None;
        let mut stream_retries = 0u32;
        loop {
            let settings = self.inner.settings.get();
            let (selected, thinking, cancel, store, tokens) = {
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
                (agent.model.clone(),agent.thinking.clone(),agent.cancel.clone(),agent.store.clone(),agent.tokens)
            };
            self.schedule_catalog(&selected);
            // Preparing a provider request cannot own the queue/content mutex.
            // New intents and aborts remain independently durable while this runs.
            let entries=tokio::select! {_=cancel.cancelled()=>return Ok(()),result=store.context(id,&selected)=>result?};
            let mut system = settings.system_prompt(&selected, &self.inner.config.cwd).await?;
            if let Some(prompt) = project_prompt.as_ref().filter(|p| !p.is_empty()) {system.push_str("\n\n");system.push_str(prompt);}
            let messages = tokio::select! {
                _=cancel.cancelled()=>return Ok(()),
                result=history::messages(&entries,system,&selected,&self.inner.config.attachment_root)=>result?,
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
            let recovery = provider::Recovery::default();
            let generation = provider::generate(&self.inner.http, &self.inner.auth, provider::Request {
                settings:&settings, selected:&selected, thinking:&thinking, messages:&messages, session_id:id,
                definitions:tools::definitions(settings.providers[&selected.provider].api != crate::settings::Api::Codex && settings.models.iter().any(|m|settings.providers[&m.provider].api == crate::settings::Api::Codex)), mode:provider::Mode::Chat, recovery:Some(&recovery),
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
                    // A new signed checkpoint advances the run. Repeated failures
                    // without progress consume the configured retry budget instead.
                    if recovery.advanced() { stream_retries = 0; }
                    let retry = !cancel.is_cancelled() && settings.agent.retry.enabled
                        && error.is::<provider::StreamFailure>() && recovery.retry_safe()
                        && stream_retries < settings.agent.retry.max_retries;
                    let mut content = runtime.content.lock().await;
                    let mut message = assistant_message(&partial, &selected, if cancel.is_cancelled() {"aborted"} else {"error"}, &mut started);
                    if !retry { message["errorMessage"] = json!(bounded(&error.to_string(), 4096)); }
                    if let Some(checkpoint) = recovery.snapshot() { message["tauReasoningRecovery"] = checkpoint; }
                    message["timestamp"] = json!(now_ms());
                    content.append(id, json!({"type":"message","origin":{"streamId":stream},"message":message})).await?;
                    content.agent.as_mut().unwrap().needs_turn = true;
                    if error.is::<auth::SignInRequired>() {
                        let _ = self.inner.events.send(tau_net::ServerMessage::CodexLoginRequired { session_id:id.into() });
                    }
                    drop(content);
                    if retry {
                        stream_retries += 1;
                        tracing::info!(session=%id, attempt=stream_retries, checkpoint_progress=recovery.advanced(), "Continuing after a transient model stream failure");
                        let delay = settings.agent.retry.base_delay_ms.saturating_mul(1u64 << (stream_retries - 1).min(16)).min(60_000);
                        tokio::select! { _=cancel.cancelled()=>return Ok(()), _=tokio::time::sleep(std::time::Duration::from_millis(delay))=>{} }
                        continue;
                    }
                    return Err(error);
                }
            };
            compacted = false;
            stream_retries = 0;
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
                                settings:&settings, selected:&selected, thinking:"low", messages:&messages, session_id:id, definitions:vec![], mode:provider::Mode::Search, recovery:None,
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
                settings,selected:&selected,thinking:"minimal",messages:&messages,session_id:id,definitions:vec![],mode:provider::Mode::Image,recovery:None,
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
        let (selected,thinking,cancel,store,before_tokens)={
            let content=runtime.content.lock().await;let agent=content.agent.as_ref().unwrap();
            (agent.model.clone(),agent.thinking.clone(),agent.cancel.clone(),agent.store.clone(),agent.tokens)
        };
        let entries=tokio::select! {_=cancel.cancelled()=>bail!("Compaction cancelled"),result=store.context(id,&selected)=>result?};
        let (first_kept,prefix)={
            let cut = history::compaction_cut(&entries, settings.agent.compaction.keep_recent_tokens)?;
            let mut prefix = entries[..cut].to_vec();
            // An earlier checkpoint can sit after its retained boundary in append order.
            prefix.extend(entries[cut..].iter().filter(|entry| entry["type"] == "compaction").cloned());
            // A rejection after the retained boundary also invalidates a
            // recovered item in the prefix being compacted. Carry only that
            // exclusion, never the suffix's new reasoning or user messages.
            for entry in &entries[cut..] {
                if let Some(recovery) = entry.pointer("/message/tauReasoningRecovery")
                    && recovery["rejectedIds"].as_array().is_some_and(|ids| !ids.is_empty()) {
                    let mut marker = recovery.clone(); marker["items"] = json!([]);
                    prefix.push(json!({"type":"message","message":{"role":"assistant","stopReason":"error","tauReasoningRecovery":marker}}));
                }
            }
            (entries[cut]["id"].clone(),prefix)
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
            mode:if native {provider::Mode::Compact} else {provider::Mode::Summary}, recovery:None,
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
        if stored.title != "New chat" { return; }
        let settings=self.inner.settings.get();
        let permit=self.inner.title_requests.try_acquire();
        let generated=if settings.daemon.generate_titles && permit.is_ok() {
            let text=bounded(text,8000);
            let messages=vec![json!({"role":"system","content":"Return only a short literal session title, no quotes or commentary."}),
                json!({"role":"user","content":settings.daemon.title_prompt.replace("{text}",&text)})];
            let (updates,_receiver)=mpsc::channel(1);
            tokio::time::timeout(std::time::Duration::from_secs(30),provider::generate(&self.inner.http,&self.inner.auth,provider::Request {
                settings:&settings,selected:settings.daemon.title_model.as_ref().unwrap_or(&stored.model),thinking:"minimal",messages:&messages,session_id:id,definitions:vec![],mode:provider::Mode::Summary,recovery:None,
            },updates)).await.ok().and_then(Result::ok).and_then(|completion|completion.message["content"].as_str().map(str::to_owned))
                .map(|title|title.trim().to_owned()).filter(|title|title.chars().count()>1 && title.chars().count()<=tau_net::MAX_TITLE_CHARS && !title.contains(['\n','\r']))
        } else { None };
        let title=generated.unwrap_or_else(|| bounded(text.lines().find(|line| !line.trim().is_empty()).unwrap_or("Unnamed chat").trim(), tau_net::MAX_TITLE_CHARS));
        if let Err(error) = self.inner.state.rename(id, title, true).await { tracing::warn!(%error, "Could not save generated title"); }
        self.broadcast_sessions().await;
    }
}

impl AgentManager {
    pub fn model_catalog(&self) -> tau_net::ModelCatalog {
        let revision = self.inner.state_clock.fetch_add(1, std::sync::atomic::Ordering::AcqRel) + 1;
        let settings = self.inner.settings.get();
        let mut models = settings.models.iter().map(|m| (format!("{}/{}",m.provider,m.id), Some(m.name.clone())))
            .collect::<BTreeMap<_,_>>();
        let mut unresolved_providers = Vec::new();
        for provider in settings.providers.keys() {
            let Some(available) = self.inner.catalog.available_models(&settings, provider) else {
                unresolved_providers.push(provider.clone()); continue;
            };
            for (id, window) in available {
                models.insert(format!("{provider}/{id}"), Some(format!("{} token context", window)));
            }
        }
        tau_net::ModelCatalog {
            revision, unresolved_providers,
            default_model: Some(settings.agent.model.clone()),
            models: models.into_iter().take(20_000).map(|(value, description)| ModelSuggestion { value, description }).collect(),
        }
    }
    pub(crate) async fn run_builtin_command(&self, id: &str, runtime: &Arc<SessionRuntime>, name: &str, arguments: &str) -> Result<PromptOutcome> {
        let mut content = runtime.content.lock().await;
        if content.agent.as_ref().unwrap().running && matches!(name, "model" | "thinking" | "compact") { bail!("Stop the current run before changing /{name}"); }
        let notice = match name {
            "model" => {
                let model: SessionModel = arguments.parse().map_err(anyhow::Error::msg)?;
                let mut settings = self.inner.settings.get(); settings.model(&model)?;
                settings.agent.model = model.clone();
                self.set_settings(settings.revision, settings).await?;
                let settings = self.inner.settings.get();
                let level = settings.agent.model_thinking_levels.get(arguments).unwrap_or(&settings.agent.thinking_level).clone();
                content.append(id,json!({"type":"model_change","provider":model.provider,"modelId":model.model_id,"thinkingLevel":level})).await?;
                self.schedule_catalog(&model);
                let usage = self.context_usage(&settings, &model, None);
                self.set_runtime_state(id, runtime, SessionStatus::Idle, None, Some(usage));
                format!("Model set to {arguments}. New chats will use it too.")
            }
            "thinking" => {
                if !crate::settings::LEVELS.contains(&arguments) { bail!("Usage: /thinking <off|minimal|low|medium|high|xhigh|max>"); }
                content.append(id, json!({"type":"thinking_level_change","thinkingLevel":arguments})).await?;
                content.agent.as_mut().unwrap().thinking = arguments.into(); format!("Thinking level set to {arguments}.")
            }
            "name" => {
                if arguments.trim().is_empty() || arguments.chars().count() > tau_net::MAX_TITLE_CHARS || arguments.contains(['\n','\r']) { bail!("Usage: /name <title>"); }
                self.inner.state.rename(id, arguments.into(), false).await?; format!("Chat renamed to {arguments}.")
            }
            "fast" => {
                let mut settings = self.inner.settings.get();
                match arguments { "on" => settings.agent.fast_mode = true, "off" => settings.agent.fast_mode = false, "status" => {}, _ => bail!("Usage: /fast <on|off|status>") }
                if arguments != "status" { settings = self.set_settings(settings.revision, settings).await?; }
                format!("Codex priority service is {}.", if settings.agent.fast_mode { "on" } else { "off" })
            }
            "compact" => {
                let agent = content.agent.as_mut().unwrap(); agent.running = true; agent.cancel = tokio_util::sync::CancellationToken::new();
                drop(content);
                let result = self.compact(id, runtime, arguments).await;
                content = runtime.content.lock().await;
                content.agent.as_mut().unwrap().running = false;
                self.set_runtime_state(id, runtime, SessionStatus::Idle, None, Some(None));
                result?; "Context compacted.".into()
            }
            _ => bail!("Unknown command /{name}"),
        };
        drop(content); self.broadcast_sessions().await;
        Ok(PromptOutcome { disposition:PromptDisposition::Handled, notice:Some(notice) })
    }
}
