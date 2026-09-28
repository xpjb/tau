//! Ephemeral, authenticated filesystem reads. CWD is a starting point, not a jail.
use serde::{Deserialize, Serialize};

pub const MAX_FILE_LINES: usize = 100_000;
pub const MAX_FILE_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_FILE_REPLY_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_PATH_BYTES: usize = 2048;
pub const DIRECTORY_PAGE: usize = 256;
pub const SEARCH_RESULTS: usize = 100;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileRequest {
    pub session_id: String,
    /// None resolves the chat's current working directory on the daemon.
    pub path: Option<String>,
    pub operation: FileOperation,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum FileOperation {
    List { after: Option<String> },
    Open { revision: Option<String> },
    Search { query: String },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry { pub path: String, pub name: String, pub directory: bool, pub symlink: bool }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum FileReply {
    Directory { path: String, parent: Option<String>, entries: Vec<FileEntry>, next: Option<String> },
    Search { path: String, entries: Vec<FileEntry>, indexing: bool, limited: bool },
    Text { path: String, revision: String, text: String },
    Unchanged { path: String, revision: String },
}
impl FileReply {
    pub fn path(&self) -> &str { match self {
        Self::Directory { path, .. } | Self::Search { path, .. } | Self::Text { path, .. } | Self::Unchanged { path, .. } => path,
    } }
}
