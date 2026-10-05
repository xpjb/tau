use super::*;
use chad::{Config, HeadlessCtx};
use std::sync::Arc;

struct Harness {
    app: App,
    ctx: HeadlessCtx,
    _root: tempfile::TempDir,
}
impl Harness {
    fn new(size: (u32, u32), scale: f32) -> Self {
        let root = tempfile::tempdir().unwrap();
        let ctx = HeadlessCtx::new(&Config { size, device_limits: crate::desktop::limits(), ..Default::default() })
            .unwrap();
        let mut app = App::new(&ctx, Store::open(root.path().into()).unwrap(), Arc::new(|| {}), true).unwrap();
        app.back();
        crate::demo::populate(&mut app.controller).unwrap();
        app.resize(size, scale, Vec2::new(0., 0.));
        app.tick(0.);
        app.root.workspace.show_chats = false;
        Self { app, ctx, _root: root }
    }
    fn frame(&mut self) {
        self.app.tick(0.);
        self.app.frame(&self.ctx, self.ctx.view());
    }
    fn tap(&mut self, field: Option<usize>) {
        let r = if let Some(d) = self.app.root.dialog.as_ref() {
            d.fields()[field.unwrap()].control.rect.unwrap()
        } else {
            self.app.root.workspace.chat.composer.field.control.rect.unwrap()
        };
        let p = Vec2::new(r.x + r.width / 2., r.y + r.height / 2.);
        self.app.press(1, p, true);
        self.app.release(1, p);
        self.frame();
    }
    fn edit(&mut self, state: &Input, text: &str) {
        let end = text.encode_utf16().count() as i32;
        self.app.native_edit(Edit {
            id: state.id,
            revision: state.revision,
            text: text.into(),
            start: end,
            end,
            composing_start: -1,
            composing_end: -1,
        });
    }
}
#[test]
fn mobile_types_and_sends_in_the_resized_chat_without_an_editor_screen() {
    for (size, scale) in [((360, 720), 1.), ((900, 1800), 2.5), ((720, 360), 1.)] {
        let mut h = Harness::new(size, scale);
        h.frame();
        assert!(h.app.native_input().is_none(), "opening a chat alone does not show a keyboard");
        h.tap(None);
        let input = h.app.native_input().unwrap();
        let text = "An inline draft with 😀 emoji and 世界\n".repeat(8);
        h.edit(&input, &text);
        h.frame();
        assert_eq!(h.app.controller.selected().unwrap().local.draft, text);
        assert!(h.app.root.dialog.is_none());
        let full_height = h.app.root.workspace.chat.transcript.scroll.rect.height;
        let reduced = (size.0, if size.1 > size.0 { (380. * scale) as u32 } else { 250 });
        h.app.resize(reduced, scale, Vec2::new(0., 0.));
        h.frame();
        let resized = h.app.native_input().unwrap();
        assert_eq!(input.id, resized.id, "keyboard insets must not replace the editor");
        assert_eq!(resized.text, text);
        assert!(h.app.root.workspace.chat.transcript.scroll.rect.height < full_height);
        assert!(resized.rect[1] + resized.rect[3] <= reduced.1 as f32);
        let send = h.app.root.workspace.chat.composer.controls.placed().find(|(a,_)| matches!(a, ui::composer::Choice::Send)).unwrap().1;
        assert!(send.y + send.height <= reduced.1 as f32, "Send stays above the IME");
        assert!(h.app.ime_rect().unwrap().y < reduced.1 as f32);
        if let Some(dir) = std::env::var_os("TAU_INLINE_INPUT_PREVIEW_DIR") {
            std::fs::create_dir_all(&dir).unwrap();
            image::save_buffer(
                PathBuf::from(dir).join(format!("{}x{}.png", size.0, size.1)),
                &h.ctx.read_rgba8().unwrap(),
                size.0,
                size.1,
                image::ColorType::Rgba8,
            )
            .unwrap();
        }
        // Exercise the real on-screen Send, with the last IME edit already drained.
        let p = Vec2::new(send.x + send.width / 2., send.y + send.height / 2.);
        h.app.press(2, p, true);
        h.app.release(2, p);
        h.frame();
        assert_eq!(h.app.controller.selected().unwrap().local.pending.last().unwrap().text, text);
        let cleared = h.app.native_input().unwrap();
        assert!(cleared.text.is_empty());
        assert_ne!(cleared.id, input.id);
        assert_eq!(cleared.request, input.request, "Send keeps, rather than reopens, the current keyboard");
        h.edit(&input, "late old draft");
        h.frame();
        assert!(
            h.app.root.workspace.chat.composer.field.editor.value.is_empty(),
            "late IME callbacks cannot resurrect a sent draft"
        );
        h.app.resize(size, scale, Vec2::new(0., 0.));
        h.frame();
        assert!(h.app.root.workspace.chat.transcript.scroll.rect.height > full_height);
    }
}
#[test]
fn native_edits_are_bound_to_the_tapped_field_and_do_not_echo_over_composition() {
    let mut h = Harness::new((360, 720), 1.);
    h.frame();
    h.tap(None);
    let composer = h.app.native_input().unwrap();
    h.edit(&composer, "a😀word");
    h.frame();
    let db = rusqlite::Connection::open(h.app.controller.store.root.join("client.sqlite3")).unwrap();
    db.execute_batch("CREATE TABLE edits (key TEXT); CREATE TRIGGER input_write AFTER UPDATE ON local WHEN NEW.key LIKE 'chat:%' BEGIN INSERT INTO edits VALUES (NEW.key); END;").unwrap();
    h.app.native_edit(Edit {
        id: composer.id,
        revision: composer.revision,
        text: "a😀word".into(),
        start: 3,
        end: 7,
        composing_start: 3,
        composing_end: 7,
    });
    h.frame();
    assert_eq!(h.app.root.workspace.chat.composer.field.editor.selected(), "word");
    assert!(h.app.root.workspace.chat.composer.field.editor.composing());
    let composing = h.app.native_input().unwrap();
    assert!(
        composing.id == composer.id && composing.revision == composer.revision,
        "selection geometry may move the toolbar, but native snapshots never reset the IME revision"
    );
    assert_eq!(db.query_row("SELECT count(*) FROM edits", [], |r| r.get::<_, u32>(0)).unwrap(), 0);
    h.app.open_ui(ui::DialogSpec::Connection).unwrap();
    h.frame();
    assert!(h.app.native_input().is_none());
    h.tap(Some(0));
    let url = h.app.native_input().unwrap();
    h.edit(&url, "http://localhost:1234");
    h.frame();
    h.tap(Some(1));
    let token = h.app.native_input().unwrap();
    assert!(token.secret && token.single_line);
    h.app.resize((360, 250), 1., Vec2::new(0., 0.));
    h.frame();
    let compact = h.app.native_input().unwrap();
    assert!(compact.rect[1] + compact.rect[3] <= 250., "the token stays inline above a tall IME too");
    let connect = h.app.root.dialog.as_ref().unwrap().button("Connect").unwrap();
    assert!(connect.y + connect.height <= 250.);
    h.edit(&url, "wrong field");
    h.edit(&composer, "wrong chat");
    assert_eq!(h.app.root.dialog.as_ref().unwrap().fields()[0].editor.value, "http://localhost:1234");
    assert!(h.app.root.dialog.as_ref().unwrap().fields()[1].editor.value.is_empty());
    h.app.back();
    h.frame();
    assert!(h.app.native_input().is_none());
    assert_eq!(h.app.controller.selected().unwrap().local.draft, "a😀word");
    h.app.with_ui(|root, cx| root.workspace.navigate_chat("three", cx)).unwrap();
    h.edit(&composer, "late after chat switch");
    h.frame();
    assert!(h.app.controller.selected().unwrap().local.draft.is_empty());
}
#[test]
fn composer_and_dialog_touch_momentum_update_after_release_without_draft_writes() {
    for dialog in [false, true] {
        let mut h = Harness::new((360, 720), 1.);
        if dialog { h.app.open_ui(ui::DialogSpec::Topic(ui::TopicEdit::New)).unwrap(); }
        h.frame();
        h.tap(dialog.then_some(1));
        let input = h.app.native_input().unwrap();
        let value = "A long field with a stable IME selection.\n".repeat(120);
        h.edit(&input, &value);
        h.frame();
        let draft = h.app.controller.selected().unwrap().local.draft.clone();
        let input = h.app.native_input().unwrap();
        let db = rusqlite::Connection::open(h.app.controller.store.root.join("client.sqlite3")).unwrap();
        db.execute_batch("CREATE TABLE momentum_writes (key TEXT); CREATE TRIGGER momentum_write AFTER UPDATE ON local WHEN NEW.key LIKE 'chat:%' BEGIN INSERT INTO momentum_writes VALUES (NEW.key); END;").unwrap();
        let start = Vec2::new(input.rect[0] + input.rect[2] / 2., input.rect[1] + input.rect[3] / 2.);
        h.app.press(81, start, true);
        let mut end = start;
        for i in 1..=3 {
            std::thread::sleep(std::time::Duration::from_millis(16));
            end.y = start.y + i as f32 * 24.;
            h.app.motion(81, end);
        }
        h.app.release(81, end);
        h.frame(); // dt=0 must preserve this field's momentum too.
        let before = h.ctx.read_rgba8().unwrap();
        assert!(h.app.tick(1. / 60.));
        h.app.frame(&h.ctx, h.ctx.view());
        assert_ne!(before, h.ctx.read_rgba8().unwrap(), "Uncaptured fields advance in their own mounted owner (dialog={dialog})");
        assert_eq!(h.app.native_input().unwrap().id, input.id);
        assert_eq!(h.app.controller.selected().unwrap().local.draft, draft);
        assert_eq!(db.query_row("SELECT count(*) FROM momentum_writes", [], |r| r.get::<_, u32>(0)).unwrap(), 0);
        h.app.cancel_pointer();
        h.frame();
        let cancelled = h.ctx.read_rgba8().unwrap();
        h.app.tick(0.2);
        h.app.frame(&h.ctx, h.ctx.view());
        assert_eq!(cancelled, h.ctx.read_rgba8().unwrap(), "Cancellation stops even a released field fling");
    }
}
#[test]
fn mobile_enter_scroll_and_long_press_edit_the_inline_field_not_the_transcript() {
    let mut h = Harness::new((360, 720), 1.);
    h.frame();
    h.tap(None);
    let input = h.app.native_input().unwrap();
    h.edit(&input, "one two three");
    h.frame();
    h.app.key("Enter", false, false);
    h.frame();
    assert_eq!(h.app.root.workspace.chat.composer.field.editor.value, "one two three\n");
    assert!(h.app.controller.selected().unwrap().local.pending.is_empty(), "Enter is a newline, not Send");
    let input = h.app.native_input().unwrap();
    h.edit(&input, &"one two three\n".repeat(40));
    h.frame();
    let original = h.app.root.workspace.chat.composer.field.editor.value.clone();
    let pixels = h.ctx.read_rgba8().unwrap();
    let transcript = h.app.root.workspace.chat.transcript.scroll.value;
    let field = h.app.native_input().unwrap().rect;
    let start = Vec2::new(field[0] + field[2] / 2., field[1] + 30.);
    let end = Vec2::new(start.x, start.y + 80.);
    h.app.press(3, start, true);
    h.app.motion(3, end);
    h.app.release(3, end);
    h.frame();
    assert_ne!(pixels, h.ctx.read_rgba8().unwrap(), "long drafts scroll inside their textarea");
    assert_eq!(h.app.root.workspace.chat.transcript.scroll.value, transcript);
    assert_eq!(h.app.root.workspace.chat.composer.field.editor.value, original);
    let scrolled = h.ctx.read_rgba8().unwrap();
    let duplicate = h.app.native_input().unwrap();
    h.edit(&duplicate, &original);
    h.frame();
    assert_eq!(
        scrolled,
        h.ctx.read_rgba8().unwrap(),
        "an unchanged IME snapshot cannot undo manual textarea scrolling"
    );
    h.app.key("Home", true, false);
    for _ in 0..5 {
        h.app.key("ArrowRight", false, false);
    }
    h.frame();
    let caret = h.app.ime_rect().unwrap();
    let point = Vec2::new(caret.x + 0.1, caret.y + caret.height / 2.);
    let stale = h.app.native_input().unwrap();
    h.app.press(4, point, true);
    h.app.ui.capture.as_mut().unwrap().started = Instant::now() - std::time::Duration::from_millis(600);
    h.app.release(4, point);
    h.frame();
    assert_eq!(h.app.root.workspace.chat.composer.field.editor.selected(), "two");
    assert!(
        h.app.actions().iter().any(|a| matches!(a, PlatformAction::InputMenu)),
        "long press requests normal clipboard actions"
    );
    h.edit(&stale, "late text before caret move");
    h.frame();
    assert_eq!(h.app.root.workspace.chat.composer.field.editor.value, original);
}
#[test]
fn holding_text_then_dragging_extends_selection_instead_of_scrolling() {
    let mut h = Harness::new((360,720), 1.);
    h.frame(); h.tap(None);
    let input = h.app.native_input().unwrap();
    h.edit(&input, "one two three"); h.frame();
    h.app.key("Home", true, false);
    for _ in 0..5 { h.app.key("ArrowRight", false, false); }
    h.frame();
    let caret = h.app.ime_rect().unwrap();
    let p = Vec2::new(caret.x, caret.y + caret.height / 2.);
    h.app.press(44, p, true);
    h.app.motion(44, Vec2::new(p.x + 1., p.y + 1.)); // Normal finger jitter is not a scroll.
    h.app.ui.capture.as_mut().unwrap().started = Instant::now() - std::time::Duration::from_millis(600);
    h.frame();
    assert_eq!(h.app.root.workspace.chat.composer.field.editor.selected(), "two",
        "The hold selects before finger-up, so the same gesture can extend it");
    assert!(h.app.ui.capture.unwrap().claimed);
    h.app.motion(44, Vec2::new(p.x + 200., p.y)); h.frame();
    h.app.release(44, Vec2::new(p.x + 200., p.y)); h.frame();
    assert_eq!(h.app.root.workspace.chat.composer.field.editor.selected(), "two three");
    assert_eq!(h.app.controller.selected().unwrap().local.draft, "one two three");
    assert!(h.app.actions().iter().any(|a| matches!(a, PlatformAction::InputMenu)));
}

fn select(h: &mut Harness, start: i32, end: i32) {
    let input = h.app.native_input().unwrap();
    h.app.native_edit(Edit { id: input.id, revision: input.revision, text: input.text,
        start, end, composing_start: -1, composing_end: -1 });
    h.frame();
}
#[test]
fn mobile_selection_handles_drag_both_ends_without_editing_or_echoing_old_ime_text() {
    for (size, scale) in [((360,720),1.), ((900,1800),2.5)] {
        for dialog in [false, true] {
            let mut h = Harness::new(size, scale);
            if dialog { h.app.open_ui(ui::DialogSpec::Connection).unwrap(); }
            h.frame(); h.tap(dialog.then_some(0));
            let input = h.app.native_input().unwrap();
            h.edit(&input, "one two three"); h.frame();
            select(&mut h, 0, 0); let first = h.app.ime_rect().unwrap();
            select(&mut h, 13, 13); let last = h.app.ime_rect().unwrap();
            select(&mut h, 4, 7);
            let stale = h.app.native_input().unwrap();
            let db = rusqlite::Connection::open(h.app.controller.store.root.join("client.sqlite3")).unwrap();
            db.execute_batch("CREATE TABLE edits (key TEXT); CREATE TRIGGER input_write AFTER UPDATE ON local WHEN NEW.key LIKE 'chat:%' BEGIN INSERT INTO edits VALUES (NEW.key); END;").unwrap();
            for (start, caret, selected) in [(true, first, "one two"), (false, last, "one two three")] {
                let handles = h.app.native_editor().unwrap().selection_handles(&h.app.services.renderer.text);
                assert_eq!(handles.len(), 2);
                let handle = handles.into_iter().find(|h| h.start == start).unwrap();
                let point = handle.center();
                let pixels = h.ctx.read_rgba8().unwrap();
                let at = ((point.y as u32 * size.0 + point.x as u32) * 4) as usize;
                assert_eq!(&pixels[at..at+3], &[0x67,0xd4,0xff], "Handles must actually be painted, not just hittable");
                let end = Vec2::new(point.x + caret.x - handle.caret.x,
                    point.y + caret.y + caret.height / 2. - handle.caret.y - handle.caret.height / 2.);
                h.app.press(45, point, true);
                assert!(h.app.ui.capture.unwrap().claimed, "A containing form must not steal a handle drag");
                h.app.motion(45, end); h.app.release(45, end); h.frame();
                assert_eq!(h.app.native_editor().unwrap().selected(), selected);
                h.edit(&stale, "late stale IME text"); h.frame();
                assert_eq!(h.app.native_editor().unwrap().value, "one two three");
            }
            assert_eq!(db.query_row("SELECT count(*) FROM edits", [], |r| r.get::<_,u32>(0)).unwrap(), 0);
            h.app.key("x", true, false); h.frame();
            assert!(h.app.native_editor().unwrap().value.is_empty());
            h.app.key("z", true, false); h.frame();
            assert_eq!(h.app.native_editor().unwrap().value, "one two three");
            if let Some(dir) = std::env::var_os("TAU_SELECTION_PREVIEW_DIR") {
                std::fs::create_dir_all(&dir).unwrap();
                image::save_buffer(PathBuf::from(dir).join(format!("handles-{}-{}.png", size.0, dialog)),
                    &h.ctx.read_rgba8().unwrap(), size.0, size.1, image::ColorType::Rgba8).unwrap();
            }
        }
    }
}
#[test]
fn touch_handle_autoscroll_extends_stationary_selection_and_cancel_stops_it() {
    let mut h = Harness::new((360,720), 1.);
    h.frame(); h.tap(None);
    let input = h.app.native_input().unwrap();
    let original = "one two three\n".repeat(40);
    h.edit(&input, &original); h.frame();
    select(&mut h, 0, 3);
    let transcript = h.app.root.workspace.chat.transcript.scroll.value;
    let handle = h.app.native_editor().unwrap().selection_handles(&h.app.services.renderer.text)
        .into_iter().find(|h| !h.start).unwrap();
    let point = handle.center();
    let field = h.app.native_input().unwrap().rect;
    let outside = Vec2::new(point.x, field[1] + field[3] + 50.);
    h.app.press(61, point, true); h.app.motion(61, outside); h.frame();
    let before = h.app.native_editor().unwrap().range().end;
    for _ in 0..20 { h.app.tick(0.05); h.app.frame(&h.ctx, h.ctx.view()); }
    assert!(h.app.native_editor().unwrap().range().end > before, "Stationary finger follows newly scrolled lines");
    assert_eq!(h.app.root.workspace.chat.transcript.scroll.value, transcript);
    assert_eq!(h.app.controller.selected().unwrap().local.draft, original);
    h.app.cancel_pointer();
    let selection = h.app.native_editor().unwrap().range();
    for _ in 0..4 { h.app.tick(0.05); h.app.frame(&h.ctx, h.ctx.view()); }
    assert_eq!(h.app.native_editor().unwrap().range(), selection);
    h.app.with_ui(|root, cx| root.workspace.navigate_chat("two", cx)).unwrap();
    h.app.motion(61, outside); h.app.release(61, outside); h.frame();
    assert!(h.app.native_input().is_none());
    assert!(h.app.controller.selected().unwrap().local.draft.is_empty());
}
