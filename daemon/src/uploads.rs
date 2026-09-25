//! Native resumable upload publication. Chunk commits and final file exports are
//! bounded; no HTTP body, per-file endpoint, or runtime load participates.
use anyhow::{Context, Result, ensure};
use tau_blocks::*;
use tokio::io::AsyncWriteExt;
use crate::manager::{AgentManager, safe_file_name};

impl AgentManager {
    pub(crate) async fn begin_upload(&self, spec: UploadSpec) -> Result<UploadStatus> {
        let expected=spec.clone();
        let status=self.inner.state.access(move |db| {
            if let UploadPurpose::File {session_id,..} = &spec.purpose {
                ensure!(db.query_row("SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1)",[session_id],|r|r.get::<_,bool>(0))?,"Chat no longer exists");
            }
            let tx = db.transaction()?;
            let result = tau_blocks::uploads::begin(&tx,&spec)?;
            tx.commit()?;
            Ok(result)
        }).await?;
        if status.sealed && matches!(expected.purpose,UploadPurpose::File {..}) {
            let _permit=self.inner.block_imports.acquire().await?;
            let published=status.file.as_ref().context("Sealed upload has no owned file")?;
            let mut file=crate::attachments::regular_file(std::path::Path::new(&published.path)).await?;
            let mut hash=blake3::Hasher::new();let mut length=0u64;let mut buffer=[0;BLOCK_CHUNK_BYTES];
            loop {
                use tokio::io::AsyncReadExt;
                let n=file.read(&mut buffer).await?;if n==0 {break;}
                length+=n as u64;ensure!(length<=expected.length,"Published upload changed");hash.update(&buffer[..n]);
            }
            ensure!(length==expected.length && hash.finalize().to_hex().as_str()==expected.hash,"Published upload failed integrity verification; preserve the local original");
        }
        Ok(status)
    }
    pub(crate) async fn write_upload(&self, spec: UploadSpec, offset: u64, bytes: Vec<u8>) -> Result<()> {
        self.inner.state.access(move |db| {
            let tx = db.transaction()?;
            tau_blocks::uploads::write(&tx,&spec,offset,&bytes)?;
            tx.commit()?;
            Ok(())
        }).await
    }
    pub(crate) async fn finish_upload(&self, spec: UploadSpec) -> Result<UploadStatus> {
        let gate={let mut gates=self.inner.upload_finishes.lock().await;gates.retain(|_,gate|gate.strong_count()>0);
            if let Some(gate)=gates.get(&spec.id).and_then(std::sync::Weak::upgrade) {gate} else {let gate=std::sync::Arc::new(tokio::sync::Mutex::new(()));gates.insert(spec.id.clone(),std::sync::Arc::downgrade(&gate));gate}};
        let finish=gate.lock_owned().await;
        let current = self.begin_upload(spec.clone()).await?;
        if current.sealed { return Ok(current); }
        let permit = self.inner.block_imports.clone().acquire_owned().await?;
        ensure!(current.offset == spec.length,"Upload is incomplete");
        let mut export = None;
        if let UploadPurpose::File {session_id,file_name} = &spec.purpose {
            tokio::fs::create_dir_all(&self.inner.config.upload_root).await?;
            let root = tokio::fs::canonicalize(&self.inner.config.upload_root).await?;
            let directory = root.join(session_id);
            tokio::fs::create_dir_all(&directory).await?;
            let directory = tokio::fs::canonicalize(directory).await?;
            ensure!(directory.starts_with(&root) && directory != root,"Unsafe upload directory");
            // The ID fixes the destination, so a lost final acknowledgement or
            // crash after rename never creates another attachment/path.
            let name = safe_file_name(file_name);
            let export_id=blake3::hash(&serde_json::to_vec(&spec)?).to_hex().to_string();
            let target = directory.join(format!("{export_id}-{name}"));
            let (id,session,path,size)=(spec.id.clone(),session_id.clone(),target.to_string_lossy().into_owned(),spec.length);
            self.inner.state.access(move |db| {
                let tx=db.transaction()?;
                use rusqlite::OptionalExtension;
                let existing:Option<(String,String,u64)>=tx.query_row("SELECT session,path,size FROM file_publications WHERE id=?1",[&id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
                if let Some(old)=existing {
                    ensure!(old==(session.clone(),path.clone(),size),"Retained publication ID is bound to a different file");
                } else {
                    let (count,bytes):(u64,u64)=tx.query_row("SELECT count(*),coalesce(sum(size),0) FROM file_publications",[],|r|Ok((r.get(0)?,r.get(1)?)))?;
                    ensure!(count<8192 && bytes+size<=4*1024*1024*1024,"Retained uploads quota reached; archive/delete chats before another upload");
                    tx.execute("INSERT INTO file_publications(id,session,path,size,created) VALUES(?1,?2,?3,?4,?5)",rusqlite::params![id,session,path,size,crate::agent::now_ms()/1000])?;
                }
                tx.execute("INSERT OR IGNORE INTO file_owners VALUES(?1,?2)",rusqlite::params![id,session])?;
                tx.commit()?;Ok(())
            }).await?;
            let temp = tempfile::NamedTempFile::new_in(&directory)?;
            let file = tokio::fs::File::from_std(temp.reopen()?);
            export = Some((temp,file,target,name,root));
        }
        let mut hash = blake3::Hasher::new();
        let mut offset = 0;
        while offset < spec.length {
            let id = spec.id.clone();
            let range = self.inner.state.access(move |db|tau_blocks::read(db,&BlockRequest {scope:UPLOAD_SCOPE.into(),id,version:1,offset,follow:false})).await?;
            ensure!(range.offset == offset && !range.bytes.is_empty(),"Upload has a missing range");
            hash.update(&range.bytes);
            if let Some((_,file,..)) = &mut export {file.write_all(&range.bytes).await?;}
            offset += range.bytes.len() as u64;
        }
        let hash = hash.finalize().to_hex().to_string();
        ensure!(offset == spec.length && hash == spec.hash,"Upload integrity check failed");
        // Publication is owned after complete verification. Cancelling the QUIC
        // stream cannot release its exclusion gates while rename/fsync is still
        // running on a blocking thread, nor orphan an untracked exported file.
        let manager=self.clone();let state=self.inner.state.clone();
        tokio::spawn(async move {
        let _permit=permit;let _finish=finish;let _publication=manager.inner.upload_publication.lock().await;
        if export.is_some() {
            let root=manager.inner.config.upload_root.clone();
            tokio::task::spawn_blocking(move ||->Result<()> {
                let mut pending=vec![root];let mut count=0u64;let mut size=0u64;
                while let Some(directory)=pending.pop() {for entry in std::fs::read_dir(directory)? {
                    let entry=entry?;count+=1;ensure!(count<=100000,"Retained upload entry quota reached");
                    let kind=entry.file_type()?;if kind.is_dir() {pending.push(entry.path());} else if kind.is_file() {size=size.saturating_add(entry.metadata()?.len());}
                    ensure!(size<=4*1024*1024*1024,"Retained upload byte quota reached; archive owned files before retrying");
                }}Ok(())
            }).await??;
        }
        let file = if let Some((temp,mut file,target,name,root)) = export {
            file.flush().await?; file.sync_all().await?; drop(file);
            let path = target.clone();
            tokio::task::spawn_blocking(move || -> Result<()> {
                temp.persist(&path)?;
                std::fs::File::open(path.parent().unwrap())?.sync_all()?;
                std::fs::File::open(&root)?.sync_all()?;
                if let Some(parent)=root.parent() {std::fs::File::open(parent)?.sync_all()?;}
                Ok(())
            }).await??;
            Some(tau_protocol::UploadedFile {name,path:target.to_str().context("Upload path is not UTF-8")?.into(),size:spec.length})
        } else {None};
        state.access(move |db| {
            let tx = db.transaction()?;
            if let UploadPurpose::File {session_id,..} = &spec.purpose {
                ensure!(tx.query_row("SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1)",[session_id],|r|r.get::<_,bool>(0))?,"Chat was deleted during upload");
            }
            if file.is_some() {tx.execute("UPDATE file_publications SET sealed=1 WHERE id=?1",[&spec.id])?;}
            let status = tau_blocks::uploads::seal(&tx,&spec,&hash,file)?;
            tx.commit()?; Ok(status)
        }).await
        }).await?
    }
}

impl AgentManager {
    pub(crate) async fn maintain_uploads(&self)->Result<()> {
        // Exclude complete native publication, including detached final fsyncs.
        let _permit=self.inner.block_imports.acquire_many(2).await?;
        let expired=crate::agent::now_ms()/1000-7*24*3600;
        let candidates=self.inner.state.access(move |db| {
            Ok(db.prepare("SELECT id,path FROM file_publications p WHERE NOT EXISTS(SELECT 1 FROM file_owners o WHERE o.id=p.id) OR (sealed=0 AND created<?1) LIMIT 128")?
                .query_map([expired],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?)
        }).await?;
        for (id,path) in candidates {
            let path=std::path::PathBuf::from(path);let root=&self.inner.config.upload_root;
            ensure!(path.starts_with(root) && path.strip_prefix(root)?.components().count()==2,"Quarantined upload outside the configured root; inspect storage manually");
            match tokio::fs::remove_file(&path).await {Ok(())=>{},Err(e) if e.kind()==std::io::ErrorKind::NotFound=>{},Err(e)=>return Err(e.into())}
            if let Some(parent)=path.parent() {let _=tokio::fs::remove_dir(parent).await;}
            self.inner.state.access(move |db| {
                let tx=db.transaction()?;tx.execute("DELETE FROM file_publications WHERE id=?1",[&id])?;
                tx.execute("DELETE FROM blocks WHERE scope='@uploads' AND id=?1",[&id])?;tx.commit()?;Ok(())
            }).await?;
        }
        self.inner.state.access(move |db| {
            let tx=db.transaction()?;
            tx.execute("DELETE FROM blocks WHERE scope='@uploads' AND position<?1",[expired])?;
            tx.execute("DELETE FROM blocks WHERE scope='@control' AND position<?1",[crate::agent::now_ms()/1000-24*3600])?;
            tx.execute("DELETE FROM block_changes WHERE scope='@control' AND id NOT IN (SELECT id FROM blocks WHERE scope='@control')",[])?;
            tx.commit()?;Ok(())
        }).await
    }
}
