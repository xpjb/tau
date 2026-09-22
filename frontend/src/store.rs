//! Local work only. Remote transcripts intentionally never enter SQLite.
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
use tau_protocol::*;

#[derive(Clone, Default, Deserialize, Serialize)]
pub struct Settings {
    pub server_url: String,
    pub token: String,
}
impl Settings {
    pub fn normalized(mut self) -> Self {
        self.server_url = self.server_url.trim().trim_end_matches('/').to_owned();
        self.token = self.token.trim().to_owned();
        self
    }
    pub fn url(&self) -> Result<url::Url> {
        let url = url::Url::parse(self.server_url.trim().trim_end_matches('/'))
            .map_err(|_| anyhow::anyhow!("Enter the daemon URL, including http:// or https:// (for example http://vibe:8787)."))?;
        ensure!(
            matches!(url.scheme(), "http" | "https") && url.host_str().is_some(),
            "Use an http:// or https:// Tau URL"
        );
        ensure!(
            url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none(),
            "URL must not contain credentials, a query, or a fragment"
        );
        ensure!(
            !self.token.is_empty() && self.token.bytes().all(|b| (33..=126).contains(&b)),
            "Enter the Tau bearer token (without spaces)"
        );
        Ok(url)
    }
    pub fn identity(&self) -> String {
        hash(&format!(
            "{}\0{}",
            self.server_url.trim().trim_end_matches('/'),
            self.token
        ))
    }
}
pub fn hash(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Account {
    pub sessions: Vec<SessionSummary>,
    pub selected: Option<String>,
    pub read_at: BTreeMap<String, u64>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LocalFile {
    pub id: String,
    pub name: String,
    pub size: u64,
    pub path: PathBuf,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Delivery {
    Preparing,
    Sending,
    Accepted,
    Unconfirmed,
    Rejected,
}
impl Delivery {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Preparing => "Preparing attachments",
            Self::Sending => "Sending",
            Self::Accepted => "Accepted",
            Self::Unconfirmed => "Delivery unconfirmed — not resent",
            Self::Rejected => "Not sent",
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Pending {
    pub request: ClientRequest,
    pub text: String,
    pub files: Vec<LocalFile>,
    pub status: Delivery,
    pub detail: Option<String>,
}
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Position {
    pub key: Option<String>,
    pub offset: f32,
    pub follow: bool,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct LocalChat {
    pub draft: String,
    pub files: Vec<LocalFile>,
    pub pending: Vec<Pending>,
    pub expanded: BTreeSet<String>, // Legacy flat details preferences.
    pub expansion: BTreeMap<String, bool>,
    pub details_default: bool,
    pub position: Position,
}
impl Default for LocalChat {
    fn default() -> Self {
        Self {
            draft: String::new(),
            files: vec![],
            pending: vec![],
            expanded: BTreeSet::new(),
            expansion: BTreeMap::new(),
            details_default: false,
            position: Position {
                follow: true,
                ..Default::default()
            },
        }
    }
}
impl LocalChat {
    pub fn has_work(&self) -> bool {
        !self.draft.is_empty() || !self.files.is_empty() || !self.pending.is_empty()
    }
    pub fn reconcile(&mut self, queue: &QueueState, delivered: &[String]) {
        self.pending.retain(|p| {
            !delivered.contains(&p.request.id)
                && !queue.requests.iter().any(|q| q.request_id == p.request.id)
        });
        if let Some(control) = &queue.control {
            self.pending.retain(|p| p.request.id != control.command_id);
        }
    }
}

pub struct Store {
    db: Connection,
    pub root: PathBuf,
}
impl Store {
    pub fn open(root: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&root)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))?;
        }
        let root = root.canonicalize()?;
        let db = Connection::open(root.join("client.sqlite3"))?;
        db.busy_timeout(std::time::Duration::from_secs(5))?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")?;
        let version: u32 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        ensure!(version <= 1, "Local state needs a newer Tau client");
        db.execute_batch("CREATE TABLE IF NOT EXISTS local (account TEXT NOT NULL, key TEXT NOT NULL, value TEXT NOT NULL, PRIMARY KEY(account,key)); PRAGMA user_version=1;")?;
        Ok(Self { db, root })
    }
    pub fn get<T: DeserializeOwned + Default>(&self, account: &str, key: &str) -> Result<T> {
        let value: Option<String> = self
            .db
            .query_row(
                "SELECT value FROM local WHERE account=? AND key=?",
                params![account, key],
                |r| r.get(0),
            )
            .optional()?;
        value
            .map(|s| {
                serde_json::from_str(&s)
                    .context("Local state could not be decoded; original database was not replaced")
            })
            .transpose()
            .map(Option::unwrap_or_default)
    }
    pub fn put<T: Serialize>(&self, account: &str, key: &str, value: &T) -> Result<()> {
        self.db.execute("INSERT INTO local VALUES(?,?,?) ON CONFLICT(account,key) DO UPDATE SET value=excluded.value", params![account,key,serde_json::to_string(value)?])?;
        Ok(())
    }
    pub fn load_chat(&self, account: &str, session: &str) -> Result<LocalChat> {
        let mut chat: LocalChat = self.get(account, &format!("chat:{session}"))?;
        for p in &mut chat.pending {
            if matches!(p.status, Delivery::Sending | Delivery::Preparing) {
                p.status = Delivery::Unconfirmed;
                p.detail = Some(
                    "The app stopped before confirmation. Check history before sending again."
                        .into(),
                );
            }
        }
        Ok(chat)
    }
    pub fn save_chat(&self, account: &str, session: &str, chat: &LocalChat) -> Result<()> {
        self.put(account, &format!("chat:{session}"), chat)
    }
    pub fn attachment_path(&self, account: &str, session: &str, entry: &str) -> PathBuf {
        self.root
            .join("downloads")
            .join(hash(account))
            .join(hash(session))
            .join(hash(entry))
    }
    pub fn import(
        &self,
        account: &str,
        session: &str,
        source: &Path,
        name: Option<&str>,
    ) -> Result<LocalFile> {
        let meta = source.metadata()?;
        ensure!(
            meta.is_file() && meta.len() > 0 && meta.len() <= MAX_UPLOAD_BYTES as u64,
            "File must be between 1 byte and 50 MB"
        );
        let id = uuid::Uuid::new_v4().to_string();
        let directory = self
            .root
            .join("files")
            .join(hash(account))
            .join(hash(session));
        std::fs::create_dir_all(&directory)?;
        let path = directory.join(&id);
        // Bounded copy handles a source which grows after the metadata check.
        let mut input = std::fs::File::open(source)?;
        let mut output = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        let size = std::io::copy(
            &mut std::io::Read::take(&mut input, MAX_UPLOAD_BYTES as u64 + 1),
            &mut output,
        )?;
        if size == 0 || size > MAX_UPLOAD_BYTES as u64 {
            drop(output);
            let _ = std::fs::remove_file(&path);
            anyhow::bail!("File changed size during import");
        }
        output.sync_all()?;
        let name = name
            .map(str::to_owned)
            .or_else(|| source.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "attachment".into());
        Ok(LocalFile {
            id,
            name,
            size,
            path,
        })
    }
    pub fn delete_chat(&self, account: &str, session: &str) -> Result<()> {
        self.db.execute(
            "DELETE FROM local WHERE account=? AND key=?",
            params![account, format!("chat:{session}")],
        )?;
        for kind in ["files", "downloads"] {
            let dir = self.root.join(kind).join(hash(account)).join(hash(session));
            if dir.exists() {
                std::fs::remove_dir_all(dir)?;
            }
        }
        Ok(())
    }
}
