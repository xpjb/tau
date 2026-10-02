use super::*;
use serde_json::json;

#[test]
fn saved_codex_summary_parts_regain_boundaries_without_guessing_or_exposing_hidden_reasoning() {
    let mut entry = json!({
        "type": "message", "id": "saved", "message": {
            "role": "assistant",
            "content": [{"type": "thinking", "thinking": "CheckCompare"}],
            "tauModelMessage": {"codex_output": [
                {"type": "reasoning", "encrypted_content": "private-cipher",
                    "summary": [{"type": "summary_text", "text": "Check"},
                                {"type": "summary_text", "text": "Compare"}]}
            ]}
        }
    });
    let saved = <Event as EventProjection>::from_entry(&entry, false).unwrap();
    assert_eq!(saved[0].text, "Check\n\nCompare");
    assert!(!saved[0].text.contains("private-cipher"));
    assert_eq!(
        entry["message"]["content"][0]["thinking"], "CheckCompare",
        "stored history is not changed"
    );

    entry["message"]["content"][0]["thinking"] = json!("Check\n\nCompare");
    assert_eq!(
        <Event as EventProjection>::from_entry(&entry, false).unwrap()[0].text,
        "Check\n\nCompare"
    );
    entry["message"]["content"][0]["thinking"] = json!("Check and Compare");
    assert_eq!(
        <Event as EventProjection>::from_entry(&entry, false).unwrap()[0].text,
        "Check and Compare"
    );

    entry["streamId"] = json!("stream");
    entry["message"]["content"][0]["thinking"] = json!("CheckCompare");
    assert_eq!(
        <Event as EventProjection>::from_entry(&entry, true).unwrap()[0].text,
        "CheckCompare",
        "a live prefix has no inferred section boundary"
    );
}

#[test]
fn transient_projection_keeps_live_events_and_bounds_saved_count_and_bytes() {
    fn event(order: u64, text: &str) -> Event {
        let mut event = Event::from_entry(&json!({"id":format!("e{order}"),
            "message":{"role":"assistant","content":text}}),false).unwrap().remove(0);
        event.order=order; event
    }
    let mut transcript=Transcript::new("lineage:chat".into(),None,0,QueueState::native());
    let mut events=(0..80).map(|order|event(order,"saved")).collect::<Vec<_>>();
    events[0].phase=EventPhase::Live;
    let live_id=events[0].id.clone();let evicted_id=events[1].id.clone();
    transcript.apply(&TranscriptChange {events,..Default::default()}).unwrap();
    assert_eq!(transcript.events.len(),PAGE_EVENTS+1);
    assert_eq!(transcript.by_id.len(),transcript.events.len());
    assert!(transcript.event(&live_id).is_some());assert!(transcript.event(&evicted_id).is_none());
    assert_eq!(transcript.events.range(1..).next().unwrap().0,&30);

    // Always retain the newest event even when it alone exceeds the byte limit.
    let newest=event(80,&"x".repeat(PAGE_BYTES+1));let newest_id=newest.id.clone();
    transcript.apply(&TranscriptChange {events:vec![newest],..Default::default()}).unwrap();
    assert_eq!(transcript.events.len(),2);assert!(transcript.event(&newest_id).is_some());
    let mut finished=transcript.event(&live_id).unwrap().clone();finished.phase=EventPhase::Saved;
    transcript.apply(&TranscriptChange {events:vec![finished],..Default::default()}).unwrap();
    assert_eq!(transcript.events.len(),1);assert_eq!(transcript.by_id.len(),1);
    assert!(transcript.event(&live_id).is_none());assert!(transcript.event(&newest_id).is_some());
}
