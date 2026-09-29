use super::*;
use crate::app::{App, PlatformAction, SavedAction, Store};
use chad::{Config, HeadlessCtx};
use sanscale::Vec2;
use std::sync::Arc;

struct Harness {
    app: App,
    ctx: HeadlessCtx,
    _dir: tempfile::TempDir,
}
impl Harness {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let ctx = HeadlessCtx::new(&Config {
            size: (360, 600),
            device_limits: crate::desktop::limits(),
            ..Default::default()
        })
        .unwrap();
        let mut app = App::new(&ctx, Store::open(dir.path().into()).unwrap(), Arc::new(|| {}), true).unwrap();
        app.back();
        crate::demo::populate(&mut app.controller).unwrap();
        app.tick(0.);
        app.root.workspace.show_chats = false;
        let template = app.controller.chats["demo"].feed.events.values().next().unwrap().clone();
        let path = dir.path().join("saved.txt");
        std::fs::write(&path, b"already saved").unwrap();
        let mut events = vec![];
        for order in 0..20 {
            let id = format!("entry-{order}");
            let mut event = template.clone();
            event.id = id.clone();
            event.entry_id = id.clone();
            event.order = order;
            event.text.clear();
            event.attachment = Some(ChatAttachment {
                source_path: None,
                kind: AttachmentKind::File,
                file_name: "saved.txt".into(),
                caption: None,
                size: Some(13),
            });
            app.controller
                .record_download(
                    &app.controller.identity.clone(),
                    "",
                    "demo",
                    &id,
                    crate::store::SavedDownload {
                        reference: path.to_string_lossy().into(),
                        location: "Downloads/Tau".into(),
                        mime_type: "text/plain".into(),
                    },
                )
                .unwrap();
            events.push(event);
        }
        app.controller
            .message(ServerMessage::TranscriptSnapshot {
                session_id: "demo".into(),
                snapshot: TranscriptSnapshot {
                    generation: "nested".into(),
                    sequence: 1,
                    events,
                    queue: QueueState::default(),
                    before: None,
                    delivered: vec![],
                },
            })
            .unwrap();
        app.root.workspace.attachments.show = true;
        let mut h = Self { app, ctx, _dir: dir };
        h.frame();
        h
    }
    fn frame(&mut self) {
        self.app.tick(0.);
        self.app.frame(&self.ctx, self.ctx.view());
    }
    fn open(&self) -> (super::super::Target, Rect) {
        self.app
            .root
            .workspace
            .attachments
            .cards
            .cards
            .values()
            .flat_map(|card| card.controls.items.iter())
            .find_map(|(_, b, a)| {
                matches!(a,CardChoice::UseSaved(_,entry,SavedAction::Open) if entry=="entry-19")
                    .then_some((b.control.target, b.control.rect.unwrap()))
            })
            .unwrap()
    }
}
#[test]
fn nested_attachment_button_promotes_touch_to_its_scroll_parent_without_activation() {
    let mut h = Harness::new();
    let (target, rect) = h.open();
    let p = Vec2::new(rect.x + rect.width / 2., rect.y + rect.height / 2.);
    h.app.press(10, p, true);
    assert_eq!(h.app.ui.capture.unwrap().target, target);
    h.app.motion(10, Vec2::new(p.x, p.y - 45.));
    assert_eq!(h.app.ui.capture.unwrap().target, h.app.root.workspace.attachments.scroll.target);
    assert!(h.app.root.workspace.attachments.scroll.value > 0.);
    assert_eq!(h.app.root.workspace.chat.transcript.scroll.value, 0.);
    h.app.release(10, p);
    assert!(h.app.actions().is_empty(), "Scroll takeover cancels the child activation even on a returning release");
    h.app.root.workspace.attachments.scroll.stop();
    h.app.root.workspace.attachments.scroll.value = 0.;
    h.frame();
    let (_, rect) = h.open();
    let p = Vec2::new(rect.x + rect.width / 2., rect.y + rect.height / 2.);
    h.app.press(11, p, true);
    h.app.release(11, p);
    assert!(
        matches!(&h.app.actions()[..],[PlatformAction::UseDownload(_,SavedAction::Open,target)] if target.entry=="entry-19")
    );
}
#[test]
fn nested_capture_is_clipped_and_cannot_activate_a_replaced_card_or_another_pointer() {
    let mut h = Harness::new();
    let (old, rect) = h.open();
    let p = Vec2::new(rect.x + rect.width / 2., rect.y + rect.height / 2.);
    h.app.press(21, p, true);
    h.app.release(22, p);
    assert!(h.app.actions().is_empty());
    assert_eq!(h.app.ui.capture.unwrap().pointer, 21);
    // Reconcile the captured node away between press and release.
    h.app.controller.chats.get_mut("demo").unwrap().feed.events.remove(&19);
    h.frame();
    assert!(h.app.ui.capture.is_none_or(|c| c.target != old));
    h.app.release(21, p);
    assert!(h.app.actions().is_empty());
    h.app.root.workspace.attachments.scroll.value = 60.;
    h.frame();
    let top = h.app.root.workspace.attachments.scroll.rect.y;
    let (_, b, _) = h
        .app
        .root
        .workspace
        .attachments
        .cards
        .cards
        .values()
        .flat_map(|card| card.controls.items.iter())
        .find(|(_, b, _)| b.control.rect.is_some_and(|r| r.y < top))
        .expect("partially clipped child");
    {
        let r = b.control.rect.unwrap();
        let p = Vec2::new(r.x + r.width / 2., top - 1.);
        assert!(!b.control.contains(p));
    }
    h.app.with_ui(|root, cx| root.workspace.attachments.handle_event(&Event::Cancel, cx));
    assert!(h.app.root.workspace.attachments.scroll.velocity == 0.);
}

#[test]
fn child_feedback_and_activation_do_not_belong_to_the_enclosing_message() {
    let mut h = Harness::new();
    h.app.root.workspace.attachments.show = false;
    h.frame();
    let row = h.app.root.workspace.chat.transcript.rows.iter()
        .find(|r| r.attachment.as_ref().is_some_and(|c| c.target.entry == "entry-19")).unwrap();
    let (_, button, _) = row.attachment.as_ref().unwrap().controls.items.iter()
        .find(|(_, _, a)| matches!(a, CardChoice::UseSaved(_, _, SavedAction::Open))).unwrap();
    let (parent, outer, target, inner) = (row.control.target, row.control.rect.unwrap(), button.control.target, button.control.rect.unwrap());
    let point = Vec2::new(inner.x + inner.width / 2., inner.y + inner.height / 2.);
    let before = h.ctx.read_rgba8().unwrap();
    h.app.hover(Some(point));
    assert_eq!(h.app.ui.hot.unwrap().0, target);
    h.frame();
    let after = h.ctx.read_rgba8().unwrap();
    let width = h.ctx.size().0 as usize;
    for (i, (old, new)) in before.chunks_exact(4).zip(after.chunks_exact(4)).enumerate() {
        let p = Vec2::new((i % width) as f32 + 0.5, (i / width) as f32 + 0.5);
        if contains(outer, p) && !contains(inner, p) { assert_eq!(old, new, "Child hover changed its parent"); }
    }
    h.app.press(1, point, true);
    assert_eq!(h.app.ui.capture.unwrap().target, target);
    assert!(h.app.root.workspace.chat.transcript.rows.iter().all(|r| r.control.ripple.is_none()));
    h.app.release(1, point);
    assert!(matches!(&h.app.actions()[..], [PlatformAction::UseDownload(_, SavedAction::Open, t)] if t.entry == "entry-19"));
    let blank = Vec2::new(outer.x + outer.width / 2., outer.y + 12.);
    h.app.press(2, blank, true);
    assert_eq!(h.app.ui.capture.unwrap().target, parent, "Blank parent space still owns its touch");
    h.app.cancel_pointer();
}
