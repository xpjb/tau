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
                    daemon_version: "fixture".into(),
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
    assert!(matches!(event(&mut network).await, Event::Ready(1)));
    let started = Instant::now();
    let sent = match event(&mut network).await {
        Event::HeartbeatSent { epoch: 1, at } => at,
        _ => panic!("expected ping"),
    };
    assert!(started.elapsed() >= HEARTBEAT_INTERVAL - Duration::from_millis(100));
    let rtt = match event(&mut network).await {
        Event::HeartbeatReply { epoch: 1, rtt } => rtt,
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
