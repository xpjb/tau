use super::*;
use crate::protocol::{PromptDisposition, ServerMessage, SessionStatus};

async fn activity(manager: &AgentManager, id: &str) -> u64 {
    manager.inner.state.get(id).await.unwrap().unwrap().updated_at_ms
}
async fn settled(manager: &AgentManager, id: &str) {
    let runtime = manager.runtime(id).await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while runtime.snapshot().status == SessionStatus::Running {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }).await.unwrap();
}

#[tokio::test]
async fn chat_activity_bumps_acceptance_and_settlement_not_streaming_tools_or_queued_continuations() {
    let first_gate = Arc::new(Notify::new());
    let mut first = completion("Intermediate commentary", vec![call("write", "write", json!({"path":"result.txt","content":"done"}))]);
    first.gate = Some(first_gate.clone());
    let stream_gate = Arc::new(Notify::new());
    let mut second = completion("Streaming final answer", vec![]);
    let split = second.bytes.windows(4).position(|b| b == b"\r\n\r\n").unwrap() + 4;
    second.body_gate = Some((split, stream_gate.clone()));
    let last_gate = Arc::new(Notify::new());
    let mut last = completion("Settled final answer", vec![]);
    last.gate = Some(last_gate.clone());
    let mut model = ModelServer::start(vec![first, second, last]).await;
    let (root, manager, _, server) = fixture(&model, Api::ChatCompletions).await;
    let id = manager.create_session(None, "general").await.unwrap();
    let created = activity(&manager, &id).await;
    let mut notices = manager.subscribe();
    manager.prompt(&id, "first prompt", "first").await.unwrap();
    let accepted = activity(&manager, &id).await;
    assert!(accepted > created);
    let mut refresh = false;
    while let Ok(message) = notices.try_recv() {
        refresh |= matches!(message, ServerMessage::ResyncRequired { session_id: None });
    }
    assert!(refresh, "accepted sends publish activity without waiting for an agent reply");
    model.request().await;
    let newer = manager.create_session(Some(&id), "general").await.unwrap();
    assert!(activity(&manager, &newer).await > accepted);
    first_gate.notify_one();
    model.request().await;
    let runtime = manager.runtime(&id).await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let live = runtime.content.lock().await.transcript.as_ref().unwrap().page(None).events
                .iter().any(|e| e.phase == crate::transcript::EventPhase::Live && e.text == "Streaming final answer");
            if live { break; }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }).await.unwrap();
    assert_eq!(std::fs::read_to_string(root.path().join("result.txt")).unwrap(), "done");
    assert_eq!(activity(&manager, &id).await, accepted, "auto title, saved commentary, tool calls/results and live answer must not bump");
    assert_eq!(manager.inner.state.list().await.unwrap()[0].0, newer);

    assert!(matches!(manager.prompt(&id, "queued follow-up", "queued").await.unwrap().disposition, PromptDisposition::Queued));
    let queued = activity(&manager, &id).await;
    assert!(queued > activity(&manager, &newer).await);
    let newest = manager.create_session(Some(&newer), "general").await.unwrap();
    stream_gate.notify_one();
    model.request().await; // queued user is consumed, but the agent has not settled
    assert_eq!(activity(&manager, &id).await, queued);
    assert_eq!(manager.inner.state.list().await.unwrap()[0].0, newest);
    last_gate.notify_one();
    settled(&manager, &id).await;
    let finished = activity(&manager, &id).await;
    assert!(finished > activity(&manager, &newest).await);
    assert_eq!(manager.inner.state.list().await.unwrap()[0].0, id);
    let ServerMessage::SessionPage { sessions, .. } = manager.list_page("catalog".into(), false, None, 0).await.unwrap() else { panic!() };
    assert_eq!(sessions.iter().find(|s| s.id == id).unwrap().updated_at_ms, finished);
    // Lost acknowledgements, reopening and daemon restart are not activity.
    manager.prompt(&id, "first prompt", "first").await.unwrap();
    manager.prompt(&id, "queued follow-up", "queued").await.unwrap();
    manager.close_session(&id).await.unwrap();
    assert_eq!(activity(&manager, &id).await, finished);
    let config = manager.inner.config.clone();
    manager.shutdown().await; server.abort(); drop(manager);
    let manager = AgentManager::new(config.clone(), StateStore::load(config.database_path).await.unwrap()).await.unwrap();
    assert_eq!(activity(&manager, &id).await, finished);
    assert_eq!(manager.inner.state.list().await.unwrap()[0].0, id);
    manager.shutdown().await;
}

#[tokio::test]
async fn chat_activity_error_and_abort_bump_only_when_the_run_settles() {
    for abort in [false, true] {
        let gate = Arc::new(Notify::new());
        let reply = Reply { status: 400, bytes: b"fixture failure".to_vec(), gate: Some(gate.clone()), body_gate: None };
        let mut model = ModelServer::start(vec![reply]).await;
        let (_root, manager, _, server) = fixture(&model, Api::ChatCompletions).await;
        let id = manager.create_session(None, "general").await.unwrap();
        manager.prompt(&id, "prompt", "send").await.unwrap();
        model.request().await;
        let accepted = activity(&manager, &id).await;
        let newer = manager.create_session(Some(&id), "general").await.unwrap();
        assert_eq!(activity(&manager, &id).await, accepted);
        if abort { manager.abort(&id, "abort").await.unwrap(); }
        else { gate.notify_one(); }
        settled(&manager, &id).await;
        assert!(activity(&manager, &id).await > activity(&manager, &newer).await);
        assert_eq!(manager.runtime(&id).await.unwrap().snapshot().status, if abort { SessionStatus::Idle } else { SessionStatus::Error });
        let finished = activity(&manager, &id).await;
        manager.abort(&id, "idle-abort").await.unwrap();
        assert_eq!(activity(&manager, &id).await, finished, "idle controls are not agent completions");
        manager.shutdown().await; server.abort();
    }
}

#[tokio::test]
async fn chat_activity_failed_acceptance_does_not_bump_and_background_metadata_stays_quiet() {
    let model = ModelServer::start(vec![]).await;
    let (_root, manager, _, server) = fixture(&model, Api::ChatCompletions).await;
    let id = manager.create_session(None, "general").await.unwrap();
    let before = activity(&manager, &id).await;
    manager.inner.state.access(|db| {
        db.execute_batch("CREATE TRIGGER reject_queue BEFORE INSERT ON queue BEGIN SELECT RAISE(ABORT,'fixture disk failure'); END;")?;
        Ok(())
    }).await.unwrap();
    assert!(manager.prompt(&id, "not accepted", "failed").await.is_err());
    assert_eq!(activity(&manager, &id).await, before);
    assert!(manager.inner.state.receipt(&id, "failed").await.unwrap().is_none());
    assert!(model.requests.is_empty());
    let runtime = manager.runtime(&id).await.unwrap();
    let mut content = runtime.content.lock().await;
    let queue = content.transcript.as_ref().unwrap().queue.clone();
    content.save_queue(&id, queue, None).await.unwrap();
    content.append(&id, json!({"type":"thinking_level_change","thinkingLevel":"high"})).await.unwrap();
    drop(content);
    manager.inner.state.rename(&id, "Generated title".into(), true).await.unwrap();
    assert_eq!(activity(&manager, &id).await, before);
    manager.shutdown().await; server.abort();
}
