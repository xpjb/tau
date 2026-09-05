use super::*;
use serde_json::json;

#[test]
fn retains_identified_content_across_updates_recovery_and_branches() {
    let raw = [
        json!({"type":"message","id":"u","parentId":null,"origin":{"requestId":"prompt","requestRevision":2},"message":{"role":"user","content":"same"}}),
        json!({"type":"model_change","id":"model","parentId":"u","message":{"content":"provider-only"}}),
        json!({"type":"message","id":"old","parentId":"model","message":{"role":"user","content":"other branch"}}),
    ];
    let entries = raw.iter().map(|entry| Entry::from_pi(entry, false).unwrap()).collect::<Vec<_>>();
    let source = PiPosition { session_id: "pi".to_owned(), generation: "source".to_owned(), sequence: 40 };
    let mut transcript = Transcript::new(entries, Some("model".to_owned()), Some(source.clone()), QueueState::default()).unwrap();
    assert_eq!(transcript.snapshot().entries.len(), 3);
    assert_eq!(transcript.snapshot().entries[0].origin.request_revision, Some(2));
    assert!(transcript.snapshot().entries[1].content.is_empty());
    assert!(!transcript.check_position(&source).unwrap());
    assert!(transcript.check_position(&PiPosition { sequence: 42, ..source.clone() }).is_err());
    assert!(transcript.check_position(&PiPosition { session_id: "other".to_owned(), ..source.clone() }).is_err());
    assert!(transcript.check_position(&PiPosition { generation: "other".to_owned(), ..source.clone() }).is_err());
    let entry = Entry::from_pi(&json!({"streamId":"stream","parentId":"model","message":{"role":"assistant","content":[
        {"type":"thinking","thinking":"visible","thinkingSignature":"private-signature"},
        {"type":"image","mimeType":"image/png","data":"private-binary"}
    ]}}), true).unwrap();
    transcript.apply(&TranscriptChange::Entry { entry: Box::new(entry) }, Some(PiPosition { sequence: 41, ..source.clone() })).unwrap();
    let cut = transcript.snapshot();
    transcript.apply(&TranscriptChange::Delta { entry_id: "live-stream".to_owned(), index: 0, delta: " retained".to_owned() },
        Some(PiPosition { sequence: 42, ..source.clone() })).unwrap();
    assert_eq!(cut.entries.last().unwrap().content[0].text, "visible");
    assert_eq!(transcript.snapshot().entries.last().unwrap().content[0].text, "visible retained");
    let encoded = serde_json::to_string(&transcript.snapshot()).unwrap();
    assert!(!encoded.contains("private-signature") && !encoded.contains("private-binary"));
    transcript.apply(&TranscriptChange::Interrupted, None).unwrap();
    assert!(transcript.apply(&TranscriptChange::Delta { entry_id: "live-stream".to_owned(), index: 0, delta: "late".to_owned() }, None).is_err());
    let mut next = Transcript::new(Vec::new(), None, None, QueueState::default()).unwrap();
    next.retain_interrupted(&transcript);
    assert_eq!(next.snapshot().entries[0].phase, EntryPhase::Interrupted);
    assert_eq!(next.snapshot().entries[0].parent_id, None);
    let saved = Entry::from_pi(&json!({"type":"message","id":"a","parentId":null,"origin":{"streamId":"stream"},
        "message":{"role":"assistant","content":[{"type":"thinking","thinking":"visible retained"}]}}), false).unwrap();
    next.apply(&TranscriptChange::Entry { entry: Box::new(saved) }, None).unwrap();
    assert_eq!(next.snapshot().entries.len(), 1);
    assert_eq!(next.snapshot().entries[0].id, "a");
    next.retain_interrupted(&transcript);
    assert_eq!(next.snapshot().entries.len(), 1);
    assert!(next.apply(&TranscriptChange::Head { head: Some("missing".to_owned()) }, None).is_err());
    assert_eq!(next.snapshot().head.as_deref(), Some("a"));
    for entries in [
        vec![json!({"type":"message","id":"a","parentId":"missing"})],
        vec![json!({"type":"message","id":"a","parentId":"b"}), json!({"type":"message","id":"b","parentId":"a"})],
        vec![json!({"type":"message","id":"a","parentId":null}), json!({"type":"message","id":"a","parentId":null})],
    ] {
        assert!(Transcript::new(entries.iter().map(|entry| Entry::from_pi(entry, false).unwrap()).collect(), None, None, QueueState::default()).is_err());
    }
    assert!(TranscriptChange::from_pi(&json!({"type":"append","leafId":"wrong","entry":{"id":"a","type":"message"}})).is_err());
    assert!(PiPosition::from_pi(&json!({"sessionId":"","generation":"g","sequence":0})).is_err());
}

#[test]
fn projects_prepared_queue_revisions_controls_and_partial_tool_output() {
    let raw = json!({
        "queuedRequests":[{"requestId":"q","revision":3,"kind":"followUp","message":{"role":"user","timestamp":123,
            "content":[{"type":"text","text":"prepared"},{"type":"image","data":"private-image","mimeType":"image/png"}]}}],
        "runId":"run", "paused":true,
        "control":{"commandId":"control","runId":"run","action":"prefix","boundary":"reasoning_checkpoint",
            "requests":[{"requestId":"q","revision":3}],"status":"waiting"},
        "capabilities":["queue_delete","queue_run_prefix"],"boundaries":["reasoning_checkpoint","turn"]
    });
    let queue = QueueState::from_pi(&raw).unwrap();
    assert_eq!(queue.requests[0].text, "prepared");
    assert_eq!(queue.requests[0].images, 1);
    assert_eq!(queue.requests[0].revision, 3);
    assert_eq!(queue.control.as_ref().unwrap().requests[0].revision, 3);
    assert!(!serde_json::to_string(&queue).unwrap().contains("private-image"));
    let mut duplicate = raw.clone();
    duplicate["queuedRequests"].as_array_mut().unwrap().push(raw["queuedRequests"][0].clone());
    assert!(QueueState::from_pi(&duplicate).is_err());
    let mut transcript = Transcript::new(Vec::new(), None, None, queue).unwrap();
    let live = Entry::from_pi(&json!({"streamId":"tool","parentId":null,"message":{"role":"toolResult","toolCallId":"call",
        "toolName":"bash","content":[{"type":"text","text":"partial output"}],"isError":false}}), true).unwrap();
    transcript.apply(&TranscriptChange::Entry { entry: Box::new(live) }, None).unwrap();
    assert_eq!(transcript.snapshot().entries[0].content[0].text, "partial output");
    let content = Content::from_pi(&json!({"type":"toolCall","id":"call","name":"bash","partialArguments":"{\"command\":\"ca"}));
    assert_eq!(content.text, "{\"command\":\"ca");
    transcript.apply(&TranscriptChange::Interrupted, None).unwrap();
    assert!(!transcript.queue.available);
    assert_eq!(transcript.queue.requests.len(), 1);
    assert_eq!(transcript.queue.control.unwrap().status, "waiting");
}
