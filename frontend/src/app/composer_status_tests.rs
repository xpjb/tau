//! Actual shared desktop/phone shaping and GPU frames, without a live connection.
use super::*;
use chad::{Config, HeadlessCtx};
use std::sync::Arc;

#[test]
fn composer_thinking_stays_visible_beside_long_models_on_desktop_and_phone() {
    for (name, size, scale, mobile) in [("desktop", (1000, 700), 1., false),
        ("phone", (320, 720), 1., true), ("scaled-phone", (900, 1800), 2.5, true)] {
        let root = tempfile::tempdir().unwrap();
        let ctx = HeadlessCtx::new(&Config { size, device_limits: crate::desktop::limits(), ..Default::default() }).unwrap();
        let mut app = App::new(&ctx, Store::open(root.path().into()).unwrap(), Arc::new(|| {}), mobile).unwrap();
        app.back(); crate::demo::populate(&mut app.controller).unwrap();
        app.resize(size, scale, Vec2::new(0., 0.)); app.tick(0.); app.root.legacy.show_chats = false;
        let mut summary = app.controller.account.sessions[0].clone();
        summary.model.as_mut().unwrap().model_id = "a-very-long-model-slug-that-must-not-hide-the-thinking-level".into();
        let row = Rect::new(14. * scale, 10. * scale, size.0 as f32 - 28. * scale, 20. * scale);
        for level in [None, Some("off"), Some("minimal"), Some("low"), Some("medium"), Some("high"), Some("xhigh"), Some("max")] {
            summary.thinking_level = level.map(str::to_owned);
            let mut layer = Layer::default();
            app.composer_model_status(&mut layer, Some(&summary), row);
            assert_eq!(layer.draws.len(), 2, "model and thinking have separate reserved space");
            for draw in &layer.draws {
                let layout = app.services.renderer.text.measure(draw.block);
                assert_eq!(layout.line_count(), 1, "no wrapped/clipped second line");
                let clip = draw.clip.unwrap();
                assert!(layout.width_em() * draw.size <= clip.width + 0.1);
                assert!(layout.height_em() * draw.size <= clip.height + 0.1);
                assert!(clip.x + clip.width <= row.x + row.width + 0.1);
            }
            let model = layer.draws[0].clip.unwrap();
            let thinking = layer.draws[1].clip.unwrap();
            assert!(model.x + model.width + 11. * scale <= thinking.x);
            let expected = app.services.renderer.label_width(&format!("Thinking: {}", level.unwrap_or("unknown")), 12. * scale, false);
            let actual = app.services.renderer.text.measure(layer.draws[1].block).width_em() * layer.draws[1].size;
            assert!((actual - expected).abs() < 0.1, "the entire thinking label is drawn, not ellipsized");
        }
        summary.thinking_level = Some("xhigh".into());
        app.controller.message(ServerMessage::Sessions { sessions: vec![summary] }).unwrap();
        app.frame(&ctx, ctx.view());
        let xhigh = ctx.read_rgba8().unwrap();
        if let Some(dir) = std::env::var_os("TAU_THINKING_PREVIEW_DIR") {
            let dir = PathBuf::from(dir); std::fs::create_dir_all(&dir).unwrap();
            image::save_buffer(dir.join(format!("{name}.png")), &xhigh, size.0, size.1, image::ColorType::Rgba8).unwrap();
        }
        app.controller.account.sessions[0].thinking_level = Some("off".into());
        app.frame(&ctx, ctx.view());
        assert_ne!(xhigh, ctx.read_rgba8().unwrap(), "changing only thinking changes the real composer frame");
        app.apply(Action::AgentSetting("demo".into(), "thinking".into())).unwrap();
        assert_eq!(app.root.dialog.as_ref().unwrap().fields()[0].editor.value, "off", "editor reports the selected chat's saved level");
    }
}
