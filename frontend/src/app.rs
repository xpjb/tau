use crate::{
    clock,
    connection::{CounterTicker, COUNTER_REFRESH},
    controller::Controller,
    details::{Line as DetailLine, Tools},
    editor::Editor,
    icons::Icon,
    render::{Interaction, Layer, Renderer, color, contains, contains_rounded},
    scroll::{Autoscroll, Drag, Lane, Scrollbar, Wheel},
    store::{Settings, Store},
    tooltip::{Content, Tooltip},
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
mod attachments;
mod notices;
mod navigation;
use crate::notice::DownloadTarget;
mod composer_status;
mod ui;
mod code_view;

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
    RetryCreate,
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
    OpenDownloadNotice(DownloadTarget),
    CopyDiagnostics,
    ClearReplica,
    CopyRecoveredDraft(String),
    ForgetRecovered(String),
    ReviewRestore(String),
    Outbox(usize),
    InspectControl(String),
    CheckControl(String),
    RetryControl(String),
    ForgetControl(String),
    Focus(Option<usize>),
    Confirm,
    CancelModal,
    DaemonSettings,
    RefreshCatalog,
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
    Attachments,
    Files,
    FileOpen(String, bool),
    FileUp,
    FileClose,
    FileFind,
    FileFindHere,
    FileClear,
    FileCopy,
    FilePage(bool),
    History,
    RemoveFile(String),
    Toggle(String, bool),
    Restore(String),
    Dismiss(String),
    RetryPending(String),
    Queue(QueueOperation),
    EditQueue(String, u64, String),
    Attachment(String, String, String, bool),
    SaveAttachment(String, String, String),
    UseSaved(String, String, SavedAction),
    CancelDownload(String),
    Zoom(f32),
    Fit,
    Suggest(String),
}
#[derive(Clone, PartialEq, Eq)]
enum Info {
    Connection,
    CacheTtl(String),
    Attachment(String, String, String), // stable key, accessible action/title, unabridged detail
}
impl Info {
    fn same_anchor(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Attachment(a, ..), Self::Attachment(b, ..)) => a == b,
            _ => self == other,
        }
    }
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
    RefreshCatalog,
    AgentCommand(String, String),
    QueueEdit(String, u64),
    ConfirmLink(String),
    Outbox,
    ForgetControl(String),
    ForgetRecovered(String),
    ReviewRestore(String),
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
    block: Option<String>, // Native interest, independent of the stable display identity.
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
    Haptic,
    Copy(String),
    Paste,
    PickFile {
        identity: String,
        session: String,
    },
    OpenUrl(String),
    SaveDownload { key: String, source: PathBuf, name: String },
    UseDownload(crate::store::SavedDownload, SavedAction, DownloadTarget),
    Edit {
        title: String,
        value: String,
        secret: bool,
        single_line: bool,
    },
    Background,
}
#[derive(Clone, Copy)]
pub enum SavedAction { Open, #[cfg(not(target_os = "android"))] Show, #[cfg(not(target_os = "android"))] Extract }
struct Viewer {
    path: PathBuf,
    name: String,
    session: String,
    entry: String,
    zoom: f32,
    pan: Vec2,
}
#[cfg(not(target_os = "android"))]
#[derive(Clone, Copy)]
pub enum ConnectionPreview {
    Received,
    Waiting,
    Disconnected,
    Unconfigured,
}
pub struct App {
    pub controller: Controller,
    root: ui::RootWidget,
    pub(crate) ui: ui::UiState,
    services: ui::Services,
}

// Transitional workspace state. Migrated widgets are siblings of this adapter
// under RootWidget; model/services and shared input state never live inside it.
struct LegacyWorkspace {
    hits: Vec<Hit>,
    composer: Editor,
    code: Option<code_view::View>,
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
    revealed_project_position: Option<usize>,
    saving_project: Option<String>,
    usage: Tooltip,
    info_tip: Tooltip,
    info_target: Info,
    info_areas: Vec<(Rect, Info)>,
    connection_counter: CounterTicker,
    counter_bucket: Option<u128>,
    dot_color: u32,
    connection_visible: bool,
    navigation: navigation::Navigation,
    show_chats: bool,
    show_attachments: bool,
    attachments_rect: Rect,
    attachment_scroll: f32,
    max_attachment_scroll: f32,
    attachment_velocity: f32,
    pending_exports: HashMap<String, (PathBuf, String)>,
    export_targets: HashMap<String, DownloadTarget>,
    saving_downloads: HashSet<String>,
    export_errors: HashMap<String, String>,
    download_identity: String,
    progress_clock: Instant,
    progress_bucket: Option<u128>,
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
    notice_popup: notices::NoticePopup,
}
impl App {
    pub fn new(ctx: &impl RenderContext, store: Store, wake: Wake, mobile: bool) -> Result<Self> {
        let controller = Controller::new(store, wake.clone())?;
        let dot_color = controller.health.color(Instant::now());
        let download_identity = controller.identity.clone();
        let navigation = navigation::Navigation::new(&controller);
        let composer = Editor::composer(
            controller
                .selected()
                .map(|c| c.local.draft.clone())
                .unwrap_or_default(),
        );
        let show_chats = controller.account.selected.is_none();
        let needs_setup = controller.settings.url().is_err();
        let mut app = Self {
            controller,
            services: ui::Services {
                renderer: Renderer::new(ctx).map_err(anyhow::Error::msg)?,
                platform: vec![],
            },
            ui: ui::UiState {
                size: ctx.size(),
                origin: Vec2::new(0., 0.),
                scale: 1.,
                mobile,
                window_focused: true,
                dirty: true,
            },
            root: ui::RootWidget {
                legacy: LegacyWorkspace {
                    hits: vec![],
                    composer,
                    code: None,
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
                    revealed_project_position: None,
                    saving_project: None,
                    usage: Tooltip::default(),
                    info_tip: Tooltip::default(),
                    info_target: Info::Connection,
                    info_areas: vec![],
                    connection_counter: CounterTicker::new(wake.clone()),
                    counter_bucket: None,
                    dot_color,
                    connection_visible: true,
                    navigation,
                    show_chats,
                    show_attachments: false,
                    attachments_rect: Rect::new(0., 0., 0., 0.),
                    attachment_scroll: 0.,
                    max_attachment_scroll: 0.,
                    attachment_velocity: 0.,
                    pending_exports: HashMap::new(),
                    export_targets: HashMap::new(),
                    saving_downloads: HashSet::new(),
                    export_errors: HashMap::new(),
                    download_identity,
                    progress_clock: Instant::now(),
                    progress_bucket: None,
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
                    notice_popup: notices::NoticePopup::default(),
                },
            },
        };
        if needs_setup {
            app.apply(Action::Settings)?;
        }
        Ok(app)
    }
    /// Headless-only attachment preview; never starts a transfer.
    #[cfg(not(target_os = "android"))]
    pub(crate) fn preview_attachments(&mut self) { self.root.legacy.show_attachments = true; }
    /// Headless-only heartbeat injection; never starts a socket or uses credentials.
    #[cfg(not(target_os = "android"))]
    pub fn preview_connection(&mut self, preview: ConnectionPreview) {
        use std::time::Duration;
        let now = Instant::now();
        self.controller.health = crate::connection::Health::default();
        if !matches!(preview, ConnectionPreview::Unconfigured) {
            self.controller.health.connected();
            for (ms, ago) in [(420, 8000), (123, 6000)] {
                self.controller.health.sent(now - Duration::from_millis(ago + ms));
                self.controller.health.reply(Duration::from_millis(ms), now - Duration::from_millis(ago));
            }
        }
        match preview {
            ConnectionPreview::Received => {
                self.controller.epoch = Some(1);
                self.controller.connection = "Connected".into();
                self.controller.health.sent(now - Duration::from_millis(1357));
                self.controller.health.reply(Duration::from_millis(123), now - Duration::from_millis(1234));
            }
            ConnectionPreview::Waiting => {
                self.controller.epoch = Some(1);
                self.controller.connection = "Connected".into();
                self.controller.health.sent(now - Duration::from_millis(4321));
            }
            ConnectionPreview::Disconnected => {
                self.controller.epoch = None;
                self.controller.connection = "Ping timed out".into();
                self.controller.health.disconnected(false);
                self.controller.health.attempt(2, now - Duration::from_millis(1234));
            }
            ConnectionPreview::Unconfigured => {
                self.controller.epoch = None;
                self.controller.connection = "Not connected".into();
            }
        }
        self.root.legacy.info_target = Info::Connection;
        self.root.legacy.info_tip.pinned = true;
        self.root.legacy.info_tip.suppressed = false;
        self.root.legacy.info_tip.progress = 1.;
    }
    pub fn set_connection_visible(&mut self, visible: bool) {
        self.root.legacy.connection_visible = visible;
        if !visible {
            self.root.legacy.connection_counter.sync(None);
            self.root.legacy.counter_bucket = None;
        }
    }
    pub fn resize(&mut self, size: (u32, u32), scale: f32, origin: Vec2) {
        if self.ui.size != size || self.ui.scale != scale || self.ui.origin != origin {
            self.cancel_pointer();
            if self.root.legacy.show_attachments && (self.ui.mobile || size.0 as f32 / scale < 1000.) {
                self.cancel_preedit();
                self.root.legacy.focus = None;
            }
            self.root.legacy.revealed_project.clear();
            self.ui.size = size;
            self.ui.scale = scale;
            self.ui.origin = origin;
            self.ui.dirty = true;
        }
    }
    #[cfg(not(target_os = "android"))]
    pub fn needs_redraw(&self) -> bool {
        self.ui.dirty
    }
    #[cfg(not(target_os = "android"))]
    pub fn hover(&mut self, point: Option<Vec2>) {
        let enabled = self.root.legacy.modal.is_none() && self.root.legacy.viewer.is_none() && self.root.legacy.context_menu.is_none();
        if enabled
            && let Some((rect, target)) =
                point.and_then(|p| self.root.legacy.info_areas.iter().find(|(r, _)| contains(*r, p)))
        {
            if !self.root.legacy.info_target.same_anchor(target) { self.root.legacy.info_tip = Tooltip::default(); }
            self.root.legacy.info_target = target.clone();
            self.root.legacy.info_tip.region = *rect;
        }
        self.root.legacy.usage
            .hover(enabled && point.is_some_and(|p| self.root.legacy.usage.contains(p)));
        self.root.legacy.info_tip
            .hover(enabled && point.is_some_and(|p| self.root.legacy.info_tip.contains(p)));
        if enabled && point.is_some_and(|p| contains(self.root.legacy.info_tip.region, p)) {
            self.root.legacy.usage.dismiss();
        } else if enabled && point.is_some_and(|p| contains(self.root.legacy.usage.region, p)) {
            self.root.legacy.info_tip.dismiss();
        }
        let old = self
            .root.legacy.hover
            .and_then(|p| self.root.legacy.hits.iter().rev().find(|h| contains(h.rect, p)))
            .map(|h| h.rect);
        let new = point
            .and_then(|p| self.root.legacy.hits.iter().rev().find(|h| contains(h.rect, p)))
            .map(|h| h.rect);
        if let Some(auto) = &mut self.root.legacy.autoscroll
            && let Some(point) = point
        {
            auto.pointer = point;
            self.ui.dirty = true;
        }
        let on_bar = |p: Option<Vec2>| {
            p.is_some_and(|p| self.root.legacy.scrollbars.iter().any(|b| contains(b.track, p)))
        };
        self.ui.dirty |= on_bar(self.root.legacy.hover) != on_bar(point);
        if self.root.legacy.modal.is_none() && self.root.legacy.viewer.is_none() && self.root.legacy.context_menu.is_none() {
            self.ui.dirty |= self.root.legacy.hover.and_then(|p| self.section_at(p).map(|(key, _)| key))
                != point.and_then(|p| self.section_at(p).map(|(key, _)| key));
        }
        self.root.legacy.hover = point;
        if self.root.legacy.context_menu.as_ref().is_some_and(|m| m.parent.is_none())
            && let Some(Action::MoveMenu(id)) = point.and_then(|p| self.root.legacy.hits.iter().rev().find(|h| contains(h.rect, p))).map(|h| h.action.clone()) {
            self.move_menu(&id);
        }
        self.ui.dirty |= old != new;
    }
    #[cfg(not(target_os = "android"))]
    pub fn cursor(&self) -> chad::winit::window::CursorIcon {
        use chad::winit::window::CursorIcon;
        if let Some(auto) = &self.root.legacy.autoscroll {
            return if auto.speed(self.ui.scale) < 0. {
                CursorIcon::NResize
            } else if auto.speed(self.ui.scale) > 0. {
                CursorIcon::SResize
            } else {
                CursorIcon::NsResize
            };
        }
        if self.root.legacy.scroll_drag.is_some() {
            return CursorIcon::Default;
        }
        let Some(point) = self.root.legacy.hover else {
            return CursorIcon::Default;
        };
        if self.root.legacy.modal.is_none()
            && self.root.legacy.viewer.is_none()
            && self.root.legacy.context_menu.is_none()
            && (self.root.legacy.info_tip.contains_card(point) || self.root.legacy.usage.contains_card(point))
        {
            return CursorIcon::Default;
        }
        if self.root.legacy.modal.is_none()
            && self.root.legacy.viewer.is_none()
            && self.root.legacy.scrollbars.iter().any(|b| contains(b.track, point))
        {
            return CursorIcon::Default;
        }
        if let Some(hit) = self.root.legacy.hits.iter().rev().find(|h| contains(h.rect, point)) {
            return if matches!(hit.action, Action::Focus(_)) {
                CursorIcon::Text
            } else {
                CursorIcon::Pointer
            };
        }
        if self.root.legacy.modal.is_none() && self.root.legacy.viewer.is_none() {
            if self.services.renderer.hit_link(point).is_some() {
                return CursorIcon::Pointer;
            }
            if contains(self.root.legacy.transcript, point) && self.services.renderer.nearest_text(point).is_some() {
                return CursorIcon::Text;
            }
        }
        CursorIcon::Default
    }
    pub fn actions(&mut self) -> Vec<PlatformAction> {
        std::mem::take(&mut self.services.platform)
    }
    pub fn report(&mut self, result: Result<()>) {
        if let Err(e) = result {
            self.controller.report_error(e);
        }
        self.ui.dirty = true;
    }
    pub fn tick(&mut self, dt: f32) -> bool {
        let visible = self.ui.window_focused && (self.ui.size.0 as f32 / self.ui.scale >= 760. || !self.root.legacy.show_chats)
            && (!self.root.legacy.show_attachments || !self.ui.mobile && self.ui.size.0 as f32 / self.ui.scale >= 1000.)
            && self.root.legacy.modal.is_none() && self.root.legacy.viewer.is_none() && self.root.legacy.code.is_none();
        if let Err(error) = self.controller.viewing(visible) { self.controller.report_error(error); }
        self.ui.dirty |= self.controller.poll();
        if self.root.legacy.download_identity != self.controller.identity {
            self.root.legacy.download_identity = self.controller.identity.clone();
            self.root.legacy.export_errors.clear();
        }
        self.finish_exports();
        if let Some(text)=self.controller.copied.take() && !text.is_empty() {self.services.platform.push(PlatformAction::Copy(text));self.ui.dirty=true;}
        self.ui.dirty |= self.root.legacy.usage.tick();
        self.ui.dirty |= self.root.legacy.info_tip.tick();
        if self.root.legacy.connecting && self.controller.epoch.is_some() {
            self.root.legacy.connecting = false;
            self.root.legacy.modal = None;
            self.root.legacy.focus = None;
            self.ui.dirty = true;
        }
        self.project_result();
        self.sync_navigation();
        self.code_tick(dt);
        // Refresh the selected Codex account periodically, even with its card closed.
        // The account view bounds requests to five minutes (30s after failure).
        if visible && self.root.legacy.connection_visible {
            match self.controller.refresh_codex_usage() {
                Ok(sent) => self.ui.dirty |= sent,
                Err(error) => self.controller.report_error(error),
            }
        }
        if visible && self.root.legacy.usage.region.width > 0. && (self.root.legacy.usage.progress > 0. || self.root.legacy.usage.pinned) {
            let quota = self.controller.codex_usage.content(self.controller.epoch.is_some());
            if self.controller.account.selected.as_ref().and_then(|id|self.controller.account.sessions.iter().find(|s|&s.id==id))
                .and_then(|s|s.model.as_ref()).is_some_and(|m|m.provider=="openai-codex") && !self.root.legacy.usage.content.text.ends_with(&quota.text) {
                self.ui.dirty=true;
            }
        }
        if self.root.legacy.connection_visible && self.root.legacy.info_tip.progress > 0. && self.root.legacy.info_tip.region.width > 0.
            && let Info::CacheTtl(id) = &self.root.legacy.info_target
            && let Some(session) = self.controller.account.sessions.iter().find(|s| &s.id == id)
        {
            self.ui.dirty |= self.root.legacy.info_tip.content != self.controller.cache_ttl(session).details();
        }
        if self.root.legacy.waiting_settings
            && let Some(document) = self.controller.daemon_settings.clone()
        {
            self.root.legacy.waiting_settings = false;
            let result = crate::daemon_settings::Draft::new(
                &document,
                self.controller.identity.clone(),
            );
            match result {
                Ok(draft) => {
                    self.root.legacy.daemon_draft = Some(draft);
                    let result = self.load_setting_field();
                    self.report(result);
                }
                Err(error) => self.report(Err(error)),
            }
        }
        if let Some(request) = &self.root.legacy.saving_settings {
            if self
                .controller
                .settings_result
                .as_ref()
                .is_some_and(|(id, _)| id == request)
            {
                let ok = self.controller.settings_result.take().unwrap().1;
                self.root.legacy.saving_settings = None;
                if ok
                    && let (Some(draft), Some(document)) =
                        (&mut self.root.legacy.daemon_draft, &self.controller.daemon_settings)
                {
                    draft.revision = document.revision;
                    self.controller.notice = Some("Settings saved".into());
                }
                self.ui.dirty = true;
            } else if self.controller.epoch.is_none() {
                self.root.legacy.saving_settings = None;
                self.controller.notice =
                    Some("Save unconfirmed. Reload before saving again; it was not resent.".into());
                self.ui.dirty = true;
            }
        }
        if self.root.legacy.modal.is_some() || self.root.legacy.viewer.is_some() {
            self.root.legacy.usage.dismiss();
            self.root.legacy.info_tip.dismiss();
            self.root.legacy.autoscroll = None;
            self.root.legacy.wheel = None;
        }
        let now = Instant::now();
        let dot_color = self.controller.health.color(now);
        self.ui.dirty |= self.root.legacy.dot_color != dot_color;
        self.root.legacy.dot_color = dot_color;
        let card_visible = self.root.legacy.connection_visible
            && self.root.legacy.info_target == Info::Connection
            && self.root.legacy.info_tip.progress > 0.
            && self.root.legacy.info_tip.region.width > 0.
            && self.root.legacy.modal.is_none() && self.root.legacy.viewer.is_none() && self.root.legacy.context_menu.is_none();
        let counter_bucket = card_visible.then(|| self.controller.health.counter(now))
            .flatten().map(|(_, ms)| ms / COUNTER_REFRESH.as_millis());
        let next_wake = if !self.root.legacy.connection_visible || self.root.legacy.modal.is_some() || self.root.legacy.viewer.is_some() {
            None
        } else if card_visible && counter_bucket.is_some() {
            Some(COUNTER_REFRESH)
        } else {
            self.controller.health.next_color_wake(now)
        };
        let indeterminate = self.root.legacy.connection_visible && self.root.legacy.modal.is_none() && self.root.legacy.viewer.is_none()
            && (!self.ui.mobile || !self.root.legacy.show_chats || self.root.legacy.show_attachments)
            && (self.controller.downloads.values().any(|d|!d.status.done && d.status.total==0)
                || !self.root.legacy.saving_downloads.is_empty());
        let progress_bucket=indeterminate.then(||now.duration_since(self.root.legacy.progress_clock).as_millis()/80);
        self.ui.dirty |= self.root.legacy.progress_bucket!=progress_bucket;
        self.root.legacy.progress_bucket=progress_bucket;
        let next_wake=if indeterminate {Some(next_wake.map_or(std::time::Duration::from_millis(80),
            |duration|duration.min(std::time::Duration::from_millis(80))))} else {next_wake};
        // Quota reset / TTL text stays current when pinned, even while offline.
        // Share the existing timer; closed cards do not acquire a redraw loop.
        let timed_tooltip = self.root.legacy.connection_visible && self.root.legacy.modal.is_none() && self.root.legacy.viewer.is_none()
            && self.root.legacy.context_menu.is_none()
            && (self.root.legacy.usage.progress > 0. && self.root.legacy.usage.region.width > 0.
                || self.root.legacy.info_tip.progress > 0. && self.root.legacy.info_tip.region.width > 0. && matches!(self.root.legacy.info_target, Info::CacheTtl(_)));
        let next_wake = if timed_tooltip { Some(next_wake.map_or(std::time::Duration::from_secs(1),
            |duration| duration.min(std::time::Duration::from_secs(1)))) } else { next_wake };
        self.ui.dirty |= self.root.legacy.notice_popup.observe(self.controller.notice.as_ref(), now);
        let next_wake = match (next_wake, self.root.legacy.notice_popup.remaining(now)) {
            (Some(a), Some(b)) => Some(a.min(b)), (a, b) => a.or(b),
        };
        self.root.legacy.connection_counter.sync(next_wake);
        // Redraw only when the visible counter or dot actually changes.
        self.ui.dirty |= self.root.legacy.counter_bucket != counter_bucket;
        self.root.legacy.counter_bucket = counter_bucket;
        if let Some(mut wheel) = self.root.legacy.wheel.take() {
            let (value, max) = self.scroll_value(wheel.lane);
            let (next, settled) = wheel.step(value, max, self.ui.scale);
            let lane = wheel.lane;
            if !settled {
                self.root.legacy.wheel = Some(wheel);
            }
            self.set_scroll(lane, next);
            self.ui.dirty = true;
        }
        if let Some(auto) = &self.root.legacy.autoscroll {
            let next =
                (self.root.legacy.scroll + auto.speed(self.ui.scale) * dt.min(0.05)).clamp(0., self.root.legacy.max_scroll);
            if (next - self.root.legacy.scroll).abs() > 0.001 {
                self.set_scroll(Lane::Transcript, next);
                self.ui.dirty = true;
            }
        }
        if self.root.legacy.field_selection.is_some()
            && let Some(point) = self.root.legacy.pointer.as_ref().filter(|p| p.dragged).map(|p| p.last)
            && let Some((editor, renderer)) = self.editor_and_renderer()
        {
            let moved = editor.drag_scroll(&mut renderer.text, renderer.faces.prose[0], point, dt);
            self.ui.dirty |= moved;
        }
        if self.root.legacy.selecting
            && let Some(p) = &self.root.legacy.pointer
            && p.dragged
        {
            let y = p.last.y;
            let margin = 16. * self.ui.scale;
            let speed = if y < self.root.legacy.transcript.y + margin {
                (y - self.root.legacy.transcript.y - margin) * 20.
            } else if y > self.root.legacy.transcript.y + self.root.legacy.transcript.height - margin {
                (y - self.root.legacy.transcript.y - self.root.legacy.transcript.height + margin) * 20.
            } else {
                0.
            };
            let next = (self.root.legacy.scroll
                + speed.clamp(-1800. * self.ui.scale, 1800. * self.ui.scale) * dt.min(0.05))
            .clamp(0., self.root.legacy.max_scroll);
            if (next - self.root.legacy.scroll).abs() > 0.001 {
                self.set_scroll(Lane::Transcript, next);
                self.ui.dirty = true;
            }
        }
        if self.root.legacy.pointer.is_none() && self.root.legacy.velocity.abs() > 4. {
            let old = self.root.legacy.scroll;
            self.root.legacy.scroll = (self.root.legacy.scroll + self.root.legacy.velocity * dt.min(0.05)).clamp(0., self.root.legacy.max_scroll);
            self.root.legacy.velocity *= (-9. * dt).exp();
            if (old - self.root.legacy.scroll).abs() < 0.1 {
                self.root.legacy.velocity = 0.;
            }
            self.remember_scroll();
            self.ui.dirty = true;
        }
        if self.root.legacy.pointer.is_none() && self.root.legacy.attachment_velocity.abs() > 4. {
            let old = self.root.legacy.attachment_scroll;
            self.root.legacy.attachment_scroll = (old + self.root.legacy.attachment_velocity * dt.min(0.05)).clamp(0., self.root.legacy.max_attachment_scroll);
            self.root.legacy.attachment_velocity *= (-9. * dt).exp();
            if (old - self.root.legacy.attachment_scroll).abs() < 0.1 { self.root.legacy.attachment_velocity = 0.; }
            self.ui.dirty = true;
        }
        if self.root.legacy.pointer.is_none() && self.root.legacy.project_velocity.abs() > 4. {
            let old = self.root.legacy.project_scroll;
            self.root.legacy.project_scroll = (old + self.root.legacy.project_velocity * dt.min(0.05)).clamp(0., self.root.legacy.max_project_scroll);
            self.root.legacy.project_velocity *= (-9. * dt).exp();
            if (old - self.root.legacy.project_scroll).abs() < 0.1 { self.root.legacy.project_velocity = 0.; }
            self.ui.dirty = true;
        }
        if let Some(point) = self.root.legacy.pointer.as_ref().filter(|p| p.touch && !p.dragged && p.started.elapsed().as_millis() >= 450).map(|p| p.start)
            && self.root.legacy.modal.is_none() && self.root.legacy.context_menu.is_none() && self.root.legacy.viewer.is_none()
            && (self.root.legacy.project_areas.iter().any(|(r,_)| contains(*r, point)) || self.root.legacy.chat_areas.iter().any(|(r,_)| contains(*r, point))) {
            self.context_at(point);
        }
        if let Some(point) = self.root.legacy.pointer.as_ref().filter(|p| p.touch && !p.dragged && p.started.elapsed().as_millis() >= 450).map(|p| p.start)
            && self.root.legacy.modal.is_none() && self.root.legacy.context_menu.is_none() && self.root.legacy.viewer.is_none()
            && let Some((_, info)) = self.root.legacy.info_areas.iter().find(|(r, info)| matches!(info, Info::Attachment(..)) && contains(*r, point)) {
            let info = info.clone();
            self.root.legacy.pointer = None;
            self.root.legacy.ripple = None;
            self.root.legacy.selecting = false;
            self.activate(Action::Info(info));
        }
        let waiting_hold = self.root.legacy.pointer.as_ref().is_some_and(|p| p.touch && !p.dragged && p.started.elapsed().as_millis() < 450)
            && self.root.legacy.modal.is_none() && self.root.legacy.context_menu.is_none() && self.root.legacy.viewer.is_none();
        if let Some(ripple) = &self.root.legacy.ripple {
            let now = Instant::now();
            if ripple.finished(now) {
                self.root.legacy.ripple = None;
                self.ui.dirty = true;
            } else if ripple.animating(now) {
                self.ui.dirty = true;
            }
        }
        let dirty = self.ui.dirty;
        self.ui.dirty = false;
        dirty || self.root.legacy.velocity.abs() > 4. || self.root.legacy.project_velocity.abs() > 4. || self.root.legacy.attachment_velocity.abs() > 4. || waiting_hold
    }
    fn section_at(&self, point: Vec2) -> Option<(&str, Rect)> {
        self.root.legacy.detail_areas.iter().rev().find(|a| a.contains(point))
            .map(|a| (a.key.as_str(), a.rect))
            .or_else(|| self.root.legacy.message_areas.iter().rev().find(|a| a.contains(point))
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
        if self.root.legacy.navigation.download.is_some() || self.root.legacy.navigation.identity != self.controller.identity { return; }
        if let Some(id) = self.controller.account.selected.clone()
            && self.root.legacy.placed_session.as_deref() == Some(id.as_str())
            && let Some(anchor) = self.root.legacy.placed.iter().find(|r| r.top + r.height >= self.root.legacy.scroll)
            && let Some(chat) = self.controller.chats.get_mut(&id)
        {
            chat.local.position.key = Some(anchor.key.clone());
            chat.local.position.offset = (self.root.legacy.scroll - anchor.top) / self.ui.scale;
            chat.local.position.follow = self.root.legacy.expansion_pin.is_none()
                && !self
                    .root.legacy.wheel
                    .as_ref()
                    .is_some_and(|w| w.lane == Lane::Transcript)
                && self.root.legacy.autoscroll.is_none()
                && !self
                    .root.legacy.scroll_drag
                    .as_ref()
                    .is_some_and(|d| d.lane == Lane::Transcript)
                && self.root.legacy.max_scroll - self.root.legacy.scroll < self.ui.scale;
        }
    }
    pub fn back(&mut self) {
        if self.root.legacy.info_tip.pinned
            || self.root.legacy.info_tip.progress > 0.
            || self.root.legacy.usage.pinned
            || self.root.legacy.usage.progress > 0.
        {
            self.root.legacy.info_tip.dismiss();
            self.root.legacy.usage.dismiss();
            self.ui.dirty = true;
            return;
        }
        if self.root.legacy.context_menu.is_some() {
            self.root.legacy.context_menu = self.root.legacy.context_menu.take().and_then(|m| m.parent.map(|p| *p));
            self.ui.dirty = true;
            return;
        }
        self.root.legacy.focus = None;
        self.root.legacy.navigation.download = None;
        if self.root.legacy.viewer.take().is_some() {
            self.root.legacy.viewer_image = None;
        } else if self.root.legacy.modal.is_some() {
            self.activate(Action::CancelModal);
        } else if self.root.legacy.code.is_some() {
            self.code_back();
        } else if self.root.legacy.show_attachments {
            self.root.legacy.show_attachments = false;
            self.cancel_pointer();
        } else if !self.root.legacy.show_chats && self.ui.size.0 as f32 / self.ui.scale < 760. {
            self.root.legacy.show_chats = true;
        } else {
            self.services.platform.push(PlatformAction::Background);
        }
        self.ui.dirty = true;
    }
    fn scroll_value(&self, lane: Lane) -> (f32, f32) {
        match lane {
            Lane::Transcript => (self.root.legacy.scroll, self.root.legacy.max_scroll),
            Lane::Sidebar => (self.root.legacy.list_scroll, self.root.legacy.max_list_scroll),
            Lane::Attachments => (self.root.legacy.attachment_scroll, self.root.legacy.max_attachment_scroll),
            Lane::Files => self.root.legacy.code.as_ref().map_or((0.,0.),|c|(c.scroll,c.max_scroll)),
            Lane::Projects => (self.root.legacy.project_scroll, self.root.legacy.max_project_scroll),
            Lane::Horizontal => (self.root.legacy.horizontal, self.root.legacy.max_horizontal),
        }
    }
    fn set_scroll(&mut self, lane: Lane, value: f32) {
        match lane {
            Lane::Transcript => {
                self.root.legacy.navigation.download = None;
                self.root.legacy.scroll = value.clamp(0., self.root.legacy.max_scroll);
                self.remember_scroll();
            }
            Lane::Sidebar => self.root.legacy.list_scroll = value.clamp(0., self.root.legacy.max_list_scroll),
            Lane::Attachments => self.root.legacy.attachment_scroll = value.clamp(0., self.root.legacy.max_attachment_scroll),
            Lane::Files => { if let Some(c)=&mut self.root.legacy.code {c.scroll=value.clamp(0.,c.max_scroll);} },
            Lane::Projects => self.root.legacy.project_scroll = value.clamp(0., self.root.legacy.max_project_scroll),
            Lane::Horizontal => self.root.legacy.horizontal = value.clamp(0., self.root.legacy.max_horizontal),
        }
    }
    pub fn cancel_autoscroll(&mut self) -> bool {
        let active = self.root.legacy.autoscroll.take().is_some();
        self.ui.dirty |= active;
        active
    }
    #[cfg(not(target_os = "android"))]
    pub fn middle(&mut self, pressed: bool, point: Vec2) {
        if pressed {
            if self.cancel_autoscroll() {
                return;
            }
            if self.root.legacy.modal.is_some()
                || self.root.legacy.viewer.is_some()
                || !contains(self.root.legacy.transcript, point)
                || self.root.legacy.max_scroll <= 0.
            {
                return;
            }
            self.cancel_pointer();
            self.root.legacy.autoscroll = Some(Autoscroll {
                anchor: point,
                pointer: point,
                pressed: Some(Instant::now()),
            });
        } else if let Some(auto) = &mut self.root.legacy.autoscroll {
            // Quick click latches; a held press scrolls only until release, as in Tau 1.
            if auto
                .pressed
                .take()
                .is_some_and(|at| at.elapsed().as_millis() >= 220)
            {
                self.root.legacy.autoscroll = None;
            }
        }
        self.remember_scroll();
        self.ui.dirty = true;
    }
    #[cfg(not(target_os = "android"))]
    pub fn wheel(&mut self, amount: f32, horizontal: bool, point: Vec2) {
        if self.root.legacy.context_menu.is_some() && contains(self.root.legacy.context_rect, point) {
            self.scroll_menu(amount);
            return;
        }
        self.root.legacy.context_menu = None;
        self.root.legacy.project_velocity = 0.;
        self.root.legacy.attachment_velocity = 0.;
        self.root.legacy.usage.dismiss();
        self.root.legacy.info_tip.dismiss();
        self.cancel_autoscroll();
        self.root.legacy.expansion_pin = None;
        self.root.legacy.history_attempt = None;
        self.root.legacy.velocity = 0.;
        if self.root.legacy.viewer.is_none() && let Some(field) = self.field_at(point) {
            let editor = match field {
                None => &mut self.root.legacy.composer,
                Some(code_view::SEARCH_FIELD) => self.root.legacy.code.as_mut().unwrap().search.as_mut().unwrap(),
                Some(i) => &mut self.root.legacy.modal.as_mut().unwrap().fields[i].1,
            };
            editor.wheel(&mut self.services.renderer.text, self.services.renderer.faces.prose[0], amount, horizontal);
            self.ui.dirty = true;
            return;
        }
        if self.root.legacy.modal.is_none() && self.code_wheel(amount, horizontal, point) { return; }
        if let Some(v) = &mut self.root.legacy.viewer {
            v.zoom = (v.zoom * (-amount * 0.002).exp()).clamp(1., 16.);
        } else if self.root.legacy.modal.is_none() {
            let lane = if contains(self.root.legacy.attachments_rect, point) {
                Lane::Attachments
            } else if contains(self.root.legacy.projects_rect, point) {
                Lane::Projects
            } else if horizontal {
                Lane::Horizontal
            } else if self.root.legacy.show_chats
                || self.ui.size.0 as f32 / self.ui.scale >= 760.
                    && point.x < self.ui.origin.x + 300. * self.ui.scale
            {
                Lane::Sidebar
            } else {
                Lane::Transcript
            };
            let (value, max) = self.scroll_value(lane);
            if let Some(wheel) = &mut self.root.legacy.wheel
                && wheel.lane == lane
            {
                wheel.target = (wheel.target + amount).clamp(0., max);
            } else {
                self.root.legacy.wheel = Some(Wheel {
                    lane,
                    target: (value + amount).clamp(0., max),
                    last: Instant::now(),
                });
            }
        }
        self.ui.dirty = true;
    }
    fn scrollbar(&mut self, layer: &mut Layer, lane: Lane, viewport: Rect) {
        let (value, max) = self.scroll_value(lane);
        if let Some(bar) = Scrollbar::new(lane, viewport, value, max, self.ui.scale) {
            let active = self.root.legacy.scroll_drag.as_ref().is_some_and(|d| d.lane == lane)
                || self.root.legacy.hover.is_some_and(|p| contains(bar.track, p));
            let width = if active { 8. } else { 6. } * self.ui.scale;
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
            self.root.legacy.scrollbars.push(bar);
        }
    }
    fn history_near_edge(&mut self, session: &str, near: bool) {
        if self.root.legacy.modal.is_some() || self.root.legacy.viewer.is_some() || !near {
            return;
        }
        let feed = &self.controller.chats[session].feed;
        if !feed.synchronized || feed.loading {
            return;
        }
        if let (Some(before), Some(epoch)) = (feed.before, self.controller.epoch) {
            let attempt = (session.to_owned(), feed.generation.clone(), before, epoch);
            if self.root.legacy.history_attempt.as_ref() == Some(&attempt) {
                return;
            }
            self.root.legacy.history_attempt = Some(attempt);
            let result = self.controller.history();
            self.report(result);
        }
    }
    pub fn press(&mut self, id: u64, point: Vec2, touch: bool) {
        if let Some(hit) = self.root.legacy.hits.iter().rev().find(|hit| contains(hit.rect, point))
            && matches!(hit.action, Action::DismissNotice | Action::OpenDownloadNotice(_)) {
            let action = hit.action.clone();
            self.cancel_pointer();
            self.activate(action);
            return;
        }
        if self.root.legacy.context_menu.is_some() && !contains(self.root.legacy.context_rect, point) {
            self.root.legacy.context_menu = None;
            self.ui.dirty = true;
            return;
        }
        if self.root.legacy.modal.is_none()
            && self.root.legacy.viewer.is_none()
            && self.root.legacy.context_menu.is_none()
            && (self.root.legacy.info_tip.contains_card(point) || self.root.legacy.usage.contains_card(point))
        {
            // Read-only cards must not activate the list/message behind them.
            self.ui.dirty = true;
            return;
        }
        if !self.root.legacy.usage.contains(point) {
            self.root.legacy.usage.dismiss();
        }
        if !self.root.legacy.info_tip.contains(point) {
            self.root.legacy.info_tip.dismiss();
        }
        self.root.legacy.wheel = None;
        self.root.legacy.expansion_pin = None;
        self.root.legacy.history_attempt = None;
        self.root.legacy.velocity = 0.;
        self.root.legacy.project_velocity = 0.;
        self.root.legacy.attachment_velocity = 0.;
        self.root.legacy.ripple = None;
        if self.root.legacy.pointer.is_some() {
            if self.root.legacy.viewer.is_some() && touch {
                self.root.legacy.pinch = Some((id, point));
            }
            return;
        }
        self.root.legacy.selecting = false;
        self.root.legacy.field_selection = None;
        if self.root.legacy.modal.is_none()
            && self.root.legacy.viewer.is_none()
            && self.root.legacy.context_menu.is_none()
            && let Some(bar) = self
                .root.legacy.scrollbars
                .iter()
                .find(|b| contains(b.track, point))
                .copied()
        {
            if contains(bar.thumb, point) {
                self.root.legacy.scroll_drag = Some(Drag {
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
            self.root.legacy.pointer = Some(Pointer {
                id,
                start: point,
                last: point,
                at: Instant::now(),
                started: Instant::now(),
                dragged: true,
                touch,
            });
            self.ui.dirty = true;
            return;
        }
        if self.code_press(id, point, touch) { return; }
        if !touch && self.root.legacy.viewer.is_none() {
            if let Some(hit) = self.root.legacy.hits.iter().rev().find(|h| contains(h.rect, point)) {
                // Controls take priority over transcript selection beneath them.
                if let Action::Focus(field) = hit.action {
                    let rect = hit.rect;
                    self.cancel_preedit();
                    self.root.legacy.focus = Some(field);
                    self.root.legacy.field_selection = Some(rect);
                    self.field_hit(point, false);
                }
            } else if self.root.legacy.modal.is_none()
                && self.root.legacy.context_menu.is_none()
                && contains(self.root.legacy.transcript, point)
                && let Some(caret) = self.services.renderer.nearest_text(point)
            {
                self.services.renderer.begin_selection(caret);
                self.root.legacy.selecting = true;
                self.root.legacy.focus = None;
            }
        }
        self.root.legacy.pointer = Some(Pointer {
            id,
            start: point,
            last: point,
            at: Instant::now(),
            started: Instant::now(),
            dragged: false,
            touch,
        });
        self.root.legacy.ripple = if self.root.legacy.modal.is_none() && self.root.legacy.viewer.is_none() && self.root.legacy.context_menu.is_none() {
            self.section_at(point).map(|(key, rect)| Ripple::new(key.to_owned(), rect, point))
        } else { None };
        self.ui.dirty = true;
    }
    pub fn motion(&mut self, id: u64, point: Vec2) {
        if self.code_motion(id, point) { return; }
        let Some(p) = &mut self.root.legacy.pointer else {
            return;
        };
        if let Some((second, other)) = &mut self.root.legacy.pinch
            && let Some(viewer) = &mut self.root.legacy.viewer
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
            self.ui.dirty = true;
            return;
        }
        if p.id != id {
            return;
        }
        if let Some(drag) = &self.root.legacy.scroll_drag {
            p.last = point;
            if let Some(bar) = self
                .root.legacy.scrollbars
                .iter()
                .find(|b| b.lane == drag.lane)
                .copied()
            {
                let value = bar.value_at(point.y, drag.grab);
                self.set_scroll(bar.lane, value);
            }
            self.ui.dirty = true;
            return;
        }
        if self.root.legacy.selecting {
            if let Some(caret) = self.services.renderer.nearest_text(point) {
                self.services.renderer.extend_selection(caret);
            }
            p.dragged |=
                (point.x - p.start.x).abs() + (point.y - p.start.y).abs() > 4. * self.ui.scale;
            if p.dragged { self.root.legacy.ripple = None; }
            p.last = point;
            self.ui.dirty = true;
            return;
        }
        if self.root.legacy.field_selection.is_some() {
            p.dragged = true;
            p.last = point;
            self.field_hit(point, true);
            self.ui.dirty = true;
            return;
        }
        let dy = point.y - p.last.y;
        let dx = point.x - p.last.x;
        p.dragged |= (point.x - p.start.x).abs() + (point.y - p.start.y).abs() > 7. * self.ui.scale;
        if p.dragged {
            if self.root.legacy.context_menu.is_some() {
                p.last = point;
                p.at = Instant::now();
                self.scroll_menu(-dy);
                return;
            }
            if let Some(v) = &mut self.root.legacy.viewer {
                v.pan.x += dx;
                v.pan.y += dy;
            } else if self.root.legacy.modal.is_none() {
                if contains(self.root.legacy.projects_rect, p.start) {
                    self.root.legacy.project_scroll = (self.root.legacy.project_scroll - dx).clamp(0., self.root.legacy.max_project_scroll);
                    if p.touch { self.root.legacy.project_velocity = (-dx / p.at.elapsed().as_secs_f32().max(0.008)).clamp(-3000. * self.ui.scale, 3000. * self.ui.scale); }
                } else if contains(self.root.legacy.attachments_rect, p.start) {
                    self.root.legacy.attachment_scroll = (self.root.legacy.attachment_scroll - dy).clamp(0., self.root.legacy.max_attachment_scroll);
                    if p.touch {
                        self.root.legacy.attachment_velocity = (-dy / p.at.elapsed().as_secs_f32().max(0.008))
                            .clamp(-3000. * self.ui.scale, 3000. * self.ui.scale);
                    }
                } else if contains(self.root.legacy.list_rect, p.start) {
                    self.root.legacy.list_scroll = (self.root.legacy.list_scroll - dy).clamp(0., self.root.legacy.max_list_scroll);
                } else if contains(self.root.legacy.transcript, p.start)
                    && self.root.legacy.max_horizontal > 0.
                    && (point.x - p.start.x).abs() > 1.5 * (point.y - p.start.y).abs()
                {
                    self.root.legacy.horizontal = (self.root.legacy.horizontal - dx).clamp(0., self.root.legacy.max_horizontal);
                    self.root.legacy.velocity = 0.;
                } else if contains(self.root.legacy.transcript, p.start) {
                    self.root.legacy.navigation.download = None;
                    self.root.legacy.scroll = (self.root.legacy.scroll - dy).clamp(0., self.root.legacy.max_scroll);
                    if p.touch {
                        self.root.legacy.velocity = (-dy / p.at.elapsed().as_secs_f32().max(0.008))
                            .clamp(-3000. * self.ui.scale, 3000. * self.ui.scale);
                    }
                }
            }
        }
        if p.dragged { self.root.legacy.ripple = None; }
        p.last = point;
        p.at = Instant::now();
        self.remember_scroll();
        self.ui.dirty = true;
    }
    pub fn release(&mut self, id: u64, point: Vec2) {
        if self.code_release(id, point) { return; }
        if self.root.legacy.pinch.take().is_some() {
            self.root.legacy.pointer = None;
            self.ui.dirty = true;
            return;
        }
        if self.root.legacy.pointer.as_ref().is_none_or(|p| p.id != id) {
            return;
        }
        let p = self.root.legacy.pointer.take().unwrap();
        self.root.legacy.scroll_drag = None;
        if !p.dragged {
            if self.root.legacy.viewer.is_some() {
                if let Some(hit) = self.root.legacy.hits.iter().rev().find(|h|
                    contains(h.rect, point) && contains(h.rect, p.start)) {
                    self.activate(hit.action.clone());
                } else if self.root.legacy.viewer_image.is_none_or(|image|
                    !contains(image, p.start) && !contains(image, point)) {
                    self.root.legacy.viewer = None;
                    self.root.legacy.viewer_image = None;
                }
            } else if p.touch && p.started.elapsed().as_millis() > 450
                && self.root.legacy.info_areas.iter().any(|(r, info)| matches!(info, Info::Attachment(..))
                    && contains(*r, point) && contains(*r, p.start)) {
                let info = self.root.legacy.info_areas.iter().find(|(r, info)| matches!(info, Info::Attachment(..))
                    && contains(*r, point) && contains(*r, p.start)).unwrap().1.clone();
                self.activate(Action::Info(info));
            } else if p.touch
                && p.started.elapsed().as_millis() > 450
                && self.root.legacy.context_menu.is_none()
                && (self.root.legacy.project_areas.iter().any(|(r,_)| contains(*r, point) && contains(*r, p.start))
                    || contains(self.root.legacy.transcript, point)
                    || self
                        .root.legacy.chat_areas
                        .iter()
                        .any(|(r, _)| contains(*r, point) && contains(*r, p.start)))
            {
                self.context_at(point);
            } else if let Some(hit) = self
                .root.legacy.hits
                .iter()
                .rev()
                .find(|h| contains(h.rect, point) && contains(h.rect, p.start))
            {
                self.activate(hit.action.clone());
            } else if self.root.legacy.context_menu.is_some() {
                // Empty menu space never activates the transcript behind it.
            } else if let Some(link) = self.services.renderer.hit_link(point) {
                self.activate(Action::Link(link));
            } else if p.touch
                && p.at.elapsed().as_millis() > 450
                && let Some((key, _)) = self.services.renderer.hit_text(point)
            {
                let end = self.services.renderer.messages[&key].source.len();
                self.services.renderer.begin_selection(crate::render::TextPoint {
                    key: key.clone(),
                    byte: 0,
                });
                self.services.renderer
                    .extend_selection(crate::render::TextPoint { key, byte: end });
            }
        }
        if p.dragged { self.root.legacy.ripple = None; }
        else if let Some(ripple) = &mut self.root.legacy.ripple { ripple.release(); }
        self.root.legacy.selecting = false;
        self.root.legacy.field_selection = None;
        if p.at.elapsed().as_millis() > 150 {
            self.root.legacy.velocity = 0.;
            self.root.legacy.project_velocity = 0.;
            self.root.legacy.attachment_velocity = 0.;
        }
        let result = self.save();
        self.report(result);
    }
    pub fn cancel_pointer(&mut self) {
        if let Some(code)=&mut self.root.legacy.code { code.drag_anchor=None; }
        self.root.legacy.context_menu = None;
        self.root.legacy.usage.dismiss();
        self.root.legacy.info_tip.dismiss();
        self.ui.dirty = true;
        self.root.legacy.hover = None;
        self.root.legacy.autoscroll = None;
        self.root.legacy.wheel = None;
        self.root.legacy.scroll_drag = None;
        self.root.legacy.expansion_pin = None;
        self.root.legacy.pointer = None;
        self.root.legacy.ripple = None;
        self.root.legacy.pinch = None;
        self.root.legacy.velocity = 0.;
        self.root.legacy.project_velocity = 0.;
        self.root.legacy.attachment_velocity = 0.;
        self.root.legacy.selecting = false;
        self.root.legacy.field_selection = None;
    }
    fn editor_and_renderer(&mut self) -> Option<(&mut Editor, &mut Renderer)> {
        let editor = match self.root.legacy.focus? {
            None => &mut self.root.legacy.composer,
            Some(code_view::SEARCH_FIELD) if self.root.legacy.modal.is_none() => self.root.legacy.code.as_mut()?.search.as_mut()?,
            Some(i) => &mut self.root.legacy.modal.as_mut()?.fields.get_mut(i)?.1,
        };
        Some((editor, &mut self.services.renderer))
    }
    fn field_hit(&mut self, point: Vec2, extend: bool) {
        if let Some((editor, renderer)) = self.editor_and_renderer() {
            editor.hit(&mut renderer.text, renderer.faces.prose[0], point, extend);
        }
    }
    fn field_at(&self, point: Vec2) -> Option<Option<usize>> {
        if let Some(modal) = &self.root.legacy.modal {
            modal.fields.iter().rposition(|(_, e, _)| e.contains(point)).map(Some)
        } else {
            if self.root.legacy.code.as_ref().and_then(|c|c.search.as_ref()).is_some_and(|e|e.contains(point)) { Some(Some(code_view::SEARCH_FIELD)) }
            else { self.root.legacy.composer.contains(point).then_some(None) }
        }
    }
    pub fn ime_rect(&self) -> Option<Rect> {
        let editor = match self.root.legacy.focus? {
            None => &self.root.legacy.composer,
            Some(code_view::SEARCH_FIELD) if self.root.legacy.modal.is_none() => self.root.legacy.code.as_ref()?.search.as_ref()?,
            Some(i) => &self.root.legacy.modal.as_ref()?.fields.get(i)?.1,
        };
        editor.ime_rect(&self.services.renderer.text)
    }
    pub fn cancel_preedit(&mut self) {
        if let Some(e) = self.editor() && e.composing() {
            e.preedit(String::new(), None);
            self.ui.dirty = true;
        }
    }
    pub fn composing(&self) -> bool {
        match self.root.legacy.focus {
            Some(None) => self.root.legacy.composer.composing(),
            Some(Some(code_view::SEARCH_FIELD)) if self.root.legacy.modal.is_none() => self.root.legacy.code.as_ref().and_then(|c|c.search.as_ref()).is_some_and(|e|e.composing()),
            Some(Some(i)) => self.root.legacy.modal.as_ref().and_then(|m| m.fields.get(i)).is_some_and(|(_, e, _)| e.composing()),
            None => false,
        }
    }
    fn editor(&mut self) -> Option<&mut Editor> {
        match self.root.legacy.focus? {
            None => Some(&mut self.root.legacy.composer),
            Some(code_view::SEARCH_FIELD) if self.root.legacy.modal.is_none() => self.root.legacy.code.as_mut()?.search.as_mut(),
            Some(i) => self.root.legacy.modal.as_mut()?.fields.get_mut(i).map(|(_, e, _)| e),
        }
    }
    pub fn input(&mut self, value: &str) {
        if self.root.legacy.focus.is_none() && self.root.legacy.modal.is_none() && self.root.legacy.code.is_some() && self.code_key(value, false, false) { return; }
        if self.editor().is_some_and(|e| e.replace(value)) { self.edited(); }
        self.ui.dirty = true;
    }
    #[cfg(target_os = "android")]
    pub fn native_edit(&mut self, value: String) {
        let value=self.code_native_value(value);
        if self.editor().is_some_and(|e| e.replace_all(&value)) { self.edited(); }
        self.ui.dirty = true;
    }
    fn edited(&mut self) {
        if matches!(
            self.root.legacy.modal.as_ref().map(|m| &m.kind),
            Some(ModalKind::Settings | ModalKind::Models | ModalKind::Daemon)
        ) {
            self.controller.notice = None;
        }
        if self.root.legacy.focus == Some(Some(code_view::SEARCH_FIELD)) && self.root.legacy.modal.is_none() { self.code_query(); }
        if self.root.legacy.focus == Some(None) {
            let result = self.controller.draft(self.root.legacy.composer.value.clone());
            self.report(result);
        }
        self.ui.dirty = true;
    }
    #[cfg(not(target_os = "android"))]
    pub fn preedit(&mut self, text: String, cursor: Option<(usize, usize)>) {
        if let Some(e) = self.editor() {
            e.preedit(text, cursor);
        }
        self.ui.dirty = true;
    }
    pub fn key(&mut self, key: &str, ctrl: bool, shift: bool) {
        // Composition belongs to the IME. Enter must not send the draft, and
        // Escape must not abort the agent, while candidate text is active.
        if self.composing() {
            if key == "Escape" {
                if let Some(e) = self.editor() { e.preedit(String::new(), None); }
                self.ui.dirty = true;
            }
            return;
        }
        if let Some(menu) = &mut self.root.legacy.context_menu {
            match key {
                "Escape" | "ArrowLeft" => {
                    self.root.legacy.context_menu = self.root.legacy.context_menu.take().and_then(|m| m.parent.map(|p| *p));
                }
                "ArrowUp" => {
                    self.root.legacy.hover = None;
                    menu.selected = (menu.selected + menu.options.len() - 1) % menu.options.len()
                }
                "ArrowDown" => {
                    self.root.legacy.hover = None;
                    menu.selected = (menu.selected + 1) % menu.options.len();
                }
                "Enter" | "ArrowRight" => {
                    let action = menu.options[menu.selected].1.clone();
                    if key == "Enter" || matches!(action, Action::MoveMenu(_)) { self.activate(action); }
                }
                _ => {}
            }
            self.reveal_menu_selection();
            self.ui.dirty = true;
            return;
        }
        if key == "Escape"
            && (self.root.legacy.usage.pinned
                || self.root.legacy.usage.progress > 0.
                || self.root.legacy.info_tip.pinned
                || self.root.legacy.info_tip.progress > 0.)
        {
            self.root.legacy.usage.dismiss();
            self.root.legacy.info_tip.dismiss();
            self.ui.dirty = true;
            return;
        }
        if self.root.legacy.modal.is_none() && self.root.legacy.viewer.is_none() && self.code_key(key, ctrl, shift) { return; }
        self.root.legacy.expansion_pin = None;
        self.root.legacy.wheel = None;
        if let Some(modal) = &self.root.legacy.modal {
            if key == "Tab" && !ctrl && !modal.fields.is_empty() {
                let fields: Vec<_> = self.root.legacy.hits.iter().filter_map(|h| match h.action {
                    Action::Focus(Some(i)) => Some(i), _ => None,
                }).collect();
                if !fields.is_empty() {
                    let current = fields.iter().position(|&i| self.root.legacy.focus == Some(Some(i)));
                    let next = if shift { current.map_or(fields.len() - 1, |i| (i + fields.len() - 1) % fields.len()) }
                        else { current.map_or(0, |i| (i + 1) % fields.len()) };
                    self.root.legacy.focus = Some(Some(fields[next]));
                }
                self.ui.dirty = true;
                return;
            }
            if key == "Enter" && matches!(modal.kind, ModalKind::Settings | ModalKind::Rename(_)) {
                self.activate(Action::Confirm);
                return;
            }
        }
        if key == "Escape" {
            if self.root.legacy.modal.is_some() || self.root.legacy.viewer.is_some() || self.root.legacy.show_attachments {
                self.back();
            } else {
                self.activate(Action::Abort);
            }
            return;
        }
        if ctrl && key.eq_ignore_ascii_case("v") {
            self.services.platform.push(PlatformAction::Paste);
            return;
        }
        if ctrl && (key.eq_ignore_ascii_case("c") || key.eq_ignore_ascii_case("x")) {
            if self.root.legacy.focus.is_none() {
                if let Some(text) = self.services.renderer.selected_text() {
                    self.services.platform.push(PlatformAction::Copy(text));
                }
                return;
            }
            if let Some(e) = self.editor() {
                let copy = e.selected().to_owned();
                let changed = key.eq_ignore_ascii_case("x") && e.replace("");
                self.services.platform.push(PlatformAction::Copy(copy));
                if changed { self.edited(); }
                self.ui.dirty = true;
            }
            return;
        }
        if key == "Enter" && !shift && self.root.legacy.focus == Some(None) {
            self.activate(Action::Send);
            return;
        }
        let Some((editor, renderer)) = self.editor_and_renderer() else { return; };
        let changed = editor.key(&mut renderer.text, renderer.faces.prose[0], key, ctrl, shift);
        if changed { self.edited(); }
        self.ui.dirty = true;
    }
    fn activate(&mut self, action: Action) {
        let result = self.apply(action);
        self.sync_navigation();
        self.report(result);
    }
    fn apply(&mut self, action: Action) -> Result<()> {
        self.cancel_preedit();
        if let Action::MoveMenu(ref id) = action { self.move_menu(id); return Ok(()); }
        if matches!(action, Action::ContextBack) {
            self.root.legacy.context_menu = self.root.legacy.context_menu.take().and_then(|m| m.parent.map(|p| *p));
            return Ok(());
        }
        if matches!(action, Action::Noop) { return Ok(()); }
        self.root.legacy.context_menu = None;
        if matches!(action, Action::Select(_) | Action::SelectProject(_) | Action::New | Action::Tail | Action::Attachments) {
            self.root.legacy.navigation.download = None;
        }
        let selected = self.controller.account.selected.clone();
        match action {
            Action::SelectProject(id) => self.navigate_project(&id)?,
            Action::NewProject | Action::RenameProject(_) | Action::ProjectPrompt(_) | Action::DeleteProject(_) | Action::RemoveProject(_) => self.project_action(action)?,
            Action::MoveChat(session_id, project_id) => {
                self.controller.request(ClientCommand::MoveSession { session_id, project_id })?;
            }
            Action::MoveMenu(_) | Action::ContextBack | Action::Noop => {}
            Action::Select(id) => self.navigate_chat(&id)?,
            Action::Info(target) => {
                self.root.legacy.usage.dismiss();
                if !self.root.legacy.info_target.same_anchor(&target) { self.root.legacy.info_tip = Tooltip::default(); }
                self.root.legacy.info_target = target;
                if let Some((rect, _)) = self
                    .root.legacy.info_areas
                    .iter()
                    .find(|(_, target)| *target == self.root.legacy.info_target)
                {
                    self.root.legacy.info_tip.region = *rect;
                }
                self.root.legacy.info_tip.pinned = !self.root.legacy.info_tip.pinned;
                self.root.legacy.info_tip.suppressed = !self.root.legacy.info_tip.pinned;
            }
            Action::Usage => {
                self.root.legacy.info_tip.dismiss();
                self.root.legacy.usage.pinned = !self.root.legacy.usage.pinned;
                self.root.legacy.usage.suppressed = !self.root.legacy.usage.pinned;
            }
            Action::New => {
                self.save()?;
                self.controller.new_chat()?;
                self.root.legacy.show_chats = false;
                self.root.legacy.focus = Some(None);
            }
            Action::RetryCreate => self.controller.retry_create_manually()?,
            Action::Back => self.back(),
            Action::Files | Action::FileOpen(..) | Action::FileUp | Action::FileClose | Action::FileFind | Action::FileFindHere | Action::FileClear | Action::FileCopy | Action::FilePage(_) => self.code_action(action)?,
            Action::Attachments => {
                self.close_code();
                self.save()?;
                self.cancel_pointer();
                self.root.legacy.focus = None;
                self.root.legacy.show_attachments = !self.root.legacy.show_attachments;
                self.root.legacy.show_chats = false;
                self.root.legacy.history_attempt = None;
            }
            Action::History => {
                self.root.legacy.history_attempt = None;
                if let Some(session) = selected { self.history_near_edge(&session, true); }
            }
            Action::ModelSettings => {
                self.controller.notice = None;
                self.root.legacy.modal = Some(Modal {
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
                self.root.legacy.focus = None;
                if let Some(id) = selected
                    && self.controller.epoch.is_some()
                    && !self.controller.chats[&id].commands_loaded
                {
                    self.controller
                        .request(ClientCommand::GetCommands { session_id: id })?;
                }
            }
            Action::ResetModels => {
                if let Some(modal) = &mut self.root.legacy.modal
                    && matches!(modal.kind, ModalKind::Models)
                {
                    modal.fields[0].1 = Editor::new(crate::models::Preferences::default().text());
                }
            }
            Action::ToggleQuickModel(slug) => {
                if let Some(modal) = &mut self.root.legacy.modal
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
            Action::CopyRecoveredDraft(id)=>{self.controller.copy_missing_draft(&id)?;self.root.legacy.composer.value=self.controller.selected().map(|c|c.local.draft.clone()).unwrap_or_default();}
            Action::ForgetRecovered(id)=>{self.root.legacy.modal=Some(Modal {kind:ModalKind::ForgetRecovered(id),title:"Forget this local chat, its drafts and files? This does not undo or cancel source work. Saved daemon actions remain in Settings.".into(),fields:vec![],options:vec![("Forget local chat".into(),Action::Confirm),("Keep".into(),Action::CancelModal)]});self.root.legacy.focus=None;}
            Action::ReviewRestore(id)=>{
                self.root.legacy.modal=Some(Modal {kind:ModalKind::ReviewRestore(id),title:"Restored history may omit external effects or paid work. Inspect those outcomes first. This acknowledgment only permits future explicit execution; it does not resume or resend anything.".into(),fields:vec![],options:vec![("Allow future explicit execution".into(),Action::Confirm),("Keep execution blocked".into(),Action::CancelModal)]});self.root.legacy.focus=None;
            }
            Action::ClearReplica=>{self.controller.clear_replica()?;self.root.legacy.modal=None;self.controller.notice=Some("Replica cache cleared. Drafts, attachments and saved intents were preserved.".into());}
            Action::Outbox(page) => {
                let mut options=self.controller.account.pending_controls.iter().skip(page*5).take(5).map(|(id,saved)| {
                    let kind=serde_json::to_value(&saved.request.command).ok().and_then(|v|v["type"].as_str().map(str::to_owned)).unwrap_or_default();
                    (format!("{kind} · {}",if saved.blocked {"needs reconciliation"} else if saved.accepted {"accepted"} else {"unconfirmed"}),Action::InspectControl(id.clone()))
                }).collect::<Vec<_>>();
                if page>0 {options.push(("Previous".into(),Action::Outbox(page-1)));}
                if (page+1)*5<self.controller.account.pending_controls.len() {options.push(("Next".into(),Action::Outbox(page+1)));}
                options.push(("Close".into(),Action::CancelModal));
                self.root.legacy.modal=Some(Modal {kind:ModalKind::Outbox,title:"Saved immutable actions".into(),fields:vec![],options});self.root.legacy.focus=None;
            }
            Action::InspectControl(id) => {
                let saved=self.controller.account.pending_controls.get(&id).ok_or_else(||anyhow::anyhow!("Action already reconciled"))?;
                let text=serde_json::to_string_pretty(&saved.request)?;
                self.root.legacy.modal=Some(Modal {kind:ModalKind::Outbox,title:format!("Action {id}"),fields:vec![],options:vec![
                    ("Copy complete saved intent".into(),Action::Copy(text)),("Check daemon receipt (no execution)".into(),Action::CheckControl(id.clone())),
                    ("Explicitly retry original ID".into(),Action::RetryControl(id.clone())),("Forget local intent…".into(),Action::ForgetControl(id)),("Back".into(),Action::Outbox(0))]});self.root.legacy.focus=None;
            }
            Action::CheckControl(id) => {self.controller.check_control(&id)?;self.root.legacy.modal=None;self.controller.notice=Some("Checking the original operation; nothing is being reexecuted".into());}
            Action::RetryControl(id) => {self.controller.retry_control(&id)?;self.root.legacy.modal=None;self.controller.notice=Some("Submitted the original immutable ID; uncertain effects are not automatically repeated".into());}
            Action::ForgetControl(id) => {
                self.root.legacy.modal=Some(Modal {kind:ModalKind::ForgetControl(id),title:"Forget this saved intent? This does NOT undo or cancel a daemon action.".into(),fields:vec![],options:vec![("Forget locally".into(),Action::Confirm),("Keep".into(),Action::Outbox(0))]});self.root.legacy.focus=None;
            }
            Action::Settings => {
                self.controller.notice = None;
                self.root.legacy.connecting = false;
                self.root.legacy.modal = Some(Modal {
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
                        (format!("Saved actions ({})",self.controller.account.pending_controls.len()),Action::Outbox(0)),
                        ("Copy connection diagnostics".into(),Action::CopyDiagnostics),
                        ("Clear replica cache (keep local work)".into(),Action::ClearReplica),
                        ("Cancel".into(), Action::CancelModal),
                    ],
                });
                self.root.legacy.focus = Some(Some(0));
            }
            Action::Focus(field) => {
                self.root.legacy.focus = Some(field);
                if self.ui.mobile {
                    let (title, value, secret, single_line) = match field {
                        Some(code_view::SEARCH_FIELD) if self.root.legacy.modal.is_none() => ("Find remote path".into(), self.root.legacy.code.as_ref().unwrap().search.as_ref().unwrap().value.clone(), false, true),
                        Some(i) => {
                            let (label, e, secret) = &self.root.legacy.modal.as_ref().unwrap().fields[i];
                            (label.clone(), e.value.clone(), *secret, e.single_line)
                        }
                        None => (
                            "Message Tau".into(),
                            self.root.legacy.composer.value.clone(),
                            false,
                            false,
                        ),
                    };
                    self.services.platform.push(PlatformAction::Edit {
                        title,
                        value,
                        secret,
                        single_line,
                    });
                }
            }
            Action::Confirm => {
                let Some(modal) = self.root.legacy.modal.as_ref() else {
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
                        if self.root.legacy.connecting && self.controller.connection == "Connecting…" {
                            return Ok(());
                        }
                        self.controller.configure(Settings {
                            server_url: values[0].clone(),
                            token: values[1].clone(),
                        })?;
                        self.root.legacy.connecting = true;
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
                        if self.root.legacy.saving_settings.is_some() {
                            return Ok(());
                        }
                        self.apply_setting_field()?;
                        let draft = self
                            .root.legacy.daemon_draft
                            .as_ref()
                            .ok_or_else(|| anyhow::anyhow!("Load settings first"))?;
                        anyhow::ensure!(
                            draft.identity == self.controller.identity,
                            "Server changed; reload settings first"
                        );
                        self.controller.notice = None;
                        self.controller.settings_result = None;
                        self.root.legacy.saving_settings =
                            Some(self.controller.request(ClientCommand::SetSettings {
                                revision: draft.revision,
                                settings: Box::new(draft.document()?),
                            })?);
                        self.root.legacy.focus = None;
                        return Ok(());
                    }
                    ModalKind::RefreshCatalog => {
                        let provider = values[0].trim();
                        anyhow::ensure!(!provider.is_empty() && provider.len() <= 120 && !provider.chars().any(char::is_whitespace),
                            "Enter a configured provider name");
                        self.controller.notice = Some(format!("Refreshing {provider} model catalog…").into());
                        self.controller.request(ClientCommand::RefreshModelCatalog { provider: provider.into() })?;
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
                    ModalKind::ConfirmLink(url) => self.services.platform.push(PlatformAction::OpenUrl(url)),
                    ModalKind::Outbox=>{},
                    ModalKind::ForgetControl(id)=>self.controller.forget_control(&id)?,
                    ModalKind::ForgetRecovered(id)=>{self.controller.forget_missing_chat(&id)?;}
                    ModalKind::ReviewRestore(id)=>{self.controller.request(ClientCommand::ReviewRestore {session_id:id})?;}
                }
                self.root.legacy.modal = None;
                self.root.legacy.focus = None;
            }
            Action::CancelModal => {
                self.root.legacy.connecting = false;
                self.root.legacy.saving_project = None;
                self.root.legacy.waiting_settings = false;
                self.root.legacy.daemon_draft = None;
                self.root.legacy.saving_settings = None;
                self.root.legacy.modal = None;
                self.root.legacy.focus = None;
            }
            Action::RefreshCatalog => {
                let provider = self.controller.daemon_settings.as_ref().map(|settings| settings.agent.model.provider.clone())
                    .or_else(|| selected.as_ref().and_then(|id| self.controller.account.sessions.iter().find(|s| &s.id == id))
                        .and_then(|s| s.model.as_ref().map(|m| m.provider.clone())))
                    .unwrap_or_else(|| "openai-codex".into());
                self.root.legacy.modal = Some(Modal { kind: ModalKind::RefreshCatalog, title: "Refresh model catalog".into(),
                    fields: vec![("Provider name".into(), Editor::line(provider), false)],
                    options: vec![("Refresh".into(), Action::Confirm), ("Cancel".into(), Action::CancelModal)] });
                self.root.legacy.focus = Some(Some(0));
            }
            Action::DaemonSettings => {
                self.controller.notice = None;
                self.controller.daemon_settings = None;
                self.controller.request(ClientCommand::GetSettings)?;
                self.root.legacy.waiting_settings = true;
                self.root.legacy.daemon_draft = None;
                self.root.legacy.saving_settings = None;
                self.root.legacy.modal = Some(Modal {
                    kind: ModalKind::Daemon,
                    title: "Daemon settings".into(),
                    fields: vec![],
                    options: vec![],
                });
                self.root.legacy.focus = None;
            }
            Action::SettingsSection(section) => {
                self.apply_setting_field()?;
                if let Some(draft) = &mut self.root.legacy.daemon_draft {
                    draft.section = section;
                    draft.field = 0;
                }
                self.load_setting_field()?;
            }
            Action::SettingsField(next) => {
                self.apply_setting_field()?;
                if let Some(draft) = &mut self.root.legacy.daemon_draft {
                    let count = crate::daemon_settings::fields(draft.section).len();
                    draft.field = (draft.field + if next { 1 } else { count - 1 }) % count;
                }
                self.load_setting_field()?;
            }
            Action::SettingToggle => {
                if let (Some(draft), Some(modal)) = (&mut self.root.legacy.daemon_draft, &mut self.root.legacy.modal) {
                    if draft.definition().kind == crate::daemon_settings::Kind::PromptOverride {
                        draft.inherit = !draft.inherit;
                        if draft.inherit {
                            modal.fields[0].1 = Editor::new(draft.default_prompt().into());
                        }
                    } else {
                        modal.fields[0].1 =
                            Editor::line((modal.fields[0].1.value != "true").to_string());
                    }
                    self.root.legacy.focus = None;
                }
            }
            Action::SettingReset => {
                if let Some(draft) = &mut self.root.legacy.daemon_draft { draft.reset()?; }
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
                        self.controller.account.sessions.iter().find(|s| s.id == session)
                            .and_then(|s| s.thinking_level.clone()).unwrap_or_default(),
                    ),
                    "fast" => ("on / off / status", String::new()),
                    _ => ("Optional compaction instructions", String::new()),
                };
                self.root.legacy.modal = Some(Modal {
                    title: format!("Chat {command}"),
                    kind: ModalKind::AgentCommand(session, command),
                    fields: vec![(label.into(), Editor::line(value), false)],
                    options: vec![
                        ("Apply".into(), Action::Confirm),
                        ("Cancel".into(), Action::CancelModal),
                    ],
                });
                self.root.legacy.focus = Some(Some(0));
            }
            Action::AgentCommand(session, text) => {
                self.controller.ensure_chat(&session)?;
                self.controller.control(ClientCommand::Prompt {
                    session_id: session,
                    text,
                })?;
                self.root.legacy.modal = None;
                self.root.legacy.focus = None;
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
                self.root.legacy.modal = Some(Modal {
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
                self.root.legacy.modal = Some(Modal {
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
            Action::Send => {
                if let Some(code)=&self.root.legacy.code {anyhow::ensure!(code.selection.is_some() && code.error.is_none(), "Select current lines again before sending this code comment");}
                if self.root.legacy.code.is_some() {self.code_reference(false);}
                self.controller.send_prompt()?;
                if let Some(code)=&mut self.root.legacy.code { code.sent(); self.root.legacy.focus=None; }
            },
            Action::Abort => {
                if let Some(id) = selected {
                    self.controller
                        .control(ClientCommand::Abort { session_id: id })?;
                }
            }
            Action::Tail => {
                self.root.legacy.scroll = self.root.legacy.max_scroll;
                self.remember_scroll();
            }
            Action::DismissNotice => self.controller.notice = None,
            Action::OpenDownloadNotice(target) => {
                self.controller.notice = None;
                self.open_download_notice(target)?;
            }
            Action::CopyDiagnostics => {self.services.platform.push(PlatformAction::Copy(self.controller.diagnostics()));}
            Action::Copy(text) => {self.controller.cancel_copy();self.services.platform.push(PlatformAction::Copy(text));}
            Action::CopyDetails(session, ids) => self.controller.copy_details(&session,ids)?,
            Action::CopySelection => {
                self.controller.cancel_copy();
                if let Some(text) = self.services.renderer.selected_text() {
                    self.services.platform.push(PlatformAction::Copy(text));
                }
            }
            Action::Link(url) => {
                let parsed = url::Url::parse(&url)?;
                anyhow::ensure!(
                    matches!(parsed.scheme(), "https" | "http" | "mailto"),
                    "Only web and mail links can be opened"
                );
                self.root.legacy.modal = Some(Modal {
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
                    self.services.platform.push(PlatformAction::PickFile {
                        identity: self.controller.identity.clone(),
                        session,
                    });
                }
            }
            Action::RemoveFile(id) => self.controller.remove_file(&id)?,
            Action::Toggle(key, expanded) => {
                self.root.legacy.wheel = None;
                self.root.legacy.velocity = 0.;
                self.root.legacy.expansion_pin = self
                    .root.legacy.expansion_positions
                    .get(&key)
                    .map(|y| (key.clone(), *y - self.root.legacy.scroll));
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
                self.root.legacy.composer =
                    Editor::composer(self.controller.selected().unwrap().local.draft.clone());
            }
            Action::RetryPending(id) => self.controller.retry_pending(&id)?,
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
                self.root.legacy.modal = Some(Modal {
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
                let key=Controller::download_key(&session,&entry);
                let path = match self.controller.download(&session,&entry,if image {10_000_000} else {50_000_000}) {
                    Ok(path)=>path,
                    Err(error)=>{let text=error.to_string();self.root.legacy.export_errors.insert(key.clone(),text.clone());
                        self.controller.notice=Some(text.into());return Ok(());}
                };
                let in_progress = self.controller.downloads.get(&key).is_some_and(|d| !d.status.done);
                if image {
                    if path.is_file() && !in_progress {
                        self.root.legacy.viewer_image = None;
                        self.root.legacy.viewer = Some(Viewer {
                            path,
                            name,
                            session,
                            entry,
                            zoom: 1.,
                            pan: Vec2::new(0., 0.),
                        });
                    }
                } else if path.is_file() && !in_progress {
                    self.begin_save(&session,&entry,path,name);
                } else {
                    self.root.legacy.export_targets.insert(key.clone(), self.export_target(&session,&entry));
                    self.root.legacy.export_errors.remove(&key);
                    self.root.legacy.pending_exports.insert(key, (path, name));
                }
            }
            Action::SaveAttachment(session,entry,name) => {
                let key=Controller::download_key(&session,&entry);
                let file = self.controller.chats.get(&session).is_some_and(|chat| chat.feed.events.values()
                    .any(|e| e.entry_id == entry && e.attachment.as_ref().is_some_and(|a| a.kind == AttachmentKind::File)));
                let path=match self.controller.download(&session,&entry,if file {50_000_000} else {10_000_000}) {
                    Ok(path)=>path,
                    Err(error)=>{let text=error.to_string();self.root.legacy.export_errors.insert(key.clone(),text.clone());
                        self.controller.notice=Some(text.into());return Ok(());}
                };
                if path.is_file() && !self.controller.downloads.get(&key).is_some_and(|d|!d.status.done) {
                    self.begin_save(&session,&entry,path,name);
                } else {
                    self.root.legacy.export_targets.insert(key.clone(),self.export_target(&session,&entry));
                    self.root.legacy.export_errors.remove(&key);
                    self.root.legacy.pending_exports.insert(key,(path,name));
                }
            }
            Action::UseSaved(session,entry,action) => {
                if let Some(saved)=self.controller.saved_download(&session,&entry) {
                    self.services.platform.push(PlatformAction::UseDownload(saved,action,self.export_target(&session,&entry)));
                }
            }
            Action::CancelDownload(key) => {
                self.root.legacy.pending_exports.remove(&key);
                self.root.legacy.export_targets.remove(&key);
                self.controller.cancel_download(&key)?;
            },
            Action::Zoom(factor) => {
                if let Some(v) = &mut self.root.legacy.viewer {
                    v.zoom = (v.zoom * factor).clamp(1., 16.);
                }
            }
            Action::Fit => {
                if let Some(v) = &mut self.root.legacy.viewer {
                    v.zoom = 1.;
                    v.pan = Vec2::new(0., 0.);
                }
            }
            Action::Suggest(text) => {
                self.root.legacy.composer = Editor::composer(text.clone());
                self.controller.draft(text)?;
            }
        }
        self.sync_navigation();
        Ok(())
    }
    pub fn frame(&mut self, ctx: &impl RenderContext, view: &wgpu::TextureView) {
        let s = self.ui.scale;
        let bounds = Rect::new(
            self.ui.origin.x,
            self.ui.origin.y,
            self.ui.size.0 as f32,
            self.ui.size.1 as f32,
        );
        let input = Interaction {
            hover: self.root.legacy.pointer.as_ref().map(|p| p.last).or(self.root.legacy.hover),
            pressed: self
                .root.legacy.pointer
                .as_ref()
                .filter(|p| !p.dragged)
                .map(|p| p.start),
            held: self.root.legacy.pointer.is_some(),
        };
        let background_input =
            if self.root.legacy.modal.is_none() && self.root.legacy.viewer.is_none() && self.root.legacy.context_menu.is_none() {
                input
            } else {
                Interaction::default()
            };
        let mut main = Layer::new(background_input);
        let mut body = Layer::new(background_input);
        let mut chrome = Layer::new(background_input);
        let mut overlay = Layer::new(input);
        self.root.legacy.hits.clear();
        self.root.legacy.scrollbars.clear();
        self.root.legacy.message_areas.clear();
        self.root.legacy.detail_areas.clear();
        self.root.legacy.chat_areas.clear();
        self.root.legacy.project_areas.clear();
        self.root.legacy.projects_rect = Rect::new(0.,0.,0.,0.);
        self.root.legacy.list_rect = Rect::new(0.,0.,0.,0.);
        self.root.legacy.transcript = Rect::new(0.,0.,0.,0.);
        self.root.legacy.attachments_rect = Rect::new(0., 0., 0., 0.);
        self.root.legacy.info_areas.clear();
        self.root.legacy.usage.region = Rect::new(0., 0., 0., 0.);
        self.root.legacy.info_tip.region = Rect::new(0., 0., 0., 0.);
        self.root.legacy.composer.hide();
        if let Some(modal) = &mut self.root.legacy.modal {
            for (_, editor, _) in &mut modal.fields { editor.hide(); }
        }
        self.services.renderer.clear_scenes();
        self.root.legacy.viewer_image = None;
        main.rect(bounds, color(0x0e141b));
        let wide = bounds.width / s >= 760.;
        let side = if wide { 300. * s } else { 0. };
        let file_side = self.root.legacy.show_attachments && !self.ui.mobile && bounds.width / s >= 1000.;
        let file_screen = self.root.legacy.show_attachments && !file_side;
        let file_width = if file_side { 320. * s } else { 0. };
        let mut interests = std::collections::BTreeSet::new();
        if !file_screen && (wide || self.root.legacy.show_chats) {
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
        if !file_screen && (wide || !self.root.legacy.show_chats) {
            self.chat(
                ctx,
                &mut body,
                &mut chrome,
                Rect::new(
                    bounds.x + side,
                    bounds.y,
                    bounds.width - side - file_width,
                    bounds.height,
                ),
                &mut interests,
            );
        }
        if self.root.legacy.show_attachments && let Some(session) = self.controller.account.selected.clone()
            && let Some(chat) = self.controller.chats.get(&session)
        {
            let files = chat.feed.events.values().rev().filter_map(|event|
                event.attachment.clone().map(|file| (event.id.clone(), event.entry_id.clone(), file)))
                .collect::<Vec<_>>();
            let older = chat.feed.before.is_some();
            let loading = chat.feed.loading;
            let synchronized = chat.feed.synchronized;
            let b = if file_side {
                Rect::new(bounds.x + bounds.width - file_width, bounds.y, file_width, bounds.height)
            } else { bounds };
            main.rect(b, color(0x0e141b));
            if file_side { main.rect(Rect::new(b.x, b.y, s, b.height), color(0x2a3541)); }
            chrome.rect(Rect::new(b.x, b.y + 56. * s, b.width, s), color(0x2a3541));
            let title_x = b.x + if file_screen { 80. * s } else { 14. * s };
            let title_width = b.width - if file_screen { 94. * s } else { 70. * s };
            self.services.renderer.label(&mut chrome, "Attachments",
                Rect::new(title_x, b.y + 8. * s, title_width, 22. * s), 16. * s, color(0xe5eaf0), true);
            self.services.renderer.label(&mut chrome, &format!("{} loaded · newest first", files.len()),
                Rect::new(title_x, b.y + 30. * s, title_width, 18. * s), 12. * s, color(0x82909f), false);
            button(&mut self.services.renderer, &mut chrome, &mut self.root.legacy.hits,
                Rect::new(if file_screen { b.x + 8. * s } else { b.x + b.width - 48. * s }, b.y + 8. * s,
                    if file_screen { 64. * s } else { 40. * s }, 40. * s),
                if file_screen { "Back" } else { "×" },
                if file_screen { Action::Back } else { Action::Attachments }, s, false);
            let viewport = Rect::new(b.x + s, b.y + 57. * s, b.width - s, (b.height - 57. * s).max(0.));
            self.root.legacy.attachments_rect = viewport;
            let heights = files.iter().map(|(_, _, file)|
                attachments::card_height(file) * s).collect::<Vec<_>>();
            let content_height = 12. * s + heights.iter().map(|h| h + 12. * s).sum::<f32>();
            self.root.legacy.max_attachment_scroll = (content_height + if older { 52. * s } else { 0. } - viewport.height).max(0.);
            self.root.legacy.attachment_scroll = self.root.legacy.attachment_scroll.clamp(0., self.root.legacy.max_attachment_scroll);
            let mut y = viewport.y + 12. * s - self.root.legacy.attachment_scroll;
            for ((id, entry, file), height) in files.iter().zip(heights) {
                let rect = Rect::new(b.x + 12. * s, y, b.width - 28. * s, height);
                if y + height >= viewport.y - viewport.height && y <= viewport.y + 2. * viewport.height {
                    interests.insert(id.clone());
                }
                if y + height >= viewport.y && y <= viewport.y + viewport.height {
                    body.clipped_corners(rect, [12. * s; 4], color(0x18212b), viewport);
                    self.attachment_card(ctx, &mut body, &session, entry, file, "attachments", rect, viewport);
                }
                y += height + 12. * s;
            }
            if files.is_empty() {
                let text = if self.controller.epoch.is_none() {
                    "No cached attachments.\nConnect to load sent files."
                } else if older || !synchronized {
                    "Loading attachments…"
                } else { "No attachments yet.\nFiles sent in this chat appear here." };
                self.services.renderer.label(&mut body, text,
                    Rect::new(b.x + 24. * s, viewport.y + 80. * s, b.width - 48. * s, 100. * s),
                    14. * s, color(0xb7c2ce), false);
            }
            if older {
                let r = crate::render::intersect(Rect::new(b.x + 16. * s, y, b.width - 44. * s, 36. * s), viewport);
                if r.height > 0. {
                    if loading || !synchronized || self.controller.epoch.is_none() {
                        self.services.renderer.clipped_label(&mut body,
                            if self.controller.epoch.is_none() { "Connect to load older files" } else { "Loading older files…" },
                            Rect::new(b.x + 16. * s, y + 8. * s, b.width - 44. * s, 24. * s),
                            12. * s, color(0x82909f), false, viewport);
                    } else {
                        button(&mut self.services.renderer, &mut body, &mut self.root.legacy.hits, r,
                            "Load older files", Action::History, s, false);
                    }
                }
                self.history_near_edge(&session, self.root.legacy.max_attachment_scroll - self.root.legacy.attachment_scroll <= 2. * viewport.height);
            }
            self.scrollbar(&mut chrome, Lane::Attachments, viewport);
        }
        if (wide || !self.root.legacy.show_chats || self.root.legacy.show_attachments)
            && let Some(session) = self.controller.account.selected.clone()
        {
            self.controller.viewport(&session, interests);
        }
        if let Some((rect, target)) = self.root.legacy.info_areas.iter().find(|(_, target)| self.root.legacy.info_target.same_anchor(target)) {
            self.root.legacy.info_tip.region = *rect;
            self.root.legacy.info_target = target.clone();
        }
        if self.root.legacy.info_tip.region.width <= 0. {
            self.root.legacy.info_tip.hover(false);
            self.root.legacy.info_tip.dismiss();
        }
        self.usage_frame(&mut overlay, bounds);
        self.info_frame(&mut overlay, bounds);
        self.context_frame(&mut overlay, bounds);
        if let Some(auto) = &self.root.legacy.autoscroll {
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
            self.services.renderer.icon(
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
        if let Some(viewer) = &self.root.legacy.viewer {
            self.root.legacy.hits.clear();
            overlay.rect(bounds, color(0x06090d));
            let path = viewer.path.clone();
            let name = viewer.name.clone();
            let zoom = viewer.zoom;
            let pan = viewer.pan;
            match self.services.renderer.image_size(ctx, &path) {
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
                    self.root.legacy.viewer_image = Some(crate::render::intersect(image, clip));
                    overlay.images.push((path.clone(), image, clip));
                }
                Err(e) => {
                    self.services.renderer.label(
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
                ("↓", Action::SaveAttachment(viewer.session.clone(), viewer.entry.clone(), name)),
            ];
            for (i, (label, action)) in buttons.into_iter().enumerate() {
                button(
                    &mut self.services.renderer,
                    &mut overlay,
                    &mut self.root.legacy.hits,
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
            self.root.legacy.modal.as_ref().map(|m| &m.kind),
            Some(ModalKind::Settings)
        ) {
            self.settings_frame(&mut overlay, bounds);
        } else if matches!(
            self.root.legacy.modal.as_ref().map(|m| &m.kind),
            Some(ModalKind::Models)
        ) {
            self.model_settings_frame(&mut overlay, bounds);
        } else if matches!(
            self.root.legacy.modal.as_ref().map(|m| &m.kind),
            Some(ModalKind::Daemon)
        ) {
            self.daemon_settings_frame(&mut overlay, bounds);
        } else if self.root.legacy.modal.as_ref().is_some_and(|m| projects::is_project_modal(&m.kind)) {
            self.project_modal_frame(&mut overlay, bounds);
        } else if self.root.legacy.modal.is_some() {
            self.modal_frame(&mut overlay, bounds);
        }
        if !matches!(
            self.root.legacy.modal.as_ref().map(|m| &m.kind),
            Some(ModalKind::Settings | ModalKind::Models | ModalKind::Daemon)
        ) && !self.root.legacy.modal.as_ref().is_some_and(|m| projects::is_project_modal(&m.kind)) {
            self.notice_frame(ctx, &mut overlay, bounds);
        }
        if self.root.legacy.selecting
            && let Some(p) = &self.root.legacy.pointer
            && p.dragged
            && let Some(caret) = self.services.renderer.nearest_text(p.last)
        {
            self.ui.dirty |= self.services.renderer.extend_selection(caret);
        }
        self.services.renderer
            .draw(ctx, view, &[main, body, chrome, overlay]);
    }
    fn sidebar(&mut self, ctx: &impl RenderContext, layer: &mut Layer, b: Rect) {
        let s = self.ui.scale;
        layer.rect(b, color(0x0e141b));
        layer.rect(
            Rect::new(b.x + b.width - s, b.y, s, b.height),
            color(0x2a3541),
        );
        let indicator = Rect::new(b.x + 72. * s, b.y + 22. * s, 28. * s, 34. * s);
        self.root.legacy.info_areas.push((indicator, Info::Connection));
        layer.rounded_rect(
            indicator,
            8. * s,
            layer.control_color(indicator, color(0x0e141b)),
        );
        layer.rounded_rect(
            Rect::new(b.x + 82. * s, b.y + 35. * s, 8. * s, 8. * s),
            4. * s,
            color(self.controller.health.color(Instant::now())),
        );
        self.root.legacy.hits.push(Hit {
            rect: indicator,
            action: Action::Info(Info::Connection),
        });
        self.services.renderer.label(
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
            &mut self.services.renderer,
            layer,
            &mut self.root.legacy.hits,
            Rect::new(b.x + 16. * s, b.y + 84. * s, b.width - 32. * s, 40. * s),
            "New chat",
            Action::New,
            s,
            true,
        );
        // Reserve the sidebar's rightmost column for its separator. The
        // scrolling tabs (including the clipped add tab) must not paint over it.
        self.project_tabs(layer, Rect::new(b.x, b.y + 140. * s, b.width - s, 34. * s));
        let clip = Rect::new(b.x, b.y + 182. * s, b.width, (b.height - 190. * s).max(0.));
        self.root.legacy.list_rect = clip;
        let sessions = self.controller.account.sessions.iter().filter(|c| c.project_id == self.controller.account.selected_project);
        self.root.legacy.max_list_scroll =
            (sessions.clone().count() as f32 * 90. * s - clip.height).max(0.);
        self.root.legacy.list_scroll = self.root.legacy.list_scroll.min(self.root.legacy.max_list_scroll);
        for (i, session) in sessions.enumerate() {
            let y = clip.y + i as f32 * 90. * s - self.root.legacy.list_scroll;
            let rect = Rect::new(b.x + 8. * s, y, b.width - 16. * s, 84. * s);
            if y + rect.height < clip.y || y > clip.y + clip.height {
                continue;
            }
            let selected = self.controller.account.selected.as_ref() == Some(&session.id);
            let targeted = self
                .root.legacy.context_menu
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
            self.root.legacy.chat_areas.push((rect, session.id.clone()));
            let unread = self.controller.unread(session);
            let title = if session.starter {
                "New chat"
            } else if session.title.is_empty() {
                "Unnamed chat"
            } else {
                &session.title
            };
            self.services.renderer.clipped_label(
                layer,
                title,
                Rect::new(rect.x + 12. * s, y + 10. * s, rect.width - 56. * s, 22. * s),
                16. * s,
                color(0xe5eaf0),
                unread || selected,
                clip,
            );
            if let Some(model) = &session.model {
                self.services.renderer.clipped_label(
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
                if self.controller.is_creating(&session.id) { "Creating…" }
                else if self.controller.chats.get(&session.id).is_some_and(|c| c.feed.queue.paused) { "Paused" }
                else { match session.status {
                    SessionStatus::Running => "Working",
                    SessionStatus::Error => "Error",
                    SessionStatus::Idle => "Ready",
                    SessionStatus::Sleeping => "Sleeping",
                }}
            );
            self.services.renderer.clipped_label(
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
            self.root.legacy.hits.push(Hit {
                rect,
                action: Action::Select(session.id.clone()),
            });
            let ring = Rect::new(rect.x + rect.width - 33. * s, y + 11. * s, 18. * s, 18. * s);
            let (ratio, tint) = self.controller.cache_ttl(session).meter();
            self.services.renderer
                .clipped_icon(ctx, layer, Icon::CacheTtl(ratio), ring, tint, clip);
            let target = crate::render::intersect(
                Rect::new(ring.x - 7. * s, ring.y - 7. * s, 32. * s, 32. * s),
                clip,
            );
            if target.height > 0. {
                let info = Info::CacheTtl(session.id.clone());
                self.root.legacy.info_areas.push((target, info.clone()));
                self.root.legacy.hits.push(Hit {
                    rect: target,
                    action: Action::Info(info),
                });
            }
        }
        if self.root.legacy.max_list_scroll == 0. && !self.controller.account.sessions.iter().any(|c| c.project_id == self.controller.account.selected_project) {
            self.services.renderer.clipped_label(layer, "No chats in this topic yet", Rect::new(b.x + 20. * s, clip.y + 20. * s, b.width - 40. * s, 40. * s), 13. * s, color(0x82909f), false, clip);
        }
        self.scrollbar(layer, Lane::Sidebar, clip);
    }
    fn rows(&self, session: &str) -> Vec<Row> {
        let chat = &self.controller.chats[session];
        let tools = Tools::new(chat.feed.events.values()).with_lengths(&chat.feed.block_lengths).with_states(&chat.feed.block_states);
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
        let user_requests = events.iter().filter(|e| e.role == EventRole::User && e.kind == EventKind::Text && e.attachment.is_none())
            .filter_map(|e| e.origin.request_id.as_deref()).collect::<std::collections::HashSet<_>>();
        let local_prompts = chat.local.pending.iter().filter(|p| matches!(p.request.command,ClientCommand::Prompt { .. }) && p.status != crate::store::Delivery::Rejected)
            .map(|p| (p.request.id.as_str(),p)).collect::<std::collections::HashMap<_,_>>();
        let mut shown_requests = std::collections::HashSet::new();
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
                    block: None,
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
            // The server owns ordering/identity; until its body is complete the
            // authored copy still owns the displayed text. Never turn it into
            // a Loading row or duplicate it at the end of the conversation.
            let local_text = (user && e.kind == EventKind::Text && e.attachment.is_none() && chat.feed.incomplete.contains(&e.id))
                .then(|| e.origin.request_id.as_deref().and_then(|id| local_prompts.get(id)))
                .flatten().map(|p| p.text.as_str());
            let title = if user {
                if local_text.is_some() { "You · synchronizing" } else { "You" }.into()
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
            let mut actions = if let Some(text) = local_text {vec![("Copy text".into(), Action::Copy(text.into()))]}
                else if chat.feed.incomplete.contains(&e.id) {vec![("Fetch complete message to copy".into(),Action::CopyDetails(session.into(),vec![e.id.clone()]))]} else {vec![("Copy message".into(), Action::Copy(e.text.clone()))]};
            if e.phase == EventPhase::Saved {
                actions.push(("Fork here".into(), Action::Fork(e.entry_id.clone())));
            }
            let request = e.origin.request_id.as_deref().filter(|id| user && e.kind == EventKind::Text && e.attachment.is_none()
                && shown_requests.insert(*id));
            rows.push(Row {
                block: Some(e.id.clone()),
                details: vec![],
                header: e.role == EventRole::System || e.is_error || e.error_message.is_some(),
                key: request.map_or_else(|| format!("{session}/{}", e.id), |id| format!("message:{session}:{id}")),
                title,
                timestamp: clock::label(clock::event_ms(e)),
                sender: if e.role == EventRole::Tool {
                    EventRole::Assistant
                } else {
                    e.role
                },
                source: if e.attachment.is_some() && chat.feed.incomplete.contains(&e.id) && e.text == "Loading…" {
                    String::new()
                } else if user {
                    literal(local_text.unwrap_or(&e.text))
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
            let represented = user_requests.contains(p.request.id.as_str()) || chat.feed.queue.requests.iter().any(|q| q.request_id == p.request.id);
            if matches!(p.request.command, ClientCommand::Prompt { .. }) && p.status != crate::store::Delivery::Rejected && represented { continue; }
            let edit = matches!(&p.request.command, ClientCommand::QueueControl { operation: QueueOperation::Edit { .. }, .. });
            let delete = matches!(&p.request.command, ClientCommand::QueueControl { operation: QueueOperation::Delete { .. }, .. });
            let control = matches!(&p.request.command, ClientCommand::QueueControl { .. } | ClientCommand::Abort { .. });
            let in_queue = chat.feed.queue.requests.iter().any(|q| match &p.request.command {
                ClientCommand::QueueControl { operation: QueueOperation::Edit { request_id, revision, .. }
                    | QueueOperation::Delete { request_id, revision }, .. } => q.request_id == *request_id && q.revision <= revision.saturating_add(1),
                _ => false,
            });
            // The queue row owns an in-flight edit/delete. Never represent it as
            // a new user message; an unresolved control remains visible by itself
            // only if its target disappeared or it was explicitly rejected.
            if (edit || delete) && in_queue && !matches!(p.status, crate::store::Delivery::Rejected) { continue; }
            let mut actions = vec![("Copy text".into(), Action::Copy(p.text.clone()))];
            if !control && matches!(p.status, crate::store::Delivery::Rejected | crate::store::Delivery::Unconfirmed) {
                actions.push(("Restore draft".into(), Action::Restore(p.request.id.clone())));
            }
            if matches!(p.request.command, ClientCommand::Prompt { .. })
                && matches!(p.status, crate::store::Delivery::Rejected | crate::store::Delivery::Unconfirmed) {
                actions.push(("Retry saved message".into(), Action::RetryPending(p.request.id.clone())));
            }
            actions.push(("Dismiss".into(), Action::Dismiss(p.request.id.clone())));
            rows.push(Row {
                block: None,
                details: vec![], header: true, key: if matches!(p.request.command, ClientCommand::Prompt { .. }) && !represented {
                    format!("message:{session}:{}", p.request.id)
                } else { format!("pending:{}", p.request.id) },
                title: if edit { format!("Queue edit · {}", p.status.label()) }
                    else if delete { format!("Queue delete · {}", p.status.label()) }
                    else if control { format!("Queue action · {}", p.status.label()) }
                    else { p.status.label().into() },
                timestamp: clock::label(p.started_at_ms),
                sender: if control { EventRole::System } else { EventRole::User },
                source: literal(&format!("{}{}", p.text, p.detail.as_ref().map(|s| format!("\n{s}")).unwrap_or_default())),
                user: !control,
                error: matches!(p.status, crate::store::Delivery::Rejected | crate::store::Delivery::Unconfirmed),
                actions, attachment: None,
            });
        }
        for (i, q) in chat.feed.queue.requests.iter().enumerate() {
            // Root and queue directories can arrive in either order during a
            // queue -> history move. The canonical user row wins that overlap.
            if user_requests.contains(q.request_id.as_str()) { continue; }
            let state = &chat.feed.queue;
            let moving = chat.feed.queue_transitions.contains_key(&q.request_id);
            let complete=!chat.feed.incomplete.contains(&format!("queued:{}",q.request_id));
            let local_text = (!complete).then(|| local_prompts.get(q.request_id.as_str()))
                .flatten().map(|p| p.text.as_str());
            let mut actions = if complete {vec![("Copy message".into(), Action::Copy(q.text.clone()))]} else {vec![]};
            if !moving && complete && state.capabilities.iter().any(|c| c == "queue_edit") {
                actions.push((
                    "Edit".into(),
                    Action::EditQueue(q.request_id.clone(), q.revision, q.text.clone()),
                ));
            }
            if !moving && state.capabilities.iter().any(|c| c == "queue_delete") {
                actions.push((
                    "Delete".into(),
                    Action::Queue(QueueOperation::Delete {
                        request_id: q.request_id.clone(),
                        revision: q.revision,
                    }),
                ));
            }
            if state.available && chat.feed.queue_transitions.is_empty() && state.capabilities.iter().any(|c| c == "queue_run_prefix")
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
            let pending = chat.local.pending.iter().rev().find(|p| {
                let target = match &p.request.command {
                    ClientCommand::QueueControl { operation: QueueOperation::Edit { request_id, revision, .. }
                        | QueueOperation::Delete { request_id, revision }, .. } =>
                        request_id == &q.request_id && q.revision <= revision.saturating_add(1),
                    _ => false,
                };
                target && matches!(p.status, crate::store::Delivery::Sending | crate::store::Delivery::Unconfirmed | crate::store::Delivery::Accepted)
            });
            let editing = pending.and_then(|p| match &p.request.command {
                ClientCommand::QueueControl { operation: QueueOperation::Edit { text, .. }, .. } => Some(text.as_str()),
                _ => None,
            });
            if let Some(text) = local_text { actions = vec![("Copy text".into(), Action::Copy(text.into()))]; }
            if pending.is_some() {
                // A second edit/delete using the old revision would race this
                // one. Wait for the durable receipt before offering actions.
                actions = vec![("Copy message".into(), Action::Copy(editing.unwrap_or(&q.text).into()))];
            }
            rows.push(Row {
                block: None,
                details: vec![],
                header: true,
                key: format!("message:{session}:{}", q.request_id),
                title: if moving { "Synchronizing message".into() } else if let Some(p) = pending {
                    format!("{} · {}", if editing.is_some() { "Queue edit" } else { "Queue delete" },
                        if p.status == crate::store::Delivery::Unconfirmed { "unconfirmed · not resent" } else if p.status==crate::store::Delivery::Accepted {"accepted · synchronizing…"} else { "saving…" })
                } else { format!("Queued{}", if state.paused { " · held" } else { "" }) },
                timestamp: clock::label(q.timestamp_ms),
                sender: EventRole::User,
                source: literal(editing.or(local_text).unwrap_or(&q.text)),
                user: true,
                error: false,
                actions,
                attachment: None,
            });
        }
        rows
    }
    fn chat(&mut self, ctx: &impl RenderContext, layer: &mut Layer, chrome: &mut Layer, b: Rect, interests: &mut std::collections::BTreeSet<String>) {
        if self.root.legacy.code.is_some() { self.code_frame(ctx, layer, chrome, b); return; }
        let s = self.ui.scale;
        let paint_at = Instant::now();
        let wide = self.ui.size.0 as f32 / s >= 760.;
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
                &mut self.services.renderer,
                chrome,
                &mut self.root.legacy.hits,
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
        let connected = self.controller.epoch.is_some();
        let title_width =
            (b.x + b.width - if running || paused { 152. * s } else { 108. * s } - title_x).max(1.);
        self.root.legacy.chat_areas.push((
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
        self.services.renderer.label(
            chrome,
            title,
            Rect::new(title_x, b.y + 8. * s, title_width, 22. * s),
            16. * s,
            color(0xe5eaf0),
            true,
        );
        self.services.renderer.label(
            chrome,
            if self.controller.is_creating(&session) {
                if self.controller.epoch.is_none() { "Saved locally · offline" } else { "Creating…" }
            } else if self.controller.epoch.is_none() {
                "Offline"
            } else if self.controller.chats[&session].feed.queue.paused {
                "Paused · resume needed"
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
        self.icon_button(
            ctx, chrome,
            Rect::new(b.x + b.width - if running || paused { 96. * s } else { 52. * s },
                header.y + (header.height - 40. * s) / 2., 40. * s, 40. * s),
            Icon::Attachments, 22., Action::Attachments, self.root.legacy.show_attachments, true,
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
        self.icon_button(ctx, chrome,
            Rect::new(b.x + b.width - if running || paused { 140. * s } else { 96. * s },
                header.y + (header.height - 40. * s) / 2., 40. * s, 40. * s),
            Icon::Folder, 22., Action::Files, false, true);
        let (x, width, _, _, composer_top) = self.composer_layout(b, &session);
        let viewport = Rect::new(
            b.x,
            b.y + 57. * s,
            b.width,
            (composer_top - b.y - 57. * s).max(1.),
        );
        self.root.legacy.transcript = viewport;
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
        self.root.legacy.max_horizontal = 0.;
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
                        let h = self.services.renderer.message_height(
                            &key,
                            &line.source,
                            text_width - line.indent * s,
                            if line.code { 12. / 0.9 * s } else { 12. * s },
                        ) + 6. * s;
                        if let Some(m) = self.services.renderer.messages.get(&key) {
                            self.root.legacy.max_horizontal = self
                                .root.legacy.max_horizontal
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
                self.services.renderer
                    .message_height(&row.key, &row.source, text_width, 16. * s)
            };
            if let Some(message) = self.services.renderer.messages.get(&row.key) {
                self.root.legacy.max_horizontal = self.root.legacy.max_horizontal.max(message.view.width - text_width);
            }
            let attachment = if let Some((_, a)) = &row.attachment {
                attachments::card_height(a) * s
            } else {
                0.
            };
            let height = (if row.header { 50. } else { 28. }) * s
                + text_height
                + 12. * s
                + attachment;
            placements.push(Placed {
                key: row.key.clone(),
                top: y,
                height,
            });
            y += height + gap;
        }
        self.services.renderer.retain_messages(&keys);
        self.root.legacy.horizontal = self.root.legacy.horizontal.clamp(0., self.root.legacy.max_horizontal);
        self.root.legacy.max_scroll = (y - viewport.height).max(0.);
        if y < viewport.height {
            for p in &mut placements {
                p.top += viewport.height - y;
            }
        }
        let old_scroll = self.root.legacy.scroll;
        self.root.legacy.expansion_positions.clear();
        for (row, p) in rows.iter().zip(&placements) {
            if let Some(layout) = detail_layouts.get(&row.key) {
                for (line, (top, _)) in row.details.iter().zip(layout) {
                    if line.toggle.is_some() {
                        self.root.legacy.expansion_positions
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
            self.root.legacy.scroll = self.root.legacy.scroll.clamp(0., self.root.legacy.max_scroll);
        } else if position.follow {
            self.root.legacy.scroll = self.root.legacy.max_scroll;
        } else if let Some(key) = &position.key
            && let Some(p) = placements.iter().find(|p| &p.key == key)
        {
            self.root.legacy.scroll = (p.top + position.offset * s).clamp(0., self.root.legacy.max_scroll);
        } else {
            self.root.legacy.scroll = self.root.legacy.scroll.clamp(0., self.root.legacy.max_scroll);
        }
        if let Some((key, screen_y)) = &self.root.legacy.expansion_pin
            && let Some(top) = self.root.legacy.expansion_positions.get(key)
        {
            self.root.legacy.scroll = (top - screen_y).clamp(0., self.root.legacy.max_scroll);
        }
        let locating_download = self.locate_download(&rows, &placements, viewport);
        if let Some(wheel) = &mut self.root.legacy.wheel
            && wheel.lane == Lane::Transcript
        {
            wheel.target = (wheel.target + self.root.legacy.scroll - old_scroll).clamp(0., self.root.legacy.max_scroll);
        }
        let mut tool_roots=std::collections::HashMap::new();
        for event in self.controller.chats[&session].feed.events.values() {
            if let Some(call)=event.tool_call_id.as_deref() {
                if event.kind==EventKind::Tool && event.role!=EventRole::Tool {tool_roots.insert(call,&event.id);}
                else {tool_roots.entry(call).or_insert(&event.id);}
            }
        }
        let top=self.root.legacy.scroll-2.*viewport.height;let bottom=self.root.legacy.scroll+3.*viewport.height;
        for (row,p) in rows.iter().zip(&placements).filter(|(_,p)|p.top+p.height>=top && p.top<=bottom) {
            if row.details.is_empty() {
                if let Some(id) = &row.block { interests.insert(id.clone()); }
            } else if let Some(layout)=detail_layouts.get(&row.key) {
                for (line,(offset,height)) in row.details.iter().zip(layout) {
                    if p.top+offset+height<top || p.top+offset>bottom {continue;}
                    if let Some(id)=line.key.strip_prefix("thinking:") {interests.insert(id.into());}
                    if let Some(key)=line.key.strip_prefix("tool:") {
                        let mut key=key;
                        if !tool_roots.contains_key(key) {
                            key=key.strip_suffix(":body").unwrap_or(key);
                            for suffix in [":Input",":Output",":Error"] {if let Some(base)=key.strip_suffix(suffix) {key=base;break;}}
                        }
                        if let Some(id)=tool_roots.get(key) {interests.insert((*id).clone());}
                    }
                }
            }
        }
        self.root.legacy.placed = placements;
        self.root.legacy.placed_session = Some(session.clone());
        if can_remember || locating_download {
            self.remember_scroll();
        }
        self.history_near_edge(&session, self.root.legacy.navigation.download.is_some() || self.root.legacy.scroll <= 2. * viewport.height);
        if quick_models {
            self.quick_models_frame(
                layer,
                &session,
                Rect::new(x, viewport.y + quick_top - self.root.legacy.scroll, width, quick_h),
                viewport,
            );
        }
        for (index, row) in rows.iter().enumerate() {
            let height = self.root.legacy.placed[index].height;
            let top = viewport.y + self.root.legacy.placed[index].top - self.root.legacy.scroll;
            if top + height < viewport.y || top > viewport.y + viewport.height {
                continue;
            }
            let x = x + if row.user { width - bubble_width } else { 0. };
            let rect = Rect::new(x, top, bubble_width, height);
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
            self.root.legacy.message_areas.push(MessageArea {
                key: row.key.clone(),
                rect,
                corners,
                clip: viewport,
                options: row.actions.clone(),
            });
            // Pin by logical key while a menu is open, not screen coordinates:
            // streaming and paging may move the target without changing its copy boundary.
            let pinned = self
                .root.legacy.context_menu
                .as_ref()
                .is_some_and(|menu| menu.section.as_deref() == Some(row.key.as_str()));
            self.services.renderer.clipped_label(
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
                    self.root.legacy.detail_areas.push(DetailArea { key: line_key.clone(), rect: line_rect, corners: inner_corners, clip: viewport });
                    if line.tool {
                        layer.clipped_rect(line_rect, color(0x111922), viewport);
                    }
                    if !line.source.is_empty() {
                        self.services.renderer.message(
                            layer,
                            &format!("{session}/{}", line.key),
                            Vec2::new(lx, top + offset),
                            Rect::new(
                                lx,
                                viewport.y,
                                text_width - line.indent * s,
                                viewport.height,
                            ),
                            self.root.legacy.horizontal,
                        );
                    } else {
                        let label_x = if let Some(open) = line.toggle {
                            self.services.renderer.clipped_icon(ctx, layer,
                                if open { Icon::ChevronDown } else { Icon::ChevronRight },
                                Rect::new(lx, top + offset + 6. * s, 14. * s, 14. * s),
                                if line.error { 0xffb4ab } else { 0xb7c2ce }, viewport);
                            lx + 18. * s
                        } else { lx };
                        if let Some(open) = line.toggle {
                            let hit = crate::render::intersect(line_rect, viewport);
                            if hit.height > 0. {
                                self.root.legacy.hits.push(Hit {
                                    rect: hit,
                                    action: Action::Toggle(line.key.clone(), !open),
                                });
                            }
                        }
                        self.services.renderer.clipped_label(
                            layer,
                            &line.label,
                            Rect::new(
                                label_x,
                                top + offset + 5. * s,
                                text_width - line.indent * s - (label_x - lx),
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
                        self.root.legacy.ripple.as_ref().and_then(|r| r.paint(&line_key, line_rect, paint_at)));
                }
                let inner_hover = layer.interaction.hover.is_some_and(|point|
                    self.root.legacy.detail_areas.iter().any(|area| area.contains(point)));
                layer.surface_highlight(rect, corners, viewport, pinned, !inner_hover,
                    self.root.legacy.ripple.as_ref().and_then(|r| r.paint(&row.key, rect, paint_at)));
                continue;
            }
            let label_rect = Rect::new(x + 14. * s, top + 28. * s, text_width, 20. * s);
            if row.header {
                self.services.renderer.clipped_label(
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
                self.services.renderer.message(
                    layer,
                    &row.key,
                    Vec2::new(x + 14. * s, text_top),
                    Rect::new(x + 14. * s, viewport.y, text_width, viewport.height),
                    self.root.legacy.horizontal,
                );
            }
            if let Some((entry, attachment)) = &row.attachment {
                self.attachment_card(ctx, layer, &session, entry, attachment, "chat", rect, viewport);
            }
            layer.surface_highlight(rect, corners, viewport, pinned, true,
                self.root.legacy.ripple.as_ref().and_then(|r| r.paint(&row.key, rect, paint_at)));
        }
        self.scrollbar(chrome, Lane::Transcript, viewport);
        self.draw_composer(ctx, chrome, b, &session, false);
    }
    fn composer_layout(&mut self, b: Rect, session: &str) -> (f32, f32, f32, f32, f32) {
        let s = self.ui.scale;
        let files = &self.controller.chats[session].local.files;
        let width = (b.width - 28. * s).min(900. * s).max(160. * s);
        let x = b.x + (b.width - width) / 2.;
        let editor_h = self
            .root.legacy.composer
            .height(&mut self.services.renderer, width - 132. * s, 16. * s);
        let queue = &self.controller.chats[session].feed.queue;
        let controls = queue
            .control
            .as_ref()
            .is_some_and(|c| matches!(c.status.as_str(), "waiting" | "applying"));
        let composer_h = editor_h
            + (44. + if files.is_empty() { 0. } else { 40. } + if controls { 40. } else { 0. }) * s;
        let bottom = b.y + b.height;
        let composer_top = (bottom - composer_h).max(b.y + 80. * s);
        (x, width, editor_h, composer_h, composer_top)
    }
    fn draw_composer(&mut self, ctx: &impl RenderContext, chrome: &mut Layer, b: Rect, session: &str, in_code: bool) {
        let s = self.ui.scale;
        let (x, width, editor_h, composer_h, composer_top) = self.composer_layout(b, session);
        let files = self.controller.chats[session].local.files.clone();
        let controls = self.controller.chats[session].feed.queue.control.as_ref().is_some_and(|c| matches!(c.status.as_str(), "waiting" | "applying"));
        let summary = self.controller.account.sessions.iter().find(|s| s.id == session).cloned();
        chrome.rect(
            Rect::new(b.x, composer_top, b.width, composer_h),
            color(0x0e141b),
        );
        let creating = self.controller.is_creating(&session);
        let choosing = self.controller.chats[session].model_request.is_some();
        let status_rect = Rect::new(x, composer_top + 10. * s,
            (width - if creating { 94. * s } else { 0. }).max(1.), 20. * s);
        if creating || choosing {
            self.services.renderer.label(chrome,
                if creating { "Creating chat… Sends are saved locally." }
                    else { "Selecting model… Sends are saved locally." },
                status_rect, 12. * s, color(0x82909f), false);
        } else {
            self.composer_model_status(chrome, summary.as_ref(), status_rect);
        }
        if creating && self.controller.epoch.is_some() {
            button(&mut self.services.renderer, chrome, &mut self.root.legacy.hits,
                Rect::new(x + width - 88. * s, composer_top + 6. * s, 88. * s, 24. * s),
                "Retry", Action::RetryCreate, s, false);
        }
        chrome.rect(Rect::new(b.x, composer_top, b.width, s), color(0x2a3541));
        let field = Rect::new(x, composer_top + 32. * s, width, editor_h);
        let edge = if self.root.legacy.focus == Some(None) { 2. * s } else { s };
        chrome.rounded_rect(
            field,
            4. * s,
            color(if self.root.legacy.focus == Some(None) {
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
        self.root.legacy.composer.draw(
            &mut self.services.renderer,
            chrome,
            composer_rect,
            16. * s,
            self.root.legacy.focus == Some(None),
            false,
            "Message Tau",
            false,
        );
        self.root.legacy.hits.push(Hit {
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
            true,
        );
        let usage_rect = Rect::new(field.x + field.width - 84. * s, iy, 40. * s, 40. * s);
        let usage = summary.as_ref().and_then(|s| s.context_usage);
        let (ratio, usage_text) = context_usage_display(usage);
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
        self.root.legacy.usage.region = usage_rect;
        self.root.legacy.usage.content = usage_text;
        if usage.is_some()
            && (!connected
                || !self.controller.chats[session].feed.synchronized
                || summary.as_ref().is_none_or(|s| {
                    !matches!(s.status, SessionStatus::Idle | SessionStatus::Running)
                }))
        {
            self.root.legacy.usage.content.line().dim("Last known value");
        }
        match summary.as_ref().and_then(|s|s.model.as_ref()).map(|m|m.provider.as_str()) {
            Some("openai-codex") => {
                self.root.legacy.usage.content.line().line();
                self.root.legacy.usage.content.append(self.controller.codex_usage.content(connected));
            }
            Some(_) => { self.root.legacy.usage.content.line().line().dim("Account quota unavailable for this provider"); }
            None => { self.root.legacy.usage.content.line().line().dim("Account quota unavailable (model unknown)"); }
        }
        let can_send = (!self.root.legacy.composer.value.trim().is_empty() || !files.is_empty())
            && (!in_code || self.root.legacy.code.as_ref().is_some_and(|c|c.selection.is_some() && c.error.is_none()));
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
        let controls_y = field.y + field.height + 4. * s;
        if !in_code && self.root.legacy.scroll + 24. * s < self.root.legacy.max_scroll {
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
        let queue = &self.controller.chats[session].feed.queue;
        if let Some(control) = &queue.control
            && matches!(control.status.as_str(), "waiting" | "applying")
        {
            button(
                &mut self.services.renderer,
                chrome,
                &mut self.root.legacy.hits,
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
                &mut self.services.renderer,
                chrome,
                &mut self.root.legacy.hits,
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
        if self.root.legacy.composer.value.starts_with('/')
            && !self.root.legacy.composer.value.contains('\n')
            && self.root.legacy.focus == Some(None)
        {
            let query = &self.root.legacy.composer.value[1..];
            let mut suggestions = vec![];
            for command in &self.controller.chats[session].commands {
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
                    &mut self.services.renderer,
                    chrome,
                    &mut self.root.legacy.hits,
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
        let columns = if width / self.ui.scale >= 520. { 2 } else { 1 };
        (72. + self
            .controller
            .model_preferences
            .slugs
            .len()
            .div_ceil(columns) as f32
            * 84.
            + 48.)
            * self.ui.scale
    }
    fn quick_models_frame(&mut self, layer: &mut Layer, session: &str, b: Rect, clip: Rect) {
        let s = self.ui.scale;
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
        self.services.renderer.clipped_label(
            layer,
            "Choose a model",
            Rect::new(b.x, b.y, b.width, 28. * s),
            20. * s,
            color(0xe5eaf0),
            true,
            clip,
        );
        self.services.renderer.clipped_label(
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
            self.services.renderer.clipped_label(
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
            self.services.renderer.clipped_label(
                layer,
                status,
                Rect::new(r.x + 12. * s, r.y + 54. * s, w - 24. * s, 16. * s),
                11. * s,
                color(if selected { 0x67d4ff } else { 0xb7c2ce }),
                false,
                crate::render::intersect(r, clip),
            );
            if enabled {
                self.root.legacy.hits.push(Hit {
                    rect: crate::render::intersect(r, clip),
                    action: Action::ChooseModel(session.into(), selector.clone()),
                });
            }
        }
        let r = Rect::new(b.x, b.y + b.height - 40. * s, b.width, 32. * s);
        layer.clipped_rounded_rect(r, 16. * s, layer.control_color(r, color(0x18212b)), clip);
        self.services.renderer.clipped_label(
            layer,
            "Configure quick models…",
            Rect::new(r.x + 12. * s, r.y + 7. * s, r.width - 24. * s, 20. * s),
            12. * s,
            color(0x67d4ff),
            false,
            crate::render::intersect(r, clip),
        );
        self.root.legacy.hits.push(Hit {
            rect: crate::render::intersect(r, clip),
            action: Action::ModelSettings,
        });
    }
    fn notice_frame(&mut self, ctx: &impl RenderContext, layer: &mut Layer, b: Rect) {
        self.root.legacy.notice_popup.observe(self.controller.notice.as_ref(), Instant::now());
        if !self.root.legacy.notice_popup.visible() { return; }
        let Some(notice) = self.controller.notice.as_deref() else { return; };
        let s = self.ui.scale;
        let size = 16. * s;
        let max_width = (b.width - 32. * s).max(1.).min(560. * s);
        let style = sanscale::Style { chain: self.services.renderer.faces.prose[0], wrap_em: None,
            align: sanscale::Align::Left, line_spacing: 1.15 };
        let natural = self.services.renderer.text.shape_transient(notice, &style)
            .map(|block| self.services.renderer.text.measure(block).width_em() * size).unwrap_or(max_width);
        let width = (natural + 72. * s).max(240. * s).min(max_width);
        let text_width = (width - 72. * s).max(1.);
        let text_height = self.services.renderer.label_height(notice, text_width, size, false);
        let height = (text_height + 24. * s).max(48. * s).min((b.height - 32. * s).max(1.));
        let rect = Rect::new(b.x + (b.width - width) / 2., b.y + 16. * s, width, height);
        let action = self.controller.notice.as_ref().and_then(|notice| notice.download.clone())
            .map_or(Action::DismissNotice, Action::OpenDownloadNotice);
        self.root.legacy.hits.push(Hit { rect, action });
        layer.rounded_rect(rect, 12. * s, color(0x263340));
        self.services.renderer.clipped_label(layer, notice,
            Rect::new(rect.x + 16. * s, rect.y + ((height - text_height) / 2.).max(12. * s), text_width, text_height),
            size, color(0xe5eaf0), false, rect);
        let close = Rect::new(rect.x + width - 44. * s, rect.y + (height - 40. * s) / 2., 40. * s, 40. * s);
        if layer.interaction.hover.is_some_and(|p| contains(close, p)) {
            layer.rounded_rect(close, 20. * s, layer.control_color(close, color(0x354454)));
        }
        self.services.renderer.icon(ctx, layer, Icon::Close,
            Rect::new(close.x + 10. * s, close.y + 10. * s, 20. * s, 20. * s), 0xe5eaf0);
        self.root.legacy.hits.push(Hit { rect: close, action: Action::DismissNotice });
    }
    pub fn context_at(&mut self, point: Vec2) {
        if self.root.legacy.hits.iter().rev().find(|hit| contains(hit.rect, point))
                .is_some_and(|hit| matches!(hit.action, Action::DismissNotice | Action::OpenDownloadNotice(_)))
            || self.root.legacy.modal.is_some()
            || self.root.legacy.viewer.is_some()
            || self.root.legacy.usage.contains_card(point)
            || self.root.legacy.info_tip.contains_card(point)
            || self.root.legacy.context_menu.is_some() && contains(self.root.legacy.context_rect, point)
        {
            return;
        }
        if let Some((_, info)) = self.root.legacy.info_areas.iter().find(|(r, info)| matches!(info, Info::Attachment(..)) && contains(*r, point)) {
            let info = info.clone();
            self.activate(Action::Info(info));
            return;
        }
        if let Some(id) = self.root.legacy.project_areas.iter().find(|(r,_)| contains(*r, point)).map(|(_,id)| id.clone()) {
            self.project_context(&id, point);
            return;
        }
        let mut chat = self
            .root.legacy.chat_areas
            .iter()
            .find(|(rect, _)| contains(*rect, point))
            .map(|(_, id)| id.clone());
        let transcript = contains(self.root.legacy.transcript, point);
        if chat.is_none() && !transcript {
            return;
        }
        if self.cancel_autoscroll() {
            return;
        }
        self.root.legacy.wheel = None;
        self.root.legacy.velocity = 0.;
        self.root.legacy.usage.dismiss();
        self.root.legacy.info_tip.dismiss();
        let area = transcript
            .then(|| self.root.legacy.message_areas.iter().find(|a| a.contains(point)))
            .flatten();
        let mut options = vec![];
        if chat.is_none() {
            if self
                .services.renderer
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
                ("Review restored history…".into(),Action::ReviewRestore(id.clone())),
                ("Clone chat".into(), Action::Clone(id.clone())),
                ("Release idle runtime".into(), Action::Sleep(id.clone())),
                ("Delete chat…".into(), Action::Delete(id.clone())),
            ];
        }
        if let Some(id)=&chat && self.controller.account.missing_chats.contains(id) {
            options=vec![("Copy draft to a new chat (not sent)".into(),Action::CopyRecoveredDraft(id.clone())),("Forget this local recovery…".into(),Action::ForgetRecovered(id.clone()))];
        }
        self.root.legacy.context_menu = (!options.is_empty()).then(|| ContextMenu {
            at: point,
            section: area.map(|a| a.key.clone()),
            chat,
            options,
            selected: 0,
            scroll: 0.,
            parent: None,
        });
        self.root.legacy.pointer = None;
        self.root.legacy.selecting = false;
        self.ui.dirty = true;
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
        let tonal = matches!(icon, Icon::Stop | Icon::Play | Icon::Attachments);
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
        let pixels = size * self.ui.scale;
        self.services.renderer.icon(
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
            self.root.legacy.hits.push(Hit { rect: r, action });
        }
    }
    fn usage_frame(&mut self, layer: &mut Layer, bounds: Rect) {
        if self.root.legacy.usage.progress <= 0.
            || self.root.legacy.usage.region.width <= 0.
            || self.root.legacy.modal.is_some()
            || self.root.legacy.viewer.is_some()
            || self.root.legacy.context_menu.is_some()
        {
            return;
        }
        self.root.legacy.usage.frame(&mut self.services.renderer, layer, "usage", bounds, self.ui.scale, 320., true);
    }
    fn info_frame(&mut self, layer: &mut Layer, bounds: Rect) {
        if self.root.legacy.info_tip.progress <= 0.
            || self.root.legacy.info_tip.region.width <= 0.
            || self.root.legacy.modal.is_some()
            || self.root.legacy.viewer.is_some()
            || self.root.legacy.context_menu.is_some()
        {
            return;
        }
        self.root.legacy.info_tip.content = match &self.root.legacy.info_target {
            Info::Attachment(_, title, detail) => {
                let mut content = Content::default();
                content.strong(title, crate::tooltip::INK).line().dim(detail);
                content
            }
            Info::Connection => {
                let mut content=self.controller.health.tooltip(&self.controller.connection,Instant::now());
                if let Some(detail)=&self.controller.transport_error {
                    content.line().dim("Last transport issue: ").push(detail,false,crate::tooltip::WARNING);
                }
                content
            }
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
                self.controller.cache_ttl(session).details()
            }
        };
        // Cover the New chat button beneath the connection card, including on phones.
        let width = if self.root.legacy.info_target == Info::Connection {
            if bounds.width / self.ui.scale < 760. { bounds.width / self.ui.scale - 16. } else { 300. }
        } else if matches!(self.root.legacy.info_target, Info::Attachment(..)) { 300. } else { 180. };
        self.root.legacy.info_tip.frame(&mut self.services.renderer, layer, "info", bounds, self.ui.scale, width, false);
    }
    fn settings_frame(&mut self, layer: &mut Layer, b: Rect) {
        let s = self.ui.scale;
        self.root.legacy.hits.clear();
        layer.rect(b, color(0x0e141b));
        let modal = self.root.legacy.modal.as_mut().unwrap();
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
        self.services.renderer.label(
            layer,
            "Tau",
            Rect::new(x, card.y + 24. * s, inner_w, 44. * s),
            32. * s,
            color(0xe5eaf0),
            true,
        );
        self.services.renderer.label(
            layer,
            concat!("Version ", env!("CARGO_PKG_VERSION"), " · Beta"),
            Rect::new(x, card.y + 66. * s, inner_w, 20. * s),
            12. * s,
            color(0xb7c2ce),
            false,
        );
        self.services.renderer.label(
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
                &mut self.services.renderer,
                layer,
                rect,
                16. * s,
                self.root.legacy.focus == Some(Some(i)),
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
                chain: self.services.renderer.faces.prose[0],
                wrap_em: None,
                align: sanscale::Align::Left,
                line_spacing: 1.,
            };
            let label_w = self
                .services.renderer
                .text
                .shape_transient(label, &style)
                .map_or(100. * s, |b| {
                    self.services.renderer.text.measure(b).width_em() * 12. * s + 12. * s
                });
            layer.rect(
                Rect::new(x + 12. * s, rect.y - 7. * s, label_w, 16. * s),
                color(0x36343b),
            );
            self.services.renderer.label(
                layer,
                label,
                Rect::new(x + 16. * s, rect.y - 8. * s, label_w, 18. * s),
                12. * s,
                color(0xb7c2ce),
                false,
            );
            self.root.legacy.hits.push(Hit {
                rect,
                action: Action::Focus(Some(i)),
            });
        }
        let buttons_y = field_y + 158. * s;
        if !self.root.legacy.connecting || self.controller.connection != "Connecting…" {
            button(
                &mut self.services.renderer,
                layer,
                &mut self.root.legacy.hits,
                Rect::new(x + inner_w - 104. * s, buttons_y, 104. * s, 40. * s),
                "Connect",
                Action::Confirm,
                s,
                true,
            );
        } else {
            self.services.renderer.label(
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
            &mut self.services.renderer,
            layer,
            &mut self.root.legacy.hits,
            Rect::new(x + inner_w - 204. * s, buttons_y, 88. * s, 40. * s),
            "Cancel",
            Action::CancelModal,
            s,
            false,
        );
        let mut y = buttons_y + 60. * s;
        button(
            &mut self.services.renderer,
            layer,
            &mut self.root.legacy.hits,
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
            self.services.renderer.label(
                layer,
                "Daemon settings",
                Rect::new(x, y, inner_w, 26. * s),
                16. * s,
                color(0xe5eaf0),
                true,
            );
            y += 28. * s;
            if connected && !self.root.legacy.waiting_settings {
                button(
                    &mut self.services.renderer,
                    layer,
                    &mut self.root.legacy.hits,
                    Rect::new(x, y, (inner_w - 8. * s) / 2., 32. * s),
                    "Open settings",
                    Action::DaemonSettings,
                    s,
                    false,
                );
                button(
                    &mut self.services.renderer,
                    layer,
                    &mut self.root.legacy.hits,
                    Rect::new(x + (inner_w + 8. * s) / 2., y, (inner_w - 8. * s) / 2., 32. * s),
                    "Refresh models",
                    Action::RefreshCatalog,
                    s,
                    false,
                );
            } else {
                self.services.renderer.label(
                    layer,
                    if self.root.legacy.waiting_settings {
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
            self.services.renderer.label(
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
        let s = self.ui.scale;
        self.root.legacy.hits.clear();
        layer.rect(b, color(0x0e141b));
        let w = (b.width - 32. * s).min(680. * s).max(1.);
        let x = b.x + (b.width - w) / 2.;
        let top = b.y + 16. * s;
        let footer = b.y + b.height - 56. * s;
        let modal = self.root.legacy.modal.as_mut().unwrap();
        self.services.renderer.label(
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
        self.services.renderer.label(layer, if compact { "One slug per line. Last chosen model is remembered. Empty disables tiles." }
            else { "One provider/model per line (max 12). New chats keep the last chosen model. This list only controls quick-select tiles; empty disables them." },
            Rect::new(x, top + 36. * s, w, help_h), 12. * s, color(0xb7c2ce), false);
        let edit_y = top + 36. * s + help_h + 8. * s;
        let edit_h = (b.height * 0.25)
            .min(164. * s)
            .min((footer - 48. * s - edit_y - if show_suggestions { 56. * s } else { 0. }).max(0.));
        let edit = Rect::new(x, edit_y, w, edit_h);
        modal.fields[0].1.draw(
            &mut self.services.renderer,
            layer,
            edit,
            16. * s,
            self.root.legacy.focus == Some(Some(0)),
            false,
            "provider/model",
            true,
        );
        self.root.legacy.hits.push(Hit {
            rect: edit,
            action: Action::Focus(Some(0)),
        });
        let search = Rect::new(x, edit.y + edit.height + 12. * s, w, 36. * s);
        if show_suggestions {
            modal.fields[1].1.draw(
                &mut self.services.renderer,
                layer,
                search,
                16. * s,
                self.root.legacy.focus == Some(Some(1)),
                false,
                "Search optional model suggestions",
                true,
            );
            self.root.legacy.hits.push(Hit {
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
            self.services.renderer.label(
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
            self.root.legacy.hits.push(Hit {
                rect: r,
                action: Action::ToggleQuickModel(existing.unwrap_or(&model.value).clone()),
            });
            shown += 1;
        }
        if shown == 0 && count > 0 {
            self.services.renderer.label(
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
        self.services.renderer.label(
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
                &mut self.services.renderer,
                layer,
                &mut self.root.legacy.hits,
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
        if let (Some(draft), Some(modal)) = (&mut self.root.legacy.daemon_draft, &self.root.legacy.modal)
            && let Some((_, editor, _)) = modal.fields.first() {
            draft.apply(&editor.value)?;
        }
        Ok(())
    }
    fn load_setting_field(&mut self) -> Result<()> {
        use crate::daemon_settings::Kind;
        if let (Some(draft), Some(modal)) = (&mut self.root.legacy.daemon_draft, &mut self.root.legacy.modal) {
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
        self.root.legacy.focus = None;
        Ok(())
    }
    fn daemon_settings_frame(&mut self, layer: &mut Layer, b: Rect) {
        use crate::daemon_settings::{Kind, SECTIONS, fields};
        let s = self.ui.scale;
        self.root.legacy.hits.clear();
        layer.rect(b, color(0x0e141b));
        let w = (b.width - 32. * s).min(900. * s).max(1.);
        let x = b.x + (b.width - w) / 2.;
        let top = b.y + 12. * s;
        let footer = b.y + b.height - 52. * s;
        let title = self
            .root.legacy.daemon_draft
            .as_ref()
            .map(|d| format!("Daemon settings · revision {}", d.revision))
            .unwrap_or_else(|| "Daemon settings".into());
        self.services.renderer.label(
            layer,
            &title,
            Rect::new(x, top, w, 28. * s),
            20. * s,
            color(0xe5eaf0),
            true,
        );
        let busy = self.root.legacy.saving_settings.is_some();
        if let Some(draft) = &self.root.legacy.daemon_draft {
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
                    &mut self.services.renderer,
                    layer,
                    &mut self.root.legacy.hits,
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
                    &mut self.services.renderer,
                    layer,
                    &mut self.root.legacy.hits,
                    Rect::new(x, y, 36. * s, 32. * s),
                    "‹",
                    Action::SettingsField(false),
                    s,
                    false,
                );
                button(
                    &mut self.services.renderer,
                    layer,
                    &mut self.root.legacy.hits,
                    Rect::new(x + w - 36. * s, y, 36. * s, 32. * s),
                    "›",
                    Action::SettingsField(true),
                    s,
                    false,
                );
            }
            self.services.renderer.label(
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
            self.services.renderer.label(
                layer,
                &help,
                Rect::new(x, y, w, help_h),
                12. * s,
                color(0xb7c2ce),
                false,
            );
            y += help_h + 8. * s;
            let modal = self.root.legacy.modal.as_mut().unwrap();
            if let Some((_, editor, _)) = modal.fields.first_mut() {
                if definition.kind == Kind::Bool {
                    button(
                        &mut self.services.renderer,
                        layer,
                        &mut self.root.legacy.hits,
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
                            &mut self.services.renderer,
                            layer,
                            &mut self.root.legacy.hits,
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
                        &mut self.services.renderer,
                        layer,
                        rect,
                        15. * s,
                        !readonly && self.root.legacy.focus == Some(Some(0)),
                        false,
                        "",
                        true,
                    );
                    if !readonly {
                        self.root.legacy.hits.push(Hit {
                            rect,
                            action: Action::Focus(Some(0)),
                        });
                    }
                }
            }
            button(
                &mut self.services.renderer,
                layer,
                &mut self.root.legacy.hits,
                Rect::new(x, footer - 80. * s, 120. * s, 28. * s),
                "Reset field",
                Action::SettingReset,
                s,
                false,
            );
            self.services.renderer.label(
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
            self.services.renderer.label(
                layer,
                if self.root.legacy.waiting_settings {
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
        self.services.renderer.label(layer,self.controller.notice.as_deref().unwrap_or(if busy {"Saving…"} else {"Credentials remain private on the daemon. Conflicts never overwrite newer settings."}),Rect::new(x,footer-42.*s,w,36.*s),12.*s,color(if self.controller.notice.as_deref() == Some("Settings saved") {0x67d4ff} else if self.controller.notice.is_some() {0xffb4ab} else {0x82909f}),false);
        if busy {
            self.root.legacy.hits.clear();
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
                &mut self.services.renderer,
                layer,
                &mut self.root.legacy.hits,
                Rect::new(x + i as f32 * (bw + 8. * s), footer, bw, 40. * s),
                label,
                action,
                s,
                i == 0,
            );
        }
    }
    fn modal_frame(&mut self, layer: &mut Layer, b: Rect) {
        let s = self.ui.scale;
        self.root.legacy.hits.clear();
        layer.rect(b, sanscale::Color([0., 0., 0., 0.8]));
        let modal = self.root.legacy.modal.as_mut().unwrap();
        let long = matches!(modal.kind, ModalKind::QueueEdit(..));
        let field_h = if long { 180. } else { 60. };
        let width = (b.width - 24. * s).min(620. * s);
        let title_h=self.services.renderer.label_height(&modal.title,width-40.*s,17.*s,true).max(36.*s);
        let height = ((title_h/s+36.
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
        self.services.renderer.label(
            layer,
            &modal.title,
            Rect::new(rect.x + 20. * s, rect.y + 18. * s, width - 40. * s, title_h),
            17. * s,
            color(0xe5eaf0),
            true,
        );
        let mut y = rect.y + title_h + 24. * s;
        for (i, (name, e, secret)) in modal.fields.iter_mut().enumerate() {
            self.services.renderer.label(
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
                &mut self.services.renderer,
                layer,
                field,
                16. * s,
                self.root.legacy.focus == Some(Some(i)),
                *secret,
                "",
                true,
            );
            self.root.legacy.hits.push(Hit {
                rect: field,
                action: Action::Focus(Some(i)),
            });
            y += (field_h + 4.) * s;
        }
        for (label, action) in &modal.options {
            button(
                &mut self.services.renderer,
                layer,
                &mut self.root.legacy.hits,
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

fn context_usage_display(usage: Option<ContextUsage>) -> (Option<f32>, Content) {
    use crate::tooltip::{INK, ACCENT, WARNING, DANGER};
    let mut content = Content::default();
    content.strong("Context", INK);
    let mut ratio = None;
    match usage {
        Some(ContextUsage { tokens: Some(used), context_window: Some(capacity), .. }) if capacity > 0 => {
            let used_ratio = used as f32 / capacity as f32;
            ratio = Some(used_ratio);
            let tint = if used_ratio >= 0.95 { DANGER } else if used_ratio >= 0.8 { WARNING } else { ACCENT };
            content.dim(" · ").strong(format!("~{:.0}%", used_ratio * 100.), tint).dim(" used")
                .line().dim(format!("{} of {} tokens", count(used), count(capacity)));
        }
        Some(ContextUsage { tokens: Some(used), .. }) => {
            content.dim(" · ").strong(format!("~{}", count(used)), ACCENT).dim(" tokens used")
                .line().dim("Capacity unknown");
        }
        Some(ContextUsage { context_window: Some(capacity), .. }) if capacity > 0 => {
            content.dim(" · usage unknown").line().dim(format!("Capacity: {} tokens", count(capacity)));
        }
        Some(_) => { content.dim(" · usage unknown").line().dim("Capacity unknown"); }
        None => { content.dim(" · usage unavailable"); }
    }
    (ratio, content)
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
mod connection_tests;
#[cfg(all(test, not(target_os = "android")))]
mod tooltip_tests;
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
#[cfg(all(test, not(target_os = "android")))]
mod navigation_tests;

#[cfg(test)]
mod usage_tests {
    use super::*;

    #[test]
    fn token_usage_does_not_require_a_guessed_context_window() {
        let unknown_capacity = Some(ContextUsage { tokens: Some(1_024), context_window: None });
        let (ring, text) = context_usage_display(unknown_capacity);
        assert_eq!(ring, None);
        assert_eq!(text.text, "Context · ~1,024 tokens used\nCapacity unknown");
        assert_eq!(context_usage_display(Some(ContextUsage { tokens: None, context_window: None })).1.text,
            "Context · usage unknown\nCapacity unknown");
        assert_eq!(context_usage_display(None).1.text, "Context · usage unavailable");

        let (ring, text) = context_usage_display(Some(ContextUsage { tokens: Some(1_024), context_window: Some(4_096) }));
        assert_eq!(ring, Some(0.25));
        assert_eq!(text.text, "Context · ~25% used\n1,024 of 4,096 tokens");
        assert_eq!(context_usage_display(Some(ContextUsage { tokens: None, context_window: Some(4_096) })).1.text,
            "Context · usage unknown\nCapacity: 4,096 tokens");
    }
}

#[cfg(all(test, not(target_os = "android")))]
mod download_render_tests;

#[cfg(all(test, not(target_os = "android")))]
mod download_interaction_tests;

#[cfg(all(test, not(target_os = "android")))]
mod composer_status_tests;
