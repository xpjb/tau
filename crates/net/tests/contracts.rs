use serde_json::json;
use tau_net::{QueueOperation, QueueState};

#[test]
fn old_queue_metadata_loads_without_retired_capability_negotiation() {
    let queue: QueueState=serde_json::from_value(json!({
        "available":true,"paused":false,"runId":"run",
        "capabilities":["queue_edit","queue_delete","queue_run_prefix"],"boundaries":["turn"],
        "requests":[{"requestId":"request","revision":3,"kind":"steer","text":"keep this message","images":0,"timestampMs":42}],
        "control":{"commandId":"pause","runId":"run","action":"pause","boundary":"turn","requests":[],"status":"waiting","detail":null}
    })).unwrap();
    assert_eq!(queue.requests[0].text,"keep this message");
    assert_eq!(queue.requests[0].revision,3);
    assert_eq!(queue.control.as_ref().unwrap().command_id,"pause");
    let saved=serde_json::to_value(queue).unwrap();
    for field in ["capabilities","boundaries"] {assert!(saved.get(field).is_none());}
    for field in ["kind","images"] {assert!(saved["requests"][0].get(field).is_none());}
    assert!(saved["control"].get("boundary").is_none());
    assert!(!QueueState::default().available,"Missing queue metadata is still unavailable, not an empty authoritative queue");
}

#[test]
fn queue_action_receipt_payloads_keep_their_original_shape() {
    // Metadata cleanup must not change the fingerprint of an already-saved intent.
    for value in [json!({"type":"pause","runId":"run","boundary":"turn"}),
        json!({"type":"prefix","runId":null,"requests":[{"requestId":"request","revision":3}],"boundary":"turn"})] {
        let operation:QueueOperation=serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(operation).unwrap(),value);
    }
}
