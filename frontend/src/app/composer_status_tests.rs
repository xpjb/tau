//! Actual shared desktop/phone shaping and GPU frames, without a live connection.
use super::*;
use chad::{Config, HeadlessCtx};
use std::sync::Arc;

#[test]
fn thinking_label_fits_beside_a_long_model_in_a_narrow_composer() {
    let root = tempfile::tempdir().unwrap();
    let size = (320, 720);
    let ctx = HeadlessCtx::new(&Config { size, device_limits: crate::desktop::limits(), ..Default::default() }).unwrap();
    let mut app = App::new(&ctx, Store::open(root.path().into()).unwrap(), Arc::new(|| {}), true).unwrap();
    app.back(); crate::demo::populate(&mut app.controller).unwrap(); app.tick(0.);
    let mut summary = app.controller.account.sessions[0].clone();
    summary.model.as_mut().unwrap().model_id = "a-very-long-model-slug-that-must-not-hide-the-thinking-level".into();
    summary.thinking_level = Some("xhigh".into());
    let row = Rect::new(14., 10., size.0 as f32 - 28., 20.);
    let mut layer = Layer::default();
    app.with_ui(|root, cx| root.workspace.chat.composer.model_status(cx, &mut layer, Some(&summary), row));
    assert!(!layer.draws.is_empty());
    for draw in &layer.draws {
        let layout = app.services.renderer.text.measure(draw.block);
        assert!(draw.at.x >= row.x && draw.at.x + layout.width_em() * draw.size <= row.x + row.width + 0.1);
        assert!(draw.at.y + layout.height_em() * draw.size <= row.y + row.height + 0.1);
    }
    app.services.renderer.draw(&ctx, ctx.view(), &[layer]);
    let high = ctx.read_rgba8().unwrap();
    summary.thinking_level = Some("off".into());
    let mut layer = Layer::default();
    app.with_ui(|root, cx| root.workspace.chat.composer.model_status(cx, &mut layer, Some(&summary), row));
    app.services.renderer.draw(&ctx, ctx.view(), &[layer]);
    assert_ne!(high, ctx.read_rgba8().unwrap(), "Changing only thinking must change this isolated status row");
}
