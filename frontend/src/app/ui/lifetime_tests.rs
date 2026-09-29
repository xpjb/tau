use super::*;
use crate::app::{App, Store};
use chad::{Config, HeadlessCtx};
use sanscale::{Rect, Vec2};
use std::sync::Arc;

struct Harness {
    app: App,
    ctx: HeadlessCtx,
    _dir: tempfile::TempDir,
}
impl Harness {
    fn new(mobile: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let size = if mobile { (360, 720) } else { (1000, 800) };
        let ctx =
            HeadlessCtx::new(&Config { size, device_limits: crate::desktop::limits(), ..Default::default() }).unwrap();
        let mut app = App::new(&ctx, Store::open(dir.path().into()).unwrap(), Arc::new(|| {}), mobile).unwrap();
        app.back();
        crate::demo::populate(&mut app.controller).unwrap();
        app.tick(0.);
        app.root.workspace.show_chats = false;
        Self { app, ctx, _dir: dir }
    }
    fn frame(&mut self) {
        self.app.tick(0.);
        self.app.frame(&self.ctx, self.ctx.view());
    }
}
fn center(r: Rect) -> Vec2 {
    Vec2::new(r.x + r.width / 2., r.y + r.height / 2.)
}

#[test]
fn retained_rows_keep_identity_on_streaming_and_reorder_but_not_source_replacement() {
    let mut h = Harness::new(false);
    let template = h.app.controller.chats["demo"].feed.events.values().next().unwrap().clone();
    let chat = h.app.controller.chats.get_mut("demo").unwrap();
    chat.feed.events.clear();
    for order in 0..120 {
        let mut event = template.clone();
        event.id = format!("row-{order}");
        event.entry_id = event.id.clone();
        event.order = order;
        event.text = format!("Message {order}");
        chat.feed.events.insert(order, event);
    }
    h.frame();
    let row =
        h.app.root.workspace.chat.transcript.rows.iter().find(|r| r.row.block.as_deref() == Some("row-119")).unwrap();
    let target = row.control.target;
    assert!(h.app.root.workspace.chat.transcript.rows.len() < 40, "Only overscan rows own interaction widgets");
    let chat = h.app.controller.chats.get_mut("demo").unwrap();
    chat.feed.events.get_mut(&119).unwrap().text.push_str(" streamed");
    let mut reordered = chat.feed.events.remove(&119).unwrap();
    reordered.order = 118;
    chat.feed.events.insert(118, reordered);
    h.frame();
    let row =
        h.app.root.workspace.chat.transcript.rows.iter().find(|r| r.row.block.as_deref() == Some("row-119")).unwrap();
    assert_eq!(row.control.target, target);
    let p = center(crate::render::intersect(row.control.rect.unwrap(), row.control.clip));
    h.app.press(1, p, true);
    assert_eq!(h.app.ui.capture.unwrap().target, target);
    assert_eq!(
        h.app.ui.capture_route,
        vec![
            h.app.root.id,
            h.app.root.workspace.id,
            h.app.root.workspace.chat.id,
            h.app.root.workspace.chat.transcript.scroll.target.scope,
            target.scope
        ]
    );
    h.app.controller.account.source_lineage = Some("replacement-history".into());
    h.app.release(1, p); // no frame can mediate the source fence
    assert!(h.app.ui.capture.is_none());
    assert!(h.app.root.menu.is_none());
    h.frame();
    let replacement =
        h.app.root.workspace.chat.transcript.rows.iter().find(|r| r.row.block.as_deref() == Some("row-119")).unwrap();
    assert_ne!(replacement.control.target, target);
}

#[test]
fn hidden_transcript_cannot_receive_release_or_context_before_another_frame() {
    let mut h = Harness::new(false);
    h.frame();
    let row = h.app.root.workspace.chat.transcript.rows.iter().find(|r| r.control.rect.is_some()).unwrap();
    let p = center(crate::render::intersect(row.control.rect.unwrap(), row.control.clip));
    h.app.press(4, p, true);
    assert!(h.app.ui.capture.is_some());
    h.app.key("Space", true, false);
    assert!(h.app.root.workspace.chat.code.view.as_ref().unwrap().search.is_some());
    assert!(h.app.ui.capture.is_none());
    h.app.release(4, p);
    h.app.context_at(p);
    assert!(h.app.root.menu.is_none(), "Old transcript geometry is not an active scope");
    h.app.input("src");
    assert_eq!(h.app.root.workspace.chat.code.view.as_ref().unwrap().search.as_ref().unwrap().editor.value, "src");
}

#[test]
fn code_search_inline_session_survives_insets_but_not_detachment() {
    let mut h = Harness::new(true);
    h.frame();
    h.app.key("Space", true, false);
    h.frame();
    let input = h.app.native_input().unwrap();
    h.app.resize((360, 360), 1., Vec2::new(0., 0.));
    h.frame();
    let resized = h.app.native_input().unwrap();
    assert_eq!(input.id, resized.id);
    let edit = || crate::mobile_input::Edit {
        id: input.id,
        revision: input.revision,
        text: "main.rs".into(),
        start: 7,
        end: 7,
        composing_start: -1,
        composing_end: -1,
    };
    h.app.native_edit(edit());
    assert_eq!(h.app.root.workspace.chat.code.view.as_ref().unwrap().search.as_ref().unwrap().editor.value, "main.rs");
    h.app.back();
    h.app.native_edit(edit());
    assert!(h.app.native_input().is_none());
    assert!(h.app.controller.selected().unwrap().local.draft.is_empty());
}

#[test]
fn claimed_text_selection_is_not_taken_by_its_scroll_parent() {
    let mut h = Harness::new(false);
    h.app.with_ui(|_, cx| {
        let mut field =
            controls::TextField::new(Id::new(), "Text", crate::editor::Editor::new("select these words".into()));
        let mut scroll = scroll::ScrollState::new(Id::new(), false);
        scroll.rect = Rect::new(0., 0., 300., 300.);
        scroll.max = 1000.;
        let mut layer = crate::render::Layer::new(crate::render::Interaction::default());
        field.visit_perframe(
            &mut Frame { layer: &mut layer, bounds: Rect::new(10., 10., 250., 100.), clip: scroll.rect },
            cx,
        );
        for event in [
            Event::Down { pointer: 8, point: Vec2::new(30., 40.), touch: false },
            Event::Move { pointer: 8, point: Vec2::new(120., 10.) },
        ] {
            let child = field.handle_event(&event, cx);
            scroll.event(&event, child, cx);
        }
        assert_eq!(cx.ui.capture.unwrap().target, field.control.target);
        assert!(cx.ui.capture.unwrap().claimed);
        assert_eq!(scroll.value, 0.);
    });
}

#[test]
fn ancestor_detach_does_not_apply_a_stale_path_to_a_replacement_focus() {
    let mut ui = UiState::new((100, 100), false);
    let root = Id::new();
    let parent = Id::new();
    let child = Id::new();
    let target = Target { scope: child, widget: Id::new() };
    ui.focus = Some(target);
    ui.focus_route = vec![root, parent, child];
    ui.detach(parent);
    assert!(ui.focus.is_none());
    ui.focus_route = vec![root, parent, child];
    let replacement = Target { scope: Id::new(), widget: Id::new() };
    ui.focus = Some(replacement);
    ui.detach(parent);
    assert_eq!(ui.focus, Some(replacement));
}

#[test]
fn operation_validation_feedback_is_painted_inside_the_opaque_form() {
    let mut h = Harness::new(false);
    h.app.open_ui(DialogSpec::Operation(Operation::Refresh)).unwrap();
    h.frame();
    h.app.key("a", true, false);
    h.app.key("Backspace", false, false);
    h.frame();
    let before = h.ctx.read_rgba8().unwrap();
    h.app.ui_event(Event::Submit);
    h.frame();
    assert!(h.app.root.dialog.is_some());
    assert!(h.app.controller.notice.as_ref().unwrap().contains("provider name"));
    assert!(h.app.root.notice.body.rect.is_none(), "No click-through popup under the form");
    assert_ne!(before, h.ctx.read_rgba8().unwrap(), "Validation is visibly painted, not just stored");
}

#[test]
fn captured_transcript_release_crosses_a_sibling_attachment_backdrop() {
    let mut h = Harness::new(false);
    h.app.root.workspace.attachments.show = true;
    h.frame();
    let row = h
        .app
        .root
        .workspace
        .chat
        .transcript
        .rows
        .iter()
        .find(|r| r.row.details.is_empty() && r.control.rect.is_some())
        .unwrap();
    let point = center(crate::render::intersect(row.control.rect.unwrap(), row.control.clip));
    h.app.press(30, point, false);
    assert!(h.app.root.workspace.chat.transcript.selecting);
    let other = center(h.app.root.workspace.attachments.scroll.rect);
    h.app.motion(30, other);
    h.app.release(31, other);
    assert_eq!(h.app.ui.capture.unwrap().pointer, 30, "Another contact cannot end the gesture");
    h.app.release(30, other);
    assert!(!h.app.root.workspace.chat.transcript.selecting, "The owning leaf receives release outside its pane");
    assert!(h.app.ui.capture.is_none());
}

#[test]
fn a_download_notice_waits_for_the_form_without_an_expired_wake_loop() {
    let mut h = Harness::new(false);
    h.frame();
    let notice = crate::notice::Notice::download("Saved download".into(), h.app.export_target("demo", "entry-1"));
    h.app.controller.notice = Some(notice.clone());
    h.frame();
    assert!(h.app.root.notice.popup.visible());
    h.app.open_ui(DialogSpec::Topic(TopicEdit::New)).unwrap();
    h.frame();
    h.app.input("Unsaved topic");
    assert!(
        h.app.root.notice.popup.remaining(std::time::Instant::now() + std::time::Duration::from_secs(60)).is_none()
    );
    assert_eq!(h.app.controller.notice, Some(notice.clone()));
    h.app.back();
    h.frame();
    assert!(h.app.root.notice.popup.visible(), "The destination can still be followed when the form closes");
    assert_eq!(h.app.controller.notice, Some(notice));
}

#[test]
fn opaque_dialog_covers_the_workspace_not_just_its_shapes() {
    for mobile in [false, true] {
        let mut h = Harness::new(mobile);
        h.app.root.workspace.show_chats = true;
        h.frame();
        let rect = h.app.root.workspace.sidebar.controls.items.iter()
            .find(|(_, _, choice)| matches!(choice, super::sidebar::Choice::Settings))
            .unwrap().1.control.rect.unwrap();
        let point = center(rect);
        h.app.press(1, point, mobile); h.app.release(1, point);
        h.frame();
        assert!(matches!(h.app.root.dialog, Some(Dialog::Connection(_))));
        let actual = h.ctx.read_rgba8().unwrap();
        // An opaque dialog must look identical with or without the workspace's
        // text, icons and shapes beneath it. No sidebar coordinates or glyph pixels.
        let bounds = Rect::new(0., 0., h.ctx.size().0 as f32, h.ctx.size().1 as f32);
        let mut layer = crate::render::Layer::default();
        h.app.with_ui(|root, cx| root.dialog.as_mut().unwrap().visit_perframe(
            &mut Frame { layer: &mut layer, bounds, clip: bounds }, cx));
        h.app.services.renderer.draw(&h.ctx, h.ctx.view(), &[layer]);
        assert!(actual == h.ctx.read_rgba8().unwrap(), "Opaque dialog leaked lower content (mobile={mobile})");
        if let Some(dir) = std::env::var_os("TAU_RETAINED_UI_PREVIEW_DIR") {
            std::fs::create_dir_all(&dir).unwrap();
            image::save_buffer(std::path::PathBuf::from(dir).join(if mobile { "opaque-phone.png" } else { "opaque-desktop.png" }),
                &actual, h.ctx.size().0, h.ctx.size().1, image::ColorType::Rgba8).unwrap();
        }
    }
}

#[test]
fn ordered_surfaces_keep_alpha_and_inherited_clips() {
    use crate::{icons::Icon, render::{Layer, color, contains}};
    let h = &mut Harness::new(false);
    let renderer = &mut h.app.services.renderer;
    let bounds = Rect::new(20., 20., 160., 80.);
    let clip = Rect::new(bounds.x, bounds.y, bounds.width / 2., bounds.height);
    let mut layer = Layer::default();
    layer.rect(bounds, color(0xff0000));
    renderer.label(&mut layer, "Lower text", bounds, 20., color(0xffffff), false);
    renderer.icon(&h.ctx, &mut layer, Icon::Gear,
        Rect::new(bounds.x + 10., bounds.y + 35., 32., 32.), 0xffffff);
    renderer.draw(&h.ctx, h.ctx.view(), std::slice::from_ref(&layer));
    let before = h.ctx.read_rgba8().unwrap();
    layer.with_clip(clip, |layer| {
        layer.above();
        layer.rect(bounds, color(0x0000ff));
        layer.above();
        layer.rect(bounds, sanscale::Color([1., 0., 0., 0.5]));
    });
    renderer.draw(&h.ctx, h.ctx.view(), &[layer]);
    let after = h.ctx.read_rgba8().unwrap();
    let width = h.ctx.size().0 as usize;
    let sample = &after[((clip.y as usize + 1) * width + clip.x as usize + 1) * 4..][..4];
    assert!(sample[0] > 0 && sample[2] > 0 && sample[1] == 0, "alpha keeps both upper red and lower blue");
    for (i, (old, new)) in before.chunks_exact(4).zip(after.chunks_exact(4)).enumerate() {
        let inside = contains(clip, Vec2::new((i % width) as f32 + 0.5, (i / width) as f32 + 0.5));
        assert_eq!(new, if inside { sample } else { old }, "surface order or inherited clip at pixel {i}");
    }
}
