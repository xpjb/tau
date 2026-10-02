use super::*;
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
