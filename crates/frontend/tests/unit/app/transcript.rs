//! Real App navigation and viewport layout; isolated stores, no network.
use super::*;
use chad::{Config, HeadlessCtx};
use std::sync::Arc;

struct Harness { app: App, ctx: HeadlessCtx, _root: tempfile::TempDir }
impl Harness {
    fn new(mobile: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        let size = if mobile { (360, 720) } else { (1000, 700) };
        let ctx = HeadlessCtx::new(&Config { size, device_limits: crate::desktop::limits(), ..Default::default() }).unwrap();
        let mut app = App::new(&ctx, Store::open(root.path().into()).unwrap(), Arc::new(|| {}), mobile).unwrap();
        app.back();
        crate::demo::populate(&mut app.controller).unwrap();
        app.controller.account.projects.push(Project { id: "other".into(), name: "Other topic".into(), prompt: String::new(), revision: 1 });
        app.controller.account.sessions.retain(|s| s.id != "three");
        for session in &mut app.controller.account.sessions {
            session.status = SessionStatus::Idle;
            if session.id == "two" { session.project_id = "other".into(); }
        }
        let template = app.controller.chats["demo"].feed.events[&0].clone();
        for session in ["demo", "two"] {
            let events = (0..80).map(|order| {
                let mut e = template.clone();
                e.id = format!("event-{order}"); e.entry_id = e.id.clone(); e.order = order;
                e.phase = EventPhase::Saved; e.origin = Origin::default();
                e.role = if order % 4 == 0 { EventRole::User } else { EventRole::Assistant };
                e.kind = match order % 4 { 1 => EventKind::Thinking, 2 => EventKind::Tool, _ => EventKind::Text };
                e.text = match e.kind {
                    EventKind::Thinking => (0..24).map(|p| format!("{session} reasoning {order}, paragraph {p}: Read the implementation, preserve **existing behavior**, inspect viewport bounds and cached layout, and verify `source_identity` before updating the rendered document.\n\n")).collect(),
                    EventKind::Tool => (0..32).map(|line| format!("printf '{session}, tool {order}, line {line}: retained output must not be parsed again on every chat switch'\n")).collect(),
                    _ => format!("{session} message {order}: a short answer with **formatted text**."),
                };
                e.tool_name = (e.kind == EventKind::Tool).then(|| "bash".into());
                e
            }).collect();
            app.controller.preview(session, events, QueueState::default(), None).unwrap();
            let chat = app.controller.chats.get_mut(session).unwrap();
            chat.local.details_default = true;
            chat.local.position.follow = true;
            for order in (2..80).step_by(4) {
                chat.local.expansion.insert(format!("tool:event-{order}"), true);
                chat.local.expansion.insert(format!("tool:event-{order}:Input"), true);
            }
            app.controller.save_chat(session).unwrap();
        }
        app.resize(size, 1., Vec2::new(0., 0.));
        Self { app, ctx, _root: root }
    }
    fn frame(&mut self) { self.app.tick(0.); self.app.frame(&self.ctx, self.ctx.view()); }
    fn settle(&mut self) {
        for _ in 0..8 {
            let again = self.app.tick(0.);
            self.app.frame(&self.ctx, self.ctx.view());
            if !again && !self.app.needs_redraw() { return; }
        }
        panic!("transcript did not settle");
    }
    fn navigate(&mut self, session: &str, topic: bool) {
        self.app.with_ui(|root, cx| if topic {
            root.workspace.navigate_project(if session == "demo" { "general" } else { "other" }, cx)
        } else { root.workspace.navigate_chat(session, cx) }).unwrap();
        self.app.ui.focus = None;
    }
}

#[test]
fn cold_expanded_transcript_shapes_the_reading_window_not_all_loaded_details() {
    for mobile in [false, true] {
        let mut h = Harness::new(mobile);
        sanscale::profiling::reset_work_counters();
        h.frame();
        let work = sanscale::profiling::work_counters();
        assert!(work.shape_calls > 0);
        assert!(h.app.services.renderer.message_measurements < 20, "cold layout must not visit all 80 bodies");
        assert!(!h.app.services.renderer.messages.contains_key("demo/thinking:event-1"));
        assert!(h.app.services.renderer.messages.contains_key("demo/event-79"));
        assert!(h.app.services.renderer.messages.contains_key("demo/tool:event-78:Input:text"));
        assert_eq!(h.app.root.workspace.chat.transcript.scroll.value, h.app.root.workspace.chat.transcript.scroll.max);
        h.settle();
        assert!(!h.app.tick(0.));
    }
}

#[test]
fn returning_to_expanded_chats_and_topics_reuses_documents_and_shaping() {
    for (mobile, topic) in [(false, false), (false, true), (true, false)] {
        let mut h = Harness::new(mobile);
        h.settle();
        h.navigate("two", topic); h.settle();
        h.navigate("demo", topic); h.settle();
        let key = "demo/tool:event-78:Input:text";
        let identity = h.app.services.renderer.messages[key].doc.identity();
        let reference = h.ctx.read_rgba8().unwrap();
        for _ in 0..3 {
            h.navigate("two", topic); h.settle();
            assert_eq!(h.app.services.renderer.messages[key].doc.identity(), identity, "leaving a chat is not eviction");
            sanscale::profiling::reset_work_counters();
            h.navigate("demo", topic); h.settle();
            let work = sanscale::profiling::work_counters();
            assert_eq!((work.shape_calls, work.flow_calls, work.source_reads), (0, 0, 0), "warm navigation must not reshape conversation text: {work:?}");
            assert_eq!(h.app.services.renderer.messages[key].doc.identity(), identity);
            assert_eq!(h.ctx.read_rgba8().unwrap(), reference, "cached return must paint the same content and scroll position");
        }
    }
}

#[test]
fn returning_layout_revalidates_background_edits_even_with_a_reused_feed_revision() {
    let mut h = Harness::new(false); h.settle();
    let revision = h.app.controller.chats["demo"].feed.revision;
    h.navigate("two", false); h.settle();
    let mut events = h.app.controller.chats["demo"].feed.events.values().cloned().collect::<Vec<_>>();
    events.last_mut().unwrap().text = "A changed answer.\n\n".repeat(8);
    h.app.controller.preview("demo", events, Default::default(), None).unwrap();
    h.app.controller.chats.get_mut("demo").unwrap().feed.revision = revision;
    h.navigate("demo", false); h.settle();
    assert_eq!(h.app.services.renderer.messages["demo/event-79"].source, "A changed answer.\n\n".repeat(8));
    assert_eq!(h.app.root.workspace.chat.transcript.scroll.value, h.app.root.workspace.chat.transcript.scroll.max);
    assert!(!h.app.services.renderer.messages.contains_key("demo/thinking:event-1"));
}

#[test]
fn disclosure_collapse_is_not_document_eviction_and_source_changes_are() {
    let mut h = Harness::new(false); h.settle();
    let key = "demo/tool:event-78:Input:text";
    let identity = h.app.services.renderer.messages[key].doc.identity();
    h.app.with_ui(|root, cx| root.workspace.chat.transcript.toggle("details:event-77".into(), false, cx));
    h.settle();
    assert_eq!(h.app.services.renderer.messages[key].doc.identity(), identity);
    h.app.with_ui(|root, cx| root.workspace.chat.transcript.toggle("details:event-77".into(), true, cx));
    h.settle();
    assert_eq!(h.app.services.renderer.messages[key].doc.identity(), identity);
    h.navigate("two", false); h.settle();
    h.app.controller.account.source_lineage = Some("replacement-history".into());
    h.app.sync_navigation();
    assert!(h.app.services.renderer.messages.is_empty());
    h.navigate("demo", false);
    assert!(h.app.root.workspace.chat.transcript.placed.is_empty(), "another source cannot reuse the old height index");
    h.app.with_ui(|root, cx| root.workspace.chat.transcript.tail(cx));
    h.settle();
    assert_ne!(h.app.services.renderer.messages[key].doc.identity(), identity);
}

#[test]
fn cached_geometry_survives_document_eviction_and_reflows_after_width_change() {
    let mut h = Harness::new(false); h.settle();
    let key = "demo/tool:event-78:Input:text";
    let identity = h.app.services.renderer.messages[key].doc.identity();
    h.app.services.renderer.message_cache_budget(1, 1);
    h.navigate("two", false); h.settle();
    assert!(!h.app.services.renderer.messages.contains_key(key), "pressure may retire old documents");
    h.navigate("demo", false); h.settle();
    assert_ne!(h.app.services.renderer.messages[key].doc.identity(), identity);
    assert_eq!(h.app.root.workspace.chat.transcript.scroll.value, h.app.root.workspace.chat.transcript.scroll.max);
    h.navigate("two", false); h.settle();
    h.app.resize((820, 700), 1.25, Vec2::new(0., 0.));
    h.navigate("demo", false); h.settle();
    assert_eq!(h.app.root.workspace.chat.transcript.scroll.value, h.app.root.workspace.chat.transcript.scroll.max);
    assert!(!h.app.services.renderer.messages.contains_key("demo/thinking:event-1"));
    assert!(h.app.services.renderer.messages["demo/event-79"].view.height > 0.);
}

#[test]
fn cold_reading_anchor_is_measured_before_applying_its_deep_saved_offset() {
    for resident in [true, false] {
        let mut h = Harness::new(false);
        if !resident {
            h.app.controller.chats.get_mut("demo").unwrap().feed = crate::feed::Feed::default();
            h.app.controller.ensure_chat("demo").unwrap();
            assert!(h.app.controller.chats["demo"].feed.bodies["event-1"].missing());
        }
        h.app.controller.chats.get_mut("demo").unwrap().local.position = crate::store::Position {
            follow: false, key: Some("demo/thinking:event-1".into()), offset: 1200.,
        };
        h.settle();
        let t = &h.app.root.workspace.chat.transcript;
        let row = t.placed.iter().find(|p| p.key == "demo/thinking:event-1").unwrap();
        assert!(row.height > 1200.);
        assert!((t.scroll.value - row.top - 1200.).abs() < 0.1);
        assert!(h.app.services.renderer.messages.contains_key("demo/thinking:event-1"));
        assert!(!h.app.services.renderer.messages.contains_key("demo/thinking:event-73"));
        h.navigate("two", false); h.settle();
        h.navigate("demo", false); h.settle();
        let t = &h.app.root.workspace.chat.transcript;
        let row = t.placed.iter().find(|p| p.key == "demo/thinking:event-1").unwrap();
        assert!((t.scroll.value - row.top - 1200.).abs() < 0.1);
    }
}
