use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::protocol::{AttachmentKind, ChatAttachment};

pub const IMAGE_LIMIT: u64 = 10_000_000;
pub const FILE_LIMIT: u64 = 50_000_000;

pub struct AttachmentRequest {
    pub kind: AttachmentKind,
    pub path: PathBuf,
    pub caption: Option<String>,
    pub size: Option<u64>,
}

pub fn attachment_request(entry: &Value) -> Option<AttachmentRequest> {
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

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: String,
    pub parent_id: Option<String>,
    pub entry_type: String,
    pub phase: EntryPhase,
    pub origin: Origin,
    pub role: Option<EntryRole>,
    pub timestamp: Option<String>,
    pub timestamp_ms: Option<u64>,
    pub content: Vec<Content>,
    pub tool_call_id: Option<String>,
    pub tool_name: Option<String>,
    pub stop_reason: Option<String>,
    pub error_message: Option<String>,
    pub is_error: bool,
    pub attachment: Option<ChatAttachment>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryPhase {
    Saved,
    Live,
    Interrupted,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryRole {
    User,
    Assistant,
    Tool,
    System,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Origin {
    pub request_id: Option<String>,
    pub request_revision: Option<u64>,
    pub stream_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Content {
    pub kind: ContentKind,
    pub text: String,
    pub tool_call_id: Option<String>,
    pub tool_name: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentKind {
    Text,
    Thinking,
    Tool,
    Image,
    Hidden,
}

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

impl QueueState {
    pub fn from_pi(raw: &Value) -> Result<Self> {
        let mut ids = HashSet::new();
        let mut requests = Vec::new();
        for request in raw.get("queuedRequests").and_then(Value::as_array).context("Pi has no queue")? {
            let request_id = request.get("requestId").and_then(Value::as_str)
                .filter(|id| !id.is_empty() && id.len() <= 128).context("Pi queue has an invalid request ID")?;
            if !ids.insert(request_id) { bail!("Pi queue has duplicate request IDs"); }
            let revision = request.get("revision").and_then(Value::as_u64).context("Pi queue has no revision")?;
            let kind = request.get("kind").and_then(Value::as_str)
                .filter(|kind| matches!(*kind, "steer" | "followUp")).context("Pi queue has an invalid kind")?;
            let message = request.get("message").context("Pi queue has no message")?;
            let (text, images) = match message.get("content") {
                Some(Value::String(text)) => (text.clone(), 0),
                Some(Value::Array(blocks)) => (
                    blocks.iter().filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
                        .filter_map(|block| block.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join("\n"),
                    blocks.iter().filter(|block| block.get("type").and_then(Value::as_str) == Some("image")).count(),
                ),
                _ => bail!("Pi queue has no editable content"),
            };
            requests.push(QueuedRequest { request_id: request_id.to_owned(), revision, kind: kind.to_owned(), text, images,
                timestamp_ms: message.get("timestamp").and_then(Value::as_u64) });
        }
        Ok(Self {
            available: true, requests,
            run_id: raw.get("runId").and_then(Value::as_str).map(str::to_owned),
            paused: raw.get("paused").and_then(Value::as_bool).context("Pi has no queue pause state")?,
            control: serde_json::from_value(raw.get("control").cloned().unwrap_or(Value::Null))?,
            capabilities: serde_json::from_value(raw.get("capabilities").context("Pi has no queue capabilities")?.clone())?,
            boundaries: serde_json::from_value(raw.get("boundaries").context("Pi has no queue boundaries")?.clone())?,
        })
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptSnapshot {
    pub generation: String,
    pub sequence: u64,
    pub head: Option<String>,
    pub entries: Vec<Entry>,
    pub queue: QueueState,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum TranscriptChange {
    Entry { entry: Box<Entry> },
    Block { entry_id: String, index: usize, content: Content },
    Delta { entry_id: String, index: usize, delta: String },
    Head { head: Option<String> },
    Queue { queue: QueueState },
    Interrupted,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PiPosition {
    pub session_id: String,
    pub generation: String,
    pub sequence: u64,
}

impl PiPosition {
    pub fn from_pi(raw: &Value) -> Result<Self> {
        Ok(Self {
            session_id: raw.get("sessionId").and_then(Value::as_str).filter(|id| !id.is_empty()).context("Pi transcript has no session ID")?.to_owned(),
            generation: raw.get("generation").and_then(Value::as_str).filter(|id| !id.is_empty()).context("Pi transcript has no generation")?.to_owned(),
            sequence: raw.get("sequence").and_then(Value::as_u64).context("Pi transcript has no sequence")?,
        })
    }
}

pub struct Transcript {
    pub generation: String,
    pub sequence: u64,
    pub source: Option<PiPosition>,
    entries: Vec<Entry>,
    by_id: HashMap<String, usize>,
    head: Option<String>,
    pub queue: QueueState,
}

impl Entry {
    pub fn from_pi(raw: &Value, live: bool) -> Result<Self> {
        let stream_id = if live {
            Some(raw.get("streamId").and_then(Value::as_str).filter(|id| !id.is_empty()).context("Pi live entry has no streamId")?.to_owned())
        } else {
            raw.pointer("/origin/streamId").and_then(Value::as_str).map(str::to_owned)
        };
        let id = if live {
            format!("live-{}", stream_id.as_deref().expect("live stream ID was checked"))
        } else {
            raw.get("id").and_then(Value::as_str).filter(|id| !id.is_empty()).context("Pi entry has no ID")?.to_owned()
        };
        let entry_type = if live {
            "message"
        } else {
            raw.get("type").and_then(Value::as_str).context("Pi entry has no type")?
        };
        let message = raw.get("message").unwrap_or(&Value::Null);
        let role = match message.get("role").and_then(Value::as_str) {
            Some("user") => Some(EntryRole::User),
            Some("assistant") => Some(EntryRole::Assistant),
            Some("toolResult") => Some(EntryRole::Tool),
            Some("bashExecution") => Some(EntryRole::System),
            _ if matches!(entry_type, "compaction" | "branch_summary") => Some(EntryRole::System),
            _ if entry_type == "custom_message" && raw.get("display").and_then(Value::as_bool) == Some(true) => Some(EntryRole::System),
            _ => None,
        };
        let body = if entry_type == "custom_message" {
            raw.get("content")
        } else if matches!(entry_type, "compaction" | "branch_summary") {
            raw.get("summary")
        } else if message.get("role").and_then(Value::as_str) == Some("bashExecution") {
            message.get("output")
        } else {
            message.get("content")
        };
        let content = match body.filter(|_| role.is_some()) {
            Some(Value::String(text)) => vec![Content::text(ContentKind::Text, text.clone())],
            Some(Value::Array(blocks)) => blocks.iter().map(Content::from_pi).collect(),
            _ => Vec::new(),
        };
        let attachment = if live { None } else { attachment_request(raw) }.and_then(|request| {
            Some(ChatAttachment {
                kind: request.kind,
                file_name: request.path.file_name()?.to_string_lossy().into_owned(),
                caption: request.caption,
                size: request.size,
            })
        });
        Ok(Self {
            id,
            parent_id: raw.get("parentId").and_then(Value::as_str).map(str::to_owned),
            entry_type: entry_type.to_owned(),
            phase: if live { EntryPhase::Live } else { EntryPhase::Saved },
            origin: Origin {
                request_id: raw.pointer("/origin/requestId").and_then(Value::as_str).map(str::to_owned),
                request_revision: raw.pointer("/origin/requestRevision").and_then(Value::as_u64),
                stream_id,
            },
            role,
            timestamp: raw.get("timestamp").and_then(Value::as_str).map(str::to_owned),
            timestamp_ms: message.get("timestamp").and_then(Value::as_u64),
            content,
            tool_call_id: message.get("toolCallId").and_then(Value::as_str).map(str::to_owned),
            tool_name: message.get("toolName").and_then(Value::as_str).map(str::to_owned),
            stop_reason: message.get("stopReason").and_then(Value::as_str).map(str::to_owned),
            error_message: message.get("errorMessage").and_then(Value::as_str).map(str::to_owned),
            is_error: message.get("isError").and_then(Value::as_bool).unwrap_or(false),
            attachment,
        })
    }
}

impl Content {
    pub fn text(kind: ContentKind, text: String) -> Self {
        Self { kind, text, tool_call_id: None, tool_name: None }
    }

    pub fn from_pi(block: &Value) -> Self {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => Self::text(ContentKind::Text, block.get("text").and_then(Value::as_str).unwrap_or_default().to_owned()),
            Some("thinking") => Self::text(ContentKind::Thinking, block.get("thinking").and_then(Value::as_str).unwrap_or_default().to_owned()),
            Some("toolCall") => Self {
                kind: ContentKind::Tool,
                text: block.get("partialArguments").and_then(Value::as_str).map(str::to_owned).unwrap_or_else(|| {
                    match block.get("arguments") {
                        Some(Value::String(text)) => text.clone(),
                        Some(arguments) => serde_json::to_string_pretty(arguments).unwrap_or_default(),
                        None => String::new(),
                    }
                }),
                tool_call_id: block.get("id").and_then(Value::as_str).map(str::to_owned),
                tool_name: block.get("name").and_then(Value::as_str).map(str::to_owned),
            },
            Some("image") => Self::text(ContentKind::Image, block.get("mimeType").and_then(Value::as_str).unwrap_or_default().to_owned()),
            _ => Self::text(ContentKind::Hidden, String::new()),
        }
    }
}

impl TranscriptChange {
    pub fn from_pi(raw: &Value) -> Result<Self> {
        match raw.get("type").and_then(Value::as_str) {
            Some("append" | "live") => {
                let live = raw.get("type").and_then(Value::as_str) == Some("live");
                let entry = Entry::from_pi(raw.get("entry").context("Pi change has no entry")?, live)?;
                if !live && raw.get("leafId").and_then(Value::as_str) != Some(entry.id.as_str()) {
                    bail!("Pi append selected an unexpected head");
                }
                Ok(Self::Entry { entry: Box::new(entry) })
            }
            Some("head") => Ok(Self::Head { head: raw.get("leafId").and_then(Value::as_str).map(str::to_owned) }),
            Some("queue") => Ok(Self::Queue { queue: QueueState::from_pi(raw)? }),
            Some("delta") => {
                let stream_id = raw.get("streamId").and_then(Value::as_str).context("Pi delta has no streamId")?;
                let entry_id = format!("live-{stream_id}");
                let delta = raw.pointer("/event/assistantMessageEvent").context("Pi delta has no content event")?;
                let index = delta.get("contentIndex").and_then(Value::as_u64)
                    .and_then(|index| usize::try_from(index).ok()).context("Pi delta has no content index")?;
                match delta.get("type").and_then(Value::as_str) {
                    Some("text_delta" | "thinking_delta" | "toolcall_delta") => Ok(Self::Delta {
                        entry_id, index,
                        delta: delta.get("delta").and_then(Value::as_str).context("Pi delta has no text")?.to_owned(),
                    }),
                    Some("text_start" | "thinking_start" | "text_end" | "thinking_end") => Ok(Self::Block {
                        entry_id, index,
                        content: Content::text(
                            if delta.get("type").and_then(Value::as_str).is_some_and(|kind| kind.starts_with("thinking")) {
                                ContentKind::Thinking
                            } else { ContentKind::Text },
                            delta.get("content").and_then(Value::as_str).unwrap_or_default().to_owned(),
                        ),
                    }),
                    Some("toolcall_start") => Ok(Self::Block { entry_id, index, content: Content {
                        kind: ContentKind::Tool,
                        text: String::new(),
                        tool_call_id: delta.get("id").and_then(Value::as_str).map(str::to_owned),
                        tool_name: delta.get("toolName").and_then(Value::as_str).map(str::to_owned),
                    } }),
                    Some("toolcall_end") => Ok(Self::Block { entry_id, index, content: Content::from_pi(
                        delta.get("toolCall").context("Pi tool completion has no tool call")?,
                    ) }),
                    _ => bail!("unsupported Pi content delta"),
                }
            }
            _ => bail!("unsupported Pi transcript change"),
        }
    }
}

impl Transcript {
    pub fn new(entries: Vec<Entry>, head: Option<String>, source: Option<PiPosition>, queue: QueueState) -> Result<Self> {
        let by_id = entries.iter().enumerate().map(|(index, entry)| (entry.id.clone(), index)).collect::<HashMap<_, _>>();
        if by_id.len() != entries.len() { bail!("transcript contains duplicate entry IDs"); }
        let mut checked = HashSet::new();
        for entry in &entries {
            if entry.id.is_empty() { bail!("transcript contains an empty entry ID"); }
            let mut path = HashSet::new();
            let mut cursor = Some(&entry.id);
            while let Some(id) = cursor {
                if checked.contains(id) { break; }
                if !path.insert(id) { bail!("transcript branch contains a cycle"); }
                let entry = &entries[*by_id.get(id).context("transcript branch has a missing parent")?];
                if let Some(parent) = entry.parent_id.as_ref() {
                    let parent = &entries[*by_id.get(parent).context("transcript branch has a missing parent")?];
                    if parent.phase != EntryPhase::Saved { bail!("transcript parent is provisional"); }
                }
                cursor = entry.parent_id.as_ref();
            }
            checked.extend(path);
        }
        if let Some(head) = &head {
            let entry = &entries[*by_id.get(head).context("transcript head references a missing entry")?];
            if entry.phase != EntryPhase::Saved { bail!("transcript head selects a provisional entry"); }
        }
        Ok(Self {
            generation: Uuid::new_v4().to_string(), sequence: 0, source,
            entries, by_id, head, queue,
        })
    }

    pub fn snapshot(&self) -> TranscriptSnapshot {
        TranscriptSnapshot {
            generation: self.generation.clone(), sequence: self.sequence, head: self.head.clone(),
            entries: self.entries.clone(), queue: self.queue.clone(),
        }
    }

    pub fn retain_interrupted(&mut self, previous: &Self) {
        let saved_streams = self.entries.iter()
            .filter(|entry| entry.phase == EntryPhase::Saved)
            .filter_map(|entry| entry.origin.stream_id.clone())
            .collect::<HashSet<_>>();
        for entry in &previous.entries {
            if entry.phase == EntryPhase::Saved || self.by_id.contains_key(&entry.id)
                || entry.origin.stream_id.as_ref().is_some_and(|id| saved_streams.contains(id))
            { continue; }
            let mut entry = entry.clone();
            entry.phase = EntryPhase::Interrupted;
            if entry.parent_id.as_ref().is_some_and(|id| !self.by_id.contains_key(id)) {
                entry.parent_id = None;
            }
            self.by_id.insert(entry.id.clone(), self.entries.len());
            self.entries.push(entry);
        }
    }

    pub fn check_position(&self, position: &PiPosition) -> Result<bool> {
        let source = self.source.as_ref().context("Pi transcript needs its initial snapshot")?;
        if position.session_id != source.session_id || position.generation != source.generation {
            bail!("Pi transcript generation changed");
        }
        if position.sequence <= source.sequence { return Ok(false); }
        if position.sequence != source.sequence + 1 { bail!("Pi transcript has an update gap"); }
        Ok(true)
    }

    pub fn apply(&mut self, change: &TranscriptChange, position: Option<PiPosition>) -> Result<()> {
        match change {
            TranscriptChange::Entry { entry } => {
                if entry.id.is_empty() || entry.parent_id.as_ref() == Some(&entry.id) {
                    bail!("Pi entry has an invalid identity or parent");
                }
                if let Some(parent) = entry.parent_id.as_ref() {
                    let parent = &self.entries[*self.by_id.get(parent).context("Pi entry references a missing parent")?];
                    if parent.phase != EntryPhase::Saved { bail!("Pi entry references a provisional parent"); }
                }
                if let Some(index) = self.by_id.get(&entry.id)
                    && self.entries[*index].phase != EntryPhase::Live
                { bail!("Pi cannot replace a retained non-live entry"); }
                if entry.phase == EntryPhase::Saved {
                    if self.by_id.contains_key(&entry.id) { bail!("Pi appended an existing saved entry ID"); }
                    if let Some(stream_id) = &entry.origin.stream_id {
                        let provisional = format!("live-{stream_id}");
                        if let Some(index) = self.by_id.remove(&provisional) {
                            self.entries.remove(index);
                            for index in index..self.entries.len() {
                                self.by_id.insert(self.entries[index].id.clone(), index);
                            }
                        }
                    }
                    self.head = Some(entry.id.clone());
                }
                if let Some(index) = self.by_id.get(&entry.id) {
                    self.entries[*index] = (**entry).clone();
                } else {
                    self.by_id.insert(entry.id.clone(), self.entries.len());
                    self.entries.push((**entry).clone());
                }
            }
            TranscriptChange::Block { entry_id, index, content } => {
                let entry = self.by_id.get(entry_id).context("Pi block references a missing entry")?;
                let entry = &mut self.entries[*entry];
                if entry.phase != EntryPhase::Live { bail!("Pi block references a non-live entry"); }
                let blocks = &mut entry.content;
                if *index > blocks.len() { bail!("Pi block references a missing content index"); }
                if *index == blocks.len() { blocks.push(content.clone()); }
                else { blocks[*index] = content.clone(); }
            }
            TranscriptChange::Delta { entry_id, index, delta } => {
                let entry = self.by_id.get(entry_id).context("Pi delta references a missing entry")?;
                let entry = &mut self.entries[*entry];
                if entry.phase != EntryPhase::Live { bail!("Pi delta references a non-live entry"); }
                entry.content.get_mut(*index).context("Pi delta references a missing block")?.text.push_str(delta);
            }
            TranscriptChange::Head { head } => {
                let mut visited = HashSet::new();
                let mut cursor = head.as_ref();
                while let Some(id) = cursor {
                    if !visited.insert(id) { bail!("Pi selected a cyclic branch"); }
                    let entry = &self.entries[*self.by_id.get(id).context("Pi head references a missing entry")?];
                    if entry.phase != EntryPhase::Saved { bail!("Pi selected a provisional entry"); }
                    cursor = entry.parent_id.as_ref();
                }
                self.head = head.clone();
            }
            TranscriptChange::Queue { queue } => self.queue = queue.clone(),
            TranscriptChange::Interrupted => {
                self.queue.available = false;
                for entry in &mut self.entries {
                    if entry.phase == EntryPhase::Live { entry.phase = EntryPhase::Interrupted; }
                }
            }
        }
        if let Some(position) = position { self.source = Some(position); }
        self.sequence += 1;
        Ok(())
    }
}

#[cfg(test)]
#[path = "transcript_test.rs"]
mod tests;
