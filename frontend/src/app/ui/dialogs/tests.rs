use super::*;
use crate::app::{App, Action, PlatformAction};
use crate::store::Store;
use chad::{Config, HeadlessCtx};
use sanscale::Vec2;
use std::sync::Arc;

struct Harness { app: App, ctx: HeadlessCtx, _dir: tempfile::TempDir }
impl Harness {
    fn new(mobile: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let size = if mobile { (360, 740) } else { (1000, 800) };
        let ctx = HeadlessCtx::new(&Config { size, device_limits: crate::desktop::limits(), ..Default::default() }).unwrap();
        let mut app = App::new(&ctx, Store::open(dir.path().into()).unwrap(), Arc::new(|| {}), mobile).unwrap();
        app.back(); crate::demo::populate(&mut app.controller).unwrap();
        app.controller.account.projects.extend([Project { id: "first".into(), name: "First".into(), prompt: "First prompt".into(), revision: 1 },
            Project { id: "second".into(), name: "Second".into(), prompt: "Second prompt".into(), revision: 2 }]);
        app.resize(size, 1., Vec2::new(0., 0.)); app.tick(0.); app.root.legacy.show_chats = false;
        Self { app, ctx, _dir: dir }
    }
    fn frame(&mut self) { self.app.tick(0.); self.app.frame(&self.ctx, self.ctx.view()); }
    fn dialog(&self) -> &Dialog { self.app.root.dialog.as_ref().unwrap() }
    fn button(&self, label: &str) -> Vec2 { center(self.dialog().button(label).expect(label)) }
    fn click(&mut self, point: Vec2) {
        self.app.press(7, point, self.app.ui.mobile); self.app.release(7, point);
    }
    fn dump(&self, name: &str) {
        if let Some(dir) = std::env::var_os("TAU_RETAINED_UI_PREVIEW_DIR") {
            std::fs::create_dir_all(&dir).unwrap(); let (w, h) = self.ctx.size();
            image::save_buffer(std::path::PathBuf::from(dir).join(name), &self.ctx.read_rgba8().unwrap(), w, h, image::ColorType::Rgba8).unwrap();
        }
    }
}
fn center(rect: Rect) -> Vec2 { Vec2::new(rect.x + rect.width / 2., rect.y + rect.height / 2.) }

#[test]
fn forms_bind_focus_and_replacement_before_the_next_input_without_a_frame() {
    for mobile in [false, true] {
        let mut h = Harness::new(mobile);
        let draft = h.app.controller.selected().unwrap().local.draft.clone();
        h.app.apply(Action::NewProject).unwrap();
        h.app.input("New topic"); h.app.key("Tab", false, false); h.app.input("Exact\nprompt\n");
        assert_eq!(h.dialog().fields()[0].editor.value, "New topic");
        assert_eq!(h.dialog().fields()[1].editor.value, "Exact\nprompt\n");
        h.app.key("Tab", false, true); assert_eq!(h.app.ui.focus, Some(h.dialog().fields()[0].control.target));
        h.app.apply(Action::RenameProject("second".into())).unwrap(); h.app.input(" edited");
        assert_eq!(h.dialog().fields()[0].editor.value, "Second edited");
        assert_eq!(h.app.controller.selected().unwrap().local.draft, draft);
        h.frame();
        assert!(!h.app.test_hits().iter().any(|hit| matches!(hit.action, Action::Focus(Some(_)) | Action::Confirm | Action::CancelModal)),
            "Migrated fields/buttons never register legacy actions");
    }
}

#[test]
fn pointer_target_uses_current_coordinates_and_never_retargets_a_replacement() {
    let mut h = Harness::new(false); h.app.apply(Action::Settings).unwrap(); h.frame();
    h.app.hover(Some(h.button("Connect"))); // Deliberately hover a different control.
    h.click(h.button("Cancel")); assert!(h.app.root.dialog.is_none());
    h.app.apply(Action::Settings).unwrap(); h.frame();
    let r = h.dialog().button("Cancel").unwrap();
    h.app.press(8, Vec2::new(r.x + 0.1, r.y + 0.1), false); h.app.release(8, center(r));
    assert!(h.app.root.dialog.is_some(), "The rectangular bounding box is not the rounded hit shape");
    let p = h.button("Cancel"); h.app.press(9, p, false);
    let old = h.app.ui.capture.unwrap().target;
    h.app.apply(Action::RenameProject("first".into())).unwrap(); h.frame();
    assert_ne!(h.dialog().id(), old.scope); assert!(h.app.ui.capture.is_none());
    h.app.release(9, h.button("Cancel"));
    assert_eq!(h.dialog().topic_key(), Some(("rename", "first")), "Release is not a new press on the replacement");
    let p = h.button("Cancel"); h.app.press(10, p, false); h.app.motion(10, Vec2::new(p.x + 100., p.y)); h.app.motion(10, p); h.app.release(10, p);
    assert!(h.app.root.dialog.is_some(), "An abandoned drag cannot later turn back into a click");
}

#[test]
fn modal_scope_blocks_legacy_pointer_wheel_middle_and_context_routes() {
    let mut h = Harness::new(false); h.frame();
    let before = h.app.controller.selected().unwrap().local.draft.clone();
    h.app.apply(Action::ProjectPrompt("first".into())).unwrap(); h.frame();
    let point = Vec2::new(8., 300.);
    h.app.press(7, point, true); h.app.motion(7, Vec2::new(8., 150.)); h.app.release(7, point);
    h.app.wheel(300., false, point); h.app.middle(true, point); h.app.context_at(point);
    assert!(h.app.root.code.pointer.is_none()); assert!(h.app.root.legacy.wheel.is_none());
    assert!(h.app.root.legacy.autoscroll.is_none()); assert!(h.app.root.menu.is_none());
    assert_eq!(h.app.controller.selected().unwrap().local.draft, before);
    assert!(h.app.root.dialog.is_some());
}

#[test]
fn native_editor_and_clipboard_callbacks_cannot_write_a_new_field_or_source() {
    let mut h = Harness::new(true); h.app.apply(Action::NewProject).unwrap(); h.frame();
    h.click(center(h.dialog().fields()[0].control.rect.unwrap()));
    let token = h.app.native_input().unwrap();
    // IME insets and backgrounding cancel gestures, not the inline field binding.
    h.app.ui.window_focused = false; h.app.cancel_pointer(); h.app.resize((360, 740), 1., Vec2::new(0., 1.));
    h.app.native_edit(snapshot(&token, "Native topic")); assert_eq!(h.dialog().fields()[0].editor.value, "Native topic");
    h.app.apply(Action::RenameProject("second".into())).unwrap();
    h.app.native_edit(snapshot(&token, "Stale replacement")); assert_eq!(h.dialog().fields()[0].editor.value, "Second");
    h.app.apply(Action::NewProject).unwrap();
    h.app.key("v", true, false);
    let token = h.app.actions().into_iter().find_map(|action| match action { PlatformAction::Paste { token } => Some(token), _ => None }).unwrap();
    h.app.key("Tab", false, false); h.app.paste(token, "Wrong field".into());
    assert!(h.dialog().fields().iter().all(|f| f.editor.value.is_empty()));
    h.app.key("v", true, false);
    let token = h.app.actions().into_iter().find_map(|action| match action { PlatformAction::Paste { token } => Some(token), _ => None }).unwrap();
    h.app.paste(token, "Right prompt".into()); assert_eq!(h.dialog().fields()[1].editor.value, "Right prompt");
    h.frame(); h.click(center(h.dialog().fields()[0].control.rect.unwrap()));
    let token = h.app.native_input().unwrap();
    let draft = h.app.controller.selected().unwrap().local.draft.clone();
    h.app.controller.identity = "replacement-account".into();
    h.app.native_edit(snapshot(&token, "Wrong account")); assert!(h.dialog().fields()[0].editor.value.is_empty());
    h.app.input("still stale"); assert!(h.app.root.dialog.is_none());
    assert_eq!(h.app.controller.selected().unwrap().local.draft, draft);
}

#[test]
fn composition_consumes_enter_escape_and_focus_loss_cancels_capture() {
    let mut h = Harness::new(false); h.app.apply(Action::Settings).unwrap(); h.frame();
    let settings = h.app.controller.settings.server_url.clone();
    h.app.preedit("入力".into(), None); assert!(h.app.composing());
    h.app.key("Enter", false, false);
    assert!(h.app.composing()); assert_eq!(h.app.controller.settings.server_url, settings);
    assert!(matches!(h.dialog(), Dialog::Connection(ConnectionDialog { attempt: None, .. })));
    h.app.key("Escape", false, false); assert!(!h.app.composing()); assert!(h.app.root.dialog.is_some());
    h.app.key("ArrowLeft", false, false); assert!(h.app.root.dialog.is_some());
    h.app.key("Enter", false, true);
    assert!(matches!(h.dialog(), Dialog::Connection(ConnectionDialog { attempt: None, .. })), "Shift+Enter is not submission");
    h.app.press(13, h.button("Cancel"), false); assert!(h.app.ui.capture.is_some());
    h.app.cancel_pointer(); h.app.release(13, h.button("Cancel")); assert!(h.app.root.dialog.is_some());
    h.app.key("Escape", false, false); assert!(h.app.root.dialog.is_none());
}

#[test]
fn structural_requests_are_fifo_and_a_failed_or_stale_request_does_not_drop_the_tail() {
    let mut h = Harness::new(false); h.app.apply(Action::Settings).unwrap();
    let old = h.dialog().id();
    h.app.ui.requests.extend([Request::Open(DialogSpec::Topic(TopicEdit::New)), Request::Close(old),
        Request::Open(DialogSpec::Topic(TopicEdit::Prompt("second".into())))]);
    h.app.finish_ui_requests().unwrap(); assert_eq!(h.dialog().topic_key(), Some(("prompt", "second")));
    h.app.input(" updated"); assert_eq!(h.dialog().fields()[0].editor.value, "Second prompt updated");
    h.app.ui.requests.extend([Request::Open(DialogSpec::Topic(TopicEdit::Rename(GENERAL_PROJECT_ID.into()))),
        Request::Open(DialogSpec::Topic(TopicEdit::Rename("first".into())))]);
    assert!(h.app.finish_ui_requests().unwrap_err().to_string().contains("General cannot be renamed"));
    assert!(h.app.ui.requests.is_empty()); assert_eq!(h.dialog().topic_key(), Some(("rename", "first")));
    let owner = h.dialog().id(); h.app.ui.requests.push_back(Request::Close(owner));
    h.app.frame(&h.ctx, h.ctx.view()); assert!(h.app.root.dialog.is_none(), "The frame boundary also drains requests outside traversal");
}

#[test]
fn async_topic_results_match_request_scope_and_preserve_edits_after_failure() {
    let mut h = Harness::new(false); h.app.apply(Action::RenameProject("first".into())).unwrap(); h.app.input(" draft");
    // Exercise the UI completion boundary independently of transport delivery;
    // Controller's real request/persistence path has separate integration coverage.
    h.app.controller.epoch = Some(7);
    let Dialog::Topic(dialog) = h.app.root.dialog.as_mut().unwrap() else { unreachable!() };
    dialog.request = Some("operation-one".into());
    h.app.controller.project_result = Some(("other-operation".into(), true)); h.app.ui_event(Event::Tick(0.));
    assert!(matches!(h.dialog(), Dialog::Topic(TopicDialog { request: Some(_), .. })));
    h.app.controller.project_result = Some(("operation-one".into(), false)); h.app.ui_event(Event::Tick(0.));
    assert_eq!(h.dialog().fields()[0].editor.value, "First draft");
    assert!(matches!(h.dialog(), Dialog::Topic(TopicDialog { request: None, .. })));
    h.app.apply(Action::RenameProject("second".into())).unwrap();
    h.app.controller.project_result = Some(("operation-one".into(), true)); h.app.ui_event(Event::Tick(0.));
    assert_eq!(h.dialog().topic_key(), Some(("rename", "second")));
    let Dialog::Topic(dialog) = h.app.root.dialog.as_mut().unwrap() else { unreachable!() }; dialog.request = Some("operation-two".into());
    h.app.controller.project_result = Some(("operation-two".into(), true)); h.app.ui_event(Event::Tick(0.));
    assert!(h.app.root.dialog.is_none(), "Matching completion closes before a frame/next input");
}

#[test]
fn connection_completion_and_focus_restoration_validate_their_owner() {
    let mut h = Harness::new(false); h.app.ui.focus=Some(h.app.root.composer.field.control.target);
    h.app.apply(Action::Settings).unwrap();
    let Dialog::Connection(dialog) = h.app.root.dialog.as_mut().unwrap() else { unreachable!() }; dialog.attempt = Some(h.app.controller.identity.clone());
    h.app.apply(Action::RenameProject("first".into())).unwrap(); h.app.controller.epoch = Some(9); h.app.ui_event(Event::Tick(0.));
    assert_eq!(h.dialog().topic_key(), Some(("rename", "first")), "Old connection success cannot close a different dialog");
    h.app.back(); assert_eq!(h.app.ui.focus, Some(h.app.root.composer.field.control.target));
    h.app.apply(Action::Settings).unwrap(); h.app.ui_event(Event::Tick(0.)); assert!(h.app.root.dialog.is_some(), "Already connected is not a new submission completion");
    h.app.controller.epoch = None; h.app.controller.connection = "Authentication failed".into();
    let Dialog::Connection(dialog) = h.app.root.dialog.as_mut().unwrap() else { unreachable!() };
    dialog.attempt = Some(h.app.controller.identity.clone()); dialog.url.control.enabled = false; dialog.token.control.enabled = false;
    h.app.ui_event(Event::Tick(0.));
    assert!(h.dialog().fields().iter().all(|field| field.control.enabled), "Failure restores editable fields without dropping their draft");
    h.app.controller.epoch = Some(9);
    let Dialog::Connection(dialog) = h.app.root.dialog.as_mut().unwrap() else { unreachable!() }; dialog.attempt = Some(h.app.controller.identity.clone());
    h.app.ui_event(Event::Tick(0.)); assert!(h.app.root.dialog.is_none());
    h.app.apply(Action::RenameProject("first".into())).unwrap(); h.app.controller.account.selected = Some("two".into());
    h.app.back(); assert_ne!(h.app.ui.focus, Some(h.app.root.composer.field.control.target), "Do not restore an editor bound to another chat");
}

#[test]
fn retained_forms_settle_idle_and_reuse_real_editor_geometry_on_desktop_and_phone() {
    for mobile in [false, true] {
        let mut h = Harness::new(mobile); h.app.apply(Action::Settings).unwrap(); h.frame();
        h.dump(if mobile { "connection-phone.png" } else { "connection-desktop.png" });
        h.app.ui.dirty = false; h.app.ui_event(Event::Tick(0.)); assert!(!h.app.ui.dirty, "Idle UI update cannot itself require a paint");
        h.click(h.button("More…")); h.frame(); h.dump(if mobile { "connection-tools-phone.png" } else { "connection-tools-desktop.png" });
        h.click(h.button("Copy connection diagnostics")); assert!(h.app.actions().iter().any(|a| matches!(a, PlatformAction::Copy(text) if text.contains("Native:"))));
        h.click(h.button("Back")); h.frame(); assert!(h.dialog().fields()[0].control.rect.is_some());
        h.app.apply(Action::ProjectPrompt("first".into())).unwrap(); h.frame();
        h.dump(if mobile { "topic-prompt-phone.png" } else { "topic-prompt-desktop.png" });
        assert!(h.app.ime_rect().is_some());
        for (_, rect) in h.dialog().buttons() { assert!(rect.y >= 0. && rect.y + rect.height <= h.app.ui.size.1 as f32); }
    }
}

#[test]
fn shared_control_clip_governs_hover_press_and_paint_with_the_same_bounds() {
    use crate::app::ui::controls::Button;
    let mut h = Harness::new(false);
    let mut button = Button::new(Id::new(), "Clipped control");
    let mut layer = crate::render::Layer::default();
    let bounds = Rect::new(20., 20., 200., 40.); let clip = Rect::new(20., 20., 60., 40.);
    h.app.with_ui(|_, cx| button.visit_perframe(&mut Frame { layer: &mut layer, bounds, clip }, cx));
    assert!(layer.draws.iter().all(|draw| draw.clip.is_some_and(|r| r.x + r.width <= 80.)));
    h.app.with_ui(|_, cx| {
        assert!(!button.handle_event(&Event::Hover(Some(Vec2::new(150., 40.))), cx));
        assert!(!button.handle_event(&Event::Down { pointer: 2, point: Vec2::new(150., 40.), touch: false }, cx));
        assert!(cx.ui.capture.is_none());
        assert!(button.handle_event(&Event::Down { pointer: 2, point: Vec2::new(50., 40.), touch: false }, cx));
        assert!(button.handle_event(&Event::Up { pointer: 2, point: Vec2::new(150., 40.) }, cx));
        assert!(!button.control.take_click(), "Release outside the clip must not activate its hidden control");
    });
}

#[test]
fn native_legacy_composer_session_survives_blur_but_not_a_navigation_round_trip() {
    let mut h = Harness::new(true);
    h.frame(); h.app.apply(Action::Focus(None)).unwrap();
    let token = h.app.native_input().unwrap();
    h.app.cancel_pointer(); h.app.native_edit(snapshot(&token, "Owned native draft"));
    assert_eq!(h.app.controller.selected().unwrap().local.draft, "Owned native draft");
    h.app.controller.select("two").unwrap(); h.app.sync_navigation();
    h.app.controller.select("demo").unwrap(); h.app.sync_navigation();
    h.app.native_edit(snapshot(&token, "Stale after remount"));
    assert_eq!(h.app.controller.selected().unwrap().local.draft, "Owned native draft",
        "Equal account/chat strings do not revive a detached editing session");
}

fn snapshot(input: &crate::mobile_input::Input, text: &str) -> crate::mobile_input::Edit {
    crate::mobile_input::Edit { id: input.id, revision: input.revision, text: text.into(),
        start: text.encode_utf16().count() as i32, end: text.encode_utf16().count() as i32, composing_start: -1, composing_end: -1 }
}

#[test]
fn remaining_forms_own_fields_and_reject_detached_callbacks() {
    let mut h = Harness::new(true);
    h.app.open_ui(DialogSpec::Models).unwrap(); h.frame();
    let target = h.dialog().fields()[0].control.target;
    h.click(center(h.dialog().fields()[0].control.rect.unwrap()));
    let native = h.app.native_input().unwrap();
    h.app.open_ui(DialogSpec::Operation(super::super::Operation::Rename("demo".into()))).unwrap();
    assert_ne!(h.dialog().fields()[0].control.target, target);
    let title = h.dialog().fields()[0].editor.value.clone();
    h.app.native_edit(snapshot(&native, "unrelated models"));
    assert_eq!(h.dialog().fields()[0].editor.value, title);
    h.app.key("a", true, false); h.app.input("inline title");
    assert_eq!(h.dialog().fields()[0].editor.value, "inline title");
    h.app.controller.identity = "other source".into(); h.app.input(" late");
    assert!(h.app.root.dialog.is_none());
}

#[test]
fn daemon_completion_is_owned_by_the_submitting_instance() {
    let mut h = Harness::new(false); h.app.open_ui(DialogSpec::Daemon).unwrap();
    let Dialog::Daemon(first) = h.app.root.dialog.as_mut().unwrap() else { panic!() };
    let old = first.id;
    h.app.open_ui(DialogSpec::Daemon).unwrap();
    h.app.controller.settings_result = Some(("old-save".into(), true));
    h.app.controller.notice = None; h.app.tick(0.);
    assert_ne!(h.dialog().id(), old); assert!(h.app.controller.notice.is_none());
    assert_eq!(h.app.controller.settings_result.as_ref().map(|(id, _)| id.as_str()), Some("old-save"));
    h.app.ui.requests.push_back(Request::Close(old)); h.app.finish_ui_requests().unwrap();
    assert!(h.app.root.dialog.is_some());
}
