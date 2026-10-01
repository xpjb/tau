//! Real fonts/GPU pixels: retained Markdown and editor handles must not blink
//! out when a late label pushes the shared shaping cache across its bound.
use super::*;
use chad::{Config, HeadlessCtx};
use sanscale::{BlockKey, ParagraphKey, Paragraphs};

const SIZE: (u32, u32) = (420, 220);

fn content(renderer: &mut Renderer, editor: &mut crate::editor::Editor) -> Layer {
    let mut layer = Layer::default();
    layer.rect(Rect::new(0., 0., SIZE.0 as f32, SIZE.1 as f32), color(0x0e141b));
    renderer.message(&mut layer, "kept", Vec2::new(12., 12.), Rect::new(12., 12., 396., 90.), 0.);
    layer.above();
    editor.draw(renderer, &mut layer, Rect::new(12., 108., 396., 60.), 16., false, false, "", false);
    layer
}

fn assert_content(actual: &[u8], expected: &[u8], description: &str) {
    // The footer is allowed to change; the complete message/editor area is not.
    let end = (SIZE.0 * 180 * 4) as usize;
    let different = actual[..end].chunks_exact(4).zip(expected[..end].chunks_exact(4))
        .filter(|(a, b)| a != b).count();
    assert_eq!(different, 0, "{description}: cached text changed in {different} pixels");
}

#[test]
fn cache_pressure_during_paint_preserves_warm_markdown_and_composer_pixels() {
    let ctx = HeadlessCtx::new(&Config {
        size: SIZE, device_limits: crate::desktop::limits(), ..Default::default()
    }).unwrap();
    let mut renderer = Renderer::new(&ctx).unwrap();
    renderer.message_height("kept", "A **retained** message\n\nText must stay visible.", 396., 16.);
    let mut editor = crate::editor::Editor::composer("A retained composer draft".into());
    let layer = content(&mut renderer, &mut editor);
    renderer.draw(&ctx, ctx.view(), &[layer]);
    let reference = ctx.read_rgba8().unwrap();
    let mut blank = Layer::default();
    blank.rect(Rect::new(0., 0., SIZE.0 as f32, SIZE.1 as f32), color(0x0e141b));
    renderer.draw(&ctx, ctx.view(), &[blank]);
    assert_ne!(reference, ctx.read_rgba8().unwrap(), "the reference must contain actual text pixels");

    // Real production capacity, not a reduced test-only cache. Share one tiny
    // paragraph so this stresses block residency, not glyph/font/texture limits.
    let style = Style { chain: renderer.faces.prose[0], wrap_em: None,
        align: Align::Left, line_spacing: 1. };
    let parts = [ParagraphKey { namespace: u64::MAX, slot: 0, generation: 1 }];
    let live = renderer.text.diagnostics().cache_occupancy().1;
    for i in 0..(131_072 - live) {
        renderer.text.shape(BlockKey(0xffff_ffff_0000_0000 | i as u64), &style, &parts, &Paragraphs(&["x"])).unwrap();
    }
    assert_eq!(renderer.text.diagnostics().cache_occupancy().1, 131_072);

    // Neither source changed. Tau reuses their handles instead of re-shaping
    // them on each paint. Preparing/drawing this warm frame is a real use.
    sanscale::profiling::reset_work_counters();
    let layer = content(&mut renderer, &mut editor);
    renderer.draw(&ctx, ctx.view(), &[layer]);
    assert_content(&ctx.read_rgba8().unwrap(), &reference, "warm frame before sweep");
    let work = sanscale::profiling::work_counters();
    assert_eq!((work.block_requests, work.shape_calls, work.flow_calls), (0, 0, 0));

    // Like a status/tooltip label appended after the transcript/composer: this
    // allocation sweeps while their Draw handles are already in the frame.
    let mut layer = content(&mut renderer, &mut editor);
    layer.above();
    renderer.label(&mut layer, "New status", Rect::new(12., 184., 396., 28.), 14., color(0xffffff), false);
    assert!(renderer.text.diagnostics().cache_occupancy().1 < 131_072, "a real sweep must occur");
    renderer.draw(&ctx, ctx.view(), &[layer]);
    let swept = ctx.read_rgba8().unwrap();
    let layer = content(&mut renderer, &mut editor);
    renderer.draw(&ctx, ctx.view(), &[layer]);
    assert_content(&ctx.read_rgba8().unwrap(), &reference, "following recovery frame");
    if let Some(dir) = std::env::var_os("TAU_REPAINT_DUMP_DIR") {
        std::fs::create_dir_all(&dir).unwrap();
        for (name, pixels) in [("reference.png", &reference), ("sweep.png", &swept)] {
            image::save_buffer(PathBuf::from(&dir).join(name), pixels, SIZE.0, SIZE.1, image::ColorType::Rgba8).unwrap();
        }
    }
    assert_content(&swept, &reference, "sweep frame must not lose text for one paint");
}
