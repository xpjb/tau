//! Retained widgets borrow model, shared UI state and services as siblings.
//! The legacy workspace is a temporary adapter; migrated dialogs never emit its
//! semantic hit records or dispatch their controls through the global Action enum.
use super::{Controller, LegacyWorkspace, PlatformAction, Renderer};
use sanscale::{Rect, Vec2};
use std::{collections::VecDeque, sync::atomic::{AtomicU64, Ordering}};

pub(super) mod controls;
mod dialogs;
pub(super) use dialogs::{Dialog, DialogSpec, TopicEdit};
use controls::TextField;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Id(u64);
impl Id {
    pub fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Target { pub scope: Id, pub widget: Id }
#[derive(Clone, Copy)]
pub(super) struct Capture {
    pub target: Target, pub pointer: u64, pub start: Vec2, pub point: Vec2,
    pub touch: bool, pub dragged: bool,
}
#[derive(Clone, Copy)]
pub(super) enum EditorTarget { Widget(Target), Legacy(Option<usize>) }
pub(super) struct NativeEdit {
    pub token: u64, pub target: EditorTarget,
    identity: String, lineage: Option<String>, session: Option<String>,
}
impl NativeEdit {
    pub fn matches(&self, token: u64, model: &Controller) -> bool {
        self.token == token && self.identity == model.identity
            && self.lineage == model.account.source_lineage
            && (matches!(self.target, EditorTarget::Widget(_)) || self.session == model.account.selected)
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
    Native { target: Target, text: &'a str, replace: bool },
    Tick(f32), Cancel, Back, Submit,
}
pub(super) struct Frame<'a> {
    pub layer: &'a mut super::Layer,
    pub bounds: Rect,
    pub clip: Rect,
}
pub(super) struct Context<'a> {
    pub model: &'a mut Controller,
    pub ui: &'a mut UiState,
    pub services: &'a mut Services,
}
impl Context<'_> {
    pub fn report(&mut self, result: anyhow::Result<()>) {
        if let Err(error) = result { self.model.report_error(error); }
        self.ui.dirty = true;
    }
    pub fn native_edit(&mut self, target: EditorTarget, title: String, value: String, secret: bool, single_line: bool) {
        let edit = self.ui.edit_target(self.model, target);
        let token = edit.token;
        self.ui.native = Some(edit);
        self.services.platform.push(PlatformAction::Edit { token, title, value, secret, single_line });
    }
    pub fn paste(&mut self, target: EditorTarget) {
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
}

#[derive(Clone, Copy)]
pub(super) enum LegacyDialog { Models, Daemon, RefreshCatalog, Outbox }
pub(super) enum Request {
    Open(DialogSpec),
    Close(Id),
    Legacy { owner: Id, dialog: LegacyDialog },
}
pub(super) struct RootWidget {
    pub(super) legacy: LegacyWorkspace,
    pub(super) dialog: Option<Dialog>,
}
impl RootWidget {
    pub fn editor(&mut self, focus: Option<Target>) -> Option<&mut TextField> {
        self.dialog.as_mut()?.field(focus?)
    }
    pub fn editor_ref(&self, focus: Option<Target>) -> Option<&TextField> {
        self.dialog.as_ref()?.field_ref(focus?)
    }
}
impl Widget for RootWidget {
    fn handle_event(&mut self, event: &Event<'_>, cx: &mut Context<'_>) -> bool {
        let Some(dialog) = &mut self.dialog else { return false; };
        dialog.handle_event(event, cx);
        // This is an opaque input scope even in empty or disabled-control space.
        true
    }
    fn visit_perframe(&mut self, frame: &mut Frame<'_>, cx: &mut Context<'_>) {
        if let Some(dialog) = &mut self.dialog {
            let bounds = frame.bounds; let clip = frame.clip;
            frame.layer.with_clip(clip, |layer| dialog.visit_perframe(&mut Frame { layer, bounds, clip }, cx));
        }
    }
}

pub(super) struct Services {
    pub(super) renderer: Renderer,
    pub(super) platform: Vec<PlatformAction>,
}
pub(crate) struct UiState {
    pub(crate) size: (u32, u32),
    pub(crate) origin: Vec2,
    pub(crate) scale: f32,
    pub(crate) mobile: bool,
    pub(crate) window_focused: bool,
    pub(crate) dirty: bool,
    pub(super) focus: Option<Target>,
    pub(super) capture: Option<Capture>,
    pub(super) hover: Option<Vec2>,
    pub(super) hot: Option<(Target, bool)>, // bool: text cursor
    pub(super) requests: VecDeque<Request>,
    pub(super) return_to: Option<(String, Option<String>, Option<String>)>,
    pub(super) native: Option<NativeEdit>,
    pub(super) paste: Option<NativeEdit>,
}
impl UiState {
    pub fn new(size: (u32, u32), mobile: bool) -> Self {
        Self { size, origin: Vec2::new(0., 0.), scale: 1., mobile, window_focused: true, dirty: true,
            focus: None, capture: None, hover: None, hot: None, requests: VecDeque::new(), return_to: None, native: None, paste: None }
    }
    fn edit_target(&self, model: &Controller, target: EditorTarget) -> NativeEdit {
        NativeEdit { token: Id::new().0, target, identity: model.identity.clone(),
            lineage: model.account.source_lineage.clone(), session: model.account.selected.clone() }
    }
    pub fn cancel(&mut self) {
        self.capture = None; self.hot = None; self.hover = None; self.dirty = true;
        // Android's native editor is a separate focused window. Pointer cancel,
        // focus loss and viewport resize must not invalidate that live editor.
        // Its scope/focus/source checks, not the gesture lifetime, own the token.
    }
    pub(super) fn navigation_changed(&mut self) {
        if self.native.as_ref().is_some_and(|e| matches!(e.target, EditorTarget::Legacy(_))) { self.native = None; }
        if self.paste.as_ref().is_some_and(|e| matches!(e.target, EditorTarget::Legacy(_))) { self.paste = None; }
    }
    pub(super) fn detach(&mut self, scope: Id) {
        if self.focus.is_some_and(|target| target.scope == scope) { self.focus = None; }
        if self.capture.is_some_and(|capture| capture.target.scope == scope) { self.capture = None; }
        if self.hot.is_some_and(|(target, _)| target.scope == scope) { self.hot = None; }
        self.native = None; self.paste = None; self.dirty = true;
    }
}
