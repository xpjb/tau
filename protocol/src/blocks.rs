//! Request-driven display blocks. A parent is an address, never an embedded tree.
//! Content bytes are separate from bounded headers and use the same range protocol
//! for text, code, tool input/output and files.
use serde::{Deserialize, Serialize};

pub const BLOCK_CHUNK_BYTES: usize = 16 * 1024;
pub const MAX_BLOCK_HEADER_BYTES: usize = 4096;
pub const MAX_BLOCK_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_FEED_PAGE: usize = 32;
pub const BLOCK_WINDOW_BYTES: u32 = 64 * 1024;

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
pub struct BulkOffer { pub node_id: String, pub port: u16, pub lineage: String }
