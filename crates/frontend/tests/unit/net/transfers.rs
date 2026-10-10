use super::*;
use crate::replica::tests::Fixture;
use rusqlite::Connection;
use std::sync::{Arc,Mutex};
use serde_json::json;

#[tokio::test(flavor="multi_thread", worker_threads=2)]
async fn file_reconnect_does_not_retry_missing_or_corrupt_content() {
    use std::sync::atomic::{AtomicUsize,Ordering};
    use tau_net::native::{Backend,Client};
    struct Broken {
        db:Arc<Mutex<Connection>>, reads:Arc<AtomicUsize>, changed:watch::Receiver<u64>, fault:u8,
    }
    impl Backend for Broken {
        fn feed(&self,_:FeedRequest)->futures_util::future::BoxFuture<'static,Result<FeedPage>> {
            Box::pin(async {anyhow::bail!("Unexpected metadata request")})
        }
        fn read(&self,request:BlockRequest)->futures_util::future::BoxFuture<'static,Result<Option<ContentRange>>> {
            self.reads.fetch_add(1,Ordering::SeqCst);
            let db=self.db.clone();let fault=self.fault;
            Box::pin(async move {
                if fault==0 {return Ok(None);}
                ensure!(fault!=1,"Source content is missing");
                let mut range=tau_block_store::read(&db.lock().unwrap(),&request)?.unwrap();
                range.hash="0".repeat(64);Ok(Some(range))
            })
        }
        fn changes(&self)->watch::Receiver<u64> {self.changed.clone()}
    }
    for (fault,message) in [(0,"no longer exists"),(1,"missing"),(2,"integrity")] {
        let mut f=Fixture::new();f.put("file",None,0,BlockKind::File,json!({}),b"file bytes");
        let reads=Arc::new(AtomicUsize::new(0));let (_changes,changed)=watch::channel(0);
        let server=tau_net::native::Server::bind("127.0.0.1:0".parse().unwrap(),Arc::new(Broken {
            db:Arc::new(Mutex::new(f.source)),reads:reads.clone(),changed,fault,
        })).await.unwrap();
        let client=Client::bind().await.unwrap();
        let peer=client.configure(&server.authorize(&client.node_id(),f.lineage.clone()).unwrap(),"127.0.0.1").await.unwrap();
        let (_ready,ready)=watch::channel(Some(peer));
        let (events,mut received)=super::super::mailbox::channel(Arc::new(||{}));let (_cancel,cancel)=watch::channel(false);
        let path=f._root.path().join("download.bin");
        let transfer=Transfers {cache:f.cache.clone(),ready,events};
        tokio::time::timeout(Duration::from_secs(5),transfer.run("file".into(),"chat".into(),"file".into(),path.clone(),MAX_BLOCK_BYTES,cancel)).await.unwrap();
        let failure=loop {
            let Event::Download {status,..}=received.recv().await.unwrap() else {panic!("Expected transfer progress");};
            if status.done {break status.failure.unwrap();}
        };
        assert!(failure.contains(message),"{failure}");
        assert_eq!(reads.load(Ordering::SeqCst),1,"Content failure is not a connection retry");
        assert!(!path.exists());assert_eq!(f.cache.block_request("chat","file").unwrap().offset,0);
        client.shutdown().await;server.shutdown().await;
    }
}

#[tokio::test(flavor="multi_thread",worker_threads=2)]
async fn download_rebinds_after_node_restart_but_never_crosses_a_source_lineage() {
    use std::sync::atomic::{AtomicBool,Ordering};
    use tau_net::native::{Backend,Client,Server};
    struct Source {db:Arc<Mutex<Connection>>,stall:Arc<AtomicBool>,changes:watch::Receiver<u64>}
    impl Backend for Source {
        fn feed(&self,_:FeedRequest)->futures_util::future::BoxFuture<'static,Result<FeedPage>> {Box::pin(async {anyhow::bail!("Unexpected feed")})}
        fn read(&self,request:BlockRequest)->futures_util::future::BoxFuture<'static,Result<Option<ContentRange>>> {
            let db=self.db.clone();let stall=self.stall.clone();
            Box::pin(async move {
                if request.offset>=MAX_BLOCK_RANGE_BYTES as u64 && stall.load(Ordering::Acquire) {std::future::pending::<()>().await;}
                tau_block_store::read(&db.lock().unwrap(),&request)
            })
        }
        fn changes(&self)->watch::Receiver<u64> {self.changes.clone()}
    }
    for changed_lineage in [false,true] {
        let mut f=Fixture::new();let bytes=vec![b'x';MAX_BLOCK_RANGE_BYTES*2];
        f.put("file",None,0,BlockKind::File,json!({}),&bytes);
        let stall=Arc::new(AtomicBool::new(true));let (_changes,changes)=watch::channel(0);
        let source=Arc::new(Source {db:Arc::new(Mutex::new(f.source)),stall:stall.clone(),changes});
        let server=Server::bind("127.0.0.1:0".parse().unwrap(),source.clone()).await.unwrap();
        let client=Client::bind().await.unwrap();
        let peer=client.configure(&server.authorize(&client.node_id(),f.lineage.clone()).unwrap(),"127.0.0.1").await.unwrap();
        let (ready,receiver)=watch::channel(Some(peer));
        let (events,mut received)=super::super::mailbox::channel(Arc::new(||{}));
        let (_cancel,cancel)=watch::channel(false);let path=f._root.path().join("restarted.bin");
        let transfer=Transfers {cache:f.cache.clone(),ready:receiver,events};
        let job=tokio::spawn(transfer.run("file".into(),"chat".into(),"file".into(),path.clone(),MAX_BLOCK_BYTES,cancel));
        tokio::time::timeout(Duration::from_secs(4),async {
            while f.cache.block_request("chat","file").unwrap().offset==0 {tokio::task::yield_now().await;}
        }).await.unwrap();
        server.shutdown().await;stall.store(false,Ordering::Release);
        let server=Server::bind("127.0.0.1:0".parse().unwrap(),source).await.unwrap();
        let lineage=if changed_lineage {"different-source".to_owned()} else {f.lineage.clone()};
        let peer=client.configure(&server.authorize(&client.node_id(),lineage.clone()).unwrap(),"127.0.0.1").await.unwrap();
        f.cache.configure(&lineage).unwrap();ready.send_replace(Some(peer));
        tokio::time::timeout(Duration::from_secs(5),job).await.unwrap().unwrap();
        let status=loop {if let Event::Download {status,..}=received.recv().await.unwrap() && status.done {break status;}};
        if changed_lineage {
            assert!(status.failure.as_ref().unwrap().contains("source"));assert!(!path.exists());
        } else {
            assert!(status.failure.is_none(),"{:?}",status.failure);
            assert_eq!(std::fs::read(&path).unwrap(),bytes);
            assert_eq!(client.stats().resumed_bytes,MAX_BLOCK_RANGE_BYTES as u64,"The new node starts from the verified prefix, not byte zero");
        }
        client.shutdown().await;server.shutdown().await;
    }
}
