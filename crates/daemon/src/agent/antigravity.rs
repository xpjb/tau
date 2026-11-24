use std::{collections::HashMap, ffi::OsString, fmt::Write as _, process::Stdio, sync::Arc, time::Duration};

use anyhow::{Context, Result, bail, ensure};
use futures_util::StreamExt;
use serde_json::{Value, json};
use tokio::{io::AsyncWriteExt, process::{Child, ChildStdin, ChildStdout, Command}, time::{Instant, sleep_until, timeout, timeout_at}};
use tokio_util::{codec::{FramedRead, LinesCodec}, sync::CancellationToken};

use crate::{config::Config, manager::{AgentManager, SessionRuntime, bounded}, settings::{DEFAULT_SYSTEM_PROMPT, Settings}, state::{SessionModel, StateStore}};
use super::now_ms;

pub(crate) const PROVIDER: &str = "antigravity";
const SETUP_TIMEOUT: Duration = Duration::from_secs(20);
const STOP_TIMEOUT: Duration = Duration::from_millis(750);
const FRAME_LIMIT: usize = 8 * 1024 * 1024;
pub(super) const TEXT_LIMIT: usize = 2 * 1024 * 1024;

pub(super) struct Start {
    pub owner: String,
    pub previous: Option<String>,
    pub pending: bool,
    pub prompts: Vec<String>,
}

pub(crate) struct Client {
    _group: super::ProcessGroup,
    _child: Child,
    stdin: ChildStdin,
    stdout: FramedRead<ChildStdout, LinesCodec>,
    next_id: u64,
    resume: bool,
    close: bool,
    idle: Duration,
}

#[derive(Debug)]
struct RpcError { method: &'static str, code: i64, login: bool, missing: bool }
impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.login {
            write!(f, "Antigravity needs sign-in. Run `agy acp --login` as the Tau daemon user with its Antigravity HOME, then resume in Tau.")
        } else {
            write!(f, "Antigravity {} failed (ACP code {}). See the local agy log for details.", self.method, self.code)
        }
    }
}
impl std::error::Error for RpcError {}

impl Client {
    pub(crate) async fn connect(config: &Config, idle: Duration, cancel: &CancellationToken) -> Result<Self> {
        ensure!(!cancel.is_cancelled(), "Antigravity run stopped");
        let bin = config.antigravity_bin.as_ref().context("Set TAU_ANTIGRAVITY_BIN to enable Antigravity on the daemon")?;
        let mut command = Command::new(bin);
        command.args(["acp", "--sandbox", "--disable-telemetry"])
            .current_dir(&config.cwd).env_clear().env("HOME", home(config))
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true);
        // Do not pass Tau's provider keys or other service credentials to the agent.
        for name in ["PATH", "LANG", "LC_ALL", "LC_CTYPE", "SSL_CERT_FILE", "SSL_CERT_DIR", "DBUS_SESSION_BUS_ADDRESS",
            "GEMINI_API_KEY", "GOOGLE_GEMINI_BASE_URL", "AGY_ADC_AUTH", "GOOGLE_APPLICATION_CREDENTIALS",
            "GOOGLE_CLOUD_PROJECT", "GOOGLE_CLOUD_LOCATION", "GOOGLE_CLOUD_QUOTA_PROJECT"] {
            if let Some(value) = std::env::var_os(name) { command.env(name, value); }
        }
        #[cfg(unix)] command.process_group(0);
        let mut child = command.spawn().with_context(|| format!("Could not start Antigravity at {}", bin.display()))?;
        let mut client = Self {
            _group: super::ProcessGroup(child.id()),
            stdin: child.stdin.take().context("Antigravity has no stdin")?,
            stdout: FramedRead::new(child.stdout.take().context("Antigravity has no stdout")?, LinesCodec::new_with_max_length(FRAME_LIMIT)),
            _child: child, next_id: 0, resume: false, close: false, idle,
        };
        let init = client.rpc("initialize", json!({"protocolVersion":1,"clientCapabilities":{},
            "clientInfo":{"name":"tau","version":env!("CARGO_PKG_VERSION")}}), cancel, None).await?;
        ensure!(init["protocolVersion"] == 1, "Antigravity must support ACP version 1");
        let capabilities = &init["agentCapabilities"]["sessionCapabilities"];
        client.resume = capabilities["resume"].is_object();
        client.close = capabilities["close"].is_object();
        Ok(client)
    }

    pub(crate) async fn session(&mut self, config: &Config, previous: Option<&str>, cancel: &CancellationToken) -> Result<(String, Value, bool)> {
        let cwd = config.cwd.to_str().context("Antigravity requires a UTF-8 working directory")?;
        if self.resume && let Some(id) = previous {
            match self.rpc("session/resume", json!({"cwd":cwd,"mcpServers":[],"sessionId":id}), cancel, None).await {
                Ok(options) => return Ok((id.to_owned(), options, true)),
                Err(error) if error.downcast_ref::<RpcError>().is_some_and(|error| error.missing) => {},
                Err(error) => return Err(error),
            }
        }
        let options = self.rpc("session/new", json!({"cwd":cwd,"mcpServers":[]}), cancel, None).await?;
        let id = options["sessionId"].as_str().filter(|id| valid_session(id)).context("Antigravity did not return a valid session ID")?.to_owned();
        Ok((id, options, false))
    }

    pub(crate) async fn close(&mut self, session: &str, cancel: &CancellationToken) -> bool {
        self.close && timeout(Duration::from_secs(2), self.rpc("session/close", json!({"sessionId":session}), cancel, None))
            .await.is_ok_and(|result| result.is_ok())
    }

    async fn send(&mut self, frame: Value, deadline: Instant) -> Result<()> {
        let mut bytes = serde_json::to_vec(&frame)?;
        ensure!(bytes.len() <= FRAME_LIMIT, "Antigravity request exceeds the ACP frame limit");
        bytes.push(b'\n');
        timeout_at(deadline, self.stdin.write_all(&bytes)).await.context("Antigravity stopped reading ACP input")??;
        Ok(())
    }

    async fn rpc(&mut self, method: &'static str, params: Value, cancel: &CancellationToken, mut turn: Option<&mut Turn<'_>>) -> Result<Value> {
        self.next_id += 1;
        let id = self.next_id;
        let session = params["sessionId"].as_str().map(str::to_owned);
        let mut deadline = Instant::now() + if turn.is_some() { self.idle } else { SETUP_TIMEOUT };
        tokio::select! { biased;
            _ = cancel.cancelled() => bail!("Antigravity run stopped"),
            result = self.send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}), deadline) => result?,
        }
        let mut stopping = false;
        let mut wire_bytes = 0usize;
        let mut paint = Instant::now();
        loop {
            let line = tokio::select! { biased;
                _ = cancel.cancelled(), if !stopping => {
                    if turn.is_none() { bail!("Antigravity run stopped"); }
                    stopping = true;
                    deadline = Instant::now() + STOP_TIMEOUT;
                    self.send(json!({"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":session}}), deadline).await?;
                    continue;
                }
                _ = sleep_until(paint), if !stopping && turn.as_ref().is_some_and(|turn| turn.dirty) => {
                    let turn = turn.as_mut().unwrap();
                    turn.runtime.content.lock().await.live(turn.id, &turn.stream, turn.message("")).await?;
                    turn.dirty = false;
                    paint = Instant::now() + Duration::from_millis(50);
                    continue;
                }
                result = timeout_at(deadline, self.stdout.next()) => {
                    result.context(if stopping { "Antigravity did not confirm Stop" } else { "Antigravity stopped responding" })?
                        .context("Antigravity exited before its ACP reply")?.map_err(|_| anyhow::anyhow!("Invalid or oversized Antigravity ACP frame"))?
                }
            };
            wire_bytes += line.len() + 1;
            ensure!(wire_bytes <= 32 * 1024 * 1024, "Antigravity sent too much ACP data");
            let frame: Value = serde_json::from_str(&line).map_err(|_| anyhow::anyhow!("Invalid Antigravity ACP JSON"))?;
            ensure!(frame["jsonrpc"] == "2.0", "Invalid Antigravity ACP version");
            if let Some(incoming) = frame["method"].as_str() {
                if let Some(request_id) = frame.get("id") {
                    let response = match incoming {
                        "session/request_permission" => {
                            let reject = frame.pointer("/params/options").and_then(Value::as_array).into_iter().flatten()
                                .find(|option| option["kind"] == "reject_once").and_then(|option| option["optionId"].as_str());
                            let outcome = match reject.filter(|_| !stopping && !cancel.is_cancelled() && session.is_some()
                                && frame["params"]["sessionId"].as_str() == session.as_deref()) {
                                Some(option) => json!({"outcome":"selected","optionId":option}),
                                None => json!({"outcome":"cancelled"}),
                            };
                            json!({"result":{"outcome":outcome}})
                        }
                        _ => json!({"error":{"code":-32601,"message":"Client method not supported by Tau"}}),
                    };
                    let mut response = response;
                    response["jsonrpc"] = json!("2.0"); response["id"] = request_id.clone();
                    self.send(response, deadline).await?;
                } else if incoming == "session/update" && !stopping && let Some(turn) = &mut turn
                    && frame["params"]["sessionId"].as_str() == session.as_deref() {
                    let update = &frame["params"]["update"];
                    if let Some(text) = update["content"]["text"].as_str().filter(|text| !text.is_empty())
                        && update["content"]["type"] == "text"
                        && let Some(kind) = match update["sessionUpdate"].as_str() {
                            Some("agent_message_chunk") => Some("text"), Some("agent_thought_chunk") => Some("thinking"), _ => None,
                        } {
                        turn.bytes += text.len();
                        ensure!(turn.bytes <= TEXT_LIMIT, "Antigravity reply exceeds 2 MiB");
                        if let Some(block) = turn.blocks.last_mut().filter(|block| block["type"] == kind) {
                            let previous = block[kind].take();
                            if let Value::String(mut previous) = previous { previous.push_str(text); block[kind] = Value::String(previous); }
                        } else {
                            ensure!(turn.blocks.len() < 512, "Antigravity sent too many reply blocks");
                            let mut block = json!({"type":kind,"timestamp":now_ms()});
                            block[kind] = json!(text); turn.blocks.push(block);
                        }
                        turn.dirty = true;
                    } else if matches!(update["sessionUpdate"].as_str(), Some("tool_call" | "tool_call_update"))
                        && let Some(call) = update["toolCallId"].as_str().filter(|call| !call.is_empty() && call.len() <= 240) {
                        let index = if let Some(index) = turn.tools.get(call) { *index } else {
                            ensure!(turn.blocks.len() < 512, "Antigravity sent too many reply blocks");
                            let index = turn.blocks.len(); turn.tools.insert(call.to_owned(), index);
                            turn.blocks.push(json!({"type":"externalTool","id":format!("{}:{call}", turn.stream),"name":"Antigravity","timestamp":now_ms(),"arguments":""}));
                            index
                        };
                        let block = &mut turn.blocks[index];
                        if let Some(title) = update["title"].as_str() { block["arguments"] = json!(bounded(title, 512)); }
                        if let Some(status) = update["status"].as_str().filter(|s| matches!(*s,"pending"|"in_progress"|"completed"|"failed")) { block["name"] = json!(format!("Antigravity · {}", status.replace('_', " "))); }
                        turn.dirty = true;
                    } else if update["sessionUpdate"] == "usage_update" {
                        turn.tokens = update["used"].as_u64();
                    }
                    deadline = Instant::now() + self.idle;
                }
                continue;
            }
            ensure!(frame["id"].as_u64() == Some(id), "Antigravity sent an unexpected reply ID");
            if stopping { bail!("Antigravity run stopped"); }
            if let Some(error) = frame.get("error") {
                let code = error["code"].as_i64().context("Invalid Antigravity ACP error")?;
                let detail = error["message"].as_str().unwrap_or_default();
                return Err(RpcError {method, code, login: code == -32000 && detail == "Authentication required",
                    missing: method == "session/resume" && code == -32603 && detail.starts_with("Failed to read session ")}.into());
            }
            return frame.get("result").cloned().context("Antigravity reply has no result");
        }
    }
}

fn home(config: &Config) -> OsString {
    config.antigravity_home.as_ref().map(|path| path.as_os_str().to_owned())
        .or_else(|| std::env::var_os("HOME")).unwrap_or_else(|| config.cwd.as_os_str().to_owned())
}

fn valid_session(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && id.bytes().all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, b'-' | b'_'))
}

struct Turn<'a> {
    id: &'a str,
    runtime: &'a Arc<SessionRuntime>,
    model: &'a SessionModel,
    stream: String,
    blocks: Vec<Value>,
    tools: HashMap<String, usize>,
    bytes: usize,
    dirty: bool,
    tokens: Option<u64>,
}
impl Turn<'_> {
    fn message(&self, stop: &str) -> Value {
        json!({"role":"assistant","content":self.blocks,"provider":PROVIDER,"model":self.model.model_id,
            "stopReason":stop,"timestamp":now_ms()})
    }
}

// Tau owns queue and transcript commits. This adapter owns exactly one ACP prompt.
pub(super) async fn execute(manager: &AgentManager, id: &str, runtime: &Arc<SessionRuntime>, store: &StateStore,
    settings: &Settings, selected: &SessionModel, thinking: &str, project: &str, start: Start, cancel: &CancellationToken) -> Result<()> {
    let config = &manager.inner.config;
    let mut turn = Turn {id, runtime, model:selected, stream:uuid::Uuid::new_v4().to_string(), blocks:Vec::new(),
        tools:HashMap::new(), bytes:0, dirty:false, tokens:None};
    let mut sent = false;
    let result: Result<(&str, Option<Value>)> = async {
        let system = settings.agent.model_system_prompts.get(&format!("{PROVIDER}/{}", selected.model_id)).map(String::as_str)
            .unwrap_or_else(|| if settings.agent.system_prompt == DEFAULT_SYSTEM_PROMPT { "" } else { &settings.agent.system_prompt });
        let bin = config.antigravity_bin.as_ref().context("Set TAU_ANTIGRAVITY_BIN to enable Antigravity on the daemon")?;
        let cwd = config.cwd.to_str().context("Antigravity requires a UTF-8 working directory")?;
        let scope = blake3::hash(&serde_json::to_vec(&json!([cwd, bin.to_string_lossy(), home(config).to_string_lossy(),
            selected.model_id, thinking, system, project]))?).to_hex().to_string();
        let previous = if !start.pending && let Some(head) = &start.previous {
            let entry = store.entry(id, head).await?;
            let marker = &entry["message"]["tauAntigravitySession"];
            if marker["owner"] == start.owner && marker["scope"] == scope {
                marker["sessionId"].as_str().filter(|id| valid_session(id)).map(str::to_owned)
            } else { None }
        } else { None };
        let mut client = Client::connect(config, Duration::from_secs(settings.agent.http_idle_timeout_seconds), cancel).await?;
        let (session, mut options, resumed) = client.session(config, previous.as_deref(), cancel).await?;
        manager.publish_antigravity_models(&options);
        // A model choice must stick even if agy restores the conversation with its current global default.
        for (category, desired, required) in [("model", selected.model_id.as_str(), selected.model_id != "default"),
            ("thought_level", if thinking == "minimal" {"low"} else {thinking}, false)] {
            if category == "model" && !required { continue; }
            let option = options["configOptions"].as_array().into_iter().flatten().find(|option| option["category"] == category);
            if let Some(option) = option && option["options"].as_array().into_iter().flatten().any(|value| value["value"] == desired) {
                if option["currentValue"] != desired {
                    let config_id = option["id"].as_str().context("Antigravity picker option has no ID")?;
                    options = client.rpc("session/set_config_option", json!({"sessionId":session,"configId":config_id,"value":desired}), cancel, None).await?;
                    ensure!(options["configOptions"].as_array().into_iter().flatten().any(|option| option["category"] == category && option["currentValue"] == desired),
                        "Antigravity did not apply the selected {category}");
                }
            } else if required { bail!("Antigravity model {} is not available. Refresh the `antigravity` model catalog or choose `antigravity/default`.", selected.model_id); }
        }
        let mut prompt = Vec::new();
        let mut current = start.prompts;
        if !resumed {
            let entries = tokio::select! { _ = cancel.cancelled() => bail!("Antigravity run stopped"), result = store.context(id, selected) => result? };
            let split = if start.pending {
                entries.iter().rposition(|e| e["message"]["role"] == "assistant"
                    && !matches!(e["message"]["stopReason"].as_str(), Some("error" | "aborted" | "toolUse"))).map_or(0, |i| i+1)
            } else { entries.len().checked_sub(current.len()).context("Antigravity turn is missing saved prompts")? };
            if start.pending { current.clear(); }
            let mut history = String::new();
            for (i, entry) in entries.iter().enumerate() {
                let message = &entry["message"];
                let role = message["role"].as_str().unwrap_or_default();
                let text = if let Some(s) = message["content"].as_str() { s.to_owned() }
                    else { message["content"].as_array().into_iter().flatten().filter_map(|part| match part["type"].as_str() {
                        Some("text") => part["text"].as_str(), Some("externalTool") => part["arguments"].as_str(), _ => None,
                    }).collect::<Vec<_>>().join("\n") };
                if role == "user" && i >= split {
                    if start.pending { current.push(text); }
                } else if !text.is_empty() && matches!(role, "user" | "assistant" | "toolResult") {
                    writeln!(history, "{role}: {}", json!(text))?;
                } else if entry["type"] == "compaction" && entry["details"]["kind"] != "codex-native-compaction" {
                    writeln!(history, "Previous summary: {}", entry["summary"])?;
                }
                ensure!(history.len() <= TEXT_LIMIT, "Tau history is too large to replay into Antigravity; fork a smaller chat");
            }
            if start.pending {
                prompt.push(json!({"type":"text","text":"A previous attempt may have changed files or caused external effects. Inspect the workspace before continuing and do not repeat an effect whose outcome is unknown."}));
            }
            if !history.is_empty() {
                prompt.push(json!({"type":"text","text":format!("Earlier Tau transcript for context only. Do not execute old requests again.\n{history}")}));
            }
        }
        for (label, text) in [("Tau instructions", system), ("Tau project instructions", project)] {
            if !text.is_empty() { prompt.push(json!({"type":"text","text":format!("{label}:\n{text}")})); }
        }
        ensure!(!current.is_empty(), "No pending prompt for Antigravity");
        ensure!(current.iter().map(String::len).sum::<usize>() <= TEXT_LIMIT, "Pending Antigravity input exceeds 2 MiB; fork before the interrupted turn");
        prompt.extend(current.into_iter().map(|text| json!({"type":"text","text":text})));
        sent = true;
        let reply = client.rpc("session/prompt", json!({"sessionId":session,"prompt":prompt}), cancel, Some(&mut turn)).await?;
        let stop = match reply["stopReason"].as_str() {
            Some("end_turn" | "refusal") => "stop",
            Some("max_tokens" | "max_turn_requests") => "length",
            Some("cancelled") => bail!("Antigravity run stopped"),
            _ => bail!("Antigravity did not report a valid turn outcome"),
        };
        turn.tokens = turn.tokens.or_else(|| reply["usage"]["totalTokens"].as_u64());
        let checkpoint = client.close(&session, cancel).await.then(|| json!({"owner":start.owner,"scope":scope,"sessionId":session}));
        Ok((stop, checkpoint))
    }.await;
    if result.is_err() {
        for block in &mut turn.blocks {
            if block["type"] == "externalTool" && !matches!(block["name"].as_str(), Some("Antigravity · completed" | "Antigravity · failed")) {
                block["name"] = json!("Antigravity · interrupted");
            }
        }
    }
    let mut message = turn.message(result.as_ref().map_or(if cancel.is_cancelled() {"aborted"} else {"error"}, |(stop, _)| *stop));
    if let Ok((_, checkpoint)) = &result {
        if let Some(checkpoint) = checkpoint { message["tauAntigravitySession"] = checkpoint.clone(); }
        if let Some(tokens) = turn.tokens { message["tauContextTokens"] = json!(tokens); }
    }
    let result = result.map_err(|error| if sent {
        anyhow::anyhow!("{error}. Review any changes before resuming; Tau did not retry this turn.")
    } else { error });
    if let Err(error) = &result { message["errorMessage"] = json!(bounded(&error.to_string(), 4096)); }
    runtime.content.lock().await.append(id, json!({"type":"message","origin":{"streamId":turn.stream},"message":message})).await?;
    result.map(|_| ())
}
