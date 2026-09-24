use super::*;
use chad::{Config, HeadlessCtx};
use std::sync::Arc;

fn pixel(image: &[u8], point: Vec2, width: usize) -> [u8; 4] {
    let at = (point.y as usize * width + point.x as usize) * 4;
    image[at..at + 4].try_into().unwrap()
}

#[test]
fn detail_rows_hover_independently_and_click_ripple_targets_the_inner_row() {
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
    let events = app
        .controller
        .selected()
        .unwrap()
        .feed
        .events
        .values()
        .take(3)
        .cloned()
        .collect();
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
    let chat = app.controller.chats.get_mut("demo").unwrap();
    chat.local.details_default = true;
    chat.local.expansion.insert("tool:event-2".into(), true);
    app.tick(0.);
    app.frame(&ctx, ctx.view());
    let outer = app
        .message_areas
        .iter()
        .find(|a| a.key == "demo/details:event-1")
        .unwrap()
        .rect;
    let inside = app
        .detail_areas
        .iter()
        .find(|a| a.key == "demo/tool:event-2")
        .unwrap()
        .rect;
    assert!(
        app.detail_areas
            .iter()
            .any(|a| a.key == "demo/tool:event-2:Input")
    );
    assert!(
        app.detail_areas
            .iter()
            .any(|a| a.key == "demo/tool:event-2:Input:text")
    );
    let outside_point = Vec2::new(outer.x + 8., outer.y + 20.);
    let inside_point = Vec2::new(inside.x + 2., inside.y + inside.height / 2.);
    assert_eq!(
        app.section_at(outside_point).unwrap().0,
        "demo/details:event-1"
    );
    assert_eq!(app.section_at(inside_point).unwrap().0, "demo/tool:event-2");
    let baseline = ctx.read_rgba8().unwrap();

    app.hover(Some(outside_point));
    assert!(app.needs_redraw());
    app.tick(0.);
    app.frame(&ctx, ctx.view());
    let outer_hover = ctx.read_rgba8().unwrap();
    assert_ne!(
        pixel(&baseline, outside_point, 1000),
        pixel(&outer_hover, outside_point, 1000)
    );

    app.hover(Some(inside_point));
    assert!(
        app.needs_redraw(),
        "moving between inner and outer sections repaints"
    );
    app.tick(0.);
    app.frame(&ctx, ctx.view());
    let inner_hover = ctx.read_rgba8().unwrap();
    assert_eq!(
        pixel(&baseline, outside_point, 1000),
        pixel(&inner_hover, outside_point, 1000),
        "inner hover does not wash the whole outer panel"
    );
    assert_ne!(
        pixel(&baseline, inside_point, 1000),
        pixel(&inner_hover, inside_point, 1000)
    );

    app.press(1, inside_point, false);
    assert_eq!(app.ripple.as_ref().unwrap().key, "demo/tool:event-2");
    app.tick(0.);
    app.frame(&ctx, ctx.view()); // exercises the real GPU circle/rounded-clip shader
    app.motion(1, Vec2::new(inside_point.x + 20., inside_point.y));
    assert!(
        app.ripple.is_none(),
        "dragging to scroll must not leave a click ripple"
    );
    app.release(1, inside_point);
    assert!(app.ripple.is_none());
}
