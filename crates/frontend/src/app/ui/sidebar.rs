use super::{Context, Controller, DialogSpec, Event, Frame, Id, Request, Target, TopicEdit, UiState, Widget};
use super::{
    controls::{ButtonStyle, Controls},
    scroll::ScrollState,
};
use crate::{app::Info, icons::Icon, render::color};
use sanscale::Rect;
use std::time::Instant;
use tau_protocol::*;
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) enum Choice {
    Select(String),
    New,
    Settings,
    Info(Info),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) enum TopicChoice {
    Select(String),
    New,
}
pub(in crate::app) struct Sidebar {
    pub controls: Controls<Choice>,
    pub projects: ProjectTabs,
    pub scroll: ScrollState,
    binding: Option<(String, Option<String>)>,
}
pub(in crate::app) struct ProjectTabs {
    pub controls: Controls<TopicChoice>,
    pub scroll: ScrollState,
    pub revealed: String,
    pub revealed_position: Option<usize>,
}
impl Sidebar {
    pub fn new() -> Self {
        let id = Id::new();
        Self {
            controls: Controls::new(id),
            projects: ProjectTabs::new(),
            scroll: ScrollState::new(id, false),
            binding: None,
        }
    }
    pub fn hide(&mut self) {
        self.controls.begin();
        self.scroll.rect = Rect::new(0., 0., 0., 0.);
        self.projects.controls.begin();
        self.projects.scroll.rect = self.scroll.rect;
    }
    pub fn hints(&self) -> impl Iterator<Item = (Rect, &Info)> {
        self.controls.hints()
    }
}
impl ProjectTabs {
    fn new() -> Self {
        let id = Id::new();
        Self {
            controls: Controls::new(id),
            scroll: ScrollState::new(id, true),
            revealed: String::new(),
            revealed_position: None,
        }
    }
}
impl Widget for Sidebar {
    fn update(&mut self, dt: f32, cx: &mut Context<'_>) {
        self.projects.update(dt, cx);
        self.scroll.update(dt, cx);
        if let Some((Choice::Select(id), point)) = self.controls.held(cx) {
            self.scroll.stop();
            cx.ui.capture = None;
            cx.chat_menu(&id, point);
        }
    }
    fn owns(&self, target: Target, model: &Controller, ui: &UiState) -> bool {
        self.controls.items.iter().any(|(_, b, choice)| {
            b.control.target == target
                && b.control.rect.is_some()
                && match choice {
                    Choice::Select(id) | Choice::Info(Info::CacheTtl(id)) => model
                        .account
                        .sessions
                        .iter()
                        .any(|s| &s.id == id && s.project_id == model.account.selected_project),
                    _ => true,
                }
        }) || self.scroll.target == target
            || self.projects.owns(target, model, ui)
    }

    fn handle_event(&mut self, event: &Event<'_>, cx: &mut Context<'_>) -> bool {
        if self.binding.as_ref().is_none_or(|(identity, lineage)| {
            identity != &cx.model.identity || lineage != &cx.model.account.source_lineage
        }) {
            return false;
        }
        if self.scroll.bar_event(event, cx) {
            return true;
        }
        if self.projects.dispatch(event, cx) {
            return true;
        }
        if let Some((Choice::Select(id), point)) = self.controls.context(event, cx) {
            self.scroll.stop();
            cx.ui.capture = None;
            cx.chat_menu(&id, point);
            return true;
        }
        let (handled, choice) = self.controls.event(event, cx);
        match choice {
            Some(Choice::Select(id)) => cx.ui.requests.push_back(Request::Select(id)),
            Some(Choice::New) => cx.ui.requests.push_back(Request::NewChat),
            Some(Choice::Settings) => cx.ui.requests.push_back(Request::Open(DialogSpec::Connection)),
            Some(Choice::Info(info)) => {
                if let Some((rect, _)) = self.hints().find(|(_, hint)| **hint == info) {
                    cx.ui.requests.push_back(Request::Tip { info, rect });
                }
            }
            None => {}
        }
        self.scroll.event(event, handled, cx)
    }
    fn visit_perframe(&mut self, frame: &mut Frame<'_>, cx: &mut Context<'_>) {
        let binding = (cx.model.identity.clone(), cx.model.account.source_lineage.clone());
        if self.binding.as_ref() != Some(&binding) {
            cx.ui.detach(self.controls.id);
            cx.ui.detach(self.projects.controls.id);
            *self = Self::new();
            self.binding = Some(binding);
        }
        let s = cx.ui.scale;
        self.controls.begin();
        let b = frame.bounds;
        let layer = &mut *frame.layer;
        layer.rect(b, color(0x0e141b));
        layer.rect(Rect::new(b.x + b.width - s, b.y, s, b.height), color(0x2a3541));
        let indicator = Rect::new(b.x + 72. * s, b.y + 22. * s, 28. * s, 34. * s);

        let control = &mut self.controls.place(Choice::Info(Info::Connection), indicator, frame.clip, false).control;
        control.info = Some(Info::Connection);
        control.corners = Some([8. * s; 4]);
        control.highlight(layer, cx.ui, false);
        layer.rounded_rect(
            Rect::new(b.x + 82. * s, b.y + 35. * s, 8. * s, 8. * s),
            4. * s,
            color(cx.model.health.color(Instant::now())),
        );
        cx.services.renderer.label(
            layer,
            "Tau",
            Rect::new(b.x + 16. * s, b.y + 20. * s, b.width - 140. * s, 36. * s),
            30. * s,
            color(0xe5eaf0),
            true,
        );
        self.controls.icon(
            cx,
            layer,
            Rect::new(b.x + b.width - 56. * s, b.y + 16. * s, 40. * s, 40. * s),
            Icon::Gear,
            22.,
            Choice::Settings,
            ButtonStyle::Quiet,
            true,
            frame.clip,
        );
        self.controls.button(
            cx,
            layer,
            Rect::new(b.x + 16. * s, b.y + 84. * s, b.width - 32. * s, 40. * s),
            "New chat",
            Choice::New,
            ButtonStyle::Primary,
            frame.clip,
        );
        // Reserve the sidebar's rightmost column for its separator. The
        // scrolling tabs (including the clipped add tab) must not paint over it.
        self.projects.visit_perframe(
            &mut Frame { layer, bounds: Rect::new(b.x, b.y + 140. * s, b.width - s, 34. * s), clip: frame.clip },
            cx,
        );
        let clip = Rect::new(b.x, b.y + 182. * s, b.width, (b.height - 190. * s).max(0.));
        self.scroll.rect = clip;
        let sessions = cx
            .model
            .account
            .sessions
            .iter()
            .filter(|c| c.project_id == cx.model.account.selected_project)
            .cloned()
            .collect::<Vec<_>>();
        let sessions = sessions.iter();
        self.scroll.max = (sessions.clone().count() as f32 * 90. * s - clip.height).max(0.);
        self.scroll.value = self.scroll.value.min(self.scroll.max);
        for (i, session) in sessions.enumerate() {
            let y = clip.y + i as f32 * 90. * s - self.scroll.value;
            let rect = Rect::new(b.x + 8. * s, y, b.width - 16. * s, 84. * s);
            if y + rect.height < clip.y || y > clip.y + clip.height {
                continue;
            }
            let selected = cx.model.account.selected.as_ref() == Some(&session.id);
            let targeted = cx.ui.menu_chat.as_ref() == Some(&session.id);
            layer.clipped_rounded_rect(rect, 12. * s,
                color(if targeted { 0x35415a } else if selected { 0x303a66 } else { 0x0e141b }), clip);
            let control = &mut self.controls.place(Choice::Select(session.id.clone()), rect, clip, false).control;
            control.corners = Some([12. * s; 4]);
            control.highlight(layer, cx.ui, false);
            let rect = crate::render::intersect(rect, clip);

            let unread = cx.model.unread(session);
            let title = if session.starter || cx.model.is_creating(&session.id) {
                "New chat"
            } else if session.title.is_empty() {
                "Unnamed chat"
            } else {
                &session.title
            };
            cx.services.renderer.clipped_label(
                layer,
                title,
                Rect::new(rect.x + 12. * s, y + 10. * s, rect.width - 56. * s, 22. * s),
                16. * s,
                color(0xe5eaf0),
                unread || selected,
                clip,
            );
            if let Some(model) = &session.model {
                cx.services.renderer.clipped_label(
                    layer,
                    &format!("{}/{}", model.provider, model.model_id),
                    Rect::new(rect.x + 12. * s, y + 38. * s, rect.width - 24. * s, 18. * s),
                    12. * s,
                    color(0xb7c2ce),
                    false,
                    clip,
                );
            }
            let status = format!(
                "{}{}",
                if unread { "●  " } else { "" },
                if cx.model.create_needs_retry(&session.id) {
                    "Needs attention"
                } else if cx.model.quick_start(&session.id)
                    && !matches!(session.status, SessionStatus::Running | SessionStatus::Error)
                    && !cx.model.chats.get(&session.id).is_some_and(|c| c.feed.queue.paused) {
                    ""
                } else {
                    match session.status {
                        SessionStatus::Running => "Working",
                        SessionStatus::Error => "Error",
                        _ if cx.model.chats.get(&session.id).is_some_and(|c| c.feed.queue.paused) => "Paused",
                        SessionStatus::Idle => "Ready",
                        SessionStatus::Sleeping => "Sleeping",
                    }
                }
            );
            cx.services.renderer.clipped_label(
                layer,
                &status,
                Rect::new(rect.x + 12. * s, y + 58. * s, rect.width - 24. * s, 18. * s),
                12. * s,
                color(if session.status == SessionStatus::Running { 0x67d4ff } else { 0x82909f }),
                false,
                clip,
            );
            let ring = Rect::new(rect.x + rect.width - 33. * s, y + 11. * s, 18. * s, 18. * s);
            let (ratio, tint) = cx.model.cache_ttl(session).meter();
            cx.services.renderer.clipped_icon(&cx.services.gpu, layer, Icon::CacheTtl(ratio), ring, tint, clip);
            let target = crate::render::intersect(Rect::new(ring.x - 7. * s, ring.y - 7. * s, 32. * s, 32. * s), clip);
            if target.height > 0. {
                let info = Info::CacheTtl(session.id.clone());
                let control = &mut self.controls.place(Choice::Info(info.clone()), target, clip, true).control;
                control.info = Some(info);
                control.highlight(layer, cx.ui, false);
            }
        }
        if self.scroll.max == 0.
            && !cx.model.account.sessions.iter().any(|c| c.project_id == cx.model.account.selected_project)
        {
            cx.services.renderer.clipped_label(
                layer,
                "No chats in this topic yet",
                Rect::new(b.x + 20. * s, clip.y + 20. * s, b.width - 40. * s, 40. * s),
                13. * s,
                color(0x82909f),
                false,
                clip,
            );
        }
        self.scroll.paint(layer, cx);
        self.controls.finish(cx);
    }
}
impl Widget for ProjectTabs {
    fn update(&mut self, dt: f32, cx: &mut Context<'_>) {
        self.scroll.update(dt, cx);
        if let Some((TopicChoice::Select(id), point)) = self.controls.held(cx) {
            self.scroll.stop();
            cx.ui.capture = None;
            cx.project_menu(&id, point);
        }
    }
    fn owns(&self, target: Target, model: &Controller, _ui: &UiState) -> bool {
        self.scroll.target == target
            || self.controls.items.iter().any(|(_, b, choice)| {
                b.control.target == target
                    && b.control.rect.is_some()
                    && match choice {
                        TopicChoice::Select(id) => model.account.projects.iter().any(|p| &p.id == id),
                        TopicChoice::New => true,
                    }
            })
    }

    fn handle_event(&mut self, event: &Event<'_>, cx: &mut Context<'_>) -> bool {
        if let Some((TopicChoice::Select(id), point)) = self.controls.context(event, cx) {
            self.scroll.stop();
            cx.ui.capture = None;
            cx.project_menu(&id, point);
            return true;
        }
        let (handled, choice) = self.controls.event(event, cx);
        match choice {
            Some(TopicChoice::Select(id)) => cx.ui.requests.push_back(Request::Project(id)),
            Some(TopicChoice::New) => cx.ui.requests.push_back(Request::Open(DialogSpec::Topic(TopicEdit::New))),
            None => {}
        }
        self.scroll.event(event, handled, cx)
    }
    fn visit_perframe(&mut self, frame: &mut Frame<'_>, cx: &mut Context<'_>) {
        let b = frame.bounds;
        let layer = &mut *frame.layer;
        self.scroll.rect = b;
        self.controls.begin();
        let s = cx.ui.scale;
        let projects = &cx.model.account.projects;
        let style = sanscale::Style {
            chain: cx.services.renderer.faces.prose[1],
            wrap_em: None,
            align: sanscale::Align::Left,
            line_spacing: 1.,
        };
        let widths = projects
            .iter()
            .map(|p| {
                let text = cx
                    .services
                    .renderer
                    .text
                    .shape_transient(&p.name, &style)
                    .map_or(70. * s, |block| cx.services.renderer.text.measure(block).width_em() * 13. * s);
                (text + 24. * s).clamp(56. * s, 220. * s)
            })
            .collect::<Vec<_>>();
        self.scroll.max = (widths.iter().sum::<f32>() + 40. * s - b.width).max(0.);
        let selected_position = projects.iter().position(|p| p.id == cx.model.account.selected_project);
        if self.revealed != cx.model.account.selected_project || self.revealed_position != selected_position {
            // A selected topic may jump left when a chat inside it is bumped.
            // Keep it in view without undoing intentional scrolling on every frame.
            self.revealed = cx.model.account.selected_project.clone();
            self.revealed_position = selected_position;
            if let Some(i) = selected_position {
                let left = widths[..i].iter().sum::<f32>();
                let right = left + widths[i];
                if left < self.scroll.value {
                    self.scroll.value = left;
                } else if right > self.scroll.value + b.width {
                    self.scroll.value = right - b.width;
                }
            }
        }
        self.scroll.value = self.scroll.value.clamp(0., self.scroll.max);
        let mut x = b.x + 4. * s - self.scroll.value;
        for (p, w) in projects.iter().zip(widths) {
            let r = Rect::new(x, b.y, w, b.height);
            let hit = crate::render::intersect(r, b);
            if hit.width > 0. {
                let selected = p.id == cx.model.account.selected_project;
                let control = &mut self.controls.place(TopicChoice::Select(p.id.clone()), r, b, false).control;
                control.corners = Some([4. * s; 4]);
                control.highlight(layer, cx.ui, false);
                let label = Rect::new(x + 8. * s, b.y + 8. * s, w - 24. * s, 18. * s);
                cx.services.renderer.clipped_label(
                    layer,
                    &p.name,
                    label,
                    13. * s,
                    color(if selected { 0x67d4ff } else { 0xb7c2ce }),
                    selected,
                    crate::render::intersect(label, b),
                );
                if cx.model.project_unread(&p.id) {
                    layer.clipped_rounded_rect(
                        Rect::new(x + w - 12. * s, b.y + 14. * s, 5. * s, 5. * s),
                        3. * s,
                        color(0x67d4ff),
                        b,
                    );
                }
                if selected {
                    layer.clipped_rounded_rect(
                        Rect::new(x + 8. * s, b.y + b.height - 3. * s, w - 16. * s, 3. * s),
                        1.5 * s,
                        color(0x67d4ff),
                        b,
                    );
                }
            }
            x += w;
        }
        let add = Rect::new(x, b.y, 36. * s, b.height);
        let hit = crate::render::intersect(add, b);
        if hit.width > 0. {
            let control = &mut self.controls.place(TopicChoice::New, add, b, false).control;
            control.corners = Some([8. * s; 4]);
            control.highlight(layer, cx.ui, false);
            cx.services.renderer.clipped_label(
                layer,
                "+",
                Rect::new(x + 10. * s, b.y + 3. * s, 24. * s, 28. * s),
                22. * s,
                color(0x67d4ff),
                false,
                b,
            );
        }
        layer.rect(Rect::new(b.x, b.y + b.height, b.width, s), color(0x2a3541));
        self.controls.finish(cx);
    }
}
