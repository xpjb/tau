//! Ephemeral, authenticated filesystem reads. CWD is a starting point, not a jail.
use serde::{Deserialize, Serialize};

pub const MAX_FILE_LINES: usize = 100_000;
pub const MAX_FILE_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_FILE_REPLY_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_PATH_BYTES: usize = 2048;
pub const DIRECTORY_PAGE: usize = 256;
pub const MAX_INDEX_PATHS: usize = 200_000;
/// Leave room for JSON framing/escaping within MAX_FILE_REPLY_BYTES.
pub const MAX_INDEX_BYTES: usize = 24 * 1024 * 1024;

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
    /// Sync names independently of the query. Known revisions receive a delta,
    /// or no names at all when unchanged; unknown revisions receive a snapshot.
    Index { revision: Option<String> },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry { pub path: String, pub name: String, pub directory: bool, pub symlink: bool }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexedPath {
    /// UTF-8 path relative to the requested root, with `/` separators.
    pub path: String,
    pub symlink: bool,
}
impl IndexedPath {
    pub fn hidden(&self) -> bool { self.path.split('/').any(|part| part.starts_with('.')) }
    /// Conservative JSON record size for paths without control characters.
    pub fn wire_bytes(&self) -> usize { self.path.len() + self.path.bytes().filter(|b| matches!(b, b'"' | b'\\')).count() + 32 }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum FileReply {
    Directory { path: String, parent: Option<String>, entries: Vec<FileEntry>, next: Option<String> },
    Index { path: String, revision: String, base: Option<String>, entries: Vec<IndexedPath>, removed: Vec<String>, indexing: bool, limited: bool },
    Text { path: String, revision: String, text: String },
    Unchanged { path: String, revision: String },
}
impl FileReply {
    pub fn path(&self) -> &str { match self {
        Self::Index { path, .. } | Self::Directory { path, .. } | Self::Text { path, .. } | Self::Unchanged { path, .. } => path,
    } }
}
