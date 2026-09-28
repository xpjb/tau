use super::{AttachmentBrowser, Composer, Context, Event, Frame, Sidebar, Widget};
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
    pub id: super::Id,
    pub header: Header,
    pub transcript: Transcript,
    pub composer: Composer,
    pub code: CodeBrowser,
}
pub(in crate::app) struct Workspace {
    pub id: super::Id,
    pub sidebar: Sidebar,
    pub chat: ChatPane,
    pub attachments: AttachmentBrowser,
    pub show_chats: bool,
    pub navigation: Navigation,
}
impl Workspace {
    pub fn new(model: &crate::controller::Controller) -> Self {
        Self {
            id: super::Id::new(),
            sidebar: Sidebar::new(),
            chat: ChatPane {
                id: super::Id::new(),
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
        if let Some(id) = &cx.model.account.selected {
            cx.model.save_chat(id)?;
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
        self.chat.code.handle_event(&Event::Cancel, cx);
        self.chat.composer.handle_event(&Event::Cancel, cx);
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
        self.sidebar.hints().chain(self.attachments.cards.hints()).chain(
            self.chat.transcript.rows.iter().filter_map(|r| r.attachment.as_ref()).flat_map(|c| c.controls.hints()),
        )
    }
    pub fn chat_visible(&self, cx: &Context<'_>) -> bool {
        cx.ui.window_focused
            && !cx.ui.covered
            && (cx.ui.size.0 as f32 / cx.ui.scale >= 760. || !self.show_chats)
            && (!self.attachments.show || !cx.ui.mobile && cx.ui.size.0 as f32 / cx.ui.scale >= 1000.)
            && self.chat.code.view.is_none()
    }
}
impl Widget for Workspace {
    fn handle_event(&mut self, event: &Event<'_>, cx: &mut Context<'_>) -> bool {
        if matches!(event, Event::Cancel) {
            self.cancel(cx);
            return false;
        }
        let width = cx.ui.size.0 as f32 / cx.ui.scale;
        let screen = self.attachments.show && (cx.ui.mobile || width < 1000.);
        let mut handled = cx.ui.routes_pointer_to(event, self.attachments.scroll.target.scope)
            && self.attachments.handle_event(event, cx);
        if !handled
            && !screen
            && (width >= 760. || self.show_chats)
            && cx.ui.routes_pointer_to(event, self.sidebar.controls.id)
        {
            handled = self.sidebar.handle_event(event, cx);
        }
        if !handled && !screen && (width >= 760. || !self.show_chats) && cx.ui.routes_pointer_to(event, self.chat.id) {
            handled = self.chat.handle_event(event, cx);
        }
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
        let b = frame.bounds;
        let s = cx.ui.scale;
        let wide = b.width / s >= 760.;
        let sidebar = if wide { 300. * s } else { b.width };
        let side = self.attachments.show && !cx.ui.mobile && b.width / s >= 1000.;
        let screen = self.attachments.show && !side;
        let file_width = if side { 320. * s } else { 0. };
        if !screen && (wide || self.show_chats) {
            self.sidebar.visit_perframe(
                &mut Frame { layer: frame.layer, bounds: Rect::new(b.x, b.y, sidebar, b.height), clip: frame.clip },
                cx,
            );
        }
        if !screen && (wide || !self.show_chats) {
            let chat = if wide { Rect::new(b.x + sidebar, b.y, b.width - sidebar - file_width, b.height) } else { b };
            self.chat.header.attachments_open = self.attachments.show;
            self.chat.visit_perframe(&mut Frame { layer: frame.layer, bounds: chat, clip: frame.clip }, cx);
        }
        if self.attachments.show {
            let bounds = if side { Rect::new(b.x + b.width - file_width, b.y, file_width, b.height) } else { b };
            self.attachments.side = side;
            self.attachments.visit_perframe(&mut Frame { layer: frame.layer, bounds, clip: frame.clip }, cx);
        }
        let mut interests = self.chat.transcript.interests.clone();
        interests.extend(self.attachments.interests.iter().cloned());
        let request = if (wide || !self.show_chats || self.attachments.show) && self.chat.code.view.is_none() {
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
impl Widget for ChatPane {
    fn handle_event(&mut self, event: &Event<'_>, cx: &mut Context<'_>) -> bool {
        if cx.ui.routes_pointer_to(event, self.code.controls.id) && self.code.handle_event(event, cx) {
            return true;
        }
        if let Some(view) = &self.code.view {
            return view.search.is_none()
                && view.document.is_some()
                && (view.selection.is_some() || cx.ui.focus == Some(self.composer.field.control.target))
                && self.composer.handle_event(event, cx);
        }
        cx.ui.routes_pointer_to(event, self.header.controls.id) && self.header.handle_event(event, cx)
            || cx.ui.routes_pointer_to(event, self.composer.controls.id) && self.composer.handle_event(event, cx)
            || cx.ui.routes_pointer_to(event, self.transcript.scroll.target.scope)
                && self.transcript.handle_event(event, cx)
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
        let code = self.code.view.as_ref();
        let comments = code.is_none_or(|v| {
            v.search.is_none() && v.document.is_some() && (v.selection.is_some() || cx.ui.focus == cx.ui.composer)
        });
        let layout = self.composer.layout(cx, frame.bounds, &session);
        let bottom = layout.4;
        if code.is_some() {
            self.code.composer_bottom = comments.then_some(bottom);
            self.code.visit_perframe(frame, cx);
        } else {
            self.header.visit_perframe(frame, cx);
            let b = frame.bounds;
            let s = cx.ui.scale;
            let bounds = Rect::new(b.x, b.y + 57. * s, b.width, (bottom - b.y - 57. * s).max(1.));
            self.transcript.visit_perframe(
                &mut Frame { layer: frame.layer, bounds, clip: crate::render::intersect(frame.clip, bounds) },
                cx,
            );
        }
        if comments {
            self.composer.in_code = self.code.view.is_some();
            self.composer.code_ready =
                self.code.view.as_ref().is_some_and(|v| v.selection.is_some() && v.error.is_none());
            self.composer.away_from_tail =
                self.transcript.scroll.value + 24. * cx.ui.scale < self.transcript.scroll.max;
            self.composer.visit_perframe(frame, cx);
        }
    }
}
