use crate::{
    controller::Controller,
    editor::Editor,
    render::{Interaction, Layer, Renderer, color, contains},
    store::{Settings, Store},
    transport::Wake,
};
use anyhow::Result;
use chad::{RenderContext, wgpu};
use sanscale::{Rect, Vec2};
use std::{collections::HashSet, path::PathBuf, time::Instant};
use tau_protocol::*;

#[derive(Clone)]
enum Action {
    Select(String),
    New,
    Settings,
    Back,
    Menu,
    Send,
    Abort,
    History,
    Tail,
    DismissNotice,
    Focus(Option<usize>),
    Confirm,
    CancelModal,
    TitlePrompt,
    ResetTitlePrompt,
    Rename,
    Delete,
    Clone,
    Sleep,
    Fork(String),
    Copy(String),
    CopySelection,
    Link(String),
    Attach,
    RemoveFile(String),
    Toggle(Vec<String>),
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
    Extension(Option<String>, Option<bool>, bool),
}
#[derive(Clone)]
enum ModalKind {
    Settings,
    Rename,
    Delete,
    Menu,
    TitlePrompt,
    QueueEdit(String, u64),
    Extension(String, Box<ExtensionUiRequest>),
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
    key: String,
    title: String,
    source: String,
    user: bool,
    error: bool,
    actions: Vec<(String, Action)>,
    attachment: Option<(String, ChatAttachment)>,
}
struct Placed {
    key: String,
    top: f32,
    height: f32,
}
struct Pointer {
    id: u64,
    start: Vec2,
    last: Vec2,
    at: Instant,
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
    composer_session: Option<String>,
    show_chats: bool,
    waiting_title: bool,
    scroll: f32,
    max_scroll: f32,
    list_scroll: f32,
    horizontal: f32,
    max_horizontal: f32,
    transcript: Rect,
    placed: Vec<Placed>,
    pointer: Option<Pointer>,
    hover: Option<Vec2>,
    pinch: Option<(u64, Vec2)>,
    velocity: f32,
    viewer: Option<Viewer>,
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
        let composer = Editor::new(
            controller
                .selected()
                .map(|c| c.local.draft.clone())
                .unwrap_or_default(),
        );
        let show_chats = composer_session.is_none();
        Ok(Self {
            controller,
            renderer: Renderer::new(ctx).map_err(anyhow::Error::msg)?,
            size: ctx.size(),
            origin: Vec2::new(0., 0.),
            scale: 1.,
            hits: vec![],
            composer,
            focus: None,
            modal: None,
            composer_session,
            show_chats,
            waiting_title: false,
            scroll: 0.,
            max_scroll: 0.,
            list_scroll: 0.,
            horizontal: 0.,
            max_horizontal: 0.,
            transcript: Rect::new(0., 0., 0., 0.),
            placed: vec![],
            pointer: None,
            hover: None,
            pinch: None,
            velocity: 0.,
            viewer: None,
            selecting: false,
            field_selection: None,
            platform: vec![],
            mobile,
            dirty: true,
        })
    }
    pub fn resize(&mut self, size: (u32, u32), scale: f32, origin: Vec2) {
        if self.size != size || self.scale != scale || self.origin != origin {
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
        let old = self
            .hover
            .and_then(|p| self.hits.iter().rev().find(|h| contains(h.rect, p)))
            .map(|h| h.rect);
        let new = point
            .and_then(|p| self.hits.iter().rev().find(|h| contains(h.rect, p)))
            .map(|h| h.rect);
        self.hover = point;
        self.dirty |= old != new;
    }
    #[cfg(not(target_os = "android"))]
    pub fn cursor(&self) -> chad::winit::window::CursorIcon {
        use chad::winit::window::CursorIcon;
        let Some(point) = self.hover else {
            return CursorIcon::Default;
        };
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
            if self.renderer.hit_text(point).is_some() {
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
        self.dirty |= self.controller.poll();
        let selected = self.controller.account.selected.clone();
        if selected != self.composer_session {
            self.composer_session = selected;
            self.scroll = 0.;
            self.horizontal = 0.;
            self.velocity = 0.;
            self.composer = Editor::new(
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
            self.composer = Editor::new(chat.local.draft.clone());
            self.dirty = true;
        }
        if self.waiting_title
            && let Some((prompt, _)) = &self.controller.title_prompt
        {
            self.waiting_title = false;
            self.modal = Some(Modal {
                kind: ModalKind::TitlePrompt,
                title: "Shared title prompt".into(),
                fields: vec![(
                    "Exact prompt (empty is allowed)".into(),
                    Editor::new(prompt.clone()),
                    false,
                )],
                options: vec![
                    ("Default".into(), Action::ResetTitlePrompt),
                    ("Save".into(), Action::Confirm),
                    ("Cancel".into(), Action::CancelModal),
                ],
            });
            self.dirty = true;
        }
        if self.modal.is_none()
            && let Some((session, r)) = self.controller.dialogs.first().cloned()
        {
            let options = match r.method.as_str() {
                "select" => r
                    .options
                    .iter()
                    .map(|o| (o.clone(), Action::Extension(Some(o.clone()), None, false)))
                    .chain(std::iter::once((
                        "Cancel".into(),
                        Action::Extension(None, None, true),
                    )))
                    .collect(),
                "confirm" => vec![
                    ("Confirm".into(), Action::Extension(None, Some(true), false)),
                    ("Cancel".into(), Action::Extension(None, Some(false), true)),
                ],
                _ => vec![
                    ("Submit".into(), Action::Confirm),
                    ("Cancel".into(), Action::Extension(None, None, true)),
                ],
            };
            let fields = if matches!(r.method.as_str(), "input" | "editor") {
                vec![(
                    r.placeholder.clone().unwrap_or_default(),
                    Editor::new(r.prefill.clone().unwrap_or_default()),
                    false,
                )]
            } else {
                vec![]
            };
            self.modal = Some(Modal {
                title: r
                    .title
                    .clone()
                    .or(r.message.clone())
                    .unwrap_or_else(|| "Agent request".into()),
                kind: ModalKind::Extension(session, Box::new(r)),
                fields,
                options,
            });
            self.dirty = true;
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
        let dirty = self.dirty;
        self.dirty = false;
        dirty || self.velocity.abs() > 4.
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
            && let Some(chat) = self.controller.chats.get_mut(&id)
        {
            let anchor = self.placed.iter().find(|r| r.top + r.height >= self.scroll);
            chat.local.position.key = anchor.map(|r| r.key.clone());
            chat.local.position.offset = anchor.map_or(0., |r| (self.scroll - r.top) / self.scale);
            chat.local.position.follow = self.max_scroll - self.scroll < 24. * self.scale;
        }
    }
    pub fn back(&mut self) {
        self.focus = None;
        if self.viewer.take().is_some() {
        } else if self.modal.is_some() {
            self.activate(Action::CancelModal);
        } else if !self.show_chats && self.size.0 as f32 / self.scale < 840. {
            self.show_chats = true;
        } else {
            self.platform.push(PlatformAction::Background);
        }
        self.dirty = true;
    }
    #[cfg(not(target_os = "android"))]
    pub fn wheel(&mut self, amount: f32, horizontal: bool, point: Vec2) {
        if let Some(v) = &mut self.viewer {
            v.zoom = (v.zoom * (-amount * 0.002).exp()).clamp(1., 16.);
        } else if horizontal {
            self.horizontal = (self.horizontal + amount).clamp(0., self.max_horizontal);
        } else if self.show_chats
            || self.size.0 as f32 / self.scale >= 840.
                && point.x < self.origin.x + 280. * self.scale
        {
            self.list_scroll = (self.list_scroll + amount).max(0.);
        } else {
            self.scroll = (self.scroll + amount).clamp(0., self.max_scroll);
            self.velocity = 0.;
            self.remember_scroll();
        }
        self.dirty = true;
    }
    pub fn press(&mut self, id: u64, point: Vec2, touch: bool) {
        self.velocity = 0.;
        if self.pointer.is_some() {
            if self.viewer.is_some() && touch {
                self.pinch = Some((id, point));
            }
            return;
        }
        self.selecting = false;
        self.field_selection = None;
        if !touch && self.viewer.is_none() {
            if let Some(hit) = self.hits.iter().rev().find(|h| contains(h.rect, point)) {
                // Controls take priority over transcript selection beneath them.
                if let Action::Focus(field) = hit.action {
                    self.focus = Some(field);
                    self.field_selection = Some(hit.rect);
                    self.field_hit(hit.rect, point, false);
                }
            } else if self.modal.is_none()
                && let Some((key, byte)) = self.renderer.hit_text(point)
            {
                self.renderer.selection = Some((key, byte, byte));
                self.selecting = true;
                self.focus = None;
            }
        }
        self.pointer = Some(Pointer {
            id,
            start: point,
            last: point,
            at: Instant::now(),
            dragged: false,
            touch,
        });
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
        if self.selecting {
            if let Some((key, byte)) = self.renderer.hit_text(point)
                && let Some((selected, _, end)) = &mut self.renderer.selection
                && *selected == key
            {
                *end = byte;
            }
            p.dragged |=
                (point.x - p.start.x).abs() + (point.y - p.start.y).abs() > 4. * self.scale;
            self.dirty = true;
            return;
        }
        if let Some(rect) = self.field_selection {
            p.dragged = true;
            self.field_hit(rect, point, true);
            self.dirty = true;
            return;
        }
        let dy = point.y - p.last.y;
        let dx = point.x - p.last.x;
        p.dragged |= (point.x - p.start.x).abs() + (point.y - p.start.y).abs() > 7. * self.scale;
        if p.dragged {
            if let Some(v) = &mut self.viewer {
                v.pan.x += dx;
                v.pan.y += dy;
            } else if self.modal.is_none() {
                if self.show_chats {
                    self.list_scroll = (self.list_scroll - dy).max(0.);
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
        if !p.dragged {
            if let Some(hit) = self
                .hits
                .iter()
                .rev()
                .find(|h| contains(h.rect, point) && contains(h.rect, p.start))
            {
                self.activate(hit.action.clone());
            } else if let Some(link) = self.renderer.hit_link(point) {
                self.activate(Action::Link(link));
            } else if p.touch
                && p.at.elapsed().as_millis() > 450
                && let Some((key, _)) = self.renderer.hit_text(point)
            {
                let end = self.renderer.messages[&key].source.len();
                self.renderer.selection = Some((key, 0, end));
            }
        }
        self.selecting = false;
        self.field_selection = None;
        if p.at.elapsed().as_millis() > 150 {
            self.velocity = 0.;
        }
        let result = self.save();
        self.report(result);
    }
    pub fn cancel_pointer(&mut self) {
        self.dirty = true;
        self.hover = None;
        self.pointer = None;
        self.pinch = None;
        self.velocity = 0.;
        self.selecting = false;
        self.field_selection = None;
    }
    fn field_hit(&mut self, rect: Rect, point: Vec2, extend: bool) {
        let s = self.scale;
        let size = if self.focus == Some(None) {
            15. * s
        } else {
            14. * s
        };
        let Some(e) = self.editor() else {
            return;
        };
        let value = e.value.clone();
        let old = e.cursor;
        let style = sanscale::Style {
            chain: self.renderer.faces.prose[0],
            wrap_em: Some((rect.width - 20.).max(1.) / size),
            align: sanscale::Align::Left,
            line_spacing: 1.15,
        };
        if let Some(block) = self.renderer.text.shape_transient(&value, &style) {
            let layout = self.renderer.text.measure(block);
            let caret = layout.caret_rect(old);
            let scroll = (caret.y_em * size + caret.height_em * size - (rect.height - 16.)).max(0.);
            if let Some(hit) = layout.hit_test(Vec2::new(
                (point.x - rect.x - 10.) / size,
                (point.y - rect.y - 8. + scroll) / size,
            )) && let Some(e) = self.editor()
            {
                e.move_to(hit.byte_index, extend);
            }
        }
    }
    fn editor(&mut self) -> Option<&mut Editor> {
        match self.focus? {
            None => Some(&mut self.composer),
            Some(i) => self.modal.as_mut()?.fields.get_mut(i).map(|(_, e, _)| e),
        }
    }
    pub fn input(&mut self, value: &str) {
        if let Some(e) = self.editor() {
            e.replace(value);
        }
        self.edited();
    }
    #[cfg(target_os = "android")]
    pub fn native_edit(&mut self, value: String) {
        if let Some(e) = self.editor() {
            *e = Editor::new(value);
        }
        self.edited();
    }
    fn edited(&mut self) {
        if self.focus == Some(None) {
            let result = self.controller.draft(self.composer.value.clone());
            self.report(result);
        }
        self.dirty = true;
    }
    #[cfg(not(target_os = "android"))]
    pub fn preedit(&mut self, text: String) {
        if let Some(e) = self.editor() {
            e.preedit = text;
        }
        self.dirty = true;
    }
    pub fn key(&mut self, key: &str, ctrl: bool, shift: bool) {
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
                if key.eq_ignore_ascii_case("x") {
                    e.replace("");
                }
                self.platform.push(PlatformAction::Copy(copy));
                self.edited();
            }
            return;
        }
        if key == "Enter" && !shift && self.focus == Some(None) {
            self.activate(Action::Send);
            return;
        }
        let Some(e) = self.editor() else {
            return;
        };
        match key {
            "a" | "A" if ctrl => {
                e.anchor = 0;
                e.cursor = e.value.len();
            }
            "z" | "Z" if ctrl => e.undo(shift),
            "y" | "Y" if ctrl => e.undo(true),
            "Backspace" => e.backspace(),
            "Delete" => e.delete(),
            "ArrowLeft" => e.move_to(e.previous(), shift),
            "ArrowRight" => e.move_to(e.next(), shift),
            "Home" => e.move_to(
                if ctrl {
                    0
                } else {
                    e.value[..e.cursor].rfind('\n').map_or(0, |n| n + 1)
                },
                shift,
            ),
            "End" => e.move_to(
                if ctrl {
                    e.value.len()
                } else {
                    e.value[e.cursor..]
                        .find('\n')
                        .map_or(e.value.len(), |n| n + e.cursor)
                },
                shift,
            ),
            "Space" if !ctrl => e.replace(" "),
            "Enter" => e.replace("\n"),
            "Tab" => e.replace("    "),
            _ => {}
        }
        self.edited();
    }
    fn activate(&mut self, action: Action) {
        let result = self.apply(action);
        self.report(result);
    }
    fn apply(&mut self, action: Action) -> Result<()> {
        let selected = self.controller.account.selected.clone();
        match action {
            Action::Select(id) => {
                self.controller.select(&id)?;
                self.show_chats = false;
                self.scroll = 0.;
                self.focus = Some(None);
            }
            Action::New => {
                self.controller.new_chat()?;
                self.show_chats = false;
            }
            Action::Back => self.back(),
            Action::Settings => {
                self.modal = Some(Modal {
                    kind: ModalKind::Settings,
                    title: "Connection settings".into(),
                    fields: vec![
                        (
                            "Server URL".into(),
                            Editor::new(self.controller.settings.server_url.clone()),
                            false,
                        ),
                        (
                            "Bearer token".into(),
                            Editor::new(self.controller.settings.token.clone()),
                            true,
                        ),
                    ],
                    options: vec![
                        ("Connect".into(), Action::Confirm),
                        ("Title prompt".into(), Action::TitlePrompt),
                        ("Cancel".into(), Action::CancelModal),
                    ],
                });
                self.focus = None;
            }
            Action::Menu => {
                self.modal = Some(Modal {
                    kind: ModalKind::Menu,
                    title: "Chat actions".into(),
                    fields: vec![],
                    options: vec![
                        ("Rename".into(), Action::Rename),
                        ("Clone chat".into(), Action::Clone),
                        ("Sleep worker".into(), Action::Sleep),
                        ("Delete chat…".into(), Action::Delete),
                        ("Cancel".into(), Action::CancelModal),
                    ],
                });
            }
            Action::Focus(field) => {
                self.focus = Some(field);
                if self.mobile {
                    let (title, value, secret) = match field {
                        Some(i) => {
                            let (label, e, secret) = &self.modal.as_ref().unwrap().fields[i];
                            (label.clone(), e.value.clone(), *secret)
                        }
                        None => ("Message Tau".into(), self.composer.value.clone(), false),
                    };
                    self.platform.push(PlatformAction::Edit {
                        title,
                        value,
                        secret,
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
                    ModalKind::Settings => self.controller.configure(Settings {
                        server_url: values[0].clone(),
                        token: values[1].clone(),
                    })?,
                    ModalKind::Rename => {
                        if let Some(id) = selected {
                            self.controller.request(ClientCommand::RenameSession {
                                session_id: id,
                                title: values[0].clone(),
                            })?;
                        }
                    }
                    ModalKind::Delete => {
                        if let Some(id) = selected {
                            self.controller
                                .request(ClientCommand::DeleteSession { session_id: id })?;
                        }
                    }
                    ModalKind::TitlePrompt => {
                        self.controller.request(ClientCommand::SetTitlePrompt {
                            prompt: values[0].clone(),
                        })?;
                    }
                    ModalKind::QueueEdit(request_id, revision) => {
                        self.apply(Action::Queue(QueueOperation::Edit {
                            request_id,
                            revision,
                            text: values[0].clone(),
                        }))?;
                    }
                    ModalKind::Extension(session, r) => self.controller.extension_response(
                        session,
                        r.id,
                        values.first().cloned(),
                        None,
                        false,
                    )?,
                    ModalKind::ConfirmLink(url) => self.platform.push(PlatformAction::OpenUrl(url)),
                    _ => {}
                }
                self.modal = None;
                self.focus = None;
            }
            Action::CancelModal => {
                if let Some(Modal {
                    kind: ModalKind::Extension(session, r),
                    ..
                }) = self.modal.as_ref()
                {
                    self.controller.extension_response(
                        session.clone(),
                        r.id.clone(),
                        None,
                        None,
                        true,
                    )?;
                }
                self.modal = None;
                self.focus = None;
            }
            Action::TitlePrompt => {
                self.controller.title_prompt = None;
                self.controller.request(ClientCommand::GetTitlePrompt)?;
                self.waiting_title = true;
            }
            Action::ResetTitlePrompt => {
                if let (Some(modal), Some((_, default))) =
                    (&mut self.modal, &self.controller.title_prompt)
                {
                    modal.fields[0].1 = Editor::new(default.clone());
                }
            }
            Action::Rename => {
                let title = self
                    .controller
                    .account
                    .sessions
                    .iter()
                    .find(|s| Some(&s.id) == selected.as_ref())
                    .map(|s| s.title.clone())
                    .unwrap_or_default();
                self.modal = Some(Modal {
                    kind: ModalKind::Rename,
                    title: "Rename chat".into(),
                    fields: vec![("Title".into(), Editor::new(title), false)],
                    options: vec![
                        ("Save".into(), Action::Confirm),
                        ("Cancel".into(), Action::CancelModal),
                    ],
                });
            }
            Action::Delete => {
                self.modal = Some(Modal {
                    kind: ModalKind::Delete,
                    title: "Permanently delete this chat and its files?".into(),
                    fields: vec![],
                    options: vec![
                        ("Delete permanently".into(), Action::Confirm),
                        ("Cancel".into(), Action::CancelModal),
                    ],
                })
            }
            Action::Clone => {
                if let Some(id) = selected {
                    self.controller
                        .request(ClientCommand::CloneSession { session_id: id })?;
                }
                self.modal = None;
            }
            Action::Sleep => {
                if let Some(id) = selected {
                    self.controller
                        .request(ClientCommand::CloseSession { session_id: id })?;
                }
                self.modal = None;
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
            Action::History => self.controller.history()?,
            Action::Tail => {
                self.scroll = self.max_scroll;
                self.remember_scroll();
            }
            Action::DismissNotice => self.controller.notice = None,
            Action::Copy(text) => self.platform.push(PlatformAction::Copy(text)),
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
            Action::Toggle(ids) => {
                if let Some(id) = selected {
                    let c = self.controller.chats.get_mut(&id).unwrap();
                    let expanded = ids.iter().any(|id| c.local.expanded.contains(id));
                    for key in ids {
                        if expanded {
                            c.local.expanded.remove(&key);
                        } else {
                            c.local.expanded.insert(key);
                        }
                    }
                    self.controller.save_chat(&id)?;
                }
            }
            Action::Restore(id) => {
                self.controller.restore_pending(&id)?;
                self.composer =
                    Editor::new(self.controller.selected().unwrap().local.draft.clone());
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
                self.composer = Editor::new(text.clone());
                self.controller.draft(text)?;
            }
            Action::Extension(value, confirmed, cancelled) => {
                if let Some(Modal {
                    kind: ModalKind::Extension(session, r),
                    ..
                }) = &self.modal
                {
                    self.controller.extension_response(
                        session.clone(),
                        r.id.clone(),
                        value,
                        confirmed,
                        cancelled,
                    )?;
                }
                self.modal = None;
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
        let background_input = if self.modal.is_none() && self.viewer.is_none() {
            input
        } else {
            Interaction::default()
        };
        let mut main = Layer::new(background_input);
        let mut body = Layer::new(background_input);
        let mut chrome = Layer::new(background_input);
        let mut overlay = Layer::new(input);
        self.hits.clear();
        self.renderer.clear_scenes();
        main.rect(bounds, color(0x090d12));
        let wide = bounds.width / s >= 840.;
        let side = if wide { 280. * s } else { 0. };
        if wide || self.show_chats {
            self.sidebar(
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
                    overlay.images.push((
                        path.clone(),
                        Rect::new(
                            bounds.x + (bounds.width - width) / 2. + pan.x,
                            bounds.y + 60. * s + (bounds.height - 100. * s - height) / 2. + pan.y,
                            width,
                            height,
                        ),
                        Rect::new(
                            bounds.x,
                            bounds.y + 56. * s,
                            bounds.width,
                            bounds.height - 100. * s,
                        ),
                    ));
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
        if self
            .renderer
            .selection
            .as_ref()
            .is_some_and(|(_, a, b)| a != b)
            && self.modal.is_none()
        {
            button(
                &mut self.renderer,
                &mut overlay,
                &mut self.hits,
                Rect::new(
                    bounds.x + bounds.width - 150. * s,
                    bounds.y + 68. * s,
                    136. * s,
                    34. * s,
                ),
                "Copy selection",
                Action::CopySelection,
                s,
                true,
            );
        }
        if self.modal.is_some() {
            self.modal_frame(&mut overlay, bounds);
        }
        self.renderer
            .draw(ctx, view, &[main, body, chrome, overlay]);
    }
    fn sidebar(&mut self, layer: &mut Layer, b: Rect) {
        let s = self.scale;
        layer.rect(b, color(0x0e141b));
        self.renderer.label(
            layer,
            "τ  Tau",
            Rect::new(b.x + 20. * s, b.y + 20. * s, b.width - 140. * s, 36. * s),
            26. * s,
            color(0xe5eaf0),
            true,
        );
        button(
            &mut self.renderer,
            layer,
            &mut self.hits,
            Rect::new(b.x + b.width - 104. * s, b.y + 16. * s, 88. * s, 40. * s),
            "Settings",
            Action::Settings,
            s,
            false,
        );
        button(
            &mut self.renderer,
            layer,
            &mut self.hits,
            Rect::new(b.x + 16. * s, b.y + 76. * s, b.width - 32. * s, 44. * s),
            "+  New chat",
            Action::New,
            s,
            true,
        );
        let clip = Rect::new(b.x, b.y + 136. * s, b.width, (b.height - 178. * s).max(0.));
        self.list_scroll = self
            .list_scroll
            .min((self.controller.account.sessions.len() as f32 * 72. * s - clip.height).max(0.));
        for (i, session) in self.controller.account.sessions.iter().enumerate() {
            let y = clip.y + i as f32 * 72. * s - self.list_scroll;
            let rect = Rect::new(b.x + 8. * s, y, b.width - 16. * s, 68. * s);
            if y + rect.height < clip.y || y > clip.y + clip.height {
                continue;
            }
            let selected = self.controller.account.selected.as_ref() == Some(&session.id);
            layer.clipped_rounded_rect(
                rect,
                12. * s,
                layer.control_color(rect, color(if selected { 0x182b38 } else { 0x0e141b })),
                clip,
            );
            let rect = crate::render::intersect(rect, clip);
            let unread = self
                .controller
                .account
                .read_at
                .get(&session.id)
                .copied()
                .unwrap_or(0)
                < session.updated_at_ms;
            let title = if session.starter {
                "New chat"
            } else if session.title.is_empty() {
                "Unnamed chat"
            } else {
                &session.title
            };
            self.renderer.label(
                layer,
                title,
                Rect::new(rect.x + 12. * s, y + 8. * s, rect.width - 24. * s, 25. * s),
                15. * s,
                color(0xe5eaf0),
                unread,
            );
            let status = format!(
                "{}{}",
                if unread { "●  " } else { "" },
                match session.status {
                    SessionStatus::Running => "Working",
                    SessionStatus::Starting => "Starting",
                    SessionStatus::Error => "Error",
                    SessionStatus::Idle => "Ready",
                    SessionStatus::Sleeping => "Sleeping",
                }
            );
            self.renderer.label(
                layer,
                &status,
                Rect::new(rect.x + 12. * s, y + 36. * s, rect.width - 24. * s, 22. * s),
                12. * s,
                color(if session.status == SessionStatus::Running {
                    0x67d4ff
                } else {
                    0x82909f
                }),
                false,
            );
            self.hits.push(Hit {
                rect,
                action: Action::Select(session.id.clone()),
            });
        }
        let label = if self.controller.epoch.is_some() {
            "●  Connected"
        } else {
            &self.controller.connection
        };
        self.renderer.label(
            layer,
            label,
            Rect::new(
                b.x + 18. * s,
                b.y + b.height - 32. * s,
                b.width - 36. * s,
                24. * s,
            ),
            12. * s,
            color(if self.controller.epoch.is_some() {
                0x4ade80
            } else {
                0xfbbf24
            }),
            false,
        );
    }
    fn rows(&self, session: &str) -> Vec<Row> {
        let chat = &self.controller.chats[session];
        let events = chat.feed.events.values().collect::<Vec<_>>();
        let mut rows = vec![];
        let mut i = 0;
        while i < events.len() {
            let e = events[i];
            if e.kind == EventKind::Hidden && e.error_message.is_none() {
                i += 1;
                continue;
            }
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
                let ids = group.iter().map(|e| e.id.clone()).collect::<Vec<_>>();
                let expanded = ids.iter().any(|id| chat.local.expanded.contains(id));
                let tools = group.iter().filter(|e| e.kind == EventKind::Tool).count();
                let title = format!(
                    "{}  Thinking & tools{}",
                    if expanded { "▾" } else { "▸" },
                    if tools > 0 {
                        format!(" · {tools}")
                    } else {
                        String::new()
                    }
                );
                let source = if expanded {
                    group
                        .iter()
                        .map(|e| {
                            format!(
                                "**{}**\n\n{}",
                                e.tool_name.as_deref().unwrap_or("Thinking"),
                                if e.kind == EventKind::Thinking {
                                    e.text.clone()
                                } else {
                                    code(&e.text)
                                }
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n\n")
                } else {
                    String::new()
                };
                rows.push(Row {
                    key: format!("{session}/{}", e.id),
                    title,
                    source,
                    user: false,
                    error: group.iter().any(|e| e.is_error),
                    actions: vec![(
                        if expanded { "Collapse" } else { "Expand" }.into(),
                        Action::Toggle(ids),
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
            let mut actions = vec![("Copy".into(), Action::Copy(e.text.clone()))];
            if user && e.phase == EventPhase::Saved {
                actions.push(("Fork".into(), Action::Fork(e.entry_id.clone())));
            }
            rows.push(Row {
                key: format!("{session}/{}", e.id),
                title,
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
        for (key, value) in &chat.statuses {
            rows.push(Row {
                key: format!("status:{key}"),
                title: "Extension status".into(),
                source: literal(value),
                user: false,
                error: false,
                actions: vec![],
                attachment: None,
            });
        }
        for (key, lines) in &chat.widgets {
            if !lines.is_empty() {
                rows.push(Row {
                    key: format!("widget:{key}"),
                    title: key.clone(),
                    source: literal(&lines.join("\n")),
                    user: false,
                    error: false,
                    actions: vec![],
                    attachment: None,
                });
            }
        }
        for p in &chat.local.pending {
            rows.push(Row {
                key: format!("pending:{}", p.request.id),
                title: p.status.label().into(),
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
            let mut actions = vec![];
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
                key: format!("queue:{}", q.request_id),
                title: format!("Queued{}", if state.paused { " · held" } else { "" }),
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
        let wide = self.size.0 as f32 / s >= 840.;
        let Some(session) = self.controller.account.selected.clone() else {
            return;
        };
        if !self.controller.chats.contains_key(&session) {
            return;
        }
        let header = Rect::new(b.x, b.y, b.width, 64. * s);
        chrome.rect(header, color(0x0e141b));
        if !wide {
            button(
                &mut self.renderer,
                chrome,
                &mut self.hits,
                Rect::new(b.x + 8. * s, b.y + 12. * s, 44. * s, 40. * s),
                "‹",
                Action::Back,
                s,
                false,
            );
        }
        let title_x = b.x + if wide { 24. * s } else { 64. * s };
        let summary = self
            .controller
            .account
            .sessions
            .iter()
            .find(|s| s.id == session)
            .cloned();
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
            Rect::new(
                title_x,
                b.y + 12. * s,
                (b.x + b.width - 64. * s - title_x).max(1.),
                28. * s,
            ),
            17. * s,
            color(0xe5eaf0),
            true,
        );
        self.renderer.label(
            chrome,
            &self.controller.connection,
            Rect::new(title_x, b.y + 40. * s, b.width - 120. * s, 18. * s),
            10. * s,
            color(if self.controller.epoch.is_some() {
                0x4ade80
            } else {
                0xfbbf24
            }),
            false,
        );
        button(
            &mut self.renderer,
            chrome,
            &mut self.hits,
            Rect::new(b.x + b.width - 52. * s, b.y + 12. * s, 40. * s, 40. * s),
            "···",
            Action::Menu,
            s,
            false,
        );
        let files = self.controller.chats[&session].local.files.clone();
        let composer_h = (if files.is_empty() { 146. } else { 188. }) * s;
        let bottom = b.y + b.height;
        let composer_top = (bottom - composer_h).max(b.y + 80. * s);
        let viewport = Rect::new(
            b.x,
            b.y + 64. * s,
            b.width,
            (composer_top - b.y - 64. * s).max(1.),
        );
        self.transcript = viewport;
        let width = (b.width - 32. * s).min(900. * s).max(80. * s);
        let x = b.x + (b.width - width) / 2.;
        let text_width = width - 28. * s;
        let rows = self.rows(&session);
        let mut placements = vec![];
        let mut y = 12. * s;
        let has_history = self.controller.chats[&session].feed.before.is_some();
        if has_history {
            y += 48. * s;
        }
        let mut keys = HashSet::new();
        self.max_horizontal = 0.;
        for row in &rows {
            keys.insert(row.key.clone());
            let text_height = if row.source.is_empty() {
                0.
            } else {
                self.renderer
                    .message_height(&row.key, &row.source, text_width, 15. * s)
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
            let height = 38. * s
                + text_height
                + if row.source.is_empty() { 0. } else { 14. * s }
                + attachment
                + 42. * s;
            placements.push(Placed {
                key: row.key.clone(),
                top: y,
                height,
            });
            y += height + 12. * s;
        }
        self.renderer.retain_messages(&keys);
        self.horizontal = self.horizontal.clamp(0., self.max_horizontal);
        self.max_scroll = (y - viewport.height).max(0.);
        let position = &self.controller.chats[&session].local.position;
        if position.follow {
            self.scroll = self.max_scroll;
        } else if let Some(key) = &position.key
            && let Some(p) = placements.iter().find(|p| &p.key == key)
        {
            self.scroll = (p.top + position.offset * s).clamp(0., self.max_scroll);
        } else {
            self.scroll = self.scroll.clamp(0., self.max_scroll);
        }
        self.placed = placements;
        if has_history {
            let label = if self.controller.chats[&session].feed.loading {
                "Loading…"
            } else {
                "Load earlier messages"
            };
            let rect = Rect::new(x, viewport.y + 8. * s - self.scroll, width, 36. * s);
            if rect.y + rect.height > viewport.y {
                button(
                    &mut self.renderer,
                    layer,
                    &mut self.hits,
                    crate::render::intersect(rect, viewport),
                    label,
                    Action::History,
                    s,
                    false,
                );
            }
        }
        for (row, p) in rows.iter().zip(&self.placed) {
            let top = viewport.y + p.top - self.scroll;
            if top + p.height < viewport.y || top > viewport.y + viewport.height {
                continue;
            }
            let rect = Rect::new(x, top, width, p.height);
            layer.clipped_rounded_rect(
                rect,
                12. * s,
                color(if row.user { 0x13232f } else { 0x18212b }),
                viewport,
            );
            let label_rect = crate::render::intersect(
                Rect::new(x + 14. * s, top + 10. * s, text_width, 24. * s),
                viewport,
            );
            self.renderer.label(
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
            );
            let text_top = top + 38. * s;
            if !row.source.is_empty() {
                self.renderer.message(
                    layer,
                    &row.key,
                    Vec2::new(x + 14. * s, text_top),
                    Rect::new(x + 14. * s, viewport.y, text_width, viewport.height),
                    self.horizontal,
                );
            }
            let mut actions = row.actions.clone();
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
                    Rect::new(ax, top + p.height - 38. * s, width, 30. * s),
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
        }
        chrome.rect(
            Rect::new(b.x, composer_top, b.width, composer_h),
            color(0x0e141b),
        );
        let mut model = summary
            .as_ref()
            .and_then(|s| s.model.as_ref())
            .map(|m| format!("{} / {}", m.provider, m.model_id))
            .unwrap_or_else(|| "Model loads when the worker starts".into());
        if let Some(usage) = summary.as_ref().and_then(|s| s.context_usage) {
            model.push_str(&format!(
                "  ·  {} / {} tokens",
                usage.tokens.map_or_else(|| "?".into(), |t| t.to_string()),
                usage.context_window
            ));
        }
        self.renderer.label(
            chrome,
            &model,
            Rect::new(x, composer_top + 8. * s, width, 20. * s),
            10. * s,
            color(0x82909f),
            false,
        );
        let composer_rect = Rect::new(x, composer_top + 32. * s, width - 90. * s, 68. * s);
        self.composer.draw(
            &mut self.renderer,
            chrome,
            composer_rect,
            15. * s,
            self.focus == Some(None),
            false,
            "Message Tau…",
        );
        self.hits.push(Hit {
            rect: composer_rect,
            action: Action::Focus(None),
        });
        button(
            &mut self.renderer,
            chrome,
            &mut self.hits,
            Rect::new(
                x + width - 82. * s,
                composer_top + 32. * s,
                82. * s,
                68. * s,
            ),
            "Send",
            Action::Send,
            s,
            true,
        );
        button(
            &mut self.renderer,
            chrome,
            &mut self.hits,
            Rect::new(x, composer_top + 108. * s, 72. * s, 30. * s),
            "Attach",
            Action::Attach,
            s,
            false,
        );
        button(
            &mut self.renderer,
            chrome,
            &mut self.hits,
            Rect::new(x + 80. * s, composer_top + 108. * s, 66. * s, 30. * s),
            "Stop",
            Action::Abort,
            s,
            false,
        );
        if self.scroll + 24. * s < self.max_scroll {
            button(
                &mut self.renderer,
                chrome,
                &mut self.hits,
                Rect::new(
                    x + width - 78. * s,
                    composer_top - 38. * s,
                    78. * s,
                    30. * s,
                ),
                "↓ Latest",
                Action::Tail,
                s,
                true,
            );
        }
        let queue = &self.controller.chats[&session].feed.queue;
        if queue.paused {
            button(
                &mut self.renderer,
                chrome,
                &mut self.hits,
                Rect::new(x + 154. * s, composer_top + 108. * s, 82. * s, 30. * s),
                "Resume",
                Action::Queue(QueueOperation::Resume {
                    run_id: queue.run_id.clone(),
                }),
                s,
                false,
            );
        }
        if let Some(control) = &queue.control
            && matches!(control.status.as_str(), "waiting" | "applying")
        {
            button(
                &mut self.renderer,
                chrome,
                &mut self.hits,
                Rect::new(x + 242. * s, composer_top + 108. * s, 100. * s, 30. * s),
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
                Rect::new(fx, composer_top + 148. * s, fw, 30. * s),
                &label,
                Action::RemoveFile(file.id),
                s,
                false,
            );
            fx += fw + 6. * s;
        }
        // Slash completion is driven by the daemon catalog, including built-in arguments.
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
        if let Some(notice) = &self.controller.notice {
            let rect = Rect::new(x, b.y + 70. * s, width, 68. * s);
            chrome.rounded_rect(rect, 12. * s, color(0x452c2a));
            self.renderer.label(
                chrome,
                notice,
                Rect::new(x + 10. * s, rect.y + 8. * s, width - 54. * s, 54. * s),
                12. * s,
                color(0xffd8d0),
                false,
            );
            button(
                &mut self.renderer,
                chrome,
                &mut self.hits,
                Rect::new(x + width - 40. * s, rect.y + 8. * s, 32. * s, 32. * s),
                "×",
                Action::DismissNotice,
                s,
                false,
            );
        }
    }
    fn modal_frame(&mut self, layer: &mut Layer, b: Rect) {
        let s = self.scale;
        self.hits.clear();
        layer.rect(b, sanscale::Color([0., 0., 0., 0.8]));
        let modal = self.modal.as_ref().unwrap();
        let long = matches!(
            modal.kind,
            ModalKind::TitlePrompt | ModalKind::QueueEdit(..) | ModalKind::Extension(..)
        );
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
        for (i, (name, e, secret)) in modal.fields.iter().enumerate() {
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
                14. * s,
                self.focus == Some(Some(i)),
                *secret,
                "",
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
    layer.rounded_rect(
        rect,
        rect.height * 0.5,
        layer.control_color(rect, color(if primary { 0x164e63 } else { 0x18212b })),
    );
    let style = sanscale::Style {
        chain: renderer.faces.prose[0],
        wrap_em: None,
        align: sanscale::Align::Left,
        line_spacing: 1.,
    };
    if let Some(block) = renderer.text.shape_transient(label, &style) {
        let layout = renderer.text.measure(block);
        let size = (12. * s).min((rect.width - 12. * s).max(1.) / layout.width_em().max(1.));
        layer.draws.push(sanscale::Draw {
            block,
            at: Vec2::new(
                rect.x + (rect.width - layout.width_em() * size) / 2.,
                rect.y + (rect.height - layout.height_em() * size) / 2.,
            ),
            size,
            color: color(if primary { 0xc7f0ff } else { 0xb7c2ce }),
            clip: Some(rect),
            ..Default::default()
        });
    }
    hits.push(Hit { rect, action });
}
fn literal(text: &str) -> String {
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
fn code(text: &str) -> String {
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
