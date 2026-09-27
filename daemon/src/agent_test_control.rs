//! Admission pressure on the real authenticated socket, with no provider calls.
use super::*;
use futures_util::SinkExt;
use tokio_tungstenite::tungstenite::Message;

#[tokio::test]
async fn a_short_control_burst_waits_without_rejecting_actions_or_blocking_pong() {
    let model = ModelServer::start(vec![]).await;
    let (_root, manager, url, server) = fixture(&model, Api::ChatCompletions).await;
    let session = manager.create_session(None, "general").await.unwrap();
    let mut client = Client::connect(&url).await;
    // Hold storage so all requests remain outstanding. This reliably exercises
    // the socket's eight-operation limit, rather than relying on machine speed.
    let state = manager.inner.state.clone();
    let (held, ready) = tokio::sync::oneshot::channel();
    let (release, gate) = std::sync::mpsc::channel();
    let storage = tokio::spawn(async move {
        state.access(move |_| { let _ = held.send(()); gate.recv_timeout(Duration::from_secs(10))?; Ok(()) }).await.unwrap();
    });
    ready.await.unwrap();
    for n in 0..12 {
        client.socket.send(Message::Text(json!({"id":format!("burst-{n}"),"type":"rename_session",
            "sessionId":session,"title":format!("Title {n}")}).to_string().into())).await.unwrap();
    }
    client.socket.send(Message::Ping(b"admission-pressure".to_vec().into())).await.unwrap();
    let mut responses = vec![];
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match client.socket.next().await.unwrap().unwrap() {
                Message::Pong(payload) if payload.as_ref() == b"admission-pressure" => break,
                Message::Text(text) => {
                    let value: Value = serde_json::from_str(&text).unwrap();
                    if value["type"] == "response" && value["requestId"].as_str().is_some_and(|id|id.starts_with("burst-")) { responses.push(value); }
                }
                _ => {}
            }
        }
    }).await.expect("Admission must not block the WebSocket health path");
    release.send(()).unwrap(); storage.await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while responses.len() < 12 {
            if let Message::Text(text) = client.socket.next().await.unwrap().unwrap() {
                let value: Value = serde_json::from_str(&text).unwrap();
                if value["type"] == "response" && value["requestId"].as_str().is_some_and(|id|id.starts_with("burst-")) { responses.push(value); }
            }
        }
    }).await.unwrap();
    assert!(responses.iter().all(|r|r["ok"] == true), "An ordinary burst is not a failed action: {responses:?}");
    let count = manager.inner.state.access(|db| Ok(db.query_row("SELECT count(*) FROM operations WHERE id LIKE 'burst-%' AND response IS NOT NULL", [], |r|r.get::<_,u32>(0))?)).await.unwrap();
    assert_eq!(count, 12, "Each original request completes exactly once; no new retry IDs");
    drop(client); manager.shutdown().await; server.abort();
}

#[tokio::test]
async fn control_waiting_is_bounded_and_disconnect_does_not_start_waiting_actions() {
    let model=ModelServer::start(vec![]).await;
    let (_root,manager,url,server)=fixture(&model,Api::ChatCompletions).await;
    let session=manager.create_session(None,"general").await.unwrap();
    let mut client=Client::connect(&url).await;
    let state=manager.inner.state.clone();let (held,ready)=tokio::sync::oneshot::channel();
    let (release,gate)=std::sync::mpsc::channel();
    let storage=tokio::spawn(async move {
        state.access(move |_| {let _=held.send(());gate.recv_timeout(Duration::from_secs(10))?;Ok(())}).await.unwrap();
    });ready.await.unwrap();
    for n in 0..40 {
        client.socket.send(Message::Text(json!({"id":format!("bounded-{n}"),"type":"rename_session",
            "sessionId":session,"title":format!("Title {n}")}).to_string().into())).await.unwrap();
    }
    client.socket.send(Message::Ping(b"bounded".to_vec().into())).await.unwrap();
    let mut refused=vec![];let mut pong=false;
    tokio::time::timeout(Duration::from_secs(2),async {
        while !pong || refused.len()<8 {match client.socket.next().await.unwrap().unwrap() {
            Message::Pong(payload) if payload.as_ref()==b"bounded"=>pong=true,
            Message::Text(text)=>{
                let value:Value=serde_json::from_str(&text).unwrap();
                if value["type"]=="response" && value["requestId"].as_str().is_some_and(|id|id.starts_with("bounded-")) {refused.push(value);}
            }
            _=>{}
        }}
    }).await.unwrap();
    assert_eq!(refused.len(),8,"Only the 32 bounded waiting/running slots may be admitted");
    assert!(refused.iter().all(|r|r["ok"]==false && r["error"]=="Too many requests are waiting. Try again shortly."));
    client.socket.close(None).await.unwrap();
    let closed=tokio::time::timeout(Duration::from_secs(2),client.socket.next()).await.unwrap();
    assert!(matches!(closed,None|Some(Err(_))|Some(Ok(Message::Close(_)))));
    release.send(()).unwrap();storage.await.unwrap();
    tokio::time::timeout(Duration::from_secs(5),async {
        loop {
            let completed=manager.inner.state.access(|db|Ok(db.query_row("SELECT count(*) FROM operations WHERE id LIKE 'bounded-%' AND response IS NOT NULL",[],|r|r.get::<_,u32>(0))?)).await.unwrap();
            if completed==8 {break;}
            assert!(completed<8,"Disconnected waiting requests must not start later");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.unwrap();
    let registered=manager.inner.state.access(|db|Ok(db.query_row("SELECT count(*) FROM operations WHERE id LIKE 'bounded-%'",[],|r|r.get::<_,u32>(0))?)).await.unwrap();
    assert_eq!(registered,8,"Only already-running operations finish; queued work was never reserved");
    drop(client);manager.shutdown().await;server.abort();
}

#[tokio::test]
async fn control_reads_finish_while_a_content_write_is_still_uncommitted() {
    let model=ModelServer::start(vec![]).await;
    let (_root,manager,url,server)=fixture(&model,Api::ChatCompletions).await;
    let session=manager.create_session(None,"general").await.unwrap();let mut client=Client::connect(&url).await;
    let state=manager.inner.state.clone();let scope=session.clone();
    let (held,ready)=tokio::sync::oneshot::channel();let (release,gate)=std::sync::mpsc::channel();
    let writer=tokio::spawn(async move {state.access(move |db|{
        let tx=db.transaction()?;
        tx.execute("UPDATE sessions SET data=json_set(data,'$.title','not committed') WHERE id=?1",[scope])?;
        let _=held.send(());gate.recv_timeout(Duration::from_secs(5))?;drop(tx);Ok(())
    }).await.unwrap()});ready.await.unwrap();
    for n in 0..12 {
        let command=match n%3 {
            0=>json!({"type":"get_session","sessionId":session}),
            1=>json!({"type":"list_sessions"}),
            _=>json!({"type":"get_operation","operationId":"absent"}),
        };
        let mut request=command;request["id"]=json!(format!("read-{n}"));
        client.socket.send(Message::Text(request.to_string().into())).await.unwrap();
    }
    tokio::time::timeout(Duration::from_secs(1),async {
        let mut replies=0;
        while replies<12 {
            if let Message::Text(text)=client.socket.next().await.unwrap().unwrap() {
                let value:Value=serde_json::from_str(&text).unwrap();
                assert!(!text.contains("not committed"),"Readers must not observe an in-progress write");
                if value["type"]=="response" && value["requestId"].as_str().is_some_and(|id|id.starts_with("read-")) {
                    assert_eq!(value["ok"],true,"{value}");replies+=1;
                }
            }
        }
    }).await.expect("Cheap status/catalogue/receipt reads must not wait for the content writer");
    assert!(!writer.is_finished());release.send(()).unwrap();writer.await.unwrap();
    drop(client);manager.shutdown().await;server.abort();
}

#[tokio::test]
async fn admission_timeout_does_not_execute_or_reserve_the_expired_request() {
    let model=ModelServer::start(vec![]).await;
    let (_root,manager,url,server)=fixture(&model,Api::ChatCompletions).await;
    let session=manager.create_session(None,"general").await.unwrap();let mut client=Client::connect(&url).await;
    let state=manager.inner.state.clone();let (held,ready)=tokio::sync::oneshot::channel();
    let (release,gate)=std::sync::mpsc::channel();
    let storage=tokio::spawn(async move {state.access(move |_|{
        let _=held.send(());gate.recv_timeout(Duration::from_secs(5))?;Ok(())
    }).await.unwrap()});ready.await.unwrap();
    for n in 0..9 {
        client.socket.send(Message::Text(json!({"id":format!("deadline-{n}"),"type":"rename_session",
            "sessionId":session,"title":format!("Title {n}")}).to_string().into())).await.unwrap();
    }
    let started=std::time::Instant::now();
    let response=tokio::time::timeout(Duration::from_secs(4),client.until(|v|v["type"]=="response" && v["requestId"]=="deadline-8")).await.unwrap();
    assert!(started.elapsed()>=Duration::from_secs(2));assert_eq!(response["ok"],false);assert_eq!(response["uncertain"],false);
    assert_eq!(response["error"],"The server could not start this request in time. Please try again.");
    assert!(matches!(manager.inner.state.operation_outcome("deadline-8").await.unwrap(),tau_protocol::ServerMessage::Operation {registered:false,..}));
    release.send(()).unwrap();storage.await.unwrap();
    // The expired request must not start when the database becomes free.
    for n in 0..8 {
        let id=format!("deadline-{n}");
        if !client.seen.iter().any(|v|v["type"]=="response" && v["requestId"]==id) {
            client.until(|v|v["type"]=="response" && v["requestId"]==id).await;
        }
    }
    assert!(matches!(manager.inner.state.operation_outcome("deadline-8").await.unwrap(),tau_protocol::ServerMessage::Operation {registered:false,..}));
    drop(client);manager.shutdown().await;server.abort();
}
