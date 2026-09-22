use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
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
pub enum EventPhase {
    Saved,
    Live,
    Interrupted,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EventRole {
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

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
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
    pub fn native() -> Self { Self { available:true, capabilities:["queue_edit","queue_delete","queue_pause","queue_resume","queue_run_prefix","queue_cancel_control"].into_iter().map(str::to_owned).collect(), boundaries:vec!["turn".into()], ..Default::default() } }
}


#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptSnapshot {
    pub generation: String,
    pub sequence: u64,
    pub events: Vec<Event>,
    pub queue: QueueState,
    pub before: Option<u64>,
    pub delivered: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryPage {
    pub events: Vec<Event>,
    pub before: Option<u64>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptChange {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<Event>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delta: Option<TextDelta>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub queue: Option<QueueState>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub delivered: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextDelta {
    pub event_id: String,
    pub text: String,
}

impl Event {
    pub fn source_key(&self) -> String {
        if let Some(id) = &self.origin.stream_id {
            format!("stream:{id}")
        } else if let Some(id) = &self.origin.request_id
            && self.role == EventRole::User
        {
            format!("request:{id}")
        } else {
            format!("entry:{}", self.entry_id)
        }
    }
}
