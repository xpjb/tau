use super::{AttachmentBrowser, Composer, Context, Controller, Event, Frame, Sidebar, Target, UiState, Widget};
use super::{header::Header, transcript::Transcript};
use crate::{
    app::{
        PlatformAction,
        code_view::{Choice as CodeChoice, CodeBrowser},
    },
    notice::DownloadTarget,
    render::color,
};
use sanscale::Rect;

pub(in crate::app) struct Navigation {
    pub identity: String,
    pub lineage: Option<String>,
    pub session: Option<String>,
}
impl Navigation {
    fn new(cx: &Context<'_>) -> Self {
        Self {
            identity: cx.model.identity.clone(),
            lineage: cx.model.account.source_lineage.clone(),
            session: cx.model.account.selected.clone(),
        }
    }
}
pub(in crate::app) struct ChatPane {
    pub header: Header,
    pub transcript: Transcript,
    pub composer: Composer,
    pub code: CodeBrowser,
}
pub(in crate::app) struct Workspace {
    pub sidebar: Sidebar,
    pub chat: ChatPane,
    pub attachments: AttachmentBrowser,
    pub show_chats: bool,
    pub navigation: Navigation,
}
impl Workspace {
    fn layout(&self, b: Rect, ui: &UiState) -> [Option<Rect>; 3] {
        let wide = b.width / ui.scale >= 760.;
        let sidebar = if wide { 300. * ui.scale } else { b.width };
        let side = self.attachments.show && !ui.mobile && b.width / ui.scale >= 1000.;
        let screen = self.attachments.show && !side;
        let files = if side { 320. * ui.scale } else { 0. };
        [
            (!screen && (wide || self.show_chats)).then_some(Rect::new(b.x, b.y, sidebar, b.height)),
            (!screen && (wide || !self.show_chats)).then_some(if wide {
                Rect::new(b.x + sidebar, b.y, b.width - sidebar - files, b.height)
            } else {
                b
            }),
            self.attachments.show.then_some(if side {
                Rect::new(b.x + b.width - files, b.y, files, b.height)
            } else {
                b
            }),
        ]
    }
    fn children(&mut self, b: Rect, ui: &UiState) -> [(Option<Rect>, &mut dyn Widget); 3] {
        let [sidebar, chat, files] = self.layout(b, ui);
        [(sidebar, &mut self.sidebar), (chat, &mut self.chat), (files, &mut self.attachments)]
    }
    pub fn new(model: &crate::controller::Controller) -> Self {
        Self {
            sidebar: Sidebar::new(),
            chat: ChatPane {
                header: Header::new(),
                transcript: Transcript::new(),
                composer: Composer::new(),
                code: CodeBrowser::new(),
            },
            attachments: AttachmentBrowser::new(),
            show_chats: model.account.selected.is_none(),
            navigation: Navigation {
                identity: model.identity.clone(),
                lineage: model.account.source_lineage.clone(),
                session: model.account.selected.clone(),
            },
        }
    }
    pub fn sync_navigation(&mut self, cx: &mut Context<'_>) {
        self.chat.transcript.validate_download_jump(cx);
        if self.navigation.identity != cx.model.identity
            || self.navigation.lineage != cx.model.account.source_lineage
            || self.navigation.session != cx.model.account.selected
        {
            if self.navigation.identity == cx.model.identity
                && self.navigation.lineage == cx.model.account.source_lineage
                && let Some(previous) = &self.navigation.session
                && self.chat.transcript.placed_session.as_ref() == Some(previous)
            {
                let result = cx.model.save_chat(previous);
                cx.report(result);
            }
            cx.ui.navigation_changed();
            self.chat.code.close_code(cx);
            self.cancel(cx);
            self.navigation = Navigation::new(cx);
            self.show_chats = self.navigation.session.is_none();
            self.attachments.reset();
            if self.show_chats {
                self.attachments.show = false;
            }
            self.chat.header.hide();
            self.sidebar.hide();
            cx.ui.dirty = true;
        }
        self.chat.transcript.bind(cx);
        self.chat.composer.bind(cx);
    }
    pub fn save(&mut self, cx: &mut Context<'_>) -> anyhow::Result<()> {
        self.chat.transcript.remember_scroll(cx);
        if let Some(id) = cx.model.account.selected.clone() {
            cx.model.save_chat(&id)?;
        }
        Ok(())
    }
    pub fn navigate_chat(&mut self, id: &str, cx: &mut Context<'_>) -> anyhow::Result<()> {
        self.save(cx)?;
        let same = cx.model.account.selected.as_deref() == Some(id);
        let topic = cx
            .model
            .account
            .sessions
            .iter()
            .find(|s| s.id == id)
            .is_none_or(|s| s.project_id == cx.model.account.selected_project);
        if !same || !topic {
            cx.model.select(id)?;
        }
        self.chat.code.close_code(cx);
        self.sync_navigation(cx);
        self.show_chats = false;
        cx.ui.focus = Some(self.chat.composer.field.control.target);
        Ok(())
    }
    pub fn navigate_project(&mut self, id: &str, cx: &mut Context<'_>) -> anyhow::Result<()> {
        if cx.model.account.selected_project == id
            && cx
                .model
                .account
                .selected
                .as_ref()
                .is_some_and(|chat| cx.model.account.sessions.iter().any(|s| &s.id == chat && s.project_id == id))
        {
            return Ok(());
        }
        self.save(cx)?;
        cx.model.select_project(id, cx.ui.size.0 as f32 / cx.ui.scale >= 760.)?;
        self.sync_navigation(cx);
        self.sidebar.scroll.value = 0.;
        self.show_chats = cx.model.account.selected.is_none();
        cx.ui.focus = None;
        Ok(())
    }
    pub fn new_chat(&mut self, cx: &mut Context<'_>) -> anyhow::Result<()> {
        self.save(cx)?;
        cx.model.new_chat()?;
        self.sync_navigation(cx);
        self.show_chats = false;
        cx.ui.focus = Some(self.chat.composer.field.control.target);
        Ok(())
    }
    pub fn open_download(&mut self, target: DownloadTarget, cx: &mut Context<'_>) -> anyhow::Result<()> {
        anyhow::ensure!(
            target.matches_source(&cx.model.identity, cx.model.account.source_lineage.as_deref()),
            "This download belongs to a different account or source history."
        );
        anyhow::ensure!(
            cx.model.account.sessions.iter().any(|s| s.id == target.session)
                && !cx.model.account.missing_chats.contains(&target.session),
            "The chat for this download is no longer available."
        );
        self.navigate_chat(&target.session, cx)?;
        self.cancel(cx);
        cx.ui.focus = None;
        self.show_chats = false;
        self.attachments.show = false;
        self.sidebar.scroll.value = 0.;
        let transcript = &mut self.chat.transcript;
        transcript.horizontal.value = 0.;
        transcript.scroll.value = 0.;
        transcript.placed.clear();
        transcript.placed_session = None;
        transcript.history_attempt = None;
        transcript.download = Some(target);
        Ok(())
    }
    pub fn files(&mut self, cx: &mut Context<'_>) -> anyhow::Result<()> {
        self.save(cx)?;
        self.cancel(cx);
        self.chat.code.code_action(CodeChoice::Files, cx)?;
        self.show_chats = false;
        self.attachments.show = false;
        Ok(())
    }
    pub fn attachments(&mut self, show: bool, cx: &mut Context<'_>) -> anyhow::Result<()> {
        self.save(cx)?;
        self.cancel(cx);
        self.chat.code.close_code(cx);
        cx.ui.focus = None;
        cx.ui.native = None;
        cx.ui.paste = None;
        self.attachments.show = show;
        self.show_chats = false;
        if !show {
            self.attachments.hide();
        }
        Ok(())
    }
    pub fn back(&mut self, cx: &mut Context<'_>) {
        self.chat.transcript.download = None;
        cx.ui.focus = None;
        cx.ui.native = None;
        cx.ui.paste = None;
        if self.chat.code.view.is_some() {
            self.chat.code.code_back(cx);
        } else if self.attachments.show {
            self.attachments.show = false;
            self.cancel(cx);
        } else if !self.show_chats && cx.ui.size.0 as f32 / cx.ui.scale < 760. {
            self.show_chats = true;
            self.cancel(cx);
        } else {
            cx.services.platform.push(PlatformAction::Background);
        }
        cx.ui.dirty = true;
    }
    pub fn cancel(&mut self, cx: &mut Context<'_>) {
        self.chat.transcript.cancel();
        self.sidebar.scroll.stop();
        self.sidebar.projects.scroll.stop();
        self.attachments.scroll.stop();
        self.chat.code.cancel_pointer(cx);
        cx.ui.cancel();
    }
    pub fn send(&mut self, cx: &mut Context<'_>) -> anyhow::Result<()> {
        if let Some(code) = &self.chat.code.view {
            anyhow::ensure!(
                code.selection.is_some() && code.error.is_none(),
                "Select current lines again before sending this code comment"
            );
        }
        if self.chat.composer.field.editor.composing() {
            self.chat.composer.field.editor.preedit(String::new(), None);
        }
        if self.chat.code.view.is_some() {
            self.chat.code.code_reference(false, cx);
            self.chat.composer.bind(cx);
        }
        self.chat.composer.send(cx)?;
        if let Some(code) = &mut self.chat.code.view {
            code.sent();
        }
        Ok(())
    }
    pub fn hints(&self) -> impl Iterator<Item = (Rect, &crate::app::Info)> {
        self.sidebar.hints().chain(self.attachments.cards.values().flat_map(|card| card.controls.hints())).chain(
            self.chat.transcript.rows.iter().filter_map(|r| r.attachment.as_ref()).flat_map(|c| c.controls.hints()),
        )
    }
    pub fn chat_visible(&self, cx: &Context<'_>) -> bool {
        cx.ui.window_focused
            && !cx.ui.covered
            && self.layout(cx.ui.bounds(), cx.ui)[1].is_some()
            && self.chat.code.view.is_none()
    }
}
impl Widget for Workspace {
    fn update(&mut self, dt: f32, cx: &mut Context<'_>) {
        for (bounds, child) in self.children(cx.ui.bounds(), cx.ui) {
            if bounds.is_some() {
                child.update(dt, cx);
            }
        }
    }
    fn owns(&self, target: Target, model: &Controller, ui: &UiState) -> bool {
        self.layout(ui.bounds(), ui)
            .into_iter()
            .zip([&self.sidebar as &dyn Widget, &self.chat, &self.attachments])
            .any(|(bounds, child)| bounds.is_some() && child.owns(target, model, ui))
    }

    fn handle_event(&mut self, event: &Event<'_>, cx: &mut Context<'_>) -> bool {
        let children = self.children(cx.ui.bounds(), cx.ui);
        let handled = super::dispatch_children(
            children.into_iter().filter_map(|(bounds, child)| bounds.map(|_| child)),
            event,
            cx,
        );
        if self.chat.code.view.is_some() {
            self.show_chats = false;
            self.attachments.show = false;
            self.chat.header.hide();
            self.chat.transcript.hide();
        }
        if std::mem::take(&mut self.chat.composer.submit) {
            let result = self.send(cx);
            cx.report(result);
        }
        if std::mem::take(&mut self.chat.composer.tail) {
            self.chat.transcript.tail(cx);
        }
        if handled {
            return true;
        }
        match *event {
            Event::Back => {
                self.back(cx);
                true
            }
            Event::Key { key: "Escape", .. } => {
                if self.attachments.show {
                    self.back(cx);
                } else if let Some(session_id) = cx.model.account.selected.clone() {
                    let result = cx.model.control(tau_protocol::ClientCommand::Abort { session_id });
                    cx.report(result);
                }
                true
            }
            Event::Key { key: "v" | "V", ctrl: true, .. } if self.chat.composer.field.control.rect.is_some() => {
                cx.ui.focus = Some(self.chat.composer.field.control.target);
                self.chat.composer.handle_event(event, cx)
            }
            _ => false,
        }
    }
    fn visit_perframe(&mut self, frame: &mut Frame<'_>, cx: &mut Context<'_>) {
        self.sync_navigation(cx);
        self.sidebar.hide();
        self.chat.header.hide();
        self.chat.composer.hide();
        self.chat.transcript.hide();
        self.attachments.hide();
        let layout = self.layout(frame.bounds, cx.ui);
        self.chat.header.attachments_open = self.attachments.show;
        self.attachments.side = layout[2].is_some_and(|r| r.width < frame.bounds.width);
        for (bounds, child) in self.children(frame.bounds, cx.ui) {
            if let Some(bounds) = bounds {
                frame.visit(bounds, child, cx);
            }
        }
        let mut interests = self.chat.transcript.interests.clone();
        interests.extend(self.attachments.interests.iter().cloned());
        let request = if (layout[1].is_some() || layout[2].is_some()) && self.chat.code.view.is_none() {
            cx.model.account.selected.clone().map(|session| (session, interests))
        } else {
            None
        };
        if let Some((session, interests)) = request {
            cx.model.viewport(&session, interests);
        } else if let Some(session) = cx.model.account.selected.clone() {
            cx.model.viewport(&session, std::collections::BTreeSet::new());
        }
    }
}
#[derive(Clone, Copy)]
enum ChatChild {
    Transcript,
    Header,
    Code,
    Composer,
}
impl ChatPane {
    fn order(&self, ui: &UiState) -> [Option<ChatChild>; 4] {
        let code = self.code.view.as_ref();
        [
            code.is_none().then_some(ChatChild::Transcript),
            code.is_none().then_some(ChatChild::Header),
            code.is_some().then_some(ChatChild::Code),
            code.is_none_or(|v| {
                v.search.is_none()
                    && v.document.is_some()
                    && (v.selection.is_some() || ui.focus == Some(self.composer.field.control.target))
            })
            .then_some(ChatChild::Composer),
        ]
    }
    fn child(&self, child: ChatChild) -> &dyn Widget {
        match child {
            ChatChild::Transcript => &self.transcript,
            ChatChild::Header => &self.header,
            ChatChild::Code => &self.code,
            ChatChild::Composer => &self.composer,
        }
    }
    fn child_mut(&mut self, child: ChatChild) -> &mut dyn Widget {
        match child {
            ChatChild::Transcript => &mut self.transcript,
            ChatChild::Header => &mut self.header,
            ChatChild::Code => &mut self.code,
            ChatChild::Composer => &mut self.composer,
        }
    }
}
impl Widget for ChatPane {
    fn update(&mut self, dt: f32, cx: &mut Context<'_>) {
        for child in self.order(cx.ui).into_iter().flatten() {
            self.child_mut(child).update(dt, cx);
        }
    }
    fn owns(&self, target: Target, model: &Controller, ui: &UiState) -> bool {
        model.selected().is_some()
            && self.order(ui).into_iter().flatten().any(|child| self.child(child).owns(target, model, ui))
    }
    fn handle_event(&mut self, event: &Event<'_>, cx: &mut Context<'_>) -> bool {
        // This accelerator belongs to the chat, even when the browser is closed.
        if !cx.ui.composing && matches!(event, Event::Key { key: "Space" | " ", ctrl: true, .. }) {
            return self.code.handle_event(event, cx);
        }
        let mut handled = false;
        for child in self.order(cx.ui).into_iter().flatten().rev() {
            handled |= self.child_mut(child).dispatch(event, cx);
            if handled {
                break;
            }
        }
        handled
    }
    fn visit_perframe(&mut self, frame: &mut Frame<'_>, cx: &mut Context<'_>) {
        let Some(session) = cx.model.account.selected.clone().filter(|s| cx.model.chats.contains_key(s)) else {
            cx.services.renderer.label(
                frame.layer,
                "Select a chat",
                Rect::new(
                    frame.bounds.x + 32. * cx.ui.scale,
                    frame.bounds.y + 40. * cx.ui.scale,
                    frame.bounds.width - 64. * cx.ui.scale,
                    50. * cx.ui.scale,
                ),
                24. * cx.ui.scale,
                color(0x82909f),
                false,
            );
            return;
        };
        let b = frame.bounds;
        let bottom = self.composer.layout(cx, b, &session).4;
        let order = self.order(cx.ui);
        self.code.composer_bottom = order[3].is_some().then_some(bottom);
        self.composer.in_code = self.code.view.is_some();
        self.composer.code_ready = self.code.view.as_ref().is_some_and(|v| v.selection.is_some() && v.error.is_none());
        let header = 57. * cx.ui.scale;
        for child in order.into_iter().flatten() {
            let bounds = match child {
                ChatChild::Transcript => Rect::new(b.x, b.y + header, b.width, (bottom - b.y - header).max(1.)),
                ChatChild::Header => Rect::new(b.x, b.y, b.width, header),
                ChatChild::Composer => {
                    self.composer.away_from_tail =
                        self.transcript.scroll.value + 24. * cx.ui.scale < self.transcript.scroll.max;
                    b
                }
                ChatChild::Code => b,
            };
            frame.visit(bounds, self.child_mut(child), cx);
        }
    }
}
