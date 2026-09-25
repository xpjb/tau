#![cfg(unix)]
//! A scripted WebSocket, no daemon, provider, GPU or account needed.
use axum::{
    Router,
    extract::ws::{Message, WebSocketUpgrade},
    routing::get,
};
use futures_util::StreamExt;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tau_frontend::{
    connection::{HEARTBEAT_INTERVAL, HEARTBEAT_TIMEOUT},
    store::Settings,
    transport::{Event, Network},
};
use tau_protocol::{PROTOCOL_VERSION, ServerMessage};

async fn fixture(reply: bool) -> (Settings, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new().route(
        "/v1/ws",
        get(move |ws: WebSocketUpgrade| async move {
            ws.on_upgrade(move |mut socket| async move {
                let hello = ServerMessage::Hello {
                    protocol_version: PROTOCOL_VERSION,
                    daemon_version: "fixture".into(),lineage:Some("fixture".into()),
                };
                socket
                    .send(Message::Text(serde_json::to_string(&hello).unwrap().into()))
                    .await
                    .unwrap();
                if reply {
                    while let Some(Ok(frame)) = socket.next().await {
                        match frame {
                            Message::Ping(payload) => {
                                socket.send(Message::Pong(payload)).await.unwrap();
                            }
                            Message::Text(_) => panic!("heartbeat must not request a session list"),
                            _ => {}
                        }
                    }
                } else {
                    // No reads: the WebSocket implementation cannot auto-pong.
                    tokio::time::sleep(Duration::from_secs(10)).await;
                }
            })
        }),
    );
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (
        Settings {
            server_url: format!("http://{address}"),
            token: "fixture-token".into(),
        },
        server,
    )
}

async fn event(network: &mut Network) -> Event {
    tokio::time::timeout(Duration::from_secs(9), network.events.recv())
        .await
        .unwrap()
        .expect("mock network shut down")
}

#[tokio::test]
async fn probe_uses_ping_pong_not_session_list_and_reports_measured_rtt() {
    let (settings, server) = fixture(true).await;
    let mut network = Network::start(settings, Arc::new(|| {}));
    assert!(matches!(event(&mut network).await,Event::Source(1,_)));
    assert!(matches!(event(&mut network).await, Event::Ready(1)));
    let started = Instant::now();
    let sent = match event(&mut network).await {
        Event::HeartbeatSent { epoch: 1, at } => at,
        _ => panic!("expected ping"),
    };
    assert!(started.elapsed() >= HEARTBEAT_INTERVAL - Duration::from_millis(100));
    let rtt = match event(&mut network).await {
        Event::HeartbeatReply { epoch: 1, at, rtt } => {
            assert!(
                at >= sent && at <= Instant::now(),
                "pong timestamp must be taken at receipt"
            );
            rtt
        }
        _ => panic!("expected matching pong"),
    };
    assert!(rtt <= sent.elapsed() && rtt < HEARTBEAT_TIMEOUT);
    drop(network);
    server.abort();
}

#[tokio::test]
async fn unanswered_ping_reconnects_on_deadline_not_on_next_probe() {
    let (settings, server) = fixture(false).await;
    let mut network = Network::start(settings, Arc::new(|| {}));
    assert!(matches!(event(&mut network).await,Event::Source(1,_)));
    assert!(matches!(event(&mut network).await, Event::Ready(1)));
    let sent = match event(&mut network).await {
        Event::HeartbeatSent { epoch: 1, at } => at,
        _ => panic!("expected ping"),
    };
    match event(&mut network).await {
        Event::Disconnected(reason) => assert!(reason.contains("Ping timed out"), "{reason}"),
        _ => panic!("expected heartbeat timeout"),
    }
    let elapsed = sent.elapsed();
    assert!(
        elapsed >= HEARTBEAT_TIMEOUT && elapsed < HEARTBEAT_TIMEOUT + Duration::from_secs(2),
        "{elapsed:?}"
    );
    drop(network);
    server.abort();
}

// Even a legacy/malicious peer cannot force a large allocation or occupy the
// control reader until heartbeat timeout: the WS length header is rejected.
#[tokio::test]
async fn oversized_legacy_frame_is_rejected_before_reading_its_body() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let pings = Arc::new(AtomicUsize::new(0));
    let counted = pings.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream = listener.local_addr().unwrap();
    let app = Router::new().route("/v1/ws", get(move |ws: WebSocketUpgrade| {
        let counted = counted.clone();
        async move { ws.on_upgrade(move |mut socket| async move {
            let hello = ServerMessage::Hello { protocol_version:PROTOCOL_VERSION, daemon_version:"audit".into(),lineage:Some("fixture".into()) };
            socket.send(Message::Text(serde_json::to_string(&hello).unwrap().into())).await.unwrap();
            let legacy=serde_json::json!({"type":"transcript_snapshot","body":"x".repeat(512*1024)});
            if socket.send(Message::Text(legacy.to_string().into())).await.is_err() {return;}
            while let Some(Ok(frame)) = socket.next().await {
                if let Message::Ping(payload) = frame {
                    if socket.send(Message::Pong(payload)).await.is_err() { break; }
                    counted.fetch_add(1, Ordering::SeqCst);
                }
            }
        }) }
    }));
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let proxy = tokio::spawn(async move {
        let mut jobs = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                accepted = listener.accept() => {
                    let (client, _) = accepted.unwrap();
                    jobs.spawn(async move {
                        let peer = tokio::net::TcpStream::connect(upstream).await?;
                        let (mut client_read, mut client_write) = client.into_split();
                        let (mut peer_read, mut peer_write) = peer.into_split();
                        let upload = tokio::io::copy(&mut client_read, &mut peer_write);
                        let download = async {
                            let mut chunk = [0u8; 512];
                            loop {
                                let n = peer_read.read(&mut chunk).await?;
                                if n == 0 { return Ok::<(), std::io::Error>(()); }
                                client_write.write_all(&chunk[..n]).await?;
                                tokio::time::sleep(Duration::from_millis(20)).await;
                            }
                        };
                        tokio::try_join!(upload, download).map(|_| ())
                    });
                }
                _ = jobs.join_next(), if !jobs.is_empty() => {}
            }
        }
    });
    let settings = Settings { server_url:format!("http://{address}"), token:"audit-fixture".into() };
    let mut network = Network::start(settings, Arc::new(|| {}));
    assert!(matches!(event(&mut network).await,Event::Source(1,_)));
    assert!(matches!(event(&mut network).await, Event::Ready(1)));
    let started=Instant::now();
    match event(&mut network).await {
        Event::Disconnected(reason)=>assert!(!reason.contains("Ping timed out"),"{reason}"),
        _=>panic!("Expected immediate oversized-frame rejection"),
    }
    assert!(started.elapsed()<HEARTBEAT_TIMEOUT);
    assert_eq!(pings.load(Ordering::SeqCst),0);
    drop(network);
    proxy.abort();
    server.abort();
}

#[tokio::test]
async fn paused_ui_coalesces_state_without_blocking_heartbeats_or_losing_receipts() {
    use std::sync::atomic::{AtomicUsize,Ordering};
    let pings=Arc::new(AtomicUsize::new(0));let seen=pings.clone();
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let address=listener.local_addr().unwrap();
    let app=Router::new().route("/v1/ws",get(move |ws:WebSocketUpgrade| {let seen=seen.clone();async move {ws.on_upgrade(move |mut socket|async move {
        socket.send(Message::Text(serde_json::to_string(&ServerMessage::Hello {protocol_version:PROTOCOL_VERSION,daemon_version:"fixture".into(),lineage:Some("fixture".into())}).unwrap().into())).await.unwrap();
        for n in 0..1500 {
            let state=ServerMessage::SessionState {revision:0,restore_review:None,session_id:"chat".into(),status:tau_protocol::SessionStatus::Running,context_usage:None,detail:Some(n.to_string())};
            socket.send(Message::Text(serde_json::to_string(&state).unwrap().into())).await.unwrap();
            if n<200 {socket.send(Message::Text(serde_json::to_string(&ServerMessage::success(format!("receipt-{n}"),None,None)).unwrap().into())).await.unwrap();}
        }
        while let Some(Ok(frame))=socket.next().await {if let Message::Ping(payload)=frame {seen.fetch_add(1,Ordering::SeqCst);if socket.send(Message::Pong(payload)).await.is_err() {break;}}}
    })}}));
    let server=tokio::spawn(async move {axum::serve(listener,app).await.unwrap()});
    let mut network=Network::start(Settings {server_url:format!("http://{address}"),token:"fixture".into()},Arc::new(||{}));
    // Deliberately no UI reads for longer than the old heartbeat deadline.
    tokio::time::sleep(HEARTBEAT_INTERVAL+HEARTBEAT_TIMEOUT+Duration::from_millis(200)).await;
    assert!(pings.load(Ordering::SeqCst)>=3,"UI backpressure blocked control probes");
    let mut receipts=std::collections::HashSet::new();let mut detail=None;
    while let Ok(event)=network.events.try_recv() {match event {
        Event::Message(_,message)=>match *message {ServerMessage::Response {request_id,..}=>{receipts.insert(request_id);},ServerMessage::SessionState {detail:d,..}=>detail=d,_=>{}},
        Event::Disconnected(reason)=>panic!("{reason}"),_=>{},
    }}
    assert_eq!(receipts.len(),200);assert_eq!(detail.as_deref(),Some("1499"));drop(network);server.abort();
}
