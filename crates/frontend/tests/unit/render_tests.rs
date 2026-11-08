use super::*;

#[test]
fn literal_user_prose_round_trips_barkdown_without_markup() {
    use barkdown::markdown::{Content, TextKind};
    let prose = "* One file for tests (tests.rs) (tests may only test public interfaces) (I'm not joking - express it end to end or it deserves its own crate) (Something has to be done to curb the slop)";
    let source = crate::app::literal(prose);
    let doc = barkdown::Document::new(&source);
    let Content::Text { kind: TextKind::Paragraph, element } = &doc.blocks()[0].content else { panic!() };
    assert_eq!(element.rich.text, prose);
    assert!(element.rich.math.is_none());

    let syntax = "[plain label](https://example.org) &amp; $x^2$ **bold**";
    let rich = barkdown::markdown::inline::parse(&crate::app::literal(syntax));
    assert_eq!(rich.text, syntax);
    assert!(rich.math.is_none());
    assert!(rich.links.is_empty());
    assert!(rich.runs.iter().all(|run| run.flags == 0));

    let punctuation: String = (b'!'..=b'~').map(char::from).filter(|c| c.is_ascii_punctuation()).collect();
    assert_eq!(barkdown::markdown::inline::parse(&crate::app::literal(&punctuation)).text, punctuation);
}

#[cfg(not(target_os = "android"))]
#[test]
fn markdown_link_underline_is_visible_below_the_baseline() {
    let ctx = chad::HeadlessCtx::new(&chad::Config {
        size: (360, 120),
        device_limits: crate::desktop::limits(),
        ..Default::default()
    }).unwrap();
    let mut renderer = Renderer::new(&ctx).unwrap();
    for size in [12., 16., 32.] {
        renderer.message_height("link", "[URL with gaps](https://example.org)", 320., size);
        let mut layer = Layer::default();
        renderer.message(
            &mut layer,
            "link",
            Vec2::new(20., 20.),
            Rect::new(20., 20., 320., 100.),
            0.,
        );
        let draw = &layer.draws[0];
        let layout = renderer.text.measure(draw.block);
        let baseline = draw.at.y + layout.line(0).unwrap().baseline_em * draw.size;
        // Sample within the label's first space: only the underline, not
        // glyph ink, can color this pixel below the measured baseline.
        let space = layout.selection(3..4)[0];
        let x = (draw.at.x + (space.x_em + space.width_em * 0.5) * draw.size).floor() as usize;
        let y = (baseline + draw.size * 0.1 + 0.5).floor() as usize;
        renderer.draw(&ctx, ctx.view(), &[layer]);
        let pixels = ctx.read_rgba8().unwrap();
        let pixel = &pixels[(y * 360 + x) * 4..][..4];
        assert!(pixel[2] > 200 && pixel[2] > pixel[0], "missing blue underline below baseline at {size}px: {pixel:?}");
        if let Some(root) = std::env::var_os("TAU_LINK_UNDERLINE_DUMP_DIR") {
            std::fs::create_dir_all(&root).unwrap();
            image::save_buffer(
                PathBuf::from(root).join(format!("link-{size}.png")),
                &pixels,
                360,
                120,
                image::ColorType::Rgba8,
            ).unwrap();
        }
    }
}

#[test]
fn subtle_hover_and_circle_have_separate_clipped_shapes() {
    let rect = Rect::new(10., 10., 120., 80.);
    let mut layer = Layer::default();
    layer.surface_highlight(rect, [12.; 4], rect, false, true, None);
    assert_eq!(layer.rects.len(), 1);
    assert_eq!(layer.rects[0].color.0[3], 0.018);
    layer.surface_highlight(rect, [12.; 4], rect, false, false, None);
    assert_eq!(layer.rects.len(), 1, "a nested section suppresses the parent hover");
    layer.surface_highlight(rect, [12.; 4], Rect::new(20., 20., 100., 60.), false, false,
        Some((Vec2::new(30., 40.), 15., 0.11)));
    let ripple = layer.rects.last().unwrap();
    assert_eq!(ripple.clip.x, 20.);
    assert_eq!(ripple.clip.y, 25.);
    assert_eq!(ripple.clip.width, 25.);
    assert_eq!(ripple.clip.height, 30.);
    assert_eq!(ripple.ripple.unwrap().1, 15.);
    assert_eq!(ripple.corners, [12.; 4]);
}

#[cfg(not(target_os = "android"))]
#[test]
fn message_cache_pressure_is_lru_and_protects_draws_and_selection() {
    use std::collections::HashSet;
    let ctx = chad::HeadlessCtx::new(&chad::Config {
        size: (360, 240), device_limits: crate::desktop::limits(), ..Default::default()
    }).unwrap();
    let mut renderer = Renderer::new(&ctx).unwrap();
    for key in ["a", "b", "c"] { renderer.message_height(key, key, 300., 16.); }
    renderer.trim_messages(&HashSet::from(["c".into()]));
    assert_eq!(renderer.messages.len(), 3, "ordinary painting cannot evict other chats");
    let b = renderer.messages["b"].doc.identity();
    renderer.message_cache_budget(usize::MAX, 2);
    renderer.trim_messages(&HashSet::from(["c".into()]));
    assert!(!renderer.messages.contains_key("a"));
    assert_eq!(renderer.messages["b"].doc.identity(), b);
    sanscale::profiling::reset_work_counters();
    renderer.message_height("b", "b", 300., 16.);
    assert_eq!(sanscale::profiling::work_counters().shape_calls, 0);
    renderer.message_height("d", "d", 300., 16.);
    renderer.trim_messages(&HashSet::from(["d".into()]));
    assert!(!renderer.messages.contains_key("c"), "b was recently reused");
    renderer.order_messages(["b".into(), "d".into()].into_iter());
    renderer.begin_selection(TextPoint { key: "b".into(), byte: 0 });
    renderer.extend_selection(TextPoint { key: "d".into(), byte: 1 });
    renderer.message_cache_budget(0, usize::MAX);
    renderer.trim_messages(&HashSet::new());
    assert_eq!(renderer.messages.len(), 2, "an active selection may exceed the cache budget");
    assert_eq!(renderer.selected_text().as_deref(), Some("b\n\nd"));
    renderer.selection = None;
    renderer.trim_messages(&HashSet::from(["d".into()]));
    assert_eq!(renderer.messages.len(), 1, "source bytes independently trigger eviction, but current draws remain pinned");
    renderer.message_cache_budget(8 * 1024 * 1024, 4096);
    renderer.message_height("b", "b", 300., 16.);
    assert_ne!(renderer.messages["b"].doc.identity(), b);
}
