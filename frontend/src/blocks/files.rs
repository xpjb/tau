//! Files use the same endpoint, connection, verified chunks and resume offsets as
//! live text. There is no per-download endpoint, HTTP grant or worker runtime.
use super::*;
use std::{io::{Read, Write}, path::{Path, PathBuf}, time::Instant};
use tau_transfer::TransferStatus;

#[derive(Clone)]
pub(crate) struct Downloads {
    pub cache:Cache,
    pub client:watch::Receiver<Option<Arc<Client>>>,
    pub ready:watch::Receiver<Option<String>>,
    pub notices:mpsc::Sender<Notice>,
    pub wake:crate::transport::Wake,
}
impl Cache {
    pub fn lineage(&self) -> Result<String> { Ok(tau_blocks::cursor(&self.db.lock().unwrap())?.lineage) }
    fn file_header(&self, scope:&str, id:&str) -> Result<Option<BlockHeader>> { tau_blocks::header(&self.db.lock().unwrap(),scope,id) }
    pub fn file_ready(&self, scope:&str, id:&str, path:&Path, limit:u64) -> Result<bool> {
        let Some(h)=self.file_header(scope,id)? else {return Ok(false);};
        let Some(expected)=h.meta.get("sha256").and_then(|v|v.as_str()) else {return Ok(false);};
        if !h.sealed || h.length>limit || !path.metadata().is_ok_and(|m|m.is_file() && m.len()==h.length) {return Ok(false);}
        use sha2::Digest;
        let mut file=std::fs::File::open(path)?; let mut buffer=[0;BLOCK_CHUNK_BYTES]; let mut hash=sha2::Sha256::new();
        loop {let n=file.read(&mut buffer)?;if n==0 {break;} hash.update(&buffer[..n]);}
        let valid=format!("{:x}",hash.finalize())==expected;
        if valid && self.exports.as_ref().is_some_and(|root|path.starts_with(root)) && let Ok(file)=std::fs::OpenOptions::new().write(true).open(path) {let _=file.set_times(std::fs::FileTimes::new().set_modified(std::time::SystemTime::now()));}
        Ok(valid)
    }
    fn export(&self, lineage:&str, scope:&str, header:&BlockHeader, target:&Path, cancel:&watch::Receiver<bool>) -> Result<()> {
        use sha2::Digest;
        let epoch=self.epoch();
        let parent=target.parent().context("Invalid cache path")?;
        std::fs::create_dir_all(parent)?;
        let mut temp=tempfile::NamedTempFile::new_in(parent)?;
        let mut hash=sha2::Sha256::new(); let mut offset=0;
        while offset<header.length {
            ensure!(!*cancel.borrow(),"Download cancelled");
            let range={
                let db=self.db.lock().unwrap();
                ensure!(super::replica_epoch(&db)?==epoch && tau_blocks::cursor(&db)?.lineage==lineage,"Data source changed");
                tau_blocks::read(&db,&BlockRequest {scope:scope.into(),id:header.id.clone(),version:header.version,offset,follow:false})?
            };
            ensure!(range.header.version==header.version && range.offset==offset && !range.bytes.is_empty(),"Cached file changed or is incomplete");
            temp.write_all(&range.bytes)?; hash.update(&range.bytes); offset+=range.bytes.len() as u64;
        }
        if let Some(expected)=header.meta.get("sha256").and_then(|v|v.as_str()) {
            ensure!(format!("{:x}",hash.finalize())==expected,"Cached file checksum mismatch");
        }
        temp.as_file().sync_all()?;
        static EXPORT_GATE:std::sync::Mutex<()>=std::sync::Mutex::new(());
        let _gate=EXPORT_GATE.lock().unwrap();
        if let Some(root)=&self.exports && target.starts_with(root) {crate::disk::collect_downloads(root,target,header.length,1024*1024*1024)?;}
        // Hold the cache fence through the final rename: clear/reset cannot be
        // followed by late publication from a previously verified generation.
        let db=self.db.lock().unwrap();
        ensure!(!*cancel.borrow() && super::replica_epoch(&db)?==epoch && tau_blocks::cursor(&db)?.lineage==lineage,"Download cancelled or data source changed");
        temp.persist(target)?;
        #[cfg(unix)] std::fs::File::open(parent)?.sync_all()?;
        Ok(())
    }
}
impl Downloads {
    async fn report(&self, key:&str, path:&Path, status:&TransferStatus) {
        let _=self.notices.send(Notice {scope:String::new(),error:None,transfer:Some((key.into(),path.into(),status.clone()))}).await;
        (self.wake)();
    }
    pub async fn run(mut self, key:String, scope:String, id:String, path:PathBuf, limit:u64, mut cancel:watch::Receiver<bool>) {
        let mut status=TransferStatus {transferred:0,total:0,network_bytes:0,done:false,failure:None};
        let cancellation=cancel.clone();
        let result=tokio::select! {
            result=self.fetch(&key,&scope,&id,&path,limit,&cancellation,&mut status) => result,
            _=async {while !*cancel.borrow() {if cancel.changed().await.is_err() {break;}}} => Err(anyhow::anyhow!("Download cancelled")),
        };
        status.done=true; status.failure=result.err().map(|e|e.to_string());
        self.report(&key,&path,&status).await;
    }
    async fn fetch(&mut self, key:&str, scope:&str, id:&str, path:&Path, limit:u64, cancel:&watch::Receiver<bool>, status:&mut TransferStatus) -> Result<()> {
        ensure!(limit<=MAX_BLOCK_BYTES,"Download limit exceeds the block limit");
        let epoch=self.cache.epoch();
        let original_lineage=self.cache.lineage()?;
        let mut request=self.cache.block_request(scope,id)?;
        let cached=self.cache.file_header(scope,id)?;
        let complete=cached.as_ref().is_some_and(|h|h.sealed && h.length==request.offset);
        let (header,lineage)=if complete {
            (cached.unwrap(),self.cache.lineage()?)
        } else {
            while self.ready.borrow().is_none() {self.ready.changed().await.context("Content service stopped")?;}
            let lineage=self.ready.borrow().clone().unwrap();
            ensure!(lineage==original_lineage,"Content source changed; retry the download");
            let client=self.client.borrow().clone().context("Content endpoint is unavailable")?;
            request=self.cache.block_request(scope,id)?; request.follow=false;
            let mut watcher=client.watch_bulk(BlockWatch::Block(request.clone())).await?;
            let mut head=None; let mut reported=Instant::now()-Duration::from_secs(1);
            status.transferred=request.offset;
            loop {
                let (frame,n)=watcher.next().await?; status.network_bytes+=n as u64;
                match &frame.header {
                    Header::Block {block} => {
                        ensure!(block.id==id && matches!(block.kind,BlockKind::File|BlockKind::Image) && block.length<=limit,"Invalid file header or size");
                        if request.version!=block.version {status.transferred=0;request.version=block.version;}
                        status.total=block.length;
                        let (cache,scope,lineage,h)=(self.cache.clone(),scope.to_owned(),lineage.clone(),block.clone());
                        tokio::task::spawn_blocking(move ||cache.header_at(&lineage,&scope,&h,epoch)).await??;
                        head=Some(block.clone());
                    }
                    Header::Data {version,offset,hash,..} => {
                        let h: &BlockHeader=head.as_ref().context("File data preceded its header")?;
                        ensure!(*version==h.version && *offset==status.transferred,"Unexpected file offset/version");
                        let bytes=frame.decoded()?; let length=bytes.len();
                        let range=ContentRange {header:h.clone(),offset:*offset,hash:hash.clone(),bytes};
                        let (cache,scope,lineage)=(self.cache.clone(),scope.to_owned(),lineage.clone());
                        tokio::task::spawn_blocking(move ||cache.range_at(&lineage,&scope,&range,epoch)).await??;
                        status.transferred+=length as u64;
                    }
                    Header::End => break,
                    Header::Error {message} => anyhow::bail!("{message}"),
                    _ => anyhow::bail!("Unexpected file response"),
                }
                let _=watcher.consumed(n).await;
                if reported.elapsed()>=Duration::from_millis(100) {self.report(key,path,status).await;reported=Instant::now();}
            }
            let h=head.context("File response had no header")?;
            ensure!(h.sealed && status.transferred==h.length,"File transfer is incomplete");
            (h,lineage)
        };
        ensure!(header.length<=limit && matches!(header.kind,BlockKind::File|BlockKind::Image),"Invalid file or download limit");
        status.total=header.length;status.transferred=header.length;
        let (cache,scope,target,cancel)=(self.cache.clone(),scope.to_owned(),path.to_owned(),cancel.clone());
        tokio::task::spawn_blocking(move ||cache.export(&lineage,&scope,&header,&target,&cancel)).await??;
        Ok(())
    }
}
