//! The only boundary allowed to replace an executing retained subtree. Requests
//! are drained after its event/visit borrow, never postponed until the next paint.
use super::*;

impl App {
    pub(super) fn with_ui<R>(&mut self, run: impl FnOnce(&mut ui::RootWidget, &mut ui::Context<'_>) -> R) -> R {
        let Self { controller, root, ui, services } = self;
        run(root, &mut ui::Context { model: controller, ui, services })
    }
    pub(super) fn update_widgets(&mut self, dt: f32) {
        self.sync_navigation();
        self.reconcile_targets();
        self.ui.covered = self.root.dialog.is_some() || self.root.viewer.is_some();
        self.ui.composing = self.composing();
        self.with_ui(|root, cx| root.update(dt, cx));
        if let Err(error) = self.finish_ui_requests() { self.report(Err(error)); }
        self.reconcile_targets();
    }
    pub(super) fn ui_event(&mut self, event: ui::Event<'_>) -> bool {
        self.sync_navigation();
        self.reconcile_targets();
        if let ui::Event::Down { pointer, .. } | ui::Event::Move { pointer, .. } | ui::Event::Up { pointer, .. } = event
            && self.ui.capture.is_some_and(|capture| capture.pointer != pointer)
        {
            return true;
        }
        self.ui.covered = self.root.dialog.is_some() || self.root.viewer.is_some();
        self.ui.composing = self.composing();
        let old_focus = self.ui.focus;
        let old_hot = self.ui.hot;
        if let ui::Event::Hover(point) = event {
            self.ui.hover = point;
            self.ui.hot = None;
            self.ui.hint = None;
        }
        let handled = self.with_ui(|root, cx| root.handle_event(&event, cx));
        if old_focus != self.ui.focus {
            if let Some(field) = self.root.editor(old_focus) {
                field.editor.preedit(String::new(), None);
            }
            if !self.ui.native.as_ref().is_some_and(|input| Some(input.target) == self.ui.focus) {
                self.ui.native = None;
            }
            self.ui.paste = None;
        }
        self.ui.dirty |= self.ui.hot != old_hot;
        if let Err(error) = self.finish_ui_requests() {
            self.report(Err(error));
        }
        if handled {
            self.sync_navigation();
        }
        if let ui::Event::Up { pointer, .. } = event
            && self.ui.capture.is_some_and(|c| c.pointer == pointer)
        {
            self.ui.capture = None;
        }
        self.reconcile_targets();
        handled
    }
    pub(super) fn reconcile_targets(&mut self) {
        let owns = |target| self.root.owns(target, &self.controller, &self.ui);
        let focus = self.ui.focus.filter(|t| owns(*t));
        let capture = self.ui.capture.filter(|c| owns(c.target));
        let hot = self.ui.hot.filter(|(t, _)| owns(*t));
        let native = self.ui.native.as_ref().is_none_or(|e| owns(e.target));
        let paste = self.ui.paste.as_ref().is_none_or(|e| owns(e.target));
        if self.ui.focus != focus {
            self.cancel_preedit();
            self.ui.dirty = true;
        }
        if self.ui.capture.is_some() && capture.is_none() {
            // The sole pointer lost its owner. Stop the abandoned selection and
            // ancestor scroll candidates too, not just the global target token.
            self.with_ui(|root, cx| root.workspace.cancel(cx));
        }
        self.ui.focus = focus;
        self.ui.capture = capture;
        self.ui.hot = hot;
        if !native {
            self.ui.native = None;
        }
        if !paste {
            self.ui.paste = None;
        }
    }
    pub(super) fn open_ui(&mut self, spec: ui::DialogSpec) -> Result<()> {
        self.ui.requests.push_back(ui::Request::Open(spec));
        self.finish_ui_requests()
    }
    pub(super) fn close_ui(&mut self) {
        if let Some(dialog) = self.root.dialog.take() {
            self.ui.detach(dialog.id());
            if let Some((identity, lineage, session)) = self.ui.return_to.take()
                && identity == self.controller.identity
                && lineage == self.controller.account.source_lineage
                && session == self.controller.account.selected
                && self.root.dialog.is_none()
            {
                self.ui.focus = Some(self.root.workspace.chat.composer.field.control.target);
            }
        }
    }
    pub(super) fn finish_ui_requests(&mut self) -> Result<()> {
        let mut failure = None;
        let mut structural = false;
        while let Some(request) = self.ui.requests.pop_front() {
            structural = true;
            let result = match request {
                ui::Request::Open(spec) => {
                    let old_focus = self.ui.focus;
                    let return_to = self.ui.return_to.clone().or_else(|| {
                        (self.root.dialog.is_none()
                            && old_focus == Some(self.root.workspace.chat.composer.field.control.target))
                        .then(|| {
                            (
                                self.controller.identity.clone(),
                                self.controller.account.source_lineage.clone(),
                                self.controller.account.selected.clone(),
                            )
                        })
                    });
                    self.cancel_preedit();
                    match self.with_ui(|_, cx| ui::Dialog::new(spec, cx)) {
                        Ok(dialog) => {
                            let initial_focus = self.ui.focus;
                            self.cancel_pointer();
                            self.close_ui();
                            self.ui.native = None;
                            self.ui.paste = None;
                            self.ui.return_to = return_to;
                            self.ui.focus = initial_focus;
                            self.root.dialog = Some(dialog);
                            self.ui.dirty = true;
                            Ok(())
                        }
                        Err(error) => {
                            self.ui.focus = old_focus;
                            Err(error)
                        }
                    }
                }
                ui::Request::Menu(menu) => {
                    self.cancel_pointer();
                    self.root.tooltips.dismiss();
                    self.ui.native = None;
                    self.ui.paste = None;
                    self.ui.focus = None;
                    self.root.menu = Some(menu);
                    Ok(())
                }
                ui::Request::CloseMenu(id) => {
                    if self.root.menu.as_ref().is_some_and(|m| m.id == id) {
                        self.root.menu = None;
                        self.ui.detach(id);
                    }
                    Ok(())
                }
                ui::Request::MoveMenu { owner, session } => {
                    if self.root.menu.as_ref().is_some_and(|m| m.id == owner) {
                        let parent = self.root.menu.take().unwrap();
                        let menu = self.with_ui(|_, cx| ui::Menu::move_submenu(parent, session, cx));
                        self.ui.requests.push_front(ui::Request::Menu(Box::new(menu)));
                    }
                    Ok(())
                }
                ui::Request::Select(id) => self.with_ui(|root, cx| root.workspace.navigate_chat(&id, cx)),
                ui::Request::Project(id) => self.with_ui(|root, cx| root.workspace.navigate_project(&id, cx)),
                ui::Request::NewChat => self.with_ui(|root, cx| root.workspace.new_chat(cx)),
                ui::Request::Attachments(show) => self.with_ui(|root, cx| root.workspace.attachments(show, cx)),
                ui::Request::Back => {
                    self.with_ui(|root, cx| root.workspace.back(cx));
                    Ok(())
                }
                ui::Request::Files => self.with_ui(|root, cx| root.workspace.files(cx)),
                ui::Request::Tip { info, rect } => {
                    self.root.tooltips.pin(info, rect);
                    Ok(())
                }
                ui::Request::Download(target) => {
                    self.controller.notice = None;
                    self.open_download_notice(target)
                }
                ui::Request::View(spec) => {
                    self.cancel_pointer();
                    self.close_ui();
                    self.root.viewer = Some(ui::ImageViewer::new(spec));
                    self.ui.native = None;
                    self.ui.paste = None;
                    Ok(())
                }
                ui::Request::CloseViewer(id) => {
                    if self.root.viewer.as_ref().is_some_and(|v| v.id == id) {
                        self.root.viewer = None;
                        self.ui.detach(id);
                    }
                    Ok(())
                }
                ui::Request::Close(owner) => {
                    if self.root.dialog.as_ref().is_some_and(|dialog| dialog.id() == owner) {
                        self.close_ui();
                        if self.controller.account.selected.is_none() {
                            self.root.workspace.show_chats = true;
                        }
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
            if let Err(error) = result {
                if failure.is_none() {
                    failure = Some(error);
                }
            }
        }
        self.ui.menu_chat = self.root.menu.as_ref().and_then(|m| m.chat.clone());
        self.ui.menu_section = self.root.menu.as_ref().and_then(|m| m.section.clone());
        self.ui.covered = self.root.dialog.is_some() || self.root.viewer.is_some();
        if structural {
            self.with_ui(|root, cx| {
                root.workspace.chat.code.code_tick(0., cx);
                root.workspace.chat.composer.bind(cx);
            });
        }
        failure.map_or(Ok(()), Err)
    }
}
