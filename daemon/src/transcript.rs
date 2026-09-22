use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::fs;
use tokio::io::{AsyncBufReadExt, BufReader};
use uuid::Uuid;

use crate::state::SessionModel;

pub const PAGE_EVENTS: usize = 50;
pub const PAGE_BYTES: usize = 256 * 1024;

pub const IMAGE_LIMIT: u64 = 10_000_000;
pub const FILE_LIMIT: u64 = 50_000_000;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatAttachment {
    #[serde(skip)]
    pub source_path: Option<PathBuf>,
    pub kind: AttachmentKind,
    pub file_name: String,
    pub caption: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentKind {
    Image,
    File,
}

pub struct AttachmentRequest {
    pub kind: AttachmentKind,
    pub path: PathBuf,
    pub caption: Option<String>,
    pub size: Option<u64>,
}

pub fn attachment_request(entry: &Value) -> Option<AttachmentRequest> {
    let message = entry.get("message")?;
    if !matches!((entry.get("type")?.as_str()?, message.get("role")?.as_str()?),
        ("message", "toolResult") | ("tau_attachment", "assistant")) { return None; }
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
        .map(|caption| caption.chars().take(1024).collect());
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub id: String,
    pub order: u64,
    pub entry_id: String,
    pub phase: EventPhase,
    pub origin: Origin,
    pub role: EventRole,
    pub kind: EventKind,
    pub text: String,
    pub timestamp: Option<String>,
    pub timestamp_ms: Option<u64>,
    pub tool_call_id: Option<String>,
    pub tool_name: Option<String>,
    pub stop_reason: Option<String>,
    pub error_message: Option<String>,
    pub is_error: bool,
    pub attachment: Option<ChatAttachment>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EventPhase { Saved, Live, Interrupted }

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EventRole { User, Assistant, Tool, System }

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Origin {
    pub request_id: Option<String>,
    pub request_revision: Option<u64>,
    pub stream_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind { Text, Thinking, Tool, Image, Hidden }

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueRef {
    pub request_id: String,
    pub revision: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueuedRequest {
    pub request_id: String,
    pub revision: u64,
    pub kind: String,
    pub text: String,
    pub images: usize,
    pub timestamp_ms: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueControl {
    pub command_id: String,
    pub run_id: Option<String>,
    pub action: String,
    pub boundary: Option<String>,
    pub requests: Vec<QueueRef>,
    pub status: String,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueState {
    pub available: bool,
    pub requests: Vec<QueuedRequest>,
    pub run_id: Option<String>,
    pub paused: bool,
    pub control: Option<QueueControl>,
    pub capabilities: Vec<String>,
    pub boundaries: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptSnapshot {
    pub generation: String,
    pub sequence: u64,
    pub events: Vec<Event>,
    pub queue: QueueState,
    pub before: Option<u64>,
    pub delivered: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryPage {
    pub events: Vec<Event>,
    pub before: Option<u64>,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptChange {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<Event>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub removed: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delta: Option<TextDelta>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub queue: Option<QueueState>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub delivered: Vec<String>,
    #[serde(skip)]
    head: Option<String>,
    #[serde(skip)]
    pub bumps_chat: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextDelta { pub event_id: String, pub text: String }

pub struct Transcript {
    pub generation: String,
    pub sequence: u64,
    events: BTreeMap<u64, Event>,
    by_id: BTreeMap<String, u64>,
    attachments: HashMap<String, u64>,
    next_order: u64,
    pub(crate) head: Option<String>,
    pub queue: QueueState,
}

impl Event {
    pub fn source_key(&self) -> String {
        if let Some(id) = &self.origin.stream_id { format!("stream:{id}") }
        else if let Some(id) = &self.origin.request_id && self.role == EventRole::User { format!("request:{id}") }
        else { format!("entry:{}", self.entry_id) }
    }

    pub fn from_entry(raw: &Value, live: bool) -> Result<Vec<Self>> {
        let message = raw.get("message").unwrap_or(&Value::Null);
        let entry_type = raw.get("type").and_then(Value::as_str).unwrap_or("message");
        let role = match message.get("role").and_then(Value::as_str) {
            Some("user") => EventRole::User,
            Some("assistant") => EventRole::Assistant,
            Some("toolResult") => EventRole::Tool,
            Some("bashExecution") => EventRole::System,
            _ if matches!(entry_type, "compaction" | "branch_summary") => EventRole::System,
            _ if entry_type == "custom_message" && raw.get("display").and_then(Value::as_bool) == Some(true) => EventRole::System,
            _ => return Ok(Vec::new()),
        };
        let stream_id = if live {
            Some(raw.get("streamId").and_then(Value::as_str).filter(|id| !id.is_empty()).context("History live entry has no stream ID")?.to_owned())
        } else { raw.pointer("/origin/streamId").and_then(Value::as_str).map(str::to_owned) };
        let entry_id = if live { format!("live-{}", stream_id.as_deref().unwrap()) }
            else { raw.get("id").and_then(Value::as_str).filter(|id| !id.is_empty()).context("History entry has no ID")?.to_owned() };
        let body = if entry_type == "custom_message" { raw.get("content") }
            else if matches!(entry_type, "compaction" | "branch_summary") { raw.get("summary") }
            else if message.get("role").and_then(Value::as_str) == Some("bashExecution") { message.get("output") }
            else { message.get("content") };
        let template = Self {
            id: String::new(), order: 0, entry_id, phase: if live { EventPhase::Live } else { EventPhase::Saved },
            origin: Origin {
                request_id: raw.pointer("/origin/requestId").and_then(Value::as_str).map(str::to_owned),
                request_revision: raw.pointer("/origin/requestRevision").and_then(Value::as_u64), stream_id,
            },
            role, kind: EventKind::Hidden, text: String::new(),
            timestamp: raw.get("timestamp").and_then(Value::as_str).map(str::to_owned),
            timestamp_ms: message.get("timestamp").and_then(Value::as_u64),
            tool_call_id: message.get("toolCallId").and_then(Value::as_str).map(str::to_owned),
            tool_name: message.get("toolName").and_then(Value::as_str).map(str::to_owned),
            stop_reason: message.get("stopReason").and_then(Value::as_str).map(str::to_owned),
            error_message: message.get("errorMessage").and_then(Value::as_str).map(str::to_owned),
            is_error: message.get("isError").and_then(Value::as_bool).unwrap_or(false), attachment: None,
        };
        let base = template.source_key();
        let mut events = Vec::new();
        match body {
            Some(Value::String(text)) => {
                let mut event = template.clone();
                event.kind = EventKind::Text; event.text = text.clone(); events.push(event);
            }
            Some(Value::Array(blocks)) => for block in blocks {
                let mut event = template.clone(); event.set_content(block); events.push(event);
            },
            _ => {}
        }
        if events.is_empty() { events.push(template); }
        for (index, event) in events.iter_mut().enumerate() { event.id = format!("{base}:{index}"); }
        if !live && let Some(request) = attachment_request(raw) {
            events[0].attachment = request.path.file_name().map(|name| ChatAttachment {
                file_name: name.to_string_lossy().into_owned(), source_path: Some(request.path.clone()),
                kind: request.kind, caption: request.caption, size: request.size,
            });
        }
        Ok(events)
    }

    fn set_content(&mut self, block: &Value) {
        if let Some(timestamp) = block.get("timestamp").and_then(Value::as_u64) { self.timestamp_ms = Some(timestamp); }
        self.text.clear();
        self.kind = match block.get("type").and_then(Value::as_str) {
            Some("text") => { self.text = block.get("text").and_then(Value::as_str).unwrap_or_default().to_owned(); EventKind::Text }
            Some("thinking") => { self.text = block.get("thinking").and_then(Value::as_str).unwrap_or_default().to_owned(); EventKind::Thinking }
            Some("toolCall") => {
                self.text = block.get("partialArguments").and_then(Value::as_str).map(str::to_owned).unwrap_or_else(|| match block.get("arguments") {
                    Some(Value::String(text)) => text.clone(), Some(value) => serde_json::to_string_pretty(value).unwrap_or_default(), None => String::new(),
                });
                self.tool_call_id = block.get("id").and_then(Value::as_str).map(str::to_owned);
                self.tool_name = block.get("name").and_then(Value::as_str).map(str::to_owned);
                EventKind::Tool
            }
            Some("image") => { self.text = block.get("mimeType").and_then(Value::as_str).unwrap_or_default().to_owned(); EventKind::Image }
            _ => EventKind::Hidden,
        };
    }
}

impl Transcript {
    pub fn new(raw: &[Value], head: Option<String>, queue: QueueState) -> Result<Self> {
        let mut by_entry = HashMap::new();
        for entry in raw {
            let id = entry.get("id").and_then(Value::as_str).filter(|id| !id.is_empty()).context("History entry has no ID")?;
            if by_entry.insert(id, entry).is_some() { bail!("History has duplicate entry IDs"); }
        }
        let mut branch = Vec::new();
        let mut seen = HashSet::new();
        let mut cursor = head.as_deref();
        while let Some(id) = cursor {
            if !seen.insert(id) { bail!("History branch contains a cycle"); }
            let Some(entry) = by_entry.get(id) else { break; };
            branch.push(*entry);
            cursor = entry.get("parentId").and_then(Value::as_str);
        }
        if head.as_ref().is_some_and(|id| !by_entry.contains_key(id.as_str())) { bail!("History head is missing"); }
        let mut transcript = Self {
            generation: Uuid::new_v4().to_string(), sequence: 0,
            events: BTreeMap::new(), by_id: BTreeMap::new(), attachments: HashMap::new(), next_order: 0, head, queue,
        };
        for raw in branch.into_iter().rev() {
            for mut event in Event::from_entry(raw, false)? {
                if transcript.by_id.contains_key(&event.id) { bail!("History has duplicate event identities"); }
                event.order = transcript.next_order; transcript.next_order += 1;
                if event.attachment.is_some() { transcript.attachments.insert(event.entry_id.clone(), event.order); }
                transcript.by_id.insert(event.id.clone(), event.order);
                transcript.events.insert(event.order, event);
            }
        }
        Ok(transcript)
    }

    pub fn event(&self, id: &str) -> Option<&Event> { self.by_id.get(id).and_then(|order| self.events.get(order)) }
    pub fn attachment(&self, entry_id: &str) -> Option<&ChatAttachment> {
        self.attachments.get(entry_id).and_then(|order| self.events.get(order)).and_then(|event| event.attachment.as_ref())
    }
    pub fn events_mut(&mut self) -> impl Iterator<Item = &mut Event> { self.events.values_mut() }

    pub fn page(&self, before: Option<u64>) -> HistoryPage {
        let mut events = Vec::new();
        let mut bytes = 0;
        let mut more = false;
        for event in self.events.range(..before.unwrap_or(self.next_order)).rev().map(|(_, event)| event) {
            let size = serde_json::to_vec(event).expect("event serialization").len();
            if !events.is_empty() && (events.len() >= PAGE_EVENTS || bytes + size > PAGE_BYTES) { more = true; break; }
            bytes += size; events.push(event.clone());
        }
        events.reverse();
        HistoryPage { before: if more { events.first().map(|event| event.order) } else { None }, events }
    }

    pub fn snapshot(&self, requests: &[String]) -> TranscriptSnapshot {
        let mut page = self.page(None);
        for event in self.events.values().filter(|event| event.phase == EventPhase::Live) {
            if page.before.is_some_and(|before| event.order < before) { page.events.push(event.clone()); }
        }
        page.events.sort_by_key(|event| event.order);
        let requests = requests.iter().collect::<HashSet<_>>();
        let delivered = self.events.values().filter(|event| event.phase == EventPhase::Saved)
            .filter_map(|event| event.origin.request_id.as_ref()).filter(|id| requests.contains(id)).cloned().collect::<HashSet<_>>();
        TranscriptSnapshot { generation: self.generation.clone(), sequence: self.sequence, events: page.events,
            queue: self.queue.clone(), before: page.before, delivered: delivered.into_iter().collect() }
    }

    pub fn project(&self, entry: &Value, live: bool) -> Result<TranscriptChange> {
        let mut change = TranscriptChange::default();
        if !live {
            let id = entry.get("id").and_then(Value::as_str).context("History append has no ID")?;
            if entry.get("parentId").and_then(Value::as_str) != self.head.as_deref() { bail!("History branch changed"); }
            change.head = Some(id.to_owned());
            if let Some(id) = entry.pointer("/origin/requestId").and_then(Value::as_str) { change.delivered.push(id.to_owned()); }
        }
        change.events = Event::from_entry(entry, live)?;
        if let Some(first) = change.events.first() {
            let base = first.source_key();
            let prefix = format!("{base}:");
            let ids = change.events.iter().map(|event| &event.id).collect::<HashSet<_>>();
            change.removed = self.by_id.range(prefix.clone()..).take_while(|(id, _)| id.starts_with(&prefix))
                .filter(|(id, order)| !ids.contains(id) && self.events[order].source_key() == base).map(|(id, _)| id.clone()).collect();
        }
        let mut next = self.next_order;
        for event in &mut change.events {
            event.order = self.by_id.get(&event.id).copied().unwrap_or_else(|| { let order = next; next += 1; order });
        }
        if change.events.windows(2).any(|pair| pair[0].order >= pair[1].order) {
            bail!("History content order changed; read its current snapshot");
        }
        change.bumps_chat = change.events.iter().any(|event|
            matches!(event.role, EventRole::User | EventRole::Assistant) && matches!(event.kind, EventKind::Text | EventKind::Image) &&
            (event.phase == EventPhase::Saved || !event.text.is_empty() && self.event(&event.id).is_none_or(|old| old.text.is_empty()))) ||
            change.delta.as_ref().is_some_and(|delta| !delta.text.is_empty() && self.event(&delta.event_id).is_some_and(|event|
                event.role == EventRole::Assistant && event.kind == EventKind::Text && event.text.is_empty()));
        Ok(change)
    }

    pub fn apply(&mut self, change: &TranscriptChange) -> Result<()> {
        if let Some(delta) = &change.delta {
            let order = self.by_id.get(&delta.event_id).context("History delta references a missing event")?;
            let event = self.events.get_mut(order).unwrap();
            if event.phase != EventPhase::Live { bail!("History delta references a finished event"); }
            event.text.push_str(&delta.text);
        }
        for id in &change.removed {
            if let Some(order) = self.by_id.remove(id) && let Some(event) = self.events.remove(&order)
                && event.attachment.is_some() { self.attachments.remove(&event.entry_id); }
        }
        for event in &change.events {
            self.next_order = self.next_order.max(event.order + 1);
            self.by_id.insert(event.id.clone(), event.order);
            if event.attachment.is_some() { self.attachments.insert(event.entry_id.clone(), event.order); }
            self.events.insert(event.order, event.clone());
        }
        if let Some(head) = &change.head { self.head = Some(head.clone()); }
        if let Some(queue) = &change.queue { self.queue = queue.clone(); }
        self.sequence += 1;
        Ok(())
    }
}

pub(crate) async fn session_model_from_file(path: &Path) -> Result<Option<SessionModel>> {
    let file = fs::File::open(path).await?;
    let mut lines = BufReader::new(file).lines();
    let mut parents = HashMap::<String, Option<String>>::new();
    let mut models = HashMap::<String, SessionModel>::new();
    let mut leaf_id = None;
    while let Some(line) = lines.next_line().await? {
        let entry = match serde_json::from_str::<serde_json::Value>(&line) {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let Some(id) = entry.get("id").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let parent_id = entry
            .get("parentId")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        parents.insert(id.to_owned(), parent_id);
        leaf_id = Some(id.to_owned());

        let model = if entry.get("type").and_then(serde_json::Value::as_str)
            == Some("model_change")
        {
            entry
                .get("provider")
                .and_then(serde_json::Value::as_str)
                .zip(entry.get("modelId").and_then(serde_json::Value::as_str))
        } else {
            entry
                .get("message")
                .filter(|message| {
                    message.get("role").and_then(serde_json::Value::as_str) == Some("assistant")
                })
                .and_then(|message| {
                    message
                        .get("provider")
                        .and_then(serde_json::Value::as_str)
                        .zip(message.get("model").and_then(serde_json::Value::as_str))
                })
        };
        if let Some((provider, model_id)) = model
            && !provider.is_empty()
            && !model_id.is_empty()
        {
            models.insert(
                id.to_owned(),
                SessionModel {
                    provider: provider.to_owned(),
                    model_id: model_id.to_owned(),
                },
            );
        }
    }

    let mut current = leaf_id;
    for _ in 0..=parents.len() {
        let Some(id) = current else {
            break;
        };
        if let Some(model) = models.get(&id) {
            return Ok(Some(model.clone()));
        }
        current = parents.get(&id).cloned().flatten();
    }
    Ok(None)
}

#[cfg(test)]
#[path = "transcript_test.rs"]
mod tests;
