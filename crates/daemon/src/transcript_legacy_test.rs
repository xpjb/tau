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
