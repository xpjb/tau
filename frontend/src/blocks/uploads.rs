//! Client-originated content uses the same managed connection as display/files.
use super::*;
use crate::store::LocalFile;
use tau_protocol::{ClientCommand, ClientRequest, MAX_PROMPT_CHARS};
use tokio::io::{AsyncReadExt, AsyncSeekExt};

impl files::Downloads {
    async fn connection(&mut self) -> Result<(Arc<Client>,String)> {
        while self.ready.borrow().is_none() {self.ready.changed().await.context("Content service stopped")?;}
        let lineage = self.ready.borrow().clone().unwrap();
        let client = self.client.borrow().clone().context("Content endpoint is unavailable")?;
        Ok((client,lineage))
    }
    pub(crate) async fn input(mut self, request: ClientRequest) -> Result<ClientRequest> {
        let bytes = serde_json::to_vec(&request)?;
        ensure!(bytes.len() as u64 <= MAX_COMMAND_BYTES,"Command exceeds its content limit");
        let (client,lineage) = self.connection().await?;
        let hash = blake3::hash(&bytes).to_hex().to_string();
        let spec = UploadSpec {id:hash.clone(),length:bytes.len() as u64,hash:hash.clone(),purpose:UploadPurpose::Command};
        for attempt in 0..4 {
            ensure!(self.ready.borrow().as_ref() == Some(&lineage),"Content source changed; reconcile before retrying");
            let result = async {
                let mut upload = client.uploader(spec.clone()).await?;
                while upload.status.offset < spec.length {
                    let start = upload.status.offset as usize;
                    upload.write(&bytes[start..bytes.len().min(start+BLOCK_CHUNK_BYTES)]).await?;
                }
                upload.finish().await?;
                Ok::<_,anyhow::Error>(())
            }.await;
            if let Err(error) = result {
                if attempt == 3 {return Err(error);}
                tokio::time::sleep(Duration::from_millis(200 << attempt)).await;
                continue;
            }
            return Ok(ClientRequest {id:request.id,command:ClientCommand::Input {content:ContentRef {lineage,scope:UPLOAD_SCOPE.into(),id:hash.clone(),length:spec.length,hash}}});
        }
        unreachable!()
    }
    pub(crate) async fn upload(mut self, session:String, mut text:String, files:Vec<LocalFile>) -> Result<String> {
        if text.trim().is_empty() {text = "Please inspect the attached files.".into();}
        if !files.is_empty() {text.push_str("\n\nAttached files are available at:\n");}
        let (client,lineage) = self.connection().await?;
        for file in files {
            let mut source = tokio::fs::File::open(&file.path).await?;
            let before = source.metadata().await?;
            ensure!(before.is_file() && before.len() == file.size && file.size <= tau_protocol::MAX_UPLOAD_BYTES as u64,"Attachment changed or exceeds 50 MB");
            let mut hasher = blake3::Hasher::new();
            let mut buffer = vec![0;BLOCK_CHUNK_BYTES];
            loop {let n=source.read(&mut buffer).await?;if n==0 {break;}hasher.update(&buffer[..n]);}
            ensure!(source.metadata().await?.modified()? == before.modified()?,"Attachment changed while hashing");
            let hash=hasher.finalize().to_hex().to_string();
            ensure!(file.hash.as_ref().is_none_or(|expected|expected==&hash),"Local attachment failed integrity verification; original intent was not sent");
            let spec = UploadSpec {
                id:blake3::hash(format!("file\0{session}\0{}",file.id).as_bytes()).to_hex().to_string(),
                length:file.size,hash,
                purpose:UploadPurpose::File {session_id:session.clone(),file_name:file.name.clone()},
            };
            let mut published = None;
            for attempt in 0..4 {
                ensure!(self.ready.borrow().as_ref() == Some(&lineage),"Content source changed; retry the attachment explicitly");
                let result = async {
                    let mut upload = client.uploader(spec.clone()).await?;
                    source.seek(std::io::SeekFrom::Start(upload.status.offset)).await?;
                    while upload.status.offset < spec.length {
                        let n = source.read(&mut buffer).await?;
                        ensure!(n > 0,"Attachment was truncated");
                        upload.write(&buffer[..n]).await?;
                    }
                    let after=source.metadata().await?;
                    ensure!(before.len()==after.len() && before.modified()?==after.modified()?,"Attachment changed during upload");
                    upload.finish().await?.file.context("Attachment was not published")
                }.await;
                match result {
                    Ok(file) => {published=Some(file);break;}
                    Err(error) if attempt == 3 => return Err(error),
                    Err(_) => tokio::time::sleep(Duration::from_millis(200 << attempt)).await,
                }
            }
            let file = published.context("Attachment upload failed")?;
            text.push_str(&format!("- {}: {}\n",file.name,file.path));
        }
        ensure!(text.chars().count() <= MAX_PROMPT_CHARS,"Prompt and attachment paths are too long");
        Ok(text)
    }

    pub(crate) async fn descriptor(mut self, reference: ContentRef) -> Result<tau_protocol::ServerMessage> {
        ensure!(reference.scope == CONTROL_SCOPE && reference.length <= MAX_BLOCK_BYTES,"Invalid data descriptor");
        let (client,lineage) = self.connection().await?;
        ensure!(reference.lineage == lineage,"Descriptor belongs to an old data source");
        let epoch=self.cache.epoch();
        let mut request = self.cache.block_request(&reference.scope,&reference.id)?;
        request.follow = false;
        let mut watcher = client.watch_descriptor(BlockWatch::Block(request)).await?;
        let mut head = None;
        loop {
            let (frame,n) = watcher.next().await?;
            match &frame.header {
                Header::Block {block} => {
                    ensure!(block.id == reference.id && block.length == reference.length && block.sealed && block.version == 1,"Descriptor changed");
                    self.cache.header_at(&lineage,&reference.scope,block,epoch)?;
                    head = Some(block.clone());
                }
                Header::Data {version,offset,hash,..} => {
                    let h = head.as_ref().context("Descriptor has no header")?;
                    ensure!(*version == h.version,"Descriptor version changed");
                    let range = ContentRange {header:h.clone(),offset:*offset,hash:hash.clone(),bytes:frame.decoded()?};
                    let (cache,scope,lineage) = (self.cache.clone(),reference.scope.clone(),lineage.clone());
                    tokio::task::spawn_blocking(move ||cache.range_at(&lineage,&scope,&range,epoch)).await??;
                }
                Header::End => break,
                Header::Error {message} => anyhow::bail!("{message}"),
                _ => anyhow::bail!("Unexpected descriptor frame"),
            }
            let _ = watcher.consumed(n).await;
        }
        let cache=self.cache.clone();
        tokio::task::spawn_blocking(move || {
            let db=cache.db.lock().unwrap();
            ensure!(super::replica_epoch(&db)?==epoch && tau_blocks::cursor(&db)?.lineage==lineage,"Descriptor source changed");
            let bytes=tau_blocks::cached_content(&db,&reference.scope,&reference.id)?;drop(db);
            ensure!(bytes.len() as u64==reference.length && blake3::hash(&bytes).to_hex().as_str()==reference.hash,"Descriptor integrity check failed");
            let message=serde_json::from_slice(&bytes)?;
            ensure!(!matches!(message,tau_protocol::ServerMessage::Data {..}|tau_protocol::ServerMessage::BlockConnection {..}),"Recursive descriptor");Ok(message)
        }).await?
    }
}
