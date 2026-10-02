use super::*;
use tau_net::{ChatCreation, SessionModel};

#[tokio::test]
async fn starting_model_and_prompt_commit_atomically_and_receipts_pin_both() {
    let model = ModelServer::start(vec![]).await;
    let (_root, manager, _url, server) = fixture(&model, Api::ChatCompletions).await;
    let id = manager.create_session(None, "general").await.unwrap();
    let runtime = manager.runtime(&id).await.unwrap();
    {
        let mut content = runtime.content.lock().await;
        let mut queue = content.transcript.as_ref().unwrap().queue.clone(); queue.paused = true;
        content.save_queue(&id, queue, None).await.unwrap();
    }
    let before = manager.inner.state.get(&id).await.unwrap().unwrap();
    let chosen: SessionModel = "openai-codex/manual-exact-id".parse().unwrap();
    manager.inner.state.access(|db| {
        db.execute_batch("CREATE TRIGGER fail_first_send BEFORE INSERT ON receipts WHEN NEW.request_id='first' BEGIN SELECT RAISE(ABORT,'full'); END")?;
        Ok(())
    }).await.unwrap();
    assert!(manager.prompt_with_model(&id, "first text", "first", Some(&chosen)).await.is_err());
    assert_eq!(manager.inner.state.get(&id).await.unwrap().unwrap().model, before.model);
    assert_eq!(runtime.content.lock().await.agent.as_ref().unwrap().model, before.model);
    assert!(manager.inner.state.queue(&id).await.unwrap().requests.is_empty());
    assert!(manager.inner.state.receipt(&id, "first").await.unwrap().is_none());
    manager.inner.state.access(|db| { db.execute_batch("DROP TRIGGER fail_first_send")?; Ok(()) }).await.unwrap();
    manager.prompt_with_model(&id, "first text", "first", Some(&chosen)).await.unwrap();
    assert_eq!(manager.inner.state.get(&id).await.unwrap().unwrap().model, chosen);
    assert_eq!(manager.inner.state.receipt(&id, "first").await.unwrap().unwrap().model, Some(chosen.clone()));
    manager.prompt_with_model(&id, "first text", "first", Some(&chosen)).await.unwrap();
    assert_eq!(manager.inner.state.queue(&id).await.unwrap().requests.len(), 1);
    for (text, choice) in [("changed text", Some(&chosen)), ("first text", None), ("first text", Some(&before.model))] {
        assert!(manager.prompt_with_model(&id, text, "first", choice).await.is_err(), "An ID owns text AND model");
    }
    assert!(manager.prompt_with_model(&id, "new text", "wrong-model", Some(&before.model)).await.is_err());
    assert!(manager.inner.state.receipt(&id, "wrong-model").await.unwrap().is_none());
    // A second send authored before the first receipt can pin the same model,
    // but must not reset an explicitly changed thinking level.
    manager.prompt(&id, "/thinking low", "thinking").await.unwrap();
    manager.prompt_with_model(&id, "second text", "second", Some(&chosen)).await.unwrap();
    assert_eq!(manager.inner.state.get(&id).await.unwrap().unwrap().thinking, "low");
    assert_eq!(manager.inner.state.queue(&id).await.unwrap().requests.len(), 2);
    assert!(manager.prompt_with_model(&id, "/model openai-codex/other", "slash", Some(&chosen)).await.is_err());
    manager.shutdown().await; server.abort();
}

#[tokio::test]
async fn pipelined_first_prompt_needs_no_create_response_and_cannot_recreate_a_deleted_chat() {
    let mut model = ModelServer::start(vec![completion("right model", vec![])]).await;
    let (_root, manager, url, server) = fixture(&model, Api::ChatCompletions).await;
    let mut client = Client::connect(&url).await;
    let id = uuid::Uuid::new_v4().to_string();
    let creation = ChatCreation { project_id: "general".into(), keep_session_id: None };
    // No standalone create or /model call at all: the first send carries both.
    let send = json!({"id":"first","type":"prompt","sessionId":id,"text":"start now",
        "model":{"provider":"openai-codex","modelId":"manual-exact-id"}, "create":creation});
    let response = client.request(send.clone()).await;
    assert_eq!(response["ok"], true, "{response}");
    assert_eq!(model.request().await["model"], "manual-exact-id");
    let original_model = manager.inner.state.get(&id).await.unwrap().unwrap().model;
    let (left, right) = tokio::join!(manager.create_session_operation(&id, &creation), manager.create_session_operation(&id, &creation));
    assert_eq!(left.unwrap(), id); assert_eq!(right.unwrap(), id);
    assert_eq!(manager.inner.state.get(&id).await.unwrap().unwrap().model, original_model, "Delayed creation never resets the chosen model");
    assert_eq!(client.request(send.clone()).await["ok"], true, "Lost prompt response uses its original receipt");
    assert!(model.requests.try_recv().is_err(), "One provider call despite duplicate delivery");
    let mut other_topic = creation.clone(); other_topic.project_id = "not-the-original".into();
    assert!(manager.create_session_operation(&id, &other_topic).await.is_err());
    manager.delete_session(&id).await.unwrap();
    assert_eq!(client.request(send).await["ok"], false);
    assert!(manager.inner.state.get(&id).await.unwrap().is_none(), "A queued retry cannot recreate deleted chats");
    assert!(model.requests.try_recv().is_err());
    manager.shutdown().await; server.abort();
}

#[tokio::test]
async fn interrupted_creation_stays_uncertain_and_never_creates_or_executes_on_retry() {
    let mut model = ModelServer::start(vec![]).await;
    let (_root, manager, url, server) = fixture(&model, Api::ChatCompletions).await;
    let id = uuid::Uuid::new_v4().to_string();
    let creation = ChatCreation { project_id: "general".into(), keep_session_id: None };
    let request = tau_net::ClientRequest { id: id.clone(), command: tau_net::ClientCommand::CreateSession {
        project_id: creation.project_id.clone(), keep_session_id: None,
    }};
    assert!(manager.inner.state.reserve_operation(&request).await.unwrap().is_none());
    manager.inner.state.recover_operations().await.unwrap();
    let mut client = Client::connect(&url).await;
    let response = client.request(serde_json::to_value(request).unwrap()).await;
    assert_eq!(response["uncertain"], true);
    let response = client.request(json!({"id":"first","type":"prompt","sessionId":id,"text":"never replay",
        "model":{"provider":"openai-codex","modelId":"manual-exact-id"}, "create":creation})).await;
    assert_eq!(response["uncertain"], true);
    assert_eq!(response["ok"], false);
    assert!(manager.inner.state.get(&id).await.unwrap().is_none());
    assert!(model.requests.try_recv().is_err());
    manager.shutdown().await; server.abort();
}
