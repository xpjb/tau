//! Interrupted Codex reasoning must survive transport failure, cancellation and reload.
use super::*;

fn reasoning(id: &str) -> Value {
    json!({"type":"reasoning","id":id,"status":"completed","encrypted_content":format!("private-cipher-{id}"),
        "summary":[{"type":"summary_text","text":"Saved reasoning summary"}]})
}
fn frame(event: Value) -> Vec<u8> { format!("data: {event}\n\n").into_bytes() }
fn interrupted(items: &[Value]) -> Reply {
    let mut bytes=frame(json!({"type":"response.reasoning_summary_text.delta","delta":"Visible interrupted thinking"}));
    for (index,item) in items.iter().enumerate() {
        bytes.extend(frame(json!({"type":"response.output_item.done","output_index":index,"item":item})));
    }
    bytes.extend(frame(json!({"type":"response.output_text.delta","delta":"discard interrupted prose"})));
    bytes.extend(frame(json!({"type":"response.output_item.added","output_index":items.len(),"item":{
        "type":"function_call","call_id":"partial-call","name":"bash","arguments":""}})));
    bytes.extend(frame(json!({"type":"response.function_call_arguments.delta","output_index":items.len(),
        "delta":"{\"command\":\"touch must-not-exist\"}"})));
    Reply {status:200,bytes,gate:None,body_gate:None}
}
async fn resume(client: &mut Client, id: &str, command: &str) {
    let snapshot=client.open(id).await;
    assert_eq!(client.request(json!({"id":command,"type":"queue_control","sessionId":id,
        "generation":snapshot["generation"],"operation":{"type":"resume","runId":null}})).await["ok"],true);
}
fn count_item(payload: &Value, id: &str) -> usize {
    payload["input"].as_array().unwrap().iter().filter(|item|item["id"]==id).count()
}

#[tokio::test]
async fn checkpoint_recovery_continues_repeated_drops_and_tool_result_without_pausing() {
    let good=reasoning("rs_good");let first=reasoning("rs_first");let second=reasoning("rs_second");let third=reasoning("rs_third");
    let effect=json!({"type":"function_call","call_id":"effect","name":"bash","arguments":json!({"command":"printf x >> effects"}).to_string()});
    let mut model=ModelServer::start(vec![codex("Prior complete answer",vec![good]), interrupted(std::slice::from_ref(&first)),
        interrupted(&[first,second]),interrupted(&[third]),codex("",vec![effect]),codex("Finished",vec![])]).await;
    let (root,manager,url,server)=fixture(&model,Api::Codex).await;
    let mut settings=manager.inner.settings.get();settings.agent.retry.max_retries=1;manager.set_settings(settings.revision,settings).await.unwrap();
    let mut client=Client::connect(&url).await;
    let id=manager.create_session(None,"general").await.unwrap();
    manager.prompt(&id,"Earlier completed task","first").await.unwrap(); model.request().await;
    client.until(|m|m["type"]=="session_state" && m["sessionId"]==id && m["status"]=="idle").await;
    let seen=client.seen.len();
    manager.prompt(&id,"Continue automatically","second").await.unwrap();
    for n in 0..5 {
        let payload=model.request().await;
        assert_eq!(count_item(&payload,"rs_good"),1);
        for (at,checkpoint) in [(1,"rs_first"),(2,"rs_second"),(3,"rs_third")] {
            assert_eq!(count_item(&payload,checkpoint),usize::from(n>=at),"request {n}: {checkpoint}");
        }
        assert!(!payload["input"].to_string().contains("discard interrupted prose"));
        assert!(!payload["input"].to_string().contains("partial-call"));
        assert!(!payload["input"].to_string().contains("must-not-exist"));
    }
    client.until(|m|m["type"]=="session_state" && m["sessionId"]==id && m["status"]=="idle").await;
    assert!(!client.seen[seen..].iter().any(|m|m["type"]=="session_state" && m["status"]=="error"));
    assert!(!manager.inner.state.queue(&id).await.unwrap().paused);
    assert_eq!(tokio::fs::read(root.path().join("effects")).await.unwrap(),b"x");
    assert!(!root.path().join("must-not-exist").exists());
    let history=manager.inner.state.context(&id,&manager.inner.settings.get().agent.model).await.unwrap();
    let failures=history.iter().filter(|e|e["message"]["stopReason"]=="error").collect::<Vec<_>>();
    assert_eq!(failures.len(),3);
    for entry in failures {
        assert!(entry["message"]["tauReasoningRecovery"]["items"].is_array());
        assert!(entry["message"].get("errorMessage").is_none(),"Recovered transport failures stay silent");
    }
    let display=client.open(&id).await;
    for message in client.seen.iter().chain(std::iter::once(&display)) {
        assert!(!message.to_string().contains("private-cipher")); assert!(!message.to_string().contains("fixture-account"));
    }
    assert!(model.requests.try_recv().is_err());manager.shutdown().await;server.abort();
}

#[tokio::test]
async fn checkpoint_recovery_abort_survives_restart_clone_and_steering() {
    let checkpoint=reasoning("rs_abort");let gate=Arc::new(Notify::new());
    let mut stopped=interrupted(std::slice::from_ref(&checkpoint));
    stopped.bytes.extend(frame(json!({"type":"response.reasoning_summary_text.delta","output_index":1,"summary_index":0,"delta":"checkpoint fully received"})));
    stopped.body_gate=Some((stopped.bytes.len(),gate));stopped.bytes.extend_from_slice(b": wait\n\nxxxxxxxxxxxxxx");
    let mut model=ModelServer::start(vec![stopped,codex("Parent resumed",vec![]),codex("Clone resumed",vec![])]).await;
    let (root,manager,url,server)=fixture(&model,Api::Codex).await;
    let mut client=Client::connect(&url).await;let id=manager.create_session(None,"general").await.unwrap();
    manager.prompt(&id,"A task","initial").await.unwrap();model.request().await;
    tokio::time::timeout(Duration::from_secs(5),async {loop {
        if client.page(&id,None).await["events"].as_array().unwrap().iter().any(|e|e["text"].as_str().is_some_and(|t|t.contains("checkpoint fully received"))) {break;}
        tokio::time::sleep(Duration::from_millis(10)).await;
    }}).await.unwrap();
    assert_eq!(client.request(json!({"id":"stop","type":"abort","sessionId":id})).await["ok"],true);
    client.until(|m|m["type"]=="session_state" && m["sessionId"]==id && m["status"]=="idle").await;
    assert_eq!(manager.inner.state.context(&id,&manager.inner.settings.get().agent.model).await.unwrap().last().unwrap()["message"]["tauReasoningRecovery"]["items"],json!([checkpoint]));
    let clone=manager.clone_session(&id).await.unwrap();let config=manager.inner.config.clone();
    client.socket.close(None).await.unwrap();manager.shutdown().await;server.abort();drop(manager);
    let manager=AgentManager::new(config.clone(),StateStore::load(config.database_path.clone()).await.unwrap()).await.unwrap();
    let (url,server)=serve(&manager).await;let mut client=Client::connect(&url).await;
    manager.prompt(&id,"New steering after the checkpoint","steer").await.unwrap();
    resume(&mut client,&id,"resume-parent").await;
    let payload=model.request().await;assert_eq!(count_item(&payload,"rs_abort"),1);
    let input=payload["input"].as_array().unwrap();let at=input.iter().position(|i|i["id"]=="rs_abort").unwrap();
    assert!(input[at+1..].iter().any(|i|i["role"]=="user" && i.to_string().contains("New steering")));
    client.until(|m|m["type"]=="session_state" && m["sessionId"]==id && m["status"]=="idle").await;
    resume(&mut client,&clone,"resume-clone").await;let payload=model.request().await;
    assert_eq!(count_item(&payload,"rs_abort"),1);assert!(!payload.to_string().contains("New steering"));
    client.until(|m|m["type"]=="session_state" && m["sessionId"]==clone && m["status"]=="idle").await;
    assert!(!root.path().join("must-not-exist").exists());manager.shutdown().await;server.abort();
}

#[tokio::test]
async fn checkpoint_recovery_rejection_is_durable_and_does_not_drop_unrelated_history() {
    let bad=Reply {status:400,bytes:json!({"error":{"code":"invalid_encrypted_content","message":"Invalid encrypted_content in reasoning item"}}).to_string().into_bytes(),gate:None,body_gate:None};
    let mut model=ModelServer::start(vec![codex("Earlier answer",vec![reasoning("rs_good")]),interrupted(&[reasoning("rs_rejected")]),bad,codex("Clean continuation",vec![])]).await;
    let (_root,manager,url,server)=fixture(&model,Api::Codex).await;
    let mut settings=manager.inner.settings.get();settings.agent.retry.enabled=false;manager.set_settings(settings.revision,settings).await.unwrap();
    let mut client=Client::connect(&url).await;let id=manager.create_session(None,"general").await.unwrap();
    manager.prompt(&id,"Earlier task","one").await.unwrap();model.request().await;
    client.until(|m|m["type"]=="session_state" && m["sessionId"]==id && m["status"]=="idle").await;
    manager.prompt(&id,"Next task","two").await.unwrap();model.request().await;
    client.until(|m|m["type"]=="session_state" && m["sessionId"]==id && m["status"]=="error").await;
    resume(&mut client,&id,"rejected").await;assert_eq!(count_item(&model.request().await,"rs_rejected"),1);
    client.until(|m|m["type"]=="session_state" && m["sessionId"]==id && m["status"]=="error").await;
    let config=manager.inner.config.clone();client.socket.close(None).await.unwrap();manager.shutdown().await;server.abort();drop(manager);
    let manager=AgentManager::new(config.clone(),StateStore::load(config.database_path.clone()).await.unwrap()).await.unwrap();
    let (url,server)=serve(&manager).await;let mut client=Client::connect(&url).await;
    resume(&mut client,&id,"clean").await;let payload=model.request().await;
    assert_eq!(count_item(&payload,"rs_rejected"),0);assert_eq!(count_item(&payload,"rs_good"),1);
    client.until(|m|m["type"]=="session_state" && m["sessionId"]==id && m["status"]=="idle").await;
    assert!(model.requests.try_recv().is_err());manager.shutdown().await;server.abort();
}

#[tokio::test]
async fn checkpoint_recovery_no_progress_has_a_bounded_retry_budget() {
    let mut model=ModelServer::start((0..3).map(|_|interrupted(&[])).collect()).await;
    let (root,manager,url,server)=fixture(&model,Api::Codex).await;
    let mut settings=manager.inner.settings.get();settings.agent.retry.max_retries=2;manager.set_settings(settings.revision,settings).await.unwrap();
    let mut client=Client::connect(&url).await;let id=manager.create_session(None,"general").await.unwrap();
    manager.prompt(&id,"Task","one").await.unwrap();
    for _ in 0..3 {model.request().await;}
    let error=client.until(|m|m["type"]=="session_state" && m["sessionId"]==id && m["status"]=="error").await;
    assert!(error["detail"].as_str().unwrap().contains("incomplete"));
    assert!(manager.inner.state.queue(&id).await.unwrap().paused);
    assert!(!root.path().join("must-not-exist").exists());assert!(model.requests.try_recv().is_err());
    manager.shutdown().await;server.abort();
}

#[tokio::test]
async fn checkpoint_recovery_idle_timeout_continues_with_the_checkpoint() {
    let mut stalled=interrupted(&[reasoning("rs_stalled")]);
    stalled.body_gate=Some((stalled.bytes.len(),Arc::new(Notify::new())));stalled.bytes.extend_from_slice(b": waiting\n\nxxxxxxxxxxxxxx");
    let mut model=ModelServer::start(vec![stalled,codex("Continued",vec![])]).await;
    let (_root,manager,url,server)=fixture(&model,Api::Codex).await;
    let mut settings=manager.inner.settings.get();settings.agent.http_idle_timeout_seconds=1;manager.set_settings(settings.revision,settings).await.unwrap();
    let mut client=Client::connect(&url).await;let id=manager.create_session(None,"general").await.unwrap();
    manager.prompt(&id,"Task","one").await.unwrap();model.request().await;
    assert_eq!(count_item(&model.request().await,"rs_stalled"),1);
    client.until(|m|m["type"]=="session_state" && m["sessionId"]==id && m["status"]=="idle").await;
    assert!(!manager.inner.state.queue(&id).await.unwrap().paused);manager.shutdown().await;server.abort();
}

#[tokio::test]
async fn checkpoint_recovery_foreign_model_account_endpoint_provider_and_api_are_excluded() {
    for change in ["model","account","endpoint","provider","api"] {
        let reply=if matches!(change,"api"|"provider") {completion("New API",vec![])} else {codex("Different scope",vec![])};
        let mut model=ModelServer::start(vec![interrupted(&[reasoning("rs_scoped")]),reply]).await;
        let (root,manager,url,server)=fixture(&model,Api::Codex).await;
        let mut settings=manager.inner.settings.get();settings.agent.retry.enabled=false;manager.set_settings(settings.revision,settings).await.unwrap();
        let mut client=Client::connect(&url).await;let id=manager.create_session(None,"general").await.unwrap();
        manager.prompt(&id,"Task","one").await.unwrap();model.request().await;
        client.until(|m|m["type"]=="session_state" && m["sessionId"]==id && m["status"]=="error").await;
        let mut settings=manager.inner.settings.get();
        let mut auth:Value=serde_json::from_slice(&tokio::fs::read(root.path().join("auth.json")).await.unwrap()).unwrap();
        match change {
            "account"=>auth["openai-codex"]["accountId"]=json!("different-fixture-account"),
            "endpoint"=>settings.providers.get_mut("openai-codex").unwrap().base_url=format!("{}/different",model.url),
            "api"=>settings.providers.get_mut("openai-codex").unwrap().api=Api::ChatCompletions,
            "provider"=>{let mut provider=settings.providers["openai-codex"].clone();provider.api=Api::ChatCompletions;settings.providers.insert("other-codex".into(),provider);auth["other-codex"]=json!({"type":"api_key","key":"fixture-other"});},
            _=>{},
        }
        if matches!(change,"model"|"provider") {
            let mut model=settings.models[0].clone();
            if change=="model" {model.id="other-model".into();} else {model.provider="other-codex".into();}
            settings.models.push(model);
        }
        manager.set_settings(settings.revision,settings).await.unwrap();
        crate::settings::atomic_write(&root.path().join("auth.json"),auth.to_string().as_bytes()).await.unwrap();
        if matches!(change,"model"|"provider") {
            let selection=if change=="model" {"openai-codex/other-model"} else {"other-codex/gpt-6-astra"};
            manager.prompt(&id,&format!("/model {selection}"),"switch").await.unwrap();
        }
        resume(&mut client,&id,"different").await;let payload=model.request().await;
        assert!(!payload.to_string().contains("private-cipher"),"{change}: foreign ciphertext sent");
        assert!(!payload.to_string().contains("reasoning_recovery"),"{change}: private envelope sent");
        assert!(!payload.to_string().contains("discard interrupted prose"));
        client.until(|m|m["type"]=="session_state" && m["sessionId"]==id && m["status"]=="idle").await;
        manager.shutdown().await;server.abort();
    }
}

#[tokio::test]
async fn checkpoint_recovery_keeps_valid_items_on_malformed_stream_and_does_not_retry_it() {
    let mut malformed=interrupted(&[reasoning("rs_valid_before_malformed")]);malformed.bytes.extend_from_slice(b"data: malformed-json\n\n");
    let mut model=ModelServer::start(vec![malformed,codex("Explicit continuation",vec![])]).await;
    let (_root,manager,url,server)=fixture(&model,Api::Codex).await;
    let mut client=Client::connect(&url).await;let id=manager.create_session(None,"general").await.unwrap();
    manager.prompt(&id,"Task","one").await.unwrap();model.request().await;
    let error=client.until(|m|m["type"]=="session_state" && m["sessionId"]==id && m["status"]=="error").await;
    assert!(error["detail"].as_str().unwrap().contains("valid JSON"));assert!(model.requests.try_recv().is_err());
    resume(&mut client,&id,"explicit").await;
    assert_eq!(count_item(&model.request().await,"rs_valid_before_malformed"),1);
    client.until(|m|m["type"]=="session_state" && m["sessionId"]==id && m["status"]=="idle").await;
    manager.shutdown().await;server.abort();
}

#[tokio::test]
async fn checkpoint_recovery_abort_during_backoff_preserves_the_checkpoint_and_stops_retrying() {
    let mut model=ModelServer::start(vec![interrupted(&[reasoning("rs_backoff")]),codex("Explicit continuation",vec![])]).await;
    let (_root,manager,url,server)=fixture(&model,Api::Codex).await;
    let mut settings=manager.inner.settings.get();settings.agent.retry.base_delay_ms=5000;manager.set_settings(settings.revision,settings).await.unwrap();
    let mut client=Client::connect(&url).await;let id=manager.create_session(None,"general").await.unwrap();
    manager.prompt(&id,"Task","one").await.unwrap();model.request().await;
    tokio::time::timeout(Duration::from_secs(3),async {loop {
        let entries=manager.inner.state.context(&id,&manager.inner.settings.get().agent.model).await.unwrap();
        if entries.iter().any(|e|e["message"]["tauReasoningRecovery"].is_object()) {break;}
        tokio::time::sleep(Duration::from_millis(10)).await;
    }}).await.unwrap();
    client.request(json!({"id":"abort-backoff","type":"abort","sessionId":id})).await;
    client.until(|m|m["type"]=="session_state" && m["sessionId"]==id && m["status"]=="idle").await;
    assert!(model.requests.try_recv().is_err());assert!(manager.inner.state.queue(&id).await.unwrap().paused);
    resume(&mut client,&id,"resume-after-stop").await;
    assert_eq!(count_item(&model.request().await,"rs_backoff"),1);
    client.until(|m|m["type"]=="session_state" && m["sessionId"]==id && m["status"]=="idle").await;
    manager.shutdown().await;server.abort();
}

#[tokio::test]
async fn checkpoint_recovery_is_folded_into_native_compaction() {
    let mut model=ModelServer::start(vec![interrupted(&[reasoning("rs_compact")]),codex("First answer",vec![]),codex("Second answer",vec![]),
        codex("",vec![json!({"type":"compaction","encrypted_content":"private-native-checkpoint"})]),codex("After compaction",vec![])]).await;
    let (_root,manager,url,server)=fixture(&model,Api::Codex).await;
    let mut client=Client::connect(&url).await;let id=manager.create_session(None,"general").await.unwrap();
    manager.prompt(&id,"First task","one").await.unwrap();model.request().await;
    assert_eq!(count_item(&model.request().await,"rs_compact"),1);
    client.until(|m|m["type"]=="session_state" && m["sessionId"]==id && m["status"]=="idle").await;
    manager.prompt(&id,"Second task","two").await.unwrap();assert_eq!(count_item(&model.request().await,"rs_compact"),1);
    client.until(|m|m["type"]=="session_state" && m["sessionId"]==id && m["status"]=="idle").await;
    manager.prompt(&id,"/compact","compact-recovery").await.unwrap();let compact=model.request().await;
    assert_eq!(count_item(&compact,"rs_compact"),1);assert_eq!(compact["input"].as_array().unwrap().last().unwrap()["type"],"compaction_trigger");
    client.until(|m|m["type"]=="session_state" && m["sessionId"]==id && m["status"]=="idle").await;
    manager.prompt(&id,"Continue after compaction","three").await.unwrap();let payload=model.request().await;
    assert_eq!(payload["input"][0]["type"],"compaction");assert_eq!(count_item(&payload,"rs_compact"),0);
    assert!(payload["input"].to_string().contains("private-native-checkpoint"));
    client.until(|m|m["type"]=="session_state" && m["sessionId"]==id && m["status"]=="idle").await;
    manager.shutdown().await;server.abort();
}

#[tokio::test]
async fn checkpoint_recovery_compaction_applies_rejections_after_its_retained_boundary() {
    let gate=Arc::new(Notify::new());let mut first=interrupted(&[reasoning("rs_rejected_prefix")]);
    first.body_gate=Some((first.bytes.len(),gate.clone()));first.bytes.extend_from_slice(b": gated\n\nxxxxxxxxxxxxxx");
    let reject=Reply {status:400,bytes:json!({"error":{"code":"invalid_encrypted_content"}}).to_string().into_bytes(),gate:None,body_gate:None};
    let mut model=ModelServer::start(vec![first,reject,codex("",vec![json!({"type":"compaction","encrypted_content":"private-clean-prefix"})])]).await;
    let (_root,manager,url,server)=fixture(&model,Api::Codex).await;
    let mut client=Client::connect(&url).await;let id=manager.create_session(None,"general").await.unwrap();
    manager.prompt(&id,"First task","one").await.unwrap();model.request().await;
    manager.prompt(&id,"Retained steering","two").await.unwrap();gate.notify_one();
    let continuation=model.request().await;assert_eq!(count_item(&continuation,"rs_rejected_prefix"),1);
    assert!(continuation["input"].to_string().contains("Retained steering"));
    client.until(|m|m["type"]=="session_state" && m["sessionId"]==id && m["status"]=="error").await;
    manager.prompt(&id,"/compact","compact-rejected").await.unwrap();let compact=model.request().await;
    assert_eq!(count_item(&compact,"rs_rejected_prefix"),0,"A suffix rejection also applies to the older compaction input");
    assert!(compact["input"].to_string().contains("First task"));assert!(!compact["input"].to_string().contains("Retained steering"));
    client.until(|m|m["type"]=="session_state" && m["sessionId"]==id && m["status"]=="idle").await;
    manager.shutdown().await;server.abort();
}

#[tokio::test]
async fn transient_stream_errors_retry_both_apis_without_pausing_or_executing_partial_tools() {
    for api in [Api::Codex, Api::ChatCompletions] {
        for code in [json!(503),json!(429),json!("upstream_error"),json!("upstream_stream_error"),json!("rate_limit_exceeded")] {
            let mut failed = if api == Api::Codex { interrupted(&[reasoning("rs_transient")]) } else {
                let mut reply = completion("Discard partial answer",vec![call("partial","bash",json!({"command":"touch must-not-exist"}))]);
                let end = reply.bytes.windows(4).position(|b| b==b"\r\n\r\n").unwrap()+4;
                reply.bytes.truncate(end); reply
            };
            failed.bytes.extend(frame(json!({"type":"error","error":{"code":code,"message":"fixture-access fixture-key fixture-account temporary upstream failure"}})));
            let effect = json!({"command":"printf x >> effects"});
            let replies = if api == Api::Codex {
                vec![failed,codex("",vec![json!({"type":"function_call","call_id":"effect","name":"bash","arguments":effect.to_string()})]),codex("Finished",vec![])]
            } else { vec![failed,completion("",vec![call("effect","bash",effect)]),completion("Finished",vec![])] };
            let mut model = ModelServer::start(replies).await;
            let (root,manager,url,server) = fixture(&model,api).await;
            let mut client = Client::connect(&url).await;
            let id = manager.create_session(None,"general").await.unwrap();
            manager.prompt(&id,"Finish without waiting for me","work").await.unwrap();
            model.request().await;
            let resumed = model.request().await;
            assert!(!resumed.to_string().contains("must-not-exist"));
            if api == Api::Codex { assert_eq!(count_item(&resumed,"rs_transient"),1); }
            model.request().await;
            client.until(|m| m["type"]=="session_state" && m["sessionId"]==id && m["status"]=="idle").await;
            assert!(!client.seen.iter().any(|m| m["type"]=="session_state" && m["status"]=="error"));
            assert!(!manager.inner.state.queue(&id).await.unwrap().paused);
            assert_eq!(tokio::fs::read(root.path().join("effects")).await.unwrap(),b"x");
            assert!(!root.path().join("must-not-exist").exists());
            let history = manager.inner.state.context(&id,&manager.inner.settings.get().agent.model).await.unwrap();
            let failure = history.iter().find(|e|e["message"]["stopReason"]=="error").unwrap();
            assert!(failure["message"].get("errorMessage").is_none());
            let display = client.page(&id,None).await.to_string();
            assert!(display.contains("Retrying automatically") && display.contains("temporary upstream failure"));
            assert!(!display.contains(if api == Api::Codex {"fixture-access"} else {"fixture-key"}));
            if api == Api::Codex { assert!(!display.contains("fixture-account")); }
            assert!(model.requests.try_recv().is_err());
            manager.shutdown().await; server.abort();
        }
    }
}

#[tokio::test]
async fn stream_errors_at_eof_keep_their_details_and_permanent_errors_do_not_retry() {
    for api in [Api::Codex,Api::ChatCompletions] {
        for code in ["invalid_api_key","insufficient_quota","invalid_request_error","invalid_encrypted_content"] {
            let bytes = format!("data: {}",json!({"type":"error","error":{"code":code,"message":"Deliberate permanent failure"}})).into_bytes();
            let mut model = ModelServer::start(vec![Reply {status:200,bytes,gate:None,body_gate:None}]).await;
            let (_root,manager,url,server) = fixture(&model,api).await;
            let mut client = Client::connect(&url).await;
            let id = manager.create_session(None,"general").await.unwrap();
            manager.prompt(&id,"Task","work").await.unwrap(); model.request().await;
            let error = client.until(|m|m["type"]=="session_state" && m["sessionId"]==id && m["status"]=="error").await;
            assert!(error["detail"].as_str().unwrap().contains(code),"{error}");
            assert!(error["detail"].as_str().unwrap().contains("Deliberate permanent failure"));
            assert!(model.requests.try_recv().is_err());
            manager.shutdown().await; server.abort();
        }
    }
}

#[tokio::test]
async fn compaction_retries_a_transient_stream_error_without_changing_history_twice() {
    for native in [true,false] {
        let failure = Reply {status:200,bytes:frame(json!({"type":"response.failed","response":{"error":{"code":"upstream_error","message":"try again"}}})),gate:None,body_gate:None};
        let checkpoint = if native { codex("",vec![json!({"type":"compaction","encrypted_content":"checkpoint"})]) }
            else {codex("Task summary",vec![])};
        let mut model = ModelServer::start(vec![codex("First answer",vec![]),failure,checkpoint]).await;
        let (_root,manager,url,server) = fixture(&model,Api::Codex).await;
        let mut settings = manager.inner.settings.get(); settings.agent.compaction.native_codex = native;
        manager.set_settings(settings.revision,settings).await.unwrap();
        let mut client = Client::connect(&url).await;
        let id = manager.create_session(None,"general").await.unwrap();
        manager.prompt(&id,"Task","work").await.unwrap();model.request().await;
        client.until(|m|m["type"]=="session_state" && m["sessionId"]==id && m["status"]=="running").await;
        client.until(|m|m["type"]=="session_state" && m["sessionId"]==id && m["status"]=="idle").await;
        manager.prompt(&id,"/compact","compact").await.unwrap();
        let first = model.request().await; let retry = model.request().await;
        assert_eq!(first,retry);
        let receipt = client.until(|m|m["type"]=="receipts" && m["reports"][0]["id"]=="compact" && m["reports"][0]["complete"]==true).await;
        assert!(receipt["reports"][0]["error"].is_null(),"{receipt}");
        let entries = manager.inner.state.context(&id,&manager.inner.settings.get().agent.model).await.unwrap();
        assert_eq!(entries.iter().filter(|e|e["type"]=="compaction").count(),1);
        assert!(!manager.inner.state.queue(&id).await.unwrap().paused);
        assert!(model.requests.try_recv().is_err());
        manager.shutdown().await;server.abort();
    }
}
