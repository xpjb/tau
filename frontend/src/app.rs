use crate::{
    clock,
    connection::{CounterTicker, COUNTER_REFRESH},
    controller::Controller,
    details::{Line as DetailLine, Tools},
    editor::Editor,
    icons::Icon,
    render::{Interaction, Layer, Renderer, color, contains, contains_rounded},
    scroll::{Autoscroll, Drag, Lane, Scrollbar, Wheel},
    store::Store,
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

mod attachments;
mod notices;
mod navigation;
use crate::notice::DownloadTarget;
mod ui;
mod ui_owner;
mod mobile_input;
use ui::Widget;
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
    MoveMenu(String),
    ContextBack,
    MoveChat(String, String),
    Noop,
    RetryCreate,
    Settings,
    ModelSettings,
    ChooseModel(String, String),
    Usage,
    Info(Info),
    Back,
    Send,
    Abort,
    Tail,
    DismissNotice,
    OpenDownloadNotice(DownloadTarget),
    CopyRecoveredDraft(String),
    ForgetRecovered(String),
    ReviewRestore(String),
    Outbox(usize),
    InspectControl(String),
    ForgetControl(String),
    Focus(Option<usize>),
    Confirm,
    CancelModal,
    DaemonSettings,
    RefreshCatalog,
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
    Suggest(String),
}
#[derive(Clone, Debug, PartialEq, Eq)]
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
    actions: Vec<(String, ui::MenuChoice)>,
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

struct MessageArea {
    key: String,
    rect: Rect,
    corners: [f32; 4],
    clip: Rect,
    options: Vec<(String, ui::MenuChoice)>,
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
    Paste { token: u64 },
    PickFile {
        identity: String,
        session: String,
    },
    OpenUrl(String),
    SaveDownload { key: String, source: PathBuf, name: String },
    UseDownload(crate::store::SavedDownload, SavedAction, DownloadTarget),
    InputMenu,
    Background,
}
#[derive(Clone, Copy, Debug)]
pub enum SavedAction { Open, #[cfg(not(target_os = "android"))] Show, #[cfg(not(target_os = "android"))] Extract }

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
    cards: ui::CardDeck,
    hits: Vec<Hit>,
    message_areas: Vec<MessageArea>,
    detail_areas: Vec<DetailArea>,
    ripple: Option<Ripple>,
    chat_areas: Vec<(Rect, String)>,
    info_areas: Vec<(Rect, Info)>,
    connection_counter: CounterTicker,
    counter_bucket: Option<u128>,
    dot_color: u32,
    connection_visible: bool,
    navigation: navigation::Navigation,
    show_chats: bool,
    scroll: f32,
    max_scroll: f32,
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
    velocity: f32,
    selecting: bool,
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
                gpu: ui::Gpu::new(ctx),
                transfers: attachments::Transfers { pending_exports: HashMap::new(), export_targets: HashMap::new(), saving_downloads: HashSet::new(), export_errors: HashMap::new(), download_identity, progress_clock: Instant::now(), progress_bucket: None, },
            },
            ui: ui::UiState::new(ctx.size(), mobile),
            root: ui::RootWidget {
                dialog: None,
                viewer: None,
                menu: None,
                notice: ui::NoticeWidget::new(),
                tooltips: ui::TooltipHost::default(),
                attachments: ui::AttachmentBrowser::new(),
                sidebar: ui::Sidebar::new(),
                composer: ui::Composer::new(),
                code: code_view::CodeBrowser::new(),
                quick_models: ui::QuickModels::new(),
                legacy: LegacyWorkspace {
                    cards: ui::CardDeck::new(),
                    hits: vec![],
                    message_areas: vec![],
                    detail_areas: vec![],
                    ripple: None,
                    chat_areas: vec![],

                    info_areas: vec![],
                    connection_counter: CounterTicker::new(wake.clone()),
                    counter_bucket: None,
                    dot_color,
                    connection_visible: true,
                    navigation,
                    show_chats,

                    scroll: 0.,
                    max_scroll: 0.,
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
                    velocity: 0.,
                    selecting: false,
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
    pub(crate) fn preview_attachments(&mut self) { self.root.attachments.show = true; }
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
        self.root.tooltips.target = Info::Connection;
        self.root.tooltips.info.pinned = true;
        self.root.tooltips.info.suppressed = false;
        self.root.tooltips.info.progress = 1.;
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
            if self.root.attachments.show && (self.ui.mobile || size.0 as f32 / scale < 1000.) {
                self.cancel_preedit();
                self.ui.focus = None;
            }
            self.root.sidebar.projects.revealed.clear();
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
        if self.ui_event(ui::Event::Hover(point)) { return; }
        let enabled = self.root.dialog.is_none() && self.root.viewer.is_none() && self.root.menu.is_none();
        if enabled
            && let Some((rect, target)) =
                point.and_then(|p| self.root.legacy.info_areas.iter().find(|(r, _)| contains(*r, p)))
        {
            if !self.root.tooltips.target.same_anchor(target) { self.root.tooltips.info = Tooltip::default(); }
            self.root.tooltips.target = target.clone();
            self.root.tooltips.info.region = *rect;
        }
        self.root.tooltips.usage
            .hover(enabled && point.is_some_and(|p| self.root.tooltips.usage.contains(p)));
        self.root.tooltips.info
            .hover(enabled && point.is_some_and(|p| self.root.tooltips.info.contains(p)));
        if enabled && point.is_some_and(|p| contains(self.root.tooltips.info.region, p)) {
            self.root.tooltips.usage.dismiss();
        } else if enabled && point.is_some_and(|p| contains(self.root.tooltips.usage.region, p)) {
            self.root.tooltips.info.dismiss();
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
        if self.root.dialog.is_none() && self.root.viewer.is_none() && self.root.menu.is_none() {
            self.ui.dirty |= self.root.legacy.hover.and_then(|p| self.section_at(p).map(|(key, _)| key))
                != point.and_then(|p| self.section_at(p).map(|(key, _)| key));
        }
        self.root.legacy.hover = point;

        self.ui.dirty |= old != new;
    }
    #[cfg(not(target_os = "android"))]
    pub fn cursor(&self) -> chad::winit::window::CursorIcon {
        use chad::winit::window::CursorIcon;
        if self.root.dialog.is_some() || self.root.viewer.is_some() || self.root.menu.is_some() || self.ui.hot.is_some() {
            return self.ui.hot.map_or(CursorIcon::Default, |(_, text)| if text { CursorIcon::Text } else { CursorIcon::Pointer });
        }
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
        if self.root.dialog.is_none()
            && self.root.viewer.is_none()
            && self.root.menu.is_none()
            && (self.root.tooltips.info.contains_card(point) || self.root.tooltips.usage.contains_card(point))
        {
            return CursorIcon::Default;
        }
        if self.root.dialog.is_none()
            && self.root.viewer.is_none()
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
        if self.root.dialog.is_none() && self.root.viewer.is_none() {
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
            && (!self.root.attachments.show || !self.ui.mobile && self.ui.size.0 as f32 / self.ui.scale >= 1000.)
            && self.root.dialog.is_none() && self.root.viewer.is_none() && self.root.code.view.is_none();
        if let Err(error) = self.controller.viewing(visible) { self.controller.report_error(error); }
        self.ui.dirty |= self.controller.poll();
        if self.services.transfers.download_identity != self.controller.identity {
            self.services.transfers.download_identity = self.controller.identity.clone();
            self.services.transfers.export_errors.clear();
        }
        self.finish_exports();
        if let Some(text)=self.controller.copied.take() && !text.is_empty() {self.services.platform.push(PlatformAction::Copy(text));self.ui.dirty=true;}

        self.ui_event(ui::Event::Tick(dt));
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
        if visible && self.root.tooltips.usage.region.width > 0. && (self.root.tooltips.usage.progress > 0. || self.root.tooltips.usage.pinned) {
            let quota = self.controller.codex_usage.content(self.controller.epoch.is_some());
            if self.controller.account.selected.as_ref().and_then(|id|self.controller.account.sessions.iter().find(|s|&s.id==id))
                .and_then(|s|s.model.as_ref()).is_some_and(|m|m.provider=="openai-codex") && !self.root.tooltips.usage.content.text.ends_with(&quota.text) {
                self.ui.dirty=true;
            }
        }
        if self.root.legacy.connection_visible && self.root.tooltips.info.progress > 0. && self.root.tooltips.info.region.width > 0.
            && let Info::CacheTtl(id) = &self.root.tooltips.target
            && let Some(session) = self.controller.account.sessions.iter().find(|s| &s.id == id)
        {
            self.ui.dirty |= self.root.tooltips.info.content != self.controller.cache_ttl(session).details();
        }
        if self.root.dialog.is_some() || self.root.viewer.is_some() {
            self.root.tooltips.usage.dismiss();
            self.root.tooltips.info.dismiss();
            self.root.legacy.autoscroll = None;
            self.root.legacy.wheel = None;
        }
        let now = Instant::now();
        let dot_color = self.controller.health.color(now);
        self.ui.dirty |= self.root.legacy.dot_color != dot_color;
        self.root.legacy.dot_color = dot_color;
        let card_visible = self.root.legacy.connection_visible
            && self.root.tooltips.target == Info::Connection
            && self.root.tooltips.info.progress > 0.
            && self.root.tooltips.info.region.width > 0.
            && self.root.dialog.is_none() && self.root.viewer.is_none() && self.root.menu.is_none();
        let counter_bucket = card_visible.then(|| self.controller.health.counter(now))
            .flatten().map(|(_, ms)| ms / COUNTER_REFRESH.as_millis());
        let next_wake = if !self.root.legacy.connection_visible || self.root.dialog.is_some() || self.root.viewer.is_some() {
            None
        } else if card_visible && counter_bucket.is_some() {
            Some(COUNTER_REFRESH)
        } else {
            self.controller.health.next_color_wake(now)
        };
        let indeterminate = self.root.legacy.connection_visible && self.root.dialog.is_none() && self.root.viewer.is_none()
            && (!self.ui.mobile || !self.root.legacy.show_chats || self.root.attachments.show)
            && (self.controller.downloads.values().any(|d|!d.status.done && d.status.total==0)
                || !self.services.transfers.saving_downloads.is_empty());
        let progress_bucket=indeterminate.then(||now.duration_since(self.services.transfers.progress_clock).as_millis()/80);
        self.ui.dirty |= self.services.transfers.progress_bucket!=progress_bucket;
        self.services.transfers.progress_bucket=progress_bucket;
        let next_wake=if indeterminate {Some(next_wake.map_or(std::time::Duration::from_millis(80),
            |duration|duration.min(std::time::Duration::from_millis(80))))} else {next_wake};
        // Quota reset / TTL text stays current when pinned, even while offline.
        // Share the existing timer; closed cards do not acquire a redraw loop.
        let timed_tooltip = self.root.legacy.connection_visible && self.root.dialog.is_none() && self.root.viewer.is_none()
            && self.root.menu.is_none()
            && (self.root.tooltips.usage.progress > 0. && self.root.tooltips.usage.region.width > 0.
                || self.root.tooltips.info.progress > 0. && self.root.tooltips.info.region.width > 0. && matches!(self.root.tooltips.target, Info::CacheTtl(_)));
        let next_wake = if timed_tooltip { Some(next_wake.map_or(std::time::Duration::from_secs(1),
            |duration| duration.min(std::time::Duration::from_secs(1)))) } else { next_wake };

        let next_wake = match (next_wake, self.root.notice.popup.remaining(now)) {
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


        if let Some(point) = self.root.legacy.pointer.as_ref().filter(|p| p.touch && !p.dragged && p.started.elapsed().as_millis() >= 450).map(|p| p.start)
            && self.root.dialog.is_none() && self.root.menu.is_none() && self.root.viewer.is_none()
            && (self.root.legacy.chat_areas.iter().any(|(r,_)| contains(*r, point))) {
            self.context_at(point);
        }
        if let Some(point) = self.root.legacy.pointer.as_ref().filter(|p| p.touch && !p.dragged && p.started.elapsed().as_millis() >= 450).map(|p| p.start)
            && self.root.dialog.is_none() && self.root.menu.is_none() && self.root.viewer.is_none()
            && let Some((_, info)) = self.root.legacy.info_areas.iter().find(|(r, info)| matches!(info, Info::Attachment(..)) && contains(*r, point)) {
            let info = info.clone();
            self.root.legacy.pointer = None;
            self.root.legacy.ripple = None;
            self.root.legacy.selecting = false;
            self.activate(Action::Info(info));
        }
        let waiting_hold = self.root.legacy.pointer.as_ref().is_some_and(|p| p.touch && !p.dragged && p.started.elapsed().as_millis() < 450)
            && self.root.dialog.is_none() && self.root.menu.is_none() && self.root.viewer.is_none();
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
        dirty || self.root.legacy.velocity.abs() > 4. || waiting_hold || self.ui.capture.is_some_and(|c| c.touch && !c.dragged && c.started.elapsed().as_millis() < 450)
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
        if self.ui_event(ui::Event::Back) { return; }
        if self.root.tooltips.info.pinned
            || self.root.tooltips.info.progress > 0.
            || self.root.tooltips.usage.pinned
            || self.root.tooltips.usage.progress > 0.
        {
            self.root.tooltips.info.dismiss();
            self.root.tooltips.usage.dismiss();
            self.ui.dirty = true;
            return;
        }

        self.ui.focus = None;
        self.root.legacy.navigation.download = None;
        if self.root.code.view.is_some() {
            self.code_back();
        } else if self.root.attachments.show {
            self.root.attachments.show = false;
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
        if self.ui_event(ui::Event::Middle { pressed, point }) { return; }
        if self.root.dialog.is_some() { return; }
        if pressed {
            if self.cancel_autoscroll() {
                return;
            }
            if self.root.dialog.is_some()
                || self.root.viewer.is_some()
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
        if self.ui_event(ui::Event::Wheel { amount, horizontal, point }) { return; }

        self.root.menu = None;


        self.root.tooltips.usage.dismiss();
        self.root.tooltips.info.dismiss();
        self.cancel_autoscroll();
        self.root.legacy.expansion_pin = None;
        self.root.legacy.history_attempt = None;
        self.root.legacy.velocity = 0.;

        if self.root.dialog.is_none() {
            let lane = if horizontal { Lane::Horizontal } else { Lane::Transcript };
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
        if self.root.dialog.is_some() || self.root.viewer.is_some() || !near {
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
        if self.ui_event(ui::Event::Down { pointer: id, point, touch }) { return; }

        if self.root.dialog.is_none()
            && self.root.viewer.is_none()
            && self.root.menu.is_none()
            && (self.root.tooltips.info.contains_card(point) || self.root.tooltips.usage.contains_card(point))
        {
            // Read-only cards must not activate the list/message behind them.
            self.ui.dirty = true;
            return;
        }
        if !self.root.tooltips.usage.contains(point) {
            self.root.tooltips.usage.dismiss();
        }
        if !self.root.tooltips.info.contains(point) {
            self.root.tooltips.info.dismiss();
        }
        self.root.legacy.wheel = None;
        self.root.legacy.expansion_pin = None;
        self.root.legacy.history_attempt = None;
        self.root.legacy.velocity = 0.;


        self.root.legacy.ripple = None;
        if self.root.legacy.pointer.is_some() {

            return;
        }
        self.root.legacy.selecting = false;

        if self.root.dialog.is_none()
            && self.root.viewer.is_none()
            && self.root.menu.is_none()
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
        if !touch && self.root.viewer.is_none() {
            if self.root.dialog.is_none()
                && self.root.menu.is_none()
                && contains(self.root.legacy.transcript, point)
                && let Some(caret) = self.services.renderer.nearest_text(point)
            {
                self.services.renderer.begin_selection(caret);
                self.root.legacy.selecting = true;
                self.ui.focus = None;
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
        self.root.legacy.ripple = if self.root.dialog.is_none() && self.root.viewer.is_none() && self.root.menu.is_none() {
            self.section_at(point).map(|(key, rect)| Ripple::new(key.to_owned(), rect, point))
        } else { None };
        self.ui.dirty = true;
    }
    pub fn motion(&mut self, id: u64, point: Vec2) {
        if self.ui_event(ui::Event::Move { pointer: id, point }) { return; }
        let Some(p) = &mut self.root.legacy.pointer else {
            return;
        };

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

        let dy = point.y - p.last.y;
        let dx = point.x - p.last.x;
        p.dragged |= (point.x - p.start.x).abs() + (point.y - p.start.y).abs() > 7. * self.ui.scale;
        if p.dragged {


            if self.root.dialog.is_none() {
                if contains(self.root.legacy.transcript, p.start)
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
        if self.ui_event(ui::Event::Up { pointer: id, point }) { return; }

        if self.root.legacy.pointer.as_ref().is_none_or(|p| p.id != id) {
            return;
        }
        let p = self.root.legacy.pointer.take().unwrap();
        self.root.legacy.scroll_drag = None;
        if !p.dragged {
            if p.touch && p.started.elapsed().as_millis() > 450
                && self.root.legacy.info_areas.iter().any(|(r, info)| matches!(info, Info::Attachment(..))
                    && contains(*r, point) && contains(*r, p.start)) {
                let info = self.root.legacy.info_areas.iter().find(|(r, info)| matches!(info, Info::Attachment(..))
                    && contains(*r, point) && contains(*r, p.start)).unwrap().1.clone();
                self.activate(Action::Info(info));
            } else if p.touch
                && p.started.elapsed().as_millis() > 450
                && self.root.menu.is_none()
                && (contains(self.root.legacy.transcript, point)
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
                let field = matches!(hit.action, Action::Focus(_));
                self.activate(hit.action.clone());
                if p.touch && field {

                    if p.started.elapsed().as_millis() >= 450 {
                        if let Some(editor) = self.editor() { editor.select_word(); }
                        self.services.platform.push(PlatformAction::InputMenu);
                    }
                }
            } else if self.root.menu.is_some() {
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

        if p.at.elapsed().as_millis() > 150 {
            self.root.legacy.velocity = 0.;


        }
        let result = self.save();
        self.report(result);
    }
    pub fn cancel_pointer(&mut self) {
        self.with_ui(|root, cx| root.handle_event(&ui::Event::Cancel, cx));
        self.ui.cancel();
        if let Some(code)=&mut self.root.code.view { code.drag_anchor=None; }
        self.root.menu = None;
        self.root.tooltips.usage.dismiss();
        self.root.tooltips.info.dismiss();
        self.ui.dirty = true;
        self.root.legacy.hover = None;
        self.root.legacy.autoscroll = None;
        self.root.legacy.wheel = None;
        self.root.legacy.scroll_drag = None;
        self.root.legacy.expansion_pin = None;
        self.root.legacy.pointer = None;
        self.root.legacy.ripple = None;

        self.root.legacy.velocity = 0.;


        self.root.legacy.selecting = false;

    }




    pub fn cancel_preedit(&mut self) {
        if let Some(e) = self.editor() && e.composing() {
            e.preedit(String::new(), None);
            self.ui.dirty = true;
        }
    }


    pub fn ime_rect(&self)->Option<Rect>{self.root.editor_ref(self.ui.focus)?.editor.ime_rect(&self.services.renderer.text)}
    pub fn composing(&self)->bool{self.root.editor_ref(self.ui.focus).is_some_and(|f|f.editor.composing())}
    fn editor(&mut self)->Option<&mut Editor>{self.root.editor(self.ui.focus).map(|f|&mut f.editor)}
    fn edited(&mut self){self.with_ui(|root,cx|{if cx.ui.focus==Some(root.composer.field.control.target){root.composer.edited(cx);}else if root.code.view.as_ref().and_then(|v|v.search.as_ref()).is_some_and(|f|Some(f.control.target)==cx.ui.focus){root.code.code_query(cx);}});}
    pub fn input(&mut self, value: &str) {
        if self.ui_event(ui::Event::Text(value)) { return; }
        if self.editor().is_some_and(|e| e.replace(value)) { self.edited(); }
        self.ui.dirty = true;
    }
    #[cfg(not(target_os = "android"))]
    pub fn can_paste_files(&mut self, token: u64) -> bool {
        let allowed = self.ui.paste.as_ref().is_some_and(|edit| edit.matches(token, &self.controller)
            && edit.target==self.root.composer.field.control.target && self.root.dialog.is_none());
        if allowed { self.ui.paste = None; }
        allowed
    }
    pub fn paste(&mut self, token: u64, value: String) {
        let Some(edit) = self.ui.paste.as_ref().filter(|edit| edit.matches(token, &self.controller)) else { return; };
        let target = edit.target; self.ui.paste = None;
        self.ui_event(ui::Event::Paste {target,text:&value});
    }

    #[cfg(not(target_os = "android"))]
    pub fn preedit(&mut self, text: String, cursor: Option<(usize, usize)>) {
        if self.ui_event(ui::Event::Preedit(&text, cursor)) { return; }
        if let Some(e) = self.editor() {
            e.preedit(text, cursor);
        }
        self.ui.dirty = true;
    }
    pub fn key(&mut self, key: &str, ctrl: bool, shift: bool) {
        if self.ui_event(ui::Event::Key { key, ctrl, shift }) { return; }
        // Composition belongs to the IME. Enter must not send the draft, and
        // Escape must not abort the agent, while candidate text is active.
        if self.composing() {
            if key == "Escape" {
                if let Some(e) = self.editor() { e.preedit(String::new(), None); }
                self.ui.dirty = true;
            }
            return;
        }

        if key == "Escape"
            && (self.root.tooltips.usage.pinned
                || self.root.tooltips.usage.progress > 0.
                || self.root.tooltips.info.pinned
                || self.root.tooltips.info.progress > 0.)
        {
            self.root.tooltips.usage.dismiss();
            self.root.tooltips.info.dismiss();
            self.ui.dirty = true;
            return;
        }
        self.root.legacy.expansion_pin = None;
        self.root.legacy.wheel = None;
        if key == "Escape" {
            if self.root.dialog.is_some() || self.root.viewer.is_some() || self.root.attachments.show {
                self.back();
            } else {
                self.activate(Action::Abort);
            }
            return;
        }

        if ctrl && (key.eq_ignore_ascii_case("c") || key.eq_ignore_ascii_case("x")) {
            if self.ui.focus.is_none() {
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
        if key == "Enter" && !shift && !self.ui.mobile && self.ui.focus == Some(self.root.composer.field.control.target) {
            self.activate(Action::Send);
            return;
        }
    }
    fn activate(&mut self, action: Action) {
        let result = self.apply(action);
        self.sync_navigation();
        self.report(result);
    }
    fn apply(&mut self, action: Action) -> Result<()> {

        self.ui.paste = None;
        self.cancel_preedit();
        if let Action::MoveMenu(session) = &action {
            if let Some(menu) = &self.root.menu { self.ui.requests.push_back(ui::Request::MoveMenu { owner: menu.id, session: session.clone() }); self.finish_ui_requests()?; }
            return Ok(());
        }
        if matches!(action, Action::ContextBack) { self.ui_event(ui::Event::Key {key:"ArrowLeft",ctrl:false,shift:false}); return Ok(()); }
        if matches!(action, Action::Noop) { return Ok(()); }
        self.root.menu = None;
        if matches!(action, Action::Select(_) | Action::SelectProject(_) | Action::New | Action::Tail | Action::Attachments) {
            self.root.legacy.navigation.download = None;
        }
        let selected = self.controller.account.selected.clone();
        match action {
            Action::SelectProject(id) => self.navigate_project(&id)?,
            Action::NewProject => self.open_ui(ui::DialogSpec::Topic(ui::TopicEdit::New))?,
            Action::RenameProject(id) => self.open_ui(ui::DialogSpec::Topic(ui::TopicEdit::Rename(id)))?,
            Action::ProjectPrompt(id) => self.open_ui(ui::DialogSpec::Topic(ui::TopicEdit::Prompt(id)))?,
            Action::DeleteProject(id) => self.open_ui(ui::DialogSpec::Topic(ui::TopicEdit::Delete(id)))?,
            Action::MoveChat(session_id, project_id) => {
                self.controller.request(ClientCommand::MoveSession { session_id, project_id })?;
            }
            Action::MoveMenu(_) | Action::ContextBack | Action::Noop => {}
            Action::Select(id) => self.navigate_chat(&id)?,
            Action::Info(target) => {
                self.root.tooltips.usage.dismiss();
                if !self.root.tooltips.target.same_anchor(&target) { self.root.tooltips.info = Tooltip::default(); }
                self.root.tooltips.target = target;
                if let Some((rect, _)) = self
                    .root.legacy.info_areas
                    .iter()
                    .find(|(_, target)| *target == self.root.tooltips.target)
                {
                    self.root.tooltips.info.region = *rect;
                }
                self.root.tooltips.info.pinned = !self.root.tooltips.info.pinned;
                self.root.tooltips.info.suppressed = !self.root.tooltips.info.pinned;
            }
            Action::Usage => {
                self.root.tooltips.info.dismiss();
                self.root.tooltips.usage.pinned = !self.root.tooltips.usage.pinned;
                self.root.tooltips.usage.suppressed = !self.root.tooltips.usage.pinned;
            }
            Action::New => {
                self.save()?;
                self.controller.new_chat()?;
                self.root.legacy.show_chats = false;
                self.ui.focus=Some(self.root.composer.field.control.target);
            }
            Action::RetryCreate => self.controller.retry_create_manually()?,
            Action::Back => self.back(),
            Action::Files | Action::FileOpen(..) | Action::FileUp | Action::FileClose | Action::FileFind | Action::FileFindHere | Action::FileClear | Action::FileCopy | Action::FilePage(_) => self.code_action(action)?,
            Action::Attachments => {
                self.close_code();
                self.save()?;
                self.cancel_pointer();
                self.ui.focus = None;
                self.root.attachments.show = !self.root.attachments.show;
                self.root.legacy.show_chats = false;
                self.root.legacy.history_attempt = None;
            }
            Action::History => {
                self.root.legacy.history_attempt = None;
                if let Some(session) = selected { self.history_near_edge(&session, true); }
            }
            Action::ModelSettings => { self.open_ui(ui::DialogSpec::Models)?; }


            Action::ChooseModel(session, slug) => self.controller.choose_model(&session, &slug)?,
            Action::CopyRecoveredDraft(id)=>{self.controller.copy_missing_draft(&id)?;self.replace_composer(self.controller.selected().map(|c|c.local.draft.clone()).unwrap_or_default());}
            Action::ForgetRecovered(id) => { self.open_ui(ui::DialogSpec::Operation(ui::Operation::ForgetRecovered(id)))?; }
            Action::ReviewRestore(id) => { self.open_ui(ui::DialogSpec::Operation(ui::Operation::Review(id)))?; }
            Action::Outbox(page) => { self.open_ui(ui::DialogSpec::Operation(ui::Operation::Outbox(page)))?; }
            Action::InspectControl(id) => { self.open_ui(ui::DialogSpec::Operation(ui::Operation::Inspect(id)))?; }


            Action::ForgetControl(id) => { self.open_ui(ui::DialogSpec::Operation(ui::Operation::ForgetControl(id)))?; }
            Action::Settings => self.open_ui(ui::DialogSpec::Connection)?,
            Action::Focus(field) => {
                let target=if field.is_none(){Some(self.root.composer.field.control.target)}else{self.root.code.view.as_ref().and_then(|v|v.search.as_ref()).map(|f|f.control.target)};
                self.ui.focus=target;
                if self.ui.mobile && let Some(target)=target && let Some(field)=self.root.editor_ref(Some(target)){let id=field.editor.native_id();self.with_ui(|_,cx|cx.focus_native(target,id));}
            }
            Action::Confirm => { self.ui_event(ui::Event::Submit); }
            Action::CancelModal => { self.ui_event(ui::Event::Back); }
            Action::RefreshCatalog => { self.open_ui(ui::DialogSpec::Operation(ui::Operation::Refresh))?; }
            Action::DaemonSettings => { self.open_ui(ui::DialogSpec::Daemon)?; }




            Action::AgentSetting(session, command) => { self.open_ui(ui::DialogSpec::Operation(ui::Operation::Agent(session, command)))?; }
            Action::AgentCommand(session, text) => {
                self.controller.ensure_chat(&session)?;
                self.controller.control(ClientCommand::Prompt {
                    session_id: session,
                    text,
                })?;
                
                self.ui.focus = None;
            }
            Action::Rename(id) => { self.open_ui(ui::DialogSpec::Operation(ui::Operation::Rename(id)))?; }
            Action::Delete(id) => { self.open_ui(ui::DialogSpec::Operation(ui::Operation::Delete(id)))?; }
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
                if let Some(code)=&self.root.code.view {anyhow::ensure!(code.selection.is_some() && code.error.is_none(), "Select current lines again before sending this code comment");}
                if self.root.code.view.is_some() {self.code_reference(false);}
                self.controller.send_prompt()?;
                if let Some(code)=&mut self.root.code.view { code.sent(); }
                self.replace_composer(self.controller.selected().map(|c| c.local.draft.clone()).unwrap_or_default());
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
            Action::Copy(text) => {self.controller.cancel_copy();self.services.platform.push(PlatformAction::Copy(text));}
            Action::CopyDetails(session, ids) => self.controller.copy_details(&session,ids)?,
            Action::CopySelection => {
                self.controller.cancel_copy();
                if let Some(text) = self.services.renderer.selected_text() {
                    self.services.platform.push(PlatformAction::Copy(text));
                }
            }
            Action::Link(url) => { self.open_ui(ui::DialogSpec::Operation(ui::Operation::Link(url)))?; }
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
                self.replace_composer(self.controller.selected().unwrap().local.draft.clone());
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
            Action::EditQueue(id, rev, text) => { if let Some(session) = selected { self.open_ui(ui::DialogSpec::Operation(ui::Operation::Queue { session, id, revision: rev, text }))?; } }
            Action::Attachment(session, entry, name, image) => { self.with_ui(|_, cx| cx.download_attachment(&session, &entry, &name, image, false))?; }
            Action::SaveAttachment(session,entry,name) => { let image = self.controller.chats.get(&session).is_none_or(|chat| !chat.feed.events.values().any(|e| e.entry_id == entry && e.attachment.as_ref().is_some_and(|a| a.kind == AttachmentKind::File))); self.with_ui(|_, cx| cx.download_attachment(&session, &entry, &name, image, true))?; }
            Action::UseSaved(session,entry,action) => {
                if let Some(saved)=self.controller.saved_download(&session,&entry) {
                    self.services.platform.push(PlatformAction::UseDownload(saved,action,self.export_target(&session,&entry)));
                }
            }
            Action::CancelDownload(key) => {
                self.services.transfers.pending_exports.remove(&key);
                self.services.transfers.export_targets.remove(&key);
                self.controller.cancel_download(&key)?;
            },


            Action::Suggest(text) => {
                self.replace_composer(text.clone());
                self.controller.draft(text)?;
            }
        }

        self.finish_ui_requests()?;
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
            hover: self.ui.capture.map(|c|c.point).or(self.ui.hover).or_else(|| self.root.legacy.pointer.as_ref().map(|p| p.last)).or(self.root.legacy.hover),
            pressed: self.ui.capture.filter(|c| !c.dragged).map(|c|c.start).or(self
                .root.legacy.pointer
                .as_ref()
                .filter(|p| !p.dragged)
                .map(|p| p.start)),
            held: self.ui.capture.is_some() || self.root.legacy.pointer.is_some(),
        };
        let background_input =
            if self.root.dialog.is_none() && self.root.viewer.is_none() && self.root.menu.is_none() {
                input
            } else {
                Interaction::default()
            };
        let mut main = Layer::new(background_input);
        let mut body = Layer::new(background_input);
        let mut chrome = Layer::new(background_input);
        let mut overlay = Layer::new(input);
        self.root.composer.hide();self.root.quick_models.controls.begin();
        self.root.sidebar.hide();
        self.root.legacy.cards.begin();
        self.root.legacy.hits.clear();
        self.root.legacy.scrollbars.clear();
        self.root.legacy.message_areas.clear();
        self.root.legacy.detail_areas.clear();
        self.root.legacy.chat_areas.clear();



        self.root.legacy.transcript = Rect::new(0.,0.,0.,0.);

        self.root.legacy.info_areas.clear();
        self.root.tooltips.usage.region = Rect::new(0., 0., 0., 0.);
        self.root.tooltips.info.region = Rect::new(0., 0., 0., 0.);
        self.root.composer.field.editor.hide();

        self.services.renderer.clear_scenes();
        main.rect(bounds, color(0x0e141b));
        let wide = bounds.width / s >= 760.;
        let side = if wide { 300. * s } else { 0. };
        let file_side = self.root.attachments.show && !self.ui.mobile && bounds.width / s >= 1000.;
        let file_screen = self.root.attachments.show && !file_side;
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
        self.root.attachments.hide();
        if self.root.attachments.show {
            let b = if file_side { Rect::new(bounds.x + bounds.width - file_width, bounds.y, file_width, bounds.height) } else { bounds };
            self.with_ui(|root,cx| {
                root.attachments.side=file_side;
                root.attachments.visit_perframe(&mut ui::Frame {layer:&mut body,bounds:b,clip:b},cx);
            });
            interests.extend(self.root.attachments.interests.iter().cloned());
        }

        if (wide || !self.root.legacy.show_chats || self.root.attachments.show)
            && let Some(session) = self.controller.account.selected.clone()
        {
            self.controller.viewport(&session, interests);
        }
        if let Some((rect, target)) = self.root.legacy.info_areas.iter().find(|(_, target)| self.root.tooltips.target.same_anchor(target)) {
            self.root.tooltips.info.region = *rect;
            self.root.tooltips.target = target.clone();
        }
        if let Some((rect, target)) = self.root.sidebar.hints().chain(self.root.legacy.cards.hints()).chain(self.root.attachments.cards.hints()).find(|(_, target)| self.root.tooltips.target.same_anchor(target)) {
            self.root.tooltips.info.region = rect; self.root.tooltips.target = target.clone();
        }
        if self.root.tooltips.info.region.width <= 0. {
            self.root.tooltips.info.hover(false);
            self.root.tooltips.info.dismiss();
        }


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


        self.with_ui(|root, cx| root.visit_perframe(&mut ui::Frame { layer: &mut overlay, bounds, clip: bounds }, cx));
        if self.root.legacy.selecting
            && let Some(p) = &self.root.legacy.pointer
            && p.dragged
            && let Some(caret) = self.services.renderer.nearest_text(p.last)
        {
            self.ui.dirty |= self.services.renderer.extend_selection(caret);
        }
        self.with_ui(|root,cx| root.legacy.cards.finish(cx));
        self.services.renderer
            .draw(ctx, view, &[main, body, chrome, overlay]);
        // Geometry is now presented. Re-probe it through the same route and
        // finalize any visit-originated structural requests outside traversal.
        if self.root.dialog.is_some() { self.ui_event(ui::Event::Hover(self.ui.hover)); }
        else if let Err(error) = self.finish_ui_requests() { self.report(Err(error)); }
    }
    fn sidebar(&mut self, _gpu: &impl RenderContext, layer: &mut Layer, bounds: Rect) {
        self.with_ui(|root,cx| root.sidebar.visit_perframe(&mut ui::Frame {layer,bounds,clip:bounds},cx));
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
                        ui::MenuChoice::CopyDetails(
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
            let mut actions = if let Some(text) = local_text {vec![("Copy text".into(), ui::MenuChoice::Copy(text.into()))]}
                else if chat.feed.incomplete.contains(&e.id) {vec![("Fetch complete message to copy".into(),ui::MenuChoice::CopyDetails(session.into(),vec![e.id.clone()]))]} else {vec![("Copy message".into(), ui::MenuChoice::Copy(e.text.clone()))]};
            if e.phase == EventPhase::Saved {
                actions.push(("Fork here".into(), ui::MenuChoice::Fork(e.entry_id.clone())));
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
            let mut actions = vec![("Copy text".into(), ui::MenuChoice::Copy(p.text.clone()))];
            if !control && matches!(p.status, crate::store::Delivery::Rejected | crate::store::Delivery::Unconfirmed) {
                actions.push(("Restore draft".into(), ui::MenuChoice::Restore(p.request.id.clone())));
            }
            if matches!(p.request.command, ClientCommand::Prompt { .. })
                && matches!(p.status, crate::store::Delivery::Rejected | crate::store::Delivery::Unconfirmed) {
                actions.push(("Retry saved message".into(), ui::MenuChoice::RetryPending(p.request.id.clone())));
            }
            actions.push(("Dismiss".into(), ui::MenuChoice::Dismiss(p.request.id.clone())));
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
            let mut actions = if complete {vec![("Copy message".into(), ui::MenuChoice::Copy(q.text.clone()))]} else {vec![]};
            if !moving && complete && state.capabilities.iter().any(|c| c == "queue_edit") {
                actions.push((
                    "Edit".into(),
                    ui::MenuChoice::EditQueue(q.request_id.clone(), q.revision, q.text.clone()),
                ));
            }
            if !moving && state.capabilities.iter().any(|c| c == "queue_delete") {
                actions.push((
                    "Delete".into(),
                    ui::MenuChoice::Queue(QueueOperation::Delete {
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
                    ui::MenuChoice::Queue(QueueOperation::Prefix {
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
            if let Some(text) = local_text { actions = vec![("Copy text".into(), ui::MenuChoice::Copy(text.into()))]; }
            if pending.is_some() {
                // A second edit/delete using the old revision would race this
                // one. Wait for the durable receipt before offering actions.
                actions = vec![("Copy message".into(), ui::MenuChoice::Copy(editing.unwrap_or(&q.text).into()))];
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
        if self.root.code.view.is_some() { self.code_frame(ctx, layer, chrome, b); return; }
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
            Icon::Attachments, 22., Action::Attachments, self.root.attachments.show, true,
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
                .root.menu
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




    fn composer_layout(&mut self,b:Rect,session:&str)->(f32,f32,f32,f32,f32){self.with_ui(|root,cx|root.composer.layout(cx,b,session))}
    fn draw_composer(&mut self,_gpu:&impl RenderContext,layer:&mut Layer,bounds:Rect,_session:&str,in_code:bool){
        let ready=self.root.code.view.as_ref().is_some_and(|c|c.selection.is_some()&&c.error.is_none());let away=self.root.legacy.scroll+24.*self.ui.scale<self.root.legacy.max_scroll;
        self.with_ui(|root,cx|{root.composer.in_code=in_code;root.composer.code_ready=ready;root.composer.away_from_tail=away;root.composer.visit_perframe(&mut ui::Frame {layer,bounds,clip:bounds},cx);});
        self.root.tooltips.usage.region=self.root.composer.usage_rect;self.root.tooltips.usage.content=self.root.composer.usage.clone();
    }
    fn quick_models_height(&mut self,width:f32)->f32{self.with_ui(|root,cx|root.quick_models.height(cx,width))}
    fn quick_models_frame(&mut self,layer:&mut Layer,_session:&str,bounds:Rect,clip:Rect){self.with_ui(|root,cx|root.quick_models.visit_perframe(&mut ui::Frame {layer,bounds,clip},cx));}
    #[cfg(test)]
    fn composer_model_status(&mut self,layer:&mut Layer,summary:Option<&SessionSummary>,rect:Rect){self.with_ui(|root,cx|root.composer.model_status(cx,layer,summary,rect));}
    #[cfg(test)]
    fn notice_frame(&mut self, _ctx: &impl RenderContext, layer: &mut Layer, b: Rect) {
        self.with_ui(|root, cx| root.notice.visit_perframe(&mut ui::Frame { layer, bounds: b, clip: b }, cx));
    }

    pub(super) fn close_code(&mut self){self.with_ui(|root,cx|root.code.close_code(cx));}
    pub(super) fn code_tick(&mut self,dt:f32){self.ui.covered=self.root.dialog.is_some()||self.root.viewer.is_some();self.ui.composing=self.composing();self.with_ui(|root,cx|{root.code.code_tick(dt,cx);root.composer.bind(cx);});}
    pub(super) fn code_reference(&mut self,remove:bool){self.with_ui(|root,cx|{root.code.code_reference(remove,cx);root.composer.bind(cx);});}
    pub(super) fn code_back(&mut self){self.with_ui(|root,cx|root.code.code_back(cx));}
    fn code_action(&mut self,action:Action)->Result<()>{
        use code_view::Choice as C;
        let choice=match action {Action::Files=>C::Files,Action::FileClose=>C::FileClose,Action::FileOpen(p,d)=>C::FileOpen(p,d),Action::FileUp=>C::FileUp,Action::FileFindHere=>C::FileFindHere,Action::FileFind=>C::FileFind,Action::FileClear=>C::FileClear,Action::FileCopy=>C::FileCopy,Action::FilePage(n)=>C::FilePage(n),_=>return Ok(())};
        self.save()?;let result=self.with_ui(|root,cx|root.code.code_action(choice,cx));if self.root.code.view.is_some(){self.root.legacy.show_chats=false;self.root.attachments.show=false;}result
    }
    fn code_frame(&mut self,ctx:&impl RenderContext,body:&mut Layer,chrome:&mut Layer,b:Rect){
        let Some(view)=&self.root.code.view else{return;};let comments=view.search.is_none()&&view.document.is_some()&&(view.selection.is_some()||self.ui.focus==self.ui.composer);let session=self.controller.account.selected.clone().unwrap();
        let bottom=comments.then(||self.composer_layout(b,&session).4);
        self.with_ui(|root,cx|{root.code.composer_bottom=bottom;root.code.visit_perframe(&mut ui::Frame {layer:body,bounds:b,clip:b},cx);});
        if comments{self.draw_composer(ctx,chrome,b,&session,true);}
    }
    pub fn context_at(&mut self, point: Vec2) {
        if self.ui_event(ui::Event::Context(point)) { return; }
        if self.root.dialog.is_some() { return; }
        if self.root.legacy.hits.iter().rev().find(|hit| contains(hit.rect, point))
                .is_some_and(|hit| matches!(hit.action, Action::DismissNotice | Action::OpenDownloadNotice(_)))
            || self.root.dialog.is_some()
            || self.root.viewer.is_some()
            || self.root.tooltips.usage.contains_card(point)
            || self.root.tooltips.info.contains_card(point)
            || self.root.menu.is_some() && self.root.menu.as_ref().is_some_and(|menu| menu.contains(point))
        {
            return;
        }
        if let Some((_, info)) = self.root.legacy.info_areas.iter().find(|(r, info)| matches!(info, Info::Attachment(..)) && contains(*r, point)) {
            let info = info.clone();
            self.activate(Action::Info(info));
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
        self.root.tooltips.usage.dismiss();
        self.root.tooltips.info.dismiss();
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
                options.push(("Copy selection".into(), ui::MenuChoice::CopySelection));
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
                    ui::MenuChoice::AgentSetting(id.clone(), "model".into()),
                ),
                (
                    "Thinking…".into(),
                    ui::MenuChoice::AgentSetting(id.clone(), "thinking".into()),
                ),
                (
                    "Compact context…".into(),
                    ui::MenuChoice::AgentSetting(id.clone(), "compact".into()),
                ),
                (
                    "Codex priority…".into(),
                    ui::MenuChoice::AgentSetting(id.clone(), "fast".into()),
                ),
                ("Move to topic  ›".into(), ui::MenuChoice::MoveMenu(id.clone())),
                ("Rename…".into(), ui::MenuChoice::Rename(id.clone())),
                ("Review restored history…".into(),ui::MenuChoice::ReviewRestore(id.clone())),
                ("Clone chat".into(), ui::MenuChoice::Clone(id.clone())),
                ("Release idle runtime".into(), ui::MenuChoice::Sleep(id.clone())),
                ("Delete chat…".into(), ui::MenuChoice::Delete(id.clone())),
            ];
        }
        if let Some(id)=&chat && self.controller.account.missing_chats.contains(id) {
            options=vec![("Copy draft to a new chat (not sent)".into(),ui::MenuChoice::CopyRecoveredDraft(id.clone())),("Forget this local recovery…".into(),ui::MenuChoice::ForgetRecovered(id.clone()))];
        }
        let section = area.map(|a| a.key.clone());
        if !options.is_empty() {
            self.with_ui(|_, cx| {
                let menu = ui::Menu::new(point, section, chat, options, cx);
                cx.ui.requests.push_back(ui::Request::Menu(Box::new(menu)));
            });
            if let Err(error) = self.finish_ui_requests() { self.report(Err(error)); }
        }
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
    ui::controls::paint_button(renderer, layer, rect, label, s, primary,
        false);

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

#[cfg(test)]
mod test_ui;
