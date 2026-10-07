use crate::{
    net::health::{COUNTER_REFRESH, CounterTicker},
    controller::Controller,
    editor::Editor,
    icons::Icon,
    render::{Layer, Renderer, color, contains},
    scroll::Autoscroll,
    store::Store,
    tooltip::Content,
    net::Wake,
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
use tau_net::*;

mod attachments;
mod notices;
use crate::notice::DownloadTarget;
mod mobile_input;
mod ui;
mod ui_owner;
use ui::Widget;
mod code_view;

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
    PickFile { identity: String, session: String },
    OpenUrl(String),
    SaveDownload { key: String, source: PathBuf, name: String },
    UseDownload(crate::store::SavedDownload, SavedAction, DownloadTarget),
    InputMenu,
    Background,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SavedAction {
    Open,
    #[cfg(not(target_os = "android"))]
    Show,
    #[cfg(not(target_os = "android"))]
    Extract,
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

impl App {
    pub fn new(ctx: &impl RenderContext, store: Store, wake: Wake, mobile: bool) -> Result<Self> {
        let controller = Controller::new(store, wake.clone())?;
        #[cfg(target_os = "android")]
        log::info!("tau-startup stage=controller-ready");
        let needs_setup = controller.settings.url().is_err();
        let root = ui::RootWidget {
            workspace: ui::Workspace::new(&controller),
            dialog: None,
            viewer: None,
            menu: None,
            notice: ui::NoticeWidget::new(),
            tooltips: ui::TooltipHost::default(),
        };
        let services = ui::Services {
            renderer: Renderer::new(ctx).map_err(anyhow::Error::msg)?,
            platform: vec![],
            gpu: ui::Gpu::new(ctx),
            wake: CounterTicker::new(wake),
            counter_bucket: None,
            dot_color: controller.health.color(Instant::now()),
            transfers: attachments::Transfers {
                pending_exports: HashMap::new(),
                export_targets: HashMap::new(),
                saving_downloads: HashSet::new(),
                #[cfg(not(target_os = "android"))]
                extracting_downloads: HashSet::new(),
                export_errors: HashMap::new(),
                download_identity: controller.identity.clone(),
                progress_clock: Instant::now(),
                progress_bucket: None,
            },
        };
        let mut app = Self { controller, root, services, ui: ui::UiState::new(ctx.size(), mobile) };
        app.sync_navigation();
        if needs_setup {
            app.open_ui(ui::DialogSpec::Connection)?;
        }
        Ok(app)
    }
    pub fn sync_navigation(&mut self) {
        let changed = self.root.workspace.navigation.identity != self.controller.identity
            || self.root.workspace.navigation.lineage != self.controller.account.source_lineage
            || self.root.workspace.navigation.session != self.controller.account.selected;
        self.with_ui(|root, cx| root.workspace.sync_navigation(cx));
        if changed {
            self.root.tooltips.dismiss();
            self.root.menu = None;
            self.ui.menu_chat = None;
            self.ui.menu_section = None;
        }
    }
    pub fn save(&mut self) -> Result<()> {
        self.sync_navigation();
        self.with_ui(|root, cx| root.workspace.chat.transcript.save(cx))
    }
    pub fn tick(&mut self, dt: f32) -> bool {
        let started = self.trace_started();
        self.ui.covered = self.root.dialog.is_some() || self.root.viewer.is_some();
        let visible = self.with_ui(|root, cx| root.workspace.chat_visible(cx));
        if let Err(error) = self.controller.viewing(visible) {
            self.controller.report_error(error);
        }
        let poll_started = self.trace_started();
        self.ui.dirty |= self.controller.poll();
        let poll_us = poll_started.map_or(0, |s| s.elapsed().as_micros() as u64);
        if self.services.transfers.download_identity != self.controller.identity {
            self.services.transfers.download_identity = self.controller.identity.clone();
            self.services.transfers.export_errors.clear();
        }
        self.finish_exports();
        if let Some(text) = self.controller.copied.take().filter(|t| !t.is_empty()) {
            self.services.platform.push(PlatformAction::Copy(text));
            self.ui.dirty = true;
        }
        self.sync_navigation();
        self.update_widgets(dt);
        self.with_ui(|root, cx| root.timers(cx));
        let waiting = self.ui.capture.is_some_and(|c| c.touch && !c.dragged && c.started.elapsed().as_millis() < 450);
        let wants_redraw = std::mem::take(&mut self.ui.dirty) || waiting;
        if let (Some(trace), Some(started)) = (&mut self.services.renderer.trace, started) {
            trace.tick(wants_redraw, self.ui.window_focused, self.ui.visible, poll_us, started);
        }
        wants_redraw
    }
    #[cfg(windows)]
    pub fn enable_desktop_diagnostics(&mut self, ctx: &impl RenderContext) {
        if self.services.renderer.trace.is_none()
            && std::env::var_os("TAU_RENDER_TRACE").is_none_or(|value| value != "off")
        {
            self.services.renderer.trace = Some(crate::render::trace::Trace::normal(
                self.controller.store.root.join("diagnostics"), ctx.device(), ctx.format()));
        }
    }
    pub(crate) fn trace_started(&self) -> Option<crate::render::trace::Stamp> {
        self.services.renderer.trace.as_ref().map(|_| crate::render::trace::Stamp::now())
    }
    #[cfg(not(target_os = "android"))]
    pub(crate) fn trace_stage(&mut self, stage: crate::render::trace::Stage, frame: u64, started: Option<crate::render::trace::Stamp>) {
        if let (Some(trace), Some(started)) = (&mut self.services.renderer.trace, started) {
            trace.stage(stage, frame, started, (self.ui.dirty, self.ui.window_focused, self.ui.visible));
        }
    }
    #[cfg(not(target_os = "android"))]
    pub(crate) fn trace_wakes(&mut self, received: u64, posted: u64) {
        if let Some(trace) = &mut self.services.renderer.trace { trace.wakes(received, posted); }
    }
    #[cfg(not(target_os = "android"))]
    pub fn save_render_trace(&mut self) -> bool {
        let Some(trace) = &self.services.renderer.trace else { return false; };
        match trace.save() {
            Ok(path) => self.controller.notice = Some(format!("Rendering diagnostics saved to {}", path.display()).into()),
            Err(error) => self.controller.report_error(anyhow::anyhow!("Could not save rendering diagnostics: {error}")),
        }
        self.ui.dirty = true;
        true
    }
    pub fn frame(&mut self, ctx: &impl RenderContext, view: &wgpu::TextureView) {
        self.sync_navigation();
        self.services.renderer.clear_scenes();
        let bounds = Rect::new(self.ui.origin.x, self.ui.origin.y, self.ui.size.0 as f32, self.ui.size.1 as f32);
        let mut layer = Layer::default();
        layer.rect(bounds, color(0x0e141b));
        self.with_ui(|root, cx| root.visit_perframe(&mut ui::Frame { layer: &mut layer, bounds, clip: bounds }, cx));
        if let Err(error) = self.finish_ui_requests() {
            self.report(Err(error));
        }
        self.reconcile_targets();
        self.services.renderer.draw(ctx, view, &[layer]);
        if let Some(point) = self.ui.hover {
            self.ui_event(ui::Event::Hover(Some(point)));
        }
    }
    pub fn press(&mut self, id: u64, point: Vec2, touch: bool) {
        self.ui_event(ui::Event::Down { pointer: id, point, touch });
    }
    pub fn motion(&mut self, id: u64, point: Vec2) {
        self.ui_event(ui::Event::Move { pointer: id, point });
    }
    pub fn release(&mut self, id: u64, point: Vec2) {
        self.ui_event(ui::Event::Up { pointer: id, point });
    }
    pub fn key(&mut self, key: &str, ctrl: bool, shift: bool) {
        self.ui_event(ui::Event::Key { key, ctrl, shift });
    }
    pub fn input(&mut self, text: &str) {
        self.ui_event(ui::Event::Text(text));
    }
    pub fn back(&mut self) {
        self.ui_event(ui::Event::Back);
    }
    pub fn context_at(&mut self, point: Vec2) {
        self.ui_event(ui::Event::Context(point));
    }
    #[cfg(not(target_os = "android"))]
    pub fn hover(&mut self, point: Option<Vec2>) {
        self.ui_event(ui::Event::Hover(point));
    }
    #[cfg(not(target_os = "android"))]
    pub fn wheel(&mut self, amount: f32, horizontal: bool, point: Vec2) {
        self.scroll(amount, horizontal, point, false);
    }
    #[cfg(not(target_os = "android"))]
    pub(crate) fn scroll(&mut self, amount: f32, horizontal: bool, point: Vec2, precise: bool) {
        self.ui_event(ui::Event::Wheel { amount, horizontal, point, precise });
    }
    #[cfg(not(target_os = "android"))]
    pub fn middle(&mut self, pressed: bool, point: Vec2) {
        self.ui_event(ui::Event::Middle { pressed, point });
    }
    #[cfg(not(target_os = "android"))]
    pub fn preedit(&mut self, text: String, cursor: Option<(usize, usize)>) {
        self.ui_event(ui::Event::Preedit(&text, cursor));
    }
    pub fn cancel_autoscroll(&mut self) -> bool {
        self.with_ui(|root, cx| root.workspace.chat.transcript.cancel_autoscroll(cx))
    }
    pub fn cancel_pointer(&mut self) {
        if self.ui.native.is_none() { self.cancel_preedit(); }
        self.with_ui(|root, cx| {
            root.workspace.cancel(cx);
            if let Some(dialog) = &mut root.dialog { dialog.stop_scrolling(); }
            if let Some(viewer) = &mut root.viewer { viewer.cancel_pointer(); }
            root.tooltips.dismiss();
        });
        self.root.menu = None;
        self.ui.cancel();
    }
    #[cfg(not(target_os = "android"))]
    pub fn cursor(&self) -> chad::winit::window::CursorIcon {
        use chad::winit::window::CursorIcon as C;
        if let Some(auto) = &self.root.workspace.chat.transcript.autoscroll {
            return if auto.speed(self.ui.scale) < 0. {
                C::NResize
            } else if auto.speed(self.ui.scale) > 0. {
                C::SResize
            } else {
                C::NsResize
            };
        }
        self.ui.hot.map_or(C::Default, |(_, cursor)| match cursor {
            ui::Cursor::Default => C::Default, ui::Cursor::Pointer => C::Pointer, ui::Cursor::Text => C::Text,
        })
    }
    pub(super) fn open_download_notice(&mut self, target: DownloadTarget) -> Result<()> {
        self.with_ui(|root, cx| root.workspace.open_download(target, cx))?;
        self.close_ui();
        self.root.viewer = None;
        Ok(())
    }
    pub(crate) fn preview_attachments(&mut self) {
        self.root.workspace.attachments.show = true;
    }
    #[cfg(not(target_os = "android"))]
    pub fn preview_connection(&mut self, preview: ConnectionPreview) {
        use std::time::Duration;
        let now = Instant::now();
        self.controller.health = crate::net::health::Health::default();
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
        self.ui.visible = visible;
        if !visible {
            self.services.wake.sync(None);
            self.services.counter_bucket = None;
            self.with_ui(|root, cx| root.workspace.chat.code.code_tick(0., cx));
        }
    }
    pub fn resize(&mut self, size: (u32, u32), scale: f32, origin: Vec2) {
        if self.ui.size != size || self.ui.scale != scale || self.ui.origin != origin {
            self.cancel_pointer();
            if self.root.workspace.attachments.show && (self.ui.mobile || size.0 as f32 / scale < 1000.) {
                self.cancel_preedit();
                self.ui.focus = None;
            }
            self.root.workspace.sidebar.projects.revealed.clear();
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
    pub fn actions(&mut self) -> Vec<PlatformAction> {
        std::mem::take(&mut self.services.platform)
    }
    pub fn report(&mut self, result: Result<()>) {
        if let Err(e) = result {
            self.controller.report_error(e);
        }
        self.ui.dirty = true;
    }
    pub fn cancel_preedit(&mut self) {
        if let Some(e) = self.editor()
            && e.composing()
        {
            e.preedit(String::new(), None);
            self.ui.dirty = true;
        }
    }
    pub fn ime_rect(&self) -> Option<Rect> {
        self.root.editor_ref(self.ui.focus)?.editor.ime_rect(&self.services.renderer.text)
    }
    pub fn composing(&self) -> bool {
        self.root.editor_ref(self.ui.focus).is_some_and(|f| f.editor.composing())
    }
    fn editor(&mut self) -> Option<&mut Editor> {
        self.root.editor(self.ui.focus).map(|f| &mut f.editor)
    }
    fn edited(&mut self) {
        self.with_ui(|root, cx| {
            if cx.ui.focus == Some(root.workspace.chat.composer.field.control.target) {
                root.workspace.chat.composer.edited(cx);
            } else if root
                .workspace
                .chat
                .code
                .view
                .as_ref()
                .and_then(|v| v.search.as_ref())
                .is_some_and(|f| Some(f.control.target) == cx.ui.focus)
            {
                root.workspace.chat.code.code_query(cx);
            }
        });
    }
    #[cfg(not(target_os = "android"))]
    pub fn can_paste_files(&mut self, token: u64) -> bool {
        let allowed = self.ui.paste.as_ref().is_some_and(|edit| {
            edit.matches(token, &self.controller)
                && edit.target == self.root.workspace.chat.composer.field.control.target
                && self.root.dialog.is_none()
        });
        if allowed {
            self.ui.paste = None;
        }
        allowed
    }
    pub fn paste(&mut self, token: u64, value: String) {
        let Some(edit) = self.ui.paste.as_ref().filter(|edit| edit.matches(token, &self.controller)) else {
            return;
        };
        let target = edit.target;
        self.ui.paste = None;
        self.ui_event(ui::Event::Paste { target, text: &value });
    }
}
pub(crate) fn literal(text: &str) -> String {
    text.chars().flat_map(|c| if "\\`*_{}[]<>()#+-.!|~>".contains(c) { vec!['\\', c] } else { vec![c] }).collect()
}
pub(crate) fn code(text: &str) -> String {
    let n = text.split(|c| c != '`').map(str::len).max().unwrap_or(0).max(2) + 1;
    let fence = "`".repeat(n);
    format!("{fence}\n{text}\n{fence}")
}

fn context_usage_display(usage: Option<ContextUsage>) -> (Option<f32>, Content) {
    use crate::tooltip::{ACCENT, DANGER, INK, WARNING};
    let mut content = Content::default();
    content.strong("Context", INK);
    let mut ratio = None;
    match usage {
        Some(ContextUsage { tokens: Some(used), context_window: Some(capacity), .. }) if capacity > 0 => {
            let used_ratio = used as f32 / capacity as f32;
            ratio = Some(used_ratio);
            let tint = if used_ratio >= 0.95 {
                DANGER
            } else if used_ratio >= 0.8 {
                WARNING
            } else {
                ACCENT
            };
            content.dim(" · ").strong(format!("~{:.0}%", used_ratio * 100.), tint).dim(" used").line().dim(format!(
                "{} of {} tokens",
                count(used),
                count(capacity)
            ));
        }
        Some(ContextUsage { tokens: Some(used), .. }) => {
            content
                .dim(" · ")
                .strong(format!("~{}", count(used)), ACCENT)
                .dim(" tokens used")
                .line()
                .dim("Capacity unknown");
        }
        Some(ContextUsage { context_window: Some(capacity), .. }) if capacity > 0 => {
            content.dim(" · usage unknown").line().dim(format!("Capacity: {} tokens", count(capacity)));
        }
        Some(_) => {
            content.dim(" · usage unknown").line().dim("Capacity unknown");
        }
        None => {
            content.dim(" · usage unavailable");
        }
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
#[path = "../tests/unit/app/repaint.rs"]
mod repaint_tests;
#[cfg(all(test, not(target_os = "android")))]
#[path = "../tests/unit/app/connection.rs"]
mod connection_tests;
#[cfg(all(test, not(target_os = "android")))]
#[path = "../tests/unit/app/control.rs"]
mod control_tests;
#[cfg(all(test, not(target_os = "android")))]
#[path = "../tests/unit/app/editor.rs"]
mod editor_tests;
#[cfg(all(test, not(target_os = "android")))]
#[path = "../tests/unit/app/navigation.rs"]
mod navigation_tests;
#[cfg(all(test, not(target_os = "android")))]
#[path = "../tests/unit/app/project.rs"]
mod project_tests;
#[cfg(all(test, not(target_os = "android")))]
#[path = "../tests/unit/app/thinking.rs"]
mod thinking_tests;
#[cfg(all(test, not(target_os = "android")))]
#[path = "../tests/unit/app/startup.rs"]
mod startup_tests;
#[cfg(all(test, not(target_os = "android")))]
#[path = "../tests/unit/app/tooltip.rs"]
mod tooltip_tests;
#[cfg(all(test, not(target_os = "android")))]
#[path = "../tests/unit/app/viewer.rs"]
mod viewer_tests;

#[cfg(test)]
#[path = "../tests/unit/app_usage.rs"]
mod usage_tests;

#[cfg(all(test, not(target_os = "android")))]
#[path = "../tests/unit/app/download_render.rs"]
mod download_render_tests;

#[cfg(all(test, not(target_os = "android")))]
#[path = "../tests/unit/app/download_interaction.rs"]
mod download_interaction_tests;

#[cfg(all(test, not(target_os = "android")))]
#[path = "../tests/unit/app/composer_status.rs"]
mod composer_status_tests;

#[cfg(test)]
use crate::tooltip::Tooltip;

#[cfg(all(test, not(target_os = "android")))]
#[path = "../tests/unit/app/transcript.rs"]
mod transcript_tests;
