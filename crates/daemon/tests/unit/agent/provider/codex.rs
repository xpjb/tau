use super::*;
#[test]
fn reasoning_summary_parts_are_lines_but_transport_chunks_are_not() {
    let mut stream = Stream::new(&Codex);
    for (index, text) in [(0, "Check"), (0, " the data"), (1, "Compare"), (1, " results")] {
        Codex.decode(&json!({"type":"response.reasoning_summary_text.delta",
            "output_index":0,"summary_index":index,"delta":text}), &mut stream).unwrap();
    }
    assert_eq!(stream.assistant_message()["reasoning"], "Check the data\n\nCompare results");
    Codex.decode(&json!({"type":"response.reasoning_summary_text.delta",
        "output_index":1,"summary_index":0,"delta":"Final check"}), &mut stream).unwrap();
    assert_eq!(stream.assistant_message()["reasoning"], "Check the data\n\nCompare results\n\nFinal check");
    Codex.decode(&json!({"type":"response.reasoning_summary_text.delta",
        "output_index":1,"summary_index":1,"delta":"\nAlready separated"}), &mut stream).unwrap();
    assert_eq!(stream.assistant_message()["reasoning"], "Check the data\n\nCompare results\n\nFinal check\n\nAlready separated");
}
#[test]
fn generated_images_enforce_encoding_format_individual_cumulative_and_count_bounds() {
    let image = |id: &str, result: String| json!({"type":"image_generation_call","id":id,"status":"completed","result":result});
    let png = STANDARD.encode(b"\x89PNG\r\n\x1a\n");
    let mut half = b"\x89PNG\r\n\x1a\n".to_vec(); half.resize(MAX_GENERATED_BYTES / 2 + 1, 0);
    let invalid = [
        vec![image("big", "A".repeat(MAX_GENERATED_BYTES.div_ceil(3)*4+4))],
        vec![image("half1", STANDARD.encode(&half)),image("half2", STANDARD.encode(&half))],
        (0..=MAX_GENERATED_FILES).map(|i| image(&i.to_string(),png.clone())).collect(),
        vec![image("not-png", STANDARD.encode(b"not a PNG"))],
    ];
    for output in invalid {
        let mut stream = Stream::new(&Codex);
        assert!(Codex.decode(&json!({"type":"response.completed","response":{"status":"completed","output":output}}),&mut stream).is_err());
        assert!(stream.images.is_empty(), "Validation failure must not expose even the valid images in the same response");
    }
    let mut stream = Stream::new(&Codex);
    Codex.decode(&json!({"type":"response.completed","response":{"status":"completed","output":(0..MAX_GENERATED_FILES).map(|i| image(&i.to_string(),png.clone())).collect::<Vec<_>>()}}),&mut stream).unwrap();
    assert!(stream.done); assert_eq!(stream.images.len(),MAX_GENERATED_FILES);
    assert!(!stream.assistant_message().to_string().contains(&png));
}

#[test]
fn stream_failures_preserve_provider_code_type_and_status_for_retry_classification() {
    for error in [json!({"type":"server_error","message":"Try again"}),
        json!({"code":"upstream_error","status":503,"message":"Disconnected"}),
        json!({"code":"insufficient_quota","type":"rate_limit_error","message":"Quota exhausted"})] {
        for event in [json!({"type":"error","error":error}),json!({"type":"response.failed","response":{"error":error}})] {
            let mut stream = Stream::new(&Codex);
            Codex.decode(&event,&mut stream).unwrap();
            assert_eq!(stream.error,Some(error.clone()));
        }
    }
}
