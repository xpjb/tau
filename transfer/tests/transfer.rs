use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::sync::Arc;
use std::time::Duration;

use tau_transfer::{MAX_FILE_BYTES, TransferDownload, TransferProvider, TransferStatus};
use tokio::net::UdpSocket;
use tokio::time::{sleep, timeout};

async fn finished(download: &TransferDownload) -> TransferStatus {
    timeout(Duration::from_secs(45), async {
        loop {
            let status = download.status();
            if status.done { return status; }
            sleep(Duration::from_millis(10)).await;
        }
    }).await.expect("native transfer did not stop")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resumes_verified_blocks_after_restart_and_preserves_completed_files() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let target = root.path().join("destination");
    let bytes = (0..8_000_000).map(|i| ((i * 31) % 251) as u8).collect::<Vec<_>>();
    std::fs::write(&source, &bytes).unwrap();
    std::fs::write(&target, b"previous complete file").unwrap();
    let provider = TransferProvider::bind("127.0.0.1:0".parse().unwrap()).await.unwrap();
    for corrupt in [false, true] {
        std::fs::write(&target, b"previous complete file").unwrap();
        let first = TransferDownload::new();
        let mut offer = provider.offer(File::open(&source).unwrap(), &first.node_id(), MAX_FILE_BYTES).await.unwrap();
        let server = format!("127.0.0.1:{}", offer.port).parse().unwrap();
        let proxy = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        offer.port = proxy.local_addr().unwrap().port();
        let relay = tokio::spawn(async move {
            let mut buffer = [0; 2048];
            let mut client = None;
            let mut sequence = 0;
            let mut packets = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    packet = proxy.recv_from(&mut buffer) => {
                        let (len, sender) = packet.unwrap();
                        let destination = if sender == server { client.unwrap() } else { client = Some(sender); server };
                        sequence += 1;
                        if sequence % 101 == 0 { continue; }
                        let bytes = buffer[..len].to_vec();
                        let socket = proxy.clone();
                        packets.spawn(async move {
                            sleep(Duration::from_millis(20)).await;
                            let _ = socket.send_to(&bytes, destination).await;
                        });
                    }
                    _ = packets.join_next(), if !packets.is_empty() => {}
                }
            }
        });
        first.start(serde_json::to_string(&offer).unwrap(), "127.0.0.1".into(), target.to_string_lossy().into_owned(), MAX_FILE_BYTES).unwrap();
        timeout(Duration::from_secs(30), async {
            loop {
                let status = first.status();
                assert!(!status.done, "transfer finished before interruption: {status:?}");
                if status.transferred >= 1_000_000 { break; }
                sleep(Duration::from_millis(10)).await;
            }
        }).await.unwrap();
        first.cancel();
        let stopped = finished(&first).await;
        assert!(stopped.failure.as_deref().unwrap().contains("cancelled"), "{stopped:?}");
        first.join();
        drop(first);
        relay.abort();
        assert_eq!(std::fs::read(&target).unwrap(), b"previous complete file");
        assert!(root.path().join(".destination.part").is_dir());

        if corrupt {
            let data = std::fs::read_dir(root.path().join(".destination.part/data")).unwrap()
                .map(|entry| entry.unwrap().path()).find(|path| path.extension().is_some_and(|ext| ext == "data")).unwrap();
            let mut damaged = std::fs::OpenOptions::new().write(true).open(data).unwrap();
            damaged.write_all(&[255]).unwrap();
            damaged.sync_all().unwrap();
        }
        let resumed = TransferDownload::new();
        let offer = provider.offer(File::open(&source).unwrap(), &resumed.node_id(), MAX_FILE_BYTES).await.unwrap();
        resumed.start(serde_json::to_string(&offer).unwrap(), "127.0.0.1".into(), target.to_string_lossy().into_owned(), MAX_FILE_BYTES).unwrap();
        let result = finished(&resumed).await;
        resumed.join();
        assert!(result.failure.is_none(), "{result:?}");
        if corrupt {
            assert!(result.network_bytes > 8_000_000, "corrupt retained data was not repaired: {result:?}");
        } else {
            assert!(result.network_bytes < 7_500_000, "resume retransmitted completed blocks: {result:?}");
        }
        assert_eq!(result.transferred, bytes.len() as u64);
        assert_eq!(std::fs::read(&target).unwrap(), bytes);
        assert!(!root.path().join(".destination.part").exists());
        println!("resumed transfer (corrupt={corrupt}): {result:?}");
    }

    let changed = TransferDownload::new();
    let offer = provider.offer(File::open(&source).unwrap(), &changed.node_id(), MAX_FILE_BYTES).await.unwrap();
    let mut file = std::fs::OpenOptions::new().write(true).open(&source).unwrap();
    file.seek(SeekFrom::Start(0)).unwrap();
    file.write_all(b"changed after grant").unwrap();
    file.sync_all().unwrap();
    changed.start(serde_json::to_string(&offer).unwrap(), "127.0.0.1".into(), target.to_string_lossy().into_owned(), MAX_FILE_BYTES).unwrap();
    let failed = finished(&changed).await;
    changed.join();
    assert!(failed.failure.is_some(), "changed source was accepted");
    assert_eq!(std::fs::read(&target).unwrap(), bytes);
    provider.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn restricts_grants_to_the_client_and_file_and_handles_empty_files() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let target = root.path().join("destination");
    std::fs::write(&source, b"private source").unwrap();
    let provider = TransferProvider::bind("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let owner = TransferDownload::new();
    let offer = provider.offer(File::open(&source).unwrap(), &owner.node_id(), MAX_FILE_BYTES).await.unwrap();
    let intruder = TransferDownload::new();
    intruder.start(serde_json::to_string(&offer).unwrap(), "127.0.0.1".into(), target.to_string_lossy().into_owned(), MAX_FILE_BYTES).unwrap();
    assert!(finished(&intruder).await.failure.is_some());
    intruder.join();
    assert!(!target.exists());

    let mut wrong = offer.clone();
    wrong.hash = blake3::hash(b"another file").to_hex().to_string();
    owner.start(serde_json::to_string(&wrong).unwrap(), "127.0.0.1".into(), target.to_string_lossy().into_owned(), MAX_FILE_BYTES).unwrap();
    assert!(finished(&owner).await.failure.is_some());
    owner.join();
    assert!(!target.exists());
    let bounded = TransferDownload::new();
    assert!(bounded.start(serde_json::to_string(&offer).unwrap(), "127.0.0.1".into(), target.to_string_lossy().into_owned(), 1).is_err());

    std::fs::write(&source, []).unwrap();
    let empty = TransferDownload::new();
    let offer = provider.offer(File::open(&source).unwrap(), &empty.node_id(), MAX_FILE_BYTES).await.unwrap();
    empty.start(serde_json::to_string(&offer).unwrap(), "127.0.0.1".into(), target.to_string_lossy().into_owned(), MAX_FILE_BYTES).unwrap();
    let result = finished(&empty).await;
    empty.join();
    assert!(result.failure.is_none(), "{result:?}");
    assert_eq!(std::fs::metadata(target).unwrap().len(), 0);
    provider.shutdown().await;
}
