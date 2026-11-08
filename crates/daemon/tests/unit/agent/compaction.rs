//! Compaction must make progress during one uninterrupted task, not require a new user turn.
use super::*;

#[cfg(unix)]
#[tokio::test]
async fn one_prompt_compacts_repeatedly_without_pausing_or_replaying_tools() {
    for native in [true, false] {
        let tool = |n, tokens| codex_with_tokens("",vec![json!({"type":"function_call","call_id":format!("call-{n}"),
            "name":"bash","arguments":json!({"command":format!("printf '{n}' >> effects; printf 'result-{n}'")}).to_string()})],tokens);
        let checkpoint = |n| if native {codex("",vec![json!({"type":"compaction","encrypted_content":format!("checkpoint-{n}")})])}
            else {codex(&format!("Task summary {n}; previous tools completed, do not repeat them"),vec![])};
        let mut model = ModelServer::with_catalog(vec![tool(1,120),tool(2,15_000),checkpoint(1),tool(3,15_000),checkpoint(2),codex("Task finished",vec![])],
            Some(json!({"models":[{"slug":"gpt-6-astra","context_window":16_384}]}))).await;
        let (root,manager,url,server) = fixture(&model,Api::Codex).await;
        let mut settings = manager.inner.settings.get();
        settings.agent.compaction.reserve_tokens = 2048;
        settings.agent.compaction.keep_recent_tokens = 64;
        settings.agent.compaction.native_codex = native;
        manager.set_settings(settings.revision,settings).await.unwrap();
        let id = manager.create_session(None,"general").await.unwrap();
        let mut client = Client::connect(&url).await;
        tokio::time::timeout(Duration::from_secs(5),async {
            while manager.context_window(&manager.inner.settings.get(),&manager.inner.settings.get().agent.model)!=Some(16_384) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }).await.unwrap();
        manager.prompt(&id,&"Complete this long task without asking me to continue. ".repeat(10),"only-prompt").await.unwrap();
        client.until(|m| m["type"]=="session_state" && m["sessionId"]==id && m["status"]=="running").await;
        let settled = client.until(|m| m["type"]=="session_state" && m["sessionId"]==id && matches!(m["status"].as_str(),Some("idle"|"error"))).await;
        assert_eq!(settled["status"],"idle","A long single task must compact instead of requiring another user message: {settled}");
        assert!(!manager.inner.state.queue(&id).await.unwrap().paused,"{settled}");
        assert_eq!(tokio::fs::read_to_string(root.path().join("effects")).await.unwrap(),"123","Compaction must not reexecute side effects: {settled}");
        let requests = std::iter::from_fn(||model.requests.try_recv().ok()).collect::<Vec<_>>();
        assert_eq!(requests.len(),6,"Two checkpoints and four chat responses, with no retries");
        for (checkpoint_index,next_index,old_call,kept_call) in [(2,3,"call-1","call-2"),(4,5,"call-2","call-3")] {
            let compact = requests[checkpoint_index]["input"].to_string();
            assert!(compact.contains(old_call)); assert!(!compact.contains(kept_call));
            let resumed = requests[next_index]["input"].as_array().unwrap();
            assert_eq!(resumed.iter().filter(|v|v["type"]=="function_call" && v["call_id"]==kept_call).count(),1);
            assert_eq!(resumed.iter().filter(|v|v["type"]=="function_call_output" && v["call_id"]==kept_call).count(),1);
            assert!(!resumed.iter().any(|v|v["call_id"]==old_call));
        }
        if native { assert!(requests[4]["input"].to_string().contains("checkpoint-1")); }
        // The non-user retained boundary must also work after the runtime is unloaded.
        manager.close_session(&id).await.unwrap();
        let selected = manager.inner.settings.get().agent.model.clone();
        let entries = manager.inner.state.context(&id,&selected).await.unwrap();
        let replay = crate::agent::history::messages(&entries,String::new(),&selected,&root.path().join("outbox")).await.unwrap();
        assert!(replay.iter().any(|m|m["tool_call_id"]=="call-3"));
        assert!(!replay.iter().any(|m|matches!(m["tool_call_id"].as_str(),Some("call-1"|"call-2"))));
        assert!(!replay.iter().any(|m|m["content"].as_str().is_some_and(|s|s.contains("effects are unknown"))));
        manager.shutdown().await; server.abort();
    }
}

#[test]
fn compaction_keeps_parallel_calls_and_results_together_and_prefers_user_boundaries() {
    use crate::agent::history::compaction_cut;
    let user = json!({"type":"message","message":{"role":"user","content":"Task"}});
    let next = json!({"type":"message","message":{"role":"assistant","content":"Next exchange","stopReason":"stop"}});
    let mut entries = vec![user.clone(),json!({"type":"message","message":{"role":"assistant","stopReason":"toolUse",
        "tauModelMessage":{"tool_calls":[{"id":"call|one"},{"id":"call|two"}]}}}),
        json!({"type":"message","message":{"role":"toolResult","toolCallId":"call","content":"Ambiguous legacy result"}}),
        user.clone(),
        json!({"type":"message","message":{"role":"toolResult","toolCallId":"call|one","content":"First result"}}),
        next.clone()];
    assert_eq!(compaction_cut(&entries[..5],0).unwrap(),1,"Never split a stored result from its call");
    assert_eq!(compaction_cut(&entries,0).unwrap(),5,"The missing second result becomes an unknown outcome, not a permanent blocker");
    entries.push(json!({"type":"message","message":{"role":"toolResult","toolCallId":"call|two","content":"Second result"}}));
    entries.push(next.clone());
    assert_eq!(compaction_cut(&entries,0).unwrap(),7,"Keep the whole newest exchange even when it exceeds the retention target");
    entries.extend([user.clone(),next]);
    assert_eq!(compaction_cut(&entries,u64::MAX).unwrap(),8,"Prefer a complete user turn when it fits");
    assert!(compaction_cut(&[user],64).is_err(),"A lone oversized input cannot be made smaller by inventing history");
    assert!(compaction_cut(&[],64).is_err());
    let legacy = vec![json!({"type":"message","message":{"role":"user","content":"Task"}}),
        json!({"type":"message","message":{"role":"assistant","content":[{"type":"toolCall","id":"legacy|item"}]}}),
        json!({"type":"message","message":{"role":"toolResult","toolCallId":"legacy","content":"Done"}}),
        json!({"type":"message","message":{"role":"user","content":"Next"}})];
    assert_eq!(compaction_cut(&legacy,0).unwrap(),3,"Unambiguous imported call aliases remain supported");
}

#[test]
fn compaction_does_not_wait_forever_for_interrupted_calls_without_results() {
    use crate::agent::history::compaction_cut;
    let calls = |ids: &[&str]| json!({"type":"message","message":{"role":"assistant","stopReason":"toolUse",
        "tauModelMessage":{"tool_calls":ids.iter().map(|id| json!({"id":id})).collect::<Vec<_>>()}}});
    let result = |id| json!({"type":"message","message":{"role":"toolResult","toolCallId":id,"content":"Done"}});
    let user = json!({"type":"message","message":{"role":"user","content":"Continue"}});
    let entries = vec![calls(&["orphan-1","orphan-2"]), user.clone(), calls(&["done-1","done-2"]),
        result("done-1"), user.clone(), result("done-2"), user.clone()];
    assert_eq!(compaction_cut(&entries,0).unwrap(),6);
    assert_eq!(compaction_cut(&entries[..6],0).unwrap(),2,"Actual parallel results still cannot be separated from their calls");
    // Resolve by occurrence, not by a global set of orphan IDs.
    let reused = vec![calls(&["reused"]),user.clone(),result("reused"),calls(&["reused"]),user];
    assert_eq!(compaction_cut(&reused,0).unwrap(),4);
    assert!(compaction_cut(&reused[..3],0).is_err(),"The earlier real call/result pair stays together");
}

#[tokio::test]
async fn interrupted_prefix_compacts_on_send_and_in_a_fork_without_replaying_tools() {
    for native in [true, false] {
        let checkpoint = || if native {codex("",vec![json!({"type":"compaction","encrypted_content":"new-checkpoint"})])}
            else {codex("Summary: interrupted calls have unknown effects; completed tools must not be repeated.",vec![])};
        let mut model = ModelServer::with_catalog(vec![checkpoint(),codex("Continued original",vec![]),checkpoint(),codex("Continued fork",vec![])],
            Some(json!({"models":[{"slug":"gpt-6-astra","context_window":16_384}]}))).await;
        let (root,manager,url,server) = fixture(&model,Api::Codex).await;
        let mut settings = manager.inner.settings.get();
        settings.agent.compaction.native_codex = native;
        settings.agent.compaction.reserve_tokens = 2048;
        settings.agent.compaction.keep_recent_tokens = 64;
        manager.set_settings(settings.revision,settings).await.unwrap();
        let id = manager.create_session(None,"general").await.unwrap();
        let runtime = manager.runtime(&id).await.unwrap();
        let mut content = runtime.content.lock().await;
        let call = |id| json!({"id":id,"type":"function","function":{"name":"bash","arguments":"{\"command\":\"touch must-not-replay\"}"}});
        content.append(&id,json!({"type":"message","message":{"role":"assistant","stopReason":"toolUse",
            "tauModelMessage":{"role":"assistant","tool_calls":[call("orphan-1"),call("orphan-2")]}}})).await.unwrap();
        let boundary = content.transcript.as_ref().unwrap().head.clone();
        for entry in [
            json!({"type":"message","message":{"role":"user","content":"Continue the task"}}),
            json!({"type":"message","message":{"role":"assistant","stopReason":"toolUse",
                "tauModelMessage":{"role":"assistant","tool_calls":[call("completed")]}}}),
            json!({"type":"message","message":{"role":"toolResult","toolCallId":"completed","content":"Already completed. ".repeat(100)}}),
            json!({"type":"compaction","summary":"Earlier task context","firstKeptEntryId":boundary}),
            json!({"type":"message","message":{"role":"assistant","stopReason":"stop","content":"Finished earlier work",
                "tauModelMessage":{"role":"assistant","content":"Finished earlier work","usage":{"total_tokens":15_000}}}}),
        ] { content.append(&id,entry).await.unwrap(); }
        drop(content);
        let fork = manager.clone_session(&id).await.unwrap();
        manager.close_session(&id).await.unwrap();
        tokio::time::timeout(Duration::from_secs(5),async {
            while manager.context_window(&manager.inner.settings.get(),&manager.inner.settings.get().agent.model)!=Some(16_384) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }).await.unwrap();
        // A new message must recover through automatic compaction after reload.
        let mut client = Client::connect(&url).await;
        manager.prompt(&id,"Continue with new work","new-work").await.unwrap();
        client.until(|m| m["type"]=="session_state" && m["sessionId"]==id && m["status"]=="running").await;
        let compact = model.request().await;
        let input = compact["input"].to_string();
        assert!(input.contains("orphan-1") && input.contains("orphan-2"));
        assert!(input.contains("effects are unknown"));
        assert!(input.contains("Earlier task context"));
        let continued = model.request().await;
        assert!(!continued["input"].to_string().contains("orphan-1"));
        client.until(|m| m["type"]=="session_state" && m["sessionId"]==id && m["status"]=="idle").await;
        assert!(!manager.inner.state.queue(&id).await.unwrap().paused);
        // The same poisoned prefix in an existing fork must also compact manually.
        manager.prompt(&fork,"/compact","compact-fork").await.unwrap();
        assert!(model.request().await["input"].to_string().contains("effects are unknown"));
        let receipt = client.until(|m| m["type"]=="receipts" && m["reports"][0]["id"]=="compact-fork" && m["reports"][0]["complete"]==true).await;
        assert!(receipt["reports"][0]["error"].is_null(),"{receipt}");
        client.until(|m| m["type"]=="session_state" && m["sessionId"]==fork && m["status"]=="idle").await;
        manager.close_session(&fork).await.unwrap();
        manager.prompt(&fork,"Continue the fork","fork-work").await.unwrap();
        assert!(!model.request().await["input"].to_string().contains("orphan-1"));
        client.until(|m| m["type"]=="session_state" && m["sessionId"]==fork && m["status"]=="idle").await;
        assert!(!manager.inner.state.queue(&fork).await.unwrap().paused);
        assert!(!root.path().join("must-not-replay").exists());
        assert!(model.requests.try_recv().is_err());
        manager.shutdown().await; server.abort();
    }
}
