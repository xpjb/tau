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
    assert_eq!(compaction_cut(&entries,0).unwrap(),1,"Neither an ambiguous result nor the first result completes the batch");
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
