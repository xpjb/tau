use super::*;
use serde_json::json;

#[test]
fn projects_flat_events_through_streaming_finalization_and_recovery() {
    let user = json!({"id":"u","type":"message","parentId":null,"origin":{"requestId":"prompt","requestRevision":2},"message":{"role":"user","content":"Check"}});
    let model = json!({"id":"model","parentId":"u","type":"model_change"});
    let raw = vec![user.clone(), model.clone(), json!({"id":"other","parentId":"u","type":"message","message":{"role":"user","content":"Other branch"}})];
    let mut source = PiPosition { session_id: "pi".into(), generation: "g".into(), sequence: 40 };
    let mut transcript = Transcript::new(&raw, &[], Some("model".into()), Some(source.clone()), QueueState::default(), None).unwrap();
    assert_eq!(transcript.snapshot(&[]).events.len(), 1);
    assert_eq!(transcript.snapshot(&[]).events[0].origin.request_revision, Some(2));
    assert!(!transcript.check_position(&source).unwrap());
    assert!(transcript.check_position(&PiPosition { sequence: 42, ..source.clone() }).is_err());
    assert!(transcript.check_position(&PiPosition { generation: "old".into(), ..source.clone() }).is_err());
    let live = json!({"streamId":"s","parentId":"model","message":{"role":"assistant","content":[]}});
    let operations = [
        json!({"type":"live","entry":live}),
        json!({"type":"delta","streamId":"s","event":{"assistantMessageEvent":{"type":"thinking_start","contentIndex":0}}}),
        json!({"type":"delta","streamId":"s","event":{"assistantMessageEvent":{"type":"thinking_delta","contentIndex":0,"delta":"Thinking π🧠"}}}),
        json!({"type":"delta","streamId":"s","event":{"assistantMessageEvent":{"type":"toolcall_start","contentIndex":1,"id":"call","toolName":"bash"}}}),
        json!({"type":"delta","streamId":"s","event":{"assistantMessageEvent":{"type":"toolcall_delta","contentIndex":1,"delta":"{}"}}}),
    ];
    for raw in operations {
        source.sequence += 1;
        assert!(transcript.check_position(&source).unwrap());
        let change = transcript.project(&raw).unwrap();
        assert!(!change.bumps_chat, "thinking and tool updates do not bump a chat");
        transcript.apply(&change, source.clone()).unwrap();
    }
    assert!(transcript.project(&json!({"type":"delta","streamId":"s","event":{"assistantMessageEvent":{"type":"text_start","contentIndex":3}}})).is_err());
    let cut = transcript.snapshot(&[]);
    assert_eq!(cut.events.iter().map(|event| event.id.as_str()).collect::<Vec<_>>(), ["request:prompt:0", "stream:s:0", "stream:s:1"]);
    assert_eq!(cut.events[1].text, "Thinking π🧠");
    assert_eq!(cut.events[2].kind, EventKind::Tool);
    assert_eq!(cut.events[2].tool_call_id.as_deref(), Some("call"));
    let saved = json!({"id":"a","parentId":"model","type":"message","origin":{"streamId":"s"},"message":{"role":"assistant","content":[
        {"type":"thinking","thinking":"Thinking π🧠","thinkingSignature":"private"},
        {"type":"toolCall","id":"call","name":"bash","arguments":{}},
        {"type":"image","mimeType":"image/png","data":"private-image"}
    ]}});
    let change = transcript.project(&json!({"type":"append","entry":saved,"leafId":"a"})).unwrap();
    source.sequence += 1; transcript.apply(&change, source.clone()).unwrap();
    assert_eq!(transcript.event("stream:s:0").unwrap().order, cut.events[1].order);
    assert_eq!(transcript.event("stream:s:0").unwrap().entry_id, "a");
    assert_eq!(transcript.event("stream:s:0").unwrap().phase, EventPhase::Saved);
    assert_eq!(cut.events[1].phase, EventPhase::Live);
    let encoded = serde_json::to_string(&transcript.snapshot(&[])).unwrap();
    assert!(!encoded.contains("private"));
    let tool = json!({"streamId":"tool","parentId":"a","message":{"role":"toolResult","toolCallId":"call","toolName":"bash","content":[{"type":"text","text":"Working"},{"type":"text","text":"Temporary"}]}});
    source.sequence += 1;
    transcript.apply(&transcript.project(&json!({"type":"live","entry":tool})).unwrap(), source.clone()).unwrap();
    let tool_order = transcript.event("stream:tool:0").unwrap().order;
    let interrupted = transcript.interrupt();
    assert!(!interrupted.queue.unwrap().available);
    assert_eq!(transcript.event("stream:tool:0").unwrap().phase, EventPhase::Interrupted);
    let mut recovered = Transcript::new(&[user.clone(), model.clone(), saved.clone()], &[], Some("a".into()), Some(source.clone()), QueueState::default(), Some(&transcript)).unwrap();
    assert_eq!(recovered.event("stream:tool:0").unwrap().order, tool_order);
    let final_tool = json!({"id":"result","parentId":"a","type":"message","origin":{"streamId":"tool"},"message":{"role":"toolResult","toolCallId":"call","toolName":"bash","content":[{"type":"text","text":"Done"}]}});
    let finalized = Transcript::new(&[user, model, saved, final_tool.clone()], &[], Some("result".into()), Some(source.clone()), QueueState::default(), Some(&recovered)).unwrap();
    assert!(finalized.event("stream:tool:1").is_none());
    let change = recovered.project(&json!({"type":"append","entry":final_tool,"leafId":"result"})).unwrap();
    source.sequence += 1; recovered.apply(&change, source).unwrap();
    assert_eq!(recovered.event("stream:tool:0").unwrap().order, tool_order);
    assert_eq!(recovered.event("stream:tool:0").unwrap().text, "Done");
    assert_eq!(recovered.event("stream:tool:0").unwrap().phase, EventPhase::Saved);
    assert!(recovered.event("stream:tool:1").is_none());
    assert_eq!(recovered.snapshot(&[]).events.len(), 5);
}

#[test]
fn pages_inside_messages_and_resolves_requests_outside_the_window() {
    let user = json!({"id":"u","parentId":null,"type":"message","origin":{"requestId":"request"},"message":{"role":"user","content":"Start"}});
    let blocks = (0..170).map(|index| json!({"type":"thinking","thinking":format!("Block {index} π🧠")})).collect::<Vec<_>>();
    let assistant = json!({"id":"a","parentId":"u","type":"message","origin":{"streamId":"s"},"message":{"role":"assistant","content":blocks}});
    let transcript = Transcript::new(&[user, assistant], &[], Some("a".into()), None, QueueState::default(), None).unwrap();
    let snapshot = transcript.snapshot(&["request".into()]);
    assert_eq!(snapshot.events.len(), PAGE_EVENTS);
    assert_eq!(snapshot.delivered, ["request"]);
    let mut events = snapshot.events;
    let mut before = snapshot.before;
    while let Some(cursor) = before {
        let page = transcript.page(Some(cursor));
        assert!(page.events.iter().all(|event| event.order < cursor));
        before = page.before;
        events.splice(0..0, page.events);
    }
    assert_eq!(events.len(), 171);
    assert!(events.windows(2).all(|pair| pair[0].order < pair[1].order));
    assert_eq!(events.last().unwrap().text, "Block 169 π🧠");
    let orphan = json!({"id":"a","parentId":"gone","type":"message","message":{"role":"assistant","content":"Kept"}});
    assert_eq!(Transcript::new(&[orphan], &[], Some("a".into()), None, QueueState::default(), None).unwrap().snapshot(&[]).events.len(), 1);
    let cycle = [json!({"id":"a","parentId":"b"}), json!({"id":"b","parentId":"a"})];
    assert!(Transcript::new(&cycle, &[], Some("a".into()), None, QueueState::default(), None).is_err());
    let duplicate = [json!({"id":"a"}), json!({"id":"a"})];
    assert!(Transcript::new(&duplicate, &[], Some("a".into()), None, QueueState::default(), None).is_err());
}

#[test]
fn preserves_identified_queue_controls_and_hides_binary_content() {
    let raw = json!({"queuedRequests":[{"requestId":"q","revision":3,"kind":"followUp","message":{"role":"user","timestamp":123,
        "content":[{"type":"text","text":"prepared"},{"type":"image","data":"private-image"}]}}],
        "runId":"run","paused":true,"control":{"commandId":"control","runId":"run","action":"prefix","boundary":"reasoning_checkpoint",
        "requests":[{"requestId":"q","revision":3}],"status":"waiting"},"capabilities":["queue_run_prefix"],"boundaries":["turn"]});
    let queue = QueueState::from_pi(&raw).unwrap();
    assert_eq!(queue.requests[0].text, "prepared");
    assert_eq!(queue.requests[0].images, 1);
    assert_eq!(queue.control.as_ref().unwrap().requests[0].revision, 3);
    assert!(!serde_json::to_string(&queue).unwrap().contains("private-image"));
    let mut duplicate = raw.clone();
    duplicate["queuedRequests"].as_array_mut().unwrap().push(raw["queuedRequests"][0].clone());
    assert!(QueueState::from_pi(&duplicate).is_err());
}

#[test]
fn recovery_restores_source_order_and_keeps_interrupted_work_between_its_neighbors() {
    let user = json!({"id":"u","type":"message","parentId":null,"message":{"role":"user","content":"Start"}});
    let thinking = json!({"id":"thinking","type":"message","parentId":"u","message":{"role":"assistant","content":[{"type":"thinking","thinking":"Earlier thinking"}]}});
    let closing = json!({"id":"closing","type":"message","parentId":"thinking","message":{"role":"assistant","content":"Closing answer"}});
    let mut old = Transcript::new(&[user.clone(), closing.clone()], &[], Some("closing".into()), None, QueueState::default(), None).unwrap();
    let abandoned = json!({"streamId":"lost","parentId":"closing","message":{"role":"assistant","content":[{"type":"thinking","thinking":"Interrupted thinking"}]}});
    let position = PiPosition { session_id: "pi".into(), generation: "g".into(), sequence: 1 };
    old.apply(&old.project(&json!({"type":"live","entry":abandoned})).unwrap(), position).unwrap();
    old.interrupt();
    let next_user = json!({"id":"next","type":"message","parentId":"closing","message":{"role":"user","content":"Continue"}});
    let current = json!({"streamId":"current","parentId":"next","message":{"role":"assistant","content":[{"type":"thinking","thinking":"Current thinking"}]}});
    let raw = [closing, thinking, user.clone(), next_user];
    let mut restored = Transcript::new(&raw, std::slice::from_ref(&current), Some("next".into()), None, QueueState::default(), Some(&old)).unwrap();
    for _ in 0..3 {
        let cut = restored.snapshot(&[]);
        assert_eq!(cut.events.iter().map(|event| event.text.as_str()).collect::<Vec<_>>(),
            ["Start", "Earlier thinking", "Closing answer", "Interrupted thinking", "Continue", "Current thinking"]);
        assert!(cut.events.windows(2).all(|pair| pair[0].order < pair[1].order));
        assert_eq!(restored.event("stream:lost:0").unwrap().phase, EventPhase::Interrupted);
        assert_eq!(restored.event("entry:closing:0").unwrap().id, old.event("entry:closing:0").unwrap().id);
        assert_ne!(restored.generation, old.generation);
        restored = Transcript::new(&raw, std::slice::from_ref(&current), Some("next".into()), None, QueueState::default(), Some(&restored)).unwrap();
    }
    let alternate = json!({"id":"alternate","type":"message","parentId":"u","message":{"role":"user","content":"Other branch"}});
    let switched = Transcript::new(&[user, alternate], &[], Some("alternate".into()), None, QueueState::default(), Some(&restored)).unwrap();
    assert!(switched.event("stream:lost:0").is_none());
    assert_eq!(switched.snapshot(&[]).events.len(), 2);
}

#[test]
fn snapshot_places_live_content_after_its_parent_and_before_later_saved_messages() {
    let user = json!({"id":"u","type":"message","parentId":null,"message":{"role":"user","content":"Start"}});
    let notice = json!({"id":"notice","type":"custom_message","parentId":"u","display":true,"content":"Later notice"});
    let live = json!({"streamId":"s","parentId":"u","message":{"role":"assistant","content":[{"type":"thinking","thinking":"Earlier thinking"}]}});
    let transcript = Transcript::new(&[user, notice], &[live], Some("notice".into()), None, QueueState::default(), None).unwrap();
    assert_eq!(transcript.snapshot(&[]).events.iter().map(|event| event.text.as_str()).collect::<Vec<_>>(), ["Start", "Earlier thinking", "Later notice"]);

    let model = json!({"id":"model","parentId":"missing","type":"model_change"});
    let lost = json!({"streamId":"lost","parentId":"model","message":{"role":"assistant","content":[{"type":"thinking","thinking":"Interrupted"}]}});
    let old = Transcript::new(std::slice::from_ref(&model), &[lost], Some("model".into()), None, QueueState::default(), None).unwrap();
    let missing = json!({"id":"missing","type":"message","parentId":null,"message":{"role":"user","content":"Recovered prefix"}});
    let next = json!({"id":"next","type":"message","parentId":"model","message":{"role":"user","content":"Next prompt"}});
    let restored = Transcript::new(&[missing, model, next], &[], Some("next".into()), None, QueueState::default(), Some(&old)).unwrap();
    assert_eq!(restored.snapshot(&[]).events.iter().map(|event| event.text.as_str()).collect::<Vec<_>>(), ["Recovered prefix", "Interrupted", "Next prompt"]);
}
