use super::*;
#[tokio::test]
async fn dangling_private_calls_receive_unknown_outcomes_not_reexecution() {
    let native=json!({"role":"assistant","codex_output":[{"type":"reasoning","encrypted_content":"opaque"},{"type":"function_call","call_id":"call|item","name":"bash","arguments":"{}"}]});
    let entries=vec![json!({"type":"message","message":{"role":"assistant","stopReason":"toolUse","tauModelMessage":native}})];
    let model=SessionModel {provider:"p".into(),model_id:"m".into()};
    let result=messages(&entries,"system".into(),&model,Path::new("/unused")).await.unwrap();
    assert_eq!(result[1]["codex_output"][0]["encrypted_content"],"opaque");assert_eq!(result[2]["tool_call_id"],"call|item");
    assert!(result[2]["content"].as_str().unwrap().contains("unknown"));
    let mut completed=entries;completed.push(json!({"type":"message","message":{"role":"toolResult","toolCallId":"call|item","content":"done"}}));
    let result=messages(&completed,"system".into(),&model,Path::new("/unused")).await.unwrap();assert_eq!(result.len(),3);assert_eq!(result[2]["content"],"done");
}
