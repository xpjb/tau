//! Native resumable upload publication. Chunk commits and final file exports are
//! bounded; no HTTP body, per-file endpoint, or runtime load participates.
use anyhow::{Context, Result, ensure};
use tau_blocks::*;
use tokio::io::AsyncWriteExt;
use crate::manager::{AgentManager, safe_file_name};

impl AgentManager {
    pub(crate) async fn begin_upload(&self, spec: UploadSpec) -> Result<UploadStatus> {
        self.inner.state.access(move |db| {
            if let UploadPurpose::File {session_id,..} = &spec.purpose {
                ensure!(db.query_row("SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1)",[session_id],|r|r.get::<_,bool>(0))?,"Chat no longer exists");
            }
            let tx = db.transaction()?;
            let result = tau_blocks::uploads::begin(&tx,&spec)?;
            tx.commit()?;
            Ok(result)
        }).await
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
        let _permit = self.inner.block_imports.acquire().await?;
        let current = self.begin_upload(spec.clone()).await?;
        if current.sealed { return Ok(current); }
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
        self.inner.state.access(move |db| {
            let tx = db.transaction()?;
            if let UploadPurpose::File {session_id,..} = &spec.purpose {
                ensure!(tx.query_row("SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1)",[session_id],|r|r.get::<_,bool>(0))?,"Chat was deleted during upload");
            }
            let status = tau_blocks::uploads::seal(&tx,&spec,&hash,file)?;
            tx.commit()?; Ok(status)
        }).await
    }
}
