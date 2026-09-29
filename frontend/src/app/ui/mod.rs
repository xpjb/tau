//! Retained widgets borrow model, shared UI state and services as siblings.
//! Input and layout belong to the same concrete owners; structural changes are
//! applied after traversal through the sibling-borrowing owner boundary.
use super::{Controller, PlatformAction, Renderer};
use sanscale::{Rect, Vec2};
use std::{
    collections::VecDeque,
    sync::atomic::{AtomicU64, Ordering},
};

mod attachments;
pub(super) mod controls;
mod dialogs;
mod menu;
mod notice;
pub(super) mod scroll;
mod settings;
pub(super) mod sidebar;
mod tooltips;
mod viewer;
pub(super) use attachments::AttachmentBrowser;
#[cfg(test)]
pub(super) use attachments::CardChoice;
pub(super) use menu::{Choice as MenuChoice, Menu};
pub(super) use notice::NoticeWidget;
pub(super) use sidebar::Sidebar;
pub(super) use tooltips::TooltipHost;
pub(super) use viewer::{ImageSpec, ImageViewer};
pub(super) mod header;
pub(super) mod message_row;
mod timers;
pub(super) mod transcript;
mod workspace;
pub(super) use workspace::Workspace;
pub(super) mod composer;
pub(super) use composer::Composer;
mod operations;
use controls::TextField;
pub(super) use dialogs::{Dialog, DialogSpec, TopicEdit};
pub(super) use operations::Operation;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Id(u64);
impl Id {
    pub fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Target {
    pub scope: Id,
    pub widget: Id,
}
#[derive(Clone, Copy)]
pub(super) struct Capture {
    pub target: Target,
    pub pointer: u64,
    pub start: Vec2,
    pub point: Vec2,
    pub touch: bool,
    pub dragged: bool,
    pub claimed: bool,
    pub started: std::time::Instant,
}
pub(super) struct NativeEdit {
    pub token: u64,
    pub target: Target,
    identity: String,
    lineage: Option<String>,
    session: Option<String>,
    session_bound: bool,
}
impl NativeEdit {
    pub fn matches(&self, token: u64, model: &Controller) -> bool {
        self.token == token
            && self.identity == model.identity
            && self.lineage == model.account.source_lineage
            && (!self.session_bound || self.session == model.account.selected)
    }
}

pub(super) enum Event<'a> {
    Down { pointer: u64, point: Vec2, touch: bool },
    Move { pointer: u64, point: Vec2 },
    Up { pointer: u64, point: Vec2 },
    Hover(Option<Vec2>),
    Wheel { amount: f32, horizontal: bool, point: Vec2 },
    Key { key: &'a str, ctrl: bool, shift: bool },
    Text(&'a str),
    Preedit(&'a str, Option<(usize, usize)>),
    Paste { target: Target, text: &'a str },
    Tick(f32),
    Cancel,
    Back,
    Submit,
    Context(Vec2),
    Middle { pressed: bool, point: Vec2 },
}
impl Event<'_> {
    fn broadcast(&self) -> bool {
        matches!(self, Self::Tick(_) | Self::Cancel)
    }
}
pub(super) struct Frame<'a> {
    pub layer: &'a mut super::Layer,
    pub bounds: Rect,
    pub clip: Rect,
}
impl Frame<'_> {
    /// Every child inherits the same clip for placement, painting and hit testing.
    pub fn visit(&mut self, bounds: Rect, child: &mut dyn Widget, cx: &mut Context<'_>) {
        let clip = crate::render::intersect(self.clip, bounds);
        self.layer.with_clip(clip, |layer| child.visit_perframe(&mut Frame { layer, bounds, clip }, cx));
    }
}
pub(super) struct Context<'a> {
    pub model: &'a mut Controller,
    pub ui: &'a mut UiState,
    pub services: &'a mut Services,
}
impl Context<'_> {
    pub fn report(&mut self, result: anyhow::Result<()>) {
        if let Err(error) = result {
            self.model.report_error(error);
        }
        self.ui.dirty = true;
    }
    pub fn focus_native(&mut self, target: Target, id: u64) {
        let mut edit = self.ui.edit_target(self.model, target);
        edit.token = id;
        self.ui.native = Some(edit);
        self.ui.input_request += 1;
    }
    pub fn paste(&mut self, target: Target) {
        let edit = self.ui.edit_target(self.model, target);
        let token = edit.token;
        self.ui.paste = Some(edit);
        self.services.platform.push(PlatformAction::Paste { token });
    }
}
pub(super) trait Widget {
    /// Consumption is independent of whether text/model data changed.
    fn handle_event(&mut self, event: &Event<'_>, cx: &mut Context<'_>) -> bool;
    fn visit_perframe(&mut self, frame: &mut Frame<'_>, cx: &mut Context<'_>);
    /// Query the actual mounted owner, never a cached ancestor/scope registry.
    fn owns(&self, _target: Target, _model: &Controller, _ui: &UiState) -> bool {
        false
    }
    fn dispatch(&mut self, event: &Event<'_>, cx: &mut Context<'_>) -> bool {
        if matches!(event, Event::Move { .. } | Event::Up { .. })
            && let Some(capture) = cx.ui.capture
            && !self.owns(capture.target, cx.model, cx.ui)
        {
            return false;
        }
        self.handle_event(event, cx)
    }
}
/// Callers supply their children in paint order. Lifecycle broadcasts are not input.
fn dispatch_children<'a>(
    children: impl DoubleEndedIterator<Item = &'a mut dyn Widget>,
    event: &Event<'_>,
    cx: &mut Context<'_>,
) -> bool {
    let mut handled = false;
    for child in children.rev() {
        handled |= child.dispatch(event, cx);
        if handled && !event.broadcast() {
            break;
        }
    }
    handled
}

pub(super) enum Request {
    Open(DialogSpec),
    View(ImageSpec),
    CloseViewer(Id),
    Download(crate::notice::DownloadTarget),
    Back,
    Files,
    Attachments(bool),
    Select(String),
    Project(String),
    NewChat,
    Tip { info: super::Info, rect: Rect },
    Close(Id),
    Menu(Box<Menu>),
    CloseMenu(Id),
    MoveMenu { owner: Id, session: String },
    Replace { owner: Id, spec: DialogSpec },
}
pub(super) struct RootWidget {
    pub(super) workspace: Workspace,
    pub(super) dialog: Option<Dialog>,
    pub(super) viewer: Option<ImageViewer>,
    pub(super) menu: Option<Box<Menu>>,
    pub(super) notice: NoticeWidget,
    pub(super) tooltips: TooltipHost,
}
#[derive(Clone, Copy)]
enum Overlay {
    Dialog,
    Viewer,
    Menu,
}
impl RootWidget {
    fn overlay(&self) -> Option<Overlay> {
        if self.dialog.is_some() {
            Some(Overlay::Dialog)
        } else if self.viewer.is_some() {
            Some(Overlay::Viewer)
        } else if self.menu.is_some() {
            Some(Overlay::Menu)
        } else {
            None
        }
    }
    fn overlay_widget(&self, overlay: Overlay) -> &dyn Widget {
        match overlay {
            Overlay::Dialog => self.dialog.as_ref().unwrap(),
            Overlay::Viewer => self.viewer.as_ref().unwrap(),
            Overlay::Menu => self.menu.as_deref().unwrap(),
        }
    }
    fn overlay_mut(&mut self, overlay: Overlay) -> &mut dyn Widget {
        match overlay {
            Overlay::Dialog => self.dialog.as_mut().unwrap(),
            Overlay::Viewer => self.viewer.as_mut().unwrap(),
            Overlay::Menu => self.menu.as_deref_mut().unwrap(),
        }
    }

    pub fn editor(&mut self, focus: Option<Target>) -> Option<&mut TextField> {
        if let Some(dialog) = &mut self.dialog {
            return dialog.field(focus?);
        }
        if focus == Some(self.workspace.chat.composer.field.control.target) {
            return Some(&mut self.workspace.chat.composer.field);
        }
        self.workspace.chat.code.view.as_mut()?.search.as_mut().filter(|f| Some(f.control.target) == focus)
    }
    pub fn editor_ref(&self, focus: Option<Target>) -> Option<&TextField> {
        if let Some(dialog) = &self.dialog {
            return dialog.field_ref(focus?);
        }
        if focus == Some(self.workspace.chat.composer.field.control.target) {
            return Some(&self.workspace.chat.composer.field);
        }
        self.workspace.chat.code.view.as_ref()?.search.as_ref().filter(|f| Some(f.control.target) == focus)
    }
}
impl Widget for RootWidget {
    fn owns(&self, target: Target, model: &Controller, ui: &UiState) -> bool {
        if let Some(overlay) = self.overlay() {
            return self.overlay_widget(overlay).owns(target, model, ui);
        }
        self.notice.owns(target, model, ui) || self.workspace.owns(target, model, ui)
    }

    fn handle_event(&mut self, event: &Event<'_>, cx: &mut Context<'_>) -> bool {
        self.workspace.sync_navigation(cx);
        // File-interest lifetime is reconciled even when an opaque scope consumes input.
        let dt = if let Event::Tick(dt) = event { *dt } else { 0. };
        self.workspace.chat.code.code_tick(dt, cx);
        self.workspace.chat.composer.bind(cx);
        if self.dialog.is_some() || self.viewer.is_some() || self.menu.is_some() {
            // Hidden notices do not run an expired wake deadline. Their model
            // destination is unchanged and receives a fresh display on return.
            self.notice.suspend(cx);
        }
        if event.broadcast() {
            self.workspace.handle_event(event, cx);
            if let Some(dialog) = &mut self.dialog {
                dialog.handle_event(event, cx);
            }
            if let Some(viewer) = &mut self.viewer {
                viewer.handle_event(event, cx);
            }
            if let Some(menu) = &mut self.menu {
                menu.handle_event(event, cx);
            }
            let visible = self.dialog.is_none() && self.viewer.is_none() && self.menu.is_none();
            if visible {
                self.notice.handle_event(event, cx);
            }
            // Advance hidden tooltip clocks without requesting paints for invisible effects.
            let dirty = cx.ui.dirty;
            self.tooltips.handle_event(event, cx);
            if !visible {
                cx.ui.dirty = dirty;
            }
            return false;
        }
        if let Some(overlay) = self.overlay() {
            self.overlay_mut(overlay).handle_event(event, cx);
            return true; // Opaque/modal input boundary, including consumed-without-action.
        }
        if self.notice.dispatch(event, cx) || self.tooltips.dispatch(event, cx) {
            return true;
        }
        if let Event::Down { point, .. } = *event {
            if !self.tooltips.usage.contains(point) {
                self.tooltips.usage.dismiss();
            }
            if !self.tooltips.info.contains(point) {
                self.tooltips.info.dismiss();
            }
        }
        let handled = self.workspace.dispatch(event, cx);
        if std::mem::take(&mut self.workspace.chat.composer.usage_toggle) {
            self.tooltips.info.dismiss();
            self.tooltips.usage.pinned = !self.tooltips.usage.pinned;
            self.tooltips.usage.suppressed = !self.tooltips.usage.pinned;
        }
        self.tooltips.hint(cx);
        if let Event::Hover(point) = *event {
            self.tooltips.usage.hover(point.is_some_and(|p| self.tooltips.usage.contains(p)));
            self.tooltips.info.hover(point.is_some_and(|p| self.tooltips.info.contains(p)));
            if cx.ui.hint.is_some() {
                self.tooltips.usage.dismiss();
            } else if point.is_some_and(|p| crate::render::contains(self.tooltips.usage.region, p)) {
                self.tooltips.info.dismiss();
            }
        }
        handled
    }
    fn visit_perframe(&mut self, frame: &mut Frame<'_>, cx: &mut Context<'_>) {
        self.workspace.visit_perframe(frame, cx);
        self.tooltips.usage.region = self.workspace.chat.composer.usage_rect;
        self.tooltips.usage.content = self.workspace.chat.composer.usage.clone();
        if self.tooltips.usage.region.width <= 0. {
            self.tooltips.usage.dismiss();
        }
        let hint = self
            .workspace
            .hints()
            .find(|(_, target)| self.tooltips.target.same_anchor(target))
            .map(|(r, t)| (r, t.clone()));
        if let Some((rect, target)) = hint {
            self.tooltips.info.region = rect;
            self.tooltips.target = target;
        } else {
            self.tooltips.info.region = Rect::new(0., 0., 0., 0.);
            self.tooltips.info.dismiss();
        }
        self.notice.hide();
        if let Some(overlay) = self.overlay() {
            frame.visit(frame.bounds, self.overlay_mut(overlay), cx);
        } else {
            self.tooltips.visit_perframe(frame, cx);
            self.notice.visit_perframe(frame, cx);
        }
    }
}

pub(super) struct Services {
    pub(super) renderer: Renderer,
    pub(super) platform: Vec<PlatformAction>,
    pub(super) gpu: Gpu,
    pub(super) transfers: super::attachments::Transfers,
    pub(super) wake: super::CounterTicker,
    pub(super) counter_bucket: Option<u128>,
    pub(super) dot_color: u32,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Cursor {
    Default,
    Pointer,
    Text,
}
pub(crate) struct UiState {
    pub(crate) size: (u32, u32),
    pub(crate) origin: Vec2,
    pub(crate) scale: f32,
    pub(crate) mobile: bool,
    pub(crate) window_focused: bool,
    pub(super) visible: bool,
    pub(crate) dirty: bool,
    pub(super) focus: Option<Target>,
    pub(super) composer: Option<Target>,
    pub(super) search: Option<Target>,
    pub(super) covered: bool,
    pub(super) composing: bool,
    pub(super) capture: Option<Capture>,
    pub(super) hover: Option<Vec2>,
    pub(super) hint: Option<(Rect, super::Info)>,
    pub(super) menu_chat: Option<String>,
    pub(super) menu_section: Option<String>,
    pub(super) hot: Option<(Target, Cursor)>,
    pub(super) requests: VecDeque<Request>,
    pub(super) return_to: Option<(String, Option<String>, Option<String>)>,
    pub(super) native: Option<NativeEdit>,
    pub(super) input_request: u64,
    pub(super) paste: Option<NativeEdit>,
}
impl UiState {
    pub fn bounds(&self) -> Rect {
        Rect::new(self.origin.x, self.origin.y, self.size.0 as f32, self.size.1 as f32)
    }
    pub fn new(size: (u32, u32), mobile: bool) -> Self {
        Self {
            size,
            origin: Vec2::new(0., 0.),
            scale: 1.,
            mobile,
            window_focused: true,
            visible: true,
            dirty: true,
            focus: None,
            composer: None,
            search: None,
            covered: false,
            composing: false,
            capture: None,
            hover: None,
            hint: None,
            menu_chat: None,
            menu_section: None,
            hot: None,
            requests: VecDeque::new(),
            return_to: None,
            native: None,
            input_request: 0,
            paste: None,
        }
    }
    fn edit_target(&self, model: &Controller, target: Target) -> NativeEdit {
        NativeEdit {
            token: Id::new().0,
            target,
            identity: model.identity.clone(),
            lineage: model.account.source_lineage.clone(),
            session: model.account.selected.clone(),
            session_bound: Some(target) == self.composer || Some(target) == self.search,
        }
    }
    pub fn cancel(&mut self) {
        self.capture = None;
        self.hot = None;
        self.hover = None;
        self.dirty = true;
        // Viewport/IME insets cancel gestures, not a valid inline editing session.
        // Scope, buffer revision and source checks own that lifetime.
    }
    pub(super) fn navigation_changed(&mut self) {
        if self.native.as_ref().is_some_and(|e| e.session_bound) {
            self.native = None;
        }
        if self.paste.as_ref().is_some_and(|e| e.session_bound) {
            self.paste = None;
        }
    }
    pub(super) fn detach_target(&mut self, target: Target) {
        if self.focus == Some(target) {
            self.focus = None;
        }
        if self.capture.is_some_and(|c| c.target == target) {
            self.capture = None;
        }
        if self.hot.is_some_and(|(t, _)| t == target) {
            self.hot = None;
        }
        if self.native.as_ref().is_some_and(|e| e.target == target) {
            self.native = None;
        }
        if self.paste.as_ref().is_some_and(|e| e.target == target) {
            self.paste = None;
        }
    }
    pub(super) fn detach(&mut self, scope: Id) {
        if self.focus.is_some_and(|t| t.scope == scope) {
            self.focus = None;
        }
        if self.capture.is_some_and(|c| c.target.scope == scope) {
            self.capture = None;
        }
        if self.hot.is_some_and(|(t, _)| t.scope == scope) {
            self.hot = None;
        }
        if self.native.as_ref().is_some_and(|e| e.target.scope == scope) {
            self.native = None;
        }
        if self.paste.as_ref().is_some_and(|e| e.target.scope == scope) {
            self.paste = None;
        }
        self.dirty = true;
    }
}

/// GPU handles are stable services. Widgets need not borrow the host window or
/// carry it through the model to upload an icon/image.
pub(super) struct Gpu {
    device: chad::wgpu::Device,
    queue: chad::wgpu::Queue,
    format: chad::wgpu::TextureFormat,
}
impl Gpu {
    pub fn new(ctx: &impl chad::RenderContext) -> Self {
        Self { device: ctx.device().clone(), queue: ctx.queue().clone(), format: ctx.format() }
    }
}
impl chad::RenderContext for Gpu {
    fn device(&self) -> &chad::wgpu::Device {
        &self.device
    }
    fn queue(&self) -> &chad::wgpu::Queue {
        &self.queue
    }
    fn format(&self) -> chad::wgpu::TextureFormat {
        self.format
    }
    fn size(&self) -> (u32, u32) {
        (0, 0)
    }
    fn dt(&self) -> f32 {
        0.
    }
    fn elapsed(&self) -> f32 {
        0.
    }
    fn frame_index(&self) -> u64 {
        0
    }
    fn alpha(&self) -> f32 {
        0.
    }
}

#[cfg(all(test, not(target_os = "android")))]
mod lifetime_tests;
