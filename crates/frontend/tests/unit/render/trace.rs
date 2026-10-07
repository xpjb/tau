use super::*;
use chad::{Config, HeadlessCtx};

fn fixture() -> (tempfile::TempDir, HeadlessCtx, Renderer) {
    let directory = tempfile::tempdir().unwrap();
    let ctx = HeadlessCtx::new(&Config {
        size: (420, 220), device_limits: crate::desktop::limits(), ..Default::default()
    }).unwrap();
    let renderer = Renderer::new(&ctx).unwrap();
    (directory, ctx, renderer)
}
fn records(path: &std::path::Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(path).unwrap().lines().map(|line| serde_json::from_str(line).unwrap()).collect()
}

#[test]
fn trace_preserves_pixels_and_reports_incomplete_batches_without_text_content() {
    let (directory, ctx, mut renderer) = fixture();
    let secret = "private-message-and-draft-MUST-NOT-BE-RECORDED";
    let mut layer = Layer::default();
    renderer.label(&mut layer, secret, Rect::new(12., 12., 396., 50.), 12., color(0xffffff), false);
    renderer.draw(&ctx, ctx.view(), std::slice::from_ref(&layer));
    let reference = ctx.read_rgba8().unwrap();
    renderer.trace = Some(Trace::new(directory.path().into(), ctx.device(), ctx.format()));
    for _ in 0..3 {
        renderer.draw(&ctx, ctx.view(), std::slice::from_ref(&layer));
        assert_eq!(ctx.read_rgba8().unwrap(), reference);
    }
    layer.draws.push(Draw { block: sanscale::ShapedHandle::INVALID, ..layer.draws[0] });
    renderer.draw(&ctx, ctx.view(), &[layer]);
    let capture = renderer.trace.as_ref().unwrap().save().unwrap();
    let raw = std::fs::read_to_string(&capture).unwrap();
    assert!(!raw.contains(secret));
    let events = records(&capture);
    assert_eq!(events[0]["kind"], "header");
    assert!(!events[0]["backend"].as_str().unwrap().is_empty());
    let frames: Vec<_> = events.iter().filter(|e| e["kind"] == "frame").collect();
    assert_eq!(frames.len(), 4);
    for frame in &frames[..3] {
        assert_eq!(frame["text_inputs"], 1);
        assert_eq!(frame["nonlive_batches"], 0);
        assert_eq!(frame["empty_layout_draws"], 0);
        assert_eq!(frame["nonfinite_text_draws"], 0);
        assert_eq!(frame["text_signature"], frames[0]["text_signature"]);
    }
    assert_eq!(frames[3]["text_inputs"], 2);
    assert_eq!(frames[3]["nonlive_batches"], 1);
    assert_eq!(frames[3]["empty_layout_draws"], 1);
}

#[test]
fn trace_is_bounded_saves_without_overwrite_and_flushes_on_drop() {
    let (directory, ctx, _renderer) = fixture();
    let mut trace = Trace::new(directory.path().into(), ctx.device(), ctx.format());
    for _ in 0..MAX_RECORDS + 7 { trace.tick(false, true, true, 0, Stamp::now()); }
    assert_eq!(trace.records.len(), MAX_RECORDS);
    assert_eq!(trace.discarded, 7);
    let first = trace.save().unwrap();
    let second = trace.save().unwrap();
    assert_ne!(first, second);
    assert_eq!(std::fs::read(first).unwrap(), std::fs::read(&second).unwrap());
    let events = records(&second);
    assert_eq!(events[1]["discarded_records"], 7);
    assert_eq!(events.len(), MAX_RECORDS + 2);
    assert_eq!(events[2]["wants_redraw"], false);
    drop(trace);
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 3);
}

#[test]
fn trace_save_failure_leaves_rendering_usable() {
    let (directory, ctx, mut renderer) = fixture();
    let file = directory.path().join("not-a-directory");
    std::fs::write(&file, b"keep").unwrap();
    renderer.trace = Some(Trace::new(file.clone(), ctx.device(), ctx.format()));
    assert!(renderer.trace.as_ref().unwrap().save().is_err());
    let mut layer = Layer::default();
    renderer.label(&mut layer, "Still renders", Rect::new(12., 12., 396., 50.), 16., color(0xffffff), false);
    renderer.draw(&ctx, ctx.view(), &[layer]);
    ctx.read_rgba8().unwrap();
    assert_eq!(std::fs::read(file).unwrap(), b"keep");
}

#[test]
fn normal_recording_is_bounded_has_no_detailed_walk_and_adds_no_gpu_work() {
    let (directory, ctx, mut renderer) = fixture();
    let mut layer = Layer::default();
    for n in 0..256 {
        renderer.label(&mut layer, &format!("Distinct label {n}"),
            Rect::new(12., 12., 396., 50.), 12., color(0xffffff), false);
    }
    let layers: Vec<_> = layer.batches().collect();
    let batch = renderer.text.prepare(ctx.device(), ctx.queue(), &layer.draws);
    let mut trace = Trace::normal(directory.path().into(), ctx.device(), ctx.format());
    let capacity = trace.records.capacity();
    assert!(capacity * std::mem::size_of::<Record>() <= 1024 * 1024);
    sanscale::profiling::reset_work_counters();
    let start = Instant::now();
    for _ in 0..10_000 {
        trace.frame(&ctx, &renderer.text, &layers, std::slice::from_ref(&batch), Commands::default(), Instant::now());
    }
    eprintln!("normal recorder, 256 text draws: {} ns/record; ring={} bytes",
        start.elapsed().as_nanos() / 10_000, capacity * std::mem::size_of::<Record>());
    assert_eq!(trace.records.capacity(), capacity);
    assert_eq!(trace.records.len(), MAX_RECORDS);
    assert!(!directory.path().join("latest.jsonl").exists(), "no per-frame file IO");
    let work = sanscale::profiling::work_counters();
    assert_eq!(work.prepares, 0);
    assert_eq!(work.vertex_upload_bytes, 0);
    assert_eq!(work.shape_calls, 0);
    let events = records(&trace.save().unwrap());
    assert_eq!(events[0]["detailed"], false);
    assert_eq!(events[2]["text_inputs"], 256);
    assert_eq!(events[2]["nonlive_batches"], 0);
    assert!(events[2].get("text_signature").is_none());
    assert!(events[2].get("empty_layout_draws").is_none());
    drop(trace);
    assert!(directory.path().join("latest.jsonl").is_file());
    drop(Trace::normal(directory.path().into(), ctx.device(), ctx.format()));
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 2,
        "automatic shutdown saves replace latest, not an unbounded file history");
}

#[test]
fn slow_work_survives_ring_turnover_and_idle_time_is_not_a_freeze() {
    let (directory, ctx, mut renderer) = fixture();
    let mut trace = Trace::normal(directory.path().into(), ctx.device(), ctx.format());
    let baseline = cache_wait_us();
    let stamp = Stamp { at: Instant::now() - std::time::Duration::from_millis(200), cache_wait_us: baseline };
    CACHE_WAIT_US.with(|n| n.set(baseline + 150_000));
    trace.stage(Stage::Pointer, 7, stamp, (true, true, true));
    assert_eq!(trace.slow.len(), 1);
    for _ in 0..MAX_RECORDS + 3 { trace.tick(false, true, true, 0, Stamp::now()); }
    assert_eq!(trace.slow.len(), 1, "a stall must remain available after ordinary history rolls over");
    let events = records(&trace.save().unwrap());
    let slow = events.iter().find(|r| r["kind"] == "slow").unwrap();
    assert_eq!(slow["sample"]["stage"], "pointer");
    assert_eq!(slow["sample"]["cache_wait_us"], 150_000, "wait time is per callback, not lifetime total");
    // An old recording clock alone means idle, not delayed rendering.
    trace.pending_redraw = None;
    trace.started -= std::time::Duration::from_secs(10);
    let batch = renderer.text.prepare(ctx.device(), ctx.queue(), &[]);
    trace.frame(&ctx, &renderer.text, &[], std::slice::from_ref(&batch), Commands::default(), Instant::now());
    assert_eq!(trace.slow.len(), 1);
    trace.pending_redraw = Some(Instant::now() - std::time::Duration::from_millis(200));
    trace.frame(&ctx, &renderer.text, &[], std::slice::from_ref(&batch), Commands::default(), Instant::now());
    assert_eq!(trace.slow.len(), 2, "a pending redraw delayed until submission must be recorded");
    for _ in 0..100 { trace.stage(Stage::Update, 8, stamp, (false, true, true)); }
    assert_eq!(trace.slow.len(), MAX_SLOW_RECORDS);
}

#[test]
fn encoded_commands_distinguish_clear_only_from_background_only() {
    let (directory, ctx, mut renderer) = fixture();
    renderer.trace = Some(Trace::normal(directory.path().into(), ctx.device(), ctx.format()));
    renderer.draw(&ctx, ctx.view(), &[]);
    let clear = ctx.read_rgba8().unwrap();
    assert!(clear.chunks_exact(4).all(|pixel| pixel == &clear[..4]));
    let mut layer = Layer::default();
    layer.rect(Rect::new(0., 0., 420., 220.), color(0x0e141b));
    renderer.draw(&ctx, ctx.view(), &[layer]);
    let background = ctx.read_rgba8().unwrap();
    assert!(background.chunks_exact(4).all(|pixel| pixel == [14, 20, 27, 255]));
    assert_ne!(clear, background);
    eprintln!("clear-only RGBA={:?}; background-only RGBA={:?}", &clear[..4], &background[..4]);
    let events = records(&renderer.trace.as_ref().unwrap().save().unwrap());
    let frames: Vec<_> = events.iter().filter(|e| e["kind"] == "frame").collect();
    assert_eq!(frames.len(), 2);
    for frame in &frames {
        assert_eq!(frame["commands"]["submitted"], true);
        assert_eq!(frame["commands"]["image_draws"], 0);
        assert_eq!(frame["commands"]["text_draws"], 0);
        assert_eq!(frame["commands"]["emoji_draws"], 0);
    }
    assert_eq!(frames[0]["commands"]["shape_draws"], 0);
    assert_eq!(frames[0]["commands"]["shape_vertices"], 0);
    assert_eq!(frames[1]["commands"]["shape_draws"], 1);
    assert_eq!(frames[1]["commands"]["shape_vertices"], 6);
}

#[test]
fn nonempty_text_inputs_can_still_encode_a_clear_only_frame() {
    let (directory, ctx, mut renderer) = fixture();
    renderer.trace = Some(Trace::normal(directory.path().into(), ctx.device(), ctx.format()));
    renderer.draw(&ctx, ctx.view(), &[]);
    let clear = ctx.read_rgba8().unwrap();
    let mut layer = Layer::default();
    renderer.label(&mut layer, "Clipped text", Rect::new(12., 12., 396., 50.), 16., color(0xffffff), false);
    let valid = layer.draws[0];
    layer.draws[0].clip = Some(Rect::new(500., 500., 40., 40.));
    renderer.draw(&ctx, ctx.view(), std::slice::from_ref(&layer));
    assert_eq!(ctx.read_rgba8().unwrap(), clear);
    layer.draws[0] = Draw { block: sanscale::ShapedHandle::INVALID, ..valid };
    renderer.draw(&ctx, ctx.view(), &[layer]);
    assert_eq!(ctx.read_rgba8().unwrap(), clear);
    let events = records(&renderer.trace.as_ref().unwrap().save().unwrap());
    let frames: Vec<_> = events.iter().filter(|e| e["kind"] == "frame").collect();
    assert_eq!(frames.len(), 3);
    for frame in &frames[1..] {
        assert_eq!(frame["text_inputs"], 1, "an input count alone is insufficient");
        assert_eq!(frame["commands"]["submitted"], true);
        assert_eq!(frame["commands"]["shape_draws"], 0);
        assert_eq!(frame["commands"]["image_draws"], 0);
        assert_eq!(frame["commands"]["text_draws"], 0);
        assert_eq!(frame["commands"]["emoji_draws"], 0);
    }
    assert_eq!(frames[1]["nonlive_batches"], 0);
    assert_eq!(frames[2]["nonlive_batches"], 1);
}

#[test]
fn mixed_ui_records_actual_shape_image_and_text_commands() {
    let (directory, ctx, mut renderer) = fixture();
    renderer.trace = Some(Trace::normal(directory.path().into(), ctx.device(), ctx.format()));
    let mut layer = Layer::default();
    layer.rect(Rect::new(0., 0., 420., 220.), color(0x0e141b));
    renderer.label(&mut layer, "Real UI commands", Rect::new(12., 12., 396., 50.), 16., color(0xffffff), false);
    renderer.icon(&ctx, &mut layer, crate::icons::Icon::Gear, Rect::new(12., 80., 24., 24.), 0xffffff);
    renderer.draw(&ctx, ctx.view(), &[layer]);
    let pixels = ctx.read_rgba8().unwrap();
    assert!(pixels.chunks_exact(4).any(|pixel| pixel != &pixels[..4]));
    let events = records(&renderer.trace.as_ref().unwrap().save().unwrap());
    let frame = events.iter().find(|e| e["kind"] == "frame").unwrap();
    assert_eq!(frame["commands"]["submitted"], true);
    assert_eq!(frame["commands"]["shape_draws"], 1);
    assert_eq!(frame["commands"]["image_draws"], 1);
    assert!(frame["commands"]["text_draws"].as_u64().unwrap() > 0);
}
