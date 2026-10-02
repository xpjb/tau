//! Socket admission and queue-control recovery, using isolated scripted providers.
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
    assert!(matches!(manager.inner.state.operation_outcome("deadline-8").await.unwrap(),tau_net::ServerMessage::Operation {registered:false,..}));
    release.send(()).unwrap();storage.await.unwrap();
    // The expired request must not start when the database becomes free.
    for n in 0..8 {
        let id=format!("deadline-{n}");
        if !client.seen.iter().any(|v|v["type"]=="response" && v["requestId"]==id) {
            client.until(|v|v["type"]=="response" && v["requestId"]==id).await;
        }
    }
    assert!(matches!(manager.inner.state.operation_outcome("deadline-8").await.unwrap(),tau_net::ServerMessage::Operation {registered:false,..}));
    drop(client);manager.shutdown().await;server.abort();
}

#[tokio::test]
async fn resume_while_abort_is_settling_is_not_overwritten_by_old_cleanup() {
    for stop_again in [false, true] {
        let mut blocked = completion("Cancelled response", vec![]);
        blocked.gate = Some(Arc::new(Notify::new()));
        let mut model = ModelServer::start(vec![blocked, completion("Resumed once", vec![])]).await;
        let (_root, manager, _, server) = fixture(&model, Api::ChatCompletions).await;
        let id = manager.create_session(None, "general").await.unwrap();
        let gate = Arc::new(Notify::new());
        *manager.inner.settle_gate.lock().unwrap() = Some(gate.clone());
        manager.prompt(&id, "Work", "prompt").await.unwrap();
        model.request().await;
        manager.abort(&id, "stop").await.unwrap();
        let runtime = manager.runtime(&id).await.unwrap();
        let (generation, run_id) = {
            let content = runtime.content.lock().await;
            assert!(content.agent.as_ref().unwrap().running, "Cleanup is gated");
            let transcript = content.transcript.as_ref().unwrap();
            (transcript.generation.clone(), transcript.queue.run_id.clone())
        };
        let resume = tau_net::QueueOperation::Resume { run_id };
        manager.queue_control(&id, &generation, "resume", resume.clone()).await.unwrap();
        manager.queue_control(&id, &generation, "resume", resume.clone()).await.unwrap();
        assert!(!manager.inner.state.queue(&id).await.unwrap().paused);
        if stop_again { manager.abort(&id, "stop-again").await.unwrap(); }
        gate.notify_one();
        if !stop_again { model.request().await; }
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if !runtime.content.lock().await.agent.as_ref().unwrap().running { break; }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }).await.unwrap();
        assert_eq!(manager.inner.state.queue(&id).await.unwrap().paused, stop_again,
            "Only a later explicit stop may override the accepted resume");
        // Lost response retries keep the original immutable ID and cannot restart
        // work after a subsequent stop, or after the resumed turn has completed.
        manager.queue_control(&id, &generation, "resume", resume).await.unwrap();
        assert!(model.requests.try_recv().is_err());
        manager.shutdown().await; server.abort();
    }
}


#[tokio::test]
async fn stop_retires_waiting_controls_and_play_drains_the_queue() {
    for action in ["prefix", "pause"] {
        let mut blocked = completion("Stopped response", vec![]);
        blocked.gate = Some(Arc::new(Notify::new()));
        let mut model = ModelServer::start(vec![blocked, completion("Queue drained", vec![])]).await;
        let (_root, manager, url, server) = fixture(&model, Api::ChatCompletions).await;
        let mut client = Client::connect(&url).await;
        let id = manager.create_session(None, "general").await.unwrap();
        let cleanup = Arc::new(Notify::new());
        *manager.inner.settle_gate.lock().unwrap() = Some(cleanup.clone());
        manager.prompt(&id, "Work", "work").await.unwrap();
        model.request().await;
        for text in ["one", "two"] { manager.prompt(&id, text, text).await.unwrap(); }
        let snapshot = client.open(&id).await;
        let generation = snapshot["generation"].clone();
        let run = snapshot["queue"]["runId"].clone();
        let mut operation = json!({"type":action,"runId":run,"boundary":"turn"});
        if action == "prefix" { operation["requests"] = json!([{"requestId":"one","revision":0}]); }
        let control = json!({"id":"limit","type":"queue_control","sessionId":id,"generation":generation,"operation":operation});
        assert_eq!(client.request(control.clone()).await["ok"], true);
        assert_eq!(client.request(json!({"id":"stop","type":"abort","sessionId":id})).await["ok"], true);
        let stopped = manager.inner.state.queue(&id).await.unwrap();
        assert!(stopped.paused);
        assert!(stopped.control.is_none(), "Stop must retire the waiting {action} in the same durable commit");
        assert_eq!(stopped.requests.len(), 2, "Stop must not discard queued messages");
        let play = json!({"id":"play","type":"queue_control","sessionId":id,"generation":generation,"operation":{"type":"resume","runId":run}});
        assert_eq!(client.request(play.clone()).await["ok"], true);
        assert_eq!(client.request(play.clone()).await["ok"], true);
        assert!(model.requests.try_recv().is_err(), "The replacement run must wait for old tools/cleanup");
        cleanup.notify_one();
        let resumed = model.request().await;
        for text in ["one", "two"] {
            assert_eq!(resumed["messages"].as_array().unwrap().iter().filter(|m| m["role"] == "user" && m["content"] == text).count(), 1);
        }
        client.until(|m| m["type"] == "session_state" && m["sessionId"] == id && m["status"] == "idle").await;
        let settled = manager.inner.state.queue(&id).await.unwrap();
        assert!(!settled.paused && settled.control.is_none() && settled.requests.is_empty());
        // A lost acknowledgement must not resurrect the retired limit or restart work.
        assert_eq!(client.request(control).await["ok"], true);
        assert_eq!(client.request(play).await["ok"], true);
        assert!(manager.inner.state.queue(&id).await.unwrap().control.is_none());
        assert!(model.requests.try_recv().is_err());
        manager.shutdown().await; server.abort();
    }
}

#[tokio::test]
async fn play_recovers_legacy_orphaned_controls_even_with_a_stopped_run_id() {
    use tau_net::{QueueControl, QueueOperation, QueueRef};
    for action in ["prefix", "pause"] {
        let mut model = ModelServer::start(vec![completion("Recovered held work", vec![])]).await;
        let (_root, manager, _, server) = fixture(&model, Api::ChatCompletions).await;
        let id = manager.create_session(None, "general").await.unwrap();
        let runtime = manager.runtime(&id).await.unwrap();
        let generation = runtime.content.lock().await.transcript.as_ref().unwrap().generation.clone();
        manager.queue_control(&id, &generation, "hold", QueueOperation::Pause { run_id:None, boundary:"turn".into() }).await.unwrap();
        for text in ["one", "two"] { manager.prompt(&id, text, text).await.unwrap(); }
        {
            // Old versions could leave an idle paused chat with a control owned by
            // a run that had already ended. No database surgery should be needed.
            let mut content = runtime.content.lock().await;
            let mut queue = content.transcript.as_ref().unwrap().queue.clone();
            queue.control = Some(QueueControl { command_id:"orphan".into(), run_id:Some("stopped-run".into()), action:action.into(),
                boundary:Some("turn".into()), requests:if action == "prefix" {vec![QueueRef {request_id:"one".into(), revision:0}]} else {vec![]},
                status:"waiting".into(), detail:None });
            content.save_queue(&id, queue, None).await.unwrap();
        }
        manager.queue_control(&id, &generation, "play", QueueOperation::Resume { run_id:Some("stopped-run".into()) }).await.unwrap();
        let resumed = model.request().await;
        assert_eq!(resumed["messages"][1]["content"], "one");
        assert_eq!(resumed["messages"][2]["content"], "two");
        tokio::time::timeout(Duration::from_secs(5), async {
            while runtime.content.lock().await.agent.as_ref().unwrap().running { tokio::time::sleep(Duration::from_millis(5)).await; }
        }).await.unwrap();
        let queue = manager.inner.state.queue(&id).await.unwrap();
        assert!(!queue.paused && queue.control.is_none() && queue.requests.is_empty());
        assert!(model.requests.try_recv().is_err());
        manager.shutdown().await; server.abort();
    }
}

#[tokio::test]
async fn newer_boundary_intents_replace_pending_ones_but_stale_inputs_do_not() {
    use tau_net::{QueueOperation, QueueRef};
    let response = Arc::new(Notify::new());
    let mut blocked = completion("Current response", vec![]); blocked.gate = Some(response.clone());
    let mut model = ModelServer::start(vec![blocked, completion("All queued work", vec![])]).await;
    let (_root, manager, _, server) = fixture(&model, Api::ChatCompletions).await;
    let id = manager.create_session(None, "general").await.unwrap();
    manager.prompt(&id, "Work", "work").await.unwrap(); model.request().await;
    for text in ["one", "two"] { manager.prompt(&id, text, text).await.unwrap(); }
    let runtime = manager.runtime(&id).await.unwrap();
    let (generation, run_id) = {
        let content = runtime.content.lock().await; let t = content.transcript.as_ref().unwrap();
        (t.generation.clone(), t.queue.run_id.clone())
    };
    let pause = QueueOperation::Pause { run_id:run_id.clone(), boundary:"turn".into() };
    let prefix = |revision| QueueOperation::Prefix { run_id:run_id.clone(), boundary:"turn".into(), requests:vec![QueueRef {request_id:"one".into(), revision}] };
    manager.queue_control(&id, &generation, "pause", pause.clone()).await.unwrap();
    for (key, scope, operation) in [
        ("stale-prefix", generation.as_str(), prefix(1)),
        ("stale-generation", "old-lineage", prefix(0)),
        ("stale-run", generation.as_str(), QueueOperation::Resume {run_id:Some("another-run".into())}),
    ] {
        assert!(manager.queue_control(&id, scope, key, operation).await.is_err());
        assert!(manager.inner.state.receipt(&id, key).await.unwrap().is_none());
        assert_eq!(manager.inner.state.queue(&id).await.unwrap().control.unwrap().command_id, "pause");
    }
    manager.queue_control(&id, &generation, "prefix", prefix(0)).await.unwrap();
    // Editing a selected prefix still needs cancellation/replacement; failed
    // validation must leave the existing accepted boundary intent untouched.
    assert!(manager.queue_control(&id, &generation, "edit-selected", QueueOperation::Edit {request_id:"one".into(), revision:0, text:"changed".into()}).await.is_err());
    manager.queue_control(&id, &generation, "pause", pause.clone()).await.unwrap();
    assert_eq!(manager.inner.state.queue(&id).await.unwrap().control.unwrap().command_id, "prefix", "Retrying an old receipt must not resurrect its intent");
    manager.queue_control(&id, &generation, "pause-new", pause).await.unwrap();
    manager.queue_control(&id, &generation, "prefix-new", prefix(0)).await.unwrap();
    manager.queue_control(&id, &generation, "play", QueueOperation::Resume {run_id}).await.unwrap();
    assert!(manager.inner.state.queue(&id).await.unwrap().control.is_none());
    response.notify_one(); model.request().await;
    tokio::time::timeout(Duration::from_secs(5), async {
        while runtime.content.lock().await.agent.as_ref().unwrap().running { tokio::time::sleep(Duration::from_millis(5)).await; }
    }).await.unwrap();
    assert!(manager.inner.state.queue(&id).await.unwrap().requests.is_empty());
    assert!(model.requests.try_recv().is_err());
    manager.shutdown().await; server.abort();
}

#[tokio::test]
async fn run_through_limits_the_queue_and_a_later_pause_wins_during_stop_cleanup() {
    use tau_net::{QueueOperation, QueueRef};
    for pause_again in [false, true] {
        let mut blocked = completion("Stopped response", vec![]); blocked.gate = Some(Arc::new(Notify::new()));
        let mut model = ModelServer::start(vec![blocked, completion("Selected work", vec![]), completion("Remaining work", vec![])]).await;
        let (_root, manager, _, server) = fixture(&model, Api::ChatCompletions).await;
        let id = manager.create_session(None, "general").await.unwrap();
        let cleanup = Arc::new(Notify::new()); *manager.inner.settle_gate.lock().unwrap() = Some(cleanup.clone());
        manager.prompt(&id, "Work", "work").await.unwrap(); model.request().await;
        for text in ["one", "two"] { manager.prompt(&id, text, text).await.unwrap(); }
        manager.abort(&id, "stop").await.unwrap();
        let runtime = manager.runtime(&id).await.unwrap();
        let (generation, run_id) = {
            let content = runtime.content.lock().await; let t = content.transcript.as_ref().unwrap();
            (t.generation.clone(), t.queue.run_id.clone())
        };
        manager.queue_control(&id, &generation, "through-one", QueueOperation::Prefix { run_id:run_id.clone(), boundary:"turn".into(),
            requests:vec![QueueRef {request_id:"one".into(), revision:0}] }).await.unwrap();
        if pause_again {
            manager.queue_control(&id, &generation, "pause-later", QueueOperation::Pause {run_id, boundary:"turn".into()}).await.unwrap();
        }
        assert!(model.requests.try_recv().is_err(), "New work must wait for cleanup");
        cleanup.notify_one();
        if !pause_again {
            let selected = model.request().await;
            let messages = selected["messages"].as_array().unwrap();
            assert!(messages.iter().any(|m| m["role"] == "user" && m["content"] == "one"));
            assert!(!messages.iter().any(|m| m["role"] == "user" && m["content"] == "two"));
        }
        tokio::time::timeout(Duration::from_secs(5), async {
            while runtime.content.lock().await.agent.as_ref().unwrap().running { tokio::time::sleep(Duration::from_millis(5)).await; }
        }).await.unwrap();
        let queue = manager.inner.state.queue(&id).await.unwrap();
        assert!(queue.paused);
        assert_eq!(queue.requests.len(), if pause_again {2} else {1});
        let control = queue.control.unwrap();
        assert_eq!(control.status, "applied");
        assert_eq!(control.command_id, if pause_again {"pause-later"} else {"through-one"});
        assert!(model.requests.try_recv().is_err(), "A limit or later pause must not silently drain the tail");
        if !pause_again {
            manager.queue_control(&id, &generation, "play-tail", QueueOperation::Resume {run_id:None}).await.unwrap();
            assert_eq!(model.request().await["messages"].as_array().unwrap().last().unwrap()["content"], "two");
        }
        manager.shutdown().await; server.abort();
    }
}

#[tokio::test]
async fn provider_failure_retires_a_waiting_limit_without_automatically_retrying() {
    use tau_net::{QueueOperation, QueueRef, SessionStatus};
    let response = Arc::new(Notify::new());
    let failed = Reply {status:400, bytes:br#"{"error":{"message":"fixture provider failure"}}"#.to_vec(), gate:Some(response.clone()), body_gate:None};
    let mut model = ModelServer::start(vec![failed, completion("Explicitly recovered", vec![])]).await;
    let (_root, manager, _, server) = fixture(&model, Api::ChatCompletions).await;
    let id = manager.create_session(None, "general").await.unwrap();
    manager.prompt(&id, "Work", "work").await.unwrap(); model.request().await;
    manager.prompt(&id, "one", "one").await.unwrap();
    let runtime = manager.runtime(&id).await.unwrap();
    let (generation, run_id) = {
        let content = runtime.content.lock().await; let t = content.transcript.as_ref().unwrap();
        (t.generation.clone(), t.queue.run_id.clone())
    };
    manager.queue_control(&id, &generation, "limit", QueueOperation::Prefix {run_id, boundary:"turn".into(),
        requests:vec![QueueRef {request_id:"one".into(), revision:0}]}).await.unwrap();
    response.notify_one();
    tokio::time::timeout(Duration::from_secs(5), async {
        while runtime.snapshot().status != SessionStatus::Error { tokio::time::sleep(Duration::from_millis(5)).await; }
    }).await.unwrap();
    let queue = manager.inner.state.queue(&id).await.unwrap();
    assert!(queue.paused && queue.run_id.is_none());
    assert_eq!(queue.requests.len(), 1);
    let control = queue.control.unwrap(); assert_eq!(control.status, "failed");
    assert!(control.detail.unwrap().contains("fixture provider failure"));
    assert!(model.requests.try_recv().is_err(), "Provider/storage errors must still wait for explicit recovery");
    manager.queue_control(&id, &generation, "play", QueueOperation::Resume {run_id:None}).await.unwrap();
    model.request().await;
    tokio::time::timeout(Duration::from_secs(5), async {
        while runtime.content.lock().await.agent.as_ref().unwrap().running { tokio::time::sleep(Duration::from_millis(5)).await; }
    }).await.unwrap();
    assert!(!manager.inner.state.queue(&id).await.unwrap().paused);
    manager.shutdown().await; server.abort();
}
