//! A menu is an opaque retained subtree, including its scrollable submenu.
//! Choices are local to this owner; fields, workspace controls and native input
//! never dispatch through it.
use super::controls::Control;
use super::scroll::ScrollState;
use super::{Context, DialogSpec, Event, Frame, Id, Operation, Request, TopicEdit, Widget};
use crate::{
    app::PlatformAction,
    render::{color, contains},
};
use anyhow::Result;
use sanscale::{Rect, Vec2};
use tau_protocol::*;

#[derive(Clone)]
pub(in crate::app) enum Choice {
    RenameProject(String),
    ProjectPrompt(String),
    DeleteProject(String),
    AgentSetting(String, String),
    MoveMenu(String),
    ContextBack,
    MoveChat(String, String),
    Noop,
    Rename(String),
    ReviewRestore(String),
    Clone(String),
    Sleep(String),
    Delete(String),
    CopyRecoveredDraft(String),
    ForgetRecovered(String),
    Copy(String),
    CopyDetails(String, Vec<String>),
    CopySelection,
    Fork(String),
    EditQueue(String, u64, String),
    Queue(QueueOperation),
    Restore(String),
    RetryPending(String),
    Dismiss(String),
}
pub(in crate::app) struct Menu {
    pub id: Id,
    pub at: Vec2,
    pub section: Option<String>,
    pub chat: Option<String>,
    pub options: Vec<(String, Choice)>,
    controls: Vec<Control>,
    pub selected: usize,
    pub scroll: ScrollState,
    pub parent: Option<Box<Menu>>,
    pub rect: Rect,
    identity: String,
    lineage: Option<String>,
    session: Option<String>,
}
impl Menu {
    pub fn new(
        at: Vec2,
        section: Option<String>,
        chat: Option<String>,
        options: Vec<(String, Choice)>,
        cx: &mut Context<'_>,
    ) -> Self {
        let id = Id::new();
        let controls = options.iter().map(|_| Control::new(id, false)).collect();
        Self {
            id,
            at,
            section,
            chat,
            options,
            controls,
            selected: 0,
            scroll: ScrollState::new(id, false),
            parent: None,
            rect: Rect::new(0., 0., 0., 0.),
            identity: cx.model.identity.clone(),
            lineage: cx.model.account.source_lineage.clone(),
            session: cx.model.account.selected.clone(),
        }
    }
    pub fn contains(&self, point: Vec2) -> bool {
        contains(self.rect, point) || self.parent.as_ref().is_some_and(|p| p.contains(point))
    }
    pub fn reveal(&mut self, scale: f32) {
        let top = self.selected as f32 * 36. * scale;
        if top < self.scroll.value {
            self.scroll.set(top);
        } else if top + 36. * scale > self.scroll.value + self.scroll.rect.height {
            self.scroll.set(top + 36. * scale - self.scroll.rect.height);
        }
    }
    fn back(&mut self, cx: &mut Context<'_>) {
        if let Some(parent) = self.parent.take() {
            cx.ui.requests.push_back(Request::Menu(parent));
        } else {
            cx.ui.requests.push_back(Request::CloseMenu(self.id));
        }
    }
    fn choose(&mut self, choice: Choice, cx: &mut Context<'_>) -> Result<()> {
        let dialog = match choice {
            Choice::RenameProject(key) => Some(DialogSpec::Topic(TopicEdit::Rename(key))),
            Choice::ProjectPrompt(key) => Some(DialogSpec::Topic(TopicEdit::Prompt(key))),
            Choice::DeleteProject(key) => Some(DialogSpec::Topic(TopicEdit::Delete(key))),
            Choice::AgentSetting(id, command) => Some(DialogSpec::Operation(Operation::Agent(id, command))),
            Choice::Rename(id) => Some(DialogSpec::Operation(Operation::Rename(id))),
            Choice::Delete(id) => Some(DialogSpec::Operation(Operation::Delete(id))),
            Choice::ReviewRestore(id) => Some(DialogSpec::Operation(Operation::Review(id))),
            Choice::ForgetRecovered(id) => Some(DialogSpec::Operation(Operation::ForgetRecovered(id))),
            Choice::EditQueue(id, revision, text) => self
                .session
                .clone()
                .map(|session| DialogSpec::Operation(Operation::Queue { session, id, revision, text })),
            Choice::MoveMenu(session) => {
                cx.ui.requests.push_back(Request::MoveMenu { owner: self.id, session });
                return Ok(());
            }
            Choice::ContextBack => {
                self.back(cx);
                return Ok(());
            }
            Choice::Noop => return Ok(()),
            Choice::MoveChat(session_id, project_id) => {
                cx.model.request(ClientCommand::MoveSession { session_id, project_id })?;
                None
            }
            Choice::Clone(session_id) => {
                cx.model.request(ClientCommand::CloneSession { session_id })?;
                None
            }
            Choice::Sleep(session_id) => {
                cx.model.request(ClientCommand::CloseSession { session_id })?;
                None
            }
            Choice::CopyRecoveredDraft(id) => {
                cx.model.copy_missing_draft(&id)?;
                None
            }
            Choice::Copy(text) => {
                cx.model.cancel_copy();
                cx.services.platform.push(PlatformAction::Copy(text));
                None
            }
            Choice::CopyDetails(session, ids) => {
                cx.model.copy_details(&session, ids)?;
                None
            }
            Choice::CopySelection => {
                cx.model.cancel_copy();
                if let Some(text) = cx.services.renderer.selected_text() {
                    cx.services.platform.push(PlatformAction::Copy(text));
                }
                None
            }
            Choice::Fork(entry_id) => {
                if let Some(session_id) = &self.session {
                    cx.model.request(ClientCommand::ForkSession { session_id: session_id.clone(), entry_id })?;
                }
                None
            }
            Choice::Queue(operation) => {
                if let Some(session_id) = &self.session {
                    let generation = cx
                        .model
                        .chats
                        .get(session_id)
                        .ok_or_else(|| anyhow::anyhow!("Chat no longer available"))?
                        .feed
                        .generation
                        .clone();
                    cx.model.control(ClientCommand::QueueControl {
                        session_id: session_id.clone(),
                        generation,
                        operation,
                    })?;
                }
                None
            }
            Choice::Restore(id) => {
                cx.model.restore_pending(&id)?;
                None
            }
            Choice::RetryPending(id) => {
                cx.model.retry_pending(&id)?;
                None
            }
            Choice::Dismiss(id) => {
                cx.model.dismiss_pending(&id)?;
                None
            }
        };
        cx.ui.requests.push_back(Request::CloseMenu(self.id));
        if let Some(dialog) = dialog {
            cx.ui.requests.push_back(Request::Open(dialog));
        }
        Ok(())
    }
    pub fn move_submenu(mut parent: Box<Self>, session: String, cx: &mut Context<'_>) -> Self {
        if let Some(original) = parent.parent.take() {
            parent = original;
        }
        let current = cx.model.account.sessions.iter().find(|s| s.id == session).map(|s| &s.project_id);
        let mut options = vec![("‹  Move to topic".into(), Choice::ContextBack)];
        options.extend(cx.model.account.projects.iter().map(|p| {
            if current == Some(&p.id) {
                (format!("✓  {}", p.name), Choice::Noop)
            } else {
                (p.name.clone(), Choice::MoveChat(session.clone(), p.id.clone()))
            }
        }));
        let mut menu = Self::new(parent.at, None, Some(session), options, cx);
        menu.selected = 1;
        menu.parent = Some(parent);
        menu
    }
    fn controls_event(&mut self, event: &Event<'_>, cx: &mut Context<'_>) -> (bool, Option<Choice>) {
        for (i, control) in self.controls.iter_mut().enumerate().rev() {
            let handled = control.handle(event, cx, false);
            if control.take_click() {
                return (true, Some(self.options[i].1.clone()));
            }
            if handled {
                if matches!(event, Event::Hover(Some(_))) {
                    self.selected = i;
                    if self.parent.is_none() && matches!(self.options[i].1, Choice::MoveMenu(_)) {
                        return (true, Some(self.options[i].1.clone()));
                    }
                }
                return (true, None);
            }
        }
        (false, None)
    }
    fn paint(&mut self, frame: &mut Frame<'_>, cx: &mut Context<'_>, active: bool) {
        let r = frame.bounds;
        let s = cx.ui.scale;
        self.rect = r;
        frame.layer.above();
        frame.layer.rounded_rect(r, 8. * s, color(0x36343b));
        let clip = crate::render::intersect(
            Rect::new(r.x + 4. * s, r.y + 4. * s, r.width - 8. * s, (r.height - 8. * s).max(0.)),
            frame.clip,
        );
        self.scroll.rect = clip;
        self.scroll.max = (self.options.len() as f32 * 36. * s - clip.height).max(0.);
        self.scroll.set(self.scroll.value);
        for (i, ((label, choice), control)) in self.options.iter().zip(&mut self.controls).enumerate() {
            let rect = Rect::new(clip.x, clip.y + i as f32 * 36. * s - self.scroll.value, clip.width, 36. * s);
            control.rect = Some(rect);
            control.clip = clip;
            control.enabled = !matches!(choice, Choice::Noop);
            if cx.ui.hover.is_some_and(|p| control.contains(p))
                || active && cx.ui.hover.is_none() && self.selected == i
                || !active && matches!(choice, Choice::MoveMenu(_))
            {
                frame.layer.clipped_rounded_rect(rect, 4. * s, color(0x494750), clip);
            }
            cx.services.renderer.clipped_label(
                frame.layer,
                label,
                Rect::new(rect.x + 12. * s, rect.y + 9. * s, rect.width - 24. * s, 22. * s),
                14. * s,
                color(if !control.enabled {
                    0x82909f
                } else if matches!(choice, Choice::Delete(_) | Choice::DeleteProject(_)) {
                    0xffb4ab
                } else {
                    0xe5eaf0
                }),
                false,
                clip,
            );
        }
        self.scroll.paint(frame.layer, cx);
    }
    #[cfg(test)]
    pub fn button(&self, predicate: impl Fn(&Choice) -> bool) -> Option<Rect> {
        self.options
            .iter()
            .zip(&self.controls)
            .find_map(|((_, a), c)| predicate(a).then(|| c.rect.map(|r| crate::render::intersect(r, c.clip))).flatten())
    }
}
impl Widget for Menu {
    fn handle_event(&mut self, event: &Event<'_>, cx: &mut Context<'_>) -> bool {
        if self.identity != cx.model.identity
            || self.lineage != cx.model.account.source_lineage
            || self.session != cx.model.account.selected
        {
            cx.ui.requests.push_back(Request::CloseMenu(self.id));
            return true;
        }
        let choice = match *event {
            Event::Cancel => {
                self.scroll.stop();
                return true;
            }
            Event::Back | Event::Key { key: "Escape" | "ArrowLeft", .. } => {
                self.back(cx);
                return true;
            }
            Event::Key { key: "ArrowUp" | "ArrowDown" | "Home" | "End", .. } => {
                if let Event::Key { key, .. } = *event {
                    self.selected = match key {
                        "ArrowUp" => (self.selected + self.options.len() - 1) % self.options.len(),
                        "ArrowDown" => (self.selected + 1) % self.options.len(),
                        "Home" => 0,
                        _ => self.options.len() - 1,
                    };
                }
                cx.ui.hover = None;
                self.reveal(cx.ui.scale);
                cx.ui.dirty = true;
                return true;
            }
            Event::Key { key: "Enter" | "ArrowRight", .. } => Some(self.options[self.selected].1.clone()),
            Event::Down { point, .. } if !self.contains(point) => {
                cx.ui.requests.push_back(Request::CloseMenu(self.id));
                return true;
            }
            _ => {
                if self.scroll.bar_event(event, cx) {
                    return true;
                }
                let (mut handled, mut choice) = self.controls_event(event, cx);
                if !handled && let Some(parent) = &mut self.parent {
                    (handled, choice) = parent.controls_event(event, cx);
                }
                self.scroll.event(event, handled, cx);
                choice
            }
        };
        if let Some(choice) = choice {
            let result = self.choose(choice, cx);
            cx.report(result);
        }
        true
    }
    fn visit_perframe(&mut self, frame: &mut Frame<'_>, cx: &mut Context<'_>) {
        let b = frame.bounds;
        let s = cx.ui.scale;
        let w = (240. * s).min(b.width - 16. * s).max(1.);
        let rect_for = |menu: &Menu, x: f32| {
            let h = ((menu.options.len() as f32 * 36. + 8.) * s).min((b.height - 16. * s).max(1.));
            Rect::new(
                x.clamp(b.x + 8. * s, (b.x + b.width - w - 8. * s).max(b.x + 8. * s)),
                menu.at.y.clamp(b.y + 8. * s, (b.y + b.height - h - 8. * s).max(b.y + 8. * s)),
                w,
                h,
            )
        };
        let mut rect = rect_for(self, self.at.x);
        if let Some(parent) = &mut self.parent
            && b.width >= w * 2. + 24. * s
        {
            let mut pr = rect_for(parent, parent.at.x);
            let x = if pr.x + 2. * w + 8. * s <= b.x + b.width {
                pr.x + w
            } else if pr.x - w >= b.x {
                pr.x - w
            } else {
                pr.x = b.x + 8. * s;
                pr.x + w
            };
            let h = rect.height;
            rect.x = x;
            rect.y = rect.y.clamp(b.y + 8. * s, (b.y + b.height - h - 8. * s).max(b.y + 8. * s));
            parent.paint(&mut Frame { layer: frame.layer, bounds: pr, clip: frame.clip }, cx, false);
        } else if let Some(parent) = &mut self.parent {
            parent.rect = Rect::new(0., 0., 0., 0.);
            for control in &mut parent.controls {
                control.rect = None;
            }
        }
        self.paint(&mut Frame { layer: frame.layer, bounds: rect, clip: frame.clip }, cx, true);
    }
}

impl Context<'_> {
    pub fn project_menu(&mut self, id: &str, point: Vec2) {
        let mut options = vec![];
        if id != GENERAL_PROJECT_ID {
            options.push(("Rename…".into(), Choice::RenameProject(id.into())));
        }
        options.push(("Edit topic prompt…".into(), Choice::ProjectPrompt(id.into())));
        if id != GENERAL_PROJECT_ID {
            options.push(("Delete topic…".into(), Choice::DeleteProject(id.into())));
        }
        let menu = Menu::new(point, None, None, options, self);
        self.ui.requests.push_back(Request::Menu(Box::new(menu)));
    }
    pub fn chat_menu(&mut self, id: &str, point: Vec2) {
        let id = id.to_owned();
        let options = if self.model.account.missing_chats.contains(&id) {
            vec![
                ("Copy draft to a new chat (not sent)".into(), Choice::CopyRecoveredDraft(id.clone())),
                ("Forget this local recovery…".into(), Choice::ForgetRecovered(id.clone())),
            ]
        } else {
            vec![
                ("Model…".into(), Choice::AgentSetting(id.clone(), "model".into())),
                ("Thinking…".into(), Choice::AgentSetting(id.clone(), "thinking".into())),
                ("Compact context…".into(), Choice::AgentSetting(id.clone(), "compact".into())),
                ("Codex priority…".into(), Choice::AgentSetting(id.clone(), "fast".into())),
                ("Move to topic  ›".into(), Choice::MoveMenu(id.clone())),
                ("Rename…".into(), Choice::Rename(id.clone())),
                ("Review restored history…".into(), Choice::ReviewRestore(id.clone())),
                ("Clone chat".into(), Choice::Clone(id.clone())),
                ("Release idle runtime".into(), Choice::Sleep(id.clone())),
                ("Delete chat…".into(), Choice::Delete(id.clone())),
            ]
        };
        let menu = Menu::new(point, None, Some(id), options, self);
        self.ui.requests.push_back(Request::Menu(Box::new(menu)));
    }
}
