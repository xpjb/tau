// SSE framing and provider decoding adapted from the user's glmbot.
use std::collections::BTreeMap;
use anyhow::{bail, Context as _, Result};
use serde_json::{json, Value};
use futures_util::StreamExt;
use tokio::sync::mpsc;
use std::time::Duration;

use crate::settings::{Api, ModelSettings, Settings};
use crate::state::SessionModel;
use super::auth::AuthStore;
mod codex;
mod completions;
const MAX_RESPONSE_BYTES: usize = 32 * 1024 * 1024;
trait Decoder: Send + Sync {
    fn response_limit(&self) -> usize { MAX_RESPONSE_BYTES }
    fn decode(&self, event: &Value, stream: &mut Stream<'_>) -> Result<()>;
}
pub struct Stream<'a> {
    decoder: &'a dyn Decoder,
    pub wire_bytes: usize,
    buffer: Vec<u8>,
    pub id: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub message: serde_json::Map<String, Value>,
    pub tool_calls: BTreeMap<usize, serde_json::Map<String, Value>>,
    pub output_items: BTreeMap<usize, Option<Value>>,
    pub finish_reason: Option<String>,
    pub usage: Option<Value>,
    pub error: Option<Value>,
    pub done: bool,
    pub data_events: u64,
    pub progress_events: u64,
    pub heartbeats: u64,
}

impl<'a> Stream<'a> {
    fn new(decoder: &'a dyn Decoder) -> Self {
        Self {
            decoder,
            wire_bytes: 0,
            buffer: Vec::new(),
            id: None,
            provider: None,
            model: None,
            message: serde_json::Map::new(),
            tool_calls: BTreeMap::new(),
            output_items: BTreeMap::new(),
            finish_reason: None,
            usage: None,
            error: None,
            done: false,
            data_events: 0,
            progress_events: 0,
            heartbeats: 0,
        }
    }

    pub fn push(&mut self, bytes: &[u8]) -> Result<()> {
        if self.done {
            return Ok(());
        }
        self.wire_bytes = self.wire_bytes.saturating_add(bytes.len());
        let limit = self.decoder.response_limit();
        if self.wire_bytes > limit {
            bail!("model response exceeded {limit} bytes");
        }
        let mut scan_from = self.buffer.len().saturating_sub(3);
        self.buffer.extend_from_slice(bytes);
        let mut buffer = std::mem::take(&mut self.buffer);
        let mut consumed = 0;
        loop {
            let mut boundary = None;
            let mut index = scan_from;
            while index < buffer.len() {
                let first = match buffer[index] {
                    b'\r' if buffer.get(index + 1) == Some(&b'\n') => 2,
                    b'\r' | b'\n' => 1,
                    _ => {
                        index += 1;
                        continue;
                    }
                };
                let next = index + first;
                let second = match buffer.get(next) {
                    Some(b'\r') if buffer.get(next + 1) == Some(&b'\n') => 2,
                    Some(b'\r' | b'\n') => 1,
                    _ => {
                        index = next;
                        continue;
                    }
                };
                boundary = Some((index, first + second));
                break;
            }
            let Some((event_end, separator_len)) = boundary else {
                buffer.drain(..consumed);
                self.buffer = buffer;
                return Ok(());
            };
            self.consume_event(&buffer[consumed..event_end])?;
            if self.done || self.error.is_some() {
                return Ok(());
            }
            consumed = event_end + separator_len;
            scan_from = consumed;
        }
    }

    pub fn finish_input(&mut self) -> Result<()> {
        if self.buffer.iter().all(u8::is_ascii_whitespace) {
            self.buffer.clear();
            return Ok(());
        }
        let event = std::mem::take(&mut self.buffer);
        self.consume_event(&event)
    }

    fn consume_event(&mut self, event: &[u8]) -> Result<()> {
        let event = std::str::from_utf8(event).context("model SSE was not valid UTF-8")?;
        let mut data = String::new();
        let mut comment = false;
        for line in event.split(['\r', '\n']) {
            if let Some(value) = line.strip_prefix("data:") {
                if !data.is_empty() {
                    data.push('\n');
                }
                data.push_str(value.strip_prefix(' ').unwrap_or(value));
            } else if line.starts_with(':') {
                comment = true;
            }
        }
        if data.is_empty() {
            if comment {
                self.heartbeats += 1;
            }
            return Ok(());
        }
        if data == "[DONE]" {
            return self.decoder.decode(&Value::String("[DONE]".into()), self);
        }

        let event: Value = serde_json::from_str(&data)
            .context("model SSE event was not valid JSON")?;
        self.data_events += 1;
        self.decoder.decode(&event, self)
    }

    pub fn assistant_message(&self) -> Value {
        let mut message = self.message.clone();
        message
            .entry("role")
            .or_insert_with(|| Value::String("assistant".into()));
        message.entry("content").or_insert(Value::Null);
        if !self.tool_calls.is_empty() {
            message.insert(
                "tool_calls".into(),
                Value::Array(
                    self.tool_calls
                        .values()
                        .cloned()
                        .map(Value::Object)
                        .collect(),
                ),
            );
        }
        Value::Object(message)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode { Chat, Compact, Search, Summary }
pub struct Completion { pub message: Value, pub tokens: Option<u64>, pub account: Option<String> }

pub async fn generate(
    http: &reqwest::Client, auth: &AuthStore, settings: &Settings, selected: &SessionModel, thinking: &str,
    messages: &[Value], session_id: &str, definitions: Vec<Value>, mode: Mode, updates: mpsc::Sender<Value>,
) -> Result<Completion> {
    let model: &ModelSettings = settings.model(selected)?;
    let provider = &settings.providers[&selected.provider];
    let decoder: &dyn Decoder = match provider.api { Api::Codex => &codex::Codex, Api::ChatCompletions => &completions::Completions };
    let effort = model.thinking_level_map.get(thinking).cloned().unwrap_or_else(|| match thinking {
        "off" => None, "minimal" => Some("low".into()), other => Some(other.into()),
    });
    let idle = Duration::from_secs(settings.agent.http_idle_timeout_seconds);
    let attempts = if settings.agent.retry.enabled { settings.agent.retry.max_retries } else { 0 };
    let mut rejected = None;
    let mut refreshed = false;
    let mut attempt = 0;
    loop {
        let (key, account) = auth.authorization(&selected.provider, provider.api_key_env.as_deref(), rejected.as_deref()).await?;
        let (route, mut body) = match provider.api {
            Api::ChatCompletions => {
                if mode == Mode::Compact { bail!("Native compaction requires Codex"); }
                let mut tools = definitions.iter().map(|tool| json!({"type":"function", "function":tool})).collect::<Vec<_>>();
                if mode != Mode::Summary && provider.web_search && selected.provider == "openrouter" {
                    tools.push(json!({"type":"openrouter:web_search", "parameters":{"engine":"exa", "max_results":5}}));
                }
                let mut body = json!({"model":model.id, "messages":messages.iter().map(|message| { let mut message = message.clone(); if let Some(map) = message.as_object_mut() { map.remove("usage"); map.remove("codex_output"); map.remove("annotations"); } message }).collect::<Vec<_>>(), "stream":true, "stream_options":{"include_usage":true}});
                if !tools.is_empty() { body["tools"] = json!(tools); }
                if let Some(effort) = &effort { body["reasoning"] = json!({"effort":effort}); }
                ("chat/completions", body)
            }
            Api::Codex => {
                let mut input = Vec::new(); let mut instructions = Vec::new();
                for (index, message) in messages.iter().enumerate() {
                    match message["role"].as_str() {
                        Some("system") => instructions.push(message["content"].as_str().context("System prompt is not text")?),
                        Some("checkpoint") => {
                            if message["accountId"].as_str() != account.as_deref() || message["model"].as_str() != Some(&model.id)
                                || message["baseUrl"].as_str().map(|url| url.trim_end_matches('/').trim_end_matches("/codex"))
                                    != Some(provider.base_url.trim_end_matches('/').trim_end_matches("/codex")) {
                                bail!("Native checkpoint belongs to a different account, model or endpoint; switch back or fork before compaction");
                            }
                            input.push(message["item"].clone());
                        }
                        Some("user") => {
                            let content = if let Some(text) = message["content"].as_str() { vec![json!({"type":"input_text", "text":text})] }
                                else { message["content"].as_array().context("Invalid user parts")?.iter().map(|part| match part["type"].as_str() {
                                    Some("image_url") => json!({"type":"input_image", "image_url":part["image_url"]["url"], "detail":"auto"}),
                                    _ => json!({"type":"input_text", "text":part["text"]}),
                                }).collect() };
                            input.push(json!({"role":"user", "content":content}));
                        }
                        Some("assistant") => {
                            if let Some(items) = message["codex_output"].as_array() { input.extend(items.iter().cloned()); }
                            else {
                                if let Some(text) = message["content"].as_str().filter(|text| !text.is_empty()) {
                                    input.push(json!({"type":"message", "role":"assistant", "status":"completed", "id":format!("msg_tau_history_{index}"),
                                        "content":[{"type":"output_text", "text":text, "annotations":[]}]}));
                                }
                                for call in message["tool_calls"].as_array().into_iter().flatten() {
                                    input.push(json!({"type":"function_call", "call_id":call["id"], "name":call["function"]["name"], "arguments":call["function"]["arguments"]}));
                                }
                            }
                        }
                        Some("tool") => input.push(json!({"type":"function_call_output", "call_id":message["tool_call_id"], "output":message["content"]})),
                        _ => bail!("Unsupported conversation role"),
                    }
                }
                if mode == Mode::Compact { input.push(json!({"type":"compaction_trigger"})); }
                let mut tools = definitions.iter().map(|tool| { let mut tool = tool.clone(); tool["type"] = json!("function"); tool["strict"] = json!(false); tool }).collect::<Vec<_>>();
                if mode != Mode::Summary && provider.web_search { tools.push(json!({"type":"web_search", "search_context_size":"medium"})); }
                let mut body = json!({"model":model.id, "store":false, "stream":true, "instructions":instructions.join("\n\n"), "input":input,
                    "tools":tools, "tool_choice":if mode == Mode::Search { json!({"type":"web_search"}) } else { json!("auto") },
                    "parallel_tool_calls":true, "include":["reasoning.encrypted_content"], "prompt_cache_key":session_id, "text":{"verbosity":"low"}});
                if let Some(effort) = &effort { body["reasoning"] = json!({"effort":effort, "summary":"auto"}); }
                if settings.agent.fast_mode { body["service_tier"] = json!("priority"); }
                ("responses", body)
            }
        };
        if mode == Mode::Search && provider.api != Api::Codex { body["tools"] = json!([{"type":"openrouter:web_search", "parameters":{"max_results":5}}]); }
        let mut request = http.post(format!("{}/{route}", provider.base_url.trim_end_matches('/'))).bearer_auth(&key)
            .header("Accept", "text/event-stream").header("User-Agent", concat!("Tau/", env!("CARGO_PKG_VERSION")));
        if provider.api == Api::Codex {
            request = request.header("chatgpt-account-id", account.as_deref().context("Codex requires an account ID")?)
                .header("originator", "tau").header("OpenAI-Beta", "responses=experimental")
                .header("session-id", session_id).header("x-codex-beta-features", "remote_compaction_v2");
            if settings.agent.fast_mode { request = request.header("x-codex-routing-hint", format!("model={};tier=priority", model.id)); }
        }
        let response = tokio::time::timeout(idle, request.json(&body).send()).await.context("Model response headers timed out")?;
        let response = match response {
            Ok(response) => response,
            Err(error) if attempt >= attempts => return Err(error).context("Model request failed"),
            Err(_) => { attempt += 1; tokio::time::sleep(Duration::from_millis(settings.agent.retry.base_delay_ms.saturating_mul(1 << (attempt - 1)))).await; continue; }
        };
        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED && provider.api == Api::Codex && !refreshed {
            rejected = Some(key); refreshed = true; continue;
        }
        if (status.as_u16() == 429 || status.is_server_error()) && attempt < attempts {
            let delay = response.headers().get("retry-after").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok())
                .map(|s| s.saturating_mul(1000)).unwrap_or_else(|| settings.agent.retry.base_delay_ms.saturating_mul(1 << attempt)).min(60_000);
            attempt += 1; tokio::time::sleep(Duration::from_millis(delay)).await; continue;
        }
        if !status.is_success() { bail!("Model provider returned HTTP {}", status.as_u16()); }
        let mut chunks = response.bytes_stream();
        let mut stream = Stream::new(decoder);
        while let Some(chunk) = tokio::time::timeout(idle, chunks.next()).await.context("Model stream stalled")? {
            stream.push(&chunk.context("Model stream disconnected")?)?;
            if stream.error.is_some() { bail!("Model provider reported a stream error"); }
            if mode == Mode::Chat { updates.send(stream.assistant_message()).await.context("Agent stopped receiving model output")?; }
            if stream.done { break; }
        }
        stream.finish_input()?;
        if !stream.done || stream.error.is_some() || !matches!(stream.finish_reason.as_deref(), Some("stop" | "tool_calls" | "length")) {
            bail!("Model response was incomplete; tools were not executed");
        }
        let mut message = stream.assistant_message();
        message["usage"] = json!(stream.usage);
        let mut ids = std::collections::HashSet::new();
        for call in message["tool_calls"].as_array().into_iter().flatten() {
            let id = call["id"].as_str().filter(|id| !id.is_empty()).context("Tool call has no ID")?;
            if !ids.insert(id) || call["function"]["name"].as_str().is_none() || call["function"]["arguments"].as_str().is_none() { bail!("Invalid tool calls"); }
        }
        return Ok(Completion { message, tokens: stream.usage.as_ref().and_then(|usage| usage["total_tokens"].as_u64()), account });
    }
}
