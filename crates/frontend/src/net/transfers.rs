//! Downloads, resumable input uploads and descriptor reads on the shared peer.
use super::{Event, mailbox::EventSender};
use crate::{replica::Cache, store::LocalFile};
use anyhow::{Context, Result, ensure};
use std::{path::{Path, PathBuf}, time::{Duration, Instant}};
use tau_net::{blocks::*, native::{Client, Priority, Update}, TransferStatus, ClientCommand, ClientRequest, MAX_PROMPT_CHARS};
use tokio::{sync::watch, io::AsyncReadExt};

#[derive(Clone)]
pub(super) struct Transfers {
    pub cache: Cache,
    pub client: Client,
    pub ready: watch::Receiver<Option<String>>,
    pub events: EventSender,
}
impl Transfers {
    fn report(&self, key:&str, path:&Path, status:&TransferStatus) {
        self.events.send(Event::Download {key:key.into(),path:path.into(),status:status.clone()});
    }
    pub async fn run(mut self, key:String, scope:String, id:String, path:PathBuf, limit:u64, mut cancel:watch::Receiver<bool>) {
        let mut status=TransferStatus {transferred:0,total:0,network_bytes:0,done:false,failure:None};
        let cancellation=cancel.clone();
        let result=tokio::select! {
            result=self.fetch(&key,&scope,&id,&path,limit,&cancellation,&mut status) => result,
            _=async {while !*cancel.borrow() {if cancel.changed().await.is_err() {break;}}} => Err(anyhow::anyhow!("Download cancelled")),
        };
        status.done=true; status.failure=result.err().map(|e|e.to_string());
        self.report(&key,&path,&status);
    }
    async fn fetch(&mut self, key:&str, scope:&str, id:&str, path:&Path, limit:u64, cancel:&watch::Receiver<bool>, status:&mut TransferStatus) -> Result<()> {
        ensure!(limit<=MAX_BLOCK_BYTES,"Download limit exceeds the block limit");
        let epoch=self.cache.epoch();
        let original_lineage=self.cache.lineage()?;
        let mut request=self.cache.block_request(scope,id)?;
        let cached=self.cache.cached_header(scope,id)?;
        let complete=cached.as_ref().is_some_and(|h|h.sealed && h.length==request.offset);
        let (header,lineage)=if complete {
            (cached.unwrap(),self.cache.lineage()?)
        } else {
            while self.ready.borrow().is_none() {self.ready.changed().await.context("Content service stopped")?;}
            let lineage=self.ready.borrow().clone().unwrap();
            ensure!(lineage==original_lineage,"Content source changed; retry the download");
            let client=self.client.clone();
            let mut failures=0;
            let mut delay=100;
            let h=loop {
                ensure!(self.cache.epoch()==epoch && self.ready.borrow().as_ref()==Some(&lineage),
                    "Content source or replica changed; retry the download");
                request=self.cache.block_request(scope,id)?; request.follow=false;
                if let Some(h)=self.cache.cached_header(scope,id)? && h.sealed && h.length==request.offset {break h;}
                let before=request.offset;
                let result=async {
                    let mut reader=client.read(BlockWatch::Block(request.clone()),Priority::Background).await?;
                    let mut head=None;let mut reported=Instant::now()-Duration::from_secs(1);
                    let previous_network=status.network_bytes;
                    status.transferred=request.offset;
                    loop {
                        let update=reader.next().await;
                        status.network_bytes=previous_network+reader.received_bytes();
                        let Some(update)=update? else {break;};
                        match &update {
                            Update::Block {block,..}=>{
                                ensure!(matches!(block.kind,BlockKind::File|BlockKind::Image) && block.length<=limit,"Invalid file header or size");
                                if request.version!=block.version {status.transferred=0;request.version=block.version;}
                                status.total=block.length;head=Some(block.clone());
                            }
                            Update::Range {range,..}=>status.transferred=range.offset+range.bytes.len() as u64,
                            Update::Absent {..}=>anyhow::bail!("Source file no longer exists"),
                            Update::Page {..}=>unreachable!(),
                        }
                        self.cache.apply(&lineage,update,epoch).await?;
                        if reported.elapsed()>=Duration::from_millis(100) {self.report(key,path,status);reported=Instant::now();}
                    }
                    let h=head.context("File response had no header")?;
                    ensure!(h.sealed && status.transferred==h.length,"File transfer is incomplete");
                    Ok::<_,anyhow::Error>(h)
                }.await;
                match result {
                    Ok(h)=>break h,
                    Err(error)=>{
                        if !tau_net::native::is_connection_error(&error) {return Err(error);}
                        if status.transferred>before {failures=0;delay=100;}
                        failures+=1;
                        if failures>=8 {return Err(error);}
                        // Only a disposable read resumes. Never retry corrupt
                        // content, local IO, changed sources or user actions.
                        log::trace!(target:"tau_native_watch", "file reconnect scope={scope} id={id} verified={} failures={failures}",status.transferred);
                        self.report(key,path,status);
                        tokio::time::sleep(Duration::from_millis(delay)).await;
                        delay=(delay*2).min(1000);
                    }
                }
            };
            (h,lineage)
        };
        ensure!(header.length<=limit && matches!(header.kind,BlockKind::File|BlockKind::Image),"Invalid file or download limit");
        status.total=header.length;status.transferred=header.length;
        let (cache,scope,target,cancel)=(self.cache.clone(),scope.to_owned(),path.to_owned(),cancel.clone());
        tokio::task::spawn_blocking(move ||cache.export(&lineage,&scope,&header,&target,&cancel)).await??;
        Ok(())
    }
}

#[derive(Debug)]
pub(crate) struct InvalidAttachment;
impl std::fmt::Display for InvalidAttachment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("Attachment needs attention") }
}
impl std::error::Error for InvalidAttachment {}

impl Transfers {
    async fn connection(&mut self) -> Result<(Client,String)> {
        while self.ready.borrow().is_none() {self.ready.changed().await.context("Content service stopped")?;}
        let lineage = self.ready.borrow().clone().unwrap();
        let client = self.client.clone();
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
                upload.copy_from(&mut std::io::Cursor::new(&bytes)).await?;
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
            let (mut source, before, hash) = async {
                let mut source = tokio::fs::File::open(&file.path).await?;
                let before = source.metadata().await?;
                ensure!(before.is_file() && before.len() == file.size && file.size <= tau_net::MAX_UPLOAD_BYTES as u64,"Attachment changed or exceeds 50 MB");
                let mut hasher = blake3::Hasher::new();
                let mut buffer = vec![0;BLOCK_CHUNK_BYTES];
                loop {let n=source.read(&mut buffer).await?;if n==0 {break;}hasher.update(&buffer[..n]);}
                ensure!(source.metadata().await?.modified()? == before.modified()?,"Attachment changed while hashing");
                let hash=hasher.finalize().to_hex().to_string();
                ensure!(file.hash.as_ref().is_none_or(|expected|expected==&hash),"Local attachment failed integrity verification; original intent was not sent");
                Ok::<_,anyhow::Error>((source, before, hash))
            }.await.context(InvalidAttachment)?;
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
                    upload.copy_from(&mut source).await?;
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
        if text.chars().count() > MAX_PROMPT_CHARS {
            return Err(anyhow::anyhow!("Prompt and attachment paths are too long")).context(InvalidAttachment);
        }
        Ok(text)
    }

    pub(crate) async fn descriptor(mut self, reference: ContentRef) -> Result<tau_net::ServerMessage> {
        ensure!(reference.scope == CONTROL_SCOPE && reference.length <= MAX_BLOCK_BYTES,"Invalid data descriptor");
        let (client,lineage) = self.connection().await?;
        ensure!(reference.lineage == lineage,"Descriptor belongs to an old data source");
        let epoch=self.cache.epoch();
        let mut request = self.cache.block_request(&reference.scope,&reference.id)?;
        request.follow = false;
        let mut reader=client.read(BlockWatch::Block(request),Priority::Descriptor).await?;
        while let Some(update)=reader.next().await? {
            match &update {
                Update::Block {block,..}=>ensure!(block.length==reference.length && block.sealed && block.version==1,"Descriptor changed"),
                Update::Absent {..}=>anyhow::bail!("Descriptor no longer exists"),
                _=>{}
            }
            self.cache.apply(&lineage,update,epoch).await?;
        }
        let cache=self.cache.clone();
        tokio::task::spawn_blocking(move || cache.descriptor(&reference,epoch)).await?
    }
}

#[cfg(test)]
#[path = "../../tests/unit/net/transfers.rs"]
mod tests;
