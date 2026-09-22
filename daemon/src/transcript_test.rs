use super::*;
use serde_json::json;

#[test]
fn native_updates_preserve_block_ids_order_deltas_finalization_and_private_payloads() {
    let user = json!({"id":"u","type":"message","parentId":null,"origin":{"requestId":"prompt","requestRevision":2},"message":{"role":"user","content":"Check"}});
    let mut transcript = Transcript::new(&[user], Some("u".into()), QueueState::default()).unwrap();
    let stream = json!({"streamId":"s","parentId":"u","message":{"role":"assistant","content":[
        {"type":"thinking","thinking":"Thinking π🧠"}, {"type":"toolCall","id":"call","name":"bash","arguments":{}}
    ]}});
    let change = transcript.project(&stream, true).unwrap();
    assert!(!change.bumps_chat);
    transcript.apply(&change).unwrap();
    let delta = TranscriptChange { delta:Some(TextDelta { event_id:"stream:s:0".into(), text:" More".into() }), ..Default::default() };
    transcript.apply(&delta).unwrap();
    assert_eq!(transcript.event("stream:s:0").unwrap().text, "Thinking π🧠 More");
    let cut = transcript.snapshot(&[]);
    assert_eq!(cut.events.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["request:prompt:0","stream:s:0","stream:s:1"]);
    let saved = json!({"id":"a","parentId":"u","type":"message","origin":{"streamId":"s"},"message":{"role":"assistant","content":[
        {"type":"thinking","thinking":"Thinking π🧠 More","thinkingSignature":"private-signature"},
        {"type":"toolCall","id":"call","name":"bash","arguments":{"command":"true"}},
        {"type":"image","mimeType":"image/png","data":"private-image"}
    ],"tauModelMessage":{"codex_output":[{"encrypted_content":"private-checkpoint"}]}}});
    transcript.apply(&transcript.project(&saved, false).unwrap()).unwrap();
    assert_eq!(transcript.event("stream:s:0").unwrap().order, cut.events[1].order);
    assert_eq!(transcript.event("stream:s:0").unwrap().entry_id, "a");
    assert_eq!(transcript.event("stream:s:0").unwrap().phase, EventPhase::Saved);
    assert!(transcript.apply(&delta).is_err(), "A finished block cannot accept a live delta");
    assert!(!serde_json::to_string(&transcript.snapshot(&[])).unwrap().contains("private"));
    let live_tool = json!({"streamId":"tool","message":{"role":"toolResult","toolCallId":"call","toolName":"bash","content":[{"type":"text","text":"Working"},{"type":"text","text":"Temporary"}]}});
    transcript.apply(&transcript.project(&live_tool, true).unwrap()).unwrap();
    let tool_order = transcript.event("stream:tool:0").unwrap().order;
    let final_tool = json!({"id":"result","parentId":"a","type":"message","origin":{"streamId":"tool"},"message":{"role":"toolResult","toolCallId":"call","toolName":"bash","content":[{"type":"text","text":"Done"}]}});
    transcript.apply(&transcript.project(&final_tool, false).unwrap()).unwrap();
    assert!(transcript.event("stream:tool:1").is_none());
    assert_eq!(transcript.event("stream:tool:0").unwrap().order, tool_order);
    assert_eq!(transcript.snapshot(&[]).events.len(), 5);
    assert_eq!(transcript.snapshot(&["prompt".into()]).delivered, ["prompt"]);
}

#[test]
fn pages_inside_messages_and_validates_active_legacy_branches() {
    let user = json!({"id":"u","parentId":null,"type":"message","origin":{"requestId":"request"},"message":{"role":"user","content":"Start"}});
    let blocks = (0..170).map(|index| json!({"type":"thinking","thinking":format!("Block {index} π🧠")})).collect::<Vec<_>>();
    let assistant = json!({"id":"a","parentId":"u","type":"message","origin":{"streamId":"s"},"message":{"role":"assistant","content":blocks}});
    let other = json!({"id":"other","parentId":"u","type":"message","message":{"role":"user","content":"Abandoned branch"}});
    let transcript = Transcript::new(&[user, assistant, other], Some("a".into()), QueueState::default()).unwrap();
    let snapshot = transcript.snapshot(&["request".into()]);
    assert_eq!(snapshot.events.len(), PAGE_EVENTS); assert_eq!(snapshot.delivered, ["request"]);
    let mut events = snapshot.events; let mut before = snapshot.before;
    while let Some(cursor) = before {
        let page = transcript.page(Some(cursor)); assert!(page.events.iter().all(|e| e.order < cursor));
        before = page.before; events.splice(0..0, page.events);
    }
    assert_eq!(events.len(), 171);
    assert!(events.windows(2).all(|pair| pair[0].order < pair[1].order));
    assert_eq!(events.last().unwrap().text, "Block 169 π🧠");
    let orphan = json!({"id":"a","parentId":"gone","type":"message","message":{"role":"assistant","content":"Kept"}});
    assert_eq!(Transcript::new(&[orphan], Some("a".into()), QueueState::default()).unwrap().snapshot(&[]).events.len(), 1);
    let cycle = [json!({"id":"a","parentId":"b"}),json!({"id":"b","parentId":"a"})];
    assert!(Transcript::new(&cycle, Some("a".into()), QueueState::default()).is_err());
    assert!(Transcript::new(&[json!({"id":"a"}),json!({"id":"a"})], Some("a".into()), QueueState::default()).is_err());
}
