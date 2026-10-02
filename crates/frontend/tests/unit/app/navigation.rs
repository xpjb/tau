//! Native hit dispatch and measured navigation; isolated stores, no OS file apps.
use super::*;
use chad::{Config, HeadlessCtx};
use std::sync::Arc;

struct Harness { app: App, ctx: HeadlessCtx, _root: tempfile::TempDir }
impl Harness {
    fn new(size: (u32, u32), scale: f32, mobile: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        let ctx = HeadlessCtx::new(&Config { size, device_limits: crate::desktop::limits(), ..Default::default() }).unwrap();
        let mut app = App::new(&ctx, Store::open(root.path().into()).unwrap(), Arc::new(|| {}), mobile).unwrap();
        app.back(); crate::demo::populate(&mut app.controller).unwrap();
        app.controller.account.projects.push(Project { id: "files".into(), name: "Files".into(), prompt: String::new(), revision: 1 });
        app.controller.account.sessions[0].project_id = "files".into();
        let template = app.controller.chats["demo"].feed.events[&0].clone();
        for session in ["demo", "two"] {
            let events = (0..70).map(|order| {
                let mut event = template.clone();
                event.id = format!("{session}-event-{order}");
                event.entry_id = format!("entry-{order}"); // Intentionally NOT the event/display ID.
                event.order = order;
                event.role = EventRole::Assistant;
                event.phase = EventPhase::Saved;
                event.origin = Origin::default();
                event.text = format!("Message {order}\n\nA little surrounding history.");
                if session == "demo" && [20, 45].contains(&order) {
                    event.role = EventRole::Tool;
                    event.text = "Output before the download controls.\n\n".repeat(25);
                    event.attachment = Some(ChatAttachment { source_path: None, kind: AttachmentKind::File,
                        file_name: "same-name.zip".into(), caption: Some("Build output".into()), size: Some(12) });
                }
                event
            }).collect();
            app.controller.preview(&(session), events, QueueState::default(), None).unwrap();
        }
        app.resize(size, scale, Vec2::new(0., 0.)); app.tick(0.);
        Self { app, ctx, _root: root }
    }
    fn frame(&mut self) { self.app.tick(0.); self.app.frame(&self.ctx, self.ctx.view()); }
    fn complete(&mut self, entry: &str) {
        let path = self._root.path().join("same-name.zip");
        std::fs::write(&path, b"saved fixture").unwrap();
        self.app.begin_save("demo", entry, path.clone(), "same-name.zip".into());
        self.app.services.platform.clear(); // The platform has dispatched the save.
        self.app.complete_save(&Controller::download_key("demo", entry), Ok(crate::store::SavedDownload {
            reference: path.to_string_lossy().into(), location: "Downloads/Tau/same-name.zip".into(), mime_type: "application/zip".into() }));
    }
    fn click_notice(&mut self, close: bool) {
        self.frame();
        let rect = if close { self.app.root.notice.close.rect } else { self.app.root.notice.body.rect }.expect("notice action");
        let point = Vec2::new(rect.x + rect.width / 2., rect.y + rect.height / 2.);
        self.app.context_at(point);
        assert!(self.app.root.menu.is_none(), "Popup blocks the underlying context menu");
        self.app.press(90, point, self.app.ui.mobile);
        self.frame(); // A repaint before release must not let the release click through.
        self.app.release(90, point);
        self.frame();
    }
    fn assert_target_visible(&self, entry: &str) {
        assert_eq!(self.app.controller.account.selected.as_deref(), Some("demo"));
        assert!(!self.app.root.workspace.chat.transcript.locating_download());
        let rect = self.app.root.workspace.chat.transcript.rows.iter().filter_map(|r|r.attachment.as_ref()).filter(|c| c.target.session == "demo" && c.target.entry == entry).flat_map(|c| c.controls.items.iter()).find_map(|(_, b, choice)|
            matches!(choice, ui::CardChoice::UseSaved(SavedAction::Open)).then_some(b.control.rect).flatten()).expect("Target download's Open control is on screen");
        assert!(rect.y >= self.app.root.workspace.chat.transcript.scroll.rect.y && rect.y + rect.height <= self.app.root.workspace.chat.transcript.scroll.rect.y + self.app.root.workspace.chat.transcript.scroll.rect.height);
        assert!(!self.app.controller.chats["demo"].local.position.follow);
        assert!(self.app.services.platform.is_empty(), "Navigation never opens/exports/re-downloads the file");
        assert!(self.app.controller.selected().unwrap().local.pending.is_empty(), "No click-through to chat controls");
    }
}

#[test]
fn download_notice_selects_current_topic_chat_and_exact_widget_on_desktop_and_phone() {
    for (size, scale, mobile, name) in [((1000, 800), 1., false, "desktop"), ((360, 720), 1., true, "phone"), ((900, 1800), 2.5, true, "scaled-phone")] {
        let mut h = Harness::new(size, scale, mobile);
        h.frame();
        h.app.with_ui(|root, cx| root.workspace.navigate_chat("two", cx)).unwrap(); h.frame();
        h.app.controller.draft("Keep my other chat's draft".into()).unwrap();
        h.app.with_ui(|root,cx| {
            let transcript=&mut root.workspace.chat.transcript;
            transcript.scroll.set(transcript.scroll.max * 0.4);transcript.remember_scroll(cx);
        }); h.app.save().unwrap();
        let position = h.app.controller.chats["two"].local.position.clone();
        h.complete("entry-20");
        // Membership is resolved on click, not captured when the save finishes.
        h.app.controller.account.projects.push(Project { id: "moved".into(), name: "Moved files".into(), prompt: String::new(), revision: 1 });
        h.app.controller.account.sessions.iter_mut().find(|s| s.id == "demo").unwrap().project_id = "moved".into();
        h.app.root.workspace.attachments.show = true;
        h.app.open_ui(ui::DialogSpec::Operation(ui::Operation::Delete("two".into()))).unwrap(); h.frame();
        assert!(h.app.root.notice.body.rect.is_none(),
            "A download notification must not navigate away from an open form");
        assert!(h.app.controller.notice.as_ref().is_some_and(|n| n.download.is_some()));
        h.app.back();
        h.click_notice(false);
        h.assert_target_visible("entry-20");
        assert_eq!(h.app.controller.account.selected_project, "moved");
        assert!(!h.app.root.workspace.show_chats && !h.app.root.workspace.attachments.show && h.app.root.dialog.is_none() && h.app.root.viewer.is_none());
        assert!(h.app.ui.focus.is_none(), "Locating a widget must not pop up the keyboard");
        let saved = h.app.controller.store.load_chat(&h.app.controller.identity, "two").unwrap();
        assert_eq!(saved.draft, "Keep my other chat's draft");
        assert_eq!(saved.position.key, position.key); assert!((saved.position.offset - position.offset).abs() < 1.);
        if let Some(dir) = std::env::var_os("TAU_NAVIGATION_PREVIEW_DIR") {
            std::fs::create_dir_all(&dir).unwrap();
            image::save_buffer(PathBuf::from(dir).join(format!("download-navigation-{name}.png")), &h.ctx.read_rgba8().unwrap(), size.0, size.1, image::ColorType::Rgba8).unwrap();
        }
        // Reselecting this same chat still navigates, and identical names/locations
        // do not alias two different widgets.
        h.app.with_ui(|root, cx| root.workspace.chat.transcript.tail(cx)); h.frame();
        let measured = h.app.services.renderer.message_measurements;
        h.complete("entry-45"); h.click_notice(false); h.assert_target_visible("entry-45");
        assert!(h.app.services.renderer.message_measurements - measured < 35,
            "Same-chat navigation must retain layout, not remeasure all 70 messages");
    }
}

#[test]
fn download_destination_survives_empty_loading_and_multiple_older_pages() {
    let mut h = Harness::new((420, 780), 1., true);
    let events = h.app.controller.chats["demo"].feed.events.values().cloned().collect::<Vec<_>>();
    h.app.with_ui(|root, cx| root.workspace.navigate_chat("two", cx)).unwrap(); h.frame();
    h.app.controller.clear_replica().unwrap();
    h.complete("entry-20"); h.click_notice(false);
    assert!(h.app.root.workspace.chat.transcript.locating_download(), "An empty cache is not a deleted widget");
    let before = h.app.controller.chats["demo"].local.position.clone();
    h.app.controller.preview("demo", events[50..].to_vec(), QueueState::default(), Some(50)).unwrap();
    h.app.controller.epoch = Some(1); // Allow the existing near-edge paging gate.
    h.app.frame(&h.ctx, h.ctx.view());
    assert_eq!(h.app.root.workspace.chat.transcript.history_attempt.as_ref().unwrap().2, 50);
    assert!(h.app.root.workspace.chat.transcript.locating_download());
    assert_eq!(h.app.controller.chats["demo"].local.position.key, before.key);
    h.app.controller.preview("demo", events[30..].to_vec(), QueueState::default(), Some(30)).unwrap();
    h.app.frame(&h.ctx, h.ctx.view());
    assert_eq!(h.app.root.workspace.chat.transcript.history_attempt.as_ref().unwrap().2, 30);
    assert!(h.app.root.workspace.chat.transcript.locating_download());
    h.app.controller.preview("demo", events[0..].to_vec(), QueueState::default(), None).unwrap();
    h.app.controller.epoch = None;
    h.frame(); h.assert_target_visible("entry-20");

    // A deliberate scroll/new destination cancels pending navigation; a late
    // history page must never drag the user back after they have moved on.
    h.app.controller.clear_replica().unwrap();
    h.complete("entry-20"); h.click_notice(false);
    assert!(h.app.root.workspace.chat.transcript.locating_download());
    let rect=h.app.root.workspace.chat.transcript.scroll.rect;
    h.app.wheel(100.,false,Vec2::new(rect.x+rect.width/2.,rect.y+rect.height/2.));
    assert!(!h.app.root.workspace.chat.transcript.locating_download());
    h.complete("entry-20"); h.click_notice(false);
    h.app.with_ui(|root, cx| root.workspace.navigate_chat("two", cx)).unwrap();
    assert!(!h.app.root.workspace.chat.transcript.locating_download());
}

#[test]
fn dismiss_replacement_failure_and_stale_destinations_do_not_navigate() {
    let mut h = Harness::new((1000, 800), 1., false);
    h.app.with_ui(|root, cx| root.workspace.navigate_chat("two", cx)).unwrap(); h.frame();
    h.complete("entry-20"); h.click_notice(true);
    assert_eq!(h.app.controller.account.selected.as_deref(), Some("two"));
    assert!(h.app.controller.notice.is_none());
    h.complete("entry-20");
    h.app.controller.notice = Some("An unrelated error".into()); h.frame();
    assert!(h.app.controller.notice.as_ref().unwrap().download.is_none());
    h.click_notice(true);
    h.app.begin_save("demo", "entry-20", h._root.path().join("cached"), "same-name.zip".into());
    h.app.services.platform.clear();
    h.app.complete_save(&Controller::download_key("demo", "entry-20"), Err("Disk full".into()));
    assert!(h.app.controller.notice.as_ref().unwrap().download.is_none());

    let target = h.app.export_target("demo", "entry-20");
    for stale in [DownloadTarget { identity: "other-account".into(), ..target.clone() },
        DownloadTarget { lineage: "other-source".into(), ..target.clone() },
        DownloadTarget { session: "deleted-chat".into(), ..target.clone() }] {
        assert!(h.app.open_download_notice(stale).is_err());
        assert_eq!(h.app.controller.account.selected.as_deref(), Some("two"));
    }
    let events = h.app.controller.chats["demo"].feed.events.values().filter(|e| e.entry_id != "entry-20").cloned().collect();
    h.app.controller.preview("demo", events, Default::default(), None).unwrap();
    h.complete("entry-20"); h.click_notice(false);
    assert!(!h.app.root.workspace.chat.transcript.locating_download());
    assert_eq!(h.app.controller.notice.as_deref(), Some("The download widget is no longer available in this chat."));
}

#[test]
fn ordinary_chat_topic_and_new_chat_navigation_rebind_editor_before_next_input() {
    for mobile in [false, true] {
        let mut h = Harness::new(if mobile { (360, 720) } else { (1000, 800) }, 1., mobile);
        h.app.controller.draft("Draft from demo".into()).unwrap(); h.frame();
        h.app.controller.chats.get_mut("two").unwrap().local.draft = "Draft from two".into();
        h.app.with_ui(|root, cx| root.workspace.navigate_chat("two", cx)).unwrap();
        // Deliberately no tick/frame between navigation and the next input event.
        assert_eq!(h.app.root.workspace.chat.composer.field.editor.value, "Draft from two");
        assert!(h.app.root.workspace.chat.transcript.placed.is_empty());
        h.app.input("!");
        assert_eq!(h.app.controller.chats["demo"].local.draft, "Draft from demo");
        assert!(h.app.controller.chats["two"].local.draft.contains("Draft from two"));
        h.app.with_ui(|root, cx| root.workspace.navigate_project("files", cx)).unwrap();
        assert_eq!(h.app.root.workspace.chat.composer.field.editor.value, if mobile { "" } else { "Draft from demo" });
        h.app.with_ui(|root, cx| root.workspace.new_chat(cx)).unwrap();
        assert!(h.app.root.workspace.chat.composer.field.editor.value.is_empty());
        h.app.input("New chat only");
        assert_eq!(h.app.controller.selected().unwrap().local.draft, "New chat only");
        assert_eq!(h.app.controller.chats["demo"].local.draft, "Draft from demo");
    }
}

#[test]
fn account_change_with_same_chat_id_discards_old_layout_and_pending_navigation() {
    let mut h = Harness::new((1000, 800), 1., false); h.frame();
    h.app.with_ui(|root, cx| root.workspace.chat.transcript.jump_to_download("entry-20".into(), cx));
    h.app.controller.identity = "different-account".into();
    h.app.controller.chats.get_mut("demo").unwrap().local.draft = "Other account".into();
    h.app.sync_navigation();
    assert!(!h.app.root.workspace.chat.transcript.locating_download());
    assert!(h.app.root.workspace.chat.transcript.placed.is_empty());
    assert_eq!(h.app.root.workspace.chat.composer.field.editor.value, "Other account");
    assert!(h.app.controller.store.load_chat("different-account", "demo").unwrap().draft.is_empty(),
        "Reconciliation must not write an old account's layout into the new account");
}

#[test]
fn remote_browser_yields_to_chat_and_download_navigation_without_retargeting_drafts() {
    for (size,mobile) in [((1000,800),false),((360,720),true)] {
        let mut h=Harness::new(size,1.,mobile);
        h.app.controller.draft("Keep this code comment draft".into()).unwrap();h.frame();
        h.app.with_ui(|root, cx| root.workspace.files(cx)).unwrap();h.frame();assert!(h.app.root.workspace.chat.code.view.is_some());
        let browser_generation=h.app.controller.viewer_generation();
        h.app.with_ui(|root, cx| root.workspace.navigate_chat("two", cx)).unwrap();
        assert!(h.app.root.workspace.chat.code.view.is_none(),"Chat selection cancels the browser before the next input event");
        assert!(h.app.controller.viewer_generation()>browser_generation);
        h.app.input("New chat text");
        assert_eq!(h.app.controller.chats["demo"].local.draft,"Keep this code comment draft");
        assert!(h.app.controller.chats["two"].local.draft.contains("New chat text"));
        assert!(!h.app.controller.chats["two"].local.draft.contains("code comment"));
        for session in ["two","demo"] {
            h.app.with_ui(|root, cx| root.workspace.navigate_chat(session, cx)).unwrap();h.frame();
            h.app.with_ui(|root, cx| root.workspace.files(cx)).unwrap();h.frame();assert!(h.app.root.workspace.chat.code.view.is_some());
            h.complete("entry-20");h.click_notice(false);
            assert!(h.app.root.workspace.chat.code.view.is_none(),"Same-chat and cross-chat notices must reveal the transcript, not the browser");
            h.assert_target_visible("entry-20");
            assert_eq!(h.app.controller.chats["demo"].local.draft,"Keep this code comment draft");
        }
    }
}


#[test]
fn reading_checkpoint_survives_cold_pages_prepend_resize_and_a_late_release() {
    let mut h = Harness::new((1000, 800), 1., false);
    let mut events = h.app.controller.chats["demo"].feed.events.values().cloned().collect::<Vec<_>>();
    events[30].kind = EventKind::Thinking;
    events[30].text = "A growing thought.\n\n".repeat(40);
    h.app.controller.preview("demo", events[10..].to_vec(), QueueState::default(), Some(10)).unwrap();
    h.frame();
    h.app.with_ui(|root, cx| {
        let transcript = &mut root.workspace.chat.transcript;
        transcript.scroll.set(transcript.scroll.max * 0.4);
        transcript.remember_scroll(cx);
    });
    h.frame(); h.app.save().unwrap();
    let saved = h.app.controller.chats["demo"].local.position.clone();
    let root = h.app.root.workspace.chat.transcript.rows.iter().find(|r| Some(&r.key) == saved.key.as_ref()).unwrap()
        .item.root(&h.app.controller.chats["demo"]).unwrap();
    let anchor = events.iter().position(|event| event.id == root).unwrap();
    let r = h.app.root.workspace.chat.transcript.scroll.rect;
    let point = Vec2::new(r.x + r.width / 2., r.y + r.height / 2.);
    h.app.press(73, point, false);
    h.app.controller.select("two").unwrap();
    h.app.release(73, point); // Navigation/source checks must run before the old release.
    assert!(h.app.controller.chats["two"].local.position.follow);
    h.app.controller.clear_replica().unwrap();
    h.app.with_ui(|root, cx| root.workspace.navigate_chat("demo", cx)).unwrap();
    h.frame(); h.app.save().unwrap();
    for (window, before) in [(&events[50..], Some(50)), (&events[anchor..=anchor], Some(anchor as u64)),
        (&events[10..], Some(10))] {
        h.app.controller.preview("demo", window.to_vec(), QueueState::default(), before).unwrap();
        h.frame(); h.app.save().unwrap();
        let checkpoint = &h.app.controller.chats["demo"].local.position;
        assert_eq!(checkpoint.key, saved.key, "An interim page cannot replace a saved anchor");
        assert!((checkpoint.offset - saved.offset).abs() < 0.1);
        assert!(!checkpoint.follow);
    }
    h.app.controller.preview("demo", events.clone(), QueueState::default(), None).unwrap();
    h.frame();
    // Saving between a DPI change and reflow must use the old measured geometry's scale.
    h.app.resize((1000, 800), 1.25, Vec2::new(0., 0.)); h.app.save().unwrap();
    assert!((h.app.controller.chats["demo"].local.position.offset - saved.offset).abs() < 0.1);
    h.frame(); h.app.save().unwrap();
    let t = &h.app.root.workspace.chat.transcript;
    let row = t.placed.iter().find(|p| Some(&p.key) == saved.key.as_ref()).unwrap();
    assert!((t.scroll.value - row.top - saved.offset * 1.25).abs() < 0.1);
    let checkpoint = h.app.controller.store.load_chat(&h.app.controller.identity, "demo").unwrap().position;
    assert_eq!(checkpoint.key, saved.key);
    assert!((checkpoint.offset - saved.offset).abs() < 0.1);

    // An expansion keeps its actual heading pinned. Checkpointing must not
    // replace that live anchor with the first visible row's saved preference.
    let heading = |app: &App| {
        let t = &app.root.workspace.chat.transcript;
        t.placed.iter().find(|p| p.key == "demo/details:demo-event-30").unwrap().top - t.scroll.value
    };
    h.app.with_ui(|root, cx| {
        let t = &mut root.workspace.chat.transcript;
        let top = t.placed.iter().find(|p| p.key == "demo/details:demo-event-30").unwrap().top;
        t.scroll.set(top - 100.); t.remember_scroll(cx);
    });
    h.frame(); let pinned = heading(&h.app);
    h.app.with_ui(|root, cx| root.workspace.chat.transcript.toggle("details:demo-event-30".into(), true, cx));
    h.frame(); h.app.save().unwrap();
    events[29].text.push_str(&"New streamed text before the heading.\n\n".repeat(30));
    h.app.controller.preview("demo", events.clone(), QueueState::default(), None).unwrap();
    h.frame(); h.app.save().unwrap();
    assert!((heading(&h.app) - pinned).abs() < 0.1);
    assert!(!h.app.controller.chats["demo"].local.position.follow);
    events[30].kind = EventKind::Text; // The pinned Details control disappears.
    h.app.controller.preview("demo", events[10..].to_vec(), QueueState::default(), Some(10)).unwrap();
    h.frame();
    assert!(h.app.root.workspace.chat.transcript.scroll.value > 0., "A dead control is not older history to seek");
}
