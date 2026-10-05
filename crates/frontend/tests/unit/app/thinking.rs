use super::*;
use chad::{Config, HeadlessCtx};
use std::sync::Arc;
use barkdown::markdown::{Content, TextKind, inline};

#[test]
fn native_thinking_stream_updates_markdown_before_sealing() {
    let root = tempfile::tempdir().unwrap();
    let ctx = HeadlessCtx::new(&Config { size: (1000, 800), device_limits: crate::desktop::limits(), ..Default::default() }).unwrap();
    let mut app = App::new(&ctx, Store::open(root.path().into()).unwrap(), Arc::new(|| {}), false).unwrap();
    app.back(); crate::demo::populate(&mut app.controller).unwrap();
    app.resize(ctx.size(), 1., Vec2::new(0., 0.));
    app.controller.chats.get_mut("demo").unwrap().local.details_default = true;
    let mut thinking = app.controller.chats["demo"].feed.events[&1].clone();
    thinking.phase = EventPhase::Live;
    thinking.text = "**First step**\nnotes\n\n**Sec".into();
    let key = "demo/thinking:event-1";
    app.controller.preview("demo", vec![thinking.clone()], QueueState::default(), None).unwrap();
    app.tick(0.); app.frame(&ctx, ctx.view());
    assert_eq!(app.services.renderer.messages[key].source, thinking.text);
    assert_eq!(app.services.renderer.messages[key].doc.blocks().len(), 2);
    let height = app.services.renderer.messages[key].view.height;
    thinking.text.push_str("ond step**");
    app.controller.preview("demo", vec![thinking.clone()], QueueState::default(), None).unwrap();
    app.tick(0.); app.frame(&ctx, ctx.view());
    let fragment = &app.services.renderer.messages[key];
    let rich = &fragment.doc.blocks()[1].elements().next().unwrap().rich;
    assert_eq!(rich.text, "Second step");
    assert!(rich.runs.iter().any(|run| run.flags & inline::STRONG != 0));
    assert_eq!(fragment.view.height, height, "Complete inline syntax does not wait for newline or seal");
    thinking.text.push_str("\n\n# Next topic");
    thinking.phase = EventPhase::Saved;
    app.controller.preview("demo", vec![thinking.clone()], QueueState::default(), None).unwrap();
    app.tick(0.); app.frame(&ctx, ctx.view());
    assert!(matches!(app.services.renderer.messages[key].doc.blocks()[2].content, Content::Text { kind: TextKind::Heading(1), .. }));
    assert!(app.controller.chats["demo"].feed.bodies[&thinking.id].reference.as_ref().unwrap().sealed);
}
