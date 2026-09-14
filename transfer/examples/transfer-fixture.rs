use std::fs::File;
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use tau_transfer::{MAX_FILE_BYTES, TransferProvider};
use tokio::net::UdpSocket;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Request {
    node_id: String,
    path: PathBuf,
    slow: bool,
}

#[tokio::main(worker_threads = 2)]
async fn main() -> anyhow::Result<()> {
    let provider = TransferProvider::bind("127.0.0.1:0".parse()?).await?;
    let mut relays = tokio::task::JoinSet::new();
    for line in std::io::stdin().lock().lines() {
        let request: Request = serde_json::from_str(&line?)?;
        let mut offer = provider.offer(File::open(request.path)?, &request.node_id, MAX_FILE_BYTES).await?;
        if request.slow {
            let server = format!("127.0.0.1:{}", offer.port).parse()?;
            let proxy = Arc::new(UdpSocket::bind("127.0.0.1:0").await?);
            offer.port = proxy.local_addr()?.port();
            relays.spawn(async move {
                let mut buffer = [0; 2048];
                let mut client = None;
                let mut sequence = 0;
                let mut packets = tokio::task::JoinSet::new();
                loop {
                    tokio::select! {
                        packet = proxy.recv_from(&mut buffer) => {
                            let Ok((len, sender)) = packet else { break; };
                            let destination = if sender == server { client.unwrap() } else { client = Some(sender); server };
                            sequence += 1;
                            if sequence % 101 == 0 { continue; }
                            let bytes = buffer[..len].to_vec();
                            let socket = proxy.clone();
                            packets.spawn(async move {
                                tokio::time::sleep(Duration::from_millis(50)).await;
                                let _ = socket.send_to(&bytes, destination).await;
                            });
                        }
                        _ = packets.join_next(), if !packets.is_empty() => {}
                    }
                }
            });
        }
        println!("{}", serde_json::to_string(&offer)?);
        std::io::stdout().flush()?;
    }
    relays.abort_all();
    provider.shutdown().await;
    Ok(())
}
