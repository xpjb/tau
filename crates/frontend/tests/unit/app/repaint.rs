//! Ordinary, small-cache repaints: settling, hover and repeated GPU work.
use super::*;
use chad::{Config, HeadlessCtx};
use std::sync::Arc;

fn fixture() -> (tempfile::TempDir, HeadlessCtx, App) {
    fixture_with_wake(Arc::new(|| {}))
}
fn fixture_with_wake(wake: crate::net::Wake) -> (tempfile::TempDir, HeadlessCtx, App) {
    let root = tempfile::tempdir().unwrap();
    let ctx = HeadlessCtx::new(&Config {
        size: (1000, 700), device_limits: crate::desktop::limits(), ..Default::default()
    }).unwrap();
    let mut app = App::new(&ctx, Store::open(root.path().into()).unwrap(), wake, false).unwrap();
    app.back();
    crate::demo::populate(&mut app.controller).unwrap();
    app.resize(ctx.size(), 1., Vec2::new(0., 0.));
    (root, ctx, app)
}

fn settle(app: &mut App, ctx: &HeadlessCtx) {
    // Mirrors the desktop's update -> paint -> request-another-frame decision.
    for _ in 0..8 {
        let again = app.tick(1. / 60.);
        app.frame(ctx, ctx.view());
        if !again && !app.needs_redraw() { return; }
    }
    panic!("an ordinary unchanged view must stop requesting frames");
}

#[test]
fn ordinary_chat_and_composer_settle_and_stay_idle_without_input() {
    let (_root, ctx, mut app) = fixture();
    settle(&mut app, &ctx);
    app.ui.focus = Some(app.root.workspace.chat.composer.field.control.target);
    app.input("An unchanged draft should not keep the renderer busy.");
    settle(&mut app, &ctx);
    for focused in [true, false] {
        app.ui.window_focused = focused;
        settle(&mut app, &ctx);
        for _ in 0..32 {
            assert!(!app.tick(1. / 60.), "idle tick requested a frame; focused={focused}");
            assert!(!app.needs_redraw());
        }
    }
    assert!(app.services.renderer.text.diagnostics().cache_occupancy().1 < 1024);
}

#[test]
fn repeated_ordinary_repaints_preserve_pixels_without_cache_pressure() {
    let (_root, ctx, mut app) = fixture();
    app.ui.focus = Some(app.root.workspace.chat.composer.field.control.target);
    app.input("Repeated ordinary frames retain this draft.");
    settle(&mut app, &ctx);
    let reference = ctx.read_rgba8().unwrap();
    sanscale::profiling::reset_work_counters();
    for _ in 0..32 {
        app.tick(1. / 60.);
        app.frame(&ctx, ctx.view());
        assert_eq!(ctx.read_rgba8().unwrap(), reference);
    }
    let work = sanscale::profiling::work_counters();
    eprintln!("32 unchanged paints: batch_buffers={} prepares={} vertex_upload_bytes={} shape_calls={} flow_calls={}",
        work.batch_buffers, work.prepares, work.vertex_upload_bytes, work.shape_calls, work.flow_calls);
    assert!(app.services.renderer.text.diagnostics().cache_occupancy().1 < 1024);
}

#[test]
fn topic_clicks_request_their_own_frames_without_network_activity() {
    let (root, ctx, mut app) = fixture();
    app.controller.account.projects.push(Project { id: "other".into(), name: "Other topic".into(), prompt: String::new(), revision: 1 });
    app.controller.account.sessions[2].project_id = "other".into();
    settle(&mut app, &ctx);
    for (topic, chat) in [("other", "three"), ("general", "demo")] {
        let rect = app.root.workspace.sidebar.projects.controls.placed().find_map(|(choice, rect)|
            matches!(choice, ui::sidebar::TopicChoice::Select(id) if id == topic).then_some(rect)).unwrap();
        let point = Vec2::new(rect.x + rect.width / 2., rect.y + rect.height / 2.);
        assert!(!app.needs_redraw());
        app.press(0, point, false);
        assert!(app.needs_redraw(), "mouse-down itself must schedule desktop redraw");
        assert!(app.tick(0.));
        app.frame(&ctx, ctx.view());
        app.release(0, point);
        assert_eq!(app.controller.account.selected_project, topic);
        assert_eq!(app.controller.account.selected.as_deref(), Some(chat));
        assert!(app.needs_redraw(), "navigation cannot depend on a later network wake");
        assert!(app.tick(0.));
        app.frame(&ctx, ctx.view());
        assert!(app.root.workspace.sidebar.controls.placed().any(|(choice, _)|
            matches!(choice, ui::sidebar::Choice::Select(id) if id == chat)));
        // Allow the real finite click ripple to finish, rather than clearing
        // dirty flags or painting continuously to hide a scheduling failure.
        std::thread::sleep(std::time::Duration::from_millis(600));
        settle(&mut app, &ctx);
    }
    // Optional persistent offline fixture for the real-window smoke test.
    if let Some(path) = std::env::var_os("TAU_NAVIGATION_FIXTURE") {
        let identity = app.controller.identity.clone();
        let lineage = app.controller.chats["demo"].feed.generation.split(':').next().unwrap().to_owned();
        app.controller.account.source_lineage = Some(lineage);
        app.controller.store.put(&identity, "account", &app.controller.account).unwrap();
        app.save().unwrap();
        drop(app);
        let kept = root.keep();
        std::fs::rename(kept, path).unwrap();
    }
}

#[test]
fn download_progress_drives_real_poll_and_paint_without_websocket_traffic() {
    use crate::net::Event as NetworkEvent;
    use super::download_render_tests::{cases, install};
    use std::{sync::mpsc, time::Duration};
    let (tx, rx) = mpsc::channel();
    let gate = crate::desktop::wake::WakeGate::new(Arc::new(move || { let _ = tx.send(()); }));
    let (_root, ctx, mut app) = fixture_with_wake(gate.callback());
    let case = cases().into_iter().find(|c| c.id == "07-progress").unwrap();
    let file = install(&mut app, &case);
    paint_download(&mut app, &ctx, &case.id, file);
    settle(&mut app, &ctx);
    let mailbox = app.controller.test_mailbox(gate.callback());
    let reference = ctx.read_rgba8().unwrap();
    let key = Controller::download_key("demo", &case.id);
    let path = app.controller.attachment_path("demo", &case.id);
    for transferred in 1..=1000 {
        mailbox.send(NetworkEvent::Download { key: key.clone(), path: path.clone(),
            status: TransferStatus { transferred, total: 2000, network_bytes: transferred, done: false, failure: None } });
    }
    rx.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(rx.try_recv().is_err(), "a progress burst needs one OS wake, not 1000");
    assert_eq!(gate.begin_update(), (1000, 1));
    assert!(app.tick(0.), "the real event mailbox must dirty the App without a heartbeat");
    assert_eq!(app.controller.downloads[&key].status.transferred, 1000);
    app.frame(&ctx, ctx.view());
    assert_ne!(ctx.read_rgba8().unwrap(), reference, "download progress must actually change pixels");
    // An event after the drain is not lost merely because an older frame was pending.
    mailbox.send(NetworkEvent::Download { key: key.clone(), path,
        status: TransferStatus { transferred: 2000, total: 2000, network_bytes: 2000, done: true, failure: None } });
    rx.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(gate.begin_update(), (1, 1));
    assert!(app.tick(0.));
    app.frame(&ctx, ctx.view());
    assert!(app.controller.downloads[&key].status.done);
}

#[test]
fn indeterminate_download_has_an_independent_timer_and_stops_when_hidden() {
    use super::download_render_tests::{cases, install};
    use std::{sync::mpsc, time::Duration};
    let (tx, rx) = mpsc::channel();
    let (_root, ctx, mut app) = fixture_with_wake(Arc::new(move || { let _ = tx.send(()); }));
    let case = cases().into_iter().find(|c| c.state == "active" && c.size.is_none()).unwrap();
    let file = install(&mut app, &case);
    paint_download(&mut app, &ctx, &case.id, file);
    let reference = ctx.read_rgba8().unwrap();
    rx.recv_timeout(Duration::from_secs(1)).expect("spinner needs its own wake, not a heartbeat");
    assert!(app.tick(0.));
    app.frame(&ctx, ctx.view());
    assert_ne!(ctx.read_rgba8().unwrap(), reference);
    app.set_connection_visible(false);
    app.tick(0.);
    while rx.try_recv().is_ok() {}
    assert!(rx.recv_timeout(Duration::from_millis(250)).is_err(), "hidden spinner must not keep waking the UI");
}

fn paint_download(app: &mut App, ctx: &HeadlessCtx, id: &str, file: ChatAttachment) {
    let mut event = app.controller.chats["demo"].feed.events.values().next().unwrap().clone();
    event.id = id.into(); event.entry_id = id.into(); event.order = 0;
    event.role = EventRole::Assistant; event.kind = EventKind::Text; event.phase = EventPhase::Saved;
    event.text.clear(); event.attachment = Some(file);
    app.controller.preview("demo", vec![event], QueueState::default(), None).unwrap();
    app.root.workspace.show_chats = false;
    app.root.workspace.attachments.show = true;
    app.tick(0.);
    app.frame(ctx, ctx.view());
}

#[test]
fn app_focus_return_and_chat_hover_submit_nonempty_ui_not_just_a_clear() {
    let (root, ctx, mut app) = fixture();
    settle(&mut app, &ctx);
    app.services.renderer.trace = Some(crate::render::trace::Trace::normal(
        root.path().join("diagnostics"), ctx.device(), ctx.format()));
    let rect = app.root.workspace.sidebar.controls.placed().find_map(|(choice, rect)|
        matches!(choice, ui::sidebar::Choice::Select(id) if id == "demo").then_some(rect)).unwrap();
    let point = Vec2::new(rect.x + rect.width / 2., rect.y + rect.height / 2.);
    // App-level regression only; the real-window probe separately drives OS
    // focus/hover. Neither reproduces a Windows AMD driver/compositor here.
    for _ in 0..3 {
        app.ui.window_focused = false;
        app.cancel_pointer();
        app.hover(None);
        settle(&mut app, &ctx);
        assert!(!app.tick(0.));
        app.ui.window_focused = true;
        app.tick(0.);
        app.frame(&ctx, ctx.view());
        let returned = ctx.read_rgba8().unwrap();
        assert!(returned.chunks_exact(4).any(|pixel| pixel != &returned[..4]));
        app.hover(Some(point));
        app.tick(0.);
        app.frame(&ctx, ctx.view());
        let hovered = ctx.read_rgba8().unwrap();
        assert!(hovered.chunks_exact(4).any(|pixel| pixel != &hovered[..4]));
    }
    let path = app.services.renderer.trace.as_ref().unwrap().save().unwrap();
    let events: Vec<serde_json::Value> = std::fs::read_to_string(path).unwrap().lines()
        .map(|line| serde_json::from_str(line).unwrap()).collect();
    let frames: Vec<_> = events.iter().filter(|e| e["kind"] == "frame").collect();
    assert!(frames.len() >= 6);
    for frame in frames {
        assert_eq!(frame["commands"]["submitted"], true);
        assert!(frame["commands"]["shape_vertices"].as_u64().unwrap() > 6);
        assert!(frame["commands"]["image_draws"].as_u64().unwrap() > 0);
        assert!(frame["commands"]["text_draws"].as_u64().unwrap() > 0);
        assert_eq!(frame["nonlive_batches"], 0);
    }
}
