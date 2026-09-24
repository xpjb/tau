use crate::{
    clock,
    controller::Controller,
    details::{Line as DetailLine, Tools},
    editor::Editor,
    icons::Icon,
    render::{Interaction, Layer, Renderer, color, contains, contains_rounded},
    scroll::{Autoscroll, Drag, Lane, Scrollbar, Wheel},
    store::{Settings, Store},
    tooltip::Tooltip,
    transport::Wake,
};
use anyhow::Result;
use chad::{RenderContext, wgpu};
use sanscale::{Rect, Vec2};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    time::Instant,
};
mod ripple;
use ripple::Ripple;
use tau_protocol::*;

mod projects;

#[derive(Clone)]
enum Action {
    Select(String),
    New,
    SelectProject(String),
    NewProject,
    RenameProject(String),
    ProjectPrompt(String),
    DeleteProject(String),
    RemoveProject(DeleteProjectMode),
    MoveMenu(String),
    ContextBack,
    MoveChat(String, String),
    Noop,
    Settings,
    ModelSettings,
    ResetModels,
    ToggleQuickModel(String),
    ChooseModel(String, String),
    Usage,
    Info(Info),
    Back,
    Send,
    Abort,
    Tail,
    DismissNotice,
    Focus(Option<usize>),
    Confirm,
    CancelModal,
    DaemonSettings,
    SettingsSection(usize),
    SettingsField(bool),
    SettingToggle,
    SettingReset,
    AgentSetting(String, String),
    AgentCommand(String, String),
    Rename(String),
    Delete(String),
    Clone(String),
    Sleep(String),
    Fork(String),
    Copy(String),
    CopyDetails(String, Vec<String>),
    CopySelection,
    Link(String),
    Attach,
    RemoveFile(String),
    Toggle(String, bool),
    Restore(String),
    Dismiss(String),
    Queue(QueueOperation),
    EditQueue(String, u64, String),
    Attachment(String, String, String, bool),
    CancelDownload(String),
    Export(PathBuf, String),
    Zoom(f32),
    Fit,
    Suggest(String),
}
#[derive(Clone, PartialEq, Eq)]
enum Info {
    Connection,
    CacheTtl(String),
}
#[derive(Clone)]
enum ModalKind {
    Settings,
    NewProject(String),
    RenameProject(Project),
    ProjectPrompt(Project),
    DeleteProject(Project),
    DeleteProjectChoice(Project),
    Models,
    Rename(String),
    Delete(String),
    Daemon,
    AgentCommand(String, String),
    QueueEdit(String, u64),
    ConfirmLink(String),
}
struct Modal {
    kind: ModalKind,
    title: String,
    fields: Vec<(String, Editor, bool)>,
    options: Vec<(String, Action)>,
}
struct Hit {
    rect: Rect,
    action: Action,
}
#[derive(Clone)]
struct Row {
    details: Vec<DetailLine>,
    header: bool,
    key: String,
    title: String,
    timestamp: String,
    sender: EventRole,
    source: String,
    user: bool,
    error: bool,
    actions: Vec<(String, Action)>,
    attachment: Option<(String, ChatAttachment)>,
}
impl Row {
    fn joins(&self, next: Option<&Self>) -> bool {
        next.is_some_and(|row| self.sender == row.sender)
    }
}
struct Placed {
    key: String,
    top: f32,
    height: f32,
}
#[derive(Clone)]
struct ContextMenu {
    at: Vec2,
    section: Option<String>,
    chat: Option<String>,
    options: Vec<(String, Action)>,
    selected: usize,
    scroll: f32,
    parent: Option<Box<ContextMenu>>,
}
struct MessageArea {
    key: String,
    rect: Rect,
    corners: [f32; 4],
    clip: Rect,
    options: Vec<(String, Action)>,
}
impl MessageArea {
    fn contains(&self, point: Vec2) -> bool {
        contains(self.clip, point) && contains_rounded(self.rect, self.corners, point)
    }
}
struct DetailArea {
    key: String,
    rect: Rect,
    corners: [f32; 4],
    clip: Rect,
}
impl DetailArea {
    fn contains(&self, point: Vec2) -> bool {
        contains(self.clip, point) && contains_rounded(self.rect, self.corners, point)
    }
}
struct Pointer {
    id: u64,
    start: Vec2,
    last: Vec2,
    at: Instant,
    started: Instant,
    dragged: bool,
    touch: bool,
}
pub enum PlatformAction {
    Copy(String),
    Paste,
    PickFile {
        identity: String,
        session: String,
    },
    OpenUrl(String),
    Export(PathBuf, String),
    Edit {
        title: String,
        value: String,
        secret: bool,
        single_line: bool,
    },
    Background,
}
struct Viewer {
    path: PathBuf,
    name: String,
    zoom: f32,
    pan: Vec2,
}
pub struct App {
    pub controller: Controller,
    renderer: Renderer,
    size: (u32, u32),
    origin: Vec2,
    scale: f32,
    hits: Vec<Hit>,
    composer: Editor,
    focus: Option<Option<usize>>,
    modal: Option<Modal>,
    context_menu: Option<ContextMenu>,
    context_rect: Rect,
    menu_viewport: Rect,
    message_areas: Vec<MessageArea>,
    detail_areas: Vec<DetailArea>,
    ripple: Option<Ripple>,
    chat_areas: Vec<(Rect, String)>,
    project_areas: Vec<(Rect, String)>,
    projects_rect: Rect,
    list_rect: Rect,
    project_scroll: f32,
    max_project_scroll: f32,
    project_velocity: f32,
    revealed_project: String,
    saving_project: Option<String>,
    usage: Tooltip,
    info_tip: Tooltip,
    info_target: Info,
    info_areas: Vec<(Rect, Info)>,
    composer_session: Option<String>,
    show_chats: bool,
    waiting_settings: bool,
    daemon_draft: Option<crate::daemon_settings::Draft>,
    saving_settings: Option<String>,
    connecting: bool,
    scroll: f32,
    max_scroll: f32,
    list_scroll: f32,
    max_list_scroll: f32,
    wheel: Option<Wheel>,
    autoscroll: Option<Autoscroll>,
    scrollbars: Vec<Scrollbar>,
    scroll_drag: Option<Drag>,
    expansion_pin: Option<(String, f32)>,
    expansion_positions: HashMap<String, f32>,
    history_attempt: Option<(String, String, u64, u64)>,
    horizontal: f32,
    max_horizontal: f32,
    transcript: Rect,
    placed: Vec<Placed>,
    placed_session: Option<String>,
    pointer: Option<Pointer>,
    hover: Option<Vec2>,
    pinch: Option<(u64, Vec2)>,
    velocity: f32,
    viewer: Option<Viewer>,
    viewer_image: Option<Rect>,
    selecting: bool,
    field_selection: Option<Rect>,
    platform: Vec<PlatformAction>,
    pub mobile: bool,
    dirty: bool,
}
impl App {
    pub fn new(ctx: &impl RenderContext, store: Store, wake: Wake, mobile: bool) -> Result<Self> {
        let controller = Controller::new(store, wake)?;
        let composer_session = controller.account.selected.clone();
        let composer = Editor::composer(
            controller
                .selected()
                .map(|c| c.local.draft.clone())
                .unwrap_or_default(),
        );
        let show_chats = composer_session.is_none();
        let needs_setup = controller.settings.url().is_err();
        let mut app = Self {
            controller,
            renderer: Renderer::new(ctx).map_err(anyhow::Error::msg)?,
            size: ctx.size(),
            origin: Vec2::new(0., 0.),
            scale: 1.,
            hits: vec![],
            composer,
            focus: None,
            modal: None,
            context_menu: None,
            context_rect: Rect::new(0., 0., 0., 0.),
            menu_viewport: Rect::new(0., 0., 0., 0.),
            message_areas: vec![],
            detail_areas: vec![],
            ripple: None,
            chat_areas: vec![],
            project_areas: vec![],
            projects_rect: Rect::new(0., 0., 0., 0.),
            list_rect: Rect::new(0., 0., 0., 0.),
            project_scroll: 0.,
            max_project_scroll: 0.,
            project_velocity: 0.,
            revealed_project: String::new(),
            saving_project: None,
            usage: Tooltip::default(),
            info_tip: Tooltip::default(),
            info_target: Info::Connection,
            info_areas: vec![],
            composer_session,
            show_chats,
            waiting_settings: false,
            daemon_draft: None,
            saving_settings: None,
            connecting: false,
            scroll: 0.,
            max_scroll: 0.,
            list_scroll: 0.,
            max_list_scroll: 0.,
            wheel: None,
            autoscroll: None,
            scrollbars: vec![],
            scroll_drag: None,
            expansion_pin: None,
            expansion_positions: HashMap::new(),
            history_attempt: None,
            horizontal: 0.,
            max_horizontal: 0.,
            transcript: Rect::new(0., 0., 0., 0.),
            placed: vec![],
            placed_session: None,
            pointer: None,
            hover: None,
            pinch: None,
            velocity: 0.,
            viewer: None,
            viewer_image: None,
            selecting: false,
            field_selection: None,
            platform: vec![],
            mobile,
            dirty: true,
        };
        if needs_setup {
            app.apply(Action::Settings)?;
        }
        Ok(app)
    }
    pub fn resize(&mut self, size: (u32, u32), scale: f32, origin: Vec2) {
        if self.size != size || self.scale != scale || self.origin != origin {
            self.cancel_pointer();
            self.revealed_project.clear();
            self.size = size;
            self.scale = scale;
            self.origin = origin;
            self.dirty = true;
        }
    }
    #[cfg(not(target_os = "android"))]
    pub fn needs_redraw(&self) -> bool {
        self.dirty
    }
    #[cfg(not(target_os = "android"))]
    pub fn hover(&mut self, point: Option<Vec2>) {
        let enabled = self.modal.is_none() && self.viewer.is_none() && self.context_menu.is_none();
        if enabled
            && let Some((rect, target)) =
                point.and_then(|p| self.info_areas.iter().find(|(r, _)| contains(*r, p)))
        {
            if self.info_target != *target {
                self.info_tip = Tooltip::default();
                self.info_target = target.clone();
            }
            self.info_tip.region = *rect;
        }
        self.usage
            .hover(enabled && point.is_some_and(|p| self.usage.contains(p)));
        self.info_tip
            .hover(enabled && point.is_some_and(|p| self.info_tip.contains(p)));
        if enabled && point.is_some_and(|p| contains(self.info_tip.region, p)) {
            self.usage.dismiss();
        } else if enabled && point.is_some_and(|p| contains(self.usage.region, p)) {
            self.info_tip.dismiss();
        }
        let old = self
            .hover
            .and_then(|p| self.hits.iter().rev().find(|h| contains(h.rect, p)))
            .map(|h| h.rect);
        let new = point
            .and_then(|p| self.hits.iter().rev().find(|h| contains(h.rect, p)))
            .map(|h| h.rect);
        if let Some(auto) = &mut self.autoscroll
            && let Some(point) = point
        {
            auto.pointer = point;
            self.dirty = true;
        }
        let on_bar = |p: Option<Vec2>| {
            p.is_some_and(|p| self.scrollbars.iter().any(|b| contains(b.track, p)))
        };
        self.dirty |= on_bar(self.hover) != on_bar(point);
        if self.modal.is_none() && self.viewer.is_none() && self.context_menu.is_none() {
            self.dirty |= self.hover.and_then(|p| self.section_at(p).map(|(key, _)| key))
                != point.and_then(|p| self.section_at(p).map(|(key, _)| key));
        }
        self.hover = point;
        if self.context_menu.as_ref().is_some_and(|m| m.parent.is_none())
            && let Some(Action::MoveMenu(id)) = point.and_then(|p| self.hits.iter().rev().find(|h| contains(h.rect, p))).map(|h| h.action.clone()) {
            self.move_menu(&id);
        }
        self.dirty |= old != new;
    }
    #[cfg(not(target_os = "android"))]
    pub fn cursor(&self) -> chad::winit::window::CursorIcon {
        use chad::winit::window::CursorIcon;
        if let Some(auto) = &self.autoscroll {
            return if auto.speed(self.scale) < 0. {
                CursorIcon::NResize
            } else if auto.speed(self.scale) > 0. {
                CursorIcon::SResize
            } else {
                CursorIcon::NsResize
            };
        }
        if self.scroll_drag.is_some() {
            return CursorIcon::Default;
        }
        let Some(point) = self.hover else {
            return CursorIcon::Default;
        };
        if self.modal.is_none()
            && self.viewer.is_none()
            && self.context_menu.is_none()
            && (self.info_tip.contains_card(point) || self.usage.contains_card(point))
        {
            return CursorIcon::Default;
        }
        if self.modal.is_none()
            && self.viewer.is_none()
            && self.scrollbars.iter().any(|b| contains(b.track, point))
        {
            return CursorIcon::Default;
        }
        if let Some(hit) = self.hits.iter().rev().find(|h| contains(h.rect, point)) {
            return if matches!(hit.action, Action::Focus(_)) {
                CursorIcon::Text
            } else {
                CursorIcon::Pointer
            };
        }
        if self.modal.is_none() && self.viewer.is_none() {
            if self.renderer.hit_link(point).is_some() {
                return CursorIcon::Pointer;
            }
            if contains(self.transcript, point) && self.renderer.nearest_text(point).is_some() {
                return CursorIcon::Text;
            }
        }
        CursorIcon::Default
    }
    pub fn actions(&mut self) -> Vec<PlatformAction> {
        std::mem::take(&mut self.platform)
    }
    pub fn report(&mut self, result: Result<()>) {
        if let Err(e) = result {
            self.controller.notice = Some(e.to_string());
        }
        self.dirty = true;
    }
    pub fn tick(&mut self, dt: f32) -> bool {
        let visible = (self.size.0 as f32 / self.scale >= 760. || !self.show_chats) && self.modal.is_none() && self.viewer.is_none();
        if let Err(error) = self.controller.viewing(visible) { self.controller.notice = Some(error.to_string()); }
        self.dirty |= self.controller.poll();
        self.dirty |= self.usage.tick();
        self.dirty |= self.info_tip.tick();
        if self.connecting && self.controller.epoch.is_some() {
            self.connecting = false;
            self.modal = None;
            self.focus = None;
            self.dirty = true;
        }
        self.project_result();
        let selected = self.controller.account.selected.clone();
        if selected != self.composer_session {
            // A selection changed outside the click path (e.g. a server reply).
            // Persist the old chat's last measured anchor before discarding it.
            if let Some(previous) = &self.composer_session
                && self.placed_session.as_deref() == Some(previous.as_str())
                && let Err(error) = self.controller.save_chat(previous) {
                self.controller.notice = Some(error.to_string());
            }
            self.placed.clear();
            self.placed_session = None;
            self.cancel_pointer();
            self.history_attempt = None;
            self.context_menu = None;
            self.usage = Tooltip::default();
            self.composer_session = selected;
            self.scroll = 0.;
            self.horizontal = 0.;
            self.velocity = 0.;
            self.composer = Editor::composer(
                self.controller
                    .selected()
                    .map(|c| c.local.draft.clone())
                    .unwrap_or_default(),
            );
            self.show_chats = self.composer_session.is_none();
            self.dirty = true;
        } else if let Some(chat) = self.controller.selected()
            && self.composer.value != chat.local.draft
        {
            self.composer = Editor::composer(chat.local.draft.clone());
            self.dirty = true;
        }
        if self.waiting_settings
            && let Some(document) = self.controller.daemon_settings.clone()
        {
            self.waiting_settings = false;
            let result = crate::daemon_settings::Draft::new(
                &document,
                self.controller.identity.clone(),
            );
            match result {
                Ok(draft) => {
                    self.daemon_draft = Some(draft);
                    let result = self.load_setting_field();
                    self.report(result);
                }
                Err(error) => self.report(Err(error)),
            }
        }
        if let Some(request) = &self.saving_settings {
            if self
                .controller
                .settings_result
                .as_ref()
                .is_some_and(|(id, _)| id == request)
            {
                let ok = self.controller.settings_result.take().unwrap().1;
                self.saving_settings = None;
                if ok
                    && let (Some(draft), Some(document)) =
                        (&mut self.daemon_draft, &self.controller.daemon_settings)
                {
                    draft.revision = document.revision;
                    self.controller.notice = Some("Settings saved".into());
                }
                self.dirty = true;
            } else if self.controller.epoch.is_none() {
                self.saving_settings = None;
                self.controller.notice =
                    Some("Save unconfirmed. Reload before saving again; it was not resent.".into());
                self.dirty = true;
            }
        }
        if self.modal.is_some() || self.viewer.is_some() {
            self.usage.dismiss();
            self.info_tip.dismiss();
            self.autoscroll = None;
            self.wheel = None;
        }
        if let Some(mut wheel) = self.wheel.take() {
            let (value, max) = self.scroll_value(wheel.lane);
            let (next, settled) = wheel.step(value, max, self.scale);
            let lane = wheel.lane;
            if !settled {
                self.wheel = Some(wheel);
            }
            self.set_scroll(lane, next);
            self.dirty = true;
        }
        if let Some(auto) = &self.autoscroll {
            let next =
                (self.scroll + auto.speed(self.scale) * dt.min(0.05)).clamp(0., self.max_scroll);
            if (next - self.scroll).abs() > 0.001 {
                self.set_scroll(Lane::Transcript, next);
                self.dirty = true;
            }
        }
        if self.field_selection.is_some()
            && let Some(point) = self.pointer.as_ref().filter(|p| p.dragged).map(|p| p.last)
            && let Some((editor, renderer)) = self.editor_and_renderer()
        {
            let moved = editor.drag_scroll(&mut renderer.text, renderer.faces.prose[0], point, dt);
            self.dirty |= moved;
        }
        if self.selecting
            && let Some(p) = &self.pointer
            && p.dragged
        {
            let y = p.last.y;
            let margin = 16. * self.scale;
            let speed = if y < self.transcript.y + margin {
                (y - self.transcript.y - margin) * 20.
            } else if y > self.transcript.y + self.transcript.height - margin {
                (y - self.transcript.y - self.transcript.height + margin) * 20.
            } else {
                0.
            };
            let next = (self.scroll
                + speed.clamp(-1800. * self.scale, 1800. * self.scale) * dt.min(0.05))
            .clamp(0., self.max_scroll);
            if (next - self.scroll).abs() > 0.001 {
                self.set_scroll(Lane::Transcript, next);
                self.dirty = true;
            }
        }
        if self.pointer.is_none() && self.velocity.abs() > 4. {
            let old = self.scroll;
            self.scroll = (self.scroll + self.velocity * dt.min(0.05)).clamp(0., self.max_scroll);
            self.velocity *= (-9. * dt).exp();
            if (old - self.scroll).abs() < 0.1 {
                self.velocity = 0.;
            }
            self.remember_scroll();
            self.dirty = true;
        }
        if self.pointer.is_none() && self.project_velocity.abs() > 4. {
            let old = self.project_scroll;
            self.project_scroll = (old + self.project_velocity * dt.min(0.05)).clamp(0., self.max_project_scroll);
            self.project_velocity *= (-9. * dt).exp();
            if (old - self.project_scroll).abs() < 0.1 { self.project_velocity = 0.; }
            self.dirty = true;
        }
        if let Some(point) = self.pointer.as_ref().filter(|p| p.touch && !p.dragged && p.started.elapsed().as_millis() >= 450).map(|p| p.start)
            && self.modal.is_none() && self.context_menu.is_none() && self.viewer.is_none()
            && (self.project_areas.iter().any(|(r,_)| contains(*r, point)) || self.chat_areas.iter().any(|(r,_)| contains(*r, point))) {
            self.context_at(point);
        }
        let waiting_hold = self.pointer.as_ref().is_some_and(|p| p.touch && !p.dragged && p.started.elapsed().as_millis() < 450)
            && self.modal.is_none() && self.context_menu.is_none() && self.viewer.is_none();
        if let Some(ripple) = &self.ripple {
            let now = Instant::now();
            if ripple.finished(now) {
                self.ripple = None;
                self.dirty = true;
            } else if ripple.animating(now) {
                self.dirty = true;
            }
        }
        let dirty = self.dirty;
        self.dirty = false;
        dirty || self.velocity.abs() > 4. || self.project_velocity.abs() > 4. || waiting_hold
    }
    fn section_at(&self, point: Vec2) -> Option<(&str, Rect)> {
        self.detail_areas.iter().rev().find(|a| a.contains(point))
            .map(|a| (a.key.as_str(), a.rect))
            .or_else(|| self.message_areas.iter().rev().find(|a| a.contains(point))
                .map(|a| (a.key.as_str(), a.rect)))
    }
    pub fn save(&mut self) -> Result<()> {
        self.remember_scroll();
        if let Some(id) = &self.controller.account.selected {
            self.controller.save_chat(id)?;
        }
        Ok(())
    }
    fn remember_scroll(&mut self) {
        if let Some(id) = self.controller.account.selected.clone()
            && self.placed_session.as_deref() == Some(id.as_str())
            && let Some(anchor) = self.placed.iter().find(|r| r.top + r.height >= self.scroll)
            && let Some(chat) = self.controller.chats.get_mut(&id)
        {
            chat.local.position.key = Some(anchor.key.clone());
            chat.local.position.offset = (self.scroll - anchor.top) / self.scale;
            chat.local.position.follow = self.expansion_pin.is_none()
                && !self
                    .wheel
                    .as_ref()
                    .is_some_and(|w| w.lane == Lane::Transcript)
                && self.autoscroll.is_none()
                && !self
                    .scroll_drag
                    .as_ref()
                    .is_some_and(|d| d.lane == Lane::Transcript)
                && self.max_scroll - self.scroll < self.scale;
        }
    }
    pub fn back(&mut self) {
        if self.info_tip.pinned
            || self.info_tip.progress > 0.
            || self.usage.pinned
            || self.usage.progress > 0.
        {
            self.info_tip.dismiss();
            self.usage.dismiss();
            self.dirty = true;
            return;
        }
        if self.context_menu.is_some() {
            self.context_menu = self.context_menu.take().and_then(|m| m.parent.map(|p| *p));
            self.dirty = true;
            return;
        }
        self.focus = None;
        if self.viewer.take().is_some() {
            self.viewer_image = None;
        } else if self.modal.is_some() {
            self.activate(Action::CancelModal);
        } else if !self.show_chats && self.size.0 as f32 / self.scale < 760. {
            self.show_chats = true;
        } else {
            self.platform.push(PlatformAction::Background);
        }
        self.dirty = true;
    }
    fn scroll_value(&self, lane: Lane) -> (f32, f32) {
        match lane {
            Lane::Transcript => (self.scroll, self.max_scroll),
            Lane::Sidebar => (self.list_scroll, self.max_list_scroll),
            Lane::Projects => (self.project_scroll, self.max_project_scroll),
            Lane::Horizontal => (self.horizontal, self.max_horizontal),
        }
    }
    fn set_scroll(&mut self, lane: Lane, value: f32) {
        match lane {
            Lane::Transcript => {
                self.scroll = value.clamp(0., self.max_scroll);
                self.remember_scroll();
            }
            Lane::Sidebar => self.list_scroll = value.clamp(0., self.max_list_scroll),
            Lane::Projects => self.project_scroll = value.clamp(0., self.max_project_scroll),
            Lane::Horizontal => self.horizontal = value.clamp(0., self.max_horizontal),
        }
    }
    pub fn cancel_autoscroll(&mut self) -> bool {
        let active = self.autoscroll.take().is_some();
        self.dirty |= active;
        active
    }
    #[cfg(not(target_os = "android"))]
    pub fn middle(&mut self, pressed: bool, point: Vec2) {
        if pressed {
            if self.cancel_autoscroll() {
                return;
            }
            if self.modal.is_some()
                || self.viewer.is_some()
                || !contains(self.transcript, point)
                || self.max_scroll <= 0.
            {
                return;
            }
            self.cancel_pointer();
            self.autoscroll = Some(Autoscroll {
                anchor: point,
                pointer: point,
                pressed: Some(Instant::now()),
            });
        } else if let Some(auto) = &mut self.autoscroll {
            // Quick click latches; a held press scrolls only until release, as in Tau 1.
            if auto
                .pressed
                .take()
                .is_some_and(|at| at.elapsed().as_millis() >= 220)
            {
                self.autoscroll = None;
            }
        }
        self.remember_scroll();
        self.dirty = true;
    }
    #[cfg(not(target_os = "android"))]
    pub fn wheel(&mut self, amount: f32, horizontal: bool, point: Vec2) {
        if self.context_menu.is_some() && contains(self.context_rect, point) {
            self.scroll_menu(amount);
            return;
        }
        self.context_menu = None;
        self.project_velocity = 0.;
        self.usage.dismiss();
        self.info_tip.dismiss();
        self.cancel_autoscroll();
        self.expansion_pin = None;
        self.history_attempt = None;
        self.velocity = 0.;
        if self.viewer.is_none() && let Some(field) = self.field_at(point) {
            let editor = match field {
                None => &mut self.composer,
                Some(i) => &mut self.modal.as_mut().unwrap().fields[i].1,
            };
            editor.wheel(&mut self.renderer.text, self.renderer.faces.prose[0], amount, horizontal);
            self.dirty = true;
            return;
        }
        if let Some(v) = &mut self.viewer {
            v.zoom = (v.zoom * (-amount * 0.002).exp()).clamp(1., 16.);
        } else if self.modal.is_none() {
            let lane = if contains(self.projects_rect, point) {
                Lane::Projects
            } else if horizontal {
                Lane::Horizontal
            } else if self.show_chats
                || self.size.0 as f32 / self.scale >= 760.
                    && point.x < self.origin.x + 300. * self.scale
            {
                Lane::Sidebar
            } else {
                Lane::Transcript
            };
            let (value, max) = self.scroll_value(lane);
            if let Some(wheel) = &mut self.wheel
                && wheel.lane == lane
            {
                wheel.target = (wheel.target + amount).clamp(0., max);
            } else {
                self.wheel = Some(Wheel {
                    lane,
                    target: (value + amount).clamp(0., max),
                    last: Instant::now(),
                });
            }
        }
        self.dirty = true;
    }
    fn scrollbar(&mut self, layer: &mut Layer, lane: Lane, viewport: Rect) {
        let (value, max) = self.scroll_value(lane);
        if let Some(bar) = Scrollbar::new(lane, viewport, value, max, self.scale) {
            let active = self.scroll_drag.as_ref().is_some_and(|d| d.lane == lane)
                || self.hover.is_some_and(|p| contains(bar.track, p));
            let width = if active { 8. } else { 6. } * self.scale;
            layer.rounded_rect(
                Rect::new(
                    bar.track.x + (bar.track.width - width) * 0.5,
                    bar.thumb.y,
                    width,
                    bar.thumb.height,
                ),
                width * 0.5,
                color(if active { 0xa0aaba } else { 0x596575 }),
            );
            self.scrollbars.push(bar);
        }
    }
    fn history_near_top(&mut self, session: &str) {
        if self.modal.is_some() || self.viewer.is_some() || self.scroll > 180. * self.scale {
            return;
        }
        let feed = &self.controller.chats[session].feed;
        if !feed.synchronized || feed.loading {
            return;
        }
        if let (Some(before), Some(epoch)) = (feed.before, self.controller.epoch) {
            let attempt = (session.to_owned(), feed.generation.clone(), before, epoch);
            if self.history_attempt.as_ref() == Some(&attempt) {
                return;
            }
            self.history_attempt = Some(attempt);
            let result = self.controller.history();
            self.report(result);
        }
    }
    pub fn press(&mut self, id: u64, point: Vec2, touch: bool) {
        if self.context_menu.is_some() && !contains(self.context_rect, point) {
            self.context_menu = None;
            self.dirty = true;
            return;
        }
        if self.modal.is_none()
            && self.viewer.is_none()
            && self.context_menu.is_none()
            && (self.info_tip.contains_card(point) || self.usage.contains_card(point))
        {
            // Informational cards must not activate the list/message behind them.
            self.dirty = true;
            return;
        }
        if !contains(self.usage.region, point) {
            self.usage.dismiss();
        }
        if !contains(self.info_tip.region, point) {
            self.info_tip.dismiss();
        }
        self.wheel = None;
        self.expansion_pin = None;
        self.history_attempt = None;
        self.velocity = 0.;
        self.project_velocity = 0.;
        self.ripple = None;
        if self.pointer.is_some() {
            if self.viewer.is_some() && touch {
                self.pinch = Some((id, point));
            }
            return;
        }
        self.selecting = false;
        self.field_selection = None;
        if self.modal.is_none()
            && self.viewer.is_none()
            && self.context_menu.is_none()
            && let Some(bar) = self
                .scrollbars
                .iter()
                .find(|b| contains(b.track, point))
                .copied()
        {
            if contains(bar.thumb, point) {
                self.scroll_drag = Some(Drag {
                    lane: bar.lane,
                    grab: (point.y - bar.thumb.y) / bar.thumb.height,
                });
            } else {
                let (value, _) = self.scroll_value(bar.lane);
                self.set_scroll(
                    bar.lane,
                    value
                        + if point.y < bar.thumb.y {
                            -bar.track.height * 0.9
                        } else {
                            bar.track.height * 0.9
                        },
                );
            }
            self.pointer = Some(Pointer {
                id,
                start: point,
                last: point,
                at: Instant::now(),
                started: Instant::now(),
                dragged: true,
                touch,
            });
            self.dirty = true;
            return;
        }
        if !touch && self.viewer.is_none() {
            if let Some(hit) = self.hits.iter().rev().find(|h| contains(h.rect, point)) {
                // Controls take priority over transcript selection beneath them.
                if let Action::Focus(field) = hit.action {
                    let rect = hit.rect;
                    self.cancel_preedit();
                    self.focus = Some(field);
                    self.field_selection = Some(rect);
                    self.field_hit(point, false);
                }
            } else if self.modal.is_none()
                && self.context_menu.is_none()
                && contains(self.transcript, point)
                && let Some(caret) = self.renderer.nearest_text(point)
            {
                self.renderer.begin_selection(caret);
                self.selecting = true;
                self.focus = None;
            }
        }
        self.pointer = Some(Pointer {
            id,
            start: point,
            last: point,
            at: Instant::now(),
            started: Instant::now(),
            dragged: false,
            touch,
        });
        self.ripple = if self.modal.is_none() && self.viewer.is_none() && self.context_menu.is_none() {
            self.section_at(point).map(|(key, rect)| Ripple::new(key.to_owned(), rect, point))
        } else { None };
        self.dirty = true;
    }
    pub fn motion(&mut self, id: u64, point: Vec2) {
        let Some(p) = &mut self.pointer else {
            return;
        };
        if let Some((second, other)) = &mut self.pinch
            && let Some(viewer) = &mut self.viewer
        {
            let distance = |a: Vec2, b: Vec2| ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt();
            let old = distance(p.last, *other);
            if id == *second {
                *other = point;
            } else if id == p.id {
                p.last = point;
            } else {
                return;
            }
            let new = distance(p.last, *other);
            if old > 1. {
                viewer.zoom = (viewer.zoom * new / old).clamp(1., 16.);
            }
            p.dragged = true;
            self.dirty = true;
            return;
        }
        if p.id != id {
            return;
        }
        if let Some(drag) = &self.scroll_drag {
            p.last = point;
            if let Some(bar) = self
                .scrollbars
                .iter()
                .find(|b| b.lane == drag.lane)
                .copied()
            {
                let value = bar.value_at(point.y, drag.grab);
                self.set_scroll(bar.lane, value);
            }
            self.dirty = true;
            return;
        }
        if self.selecting {
            if let Some(caret) = self.renderer.nearest_text(point) {
                self.renderer.extend_selection(caret);
            }
            p.dragged |=
                (point.x - p.start.x).abs() + (point.y - p.start.y).abs() > 4. * self.scale;
            if p.dragged { self.ripple = None; }
            p.last = point;
            self.dirty = true;
            return;
        }
        if self.field_selection.is_some() {
            p.dragged = true;
            p.last = point;
            self.field_hit(point, true);
            self.dirty = true;
            return;
        }
        let dy = point.y - p.last.y;
        let dx = point.x - p.last.x;
        p.dragged |= (point.x - p.start.x).abs() + (point.y - p.start.y).abs() > 7. * self.scale;
        if p.dragged {
            if self.context_menu.is_some() {
                p.last = point;
                p.at = Instant::now();
                self.scroll_menu(-dy);
                return;
            }
            if let Some(v) = &mut self.viewer {
                v.pan.x += dx;
                v.pan.y += dy;
            } else if self.modal.is_none() {
                if contains(self.projects_rect, p.start) {
                    self.project_scroll = (self.project_scroll - dx).clamp(0., self.max_project_scroll);
                    if p.touch { self.project_velocity = (-dx / p.at.elapsed().as_secs_f32().max(0.008)).clamp(-3000. * self.scale, 3000. * self.scale); }
                } else if contains(self.list_rect, p.start) {
                    self.list_scroll = (self.list_scroll - dy).clamp(0., self.max_list_scroll);
                } else if contains(self.transcript, p.start)
                    && self.max_horizontal > 0.
                    && (point.x - p.start.x).abs() > 1.5 * (point.y - p.start.y).abs()
                {
                    self.horizontal = (self.horizontal - dx).clamp(0., self.max_horizontal);
                    self.velocity = 0.;
                } else if contains(self.transcript, p.start) {
                    self.scroll = (self.scroll - dy).clamp(0., self.max_scroll);
                    if p.touch {
                        self.velocity = (-dy / p.at.elapsed().as_secs_f32().max(0.008))
                            .clamp(-3000. * self.scale, 3000. * self.scale);
                    }
                }
            }
        }
        if p.dragged { self.ripple = None; }
        p.last = point;
        p.at = Instant::now();
        self.remember_scroll();
        self.dirty = true;
    }
    pub fn release(&mut self, id: u64, point: Vec2) {
        if self.pinch.take().is_some() {
            self.pointer = None;
            self.dirty = true;
            return;
        }
        if self.pointer.as_ref().is_none_or(|p| p.id != id) {
            return;
        }
        let p = self.pointer.take().unwrap();
        self.scroll_drag = None;
        if !p.dragged {
            if self.viewer.is_some() {
                if let Some(hit) = self.hits.iter().rev().find(|h|
                    contains(h.rect, point) && contains(h.rect, p.start)) {
                    self.activate(hit.action.clone());
                } else if self.viewer_image.is_none_or(|image|
                    !contains(image, p.start) && !contains(image, point)) {
                    self.viewer = None;
                    self.viewer_image = None;
                }
            } else if p.touch
                && p.started.elapsed().as_millis() > 450
                && self.context_menu.is_none()
                && (self.project_areas.iter().any(|(r,_)| contains(*r, point) && contains(*r, p.start))
                    || contains(self.transcript, point)
                    || self
                        .chat_areas
                        .iter()
                        .any(|(r, _)| contains(*r, point) && contains(*r, p.start)))
            {
                self.context_at(point);
            } else if let Some(hit) = self
                .hits
                .iter()
                .rev()
                .find(|h| contains(h.rect, point) && contains(h.rect, p.start))
            {
                self.activate(hit.action.clone());
            } else if self.context_menu.is_some() {
                // Empty menu space never activates the transcript behind it.
            } else if let Some(link) = self.renderer.hit_link(point) {
                self.activate(Action::Link(link));
            } else if p.touch
                && p.at.elapsed().as_millis() > 450
                && let Some((key, _)) = self.renderer.hit_text(point)
            {
                let end = self.renderer.messages[&key].source.len();
                self.renderer.begin_selection(crate::render::TextPoint {
                    key: key.clone(),
                    byte: 0,
                });
                self.renderer
                    .extend_selection(crate::render::TextPoint { key, byte: end });
            }
        }
        if p.dragged { self.ripple = None; }
        else if let Some(ripple) = &mut self.ripple { ripple.release(); }
        self.selecting = false;
        self.field_selection = None;
        if p.at.elapsed().as_millis() > 150 {
            self.velocity = 0.;
            self.project_velocity = 0.;
        }
        let result = self.save();
        self.report(result);
    }
    pub fn cancel_pointer(&mut self) {
        self.context_menu = None;
        self.usage.dismiss();
        self.info_tip.dismiss();
        self.dirty = true;
        self.hover = None;
        self.autoscroll = None;
        self.wheel = None;
        self.scroll_drag = None;
        self.expansion_pin = None;
        self.pointer = None;
        self.ripple = None;
        self.pinch = None;
        self.velocity = 0.;
        self.project_velocity = 0.;
        self.selecting = false;
        self.field_selection = None;
    }
    fn editor_and_renderer(&mut self) -> Option<(&mut Editor, &mut Renderer)> {
        let editor = match self.focus? {
            None => &mut self.composer,
            Some(i) => &mut self.modal.as_mut()?.fields.get_mut(i)?.1,
        };
        Some((editor, &mut self.renderer))
    }
    fn field_hit(&mut self, point: Vec2, extend: bool) {
        if let Some((editor, renderer)) = self.editor_and_renderer() {
            editor.hit(&mut renderer.text, renderer.faces.prose[0], point, extend);
        }
    }
    fn field_at(&self, point: Vec2) -> Option<Option<usize>> {
        if let Some(modal) = &self.modal {
            modal.fields.iter().rposition(|(_, e, _)| e.contains(point)).map(Some)
        } else {
            self.composer.contains(point).then_some(None)
        }
    }
    pub fn ime_rect(&self) -> Option<Rect> {
        let editor = match self.focus? {
            None => &self.composer,
            Some(i) => &self.modal.as_ref()?.fields.get(i)?.1,
        };
        editor.ime_rect(&self.renderer.text)
    }
    pub fn cancel_preedit(&mut self) {
        if let Some(e) = self.editor() && e.composing() {
            e.preedit(String::new(), None);
            self.dirty = true;
        }
    }
    pub fn composing(&self) -> bool {
        match self.focus {
            Some(None) => self.composer.composing(),
            Some(Some(i)) => self.modal.as_ref().and_then(|m| m.fields.get(i)).is_some_and(|(_, e, _)| e.composing()),
            None => false,
        }
    }
    fn editor(&mut self) -> Option<&mut Editor> {
        match self.focus? {
            None => Some(&mut self.composer),
            Some(i) => self.modal.as_mut()?.fields.get_mut(i).map(|(_, e, _)| e),
        }
    }
    pub fn input(&mut self, value: &str) {
        if self.editor().is_some_and(|e| e.replace(value)) { self.edited(); }
        self.dirty = true;
    }
    #[cfg(target_os = "android")]
    pub fn native_edit(&mut self, value: String) {
        if self.editor().is_some_and(|e| e.replace_all(&value)) { self.edited(); }
        self.dirty = true;
    }
    fn edited(&mut self) {
        if matches!(
            self.modal.as_ref().map(|m| &m.kind),
            Some(ModalKind::Settings | ModalKind::Models | ModalKind::Daemon)
        ) {
            self.controller.notice = None;
        }
        if self.focus == Some(None) {
            let result = self.controller.draft(self.composer.value.clone());
            self.report(result);
        }
        self.dirty = true;
    }
    #[cfg(not(target_os = "android"))]
    pub fn preedit(&mut self, text: String, cursor: Option<(usize, usize)>) {
        if let Some(e) = self.editor() {
            e.preedit(text, cursor);
        }
        self.dirty = true;
    }
    pub fn key(&mut self, key: &str, ctrl: bool, shift: bool) {
        // Composition belongs to the IME. Enter must not send the draft, and
        // Escape must not abort the agent, while candidate text is active.
        if self.composing() {
            if key == "Escape" {
                if let Some(e) = self.editor() { e.preedit(String::new(), None); }
                self.dirty = true;
            }
            return;
        }
        if let Some(menu) = &mut self.context_menu {
            match key {
                "Escape" | "ArrowLeft" => {
                    self.context_menu = self.context_menu.take().and_then(|m| m.parent.map(|p| *p));
                }
                "ArrowUp" => {
                    self.hover = None;
                    menu.selected = (menu.selected + menu.options.len() - 1) % menu.options.len()
                }
                "ArrowDown" => {
                    self.hover = None;
                    menu.selected = (menu.selected + 1) % menu.options.len();
                }
                "Enter" | "ArrowRight" => {
                    let action = menu.options[menu.selected].1.clone();
                    if key == "Enter" || matches!(action, Action::MoveMenu(_)) { self.activate(action); }
                }
                _ => {}
            }
            self.reveal_menu_selection();
            self.dirty = true;
            return;
        }
        if key == "Escape"
            && (self.usage.pinned
                || self.usage.progress > 0.
                || self.info_tip.pinned
                || self.info_tip.progress > 0.)
        {
            self.usage.dismiss();
            self.info_tip.dismiss();
            self.dirty = true;
            return;
        }
        self.expansion_pin = None;
        self.wheel = None;
        if let Some(modal) = &self.modal {
            if key == "Tab" && !ctrl && !modal.fields.is_empty() {
                let fields: Vec<_> = self.hits.iter().filter_map(|h| match h.action {
                    Action::Focus(Some(i)) => Some(i), _ => None,
                }).collect();
                if !fields.is_empty() {
                    let current = fields.iter().position(|&i| self.focus == Some(Some(i)));
                    let next = if shift { current.map_or(fields.len() - 1, |i| (i + fields.len() - 1) % fields.len()) }
                        else { current.map_or(0, |i| (i + 1) % fields.len()) };
                    self.focus = Some(Some(fields[next]));
                }
                self.dirty = true;
                return;
            }
            if key == "Enter" && matches!(modal.kind, ModalKind::Settings | ModalKind::Rename(_)) {
                self.activate(Action::Confirm);
                return;
            }
        }
        if key == "Escape" {
            if self.modal.is_some() || self.viewer.is_some() {
                self.back();
            } else {
                self.activate(Action::Abort);
            }
            return;
        }
        if ctrl && key.eq_ignore_ascii_case("v") {
            self.platform.push(PlatformAction::Paste);
            return;
        }
        if ctrl && (key.eq_ignore_ascii_case("c") || key.eq_ignore_ascii_case("x")) {
            if self.focus.is_none() {
                if let Some(text) = self.renderer.selected_text() {
                    self.platform.push(PlatformAction::Copy(text));
                }
                return;
            }
            if let Some(e) = self.editor() {
                let copy = e.selected().to_owned();
                let changed = key.eq_ignore_ascii_case("x") && e.replace("");
                self.platform.push(PlatformAction::Copy(copy));
                if changed { self.edited(); }
                self.dirty = true;
            }
            return;
        }
        if key == "Enter" && !shift && self.focus == Some(None) {
            self.activate(Action::Send);
            return;
        }
        let Some((editor, renderer)) = self.editor_and_renderer() else { return; };
        let changed = editor.key(&mut renderer.text, renderer.faces.prose[0], key, ctrl, shift);
        if changed { self.edited(); }
        self.dirty = true;
    }
    fn activate(&mut self, action: Action) {
        let result = self.apply(action);
        self.report(result);
    }
    fn apply(&mut self, action: Action) -> Result<()> {
        self.cancel_preedit();
        if let Action::MoveMenu(ref id) = action { self.move_menu(id); return Ok(()); }
        if matches!(action, Action::ContextBack) {
            self.context_menu = self.context_menu.take().and_then(|m| m.parent.map(|p| *p));
            return Ok(());
        }
        if matches!(action, Action::Noop) { return Ok(()); }
        self.context_menu = None;
        let selected = self.controller.account.selected.clone();
        match action {
            Action::SelectProject(id) => {
                if self.controller.account.selected_project == id
                    && self.controller.account.selected.as_ref().is_some_and(|chat|
                        self.controller.account.sessions.iter().any(|s| s.id == chat.as_str() && s.project_id == id)) {
                    return Ok(());
                }
                self.save()?;
                self.controller.select_project(&id)?;
                self.list_scroll = 0.;
                self.show_chats = self.controller.account.selected.is_none();
                self.focus = None;
            }
            Action::NewProject | Action::RenameProject(_) | Action::ProjectPrompt(_) | Action::DeleteProject(_) | Action::RemoveProject(_) => self.project_action(action)?,
            Action::MoveChat(session_id, project_id) => {
                self.controller.request(ClientCommand::MoveSession { session_id, project_id })?;
            }
            Action::MoveMenu(_) | Action::ContextBack | Action::Noop => {}
            Action::Select(id) => {
                if selected.as_deref() != Some(id.as_str()) {
                    self.save()?;
                    self.controller.select(&id)?;
                    self.scroll = 0.;
                }
                self.show_chats = false;
                self.focus = Some(None);
            }
            Action::Info(target) => {
                self.usage.dismiss();
                if self.info_target != target {
                    self.info_tip = Tooltip::default();
                    self.info_target = target;
                }
                if let Some((rect, _)) = self
                    .info_areas
                    .iter()
                    .find(|(_, target)| *target == self.info_target)
                {
                    self.info_tip.region = *rect;
                }
                self.info_tip.pinned = !self.info_tip.pinned;
                self.info_tip.suppressed = !self.info_tip.pinned;
            }
            Action::Usage => {
                self.info_tip.dismiss();
                self.usage.pinned = !self.usage.pinned;
                self.usage.suppressed = !self.usage.pinned;
            }
            Action::New => {
                self.controller.new_chat()?;
                self.show_chats = false;
            }
            Action::Back => self.back(),
            Action::ModelSettings => {
                self.controller.notice = None;
                self.modal = Some(Modal {
                    kind: ModalKind::Models,
                    title: "Quick model selection".into(),
                    fields: vec![
                        (
                            "Quick models".into(),
                            Editor::new(self.controller.model_preferences.text()),
                            false,
                        ),
                        (
                            "Search model suggestions".into(),
                            Editor::line(String::new()),
                            false,
                        ),
                    ],
                    options: vec![],
                });
                self.focus = None;
                if let Some(id) = selected
                    && self.controller.epoch.is_some()
                    && !self.controller.chats[&id].commands_loaded
                {
                    self.controller
                        .request(ClientCommand::GetCommands { session_id: id })?;
                }
            }
            Action::ResetModels => {
                if let Some(modal) = &mut self.modal
                    && matches!(modal.kind, ModalKind::Models)
                {
                    modal.fields[0].1 = Editor::new(crate::models::Preferences::default().text());
                }
            }
            Action::ToggleQuickModel(slug) => {
                if let Some(modal) = &mut self.modal
                    && matches!(modal.kind, ModalKind::Models)
                {
                    let mut preferences =
                        crate::models::Preferences::parse(&modal.fields[0].1.value)?;
                    if preferences.slugs.contains(&slug) {
                        preferences.slugs.retain(|s| s != &slug);
                    } else {
                        anyhow::ensure!(
                            preferences.slugs.len() < 12,
                            "Choose at most 12 quick models"
                        );
                        preferences.slugs.push(slug);
                    }
                    modal.fields[0].1 = Editor::new(preferences.text());
                }
            }
            Action::ChooseModel(session, slug) => self.controller.choose_model(&session, &slug)?,
            Action::Settings => {
                self.controller.notice = None;
                self.connecting = false;
                self.modal = Some(Modal {
                    kind: ModalKind::Settings,
                    title: "Connection settings".into(),
                    fields: vec![
                        (
                            "Server URL".into(),
                            Editor::line(self.controller.settings.server_url.clone()),
                            false,
                        ),
                        (
                            "Access token".into(),
                            Editor::line(self.controller.settings.token.clone()),
                            true,
                        ),
                    ],
                    options: vec![
                        ("Connect".into(), Action::Confirm),
                        ("Daemon settings".into(), Action::DaemonSettings),
                        ("Cancel".into(), Action::CancelModal),
                    ],
                });
                self.focus = Some(Some(0));
            }
            Action::Focus(field) => {
                self.focus = Some(field);
                if self.mobile {
                    let (title, value, secret, single_line) = match field {
                        Some(i) => {
                            let (label, e, secret) = &self.modal.as_ref().unwrap().fields[i];
                            (label.clone(), e.value.clone(), *secret, e.single_line)
                        }
                        None => (
                            "Message Tau".into(),
                            self.composer.value.clone(),
                            false,
                            false,
                        ),
                    };
                    self.platform.push(PlatformAction::Edit {
                        title,
                        value,
                        secret,
                        single_line,
                    });
                }
            }
            Action::Confirm => {
                let Some(modal) = self.modal.as_ref() else {
                    return Ok(());
                };
                let values = modal
                    .fields
                    .iter()
                    .map(|(_, e, _)| e.value.clone())
                    .collect::<Vec<_>>();
                match modal.kind.clone() {
                    ModalKind::NewProject(_) | ModalKind::RenameProject(_) | ModalKind::ProjectPrompt(_) | ModalKind::DeleteProject(_) | ModalKind::DeleteProjectChoice(_) => {
                        self.confirm_project()?;
                        return Ok(());
                    }
                    ModalKind::Settings => {
                        if self.connecting && self.controller.connection == "Connecting…" {
                            return Ok(());
                        }
                        self.controller.configure(Settings {
                            server_url: values[0].clone(),
                            token: values[1].clone(),
                        })?;
                        self.connecting = true;
                        // Stay on the form until the authenticated protocol hello succeeds.
                        return Ok(());
                    }
                    ModalKind::Models => {
                        self.controller.save_model_preferences(
                            crate::models::Preferences::parse(&values[0])?,
                        )?;
                    }
                    ModalKind::Rename(session_id) => {
                        self.controller.request(ClientCommand::RenameSession {
                            session_id,
                            title: values[0].clone(),
                        })?;
                    }
                    ModalKind::Delete(session_id) => {
                        self.controller
                            .request(ClientCommand::DeleteSession { session_id })?;
                    }
                    ModalKind::Daemon => {
                        if self.saving_settings.is_some() {
                            return Ok(());
                        }
                        self.apply_setting_field()?;
                        let draft = self
                            .daemon_draft
                            .as_ref()
                            .ok_or_else(|| anyhow::anyhow!("Load settings first"))?;
                        anyhow::ensure!(
                            draft.identity == self.controller.identity,
                            "Server changed; reload settings first"
                        );
                        self.controller.notice = None;
                        self.controller.settings_result = None;
                        self.saving_settings =
                            Some(self.controller.request(ClientCommand::SetSettings {
                                revision: draft.revision,
                                settings: Box::new(draft.document()?),
                            })?);
                        self.focus = None;
                        return Ok(());
                    }
                    ModalKind::AgentCommand(session, command) => {
                        self.apply(Action::AgentCommand(
                            session,
                            format!("/{command} {}", values[0]),
                        ))?;
                    }
                    ModalKind::QueueEdit(request_id, revision) => {
                        self.apply(Action::Queue(QueueOperation::Edit {
                            request_id,
                            revision,
                            text: values[0].clone(),
                        }))?;
                    }
                    ModalKind::ConfirmLink(url) => self.platform.push(PlatformAction::OpenUrl(url)),
                }
                self.modal = None;
                self.focus = None;
            }
            Action::CancelModal => {
                self.connecting = false;
                self.saving_project = None;
                self.waiting_settings = false;
                self.daemon_draft = None;
                self.saving_settings = None;
                self.modal = None;
                self.focus = None;
            }
            Action::DaemonSettings => {
                self.controller.notice = None;
                self.controller.daemon_settings = None;
                self.controller.request(ClientCommand::GetSettings)?;
                self.waiting_settings = true;
                self.daemon_draft = None;
                self.saving_settings = None;
                self.modal = Some(Modal {
                    kind: ModalKind::Daemon,
                    title: "Daemon settings".into(),
                    fields: vec![],
                    options: vec![],
                });
                self.focus = None;
            }
            Action::SettingsSection(section) => {
                self.apply_setting_field()?;
                if let Some(draft) = &mut self.daemon_draft {
                    draft.section = section;
                    draft.field = 0;
                }
                self.load_setting_field()?;
            }
            Action::SettingsField(next) => {
                self.apply_setting_field()?;
                if let Some(draft) = &mut self.daemon_draft {
                    let count = crate::daemon_settings::fields(draft.section).len();
                    draft.field = (draft.field + if next { 1 } else { count - 1 }) % count;
                }
                self.load_setting_field()?;
            }
            Action::SettingToggle => {
                if let (Some(draft), Some(modal)) = (&mut self.daemon_draft, &mut self.modal) {
                    if draft.definition().kind == crate::daemon_settings::Kind::PromptOverride {
                        draft.inherit = !draft.inherit;
                        if draft.inherit {
                            modal.fields[0].1 = Editor::new(draft.default_prompt().into());
                        }
                    } else {
                        modal.fields[0].1 =
                            Editor::line((modal.fields[0].1.value != "true").to_string());
                    }
                    self.focus = None;
                }
            }
            Action::SettingReset => {
                if let Some(draft) = &mut self.daemon_draft { draft.reset()?; }
                self.load_setting_field()?;
            }
            Action::AgentSetting(session, command) => {
                let (label, value) = match command.as_str() {
                    "model" => (
                        "provider/model",
                        self.controller
                            .account
                            .sessions
                            .iter()
                            .find(|s| s.id == session)
                            .and_then(|s| s.model.as_ref())
                            .map(|m| format!("{}/{}", m.provider, m.model_id))
                            .unwrap_or_default(),
                    ),
                    "thinking" => (
                        "off / minimal / low / medium / high / xhigh / max",
                        String::new(),
                    ),
                    "fast" => ("on / off / status", String::new()),
                    _ => ("Optional compaction instructions", String::new()),
                };
                self.modal = Some(Modal {
                    title: format!("Chat {command}"),
                    kind: ModalKind::AgentCommand(session, command),
                    fields: vec![(label.into(), Editor::line(value), false)],
                    options: vec![
                        ("Apply".into(), Action::Confirm),
                        ("Cancel".into(), Action::CancelModal),
                    ],
                });
                self.focus = Some(Some(0));
            }
            Action::AgentCommand(session, text) => {
                self.controller.ensure_chat(&session)?;
                self.controller.control(ClientCommand::Prompt {
                    session_id: session,
                    text,
                })?;
                self.modal = None;
                self.focus = None;
            }
            Action::Rename(id) => {
                let title = self
                    .controller
                    .account
                    .sessions
                    .iter()
                    .find(|s| s.id == id)
                    .map(|s| s.title.clone())
                    .unwrap_or_default();
                self.modal = Some(Modal {
                    kind: ModalKind::Rename(id),
                    title: "Rename chat".into(),
                    fields: vec![("Title".into(), Editor::line(title), false)],
                    options: vec![
                        ("Save".into(), Action::Confirm),
                        ("Cancel".into(), Action::CancelModal),
                    ],
                });
            }
            Action::Delete(id) => {
                self.modal = Some(Modal {
                    kind: ModalKind::Delete(id),
                    title: "Permanently delete this chat and its files?".into(),
                    fields: vec![],
                    options: vec![
                        ("Delete permanently".into(), Action::Confirm),
                        ("Cancel".into(), Action::CancelModal),
                    ],
                })
            }
            Action::Clone(session_id) => {
                self.controller
                    .request(ClientCommand::CloneSession { session_id })?;
            }
            Action::Sleep(session_id) => {
                self.controller
                    .request(ClientCommand::CloseSession { session_id })?;
            }
            Action::Fork(entry_id) => {
                if let Some(id) = selected {
                    self.controller.request(ClientCommand::ForkSession {
                        session_id: id,
                        entry_id,
                    })?;
                }
            }
            Action::Send => self.controller.send_prompt()?,
            Action::Abort => {
                if let Some(id) = selected {
                    self.controller
                        .control(ClientCommand::Abort { session_id: id })?;
                }
            }
            Action::Tail => {
                self.scroll = self.max_scroll;
                self.remember_scroll();
            }
            Action::DismissNotice => self.controller.notice = None,
            Action::Copy(text) => self.platform.push(PlatformAction::Copy(text)),
            Action::CopyDetails(session, ids) => {
                if let Some(chat) = self.controller.chats.get(&session) {
                    let tools = Tools::new(chat.feed.events.values());
                    let group = ids
                        .iter()
                        .filter_map(|id| chat.feed.event(id))
                        .collect::<Vec<_>>();
                    let text = tools.copy(&group);
                    if !text.is_empty() {
                        self.platform.push(PlatformAction::Copy(text));
                    }
                }
            }
            Action::CopySelection => {
                if let Some(text) = self.renderer.selected_text() {
                    self.platform.push(PlatformAction::Copy(text));
                }
            }
            Action::Link(url) => {
                let parsed = url::Url::parse(&url)?;
                anyhow::ensure!(
                    matches!(parsed.scheme(), "https" | "http" | "mailto"),
                    "Only web and mail links can be opened"
                );
                self.modal = Some(Modal {
                    kind: ModalKind::ConfirmLink(url.clone()),
                    title: format!("Open link?\n{url}"),
                    fields: vec![],
                    options: vec![
                        ("Open".into(), Action::Confirm),
                        ("Cancel".into(), Action::CancelModal),
                    ],
                });
            }
            Action::Attach => {
                if let Some(session) = selected {
                    self.platform.push(PlatformAction::PickFile {
                        identity: self.controller.identity.clone(),
                        session,
                    });
                }
            }
            Action::RemoveFile(id) => self.controller.remove_file(&id)?,
            Action::Toggle(key, expanded) => {
                self.wheel = None;
                self.velocity = 0.;
                self.expansion_pin = self
                    .expansion_positions
                    .get(&key)
                    .map(|y| (key.clone(), *y - self.scroll));
                if let Some(id) = selected {
                    let c = self.controller.chats.get_mut(&id).unwrap();
                    if key.starts_with("details:") {
                        c.local.details_default = expanded;
                    }
                    c.local.expansion.insert(key, expanded);
                    c.local.position.follow = false;
                    self.controller.save_chat(&id)?;
                }
            }
            Action::Restore(id) => {
                self.controller.restore_pending(&id)?;
                self.composer =
                    Editor::composer(self.controller.selected().unwrap().local.draft.clone());
            }
            Action::Dismiss(id) => self.controller.dismiss_pending(&id)?,
            Action::Queue(operation) => {
                if let Some(id) = selected {
                    let generation = self.controller.chats[&id].feed.generation.clone();
                    self.controller.control(ClientCommand::QueueControl {
                        session_id: id,
                        generation,
                        operation,
                    })?;
                }
            }
            Action::EditQueue(id, rev, text) => {
                self.modal = Some(Modal {
                    kind: ModalKind::QueueEdit(id, rev),
                    title: "Edit queued message".into(),
                    fields: vec![("Message".into(), Editor::new(text), false)],
                    options: vec![
                        ("Save".into(), Action::Confirm),
                        ("Cancel".into(), Action::CancelModal),
                    ],
                })
            }
            Action::Attachment(session, entry, name, image) => {
                let path = self.controller.download(
                    &session,
                    &entry,
                    if image { 10_000_000 } else { 50_000_000 },
                )?;
                if path.is_file() {
                    if image {
                        self.viewer_image = None;
                        self.viewer = Some(Viewer {
                            path,
                            name,
                            zoom: 1.,
                            pan: Vec2::new(0., 0.),
                        });
                    } else {
                        self.platform.push(PlatformAction::Export(path, name));
                    }
                }
            }
            Action::CancelDownload(key) => self.controller.cancel_download(&key)?,
            Action::Export(path, name) => self.platform.push(PlatformAction::Export(path, name)),
            Action::Zoom(factor) => {
                if let Some(v) = &mut self.viewer {
                    v.zoom = (v.zoom * factor).clamp(1., 16.);
                }
            }
            Action::Fit => {
                if let Some(v) = &mut self.viewer {
                    v.zoom = 1.;
                    v.pan = Vec2::new(0., 0.);
                }
            }
            Action::Suggest(text) => {
                self.composer = Editor::composer(text.clone());
                self.controller.draft(text)?;
            }
        }
        Ok(())
    }
    pub fn frame(&mut self, ctx: &impl RenderContext, view: &wgpu::TextureView) {
        let s = self.scale;
        let bounds = Rect::new(
            self.origin.x,
            self.origin.y,
            self.size.0 as f32,
            self.size.1 as f32,
        );
        let input = Interaction {
            hover: self.pointer.as_ref().map(|p| p.last).or(self.hover),
            pressed: self
                .pointer
                .as_ref()
                .filter(|p| !p.dragged)
                .map(|p| p.start),
            held: self.pointer.is_some(),
        };
        let background_input =
            if self.modal.is_none() && self.viewer.is_none() && self.context_menu.is_none() {
                input
            } else {
                Interaction::default()
            };
        let mut main = Layer::new(background_input);
        let mut body = Layer::new(background_input);
        let mut chrome = Layer::new(background_input);
        let mut overlay = Layer::new(input);
        self.hits.clear();
        self.scrollbars.clear();
        self.message_areas.clear();
        self.detail_areas.clear();
        self.chat_areas.clear();
        self.project_areas.clear();
        self.projects_rect = Rect::new(0.,0.,0.,0.);
        self.list_rect = Rect::new(0.,0.,0.,0.);
        self.transcript = Rect::new(0.,0.,0.,0.);
        self.info_areas.clear();
        self.usage.region = Rect::new(0., 0., 0., 0.);
        self.info_tip.region = Rect::new(0., 0., 0., 0.);
        self.composer.hide();
        if let Some(modal) = &mut self.modal {
            for (_, editor, _) in &mut modal.fields { editor.hide(); }
        }
        self.renderer.clear_scenes();
        self.viewer_image = None;
        main.rect(bounds, color(0x0e141b));
        let wide = bounds.width / s >= 760.;
        let side = if wide { 300. * s } else { 0. };
        if wide || self.show_chats {
            self.sidebar(
                ctx,
                &mut main,
                Rect::new(
                    bounds.x,
                    bounds.y,
                    if wide { side } else { bounds.width },
                    bounds.height,
                ),
            );
        }
        if wide || !self.show_chats {
            self.chat(
                ctx,
                &mut body,
                &mut chrome,
                Rect::new(
                    bounds.x + side,
                    bounds.y,
                    bounds.width - side,
                    bounds.height,
                ),
            );
        }
        if let Some((rect, _)) = self
            .info_areas
            .iter()
            .find(|(_, target)| *target == self.info_target)
        {
            self.info_tip.region = *rect;
        }
        if self.info_tip.region.width <= 0. {
            self.info_tip.hover(false);
            self.info_tip.dismiss();
        }
        self.usage_frame(&mut overlay, bounds);
        self.info_frame(&mut overlay, bounds);
        self.context_frame(&mut overlay, bounds);
        if let Some(auto) = &self.autoscroll {
            let a = auto.anchor;
            let radius = 13.5 * s;
            overlay.rounded_rect(
                Rect::new(a.x - radius, a.y - radius, radius * 2., radius * 2.),
                radius,
                color(0x67d4ff),
            );
            overlay.rounded_rect(
                Rect::new(
                    a.x - radius + 2. * s,
                    a.y - radius + 2. * s,
                    (radius - 2. * s) * 2.,
                    (radius - 2. * s) * 2.,
                ),
                radius - 2. * s,
                color(0x18212b),
            );
            let icon_size = 24. * s;
            self.renderer.icon(
                ctx,
                &mut overlay,
                Icon::Autoscroll,
                Rect::new(
                    a.x - icon_size / 2.,
                    a.y - icon_size / 2.,
                    icon_size,
                    icon_size,
                ),
                0x67d4ff,
            );
        }
        if let Some(viewer) = &self.viewer {
            self.hits.clear();
            overlay.rect(bounds, color(0x06090d));
            let path = viewer.path.clone();
            let name = viewer.name.clone();
            let zoom = viewer.zoom;
            let pan = viewer.pan;
            match self.renderer.image_size(ctx, &path) {
                Ok((w, h)) => {
                    let fit = (bounds.width / w as f32).min((bounds.height - 100. * s) / h as f32);
                    let width = w as f32 * fit * zoom;
                    let height = h as f32 * fit * zoom;
                    let image = Rect::new(
                        bounds.x + (bounds.width - width) / 2. + pan.x,
                        bounds.y + 60. * s + (bounds.height - 100. * s - height) / 2. + pan.y,
                        width,
                        height,
                    );
                    let clip = Rect::new(
                        bounds.x,
                        bounds.y + 56. * s,
                        bounds.width,
                        bounds.height - 100. * s,
                    );
                    self.viewer_image = Some(crate::render::intersect(image, clip));
                    overlay.images.push((path.clone(), image, clip));
                }
                Err(e) => {
                    self.renderer.label(
                        &mut overlay,
                        &e,
                        Rect::new(
                            bounds.x + 20. * s,
                            bounds.y + 80. * s,
                            bounds.width - 40. * s,
                            100. * s,
                        ),
                        15. * s,
                        color(0xffb4ab),
                        false,
                    );
                }
            }
            let buttons = [
                ("Back", Action::Back),
                ("−", Action::Zoom(0.8)),
                ("Fit", Action::Fit),
                ("+", Action::Zoom(1.25)),
                ("Save", Action::Export(path, name)),
            ];
            for (i, (label, action)) in buttons.into_iter().enumerate() {
                button(
                    &mut self.renderer,
                    &mut overlay,
                    &mut self.hits,
                    Rect::new(
                        bounds.x + (12. + i as f32 * 66.) * s,
                        bounds.y + 8. * s,
                        60. * s,
                        38. * s,
                    ),
                    label,
                    action,
                    s,
                    false,
                );
            }
        }
        if matches!(
            self.modal.as_ref().map(|m| &m.kind),
            Some(ModalKind::Settings)
        ) {
            self.settings_frame(&mut overlay, bounds);
        } else if matches!(
            self.modal.as_ref().map(|m| &m.kind),
            Some(ModalKind::Models)
        ) {
            self.model_settings_frame(&mut overlay, bounds);
        } else if matches!(
            self.modal.as_ref().map(|m| &m.kind),
            Some(ModalKind::Daemon)
        ) {
            self.daemon_settings_frame(&mut overlay, bounds);
        } else if self.modal.as_ref().is_some_and(|m| projects::is_project_modal(&m.kind)) {
            self.project_modal_frame(&mut overlay, bounds);
        } else if self.modal.is_some() {
            self.modal_frame(&mut overlay, bounds);
        }
        if !matches!(
            self.modal.as_ref().map(|m| &m.kind),
            Some(ModalKind::Settings | ModalKind::Models | ModalKind::Daemon)
        ) && !self.modal.as_ref().is_some_and(|m| projects::is_project_modal(&m.kind)) {
            self.notice_frame(&mut overlay, bounds);
        }
        if self.selecting
            && let Some(p) = &self.pointer
            && p.dragged
            && let Some(caret) = self.renderer.nearest_text(p.last)
        {
            self.dirty |= self.renderer.extend_selection(caret);
        }
        self.renderer
            .draw(ctx, view, &[main, body, chrome, overlay]);
    }
    fn sidebar(&mut self, ctx: &impl RenderContext, layer: &mut Layer, b: Rect) {
        let s = self.scale;
        layer.rect(b, color(0x0e141b));
        layer.rect(
            Rect::new(b.x + b.width - s, b.y, s, b.height),
            color(0x2a3541),
        );
        let indicator = Rect::new(b.x + 72. * s, b.y + 22. * s, 28. * s, 34. * s);
        self.info_areas.push((indicator, Info::Connection));
        layer.rounded_rect(
            indicator,
            8. * s,
            layer.control_color(indicator, color(0x0e141b)),
        );
        layer.rounded_rect(
            Rect::new(b.x + 82. * s, b.y + 35. * s, 8. * s, 8. * s),
            4. * s,
            color(self.controller.health.color()),
        );
        self.hits.push(Hit {
            rect: indicator,
            action: Action::Info(Info::Connection),
        });
        self.renderer.label(
            layer,
            "Tau",
            Rect::new(b.x + 16. * s, b.y + 20. * s, b.width - 140. * s, 36. * s),
            30. * s,
            color(0xe5eaf0),
            true,
        );
        self.icon_button(
            ctx,
            layer,
            Rect::new(b.x + b.width - 56. * s, b.y + 16. * s, 40. * s, 40. * s),
            Icon::Gear,
            22.,
            Action::Settings,
            false,
            true,
        );
        button(
            &mut self.renderer,
            layer,
            &mut self.hits,
            Rect::new(b.x + 16. * s, b.y + 84. * s, b.width - 32. * s, 40. * s),
            "New chat",
            Action::New,
            s,
            true,
        );
        self.project_tabs(layer, Rect::new(b.x, b.y + 140. * s, b.width, 34. * s));
        let clip = Rect::new(b.x, b.y + 182. * s, b.width, (b.height - 190. * s).max(0.));
        self.list_rect = clip;
        let sessions = self.controller.account.sessions.iter().filter(|c| c.project_id == self.controller.account.selected_project);
        self.max_list_scroll =
            (sessions.clone().count() as f32 * 90. * s - clip.height).max(0.);
        self.list_scroll = self.list_scroll.min(self.max_list_scroll);
        for (i, session) in sessions.enumerate() {
            let y = clip.y + i as f32 * 90. * s - self.list_scroll;
            let rect = Rect::new(b.x + 8. * s, y, b.width - 16. * s, 84. * s);
            if y + rect.height < clip.y || y > clip.y + clip.height {
                continue;
            }
            let selected = self.controller.account.selected.as_ref() == Some(&session.id);
            let targeted = self
                .context_menu
                .as_ref()
                .is_some_and(|m| m.chat.as_ref() == Some(&session.id));
            layer.clipped_rounded_rect(
                rect,
                12. * s,
                layer.control_color(
                    rect,
                    color(if targeted {
                        0x35415a
                    } else if selected {
                        0x303a66
                    } else {
                        0x0e141b
                    }),
                ),
                clip,
            );
            let rect = crate::render::intersect(rect, clip);
            self.chat_areas.push((rect, session.id.clone()));
            let unread = self.controller.unread(session);
            let title = if session.starter {
                "New chat"
            } else if session.title.is_empty() {
                "Unnamed chat"
            } else {
                &session.title
            };
            self.renderer.clipped_label(
                layer,
                title,
                Rect::new(rect.x + 12. * s, y + 10. * s, rect.width - 56. * s, 22. * s),
                16. * s,
                color(0xe5eaf0),
                unread || selected,
                clip,
            );
            if let Some(model) = &session.model {
                self.renderer.clipped_label(
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
                match session.status {
                    SessionStatus::Running => "Working",
                    SessionStatus::Error => "Error",
                    SessionStatus::Idle => "Ready",
                    SessionStatus::Sleeping => "Sleeping",
                }
            );
            self.renderer.clipped_label(
                layer,
                &status,
                Rect::new(rect.x + 12. * s, y + 58. * s, rect.width - 24. * s, 18. * s),
                12. * s,
                color(if session.status == SessionStatus::Running {
                    0x67d4ff
                } else {
                    0x82909f
                }),
                false,
                clip,
            );
            self.hits.push(Hit {
                rect,
                action: Action::Select(session.id.clone()),
            });
            let ring = Rect::new(rect.x + rect.width - 33. * s, y + 11. * s, 18. * s, 18. * s);
            let (ratio, tint) = self.controller.cache_ttl(session).meter();
            self.renderer
                .clipped_icon(ctx, layer, Icon::CacheTtl(ratio), ring, tint, clip);
            let target = crate::render::intersect(
                Rect::new(ring.x - 7. * s, ring.y - 7. * s, 32. * s, 32. * s),
                clip,
            );
            if target.height > 0. {
                let info = Info::CacheTtl(session.id.clone());
                self.info_areas.push((target, info.clone()));
                self.hits.push(Hit {
                    rect: target,
                    action: Action::Info(info),
                });
            }
        }
        if self.max_list_scroll == 0. && !self.controller.account.sessions.iter().any(|c| c.project_id == self.controller.account.selected_project) {
            self.renderer.clipped_label(layer, "No chats in this topic yet", Rect::new(b.x + 20. * s, clip.y + 20. * s, b.width - 40. * s, 40. * s), 13. * s, color(0x82909f), false, clip);
        }
        self.scrollbar(layer, Lane::Sidebar, clip);
    }
    fn rows(&self, session: &str) -> Vec<Row> {
        let chat = &self.controller.chats[session];
        let tools = Tools::new(chat.feed.events.values());
        let events = chat
            .feed
            .events
            .values()
            .filter(|e| {
                let empty = e.attachment.is_none()
                    && e.error_message.is_none()
                    && !e.is_error
                    && (e.kind == EventKind::Hidden
                        || matches!(e.kind, EventKind::Thinking | EventKind::Text)
                            && e.text.is_empty());
                !empty && !(e.attachment.is_none() && tools.paired_result(e))
            })
            .collect::<Vec<_>>();
        let mut rows = vec![];
        let mut i = 0;
        while i < events.len() {
            let e = events[i];
            let detail = |e: &Event| {
                e.attachment.is_none()
                    && (e.kind == EventKind::Thinking
                        || e.kind == EventKind::Tool
                        || e.role == EventRole::Tool)
            };
            if detail(e) {
                let start = i;
                while i < events.len() && detail(events[i]) {
                    i += 1;
                }
                let group = &events[start..i];
                let details = tools.lines(group, &chat.local);
                rows.push(Row {
                    key: format!("{session}/{}", details[0].key),
                    details,
                    header: false,
                    title: String::new(),
                    timestamp: clock::label(group.iter().find_map(|e| clock::event_ms(e))),
                    sender: EventRole::Assistant,
                    source: String::new(),
                    user: false,
                    error: false,
                    actions: vec![(
                        "Copy message".into(),
                        Action::CopyDetails(
                            session.into(),
                            group.iter().map(|e| e.id.clone()).collect(),
                        ),
                    )],
                    attachment: None,
                });
                continue;
            }
            let user = e.role == EventRole::User;
            let title = if user {
                "You".into()
            } else if let Some(error) = &e.error_message {
                format!("Assistant · {error}")
            } else {
                format!(
                    "{}{}",
                    if e.role == EventRole::Assistant {
                        "Tau"
                    } else {
                        "System"
                    },
                    if e.phase == EventPhase::Live {
                        " · writing"
                    } else if e.phase == EventPhase::Interrupted {
                        " · interrupted"
                    } else {
                        ""
                    }
                )
            };
            let mut actions = vec![("Copy message".into(), Action::Copy(e.text.clone()))];
            if e.phase == EventPhase::Saved {
                actions.push(("Fork here".into(), Action::Fork(e.entry_id.clone())));
            }
            rows.push(Row {
                details: vec![],
                header: e.role == EventRole::System || e.is_error || e.error_message.is_some(),
                key: format!("{session}/{}", e.id),
                title,
                timestamp: clock::label(clock::event_ms(e)),
                sender: if e.role == EventRole::Tool {
                    EventRole::Assistant
                } else {
                    e.role
                },
                source: if user {
                    literal(&e.text)
                } else {
                    e.text.clone()
                },
                user,
                error: e.is_error || e.error_message.is_some(),
                actions,
                attachment: e.attachment.clone().map(|a| (e.entry_id.clone(), a)),
            });
            i += 1;
        }
        for p in &chat.local.pending {
            rows.push(Row {
                details: vec![],
                header: true,
                key: format!("pending:{}", p.request.id),
                title: p.status.label().into(),
                timestamp: clock::label(p.started_at_ms),
                sender: EventRole::User,
                source: literal(&format!(
                    "{}{}",
                    p.text,
                    p.detail
                        .as_ref()
                        .map(|s| format!("\n{s}"))
                        .unwrap_or_default()
                )),
                user: true,
                error: matches!(
                    p.status,
                    crate::store::Delivery::Rejected | crate::store::Delivery::Unconfirmed
                ),
                actions: vec![
                    ("Copy message".into(), Action::Copy(p.text.clone())),
                    (
                        "Restore draft".into(),
                        Action::Restore(p.request.id.clone()),
                    ),
                    ("Dismiss".into(), Action::Dismiss(p.request.id.clone())),
                ],
                attachment: None,
            });
        }
        for (i, q) in chat.feed.queue.requests.iter().enumerate() {
            let state = &chat.feed.queue;
            let mut actions = vec![("Copy message".into(), Action::Copy(q.text.clone()))];
            if state.capabilities.iter().any(|c| c == "queue_edit") {
                actions.push((
                    "Edit".into(),
                    Action::EditQueue(q.request_id.clone(), q.revision, q.text.clone()),
                ));
            }
            if state.capabilities.iter().any(|c| c == "queue_delete") {
                actions.push((
                    "Delete".into(),
                    Action::Queue(QueueOperation::Delete {
                        request_id: q.request_id.clone(),
                        revision: q.revision,
                    }),
                ));
            }
            if state.capabilities.iter().any(|c| c == "queue_run_prefix")
                && let Some(boundary) = state
                    .boundaries
                    .iter()
                    .find(|b| b.as_str() == "reasoning_checkpoint")
                    .or(state.boundaries.first())
            {
                actions.push((
                    "Run through here".into(),
                    Action::Queue(QueueOperation::Prefix {
                        run_id: state.run_id.clone(),
                        requests: state.requests[..=i]
                            .iter()
                            .map(|q| QueueRef {
                                request_id: q.request_id.clone(),
                                revision: q.revision,
                            })
                            .collect(),
                        boundary: boundary.clone(),
                    }),
                ));
            }
            rows.push(Row {
                details: vec![],
                header: true,
                key: format!("queue:{}", q.request_id),
                title: format!("Queued{}", if state.paused { " · held" } else { "" }),
                timestamp: clock::label(q.timestamp_ms),
                sender: EventRole::User,
                source: literal(&q.text),
                user: true,
                error: false,
                actions,
                attachment: None,
            });
        }
        rows
    }
    fn chat(&mut self, ctx: &impl RenderContext, layer: &mut Layer, chrome: &mut Layer, b: Rect) {
        let s = self.scale;
        let paint_at = Instant::now();
        let wide = self.size.0 as f32 / s >= 760.;
        let Some(session) = self.controller.account.selected.clone() else {
            return;
        };
        if !self.controller.chats.contains_key(&session) {
            return;
        }
        let header = Rect::new(b.x, b.y, b.width, 56. * s);
        chrome.rect(header, color(0x0e141b));
        chrome.rect(Rect::new(b.x, b.y + 56. * s, b.width, s), color(0x2a3541));
        if !wide {
            button(
                &mut self.renderer,
                chrome,
                &mut self.hits,
                Rect::new(
                    b.x + 8. * s,
                    header.y + (header.height - 40. * s) / 2.,
                    40. * s,
                    40. * s,
                ),
                "‹",
                Action::Back,
                s,
                false,
            );
        }
        let title_x = b.x + if wide { 14. * s } else { 64. * s };
        let summary = self
            .controller
            .account
            .sessions
            .iter()
            .find(|s| s.id == session)
            .cloned();
        let running = summary
            .as_ref()
            .is_some_and(|s| s.status == SessionStatus::Running);
        let paused = self.controller.chats[&session].feed.queue.paused;
        let title_width =
            (b.x + b.width - if running || paused { 64. * s } else { 12. * s } - title_x).max(1.);
        self.chat_areas.push((
            Rect::new(title_x, b.y, title_width, header.height),
            session.clone(),
        ));
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
        self.renderer.label(
            chrome,
            title,
            Rect::new(title_x, b.y + 8. * s, title_width, 22. * s),
            16. * s,
            color(0xe5eaf0),
            true,
        );
        self.renderer.label(
            chrome,
            if self.controller.epoch.is_none() {
                "Offline"
            } else if summary
                .as_ref()
                .is_some_and(|s| s.status == SessionStatus::Running)
            {
                "Working"
            } else {
                "Ready"
            },
            Rect::new(title_x, b.y + 30. * s, title_width, 18. * s),
            12. * s,
            color(if self.controller.epoch.is_some() {
                0x4ade80
            } else {
                0xfbbf24
            }),
            false,
        );
        let files = self.controller.chats[&session].local.files.clone();
        let width = (b.width - 28. * s).min(900. * s).max(160. * s);
        let x = b.x + (b.width - width) / 2.;
        let editor_h = self
            .composer
            .height(&mut self.renderer, width - 132. * s, 16. * s);
        let queue = &self.controller.chats[&session].feed.queue;
        let controls = queue
            .control
            .as_ref()
            .is_some_and(|c| matches!(c.status.as_str(), "waiting" | "applying"));
        let composer_h = editor_h
            + (44. + if files.is_empty() { 0. } else { 40. } + if controls { 40. } else { 0. }) * s;
        let bottom = b.y + b.height;
        let composer_top = (bottom - composer_h).max(b.y + 80. * s);
        let viewport = Rect::new(
            b.x,
            b.y + 57. * s,
            b.width,
            (composer_top - b.y - 57. * s).max(1.),
        );
        self.transcript = viewport;
        let bubble_width = width * 0.9;
        let text_width = bubble_width - 28. * s;
        let rows = self.rows(&session);
        let quick_models = self.controller.quick_start(&session)
            && !self.controller.model_preferences.slugs.is_empty();
        let quick_h = if quick_models {
            self.quick_models_height(width)
        } else {
            0.
        };
        let quick_top = if quick_models && rows.is_empty() {
            ((viewport.height - quick_h) / 2.).max(12. * s)
        } else {
            12. * s
        };
        let mut placements = vec![];
        let mut y = quick_top + quick_h;

        let mut keys = HashSet::new();
        let mut detail_layouts: HashMap<String, Vec<(f32, f32)>> = HashMap::new();
        self.max_horizontal = 0.;
        for (index, row) in rows.iter().enumerate() {
            let gap = if row.joins(rows.get(index + 1)) {
                0.
            } else {
                12. * s
            };
            if !row.details.is_empty() {
                let mut layout = vec![];
                let mut top = 26. * s;
                for line in &row.details {
                    let key = format!("{session}/{}", line.key);
                    let h = if line.source.is_empty() {
                        if line.toggle.is_some() {
                            28. * s
                        } else {
                            18. * s
                        }
                    } else {
                        keys.insert(key.clone());
                        let h = self.renderer.message_height(
                            &key,
                            &line.source,
                            text_width - line.indent * s,
                            if line.code { 12. / 0.9 * s } else { 12. * s },
                        ) + 6. * s;
                        if let Some(m) = self.renderer.messages.get(&key) {
                            self.max_horizontal = self
                                .max_horizontal
                                .max(m.view.width - (text_width - line.indent * s));
                        }
                        h
                    };
                    layout.push((top, h));
                    top += h;
                }
                let height = top + 8. * s;
                detail_layouts.insert(row.key.clone(), layout);
                placements.push(Placed {
                    key: row.key.clone(),
                    top: y,
                    height,
                });
                y += height + gap;
                continue;
            }
            keys.insert(row.key.clone());
            let text_height = if row.source.is_empty() {
                0.
            } else {
                self.renderer
                    .message_height(&row.key, &row.source, text_width, 16. * s)
            };
            if let Some(message) = self.renderer.messages.get(&row.key) {
                self.max_horizontal = self.max_horizontal.max(message.view.width - text_width);
            }
            let attachment = if let Some((entry, a)) = &row.attachment {
                (96. + if a.kind == AttachmentKind::Image
                    && self
                        .controller
                        .store
                        .attachment_path(&self.controller.identity, &session, entry)
                        .is_file()
                {
                    240.
                } else {
                    0.
                }) * s
            } else {
                0.
            };
            let height = (if row.header { 50. } else { 28. }) * s
                + text_height
                + 12. * s
                + attachment
                + if row.attachment.is_some() {
                    30. * s
                } else {
                    0.
                };
            placements.push(Placed {
                key: row.key.clone(),
                top: y,
                height,
            });
            y += height + gap;
        }
        self.renderer.retain_messages(&keys);
        self.horizontal = self.horizontal.clamp(0., self.max_horizontal);
        self.max_scroll = (y - viewport.height).max(0.);
        if y < viewport.height {
            for p in &mut placements {
                p.top += viewport.height - y;
            }
        }
        let old_scroll = self.scroll;
        self.expansion_positions.clear();
        for (row, p) in rows.iter().zip(&placements) {
            if let Some(layout) = detail_layouts.get(&row.key) {
                for (line, (top, _)) in row.details.iter().zip(layout) {
                    if line.toggle.is_some() {
                        self.expansion_positions
                            .insert(line.key.clone(), p.top + top);
                    }
                }
            }
        }
        let position = &self.controller.chats[&session].local.position;
        // An empty/not-yet-loaded frame must not erase the saved anchor. If the
        // anchor is on an older page, near-top paging can find it before rebasing.
        let can_remember = !placements.is_empty()
            && self.controller.chats[&session].feed.synchronized
            && (position.follow
                || position
                    .key
                    .as_ref()
                    .is_none_or(|key| placements.iter().any(|p| &p.key == key))
                || self.controller.chats[&session].feed.before.is_none());
        if quick_models {
            self.scroll = self.scroll.clamp(0., self.max_scroll);
        } else if position.follow {
            self.scroll = self.max_scroll;
        } else if let Some(key) = &position.key
            && let Some(p) = placements.iter().find(|p| &p.key == key)
        {
            self.scroll = (p.top + position.offset * s).clamp(0., self.max_scroll);
        } else {
            self.scroll = self.scroll.clamp(0., self.max_scroll);
        }
        if let Some((key, screen_y)) = &self.expansion_pin
            && let Some(top) = self.expansion_positions.get(key)
        {
            self.scroll = (top - screen_y).clamp(0., self.max_scroll);
        }
        if let Some(wheel) = &mut self.wheel
            && wheel.lane == Lane::Transcript
        {
            wheel.target = (wheel.target + self.scroll - old_scroll).clamp(0., self.max_scroll);
        }
        self.placed = placements;
        self.placed_session = Some(session.clone());
        if can_remember {
            self.remember_scroll();
        }
        self.history_near_top(&session);
        if quick_models {
            self.quick_models_frame(
                layer,
                &session,
                Rect::new(x, viewport.y + quick_top - self.scroll, width, quick_h),
                viewport,
            );
        }
        for (index, (row, p)) in rows.iter().zip(&self.placed).enumerate() {
            let top = viewport.y + p.top - self.scroll;
            if top + p.height < viewport.y || top > viewport.y + viewport.height {
                continue;
            }
            let x = x + if row.user { width - bubble_width } else { 0. };
            let rect = Rect::new(x, top, bubble_width, p.height);
            let joined_above = index > 0 && row.joins(rows.get(index - 1));
            let joined_below = row.joins(rows.get(index + 1));
            let upper = if joined_above { 0. } else { 12. * s };
            let lower = if joined_below { 0. } else { 12. * s };
            let corners = [upper, upper, lower, lower];
            layer.clipped_corners(
                rect,
                corners,
                color(if row.user { 0x164e63 } else { 0x18212b }),
                viewport,
            );
            if joined_above {
                layer.clipped_rect(
                    Rect::new(x + 14. * s, top, text_width, s),
                    color(if row.user { 0x286176 } else { 0x2a3541 }),
                    viewport,
                );
            }
            self.message_areas.push(MessageArea {
                key: row.key.clone(),
                rect,
                corners,
                clip: viewport,
                options: row.actions.clone(),
            });
            // Pin by logical key while a menu is open, not screen coordinates:
            // streaming and paging may move the target without changing its copy boundary.
            let pinned = self
                .context_menu
                .as_ref()
                .is_some_and(|menu| menu.section.as_deref() == Some(row.key.as_str()));
            self.renderer.clipped_label(
                layer,
                &row.timestamp,
                Rect::new(x + 14. * s, top + 8. * s, text_width, 16. * s),
                11. * s,
                color(if row.user { 0xa7bdc9 } else { 0x82909f }),
                false,
                viewport,
            );
            if let Some(layout) = detail_layouts.get(&row.key) {
                for (line, (offset, h)) in row.details.iter().zip(layout) {
                    let lx = x + 14. * s + line.indent * s;
                    let line_rect = Rect::new(
                        lx - 4. * s,
                        top + offset,
                        text_width - line.indent * s + 8. * s,
                        *h,
                    );
                    let line_key = format!("{session}/{}", line.key);
                    let inner_corners = [4. * s; 4];
                    self.detail_areas.push(DetailArea { key: line_key.clone(), rect: line_rect, corners: inner_corners, clip: viewport });
                    if line.tool {
                        layer.clipped_rect(line_rect, color(0x111922), viewport);
                    }
                    if !line.source.is_empty() {
                        self.renderer.message(
                            layer,
                            &format!("{session}/{}", line.key),
                            Vec2::new(lx, top + offset),
                            Rect::new(
                                lx,
                                viewport.y,
                                text_width - line.indent * s,
                                viewport.height,
                            ),
                            self.horizontal,
                        );
                    } else {
                        let label = if let Some(open) = line.toggle {
                            format!("{} {}", if open { "▾" } else { "▸" }, line.label)
                        } else {
                            line.label.clone()
                        };
                        if let Some(open) = line.toggle {
                            let hit = crate::render::intersect(line_rect, viewport);
                            if hit.height > 0. {
                                self.hits.push(Hit {
                                    rect: hit,
                                    action: Action::Toggle(line.key.clone(), !open),
                                });
                            }
                        }
                        self.renderer.clipped_label(
                            layer,
                            &label,
                            Rect::new(
                                lx,
                                top + offset + 5. * s,
                                text_width - line.indent * s,
                                20. * s,
                            ),
                            if line.toggle.is_some() {
                                12. * s
                            } else {
                                11. * s
                            },
                            color(if line.error { 0xffb4ab } else { 0xb7c2ce }),
                            false,
                            viewport,
                        );
                    }
                    layer.surface_highlight(line_rect, inner_corners, viewport, false, true,
                        self.ripple.as_ref().and_then(|r| r.paint(&line_key, line_rect, paint_at)));
                }
                let inner_hover = layer.interaction.hover.is_some_and(|point|
                    self.detail_areas.iter().any(|area| area.contains(point)));
                layer.surface_highlight(rect, corners, viewport, pinned, !inner_hover,
                    self.ripple.as_ref().and_then(|r| r.paint(&row.key, rect, paint_at)));
                continue;
            }
            let label_rect = Rect::new(x + 14. * s, top + 28. * s, text_width, 20. * s);
            if row.header {
                self.renderer.clipped_label(
                    layer,
                    &row.title,
                    label_rect,
                    12. * s,
                    color(if row.error {
                        0xffb4ab
                    } else if row.user {
                        0x67d4ff
                    } else {
                        0xa5b4fc
                    }),
                    true,
                    viewport,
                );
            }
            let text_top = top + if row.header { 50. * s } else { 28. * s };
            if !row.source.is_empty() {
                self.renderer.message(
                    layer,
                    &row.key,
                    Vec2::new(x + 14. * s, text_top),
                    Rect::new(x + 14. * s, viewport.y, text_width, viewport.height),
                    self.horizontal,
                );
            }
            let mut actions: Vec<(String, Action)> = vec![];
            if let Some((entry, attachment)) = &row.attachment {
                let image = attachment.kind == AttachmentKind::Image;
                let path = self.controller.store.attachment_path(
                    &self.controller.identity,
                    &session,
                    entry,
                );
                let key = Controller::download_key(&session, entry);
                if image && path.is_file() {
                    let preview = crate::render::intersect(
                        Rect::new(x + 14. * s, top + p.height - 372. * s, text_width, 230. * s),
                        viewport,
                    );
                    if let Ok((w, h)) = self.renderer.image_size(ctx, &path) {
                        let fit = (text_width / w as f32).min(230. * s / h as f32);
                        layer.images.push((
                            path.clone(),
                            Rect::new(
                                x + 14. * s,
                                top + p.height - 372. * s,
                                w as f32 * fit,
                                h as f32 * fit,
                            ),
                            preview,
                        ));
                        self.hits.push(Hit {
                            rect: preview,
                            action: Action::Attachment(
                                session.clone(),
                                entry.clone(),
                                attachment.file_name.clone(),
                                true,
                            ),
                        });
                    }
                } else if image
                    && self.controller.epoch.is_some()
                    && !self.controller.downloads.contains_key(&key)
                    && let Err(error) = self.controller.download(&session, entry, 10_000_000)
                {
                    self.controller.notice = Some(error.to_string());
                }
                let status = if path.is_file() {
                    "Saved on this device".into()
                } else if let Some(d) = self.controller.downloads.get(&key) {
                    if let Some(error) = &d.status.failure {
                        error.clone()
                    } else {
                        format!("{} / {} bytes", d.status.transferred, d.status.total)
                    }
                } else {
                    format!(
                        "{} · {}",
                        if image { "Image" } else { "File" },
                        attachment
                            .size
                            .map_or_else(|| "size unknown".into(), |n| format!("{n} bytes"))
                    )
                };
                let label = format!(
                    "{}\n{}",
                    attachment
                        .caption
                        .as_deref()
                        .unwrap_or(&attachment.file_name),
                    status
                );
                self.renderer.label(
                    layer,
                    &label,
                    crate::render::intersect(
                        Rect::new(x + 14. * s, top + p.height - 132. * s, text_width, 78. * s),
                        viewport,
                    ),
                    13. * s,
                    color(0xb7c2ce),
                    false,
                );
                actions.insert(
                    0,
                    (
                        if path.is_file() {
                            if image { "View image" } else { "Save file" }
                        } else {
                            "Download"
                        }
                        .into(),
                        Action::Attachment(
                            session.clone(),
                            entry.clone(),
                            attachment.file_name.clone(),
                            image,
                        ),
                    ),
                );
                if self
                    .controller
                    .downloads
                    .get(&key)
                    .is_some_and(|d| !d.status.done)
                {
                    actions.push(("Cancel".into(), Action::CancelDownload(key)));
                }
            }
            let mut ax = x + 10. * s;
            for (label, action) in actions {
                let width = (label.chars().count() as f32 * 7. + 18.) * s;
                if ax + width > x + rect.width {
                    break;
                }
                let r = crate::render::intersect(
                    Rect::new(ax, top + p.height - 28. * s, width, 24. * s),
                    viewport,
                );
                if r.height > 10. * s {
                    button(
                        &mut self.renderer,
                        layer,
                        &mut self.hits,
                        r,
                        &label,
                        action,
                        s,
                        false,
                    );
                }
                ax += width + 5. * s;
            }
            layer.surface_highlight(rect, corners, viewport, pinned, true,
                self.ripple.as_ref().and_then(|r| r.paint(&row.key, rect, paint_at)));
        }
        self.scrollbar(chrome, Lane::Transcript, viewport);
        chrome.rect(
            Rect::new(b.x, composer_top, b.width, composer_h),
            color(0x0e141b),
        );
        let model = summary
            .as_ref()
            .and_then(|s| s.model.as_ref())
            .map(|m| format!("{}/{}", m.provider, m.model_id))
            .unwrap_or_else(|| "Model loads when the worker starts".into());
        self.renderer.label(
            chrome,
            &model,
            Rect::new(x, composer_top + 10. * s, width, 20. * s),
            12. * s,
            color(0x82909f),
            false,
        );
        chrome.rect(Rect::new(b.x, composer_top, b.width, s), color(0x2a3541));
        let field = Rect::new(x, composer_top + 32. * s, width, editor_h);
        let edge = if self.focus == Some(None) { 2. * s } else { s };
        chrome.rounded_rect(
            field,
            4. * s,
            color(if self.focus == Some(None) {
                0x67d4ff
            } else {
                0x526170
            }),
        );
        chrome.rounded_rect(
            Rect::new(
                field.x + edge,
                field.y + edge,
                field.width - 2. * edge,
                field.height - 2. * edge,
            ),
            3. * s,
            color(0x0e141b),
        );
        let composer_rect = Rect::new(
            field.x + 40. * s,
            field.y,
            field.width - 132. * s,
            field.height,
        );
        self.composer.draw(
            &mut self.renderer,
            chrome,
            composer_rect,
            16. * s,
            self.focus == Some(None),
            false,
            "Message Tau",
            false,
        );
        self.hits.push(Hit {
            rect: composer_rect,
            action: Action::Focus(None),
        });
        let iy = field.y + (field.height - 40. * s) / 2.;
        let connected = self.controller.epoch.is_some();
        self.icon_button(
            ctx,
            chrome,
            Rect::new(field.x + 4. * s, iy, 36. * s, 40. * s),
            Icon::Attach,
            22.,
            Action::Attach,
            false,
            connected,
        );
        let usage_rect = Rect::new(field.x + field.width - 84. * s, iy, 40. * s, 40. * s);
        let usage = summary.as_ref().and_then(|s| s.context_usage);
        let capacity = usage.map(|u| u.context_window).filter(|n| *n > 0);
        let used = usage.and_then(|u| u.tokens);
        let ratio = capacity
            .zip(used)
            .map(|(capacity, used)| used as f32 / capacity as f32);
        self.icon_button(
            ctx,
            chrome,
            usage_rect,
            Icon::Context(ratio),
            20.,
            Action::Usage,
            false,
            true,
        );
        self.usage.region = usage_rect;
        self.usage.text = if let Some(capacity) = capacity {
            if let Some(used) = used {
                format!(
                    "Estimated context usage: {:.0}%\n{} of {} tokens",
                    ratio.unwrap() * 100.,
                    count(used),
                    count(capacity)
                )
            } else {
                format!(
                    "Context usage unknown\nCapacity: {} tokens",
                    count(capacity)
                )
            }
        } else {
            "Context usage unavailable".into()
        };
        if capacity.is_some()
            && (!connected
                || !self.controller.chats[&session].feed.synchronized
                || summary.as_ref().is_none_or(|s| {
                    !matches!(s.status, SessionStatus::Idle | SessionStatus::Running)
                }))
        {
            self.usage.text.push_str("\nLast known value");
        }
        let choosing = self.controller.chats[&session].model_request.is_some();
        let can_send =
            connected && !choosing && (!self.composer.value.trim().is_empty() || !files.is_empty());
        self.icon_button(
            ctx,
            chrome,
            Rect::new(field.x + field.width - 44. * s, iy, 40. * s, 40. * s),
            Icon::Send,
            20.,
            Action::Send,
            true,
            can_send,
        );
        if running || paused {
            let (icon, action) = if running {
                (Icon::Stop, Action::Abort)
            } else {
                (Icon::Play, Action::Queue(QueueOperation::Resume {
                    run_id: self.controller.chats[&session].feed.queue.run_id.clone(),
                }))
            };
            self.icon_button(
                ctx,
                chrome,
                Rect::new(
                    b.x + b.width - 52. * s,
                    header.y + (header.height - 40. * s) / 2.,
                    40. * s,
                    40. * s,
                ),
                icon,
                20.,
                action,
                false,
                connected,
            );
        }
        let controls_y = field.y + field.height + 4. * s;
        if self.scroll + 24. * s < self.max_scroll {
            self.icon_button(
                ctx,
                chrome,
                Rect::new(x + width - 40. * s, composer_top - 48. * s, 40. * s, 40. * s),
                Icon::ChevronDown,
                20.,
                Action::Tail,
                true,
                true,
            );
        }
        let queue = &self.controller.chats[&session].feed.queue;
        if let Some(control) = &queue.control
            && matches!(control.status.as_str(), "waiting" | "applying")
        {
            button(
                &mut self.renderer,
                chrome,
                &mut self.hits,
                Rect::new(x, controls_y, 100. * s, 30. * s),
                "Cancel control",
                Action::Queue(QueueOperation::Cancel {
                    control_id: control.command_id.clone(),
                }),
                s,
                false,
            );
        }
        let mut fx = x;
        for file in files {
            let label = format!("{} ×", file.name);
            let fw = ((label.chars().count() as f32 * 7. + 20.) * s).min(width);
            if fx + fw > x + width {
                break;
            }
            button(
                &mut self.renderer,
                chrome,
                &mut self.hits,
                Rect::new(
                    fx,
                    controls_y + if controls { 40. * s } else { 0. },
                    fw,
                    30. * s,
                ),
                &label,
                Action::RemoveFile(file.id),
                s,
                false,
            );
            fx += fw + 6. * s;
        }
        // Slash completion uses optional arguments advertised by the daemon.
        if self.composer.value.starts_with('/')
            && !self.composer.value.contains('\n')
            && self.focus == Some(None)
        {
            let query = &self.composer.value[1..];
            let mut suggestions = vec![];
            for command in &self.controller.chats[&session].commands {
                if let Some(arg) = query.strip_prefix(&format!("{} ", command.name)) {
                    for a in &command.arguments {
                        if a.value.starts_with(arg) {
                            suggestions.push(format!("/{} {}", command.name, a.value));
                        }
                    }
                } else if command.name.starts_with(query) {
                    suggestions.push(format!("/{} ", command.name));
                }
            }
            for (i, text) in suggestions.into_iter().take(5).enumerate() {
                button(
                    &mut self.renderer,
                    chrome,
                    &mut self.hits,
                    Rect::new(x, composer_top - (i + 1) as f32 * 36. * s, width, 34. * s),
                    text.trim(),
                    Action::Suggest(text.clone()),
                    s,
                    false,
                );
            }
        }
    }
    fn quick_models_height(&self, width: f32) -> f32 {
        let columns = if width / self.scale >= 520. { 2 } else { 1 };
        (72. + self
            .controller
            .model_preferences
            .slugs
            .len()
            .div_ceil(columns) as f32
            * 84.
            + 48.)
            * self.scale
    }
    fn quick_models_frame(&mut self, layer: &mut Layer, session: &str, b: Rect, clip: Rect) {
        let s = self.scale;
        let chat = &self.controller.chats[session];
        let connected = self.controller.epoch.is_some();
        let busy = chat.model_request.is_some();
        let hint = if !connected {
            "Connect to choose a model"
        } else if busy {
            "Selecting model… your draft is kept"
        } else {
            "Choose before your first message"
        };
        self.renderer.clipped_label(
            layer,
            "Choose a model",
            Rect::new(b.x, b.y, b.width, 28. * s),
            20. * s,
            color(0xe5eaf0),
            true,
            clip,
        );
        self.renderer.clipped_label(
            layer,
            hint,
            Rect::new(b.x, b.y + 32. * s, b.width, 34. * s),
            12. * s,
            color(0xb7c2ce),
            false,
            clip,
        );
        let columns = if b.width / s >= 520. { 2 } else { 1 };
        let w = (b.width - (columns - 1) as f32 * 8. * s) / columns as f32;
        let current = self
            .controller
            .account
            .sessions
            .iter()
            .find(|c| c.id == session)
            .and_then(|c| c.model.as_ref())
            .map(|m| format!("{}/{}", m.provider, m.model_id));
        for (i, selector) in self.controller.model_preferences.slugs.iter().enumerate() {
            let r = Rect::new(
                b.x + (i % columns) as f32 * (w + 8. * s),
                b.y + (72. + (i / columns) as f32 * 84.) * s,
                w,
                76. * s,
            );
            let valid = selector.parse::<tau_protocol::SessionModel>().is_ok();
            let selected = current.as_deref() == Some(selector.as_str());
            let enabled = connected && !busy && valid;
            let base = color(if selected { 0x303a66 } else { 0x18212b });
            layer.clipped_rounded_rect(
                r,
                12. * s,
                if enabled {
                    layer.control_color(r, base)
                } else {
                    base
                },
                clip,
            );
            self.renderer.clipped_label(
                layer,
                selector,
                Rect::new(r.x + 12. * s, r.y + 10. * s, w - 24. * s, 32. * s),
                12. * s,
                color(if enabled || selected {
                    0xe5eaf0
                } else {
                    0x82909f
                }),
                false,
                crate::render::intersect(r, clip),
            );
            let status = if !connected {
                "Offline"
            } else if !valid {
                "Invalid provider/model ID"
            } else if chat
                .model_request
                .as_ref()
                .is_some_and(|(_, slug)| slug == selector)
            {
                "Selecting…"
            } else if selected {
                "Selected"
            } else {
                "Select"
            };
            self.renderer.clipped_label(
                layer,
                status,
                Rect::new(r.x + 12. * s, r.y + 54. * s, w - 24. * s, 16. * s),
                11. * s,
                color(if selected { 0x67d4ff } else { 0xb7c2ce }),
                false,
                crate::render::intersect(r, clip),
            );
            if enabled {
                self.hits.push(Hit {
                    rect: crate::render::intersect(r, clip),
                    action: Action::ChooseModel(session.into(), selector.clone()),
                });
            }
        }
        let r = Rect::new(b.x, b.y + b.height - 40. * s, b.width, 32. * s);
        layer.clipped_rounded_rect(r, 16. * s, layer.control_color(r, color(0x18212b)), clip);
        self.renderer.clipped_label(
            layer,
            "Configure quick models…",
            Rect::new(r.x + 12. * s, r.y + 7. * s, r.width - 24. * s, 20. * s),
            12. * s,
            color(0x67d4ff),
            false,
            crate::render::intersect(r, clip),
        );
        self.hits.push(Hit {
            rect: crate::render::intersect(r, clip),
            action: Action::ModelSettings,
        });
    }
    fn notice_frame(&mut self, layer: &mut Layer, b: Rect) {
        let s = self.scale;
        let width = (b.width - 32. * s).min(640. * s);
        let x = b.x + (b.width - width) / 2.;
        if let Some(notice) = &self.controller.notice {
            let rect = Rect::new(x, b.y + 16. * s, width, 68. * s);
            layer.rounded_rect(rect, 12. * s, color(0x452c2a));
            self.renderer.label(
                layer,
                notice,
                Rect::new(x + 10. * s, rect.y + 8. * s, width - 54. * s, 54. * s),
                12. * s,
                color(0xffd8d0),
                false,
            );
            button(
                &mut self.renderer,
                layer,
                &mut self.hits,
                Rect::new(x + width - 40. * s, rect.y + 8. * s, 32. * s, 32. * s),
                "×",
                Action::DismissNotice,
                s,
                false,
            );
        }
    }
    pub fn context_at(&mut self, point: Vec2) {
        if self.modal.is_some()
            || self.viewer.is_some()
            || self.usage.contains_card(point)
            || self.info_tip.contains_card(point)
            || self.context_menu.is_some() && contains(self.context_rect, point)
        {
            return;
        }
        if let Some(id) = self.project_areas.iter().find(|(r,_)| contains(*r, point)).map(|(_,id)| id.clone()) {
            self.project_context(&id, point);
            return;
        }
        let mut chat = self
            .chat_areas
            .iter()
            .find(|(rect, _)| contains(*rect, point))
            .map(|(_, id)| id.clone());
        let transcript = contains(self.transcript, point);
        if chat.is_none() && !transcript {
            return;
        }
        if self.cancel_autoscroll() {
            return;
        }
        self.wheel = None;
        self.velocity = 0.;
        self.usage.dismiss();
        self.info_tip.dismiss();
        let area = transcript
            .then(|| self.message_areas.iter().find(|a| a.contains(point)))
            .flatten();
        let mut options = vec![];
        if chat.is_none() {
            if self
                .renderer
                .selected_text()
                .is_some_and(|text| !text.is_empty())
            {
                options.push(("Copy selection".into(), Action::CopySelection));
            }
            if let Some(area) = area {
                options.extend(area.options.clone());
            }
            if options.is_empty() {
                chat = self.controller.account.selected.clone();
            }
        }
        if let Some(id) = &chat {
            options = vec![
                (
                    "Model…".into(),
                    Action::AgentSetting(id.clone(), "model".into()),
                ),
                (
                    "Thinking…".into(),
                    Action::AgentSetting(id.clone(), "thinking".into()),
                ),
                (
                    "Compact context…".into(),
                    Action::AgentSetting(id.clone(), "compact".into()),
                ),
                (
                    "Codex priority…".into(),
                    Action::AgentSetting(id.clone(), "fast".into()),
                ),
                ("Move to topic  ›".into(), Action::MoveMenu(id.clone())),
                ("Rename…".into(), Action::Rename(id.clone())),
                ("Clone chat".into(), Action::Clone(id.clone())),
                ("Release idle runtime".into(), Action::Sleep(id.clone())),
                ("Delete chat…".into(), Action::Delete(id.clone())),
            ];
        }
        self.context_menu = (!options.is_empty()).then(|| ContextMenu {
            at: point,
            section: area.map(|a| a.key.clone()),
            chat,
            options,
            selected: 0,
            scroll: 0.,
            parent: None,
        });
        self.pointer = None;
        self.selecting = false;
        self.dirty = true;
    }
    #[allow(clippy::too_many_arguments)]
    fn icon_button(
        &mut self,
        ctx: &impl RenderContext,
        layer: &mut Layer,
        r: Rect,
        icon: Icon,
        size: f32,
        action: Action,
        primary: bool,
        enabled: bool,
    ) {
        let hovered = layer.interaction.hover.is_some_and(|p| contains(r, p));
        let tonal = matches!(icon, Icon::Stop | Icon::Play);
        if primary || tonal || enabled && hovered {
            layer.rounded_rect(
                r,
                r.height / 2.,
                layer.control_color(
                    r,
                    color(if !enabled {
                        0x303942
                    } else if primary {
                        0x67d4ff
                    } else if tonal {
                        0x18212b
                    } else {
                        0x24303b
                    }),
                ),
            );
        }
        let pixels = size * self.scale;
        self.renderer.icon(
            ctx,
            layer,
            icon,
            Rect::new(
                r.x + (r.width - pixels) / 2.,
                r.y + (r.height - pixels) / 2.,
                pixels,
                pixels,
            ),
            if !enabled {
                0x68727e
            } else if primary {
                0x003546
            } else if matches!(icon, Icon::Attach) {
                0xb7c2ce
            } else {
                0x67d4ff
            },
        );
        if enabled {
            self.hits.push(Hit { rect: r, action });
        }
    }
    fn usage_frame(&mut self, layer: &mut Layer, bounds: Rect) {
        if self.usage.progress <= 0.
            || self.usage.region.width <= 0.
            || self.modal.is_some()
            || self.viewer.is_some()
            || self.context_menu.is_some()
        {
            return;
        }
        let s = self.scale;
        let w = (300. * s).min(bounds.width - 16. * s);
        let h = (self.usage.text.lines().count() as f32 * 17. + 20.) * s;
        let anchor = self.usage.region;
        let x = (anchor.x + anchor.width / 2. - w / 2.)
            .clamp(bounds.x + 8. * s, bounds.x + bounds.width - w - 8. * s);
        let bottom = anchor.y - 8. * s;
        let full = Rect::new(x, bottom - h, w, h);
        self.usage.card = full;
        let t = self.usage.progress;
        let animated = Rect::new(x + w * (1. - t) / 2., bottom - h * t, w * t, h * t);
        // Opaque X/Y expansion from the indicator. No opacity fade.
        layer.rounded_rect(animated, 6. * s, color(0xe5e1e6));
        self.renderer.clipped_label(
            layer,
            &self.usage.text,
            Rect::new(
                full.x + 10. * s,
                full.y + 10. * s,
                full.width - 20. * s,
                full.height - 20. * s,
            ),
            12. * s,
            color(0x303038),
            false,
            animated,
        );
    }
    fn info_frame(&mut self, layer: &mut Layer, bounds: Rect) {
        if self.info_tip.progress <= 0.
            || self.info_tip.region.width <= 0.
            || self.modal.is_some()
            || self.viewer.is_some()
            || self.context_menu.is_some()
        {
            return;
        }
        self.info_tip.text = match &self.info_target {
            Info::Connection => self
                .controller
                .health
                .details(&self.controller.settings, &self.controller.connection),
            Info::CacheTtl(id) => {
                let Some(session) = self
                    .controller
                    .account
                    .sessions
                    .iter()
                    .find(|session| &session.id == id)
                else {
                    return;
                };
                self.controller
                    .cache_ttl(session)
                    .details(self.controller.epoch.is_some())
            }
        };
        let s = self.scale;
        let margin = 8. * s;
        let w = (360. * s).min((bounds.width - margin * 2.).max(1.));
        let h = (self.renderer.label_height(
            &self.info_tip.text,
            (w - 20. * s).max(1.),
            12. * s,
            false,
        ) + 20. * s)
            .min((bounds.height - margin * 2.).max(1.));
        let anchor = self.info_tip.region;
        let x = (anchor.x + anchor.width / 2. - w / 2.).clamp(
            bounds.x + margin,
            (bounds.x + bounds.width - margin - w).max(bounds.x + margin),
        );
        let y = (anchor.y + anchor.height + 6. * s).clamp(
            bounds.y + margin,
            (bounds.y + bounds.height - margin - h).max(bounds.y + margin),
        );
        let full = Rect::new(x, y, w, h);
        self.info_tip.card = full;
        let t = self.info_tip.progress;
        let pivot = (anchor.x + anchor.width / 2.).clamp(full.x, full.x + full.width);
        let animated = Rect::new(pivot + (x - pivot) * t, y, w * t, h * t);
        layer.rounded_rect(animated, 6. * s, color(0xe5e1e6));
        // Keep revealed text inside the rounded mask as the opaque card expands.
        let clip = Rect::new(
            animated.x + 6. * s,
            animated.y + 6. * s,
            (animated.width - 12. * s).max(0.),
            (animated.height - 12. * s).max(0.),
        );
        self.renderer.clipped_label(
            layer,
            &self.info_tip.text,
            Rect::new(
                full.x + 10. * s,
                full.y + 10. * s,
                (full.width - 20. * s).max(1.),
                (full.height - 20. * s).max(0.),
            ),
            12. * s,
            color(0x303038),
            false,
            clip,
        );
    }
    fn settings_frame(&mut self, layer: &mut Layer, b: Rect) {
        let s = self.scale;
        self.hits.clear();
        layer.rect(b, color(0x0e141b));
        let modal = self.modal.as_mut().unwrap();
        let connected = self.controller.epoch.is_some()
            && modal.fields[0].1.value.trim().trim_end_matches('/')
                == self.controller.settings.server_url
            && modal.fields[1].1.value.trim() == self.controller.settings.token;
        let configured = !self.controller.settings.token.is_empty();
        let connection_error = (!matches!(
            self.controller.connection.as_str(),
            "Connected" | "Connecting…" | "Not connected"
        ))
        .then_some(self.controller.connection.as_str());
        let error = self.controller.notice.as_deref().or(connection_error);
        let width = (b.width - 48. * s).min(520. * s).max(240. * s);
        let inner_w = width - 48. * s;
        let height =
            (416. + if configured { 92. } else { 0. } + if error.is_some() { 84. } else { 0. }) * s;
        let card = Rect::new(
            b.x + (b.width - width) / 2.,
            b.y + ((b.height - height) / 2.).max(12. * s),
            width,
            height,
        );
        layer.rounded_rect(card, 12. * s, color(0x36343b));
        let x = card.x + 24. * s;
        self.renderer.label(
            layer,
            "Tau",
            Rect::new(x, card.y + 24. * s, inner_w, 44. * s),
            32. * s,
            color(0xe5eaf0),
            true,
        );
        self.renderer.label(
            layer,
            concat!("Version ", env!("CARGO_PKG_VERSION"), " · Beta"),
            Rect::new(x, card.y + 66. * s, inner_w, 20. * s),
            12. * s,
            color(0xb7c2ce),
            false,
        );
        self.renderer.label(
            layer,
            "Connect directly to the Tau daemon over your Tailnet.",
            Rect::new(x, card.y + 102. * s, inner_w, 42. * s),
            16. * s,
            color(0xe5eaf0),
            false,
        );
        let field_y = card.y + 146. * s;
        for (i, (name, editor, secret)) in modal.fields.iter_mut().enumerate() {
            let rect = Rect::new(x, field_y + i as f32 * 80. * s, inner_w, 56. * s);
            editor.draw(
                &mut self.renderer,
                layer,
                rect,
                16. * s,
                self.focus == Some(Some(i)),
                *secret,
                if i == 0 {
                    "http://vibe:8787"
                } else {
                    "Access token"
                },
                true,
            );
            let label = if i == 0 { "Daemon URL" } else { name };
            let style = sanscale::Style {
                chain: self.renderer.faces.prose[0],
                wrap_em: None,
                align: sanscale::Align::Left,
                line_spacing: 1.,
            };
            let label_w = self
                .renderer
                .text
                .shape_transient(label, &style)
                .map_or(100. * s, |b| {
                    self.renderer.text.measure(b).width_em() * 12. * s + 12. * s
                });
            layer.rect(
                Rect::new(x + 12. * s, rect.y - 7. * s, label_w, 16. * s),
                color(0x36343b),
            );
            self.renderer.label(
                layer,
                label,
                Rect::new(x + 16. * s, rect.y - 8. * s, label_w, 18. * s),
                12. * s,
                color(0xb7c2ce),
                false,
            );
            self.hits.push(Hit {
                rect,
                action: Action::Focus(Some(i)),
            });
        }
        let buttons_y = field_y + 158. * s;
        if !self.connecting || self.controller.connection != "Connecting…" {
            button(
                &mut self.renderer,
                layer,
                &mut self.hits,
                Rect::new(x + inner_w - 104. * s, buttons_y, 104. * s, 40. * s),
                "Connect",
                Action::Confirm,
                s,
                true,
            );
        } else {
            self.renderer.label(
                layer,
                "Connecting…",
                Rect::new(
                    x + inner_w - 132. * s,
                    buttons_y + 10. * s,
                    132. * s,
                    28. * s,
                ),
                14. * s,
                color(0x67d4ff),
                false,
            );
        }
        button(
            &mut self.renderer,
            layer,
            &mut self.hits,
            Rect::new(x + inner_w - 204. * s, buttons_y, 88. * s, 40. * s),
            "Cancel",
            Action::CancelModal,
            s,
            false,
        );
        let mut y = buttons_y + 60. * s;
        button(
            &mut self.renderer,
            layer,
            &mut self.hits,
            Rect::new(x, y, inner_w, 32. * s),
            "Quick model selection",
            Action::ModelSettings,
            s,
            false,
        );
        y += 44. * s;
        if configured {
            layer.rect(Rect::new(x, y, inner_w, s), color(0x526170));
            y += 12. * s;
            self.renderer.label(
                layer,
                "Daemon settings",
                Rect::new(x, y, inner_w, 26. * s),
                16. * s,
                color(0xe5eaf0),
                true,
            );
            y += 28. * s;
            if connected && !self.waiting_settings {
                button(
                    &mut self.renderer,
                    layer,
                    &mut self.hits,
                    Rect::new(x, y, 168. * s, 32. * s),
                    "Open settings",
                    Action::DaemonSettings,
                    s,
                    false,
                );
            } else {
                self.renderer.label(
                    layer,
                    if self.waiting_settings {
                        "Loading settings…"
                    } else {
                        "Connect to edit daemon, agent and prompt settings."
                    },
                    Rect::new(x, y, inner_w, 36. * s),
                    14. * s,
                    color(0xb7c2ce),
                    false,
                );
            }
            y += 42. * s;
        }
        if let Some(error) = error {
            self.renderer.label(
                layer,
                error,
                Rect::new(x, y, inner_w, 72. * s),
                14. * s,
                color(0xffb4ab),
                false,
            );
        }
    }
    fn model_settings_frame(&mut self, layer: &mut Layer, b: Rect) {
        let s = self.scale;
        self.hits.clear();
        layer.rect(b, color(0x0e141b));
        let w = (b.width - 32. * s).min(680. * s).max(1.);
        let x = b.x + (b.width - w) / 2.;
        let top = b.y + 16. * s;
        let footer = b.y + b.height - 56. * s;
        let modal = self.modal.as_mut().unwrap();
        self.renderer.label(
            layer,
            "Quick model selection",
            Rect::new(x, top, w, 30. * s),
            20. * s,
            color(0xe5eaf0),
            true,
        );
        let compact = b.height / s < 480.;
        let show_suggestions = b.height / s >= 320.;
        let help_h = if compact { 32. } else { 58. } * s;
        self.renderer.label(layer, if compact { "One slug per line. Last chosen model is remembered. Empty disables tiles." }
            else { "One provider/model per line (max 12). New chats keep the last chosen model. This list only controls quick-select tiles; empty disables them." },
            Rect::new(x, top + 36. * s, w, help_h), 12. * s, color(0xb7c2ce), false);
        let edit_y = top + 36. * s + help_h + 8. * s;
        let edit_h = (b.height * 0.25)
            .min(164. * s)
            .min((footer - 48. * s - edit_y - if show_suggestions { 56. * s } else { 0. }).max(0.));
        let edit = Rect::new(x, edit_y, w, edit_h);
        modal.fields[0].1.draw(
            &mut self.renderer,
            layer,
            edit,
            16. * s,
            self.focus == Some(Some(0)),
            false,
            "provider/model",
            true,
        );
        self.hits.push(Hit {
            rect: edit,
            action: Action::Focus(Some(0)),
        });
        let search = Rect::new(x, edit.y + edit.height + 12. * s, w, 36. * s);
        if show_suggestions {
            modal.fields[1].1.draw(
                &mut self.renderer,
                layer,
                search,
                16. * s,
                self.focus == Some(Some(1)),
                false,
                "Search optional model suggestions",
                true,
            );
            self.hits.push(Hit {
                rect: search,
                action: Action::Focus(Some(1)),
            });
        }
        let list_y = search.y + search.height + 8. * s;
        let list_bottom = footer - 52. * s;
        let query = modal.fields[1].1.value.to_lowercase();
        let commands = self
            .controller
            .selected()
            .map(|c| c.commands.as_slice())
            .unwrap_or(&[]);
        let preferences = crate::models::Preferences::parse(&modal.fields[0].1.value).ok();
        let count = if show_suggestions {
            ((list_bottom - list_y) / (32. * s)).max(0.) as usize
        } else {
            0
        };
        let suggestions = commands.iter()
            .find(|c| c.name == "model" && c.source == tau_protocol::SlashCommandSource::Builtin)
            .map(|c| c.arguments.as_slice()).unwrap_or(&[]);
        let mut shown = 0;
        for model in suggestions
            .iter()
            .filter(|m| m.value.to_lowercase().contains(&query))
            .take(count)
        {
            let existing = preferences.as_ref().and_then(|p| {
                p.slugs.iter().find(|slug| {
                    *slug == &model.value
                })
            });
            let r = Rect::new(x, list_y + shown as f32 * 32. * s, w, 30. * s);
            layer.rounded_rect(r, 6. * s, layer.control_color(r, color(0x18212b)));
            self.renderer.label(
                layer,
                &format!(
                    "{} {}",
                    if existing.is_some() { "−" } else { "+" },
                    model.value
                ),
                Rect::new(r.x + 8. * s, r.y + 6. * s, w - 16. * s, 20. * s),
                12. * s,
                color(0x67d4ff),
                false,
            );
            self.hits.push(Hit {
                rect: r,
                action: Action::ToggleQuickModel(existing.unwrap_or(&model.value).clone()),
            });
            shown += 1;
        }
        if shown == 0 && count > 0 {
            self.renderer.label(
                layer,
                if suggestions.is_empty() {
                    "No suggestions loaded. You can enter any provider/model above."
                } else {
                    "No matching suggestions. You can still enter the ID above."
                },
                Rect::new(x, list_y, w, 40. * s),
                12. * s,
                color(0x82909f),
                false,
            );
        }
        self.renderer.label(
            layer,
            self.controller
                .notice
                .as_deref()
                .unwrap_or("Enter any provider/model. Suggestions are optional; the provider decides availability."),
            Rect::new(x, footer - 44. * s, w, 36. * s),
            12. * s,
            color(if self.controller.notice.is_some() {
                0xffb4ab
            } else {
                0x82909f
            }),
            false,
        );
        let button_w = (w - 16. * s) / 3.;
        for (i, (label, action)) in [
            ("Save", Action::Confirm),
            ("Presets", Action::ResetModels),
            ("Cancel", Action::CancelModal),
        ]
        .into_iter()
        .enumerate()
        {
            button(
                &mut self.renderer,
                layer,
                &mut self.hits,
                Rect::new(
                    x + i as f32 * (button_w + 8. * s),
                    footer,
                    button_w,
                    40. * s,
                ),
                label,
                action,
                s,
                i == 0,
            );
        }
    }
    fn apply_setting_field(&mut self) -> Result<()> {
        if let (Some(draft), Some(modal)) = (&mut self.daemon_draft, &self.modal)
            && let Some((_, editor, _)) = modal.fields.first() {
            draft.apply(&editor.value)?;
        }
        Ok(())
    }
    fn load_setting_field(&mut self) -> Result<()> {
        use crate::daemon_settings::Kind;
        if let (Some(draft), Some(modal)) = (&mut self.daemon_draft, &mut self.modal) {
            let text = draft.text()?;
            let definition = draft.definition();
            let editor = if matches!(
                definition.kind,
                Kind::Line | Kind::Number | Kind::Bool | Kind::Model | Kind::OptionalModel | Kind::PromptModel
            ) {
                Editor::line(text)
            } else {
                Editor::new(text)
            };
            modal.fields = vec![(definition.name.into(), editor, false)];
        }
        self.focus = None;
        Ok(())
    }
    fn daemon_settings_frame(&mut self, layer: &mut Layer, b: Rect) {
        use crate::daemon_settings::{Kind, SECTIONS, fields};
        let s = self.scale;
        self.hits.clear();
        layer.rect(b, color(0x0e141b));
        let w = (b.width - 32. * s).min(900. * s).max(1.);
        let x = b.x + (b.width - w) / 2.;
        let top = b.y + 12. * s;
        let footer = b.y + b.height - 52. * s;
        let title = self
            .daemon_draft
            .as_ref()
            .map(|d| format!("Daemon settings · revision {}", d.revision))
            .unwrap_or_else(|| "Daemon settings".into());
        self.renderer.label(
            layer,
            &title,
            Rect::new(x, top, w, 28. * s),
            20. * s,
            color(0xe5eaf0),
            true,
        );
        let busy = self.saving_settings.is_some();
        if let Some(draft) = &self.daemon_draft {
            let columns = if w / s >= 600. { SECTIONS.len() } else { 3 };
            let tab_w = (w - 8. * s * (columns - 1) as f32) / columns as f32;
            let mut y = top + 36. * s;
            for (i, label) in SECTIONS.iter().enumerate() {
                let r = Rect::new(
                    x + (i % columns) as f32 * (tab_w + 8. * s),
                    y + (i / columns) as f32 * 36. * s,
                    tab_w,
                    30. * s,
                );
                button(
                    &mut self.renderer,
                    layer,
                    &mut self.hits,
                    r,
                    label,
                    Action::SettingsSection(i),
                    s,
                    i == draft.section,
                );
            }
            y += SECTIONS.len().div_ceil(columns) as f32 * 36. * s + 8. * s;
            let definition = draft.definition();
            let count = fields(draft.section).len();
            if count > 1 {
                button(
                    &mut self.renderer,
                    layer,
                    &mut self.hits,
                    Rect::new(x, y, 36. * s, 32. * s),
                    "‹",
                    Action::SettingsField(false),
                    s,
                    false,
                );
                button(
                    &mut self.renderer,
                    layer,
                    &mut self.hits,
                    Rect::new(x + w - 36. * s, y, 36. * s, 32. * s),
                    "›",
                    Action::SettingsField(true),
                    s,
                    false,
                );
            }
            self.renderer.label(
                layer,
                &format!(
                    "{}{}",
                    definition.name,
                    if count > 1 {
                        format!("  ({}/{count})", draft.field + 1)
                    } else {
                        String::new()
                    }
                ),
                Rect::new(
                    x + if count > 1 { 44. * s } else { 0. },
                    y,
                    w - if count > 1 { 88. * s } else { 0. },
                    32. * s,
                ),
                15. * s,
                color(0xe5eaf0),
                true,
            );
            y += 40. * s;
            let help = if definition.kind == Kind::PromptOverride {
                format!("{}\n{}", draft.prompt_model, definition.help)
            } else { definition.help.into() };
            let help_h = if b.height / s < 480. { 44. } else { 72. } * s;
            self.renderer.label(
                layer,
                &help,
                Rect::new(x, y, w, help_h),
                12. * s,
                color(0xb7c2ce),
                false,
            );
            y += help_h + 8. * s;
            let modal = self.modal.as_mut().unwrap();
            if let Some((_, editor, _)) = modal.fields.first_mut() {
                if definition.kind == Kind::Bool {
                    button(
                        &mut self.renderer,
                        layer,
                        &mut self.hits,
                        Rect::new(x, y, w, 42. * s),
                        if editor.value == "true" {
                            "✓ Enabled — tap to disable"
                        } else {
                            "Disabled — tap to enable"
                        },
                        Action::SettingToggle,
                        s,
                        editor.value == "true",
                    );
                } else {
                    let readonly = definition.kind == Kind::PromptOverride && draft.inherit;
                    if definition.kind == Kind::PromptOverride {
                        button(
                            &mut self.renderer,
                            layer,
                            &mut self.hits,
                            Rect::new(x, y, w, 32. * s),
                            if draft.inherit {
                                "✓ Inherit default prompt — switch to override"
                            } else {
                                "Model override — switch to inherited default"
                            },
                            Action::SettingToggle,
                            s,
                            draft.inherit,
                        );
                        y += 40. * s;
                    }
                    let available = (footer - 88. * s - y).max(1.);
                    let h = if editor.single_line {
                        available.min(48. * s)
                    } else {
                        available
                    };
                    let rect = Rect::new(x, y, w, h);
                    editor.draw(
                        &mut self.renderer,
                        layer,
                        rect,
                        15. * s,
                        !readonly && self.focus == Some(Some(0)),
                        false,
                        "",
                        true,
                    );
                    if !readonly {
                        self.hits.push(Hit {
                            rect,
                            action: Action::Focus(Some(0)),
                        });
                    }
                }
            }
            button(
                &mut self.renderer,
                layer,
                &mut self.hits,
                Rect::new(x, footer - 80. * s, 120. * s, 28. * s),
                "Reset field",
                Action::SettingReset,
                s,
                false,
            );
            self.renderer.label(
                layer,
                "Edits are staged until Save. Reload discards them.",
                Rect::new(
                    x + 132. * s,
                    footer - 78. * s,
                    (w - 132. * s).max(1.),
                    28. * s,
                ),
                11. * s,
                color(0x82909f),
                false,
            );
        } else {
            self.renderer.label(
                layer,
                if self.waiting_settings {
                    "Loading the daemon's settings…"
                } else {
                    "Connect and reload to edit settings."
                },
                Rect::new(x, top + 60. * s, w, 60. * s),
                15. * s,
                color(0xb7c2ce),
                false,
            );
        }
        self.renderer.label(layer,self.controller.notice.as_deref().unwrap_or(if busy {"Saving…"} else {"Credentials remain private on the daemon. Conflicts never overwrite newer settings."}),Rect::new(x,footer-42.*s,w,36.*s),12.*s,color(if self.controller.notice.as_deref() == Some("Settings saved") {0x67d4ff} else if self.controller.notice.is_some() {0xffb4ab} else {0x82909f}),false);
        if busy {
            self.hits.clear();
        }
        let bw = (w - 16. * s) / 3.;
        for (i, (label, action)) in [
            (if busy { "Saving…" } else { "Save" }, Action::Confirm),
            ("Reload", Action::DaemonSettings),
            ("Close", Action::CancelModal),
        ]
        .into_iter()
        .enumerate()
        {
            if busy && i < 2 {
                continue;
            }
            button(
                &mut self.renderer,
                layer,
                &mut self.hits,
                Rect::new(x + i as f32 * (bw + 8. * s), footer, bw, 40. * s),
                label,
                action,
                s,
                i == 0,
            );
        }
    }
    fn modal_frame(&mut self, layer: &mut Layer, b: Rect) {
        let s = self.scale;
        self.hits.clear();
        layer.rect(b, sanscale::Color([0., 0., 0., 0.8]));
        let modal = self.modal.as_mut().unwrap();
        let long = matches!(modal.kind, ModalKind::QueueEdit(..));
        let field_h = if long { 180. } else { 60. };
        let width = (b.width - 24. * s).min(620. * s);
        let height = ((96.
            + modal.fields.len() as f32 * (field_h + 26.)
            + modal.options.len() as f32 * 44.)
            * s)
            .min(b.height - 24. * s);
        let rect = Rect::new(
            b.x + (b.width - width) / 2.,
            b.y + (b.height - height) / 2.,
            width,
            height,
        );
        layer.rounded_rect(rect, 16. * s, color(0x111b25));
        self.renderer.label(
            layer,
            &modal.title,
            Rect::new(rect.x + 20. * s, rect.y + 18. * s, width - 40. * s, 60. * s),
            17. * s,
            color(0xe5eaf0),
            true,
        );
        let mut y = rect.y + 84. * s;
        for (i, (name, e, secret)) in modal.fields.iter_mut().enumerate() {
            self.renderer.label(
                layer,
                name,
                Rect::new(rect.x + 20. * s, y, width - 40. * s, 20. * s),
                11. * s,
                color(0xb7c2ce),
                false,
            );
            y += 22. * s;
            let field = Rect::new(rect.x + 20. * s, y, width - 40. * s, field_h * s);
            e.draw(
                &mut self.renderer,
                layer,
                field,
                16. * s,
                self.focus == Some(Some(i)),
                *secret,
                "",
                true,
            );
            self.hits.push(Hit {
                rect: field,
                action: Action::Focus(Some(i)),
            });
            y += (field_h + 4.) * s;
        }
        for (label, action) in &modal.options {
            button(
                &mut self.renderer,
                layer,
                &mut self.hits,
                Rect::new(rect.x + 20. * s, y, width - 40. * s, 36. * s),
                label,
                action.clone(),
                s,
                matches!(action, Action::Confirm),
            );
            y += 44. * s;
        }
    }
}
#[allow(clippy::too_many_arguments)]
fn button(
    renderer: &mut Renderer,
    layer: &mut Layer,
    hits: &mut Vec<Hit>,
    rect: Rect,
    label: &str,
    action: Action,
    s: f32,
    primary: bool,
) {
    if rect.width <= 0. || rect.height <= 0. {
        return;
    }
    let destructive = matches!(action, Action::RemoveProject(DeleteProjectMode::DeleteChats));
    layer.rounded_rect(
        rect,
        rect.height * 0.5,
        layer.control_color(rect, color(if destructive { 0x402b30 } else if primary { 0x67d4ff } else { 0x18212b })),
    );
    let style = sanscale::Style {
        chain: renderer.faces.prose[0],
        wrap_em: None,
        align: sanscale::Align::Left,
        line_spacing: 1.,
    };
    if let Some(block) = renderer.text.shape_transient(label, &style) {
        let layout = renderer.text.measure(block);
        let size = (if label.chars().count() == 1 { 22. } else { 14. } * s)
            .min((rect.width - 12. * s).max(1.) / layout.width_em().max(1.));
        layer.draws.push(sanscale::Draw {
            block,
            at: Vec2::new(
                rect.x + (rect.width - layout.width_em() * size) / 2.,
                rect.y + (rect.height - layout.height_em() * size) / 2.,
            ),
            size,
            color: color(if destructive { 0xffb4ab } else if primary { 0x003546 } else { 0x67d4ff }),
            clip: Some(rect),
            ..Default::default()
        });
    }
    hits.push(Hit { rect, action });
}
pub(crate) fn literal(text: &str) -> String {
    text.chars()
        .flat_map(|c| {
            if "\\`*_{}[]<>()#+-.!|~>".contains(c) {
                vec!['\\', c]
            } else {
                vec![c]
            }
        })
        .collect()
}
pub(crate) fn code(text: &str) -> String {
    let n = text
        .split(|c| c != '`')
        .map(str::len)
        .max()
        .unwrap_or(0)
        .max(2)
        + 1;
    let fence = "`".repeat(n);
    format!("{fence}\n{text}\n{fence}")
}

fn count(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(all(test, not(target_os = "android")))]
mod editor_tests;
#[cfg(all(test, not(target_os = "android")))]
mod project_tests;
#[cfg(all(test, not(target_os = "android")))]
mod thinking_tests;
#[cfg(all(test, not(target_os = "android")))]
mod control_tests;
#[cfg(all(test, not(target_os = "android")))]
mod icon_controls_tests;
#[cfg(all(test, not(target_os = "android")))]
mod hover_tests;
#[cfg(all(test, not(target_os = "android")))]
mod viewer_tests;
#[cfg(all(test, not(target_os = "android")))]
mod scroll_tests;
