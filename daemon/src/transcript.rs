use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use serde_json::Value;
use uuid::Uuid;


pub const PAGE_EVENTS: usize = 50;
pub const PAGE_BYTES: usize = 256 * 1024;

pub const IMAGE_LIMIT: u64 = 10_000_000;
pub const FILE_LIMIT: u64 = 50_000_000;

pub use tau_protocol::{ChatAttachment, AttachmentKind, Event, EventPhase, EventRole, Origin, EventKind, QueuedRequest, QueueControl, QueueState, TranscriptSnapshot, HistoryPage, TextDelta};

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

#[derive(Clone, Debug, Default)]
pub struct TranscriptChange {
    pub wire: tau_protocol::TranscriptChange,
    pub(crate) head: Option<String>,
    pub bumps_chat: bool,
}
impl std::ops::Deref for TranscriptChange {
    type Target = tau_protocol::TranscriptChange;
    fn deref(&self) -> &Self::Target { &self.wire }
}
impl std::ops::DerefMut for TranscriptChange {
    fn deref_mut(&mut self) -> &mut Self::Target { &mut self.wire }
}

#[derive(Clone)]
pub struct Transcript {
    pub generation: String,
    pub sequence: u64,
    events: BTreeMap<u64, Event>,
    by_id: BTreeMap<String, u64>,
    has_older: bool,
    next_order: u64,
    pub(crate) head: Option<String>,
    pub queue: QueueState,
}

pub trait EventProjection: Sized {
    fn from_entry(raw: &Value, live: bool) -> Result<Vec<Self>>;
    fn set_content(&mut self, block: &Value);
}
impl EventProjection for Event {
    fn from_entry(raw: &Value, live: bool) -> Result<Vec<Self>> {
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
    pub fn new(page: HistoryPage, head: Option<String>, next_order: u64, queue: QueueState) -> Self {
        Self { generation:Uuid::new_v4().to_string(),sequence:0,
            by_id:page.events.iter().map(|event| (event.id.clone(),event.order)).collect(),
            events:page.events.into_iter().map(|event| (event.order,event)).collect(),
            has_older:page.before.is_some(), next_order, head, queue }
    }
    pub fn event(&self, id: &str) -> Option<&Event> { self.by_id.get(id).and_then(|order| self.events.get(order)) }
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
        HistoryPage { before: if more || self.has_older { events.first().map(|event| event.order) } else { None }, events }
    }

    pub fn snapshot(&self) -> TranscriptSnapshot {
        let mut page = self.page(None);
        for event in self.events.values().filter(|event| event.phase == EventPhase::Live) {
            if page.before.is_some_and(|before| event.order < before) { page.events.push(event.clone()); }
        }
        page.events.sort_by_key(|event| event.order);
        TranscriptSnapshot { generation:self.generation.clone(),sequence:self.sequence,events:page.events,
            queue:self.queue.clone(),before:page.before,delivered:Vec::new() }
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
            if let Some(order) = self.by_id.remove(id) { self.events.remove(&order); }
        }
        for event in &change.events {
            self.next_order = self.next_order.max(event.order + 1);
            self.by_id.insert(event.id.clone(), event.order);
            self.events.insert(event.order, event.clone());
        }
        if let Some(head) = &change.head { self.head = Some(head.clone()); }
        if let Some(queue) = &change.queue { self.queue = queue.clone(); }
        self.sequence += 1;
        if let Some(first) = self.page(None).events.first().map(|event| event.order) {
            self.events.retain(|order,event| {
                if *order >= first || event.phase == EventPhase::Live { true }
                else { self.by_id.remove(&event.id); self.has_older = true; false }
            });
        }
        Ok(())
    }
}
