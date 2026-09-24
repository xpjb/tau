//! Characterization of protocol-15 costs, not desired protocol guarantees.
//! Replace these expectations with bounded/delta-sync guarantees in the redesign.
use crate::manager::SessionContent;
use crate::transcript::{HistoryPage, QueueState, Transcript, PAGE_BYTES};
use serde_json::json;

fn content() -> SessionContent {
    let mut transcript = Transcript::new(HistoryPage { events:vec![], before:None }, None, 0, QueueState::native());
    transcript.generation = "audit-generation".into();
    SessionContent { transcript:Some(transcript), ..Default::default() }
}

#[tokio::test]
async fn audit_streaming_wire_costs_distinguish_text_deltas_from_growing_tools() {
    async fn measure(kind: &str) -> usize {
        let mut content = content();
        let mut messages = content.events.subscribe();
        let mut bytes = 0;
        for n in 1..=128 {
            let text = "x".repeat(n * 256);
            let block = match kind {
                "text" => json!({"type":"text","text":text}),
                "thinking" => json!({"type":"thinking","thinking":text}),
                "tool" => json!({"type":"toolCall","id":"call","name":"write","partialArguments":text}),
                _ => unreachable!(),
            };
            content.live("chat", "stream", json!({"role":"assistant","content":[block]})).await.unwrap();
            let message = messages.try_recv().unwrap();
            bytes += serde_json::to_vec(message.as_ref()).unwrap().len();
        }
        bytes
    }
    let text = measure("text").await;
    let thinking = measure("thinking").await;
    let tool = measure("tool").await;
    println!("128 updates ending at 32768 content bytes: text={text}, thinking={thinking}, tool={tool} JSON bytes");
    assert!(text < 70_000 && thinking < 70_000, "Single text/thinking blocks already use deltas");
    assert!(tool < 70_000, "Tool input now uses append deltas too");
}

#[tokio::test]
async fn audit_page_budget_is_not_a_hard_frame_limit() {
    let mut content = content();
    content.live("chat", "stream", json!({"role":"assistant","content":[
        {"type":"toolCall","id":"call","name":"write","partialArguments":"x".repeat(PAGE_BYTES * 2)}
    ]})).await.unwrap();
    let transcript = content.transcript.as_ref().unwrap();
    let page = transcript.page(None);
    assert_eq!(page.events.len(),1);
    let snapshot_bytes = serde_json::to_vec(&transcript.snapshot()).unwrap().len();
    println!("Nominal page budget={PAGE_BYTES}; one-event snapshot={snapshot_bytes} JSON bytes");
    assert!(snapshot_bytes > PAGE_BYTES * 2, "The first event can exceed the soft page budget");
}
