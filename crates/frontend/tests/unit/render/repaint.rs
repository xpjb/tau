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

#[test]
fn queued_repaints_keep_text_pixels_stable_across_surfaces_and_atlas_updates() {
    let size = (640, 320);
    let ctx = HeadlessCtx::new(&Config {
        size, device_limits: crate::desktop::limits(), ..Default::default()
    }).unwrap();
    let mut renderer = Renderer::new(&ctx).unwrap();
    let mut layer = Layer::default();
    layer.rect(Rect::new(0., 0., size.0 as f32, size.1 as f32), color(0x0e141b));
    for i in 0..64 {
        layer.above();
        let rect = Rect::new((i % 8) as f32 * 80. + 4., (i / 8) as f32 * 40. + 4., 72., 32.);
        layer.rect(rect, color(0x18212b));
        renderer.label(&mut layer, &format!("Warm text {i}"), rect, 12., color(0xe5eaf0), i % 2 == 0);
        if i % 8 == 0 {
            renderer.icon(&ctx, &mut layer, crate::icons::Icon::Gear,
                Rect::new(rect.x + 56., rect.y + 16., 12., 12.), 0x67d4ff);
        }
    }
    renderer.draw(&ctx, ctx.view(), std::slice::from_ref(&layer));
    let reference = ctx.read_rgba8().unwrap();
    assert!(reference.chunks_exact(4).any(|p| p[0] > 180), "the reference must contain bright text ink");
    let extent = wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 };
    let target = ctx.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("queued repaint target"), size: extent, mip_level_count: 1, sample_count: 1,
        dimension: wgpu::TextureDimension::D2, format: ctx.format(),
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC, view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let style = Style { chain: renderer.faces.prose[0], wrap_em: None, align: Align::Left, line_spacing: 1. };
    let mut snapshots = Vec::new();
    sanscale::profiling::reset_work_counters();
    for i in 0..32 {
        // New glyphs exercise atlas uploads while every displayed draw stays
        // unchanged. No readback/poll serializes these submitted repaint frames.
        renderer.text.shape_transient(&char::from_u32(33 + i).unwrap().to_string(), &style).unwrap();
        renderer.draw(&ctx, &view, std::slice::from_ref(&layer));
        let snapshot = ctx.device().create_buffer(&wgpu::BufferDescriptor {
            label: Some("queued repaint snapshot"), size: u64::from(size.0 * size.1 * 4),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false,
        });
        let mut encoder = ctx.device().create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(target.as_image_copy(), wgpu::TexelCopyBufferInfo {
            buffer: &snapshot, layout: wgpu::TexelCopyBufferLayout {
                offset: 0, bytes_per_row: Some(size.0 * 4), rows_per_image: Some(size.1),
            },
        }, extent);
        ctx.queue().submit([encoder.finish()]);
        snapshots.push(snapshot);
    }
    assert!(sanscale::profiling::work_counters().text_atlas_upload_bytes > 0, "new glyphs must actually update the GPU atlas");
    let (tx, rx) = std::sync::mpsc::channel();
    for snapshot in &snapshots {
        let tx = tx.clone();
        snapshot.slice(..).map_async(wgpu::MapMode::Read, move |result| tx.send(result).unwrap());
    }
    ctx.device().poll(wgpu::PollType::wait_indefinitely()).unwrap();
    for _ in &snapshots { rx.recv().unwrap().unwrap(); }
    for (i, snapshot) in snapshots.iter().enumerate() {
        let pixels = snapshot.slice(..).get_mapped_range().unwrap();
        let different = pixels.chunks_exact(4).zip(reference.chunks_exact(4)).filter(|(a, b)| a != b).count();
        assert_eq!(different, 0, "queued repaint {i} lost/changed text in {different} pixels");
    }
}
