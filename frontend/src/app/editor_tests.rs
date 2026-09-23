//! Shared input through the actual App, SQLite store, renderer and real fonts.
//! Only the connection state is the explicitly offline demo; no live account.
use super::*;
use chad::{Config, HeadlessCtx};
use std::sync::Arc;

struct Harness {
    app: App,
    ctx: HeadlessCtx,
    _root: tempfile::TempDir,
}
impl Harness {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let ctx = HeadlessCtx::new(&Config {
            size: (1000, 800), device_limits: crate::desktop::limits(), ..Default::default()
        }).unwrap();
        let mut app = App::new(&ctx, Store::open(root.path().into()).unwrap(), Arc::new(|| {}), false).unwrap();
        app.back();
        crate::demo::populate(&mut app.controller).unwrap();
        app.resize(ctx.size(), 1., Vec2::new(0., 0.));
        app.tick(0.);
        app.focus = Some(None);
        Self { app, ctx, _root: root }
    }
    fn frame(&mut self) -> Vec<u8> {
        self.app.tick(0.);
        self.app.frame(&self.ctx, self.ctx.view());
        self.ctx.read_rgba8().unwrap()
    }
    fn dump(&self, name: &str, pixels: &[u8]) {
        if let Some(root) = std::env::var_os("TAU_EDITOR_DUMP_DIR") {
            std::fs::create_dir_all(&root).unwrap();
            image::save_buffer(PathBuf::from(root).join(name), pixels, 1000, 800, image::ColorType::Rgba8).unwrap();
        }
    }
    fn copy(&mut self) -> String {
        self.app.key("c", true, false);
        self.app.actions().into_iter().find_map(|action| match action {
            PlatformAction::Copy(text) => Some(text), _ => None,
        }).expect("real App clipboard action")
    }
}

#[test]
fn composer_navigation_repaints_without_sqlite_draft_writes_or_reshaping() {
    let mut h = Harness::new();
    h.app.input("abcdefghij\nab\nabcdefghij\n👩‍💻");
    h.frame();
    h.app.key("Home", true, false);
    let before = h.frame();
    h.dump("composer-before.png", &before);
    // A trigger observes even redundant same-value UPDATEs. Counting changed
    // text alone would miss the old synchronous FULL/WAL write on every arrow.
    let db = rusqlite::Connection::open(h.app.controller.store.root.join("client.sqlite3")).unwrap();
    db.execute_batch("CREATE TABLE editor_writes (key TEXT); CREATE TRIGGER editor_write AFTER UPDATE ON local WHEN NEW.key LIKE 'chat:%' BEGIN INSERT INTO editor_writes VALUES (NEW.key); END;").unwrap();
    sanscale::profiling::reset_work_counters();
    for _ in 0..9 { h.app.key("ArrowRight", false, false); }
    h.app.key("ArrowDown", false, false);
    h.app.key("ArrowDown", false, false);
    h.app.key("ArrowLeft", false, true);
    assert_eq!(h.copy(), "i");
    assert!(h.app.needs_redraw(), "on-demand input must schedule a frame immediately");
    let work = sanscale::profiling::work_counters();
    assert_eq!((work.block_requests, work.shape_calls, work.flow_calls), (0, 0, 0));
    assert_eq!(db.query_row("SELECT count(*) FROM editor_writes", [], |r| r.get::<_, u32>(0)).unwrap(), 0);
    let after = h.frame();
    assert_ne!(after, before, "selection/caret must visibly move in the GPU frame");
    h.dump("composer-selection.png", &after);
    assert_eq!(after, h.frame(), "same input/frame must remain stable");
    h.app.key("x", true, false);
    assert_eq!(h.app.controller.selected().unwrap().local.draft, "abcdefghij\nab\nabcdefghj\n👩‍💻");
    assert_eq!(db.query_row("SELECT count(*) FROM editor_writes", [], |r| r.get::<_, u32>(0)).unwrap(), 1);
    h.app.key("z", true, false);
    assert_eq!(h.app.composer.value, "abcdefghij\nab\nabcdefghij\n👩‍💻");
}

#[test]
fn actual_prompt_settings_reuse_input_geometry_clipboard_ime_and_scrolling() {
    let mut h = Harness::new();
    let content = format!("abcdefghij\nab\nabcdefghij\n{}", "a long settings prompt with emoji 😀\n".repeat(35));
    let mut settings = tau_protocol::settings::Settings::default();
    settings.agent.system_prompt = content.clone();
    h.app.daemon_draft = Some(crate::daemon_settings::Draft::new(&settings, h.app.controller.identity.clone()).unwrap());
    h.app.modal = Some(Modal { kind: ModalKind::Daemon, title: "Daemon settings".into(), fields: vec![], options: vec![] });
    h.app.load_setting_field().unwrap();
    h.app.focus = Some(Some(0));
    h.frame();
    h.app.key("Home", true, false);
    h.app.key("ArrowDown", false, false);
    h.frame();
    let caret = h.app.ime_rect().unwrap();
    h.app.key("End", true, false);
    h.app.key("ArrowLeft", false, false);
    // Do not repaint before the click: hit-testing must use the displayed
    // scroll origin, not silently scroll to the newly placed caret first.
    let point = Vec2::new(caret.x + 0.1, caret.y + caret.height * 0.5);
    h.app.press(1, point, false);
    h.app.release(1, point);
    h.app.key("ArrowRight", true, true);
    assert_eq!(h.copy(), "ab", "settings uses its real 15px layout, not a hard-coded 16px hit layout");
    let committed = h.app.modal.as_ref().unwrap().fields[0].1.value.clone();
    h.app.preedit("世界".into(), Some((3, 6)));
    let composing = h.frame();
    h.dump("settings-preedit.png", &composing);
    h.app.key("Enter", false, false);
    assert_eq!(h.app.modal.as_ref().unwrap().fields[0].1.value, committed);
    h.app.input("世界");
    assert_eq!(h.app.modal.as_ref().unwrap().fields[0].1.value, content.replacen("\nab\n", "\n世界\n", 1));
    h.app.key("z", true, false);
    assert_eq!(h.app.modal.as_ref().unwrap().fields[0].1.value, content);
    h.app.key("End", true, false);
    h.frame();
    let field = h.app.hits.iter().find(|hit| matches!(hit.action, Action::Focus(Some(0)))).unwrap().rect;
    let pointer = Vec2::new(field.x + field.width * 0.5, field.y + field.height * 0.5);
    let transcript_scroll = h.app.scroll;
    h.app.wheel(-100_000., false, pointer);
    let scrolled = h.frame();
    assert_eq!(h.app.scroll, transcript_scroll, "wheel over settings is not transcript scrolling");
    assert_eq!(scrolled, h.frame(), "idle render must not undo manual field scrolling");
    h.dump("settings-scrolled.png", &scrolled);
    h.app.key("ArrowLeft", false, false);
    let following = h.frame();
    assert_ne!(scrolled, following, "navigation reveals the caret again");
}

#[test]
fn key_repeat_cost_reports_work_not_an_idle_polling_loop() {
    let mut h = Harness::new();
    h.app.input(&"repeat motion without storing a draft\n".repeat(80));
    h.frame();
    h.app.key("Home", true, false);
    sanscale::profiling::reset_work_counters();
    let mut samples = Vec::new();
    for i in 0..300 {
        h.app.dirty = false;
        let started = Instant::now();
        h.app.key(if i % 2 == 0 { "ArrowDown" } else { "ArrowUp" }, false, false);
        samples.push(started.elapsed().as_nanos());
        assert!(h.app.needs_redraw());
    }
    samples.sort_unstable();
    let work = sanscale::profiling::work_counters();
    assert_eq!((work.block_requests, work.shape_calls, work.flow_calls), (0, 0, 0));
    eprintln!("300 warm App keys (debug, no frame/OS repeat delay): p50={}ns p95={}ns; zero shape requests", samples[150], samples[284]);
    let started = Instant::now(); h.frame();
    eprintln!("one completed headless frame including readback: {:?}", started.elapsed());
    assert!(!h.app.tick(0.), "idle editor needs neither a timer nor continuous redraws");
}
