use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;
use crate::settings::Settings;
use crate::state::SessionModel;
use crate::transcript::{QueueState, Transcript};

pub type Receipts = HashMap<String, (String, crate::protocol::PromptDisposition)>;

pub struct Journal { pub path: PathBuf, pub entries: Vec<Value> }
impl Journal {
    pub async fn open(root: &Path, path: Option<&str>, settings: &Settings) -> Result<Self> {
        tokio::fs::create_dir_all(root).await?;
        let path = if let Some(path) = path {
            let root = tokio::fs::canonicalize(root).await?;
            let path = tokio::fs::canonicalize(path).await.context("Session history is unavailable")?;
            if !path.starts_with(root) || !tokio::fs::metadata(&path).await?.is_file() { bail!("Session history is outside the session directory or is not a file"); }
            path
        } else {
            let path = root.join(format!("{}.jsonl", uuid::Uuid::new_v4()));
            let header = json!({"type":"session","version":3,"id":uuid::Uuid::new_v4().to_string(),"tauVersion":1});
            crate::settings::atomic_write(&path, format!("{header}\n").as_bytes()).await?;
            path
        };
        let bytes = tokio::fs::read(&path).await?;
        let mut records = Vec::new();
        for (index, line) in bytes.split(|byte| *byte == b'\n').enumerate() {
            if line.iter().all(u8::is_ascii_whitespace) { continue; }
            match serde_json::from_slice::<Value>(line) {
                Ok(entry) if entry["type"] == "session" => {},
                Ok(entry) if entry["id"].as_str().is_some() => records.push(entry),
                _ => tracing::warn!(line = index + 1, "Skipping malformed history record"),
            }
        }
        // Repair only a torn final line before appending, never concatenate new JSON to it.
        if !bytes.is_empty() && bytes.last() != Some(&b'\n') {
            let tail = bytes.rsplit(|byte| *byte == b'\n').next().unwrap();
            let mut file = tokio::fs::OpenOptions::new().write(true).append(true).open(&path).await?;
            if serde_json::from_slice::<Value>(tail).is_ok() { file.write_all(b"\n").await?; }
            else { file.set_len((bytes.len() - tail.len()) as u64).await?; }
            file.sync_all().await?;
        }
        let by_id = records.iter().map(|entry| (entry["id"].as_str().unwrap(), entry)).collect::<HashMap<_, _>>();
        if by_id.len() != records.len() { bail!("Duplicate history entry IDs"); }
        let mut entries = Vec::new(); let mut seen = HashSet::new();
        let mut cursor = records.last().and_then(|entry| entry["id"].as_str());
        while let Some(id) = cursor {
            if !seen.insert(id) { bail!("History branch contains a cycle"); }
            let Some(entry) = by_id.get(id) else { break; };
            entries.push((*entry).clone()); cursor = entry["parentId"].as_str();
        }
        entries.reverse();
        let mut journal = Self { path, entries };
        if journal.entries.is_empty() {
            let entry = journal.entry(json!({"type":"model_change","provider":settings.agent.model.provider,"modelId":settings.agent.model.model_id}));
            journal.append(entry).await?;
        }
        Ok(journal)
    }
    pub fn head(&self) -> Option<String> { self.entries.last().and_then(|entry| entry["id"].as_str()).map(str::to_owned) }
    pub fn entry(&self, mut value: Value) -> Value {
        value["id"] = json!(uuid::Uuid::new_v4().to_string()); value["parentId"] = json!(self.head());
        value["timestamp"] = json!(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)); value
    }
    pub async fn append(&mut self, entry: Value) -> Result<()> {
        let mut file = tokio::fs::OpenOptions::new().append(true).write(true).open(&self.path).await?;
        let previous = file.metadata().await?.len();
        let mut bytes = serde_json::to_vec(&entry)?; bytes.push(b'\n');
        if let Err(error) = async { file.write_all(&bytes).await?; file.sync_all().await }.await {
            file.set_len(previous).await.context("Could not roll back incomplete session write")?;
            file.sync_all().await?; return Err(error).context("Could not save session history");
        }
        self.entries.push(entry); Ok(())
    }
    pub fn restored(&self, settings: &Settings) -> Result<(SessionModel, String, QueueState, Receipts)> {
        let mut model = settings.agent.model.clone(); let mut thinking = None;
        let mut queue = QueueState { available: true, capabilities: vec!["queue_edit".into(), "queue_delete".into(), "queue_pause".into(), "queue_resume".into(), "queue_run_prefix".into(), "queue_cancel_control".into()], boundaries: vec!["turn".into()], ..Default::default() };
        let mut receipts = HashMap::new();
        for entry in &self.entries {
            match entry["type"].as_str() {
                Some("model_change") => model = SessionModel { provider: entry["provider"].as_str().context("Model entry has no provider")?.into(), model_id: entry["modelId"].as_str().context("Model entry has no model")?.into() },
                Some("thinking_level_change") => thinking = entry["thinkingLevel"].as_str().map(str::to_owned),
                Some("tau_queue") => {
                    queue = serde_json::from_value(entry["queue"].clone()).context("Invalid persisted queue")?;
                    if let Some(id) = entry["accepted"]["id"].as_str() {
                        receipts.insert(id.into(), (entry["accepted"]["text"].as_str().context("Invalid prompt receipt")?.into(), serde_json::from_value(entry["accepted"]["disposition"].clone())?));
                    }
                }
                Some("message") if entry["message"]["role"] == "user" => {
                    if let Some(id) = entry["origin"]["requestId"].as_str() {
                        queue.requests.retain(|request| request.request_id != id);
                        receipts.entry(id.into()).or_insert_with(|| (entry["message"]["content"].as_str().unwrap_or_default().into(), crate::protocol::PromptDisposition::Submitted));
                    }
                }
                _ => {},
            }
        }
        queue.available = true; queue.run_id = None; queue.control = None;
        if !queue.requests.is_empty() { queue.paused = true; }
        let level = thinking.unwrap_or_else(|| settings.agent.model_thinking_levels.get(&format!("{}/{}", model.provider, model.model_id)).unwrap_or(&settings.agent.thinking_level).clone());
        Ok((model, level, queue, receipts))
    }
    pub fn transcript(&self, queue: QueueState) -> Result<Transcript> {
        Transcript::new(&self.entries, self.head(), queue)
    }
    pub async fn messages(&self, system: String, selected: &SessionModel, attachment_root: &Path) -> Result<Vec<Value>> {
        let mut output = vec![json!({"role":"system", "content":system})];
        let mut start = 0;
        for entry in self.entries.iter().rev().filter(|entry| entry["type"] == "compaction") {
            let native = entry.pointer("/details/kind").and_then(Value::as_str) == Some("codex-native-compaction");
            if native && (entry["details"]["model"] != selected.model_id || entry["details"]["provider"] != selected.provider) { continue; }
            let first = entry["firstKeptEntryId"].as_str().context("Compaction has no retained boundary")?;
            start = self.entries.iter().position(|entry| entry["id"] == first).context("Compaction boundary is missing")?;
            if native {
                output.push(json!({"role":"checkpoint","model":entry["details"]["model"],"accountId":entry["details"]["accountId"],
                    "baseUrl":entry["details"]["baseUrl"],"item":entry["details"]["item"]}));
            } else { output.push(json!({"role":"user","content":format!("Previous context summary:\n{}", entry["summary"].as_str().unwrap_or_default())})); }
            break;
        }
        for entry in &self.entries[start..] {
            if entry["type"] == "tau_attachment" {
                let request = crate::transcript::attachment_request(entry).context("Invalid generated attachment record")?;
                let mut parts = vec![json!({"type":"text","text":"The assistant generated this image in the preceding response. It is a reference image, not a new user request."})];
                match crate::attachments::image_reference(attachment_root, &request).await {
                    Ok(image) => parts.push(image),
                    Err(_) => parts.push(json!({"type":"text","text":"The original generated image file is no longer available. Do not assume its contents or silently regenerate it."})),
                }
                output.push(json!({"role":"user","content":parts})); continue;
            }
            if entry["type"] == "custom_message" {
                output.push(json!({"role":"user","content":entry["content"].as_str().unwrap_or_default()})); continue;
            }
            if entry["type"] != "message" { continue; }
            let message = &entry["message"];
            if let Some(native) = message.get("tauModelMessage") { output.push(native.clone()); continue; }
            let content = &message["content"];
            let text = content.as_str().map(str::to_owned).unwrap_or_else(|| content.as_array().into_iter().flatten()
                .filter(|part| part["type"] == "text").filter_map(|part| part["text"].as_str()).collect::<Vec<_>>().join("\n"));
            let images = content.as_array().into_iter().flatten().filter(|part| part["type"] == "image").map(|part|
                json!({"type":"image_url", "image_url":{"url":format!("data:{};base64,{}", part["mimeType"].as_str().unwrap_or("image/png"), part["data"].as_str().unwrap_or_default())}})).collect::<Vec<_>>();
            match message["role"].as_str() {
                Some("user") => {
                    if images.is_empty() { output.push(json!({"role":"user","content":text})); }
                    else { let mut parts = vec![json!({"type":"text","text":text})]; parts.extend(images); output.push(json!({"role":"user","content":parts})); }
                }
                Some("assistant") => {
                    if matches!(message["stopReason"].as_str(), Some("aborted" | "error")) { continue; }
                    let calls = content.as_array().into_iter().flatten().filter(|part| part["type"] == "toolCall").map(|part|
                        json!({"id":part["id"].as_str().unwrap_or_default().split('|').next().unwrap_or_default(),"type":"function","function":{"name":part["name"],"arguments":part["arguments"].to_string()}})).collect::<Vec<_>>();
                    let mut converted = json!({"role":"assistant","content":text});
                    if !calls.is_empty() { converted["tool_calls"] = json!(calls); }
                    output.push(converted);
                }
                Some("toolResult") => {
                    output.push(json!({"role":"tool","tool_call_id":message["toolCallId"].as_str().unwrap_or_default().split('|').next().unwrap_or_default(),"content":text}));
                    if !images.is_empty() { output.push(json!({"role":"user","content":images})); }
                }
                Some("bashExecution") => output.push(json!({"role":"user","content":format!("Shell output:\n{}", message["output"].as_str().unwrap_or_default())})),
                _ => {},
            }
        }
        // A daemon crash must not cause a tool call to be executed twice. Missing results are explicit.
        let completed = output.iter().filter_map(|message| message["tool_call_id"].as_str()).map(str::to_owned).collect::<HashSet<_>>();
        let mut repaired = Vec::new();
        for message in output {
            let missing = message["tool_calls"].as_array().into_iter().flatten().filter_map(|call| call["id"].as_str())
                .filter(|id| !completed.contains(*id)).map(str::to_owned).collect::<Vec<_>>();
            repaired.push(message);
            for id in missing { repaired.push(json!({"role":"tool","tool_call_id":id,"content":"Execution was interrupted. Its effects are unknown; inspect the filesystem before retrying."})); }
        }
        Ok(repaired)
    }
}

// Images/checkpoints are opaque provider inputs, not millions of base64 text tokens.
pub fn estimate_tokens(value: &Value) -> u64 {
    match value {
        Value::String(text) => (text.len() as u64).div_ceil(4),
        Value::Array(items) => items.iter().map(estimate_tokens).sum(),
        Value::Object(fields) => {
            if matches!(fields.get("type").and_then(Value::as_str), Some("image" | "image_url" | "input_image" | "compaction")) { return 2048; }
            fields.iter().filter(|(key,_)| !matches!(key.as_str(), "encrypted_content" | "tauModelMessage" | "codex_output" | "usage"))
                .map(|(_,value)| estimate_tokens(value)).sum()
        }
        _ => 0,
    }
}
