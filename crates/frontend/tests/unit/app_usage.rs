use super::*;

#[test]
fn token_usage_does_not_require_a_guessed_context_window() {
    let unknown_capacity = Some(ContextUsage { tokens: Some(1_024), context_window: None });
    let (ring, text) = context_usage_display(unknown_capacity);
    assert_eq!(ring, None);
    assert_eq!(text.text, "Context · ~1,024 tokens used\nCapacity unknown");
    assert_eq!(
        context_usage_display(Some(ContextUsage { tokens: None, context_window: None })).1.text,
        "Context · usage unknown\nCapacity unknown"
    );
    assert_eq!(context_usage_display(None).1.text, "Context · usage unavailable");

    let (ring, text) =
        context_usage_display(Some(ContextUsage { tokens: Some(1_024), context_window: Some(4_096) }));
    assert_eq!(ring, Some(0.25));
    assert_eq!(text.text, "Context · ~25% used\n1,024 of 4,096 tokens");
    assert_eq!(
        context_usage_display(Some(ContextUsage { tokens: None, context_window: Some(4_096) })).1.text,
        "Context · usage unknown\nCapacity: 4,096 tokens"
    );
}
