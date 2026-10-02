use super::*;
use crate::replica::tests::Fixture;
use rusqlite::Connection;
use std::sync::Mutex;
use serde_json::json;

#[tokio::test(flavor="multi_thread", worker_threads=2)]
async fn file_reconnect_does_not_retry_missing_or_corrupt_content() {
    use std::sync::atomic::{AtomicUsize,Ordering};
    use tau_net::native::Backend;
    struct Broken {
        db:Arc<Mutex<Connection>>, reads:Arc<AtomicUsize>, changed:watch::Receiver<u64>, corrupt:bool,
    }
    impl Backend for Broken {
        fn feed(&self,_:FeedRequest)->futures_util::future::BoxFuture<'static,Result<FeedPage>> {
            Box::pin(async {anyhow::bail!("Unexpected metadata request")})
        }
        fn read(&self,request:BlockRequest)->futures_util::future::BoxFuture<'static,Result<ContentRange>> {
            self.reads.fetch_add(1,Ordering::SeqCst);
            let db=self.db.clone();let corrupt=self.corrupt;
            Box::pin(async move {
                ensure!(corrupt,"Source content is missing");
                let mut range=tau_block_store::read(&db.lock().unwrap(),&request)?;
                range.hash="0".repeat(64);Ok(range)
            })
        }
        fn changes(&self)->watch::Receiver<u64> {self.changed.clone()}
    }
    for corrupt in [false,true] {
        let mut f=Fixture::new();f.put("file",None,0,BlockKind::File,json!({}),b"file bytes");
        let reads=Arc::new(AtomicUsize::new(0));let (_changes,changed)=watch::channel(0);
        let server=tau_net::native::Server::bind("127.0.0.1:0".parse().unwrap(),Arc::new(Broken {
            db:Arc::new(Mutex::new(f.source)),reads:reads.clone(),changed,corrupt,
        })).await.unwrap();
        let client=Arc::new(Client::bind().await.unwrap());
        client.configure(&server.authorize(&client.node_id(),f.lineage.clone()).unwrap(),"127.0.0.1").await.unwrap();
        let (_endpoint,endpoint)=watch::channel(Some(client.clone()));let (_ready,ready)=watch::channel(Some(f.lineage));
        let (events,mut received)=super::super::mailbox::channel(Arc::new(||{}));let (_cancel,cancel)=watch::channel(false);
        let path=f._root.path().join("download.bin");
        let transfer=Transfers {cache:f.cache.clone(),client:endpoint,ready,events};
        tokio::time::timeout(Duration::from_secs(5),transfer.run("file".into(),"chat".into(),"file".into(),path.clone(),MAX_BLOCK_BYTES,cancel)).await.unwrap();
        let failure=loop {
            let Event::Download {status,..}=received.recv().await.unwrap() else {panic!("Expected transfer progress");};
            if status.done {break status.failure.unwrap();}
        };
        assert!(failure.contains(if corrupt {"integrity"} else {"missing"}),"{failure}");
        assert_eq!(reads.load(Ordering::SeqCst),1,"Content failure is not a connection retry");
        assert!(!path.exists());assert_eq!(f.cache.block_request("chat","file").unwrap().offset,0);
        client.shutdown().await;server.shutdown().await;
    }
}
