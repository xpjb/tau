use super::controls::{ButtonStyle, Control, Controls};
use super::{Context, Controller, Event, Frame, Id, Request, Target, UiState, Widget};
use crate::{icons::Icon, render::color};
use sanscale::Rect;
use tau_protocol::*;
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) enum Choice {
    Back,
    Attachments,
    Files,
    Abort,
    Queue(QueueOperation),
}
pub(in crate::app) struct Header {
    pub controls: Controls<Choice>,
    pub title: Control,
    pub attachments_open: bool,
}
impl Header {
    pub fn new() -> Self {
        let id = Id::new();
        Self { controls: Controls::new(id), title: Control::new(id, false), attachments_open: false }
    }
    pub fn hide(&mut self) {
        self.controls.begin();
        self.title.rect = None;
    }
}
impl Widget for Header {
    fn update(&mut self, _dt: f32, cx: &mut Context<'_>) {
        if let Some(point) = self.title.held(cx)
            && let Some(session) = cx.model.account.selected.clone()
        {
            cx.ui.capture = None;
            cx.chat_menu(&session, point);
        }
    }
    fn owns(&self, target: Target, _model: &Controller, _ui: &UiState) -> bool {
        self.title.target == target && self.title.rect.is_some() || self.controls.owns(target)
    }

    fn handle_event(&mut self, event: &Event<'_>, cx: &mut Context<'_>) -> bool {
        if matches!(event,Event::Context(p) if self.title.contains(*p))
            || matches!(event, Event::Up { .. })
                && cx.ui.capture.is_some_and(|c| {
                    c.target == self.title.target && c.touch && !c.dragged && c.started.elapsed().as_millis() >= 450
                })
        {
            let point = if let Event::Context(p) = event { *p } else { cx.ui.capture.unwrap().point };
            cx.ui.capture = None;
            if let Some(session) = cx.model.account.selected.clone() {
                cx.chat_menu(&session, point);
            }
            return true;
        }
        let (handled, choice) = self.controls.event(event, cx);
        if let Some(choice) = choice {
            match choice {
                Choice::Back => cx.ui.requests.push_back(Request::Back),
                Choice::Attachments => cx.ui.requests.push_back(Request::Attachments(!self.attachments_open)),
                Choice::Files => cx.ui.requests.push_back(Request::Files),
                Choice::Abort => {
                    if let Some(session_id) = cx.model.account.selected.clone() {
                        let result = cx.model.control(ClientCommand::Abort { session_id });
                        cx.report(result);
                    }
                }
                Choice::Queue(operation) => {
                    if let Some(session_id) = cx.model.account.selected.clone() {
                        let generation = cx.model.chats[&session_id].feed.generation.clone();
                        let result =
                            cx.model.control(ClientCommand::QueueControl { session_id, generation, operation });
                        cx.report(result);
                    }
                }
            }
            return true;
        }
        if handled { return true; }
        let handled = self.title.handle(event, cx, false);
        if self.title.take_click() && let Some(session) = cx.model.account.sessions.iter()
            .find(|s| Some(&s.id) == cx.model.account.selected.as_ref() && s.status == SessionStatus::Error) {
            cx.model.notice = Some(session.detail.as_deref().unwrap_or("The agent stopped without an error detail").into());
        }
        handled
    }
    fn visit_perframe(&mut self, frame: &mut Frame<'_>, cx: &mut Context<'_>) {
        self.controls.begin();
        let Some(session) = cx.model.account.selected.clone() else {
            return;
        };
        let s = cx.ui.scale;
        let b = frame.bounds;
        let chrome = &mut *frame.layer;
        let wide = cx.ui.size.0 as f32 / s >= 760.;
        let header = Rect::new(b.x, b.y, b.width, 56. * s);
        chrome.rect(header, color(0x0e141b));
        chrome.rect(Rect::new(b.x, b.y + 56. * s, b.width, s), color(0x2a3541));
        if !wide {
            self.controls.button(
                cx,
                chrome,
                Rect::new(b.x + 8. * s, header.y + (header.height - 40. * s) / 2., 40. * s, 40. * s),
                "‹",
                Choice::Back,
                ButtonStyle::Tonal,
                frame.clip,
            );
        }
        let title_x = b.x + if wide { 14. * s } else { 64. * s };
        let summary = cx.model.account.sessions.iter().find(|s| s.id == session).cloned();
        let running = summary.as_ref().is_some_and(|s| s.status == SessionStatus::Running);
        let queue = &cx.model.chats[&session].feed.queue;
        let paused = queue.paused;
        let run_hint = match queue.control.as_ref().map(|c| (c.action.as_str(), c.status.as_str())) {
            Some(("prefix", "waiting")) => "Run-through pending",
            Some(("pause", "waiting")) => "Pause pending",
            Some(("prefix", "applied")) => "Finishing run-through",
            _ => "Working",
        };
        let failed = summary.as_ref().is_some_and(|s| s.status == SessionStatus::Error);
        let connected = cx.model.epoch.is_some();
        let title_width = (b.x + b.width - if running || paused { 152. * s } else { 108. * s } - title_x).max(1.);
        self.title.rect = Some(Rect::new(title_x, b.y, title_width, header.height));
        self.title.clip = frame.clip;
        self.title.highlight(chrome, cx.ui, false);
        let title = summary
            .as_ref()
            .map(|s| {
                if s.starter {
                    "New chat"
                } else if s.title.is_empty() {
                    "Unnamed chat"
                } else {
                    &s.title
                }
            })
            .unwrap_or("Chat");
        cx.services.renderer.label(
            chrome,
            title,
            Rect::new(title_x, b.y + 8. * s, title_width, 22. * s),
            16. * s,
            color(0xe5eaf0),
            true,
        );
        cx.services.renderer.label(
            chrome,
            if cx.model.is_creating(&session) {
                if cx.model.epoch.is_none() { "Saved locally · offline" } else { "Creating…" }
            } else if cx.model.epoch.is_none() {
                "Offline"
            } else if failed {
                summary.as_ref().and_then(|s| s.detail.as_deref()).unwrap_or("Error · tap for details")
            } else if running {
                summary.as_ref().and_then(|s| s.detail.as_deref()).unwrap_or(run_hint)
            } else if paused {
                "Paused · resume needed"
            } else {
                "Ready"
            },
            Rect::new(title_x, b.y + 30. * s, title_width, 18. * s),
            12. * s,
            color(if !connected { 0xfbbf24 } else if failed { 0xf87171 } else if paused && !running { 0xfbbf24 } else { 0x4ade80 }),
            false,
        );
        self.controls.icon(
            cx,
            chrome,
            Rect::new(
                b.x + b.width - if running || paused { 96. * s } else { 52. * s },
                header.y + (header.height - 40. * s) / 2.,
                40. * s,
                40. * s,
            ),
            Icon::Attachments,
            22.,
            Choice::Attachments,
            if self.attachments_open { ButtonStyle::Primary } else { ButtonStyle::Tonal },
            true,
            frame.clip,
        );
        if running || paused {
            let (icon, action) = if running {
                (Icon::Stop, Choice::Abort)
            } else {
                (
                    Icon::Play,
                    Choice::Queue(QueueOperation::Resume {
                        run_id: cx.model.chats[&session].feed.queue.run_id.clone(),
                    }),
                )
            };
            self.controls.icon(
                cx,
                chrome,
                Rect::new(b.x + b.width - 52. * s, header.y + (header.height - 40. * s) / 2., 40. * s, 40. * s),
                icon,
                20.,
                action,
                ButtonStyle::Tonal,
                connected,
                frame.clip,
            );
        }
        self.controls.icon(
            cx,
            chrome,
            Rect::new(
                b.x + b.width - if running || paused { 140. * s } else { 96. * s },
                header.y + (header.height - 40. * s) / 2.,
                40. * s,
                40. * s,
            ),
            Icon::Folder,
            22.,
            Choice::Files,
            ButtonStyle::Tonal,
            true,
            frame.clip,
        );
        self.controls.finish(cx);
    }
}
