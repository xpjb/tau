//! The only boundary allowed to replace an executing retained subtree. Requests
//! are drained after its event/visit borrow, never postponed until the next paint.
use super::*;

impl App {
    pub(super) fn with_ui<R>(&mut self, run: impl FnOnce(&mut ui::RootWidget, &mut ui::Context<'_>) -> R) -> R {
        let Self { controller, root, ui, services } = self;
        run(root, &mut ui::Context { model: controller, ui, services })
    }
    pub(super) fn ui_event(&mut self, event: ui::Event<'_>) -> bool {
        let old_focus = self.ui.focus;
        let old_hot = self.ui.hot;
        if let ui::Event::Hover(point) = event { self.ui.hover = point; self.ui.hot = None; self.ui.hint = None; }
        let handled = self.with_ui(|root, cx| root.handle_event(&event, cx));
        if old_focus != self.ui.focus {
            if let Some(field) = self.root.editor(old_focus) { field.editor.preedit(String::new(), None); }
            if !self.ui.native.as_ref().is_some_and(|input| matches!(input.target, ui::EditorTarget::Widget(target) if Some(target) == self.ui.focus)) {
                self.ui.native = None;
            }
            self.ui.paste = None;
        }
        self.ui.dirty |= self.ui.hot != old_hot;
        if let Err(error) = self.finish_ui_requests() { self.report(Err(error)); }
        if handled { self.sync_navigation(); }
        handled
    }
    pub(super) fn open_ui(&mut self, spec: ui::DialogSpec) -> Result<()> {
        self.ui.requests.push_back(ui::Request::Open(spec));
        self.finish_ui_requests()
    }
    pub(super) fn close_ui(&mut self) {
        if let Some(dialog) = self.root.dialog.take() {
            self.ui.detach(dialog.id());
            if let Some((identity, lineage, session)) = self.ui.return_to.take()
                && identity == self.controller.identity && lineage == self.controller.account.source_lineage
                && session == self.controller.account.selected && self.root.dialog.is_none() {
                self.root.legacy.focus = Some(None);
            }
        }
    }
    pub(super) fn finish_ui_requests(&mut self) -> Result<()> {
        let mut failure = None;
        let mut structural = false;
        while let Some(request) = self.ui.requests.pop_front() {
            structural = true;
            let result = match request {
                ui::Request::Open(spec) => self.with_ui(|_, cx| ui::Dialog::new(spec, cx)).map(|dialog| {
                    let return_to = self.ui.return_to.take().or_else(|| {
                        (self.root.dialog.is_none() && self.root.legacy.focus == Some(None))
                            .then(|| (self.controller.identity.clone(), self.controller.account.source_lineage.clone(), self.controller.account.selected.clone()))
                    });
                    self.cancel_pointer();
                    self.ui.native = None; self.ui.paste = None;
                    self.close_ui();
                    self.ui.return_to = return_to;
                    
                    self.root.legacy.focus = None;
                    self.root.dialog = Some(dialog);
                    self.ui.dirty = true;
                }),
                ui::Request::Menu(menu) => {
                    self.cancel_pointer(); self.root.tooltips.dismiss(); self.ui.native = None; self.ui.paste = None; self.root.legacy.focus = None;
                    self.root.menu = Some(menu); Ok(())
                }
                ui::Request::CloseMenu(id) => {
                    if self.root.menu.as_ref().is_some_and(|m| m.id == id) { self.root.menu = None; self.ui.detach(id); } Ok(())
                }
                ui::Request::MoveMenu { owner, session } => {
                    if self.root.menu.as_ref().is_some_and(|m| m.id == owner) {
                        let parent = self.root.menu.take().unwrap();
                        let menu = self.with_ui(|_, cx| ui::Menu::move_submenu(parent, session, cx));
                        self.ui.requests.push_front(ui::Request::Menu(Box::new(menu)));
                    }
                    Ok(())
                }
                ui::Request::Tip { info, rect } => { self.root.tooltips.pin(info,rect); Ok(()) }
                ui::Request::Download(target) => {
                    self.controller.notice = None;
                    self.open_download_notice(target)
                }
                ui::Request::View(spec) => {
                    self.cancel_pointer(); self.close_ui(); self.root.viewer = Some(ui::ImageViewer::new(spec)); self.ui.native = None; self.ui.paste = None; Ok(())
                }
                ui::Request::CloseViewer(id) => {
                    if self.root.viewer.as_ref().is_some_and(|v| v.id == id) { self.root.viewer = None; self.ui.detach(id); } Ok(())
                }
                ui::Request::Close(owner) => {
                    if self.root.dialog.as_ref().is_some_and(|dialog| dialog.id() == owner) {
                        self.close_ui();
                        if self.controller.account.selected.is_none() { self.root.legacy.show_chats = true; }
                    }
                    Ok(())
                }
                ui::Request::Replace { owner, spec } => {
                    if self.root.dialog.as_ref().is_some_and(|dialog| dialog.id() == owner) {
                        self.ui.requests.push_front(ui::Request::Open(spec));
                    }
                    Ok(())
                }
            };
            // One failed request must not drop the remainder of a taken queue.
            if let Err(error) = result { if failure.is_none() { failure = Some(error); } }
        }
        if structural { self.code_tick(0.); }
        failure.map_or(Ok(()), Err)
    }
}
