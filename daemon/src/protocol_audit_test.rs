//! Acceptance tests for the protocol cutover, not legacy cost characterizations.
use serde_json::json;
use tau_protocol::{ClientRequest, ServerMessage, MAX_CONTROL_BYTES};

#[test]
fn retired_transcript_commands_and_messages_are_not_wire_apis() {
    for name in ["open_session","get_history"] {
        assert!(serde_json::from_value::<ClientRequest>(json!({"id":"old","type":name,"sessionId":"chat","generation":"g","before":1})).is_err());
    }
    for name in ["transcript_snapshot","transcript_update","transcript_page"] {
        assert!(serde_json::from_value::<ServerMessage>(json!({"type":name})).is_err());
    }
}

#[tokio::test]
async fn large_descriptors_leave_only_bounded_references_and_receipts_on_control() {
    let root=tempfile::tempdir().unwrap();
    let state=crate::state::StateStore::load(root.path().join("state.db")).await.unwrap();
    let mut response=ServerMessage::success("operation".into(),Some("chat".into()),Some("🦀".repeat(64*1024)));
    if let ServerMessage::Response {notice,..}=&mut response {*notice=Some("\0\"\\".repeat(4096));}
    let frame=state.control_frame(&response).await.unwrap();
    assert!(frame.len()<=MAX_CONTROL_BYTES);
    let ServerMessage::Data {content,reports,..}=serde_json::from_str(&frame).unwrap() else {panic!("Expected native data reference");};
    assert_eq!(reports.len(),1);assert!(reports[0].accepted && reports[0].complete);
    let original=serde_json::to_vec(&response).unwrap();
    assert_eq!(content.length,original.len() as u64);
    let bytes=state.access(move |db|tau_blocks::cached_content(db,&content.scope,&content.id)).await.unwrap();
    assert_eq!(bytes,original,"No long field may be silently truncated");
}
