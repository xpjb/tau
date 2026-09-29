//! Same-window IME adapter. All input destinations are retained, source-bound fields.
use super::*;
use crate::mobile_input::{Edit, Input};
impl App {
    fn native_editor(&self) -> Option<&Editor> {
        if !self.ui.mobile || self.root.viewer.is_some() || self.root.menu.is_some() {
            return None;
        }
        let edit = self.ui.native.as_ref()?;
        if !self.root.owns(edit.target, &self.controller, &self.ui) { return None; }
        if !edit.matches(edit.token, &self.controller) || self.ui.focus != Some(edit.target) {
            return None;
        }
        let field = self.root.editor_ref(Some(edit.target))?;
        if !field.control.enabled
            || field.control.rect.is_none()
            || field.control.clip.width <= 0.
            || field.control.clip.height <= 0.
        {
            return None;
        }
        (field.editor.native_id() == edit.token).then_some(&field.editor)
    }
    pub fn native_input(&self) -> Option<Input> {
        let mut input = self.native_editor()?.native_input(self.ui.input_request)?;
        let field = self.root.editor_ref(Some(self.ui.native.as_ref()?.target))?;
        let rect = crate::render::intersect(field.control.rect?, field.control.clip);
        if rect.width <= 0. || rect.height <= 0. {
            return None;
        }
        input.rect = [rect.x, rect.y, rect.width, rect.height];
        Some(input)
    }
    pub fn native_edit(&mut self, edit: Edit) {
        if !self.native_editor().is_some_and(|e| e.accepts_native_edit(&edit)) {
            return;
        }
        let target = self.ui.native.as_ref().unwrap().target;
        let changed = self.root.editor(Some(target)).is_some_and(|f| f.editor.native_edit(edit));
        if changed {
            self.edited();
        }
        self.ui.dirty = true;
    }
}

#[cfg(test)]
mod tests {
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
            composing.same_configuration(&composer),
            "same-field native snapshots never restart or overwrite the IME"
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
}
