//! Native upload restart, immutable input identity and shared-connection tests.
use std::sync::{Arc,Mutex};
use futures_util::{FutureExt,future::BoxFuture};
use rusqlite::Connection;
use tau_blocks::*;
use tau_transfer::blocks::{Backend,Server,Client,Header};
use tokio::sync::watch;

struct Store {db:Arc<Mutex<Connection>>,changes:watch::Sender<u64>}
impl Store {
    fn open(path:&std::path::Path)->Arc<Self> {
        let db=Connection::open(path).unwrap();db.execute_batch("PRAGMA foreign_keys=ON;PRAGMA journal_mode=WAL;PRAGMA synchronous=FULL").unwrap();tau_blocks::initialize(&db).unwrap();
        Arc::new(Self {db:Arc::new(Mutex::new(db)),changes:watch::channel(0).0})
    }
    fn lineage(&self)->String {cursor(&self.db.lock().unwrap()).unwrap().lineage}
}
impl Backend for Store {
    fn feed(&self,r:FeedRequest)->BoxFuture<'static,anyhow::Result<FeedPage>> {let db=self.db.clone();async move {feed(&db.lock().unwrap(),&r)}.boxed()}
    fn read(&self,r:BlockRequest)->BoxFuture<'static,anyhow::Result<ContentRange>> {let db=self.db.clone();async move {read(&db.lock().unwrap(),&r)}.boxed()}
    fn changes(&self)->watch::Receiver<u64> {self.changes.subscribe()}
    fn upload_begin(&self,s:UploadSpec)->BoxFuture<'static,anyhow::Result<UploadStatus>> {let db=self.db.clone();async move {let mut db=db.lock().unwrap();let tx=db.transaction()?;let status=uploads::begin(&tx,&s)?;tx.commit()?;Ok(status)}.boxed()}
    fn upload_write(&self,s:UploadSpec,offset:u64,bytes:Vec<u8>)->BoxFuture<'static,anyhow::Result<()>> {let db=self.db.clone();async move {let mut db=db.lock().unwrap();let tx=db.transaction()?;uploads::write(&tx,&s,offset,&bytes)?;tx.commit()?;Ok(())}.boxed()}
    fn upload_finish(&self,s:UploadSpec)->BoxFuture<'static,anyhow::Result<UploadStatus>> {let db=self.db.clone();async move {let mut db=db.lock().unwrap();let tx=db.transaction()?;let bytes=cached_content(&tx,UPLOAD_SCOPE,&s.id)?;let status=uploads::seal(&tx,&s,&blake3::hash(&bytes).to_hex(),None)?;tx.commit()?;Ok(status)}.boxed()}
}
async fn connect(store:Arc<Store>)->(Server,Client) {
    let server=Server::bind("127.0.0.1:0".parse().unwrap(),store.clone()).await.unwrap();
    let client=Client::bind().await.unwrap();
    let offer=server.authorize(&client.node_id(),store.lineage()).unwrap();client.configure(&offer,"127.0.0.1").await.unwrap();(server,client)
}

#[tokio::test]
async fn interrupted_upload_resumes_durable_bytes_after_both_endpoints_restart() {
    let root=tempfile::tempdir().unwrap();let path=root.path().join("source.db");let store=Store::open(&path);
    let bytes=(0..900_000).map(|n|((n*31)%251) as u8).collect::<Vec<_>>();
    let spec=UploadSpec {id:"operation".into(),length:bytes.len() as u64,hash:blake3::hash(&bytes).to_hex().to_string(),purpose:UploadPurpose::Command};
    let (server,client)=connect(store.clone()).await;
    let mut upload=client.uploader(spec.clone()).await.unwrap();
    for chunk in bytes[..128*1024].chunks(BLOCK_CHUNK_BYTES) {upload.write(chunk).await.unwrap();}
    // A second stream proves progress is committed, not just client queued.
    tokio::time::timeout(std::time::Duration::from_secs(5),async {
        loop {let status=uploads::status(&store.db.lock().unwrap(),&spec).unwrap();if status.offset>=64*1024 {break;}tokio::task::yield_now().await;}
    }).await.unwrap();
    drop(upload);client.shutdown().await;server.shutdown().await;drop(client);drop(server);drop(store);
    let store=Store::open(&path);let saved=uploads::status(&store.db.lock().unwrap(),&spec).unwrap().offset;
    assert!(saved>=64*1024 && saved<spec.length);
    let (server,client)=connect(store.clone()).await;
    let mut upload=client.uploader(spec.clone()).await.unwrap();assert_eq!(upload.status.offset,saved);
    for chunk in bytes[saved as usize..].chunks(BLOCK_CHUNK_BYTES) {upload.write(chunk).await.unwrap();}
    let status=upload.finish().await.unwrap();assert!(status.sealed);drop(upload);
    let retry=client.uploader(spec.clone()).await.unwrap();assert!(retry.status.sealed);assert_eq!(retry.status.offset,spec.length);drop(retry);
    assert_eq!(cached_content(&store.db.lock().unwrap(),UPLOAD_SCOPE,&spec.id).unwrap(),bytes);
    let mut wrong=spec.clone();wrong.hash=blake3::hash(b"other").to_hex().to_string();assert!(client.uploader(wrong).await.is_err());
    let mut feed=client.watch(BlockWatch::Feed(FeedRequest {scope:UPLOAD_SCOPE.into(),parent:None,cursor:None,floor:0,before:None})).await.unwrap();
    assert!(matches!(feed.next().await.unwrap().0.header,Header::Record {..}));
    client.shutdown().await;server.shutdown().await;
}

#[tokio::test]
async fn upload_checks_authorization_hashes_size_gaps_and_unsealed_references() {
    let root=tempfile::tempdir().unwrap();let store=Store::open(&root.path().join("source.db"));
    let (server,client)=connect(store.clone()).await;
    let spec=UploadSpec {id:"bound".into(),length:4,hash:blake3::hash(b"good").to_hex().to_string(),purpose:UploadPurpose::Command};
    let mut upload=client.uploader(spec.clone()).await.unwrap();upload.write(b"evil").await.unwrap();assert!(upload.finish().await.is_err());drop(upload);
    assert!(!uploads::status(&store.db.lock().unwrap(),&spec).unwrap().sealed);
    let reference=ContentRef {lineage:store.lineage(),scope:UPLOAD_SCOPE.into(),id:spec.id.clone(),length:4,hash:spec.hash.clone()};
    assert!(uploads::input(&store.db.lock().unwrap(),&reference).is_err());
    let mut too_big=spec.clone();too_big.length=MAX_COMMAND_BYTES+1;assert!(client.uploader(too_big).await.is_err());
    let outsider=Client::bind().await.unwrap();
    let offer=server.authorize(&client.node_id(),store.lineage()).unwrap();outsider.configure(&offer,"127.0.0.1").await.unwrap();
    assert!(outsider.uploader(spec.clone()).await.is_err());
    server.revoke(&client.node_id());assert!(client.uploader(spec).await.is_err());
    outsider.shutdown().await;client.shutdown().await;server.shutdown().await;
}
