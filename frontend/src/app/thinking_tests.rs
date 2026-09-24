use super::*;
use chad::{Config, HeadlessCtx};
use std::sync::Arc;
use tau_markdown::markdown::{Content, TextKind, inline};

#[test]
fn live_summary_sections_are_distinct_markdown_blocks_before_and_after_save() {
    let root = tempfile::tempdir().unwrap();
    let ctx = HeadlessCtx::new(&Config {
        size: (1000, 800),
        device_limits: crate::desktop::limits(),
        ..Default::default()
    })
    .unwrap();
    let mut app = App::new(
        &ctx,
        Store::open(root.path().into()).unwrap(),
        Arc::new(|| {}),
        false,
    )
    .unwrap();
    app.back();
    crate::demo::populate(&mut app.controller).unwrap();
    app.resize(ctx.size(), 1., Vec2::new(0., 0.));
    app.controller
        .chats
        .get_mut("demo")
        .unwrap()
        .local
        .details_default = true;

    let mut events = app
        .controller
        .selected()
        .unwrap()
        .feed
        .events
        .values()
        .take(2)
        .cloned()
        .collect::<Vec<_>>();
    let mut thinking = events[1].clone();
    thinking.phase = EventPhase::Live;
    // The provider separates actual summary parts with a blank line. One
    // newline *inside* a part still has normal Markdown soft-break semantics.
    thinking.text = "**First step**\nnotes\n\n**Sec".into();
    events[1] = thinking.clone();
    let mut next = thinking.clone();
    next.id = "thinking-next".into();
    next.entry_id = "thinking-next".into();
    next.order = 2;
    next.phase = EventPhase::Saved;
    next.text = "# Another summary".into();
    events.push(next);
    app.controller
        .message(ServerMessage::TranscriptSnapshot {
            session_id: "demo".into(),
            snapshot: TranscriptSnapshot {
                generation: "demo".into(),
                sequence: 1,
                events,
                queue: QueueState::default(),
                before: None,
                delivered: vec![],
            },
        })
        .unwrap();
    app.tick(0.);
    app.frame(&ctx, ctx.view());
    let key = "demo/thinking:event-1";
    let fragment = &app.renderer.messages[key];
    assert_eq!(fragment.source, thinking.text);
    assert_eq!(
        fragment.doc.blocks().len(),
        2,
        "summary headings do not join one paragraph"
    );
    assert_eq!(
        fragment.doc.blocks()[0]
            .elements()
            .next()
            .unwrap()
            .rich
            .text,
        "First step notes"
    );
    assert_eq!(
        fragment.doc.blocks()[1]
            .elements()
            .next()
            .unwrap()
            .rich
            .text,
        "**Sec"
    );
    let initial_height = fragment.view.height;
    let rows = app.rows("demo");
    assert_eq!(
        rows[1]
            .details
            .iter()
            .filter(|line| line.key.starts_with("thinking:"))
            .count(),
        2
    );
    assert!(matches!(
        app.renderer.messages["demo/thinking:thinking-next"]
            .doc
            .blocks()[0]
            .content,
        Content::Text {
            kind: TextKind::Heading(1),
            ..
        }
    ));

    app.controller
        .message(ServerMessage::TranscriptUpdate {
            session_id: "demo".into(),
            generation: "demo".into(),
            sequence: 2,
            change: TranscriptChange {
                delta: Some(TextDelta {
                    event_id: thinking.id.clone(),
                    text: "ond step**".into(),
                }),
                ..Default::default()
            },
        })
        .unwrap();
    app.tick(0.);
    app.frame(&ctx, ctx.view());
    let fragment = &app.renderer.messages[key];
    let rich = &fragment.doc.blocks()[1].elements().next().unwrap().rich;
    assert_eq!(rich.text, "Second step");
    assert!(rich.runs.iter().any(|run| run.flags & inline::STRONG != 0));
    assert_eq!(
        fragment.view.height, initial_height,
        "valid inline Markdown must not wait for a newline"
    );

    // A completed block starter also renders before its line is terminated.
    app.controller
        .message(ServerMessage::TranscriptUpdate {
            session_id: "demo".into(),
            generation: "demo".into(),
            sequence: 3,
            change: TranscriptChange {
                delta: Some(TextDelta {
                    event_id: thinking.id.clone(),
                    text: "\n\n# Next topic".into(),
                }),
                ..Default::default()
            },
        })
        .unwrap();
    app.tick(0.);
    app.frame(&ctx, ctx.view());
    let fragment = &app.renderer.messages[key];
    assert!(matches!(
        fragment.doc.blocks()[2].content,
        Content::Text {
            kind: TextKind::Heading(1),
            ..
        }
    ));
    let heading_height = fragment.view.height;

    thinking.phase = EventPhase::Saved;
    thinking.text.push_str("ond step**\n\n# Next topic");
    app.controller
        .message(ServerMessage::TranscriptUpdate {
            session_id: "demo".into(),
            generation: "demo".into(),
            sequence: 4,
            change: TranscriptChange {
                events: vec![thinking.clone()],
                ..Default::default()
            },
        })
        .unwrap();
    app.tick(0.);
    app.frame(&ctx, ctx.view());
    let fragment = &app.renderer.messages[key];
    assert_eq!(fragment.source, thinking.text);
    assert_eq!(
        fragment.doc.blocks()[1]
            .elements()
            .next()
            .unwrap()
            .rich
            .text,
        "Second step"
    );
    assert_eq!(fragment.view.height, heading_height);
    assert_eq!(app.rows("demo")[1].details[1].source, thinking.text);
}
