//! Request-driven display blocks. A parent is an address, never an embedded tree.
//! Content bytes are separate from bounded headers and use the same range protocol
//! for text, code, tool input/output and files.
use serde::{Deserialize, Serialize};

pub const BLOCK_CHUNK_BYTES: usize = 16 * 1024;
pub const MAX_BLOCK_HEADER_BYTES: usize = 4096;
pub const MAX_BLOCK_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_FEED_PAGE: usize = 32;
pub const BLOCK_WINDOW_BYTES: u32 = 64 * 1024;
pub const UPLOAD_SCOPE: &str = "@uploads";
pub const CONTROL_SCOPE: &str = "@control";
pub const MAX_COMMAND_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BlockKind { Text, Thinking, Code, Tool, File, Image, State, Queue }

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BlockHeader {
    pub id: String,
    pub parent: Option<String>,
    pub order: u64,
    pub kind: BlockKind,
    /// Small display attributes only. Never put content or child lists here.
    pub meta: serde_json::Value,
    /// Changes only when content is replaced, not on append or metadata changes.
    pub version: u64,
    pub length: u64,
    pub sealed: bool,
    /// Durable, database-lineage-scoped metadata sequence.
    pub revision: u64,
}

/// A body address is meaningful only within its source lineage and chat scope.
/// Availability belongs to the reader, not this immutable native identity.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BodyRef {
    pub source: String,
    pub scope: String,
    pub id: String,
    pub version: u64,
    pub length: u64,
    pub sealed: bool,
}
impl BlockHeader {
    pub fn body_ref(&self, source: &str, scope: &str) -> BodyRef {
        BodyRef { source: source.into(), scope: scope.into(), id: self.id.clone(),
            version: self.version, length: self.length, sealed: self.sealed }
    }
}

/// Native addresses and public tool metadata; provider call IDs are not UI identity.
pub fn tool_input_id(tool: &str) -> String { format!("{tool}/input") }

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolState { Writing, Running, Completed, Failed, Interrupted }
impl ToolState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Writing => "writing", Self::Running => "running", Self::Completed => "completed",
            Self::Failed => "failed", Self::Interrupted => "interrupted",
        }
    }
    pub fn finished(self) -> bool { matches!(self, Self::Completed | Self::Failed | Self::Interrupted) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolBody { Input, Output, Error }
impl ToolBody {
    /// Also the persisted disclosure suffix, not a decoder for block identity.
    pub fn label(self) -> &'static str {
        match self { Self::Input => "Input", Self::Output => "Output", Self::Error => "Error" }
    }
}
impl BlockHeader {
    pub fn tool_state(&self) -> Option<ToolState> {
        if self.kind != BlockKind::Tool { return None; }
        match self.meta.get("toolState") {
            Some(state) => serde_json::from_value(state.clone()).ok(),
            None => Some(if self.sealed { ToolState::Running } else { ToolState::Writing }),
        }
    }
    /// Only textual input/results are tool bodies. Metadata and attachment payloads are not.
    pub fn tool_body(&self) -> Option<ToolBody> {
        if self.kind == BlockKind::Code && self.parent.is_some()
            && self.meta.get("inputFor").and_then(|v| v.as_str()) == self.parent.as_deref() {
            Some(ToolBody::Input)
        } else if self.meta.pointer("/event/role").and_then(|v| v.as_str()) == Some("tool")
            && self.meta.pointer("/event/kind").and_then(|v| v.as_str()) == Some("text")
            && !self.meta.pointer("/event/attachment").is_some_and(|v| v.is_object()) {
            Some(if self.meta.pointer("/event/isError").and_then(|v| v.as_bool()) == Some(true) {
                ToolBody::Error
            } else { ToolBody::Output })
        } else { None }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FeedCursor { pub lineage: String, pub sequence: u64 }

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "camelCase")]
pub struct FeedPosition { pub order:u64, pub id:String }

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedRequest {
    pub scope: String,
    pub parent: Option<String>,
    pub cursor: Option<FeedCursor>,
    /// The loaded window's lower bound; older history is requested explicitly.
    pub floor: u64,
    /// A finite older page. None selects/follows the current tail.
    pub before: Option<FeedPosition>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum BlockRecord {
    Put { block: BlockHeader },
    Remove { id: String, revision: u64 },
}
impl BlockRecord {
    pub fn revision(&self) -> u64 {
        match self { Self::Put { block } => block.revision, Self::Remove { revision, .. } => *revision }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedPage {
    pub reset: bool,
    pub records: Vec<BlockRecord>,
    pub cursor: FeedCursor,
    pub floor: u64,
    pub before: Option<FeedPosition>,
    pub more: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockRequest {
    pub scope: String,
    pub id: String,
    pub version: u64,
    pub offset: u64,
    pub follow: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum BlockWatch { Feed(FeedRequest), Feeds { requests:Vec<FeedRequest> }, Block(BlockRequest) }

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BulkOffer { pub node_id: String, pub port: u16, #[serde(default)] pub port_v6:Option<u16>, pub lineage: String }

/// Immutable, verified input. Retrying an ID with a different specification is
/// an error, including after a disconnect or a process restart.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UploadSpec {
    pub id: String,
    pub length: u64,
    pub hash: String,
    pub purpose: UploadPurpose,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum UploadPurpose { Command, File { session_id: String, file_name: String } }
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadStatus {
    pub offset: u64,
    pub sealed: bool,
    pub file: Option<crate::UploadedFile>,
}

/// References are scoped to an authenticated database lineage, never just a
/// hash. Both length and raw BLAKE3 are checked before decoding a descriptor.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContentRef {
    pub lineage: String,
    pub scope: String,
    pub id: String,
    pub length: u64,
    pub hash: String,
}
