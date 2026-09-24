//! Explicit, offline screenshot fixture. Never used for real connection state.
use crate::controller::Controller;
use anyhow::Result;
use tau_protocol::*;
pub fn populate(c: &mut Controller) -> Result<()> {
    c.account.sessions = vec![SessionSummary {
        id: "demo".into(),
        title: "A native Tau frontend".into(),
        starter: false,
        status: SessionStatus::Running,
        detail: None,
        context_usage: Some(ContextUsage {
            tokens: Some(18340),
            context_window: Some(200000),
            source: Some(ContextCapacitySource::Provider),
        }),
        model: Some(SessionModel {
            provider: "anthropic".into(),
            model_id: "claude-sonnet-4".into(),
        }),
        parent_id: None,
        created_at_ms: 1,
        updated_at_ms: 4,
    }];
    for (id, title) in [
        ("two", "Incremental Markdown"),
        ("three", "Android input and file transfer"),
    ] {
        let mut s = c.account.sessions[0].clone();
        s.id = id.into();
        s.title = title.into();
        s.status = SessionStatus::Sleeping;
        c.account.sessions.push(s);
    }
    c.account.selected = Some("demo".into());
    c.ensure_chat("demo")?;
    let mut events = vec![];
    for (i,(role,kind,text)) in [
        (EventRole::User,EventKind::Text,"Move the Tau frontend to Rust. Keep the interface familiar, and render Markdown while the reply is arriving."),
        (EventRole::Assistant,EventKind::Thinking,"Review the existing client and keep local work safe across reconnects."),
        (EventRole::Assistant,EventKind::Tool,"cargo nextest run --workspace"),
        (EventRole::Assistant,EventKind::Text,"## Native, end to end\n\nThe new client uses **Chad** for the platform layer and **Sanscale** for text. No JVM is needed on desktop.\n\n| Component | Direction |\n| --- | --- |\n| Protocol | Shared Rust types |\n| Files | Verified QUIC downloads |\n| Markdown | Incremental parsing and layout |\n\n### Stream without the jump\n\nParagraphs, **bold**, *emphasis*, `code`, and tables are rendered as each chunk arrives—not only when the assistant finishes.\n\n```rust\nmessage.append(delta)?;\nview.sync(&message, &mut text, faces, theme, width, size);\n```\n\nLocal drafts survive restarts. Unconfirmed sends remain visible and are **never replayed automatically**."),
    ].into_iter().enumerate(){events.push(Event {id:format!("event-{i}"),order:i as u64,entry_id:format!("entry-{i}"),phase:if i==3{EventPhase::Live}else{EventPhase::Saved},origin:Origin::default(),role,kind,text:text.into(),timestamp:None,timestamp_ms:None,tool_call_id:None,tool_name:if kind==EventKind::Tool{Some("bash".into())}else{None},stop_reason:None,error_message:None,is_error:false,attachment:None});}
    c.message(ServerMessage::TranscriptSnapshot {
        session_id: "demo".into(),
        snapshot: TranscriptSnapshot {
            generation: "demo".into(),
            sequence: 0,
            events,
            queue: QueueState::default(),
            before: None,
            delivered: vec![],
        },
    })?;
    c.connection = "Offline preview".into();
    Ok(())
}
