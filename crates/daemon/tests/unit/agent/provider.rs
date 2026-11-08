use super::*;
#[test]
fn completed_checkpoint_survives_an_error_in_the_same_transport_chunk() {
    for tail in [b"data: {\"type\":\"error\",\"error\":{\"code\":\"server_error\"}}\n\n".as_slice(), b"data: malformed-json\n\n"] {
        let capture = Recovery::default();
        recovery::Replay::new(&SessionModel {provider:"openai-codex".into(),model_id:"gpt-6-astra".into()},"fixture","http://fixture",&[]).capture_into(&capture);
        let item = json!({"type":"reasoning","id":"rs_chunk","encrypted_content":"private-cipher","summary":[]});
        let mut wire = format!("data: {}\n\n",json!({"type":"response.output_item.done","output_index":0,"item":item})).into_bytes();
        wire.extend_from_slice(tail);
        let mut stream = Stream::new(&codex::Codex); stream.recovery = Some(&capture);
        let _ = stream.push(&wire);
        assert_eq!(capture.snapshot().unwrap()["items"],json!([item]));
        assert!(!stream.assistant_message().to_string().contains("private-cipher"));
    }
}
#[test]
fn frames_sse_at_every_byte_boundary_and_rejects_invalid_or_oversized_input() {
    for separator in ["\n\n", "\r\n\r\n", "\r\r"] {
        let wire = format!(": heartbeat{separator}data: {}{separator}data: [DONE]{separator}data: ignored{separator}",
            json!({"choices":[{"index":0,"delta":{"content":"Hello π🧠"},"finish_reason":"stop"}]}));
        for cut in 0..wire.len() {
            let mut stream = Stream::new(&completions::Completions);
            stream.push(&wire.as_bytes()[..cut]).unwrap(); stream.push(&wire.as_bytes()[cut..]).unwrap();
            assert!(stream.done); assert_eq!(stream.message["content"], "Hello π🧠");
        }
    }
    let mut malformed = Stream::new(&completions::Completions);
    assert!(malformed.push(b"data: nope\n\n").is_err());
    let mut large = Stream::new(&completions::Completions);
    assert!(large.push(&vec![b'x'; MAX_RESPONSE_BYTES + 1]).is_err());
    let mut codex = Stream::new(&codex::Codex);
    assert!(codex.push(b"data: [DONE]\n\n").is_err(), "Codex requires a completed response");
}

#[test]
fn transient_stream_classification_preserves_details_without_retrying_permanent_causes() {
    for code in [json!(408),json!(429),json!(502),json!("503"),json!("upstream_error"),json!("server_error"),json!("overloaded_error")] {
        assert!(transient_stream_error(&json!({"code":code})),"{code}");
    }
    for error in [json!({"code":"insufficient_quota","type":"rate_limit_error"}),
        json!({"code":"invalid_api_key","type":"server_error"}),
        json!({"code":"context_length_exceeded"}),json!({"code":"invalid_reasoning_signature"}),
        json!({"code":400}),json!({"code":401}),json!({"code":"unknown_error"})] {
        assert!(!transient_stream_error(&error),"{error}");
    }
    let error = stream_error(&json!({"code":"upstream_error","message":"secret-token account-value disconnected"}),"secret-token",Some("account-value"));
    assert!(error.is::<StreamFailure>());
    assert_eq!(error.to_string(),"Model provider stream error (upstream_error): [redacted] [redacted] disconnected");
}
