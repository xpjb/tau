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
        if let ui::Event::Hover(point) = event { self.ui.hover = point; self.ui.hot = None; }
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
                && session == self.controller.account.selected && self.root.legacy.modal.is_none() {
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
                        (self.root.legacy.modal.is_none() && self.root.legacy.focus == Some(None))
                            .then(|| (self.controller.identity.clone(), self.controller.account.source_lineage.clone(), self.controller.account.selected.clone()))
                    });
                    self.cancel_pointer();
                    self.close_ui();
                    self.ui.return_to = return_to;
                    self.root.legacy.modal = None;
                    self.root.legacy.focus = None;
                    self.root.dialog = Some(dialog);
                    self.ui.dirty = true;
                }),
                ui::Request::Close(owner) => {
                    if self.root.dialog.as_ref().is_some_and(|dialog| dialog.id() == owner) {
                        self.close_ui();
                        if self.controller.account.selected.is_none() { self.root.legacy.show_chats = true; }
                    }
                    Ok(())
                }
                ui::Request::Legacy { owner, dialog } => {
                    if self.root.dialog.as_ref().is_some_and(|dialog| dialog.id() == owner) {
                        let old = self.root.dialog.take();
                        let action = match dialog { ui::LegacyDialog::Models => Action::ModelSettings,
                            ui::LegacyDialog::Daemon => Action::DaemonSettings,
                            ui::LegacyDialog::RefreshCatalog => Action::RefreshCatalog,
                            ui::LegacyDialog::Outbox => Action::Outbox(0) };
                        match self.apply(action) {
                            Ok(()) => { self.ui.detach(owner); self.ui.return_to = None; Ok(()) }
                            Err(error) => { self.root.dialog = old; Err(error) }
                        }
                    } else { Ok(()) }
                }
            };
            // One failed request must not drop the remainder of a taken queue.
            if let Err(error) = result { if failure.is_none() { failure = Some(error); } }
        }
        if structural { self.code_tick(0.); }
        failure.map_or(Ok(()), Err)
    }
}
