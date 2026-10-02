use super::controls::{ButtonStyle, Controls, TextField};
use super::{Context, Controller, DialogSpec, Event, Frame, Id, Request, Target, UiState, Widget};
use crate::{
    app::{PlatformAction, context_usage_display},
    editor::Editor,
    icons::Icon,
    render::{Layer, color},
    tooltip::Content,
};
use sanscale::Rect;
use tau_protocol::*;
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) enum Choice {
    RetryCreate,
    Attach,
    Usage,
    Send,
    Tail,
    Queue(QueueOperation),
    RemoveFile(String),
    Suggest(String),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) enum ModelChoice {
    Select(String, String),
    Configure,
}
pub(in crate::app) struct Composer {
    pub field: TextField,
    pub controls: Controls<Choice>,
    binding: Option<(String, Option<String>, Option<String>)>,
    pub submit: bool,
    pub tail: bool,
    pub usage_toggle: bool,
    pub in_code: bool,
    pub code_ready: bool,
    pub away_from_tail: bool,
    pub usage_rect: Rect,
    pub usage: Content,
}
impl Composer {
    pub fn new() -> Self {
        let id = Id::new();
        let mut field = TextField::new(id, "", Editor::composer(String::new()));
        field.size = 16.;
        field.placeholder = "Message Tau".into();
        field.decorated = false;
        Self {
            field,
            controls: Controls::new(id),
            binding: None,
            submit: false,
            tail: false,
            usage_toggle: false,
            in_code: false,
            code_ready: false,
            away_from_tail: false,
            usage_rect: Rect::new(0., 0., 0., 0.),
            usage: Content::default(),
        }
    }
    pub fn hide(&mut self) {
        self.field.control.rect = None;
        self.field.editor.hide();
        self.controls.begin();
        self.usage_rect = Rect::new(0., 0., 0., 0.);
    }
    pub fn bound(&self, model: &crate::controller::Controller) -> bool {
        self.binding.as_ref().is_some_and(|(identity, lineage, session)| {
            identity == &model.identity
                && lineage == &model.account.source_lineage
                && session == &model.account.selected
        })
    }
    pub fn bind(&mut self, cx: &mut Context<'_>) {
        let binding =
            (cx.model.identity.clone(), cx.model.account.source_lineage.clone(), cx.model.account.selected.clone());
        let value = cx.model.selected().map(|c| c.local.draft.clone()).unwrap_or_default();
        if self.binding.as_ref() != Some(&binding) {
            let focused = cx.ui.focus == Some(self.field.control.target);
            cx.ui.detach(self.controls.id);
            let mut next = Self::new();
            next.field.editor = Editor::composer(value);
            next.binding = Some(binding);
            *self = next;
            if focused {
                cx.ui.focus = Some(self.field.control.target);
            }
            cx.ui.dirty = true;
        } else if self.field.editor.value != value {
            self.replace(value, cx);
        }
        cx.ui.composer = Some(self.field.control.target);
    }
    pub fn replace(&mut self, value: String, cx: &mut Context<'_>) {
        let old = self.field.editor.native_id();
        self.field.editor = Editor::composer(value);
        if let Some(native) = &mut cx.ui.native
            && native.token == old
            && native.target == self.field.control.target
        {
            native.token = self.field.editor.native_id();
        }
        cx.ui.dirty = true;
    }
    pub fn edited(&mut self, cx: &mut Context<'_>) {
        if self.bound(cx.model) {
            let result = cx.model.draft(self.field.editor.value.clone());
            cx.report(result);
        }
    }
    pub fn send(&mut self, cx: &mut Context<'_>) -> anyhow::Result<()> {
        cx.model.send_prompt()?;
        self.replace(cx.model.selected().map(|c| c.local.draft.clone()).unwrap_or_default(), cx);
        Ok(())
    }
    pub fn layout(&mut self, cx: &mut Context<'_>, b: Rect, session: &str) -> (f32, f32, f32, f32, f32) {
        let s = cx.ui.scale;
        let files = &cx.model.chats[session].local.files;
        let width = (b.width - 28. * s).min(900. * s).max(160. * s);
        let x = b.x + (b.width - width) / 2.;
        let editor_h = self.field.editor.height(&mut cx.services.renderer, width - 132. * s, 16. * s);
        let queue = &cx.model.chats[session].feed.queue;
        let controls = queue.control.as_ref().is_some_and(|c| matches!(c.status.as_str(), "waiting" | "applying"));
        let chrome = (44. + if files.is_empty() { 0. } else { 40. } + if controls { 40. } else { 0. }) * s;
        let editor_h = if cx.ui.mobile { editor_h.min((b.height - 80. * s - chrome).max(16. * s)) } else { editor_h };
        let composer_h = editor_h + chrome;
        let bottom = b.y + b.height;
        let composer_top = (bottom - composer_h).max(b.y + 80. * s);
        (x, width, editor_h, composer_h, composer_top)
    }
    pub fn model_status(
        &mut self,
        cx: &mut Context<'_>,
        layer: &mut Layer,
        summary: Option<&SessionSummary>,
        rect: Rect,
    ) {
        let model = summary
            .and_then(|s| s.model.as_ref())
            .map(|m| format!("{}/{}", m.provider, m.model_id))
            .unwrap_or_else(|| "Model: unknown".into());
        let level =
            summary.and_then(|s| s.thinking_level.as_deref()).filter(|level| !level.is_empty()).unwrap_or("unknown");
        let thinking = format!("Thinking: {level}");
        let size = 12. * cx.ui.scale;
        let gap = 12. * cx.ui.scale;
        let thinking_width = cx.services.renderer.label_width(&thinking, size, false).ceil();
        let model_width = cx
            .services
            .renderer
            .label_width(&model, size, false)
            .ceil()
            .min((rect.width - thinking_width - gap).max(0.));
        cx.services.renderer.ellipsized_label(
            layer,
            &model,
            Rect::new(rect.x, rect.y, model_width, rect.height),
            size,
            color(0x82909f),
            false,
            false,
            rect,
        );
        cx.services.renderer.label(
            layer,
            &thinking,
            Rect::new(rect.x + model_width + gap, rect.y, thinking_width, rect.height),
            size,
            color(0xb7c2ce),
            false,
        );
    }
}
impl Widget for Composer {
    fn update(&mut self, dt: f32, cx: &mut Context<'_>) { self.field.update_scroll(dt, cx); }
    fn owns(&self, target: Target, model: &Controller, _ui: &UiState) -> bool {
        self.bound(model) && (self.field.control.target == target || self.controls.owns(target))
    }

    fn handle_event(&mut self, event: &Event<'_>, cx: &mut Context<'_>) -> bool {
        if !self.bound(cx.model) {
            return false;
        }
        if matches!(event, Event::Key { key: "Enter", ctrl: false, shift: false })
            && !cx.ui.mobile
            && cx.ui.focus == Some(self.field.control.target)
            && !self.field.editor.composing()
        {
            self.submit = true;
            return true;
        }
        // Escape/desktop-global accelerators belong to the workspace unless IME composition owns them.
        let global =
            matches!(event, Event::Key { key: "Escape", .. } | Event::Key { key: "Space" | " ", ctrl: true, .. })
                && !self.field.editor.composing();
        let old = self.field.editor.value.clone();
        let field = !global && self.field.handle_event(event, cx);
        if self.field.editor.value != old {
            self.edited(cx);
        }
        if field { return true; }
        let (handled, choice) = self.controls.event(event, cx);
        let result = match choice {
            Some(Choice::RetryCreate) => cx.model.retry_create_manually(),
            Some(Choice::Attach) => {
                if let Some(session) = cx.model.account.selected.clone() {
                    cx.services
                        .platform
                        .push(PlatformAction::PickFile { identity: cx.model.identity.clone(), session });
                }
                Ok(())
            }
            Some(Choice::Usage) => {
                self.usage_toggle = true;
                Ok(())
            }
            Some(Choice::Send) => {
                self.submit = true;
                Ok(())
            }
            Some(Choice::Tail) => {
                self.tail = true;
                Ok(())
            }
            Some(Choice::Queue(operation)) => {
                let session_id = cx.model.account.selected.clone().unwrap();
                let generation = cx.model.chats[&session_id].feed.generation.clone();
                cx.model.control(ClientCommand::QueueControl { session_id, generation, operation })
            }
            Some(Choice::RemoveFile(id)) => cx.model.remove_file(&id),
            Some(Choice::Suggest(text)) => {
                self.replace(text, cx);
                cx.ui.focus = Some(self.field.control.target);
                self.edited(cx);
                Ok(())
            }
            None => return handled,
        };
        cx.report(result);
        handled
    }
    fn visit_perframe(&mut self, frame: &mut Frame<'_>, cx: &mut Context<'_>) {
        let Some(session) = cx.model.account.selected.clone().filter(|id| cx.model.chats.contains_key(id)) else {
            return;
        };
        let session = session.as_str();
        let b = frame.bounds;
        let chrome = &mut *frame.layer;
        chrome.above();
        let s = cx.ui.scale;
        self.controls.begin();
        let in_code = self.in_code;
        let (x, width, editor_h, composer_h, composer_top) = self.layout(cx, b, session);
        let files = cx.model.chats[session].local.files.clone();
        let controls = cx.model.chats[session]
            .feed
            .queue
            .control
            .as_ref()
            .is_some_and(|c| matches!(c.status.as_str(), "waiting" | "applying"));
        let summary = cx.model.account.sessions.iter().find(|s| s.id == session).cloned();
        chrome.rect(Rect::new(b.x, composer_top, b.width, composer_h), color(0x0e141b));
        let retry_create = cx.model.create_needs_retry(session);
        let status_rect =
            Rect::new(x, composer_top + 10. * s, (width - if retry_create { 94. * s } else { 0. }).max(1.), 20. * s);
        if summary.as_ref().is_some_and(|s| s.model.is_some()) {
            self.model_status(cx, chrome, summary.as_ref(), status_rect);
        }
        if retry_create {
            self.controls.button(
                cx,
                chrome,
                Rect::new(x + width - 88. * s, composer_top + 6. * s, 88. * s, 24. * s),
                "Retry",
                Choice::RetryCreate,
                ButtonStyle::Tonal,
                frame.clip,
            );
        }
        chrome.rect(Rect::new(b.x, composer_top, b.width, s), color(0x2a3541));
        let field = Rect::new(x, composer_top + 32. * s, width, editor_h);
        let edge = if cx.ui.focus == Some(self.field.control.target) { 2. * s } else { s };
        chrome.rounded_rect(
            field,
            4. * s,
            color(if cx.ui.focus == Some(self.field.control.target) { 0x67d4ff } else { 0x526170 }),
        );
        chrome.rounded_rect(
            Rect::new(field.x + edge, field.y + edge, field.width - 2. * edge, field.height - 2. * edge),
            3. * s,
            color(0x0e141b),
        );
        let composer_rect = Rect::new(field.x + 40. * s, field.y, field.width - 132. * s, field.height);
        self.field.visit_perframe(&mut Frame { layer: chrome, bounds: composer_rect, clip: frame.clip }, cx);
        let iy = field.y + (field.height - 40. * s) / 2.;
        let connected = cx.model.epoch.is_some();
        self.controls.icon(
            cx,
            chrome,
            Rect::new(field.x + 4. * s, iy, 36. * s, 40. * s),
            Icon::Attach,
            22.,
            Choice::Attach,
            ButtonStyle::Quiet,
            true,
            frame.clip,
        );
        let usage_rect = Rect::new(field.x + field.width - 84. * s, iy, 40. * s, 40. * s);
        let usage = summary.as_ref().and_then(|s| s.context_usage);
        let (ratio, usage_text) = context_usage_display(usage);
        self.controls.icon(
            cx,
            chrome,
            usage_rect,
            Icon::Context(ratio),
            20.,
            Choice::Usage,
            ButtonStyle::Quiet,
            true,
            frame.clip,
        );
        self.usage_rect = usage_rect;
        self.usage = usage_text;
        if usage.is_some()
            && (!connected
                || !cx.model.chats[session].feed.synchronized
                || summary.as_ref().is_none_or(|s| !matches!(s.status, SessionStatus::Idle | SessionStatus::Running)))
        {
            self.usage.line().dim("Last known value");
        }
        match summary.as_ref().and_then(|s| s.model.as_ref()).map(|m| m.provider.as_str()) {
            Some("openai-codex") => {
                self.usage.line().line();
                self.usage.append(cx.model.codex_usage.content(connected));
            }
            Some(_) => {
                self.usage.line().line().dim("Account quota unavailable for this provider");
            }
            None => {
                self.usage.line().line().dim("Account quota unavailable (model unknown)");
            }
        }
        let can_send =
            (!self.field.editor.value.trim().is_empty() || !files.is_empty()) && (!in_code || self.code_ready);
        self.controls.icon(
            cx,
            chrome,
            Rect::new(field.x + field.width - 44. * s, iy, 40. * s, 40. * s),
            Icon::Send,
            20.,
            Choice::Send,
            ButtonStyle::Primary,
            can_send,
            frame.clip,
        );
        let controls_y = field.y + field.height + 4. * s;
        if !in_code && self.away_from_tail {
            self.controls.icon(
                cx,
                chrome,
                Rect::new(x + width - 40. * s, composer_top - 48. * s, 40. * s, 40. * s),
                Icon::ChevronDown,
                20.,
                Choice::Tail,
                ButtonStyle::Primary,
                true,
                frame.clip,
            );
        }
        let queue = &cx.model.chats[session].feed.queue;
        if let Some(control) = &queue.control
            && matches!(control.status.as_str(), "waiting" | "applying")
        {
            self.controls.button(
                cx,
                chrome,
                Rect::new(x, controls_y, 140. * s, 30. * s),
                if control.action == "prefix" { "Cancel run limit" } else { "Cancel pause" },
                Choice::Queue(QueueOperation::Cancel { control_id: control.command_id.clone() }),
                ButtonStyle::Tonal,
                frame.clip,
            );
        }
        let mut fx = x;
        for file in files {
            let label = format!("{} ×", file.name);
            let fw = ((label.chars().count() as f32 * 7. + 20.) * s).min(width);
            if fx + fw > x + width {
                break;
            }
            self.controls.button(
                cx,
                chrome,
                Rect::new(fx, controls_y + if controls { 40. * s } else { 0. }, fw, 30. * s),
                &label,
                Choice::RemoveFile(file.id),
                ButtonStyle::Tonal,
                frame.clip,
            );
            fx += fw + 6. * s;
        }
        // Slash completion uses optional arguments advertised by the daemon.
        if self.field.editor.value.starts_with('/')
            && !self.field.editor.value.contains('\n')
            && cx.ui.focus == Some(self.field.control.target)
        {
            let query = &self.field.editor.value[1..];
            let mut suggestions = vec![];
            for command in &cx.model.chats[session].commands {
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
                self.controls.button(
                    cx,
                    chrome,
                    Rect::new(x, composer_top - (i + 1) as f32 * 36. * s, width, 34. * s),
                    text.trim(),
                    Choice::Suggest(text.clone()),
                    ButtonStyle::Tonal,
                    frame.clip,
                );
            }
        }
        self.controls.finish(cx);
    }
}
pub(in crate::app) struct QuickModels {
    pub controls: Controls<ModelChoice>,
}
impl QuickModels {
    pub fn new() -> Self {
        Self { controls: Controls::new(Id::new()) }
    }
    pub fn height(&self, cx: &Context<'_>, width: f32) -> f32 {
        let columns = if width / cx.ui.scale >= 520. { 2 } else { 1 };
        (72. + cx.model.model_preferences.slugs.len().div_ceil(columns) as f32 * 84. + 48.) * cx.ui.scale
    }
}
impl Widget for QuickModels {
    fn owns(&self, target: Target, _model: &Controller, _ui: &UiState) -> bool {
        self.controls.owns(target)
    }

    fn handle_event(&mut self, event: &Event<'_>, cx: &mut Context<'_>) -> bool {
        let (handled, choice) = self.controls.event(event, cx);
        match choice {
            Some(ModelChoice::Select(session, slug)) if Some(&session) == cx.model.account.selected.as_ref() => {
                let result = cx.model.choose_model(&session, &slug);
                cx.report(result);
            }
            Some(ModelChoice::Configure) => cx.ui.requests.push_back(Request::Open(DialogSpec::Models)),
            _ => {}
        }
        handled
    }
    fn visit_perframe(&mut self, frame: &mut Frame<'_>, cx: &mut Context<'_>) {
        self.controls.begin();
        let Some(session) = cx.model.account.selected.clone() else {
            return;
        };
        let session = session.as_str();
        let b = frame.bounds;
        let clip = frame.clip;
        let layer = &mut *frame.layer;
        let s = cx.ui.scale;
        let chat = &cx.model.chats[session];
        let ready = cx.model.can_choose_model(session);
        let hint = "Optional · new chats use your last model";
        cx.services.renderer.clipped_label(
            layer,
            "Choose a model",
            Rect::new(b.x, b.y, b.width, 28. * s),
            20. * s,
            color(0xe5eaf0),
            true,
            clip,
        );
        cx.services.renderer.clipped_label(
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
        let current = cx
            .model
            .account
            .sessions
            .iter()
            .find(|c| c.id == session)
            .and_then(|c| c.model.as_ref())
            .map(|m| format!("{}/{}", m.provider, m.model_id));
        for (i, selector) in cx.model.model_preferences.slugs.iter().enumerate() {
            let r = Rect::new(
                b.x + (i % columns) as f32 * (w + 8. * s),
                b.y + (72. + (i / columns) as f32 * 84.) * s,
                w,
                76. * s,
            );
            let valid = selector.parse::<tau_protocol::SessionModel>().is_ok();
            let selected = current.as_deref() == Some(selector.as_str());
            let enabled = ready && valid;
            let base = color(if selected { 0x303a66 } else { 0x18212b });
            layer.clipped_rounded_rect(r, 12. * s, base, clip);
            let control = &mut self.controls.place(ModelChoice::Select(session.into(), selector.clone()), r, clip, false).control;
            control.enabled = enabled;
            control.corners = Some([12. * s; 4]);
            control.highlight(layer, cx.ui, false);
            cx.services.renderer.clipped_label(
                layer,
                selector,
                Rect::new(r.x + 12. * s, r.y + 10. * s, w - 24. * s, 32. * s),
                12. * s,
                color(if enabled || selected { 0xe5eaf0 } else { 0x82909f }),
                false,
                crate::render::intersect(r, clip),
            );
            let status = if chat.model_request.as_ref().is_some_and(|(_, slug)| slug == selector) {
                "Selecting…"
            } else if !valid {
                "Invalid provider/model ID"
            } else if selected {
                "Selected"
            } else if !ready {
                "Available when connected"
            } else {
                "Select"
            };
            cx.services.renderer.clipped_label(
                layer,
                status,
                Rect::new(r.x + 12. * s, r.y + 54. * s, w - 24. * s, 16. * s),
                11. * s,
                color(if selected { 0x67d4ff } else { 0xb7c2ce }),
                false,
                crate::render::intersect(r, clip),
            );
        }
        let r = Rect::new(b.x, b.y + b.height - 40. * s, b.width, 32. * s);
        layer.clipped_rounded_rect(r, 16. * s, color(0x18212b), clip);
        self.controls.place(ModelChoice::Configure, r, clip, true).control.highlight(layer, cx.ui, false);
        cx.services.renderer.clipped_label(
            layer,
            "Configure quick models…",
            Rect::new(r.x + 12. * s, r.y + 7. * s, r.width - 24. * s, 20. * s),
            12. * s,
            color(0x67d4ff),
            false,
            crate::render::intersect(r, clip),
        );
        self.controls.finish(cx);
    }
}
