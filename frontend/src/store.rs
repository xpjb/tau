//! Local work only. Remote transcripts intentionally never enter SQLite.
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
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

#[derive(Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct Account {
    #[serde(default)] pub source_lineage:Option<String>,
    pub create_blocked:bool,
    pub projects: Vec<Project>,
    pub selected_project: String,
    /// Per-topic resume target. Kept client-local; server membership is authoritative.
    pub last_chat_by_project: BTreeMap<String, String>,
    pub sessions: Vec<SessionSummary>,
    pub selected: Option<String>,
    pub read_at: BTreeMap<String, u64>,
    pub pending_create: Option<ClientRequest>,
    pub pending_controls:BTreeMap<String,PendingControl>,
    #[serde(default)] pub missing_chats:BTreeSet<String>,
}
impl Default for Account {
    fn default() -> Self {
        Self { missing_chats:BTreeSet::new(),source_lineage:None,create_blocked:false,projects: vec![Project::general()], selected_project: general_project_id(),
            last_chat_by_project: BTreeMap::new(), sessions: vec![], selected: None, read_at: BTreeMap::new(), pending_create: None, pending_controls:BTreeMap::new() }
    }
}
/// Complete immutable intent, saved before control or input-upload submission.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PendingControl {
    pub request:ClientRequest,
    pub deleted_chats:Vec<String>,
    pub blocked:bool,
    #[serde(default)] pub accepted:bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LocalFile {
    pub id: String,
    pub name: String,
    pub size: u64,
    pub path: PathBuf,
    #[serde(default)] pub hash:Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Delivery {
    WaitingForChat,
    WaitingForConnection,
    WaitingForModel,
    Preparing,
    Sending,
    Accepted,
    Unconfirmed,
    Rejected,
}
impl Delivery {
    pub fn label(&self) -> &'static str {
        match self {
            Self::WaitingForChat => "Waiting for chat creation",
            Self::WaitingForConnection => "Waiting for connection",
            Self::WaitingForModel => "Waiting for model selection",
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
    #[serde(default)]
    pub started_at_ms: Option<u64>,
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
        self.reconcile_complete(queue,delivered,&HashSet::new());
    }
    pub fn reconcile_complete(&mut self, queue:&QueueState, delivered:&[String], incomplete:&HashSet<String>) {
        self.pending.retain(|p| {
            if matches!(p.status,Delivery::Rejected) {return true;}
            match &p.request.command {
                ClientCommand::QueueControl {operation:QueueOperation::Edit {request_id,revision,text},..} if p.status==Delivery::Accepted => {
                    if !queue.available {return true;}
                    queue.requests.iter().find(|q|&q.request_id==request_id).is_some_and(|q|
                        q.revision<=*revision || incomplete.contains(&format!("queued:{request_id}")) || q.revision==revision+1 && &q.text!=text)
                }
                ClientCommand::QueueControl {operation:QueueOperation::Delete {request_id,..},..} if p.status==Delivery::Accepted =>
                    !queue.available || queue.requests.iter().any(|q|&q.request_id==request_id),
                _ => !delivered.contains(&p.request.id) && !queue.requests.iter().any(|q|q.request_id==p.request.id)
                    && !queue.control.as_ref().is_some_and(|c|c.command_id==p.request.id),
            }
        });
    }

}

pub struct Store {
    db: Connection,
    pub root: PathBuf,
}
impl Store {
    pub fn block_cache(&self, identity: &str) -> Result<crate::blocks::Cache> {
        use sha2::Digest;
        let key = format!("{:x}",sha2::Sha256::digest(identity.as_bytes()));
        crate::blocks::Cache::open(&self.root.join("blocks").join(format!("{key}.sqlite3")))
    }
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
        ensure!(version <= 2, "Local state needs a newer Tau client");
        db.execute_batch("CREATE TABLE IF NOT EXISTS local (account TEXT NOT NULL, key TEXT NOT NULL, value TEXT NOT NULL, PRIMARY KEY(account,key)); CREATE TABLE IF NOT EXISTS chat_aliases(account TEXT NOT NULL,old TEXT NOT NULL,new TEXT NOT NULL,PRIMARY KEY(account,old)); PRAGMA user_version=2;")?;
        let page_size:u64=db.query_row("PRAGMA page_size",[],|r|r.get(0))?;db.pragma_update(None,"max_page_count",512u64*1024*1024/page_size)?;
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
    pub fn bind_source(&self,account:&str,lineage:&str)->Result<bool> {
        let tx=self.db.unchecked_transaction()?;
        let mut state:Account=self.get(account,"account")?;
        if state.source_lineage.as_deref()==Some(lineage) {return Ok(false);}
        let changed=state.source_lineage.is_some();state.source_lineage=Some(lineage.into());
        if changed {
            state.create_blocked=true;
            for saved in state.pending_controls.values_mut() {saved.blocked=true;}
            let ids=self.db.prepare("SELECT substr(key,6) FROM local WHERE account=?1 AND key LIKE 'chat:%'")?.query_map([account],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            for id in ids {
                let mut chat=self.load_chat(account,&id)?;
                for pending in &mut chat.pending {pending.status=Delivery::Unconfirmed;pending.detail=Some("Source lineage changed. Inspect the restored source; explicitly retry only if execution is intended.".into());}
                tx.execute("UPDATE local SET value=?3 WHERE account=?1 AND key=?2",params![account,format!("chat:{id}"),serde_json::to_string(&chat)?])?;
            }
        }
        tx.execute("INSERT INTO local(account,key,value) VALUES(?1,'account',?2) ON CONFLICT(account,key) DO UPDATE SET value=excluded.value",params![account,serde_json::to_string(&state)?])?;
        tx.commit()?;Ok(changed)
    }
    pub fn work_chats(&self,account:&str)->Result<Vec<String>> {
        Ok(self.db.prepare("SELECT substr(key,6) FROM local WHERE account=?1 AND key LIKE 'chat:%' AND (length(json_extract(value,'$.draft'))>0 OR json_array_length(value,'$.files')>0 OR json_array_length(value,'$.pending')>0)")?
            .query_map([account],|r|r.get(0))?.collect::<rusqlite::Result<_>>()?)
    }
    pub fn discard_import(&self,account:&str,session:&str,file:&LocalFile)->Result<()> {
        let expected=self.root.join("files").join(hash(account)).join(hash(&self.resolve_chat(account,session)?)).join(&file.id);
        ensure!(file.path==expected,"Refusing to delete a file outside this chat's owned imports");
        match std::fs::remove_file(expected) {Ok(())=>Ok(()),Err(e) if e.kind()==std::io::ErrorKind::NotFound=>Ok(()),Err(e)=>Err(e.into())}
    }
    pub fn resolve_chat(&self,account:&str,session:&str)->Result<String> {
        let mut id=session.to_owned();let mut seen=HashSet::new();
        loop {
            ensure!(seen.insert(id.clone()) && seen.len()<=32,"Invalid local chat alias chain");
            let next=self.db.query_row("SELECT new FROM chat_aliases WHERE account=?1 AND old=?2",params![account,id],|r|r.get::<_,String>(0)).optional()?;
            match next {Some(next)=>id=next,None=>return Ok(id)}
        }
    }
    pub fn load_chat(&self, account: &str, session: &str) -> Result<LocalChat> {
        let session=self.resolve_chat(account,session)?;
        let size:Option<u64>=self.db.query_row("SELECT length(CAST(value AS BLOB)) FROM local WHERE account=?1 AND key=?2",params![account,format!("chat:{session}")],|r|r.get(0)).optional()?;
        ensure!(size.unwrap_or(0)<=32*1024*1024,"Local chat exceeds 32 MiB; preserve/export the authored store before repair");
        let mut chat: LocalChat = self.get(account, &format!("chat:{session}"))?;
        for p in &mut chat.pending {
            if matches!(p.status, Delivery::Sending | Delivery::Preparing) {
                p.status = Delivery::Unconfirmed;
                p.detail = Some(
                    "The app stopped before confirmation. Check history before sending again."
                        .into(),
                );
            } else if p.status == Delivery::WaitingForModel {
                p.status = Delivery::Rejected;
                p.detail = Some("Model selection was not confirmed; check the current model, then restore this draft".into());
            }
        }
        Ok(chat)
    }
    pub fn save_chat(&self, account: &str, session: &str, chat: &LocalChat) -> Result<()> {
        let session=self.resolve_chat(account,session)?;
        ensure!(chat.pending.len()<=256 && chat.files.len()<=8,"Reconcile pending work before saving more (256 intents / eight draft attachments)");
        struct Count(usize);
        impl std::io::Write for Count {fn write(&mut self,bytes:&[u8])->std::io::Result<usize> {self.0+=bytes.len();if self.0>32*1024*1024 {return Err(std::io::Error::other("Local chat exceeds 32 MiB; reconcile saved work before adding more"));}Ok(bytes.len())}fn flush(&mut self)->std::io::Result<()> {Ok(())}}
        serde_json::to_writer(Count(0),chat)?;
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
        crate::disk::admit_import(&self.root.join("files"),meta.len())?;
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
        let mut output=tempfile::NamedTempFile::new_in(&directory)?;
        let (size,hash)=copy_bytes(&mut input,&mut output,MAX_UPLOAD_BYTES as u64+1)?;
        let after=input.metadata()?;
        ensure!(meta.len()==after.len() && meta.modified()?==after.modified()?,"Attachment changed while importing");
        if size == 0 || size > MAX_UPLOAD_BYTES as u64 {
            drop(output);
            let _ = std::fs::remove_file(&path);
            anyhow::bail!("File changed size during import");
        }
        output.as_file().sync_all()?;output.persist(&path)?;
        #[cfg(unix)] {
            let mut at=directory.as_path();
            loop {std::fs::File::open(at)?.sync_all()?;if at==self.root {break;}at=at.parent().context("Attachment directory escaped local storage")?;}
        }
        let name = name
            .map(str::to_owned)
            .or_else(|| source.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "attachment".into());
        Ok(LocalFile {
            id,
            name,
            size,
            path,hash:Some(hash),
        })
    }
    /// Move an optimistic chat's local intent to the daemon-confirmed ID.
    /// Copy attachment bytes before changing the SQLite record; a failure leaves
    /// the original local record and files intact for recovery.
    pub fn merge_chat(&self, account: &str, from: &str, into: &str,
        source: &LocalChat, loaded_target: Option<&LocalChat>) -> Result<LocalChat> {
        if self.resolve_chat(account,from)?==into {return self.load_chat(account,into);}
        let mut source = source.clone();
        // A live chat's Sending receipts must not be reclassified as uncertain
        // merely because this local-only merge read them from disk.
        let mut target = if let Some(chat) = loaded_target { chat.clone() } else { self.load_chat(account, into)? };
        let files = self.root.join("files").join(hash(account)).join(hash(into));
        let copy = |file: &mut LocalFile| -> Result<()> {
            let dest = files.join(&file.id);
            if dest != file.path {
                std::fs::create_dir_all(&files)?;
                let mut source=std::fs::File::open(&file.path)?;let mut temp=tempfile::NamedTempFile::new_in(&files)?;
                let (size,hash)=copy_bytes(&mut source,temp.as_file_mut(),MAX_UPLOAD_BYTES as u64+1)?;
                ensure!(size==file.size && file.hash.as_ref().is_none_or(|expected|expected==&hash),"Local attachment failed integrity verification");
                temp.as_file().sync_all()?;temp.persist(&dest)?;
                #[cfg(unix)] {std::fs::File::open(&files)?.sync_all()?;std::fs::File::open(files.parent().unwrap())?.sync_all()?;}
                file.hash=Some(hash);
                file.path = dest;
            }
            Ok(())
        };
        for file in &mut source.files { copy(file)?; }
        for pending in &mut source.pending {
            for file in &mut pending.files { copy(file)?; }
            if let ClientCommand::Prompt { session_id, .. } = &mut pending.request.command { *session_id = into.into(); }
        }
        if !source.draft.is_empty() || !source.files.is_empty() {
            if target.draft.is_empty() && target.files.is_empty() {target.draft=source.draft;target.files=source.files;}
            else if target.draft!=source.draft || target.files.iter().map(|f|&f.id).collect::<Vec<_>>()!=source.files.iter().map(|f|&f.id).collect::<Vec<_>>() {
                // A draft and its attachments are one authored bundle. Do not
                // silently attach the provisional files to a different draft.
                target.pending.push(Pending {request:ClientRequest {id:uuid::Uuid::new_v4().to_string(),command:ClientCommand::Prompt {session_id:into.into(),text:source.draft.clone()}},started_at_ms:None,text:source.draft,files:source.files,status:Delivery::Rejected,detail:Some("Another local draft was present; restore this draft and its attachments explicitly".into())});
            }
        }
        let pending_ids: HashSet<_> = target.pending.iter().map(|p| p.request.id.clone()).collect();
        target.pending.extend(source.pending.into_iter().filter(|p| !pending_ids.contains(&p.request.id)));
        let tx=self.db.unchecked_transaction()?;
        self.save_chat(account,into,&target)?;
        tx.execute("DELETE FROM local WHERE account=?1 AND key=?2",params![account,format!("chat:{from}")])?;
        tx.execute("INSERT INTO chat_aliases VALUES(?1,?2,?3)",params![account,from,into])?;
        tx.commit()?;
        let old = self.root.join("files").join(hash(account)).join(hash(from));
        if old.exists() { let _ = std::fs::remove_dir_all(old); }
        Ok(target)
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

fn copy_bytes(input:&mut impl std::io::Read,output:&mut impl std::io::Write,limit:u64)->Result<(u64,String)> {
    let mut hash=blake3::Hasher::new();let mut count=0;let mut buffer=[0;32*1024];
    while count<limit {
        let n=input.read(&mut buffer[..(limit-count).min(32*1024) as usize])?;if n==0 {break;}
        output.write_all(&buffer[..n])?;hash.update(&buffer[..n]);count+=n as u64;
    }
    Ok((count,hash.finalize().to_hex().to_string()))
}

#[cfg(all(test,target_os="linux"))]
mod disk_fault_tests {
    #[test]
    fn kernel_enospc_aborts_the_bounded_import_copy() {
        let mut input=std::io::Cursor::new(vec![5;32768]);let mut full=std::fs::OpenOptions::new().write(true).open("/dev/full").unwrap();
        let error=super::copy_bytes(&mut input,&mut full,50000).unwrap_err();assert!(format!("{error:#}").contains("No space left"));
    }
}
