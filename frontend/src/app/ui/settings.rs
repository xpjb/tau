//! Settings widgets own staged edits and completion lifetimes. Settings schema,
//! parsing and validation remain in daemon_settings / models, not in the UI tree.
use super::controls::{Button, Form, TextField};
use super::{Context, Controller, Event, Frame, Id, Request, Target, UiState, Widget};
use crate::{
    daemon_settings::{Draft, Kind, SECTIONS, fields},
    editor::Editor,
    render::color,
};
use anyhow::Result;
use sanscale::Rect;
use tau_protocol::*;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ModelChoice {
    Save,
    Presets,
    Close,
}
pub(in crate::app) struct ModelsDialog {
    pub id: Id,
    pub models: TextField,
    pub search: TextField,
    form: Form<ModelChoice>,
    suggestions: Vec<(String, Button)>,
    identity: String,
}
impl ModelsDialog {
    pub fn new(cx: &mut Context<'_>) -> Result<Self> {
        let id = Id::new();
        let mut models = TextField::new(id, "Quick models", Editor::new(cx.model.model_preferences.text()));
        models.placeholder = "provider/model".into();
        models.size = 16.;
        let mut search = TextField::new(id, "Search model suggestions", Editor::line(String::new()));
        search.placeholder = "Search optional model suggestions".into();
        search.size = 16.;
        if let Some(chat) = cx.model.selected()
            && !chat.commands_loaded
            && cx.model.epoch.is_some()
        {
            cx.model.request(ClientCommand::GetCommands { session_id: cx.model.account.selected.clone().unwrap() })?;
        }
        cx.ui.focus = Some(models.control.target);
        Ok(Self {
            id,
            models,
            search,
            form: Form::new(
                id,
                &[(ModelChoice::Save, "Save"), (ModelChoice::Presets, "Presets"), (ModelChoice::Close, "Cancel")],
            ),
            suggestions: vec![],
            identity: cx.model.identity.clone(),
        })
    }
    pub fn buttons(&self) -> Vec<(&str, Rect)> {
        self.form
            .buttons
            .iter()
            .map(|(_, b)| b)
            .chain(self.suggestions.iter().map(|(_, b)| b))
            .filter_map(|b| b.control.rect.map(|r| (b.label.as_str(), r)))
            .collect()
    }
}
impl Widget for ModelsDialog {
    fn owns(&self, target: Target, _model: &Controller, _ui: &UiState) -> bool {
        self.form.owns(target)
            || self.models.control.target == target
            || self.search.control.target == target
            || self.suggestions.iter().any(|(_, b)| b.control.target == target && b.control.rect.is_some())
    }

    fn handle_event(&mut self, event: &Event<'_>, cx: &mut Context<'_>) -> bool {
        if self.identity != cx.model.identity {
            cx.ui.requests.push_back(Request::Close(self.id));
            return true;
        }
        let composing = self.models.editor.composing() || self.search.editor.composing();
        let choice = match event {
            Event::Back | Event::Key { key: "Escape", .. } if !composing => Some(ModelChoice::Close),
            Event::Submit if !composing => Some(ModelChoice::Save),
            _ => {
                for (slug, button) in &mut self.suggestions {
                    if button.handle_event(event, cx) {
                        if button.control.take_click() {
                            let result = (|| -> Result<()> {
                                let mut prefs = crate::models::Preferences::parse(&self.models.editor.value)?;
                                if prefs.slugs.contains(slug) {
                                    prefs.slugs.retain(|s| s != slug);
                                } else {
                                    anyhow::ensure!(prefs.slugs.len() < 12, "Choose at most 12 quick models");
                                    prefs.slugs.push(slug.clone());
                                }
                                self.models.editor = Editor::new(prefs.text());
                                Ok(())
                            })();
                            cx.report(result);
                        }
                        return true;
                    }
                }
                self.form.event(event, &mut [&mut self.models, &mut self.search], cx).1
            }
        };
        match choice {
            Some(ModelChoice::Close) => cx.ui.requests.push_back(Request::Close(self.id)),
            Some(ModelChoice::Presets) => {
                self.models.editor = Editor::new(crate::models::Preferences::default().text());
                cx.ui.dirty = true;
            }
            Some(ModelChoice::Save) => {
                let result = crate::models::Preferences::parse(&self.models.editor.value)
                    .and_then(|prefs| cx.model.save_model_preferences(prefs));
                if result.is_ok() {
                    cx.ui.requests.push_back(Request::Close(self.id));
                }
                cx.report(result);
            }
            None => {}
        }
        true
    }
    fn visit_perframe(&mut self, frame: &mut Frame<'_>, cx: &mut Context<'_>) {
        let b = frame.bounds;
        let s = cx.ui.scale;
        frame.layer.rect(b, color(0x0e141b));
        self.form.begin_frame();
        let w = (b.width - 32. * s).min(680. * s).max(1.);
        let x = b.x + (b.width - w) / 2.;
        let top = b.y + 12. * s;
        let footer = b.y + b.height - 52. * s;
        cx.services.renderer.label(
            frame.layer,
            "Quick model selection",
            Rect::new(x, top, w, 30. * s),
            20. * s,
            color(0xe5eaf0),
            true,
        );
        let compact = b.height / s < 480.;
        let show_suggestions = b.height / s >= 320.;
        let help_h = if compact { 32. } else { 58. } * s;
        cx.services.renderer.label(
            frame.layer,
            "One provider/model per line (max 12). Last chosen model is remembered. Empty disables quick-select tiles.",
            Rect::new(x, top + 36. * s, w, help_h),
            12. * s,
            color(0xb7c2ce),
            false,
        );
        let y = top + 44. * s + help_h;
        let h = (b.height * 0.25)
            .min(164. * s)
            .min((footer - 48. * s - y - if show_suggestions { 56. * s } else { 0. }).max(1.));
        self.models
            .visit_perframe(&mut Frame { layer: frame.layer, bounds: Rect::new(x, y, w, h), clip: frame.clip }, cx);
        let search_y = y + h + 12. * s;
        self.search.control.rect = None;
        self.search.editor.hide();
        if show_suggestions {
            self.search.visit_perframe(
                &mut Frame { layer: frame.layer, bounds: Rect::new(x, search_y, w, 36. * s), clip: frame.clip },
                cx,
            );
        }
        let list_y = search_y + 44. * s;
        let query = self.search.editor.value.to_lowercase();
        let count = if show_suggestions { ((footer - 52. * s - list_y) / (32. * s)).max(0.) as usize } else { 0 };
        let slugs = cx
            .model
            .selected()
            .and_then(|c| c.commands.iter().find(|c| c.name == "model" && c.source == SlashCommandSource::Builtin))
            .map(|c| {
                c.arguments
                    .iter()
                    .filter(|m| m.value.to_lowercase().contains(&query))
                    .take(count)
                    .map(|m| m.value.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        self.suggestions.retain(|(slug, _)| slugs.contains(slug));
        let preferences = crate::models::Preferences::parse(&self.models.editor.value).ok();
        for (i, slug) in slugs.into_iter().enumerate() {
            if !self.suggestions.iter().any(|(key, _)| key == &slug) {
                self.suggestions.push((slug.clone(), Button::new(self.id, "")));
            }
            let button = &mut self.suggestions.iter_mut().find(|(key, _)| key == &slug).unwrap().1;
            button.label = format!(
                "{} {}",
                if preferences.as_ref().is_some_and(|p| p.slugs.contains(&slug)) { "−" } else { "+" },
                slug
            );
            button.visit_perframe(
                &mut Frame {
                    layer: frame.layer,
                    bounds: Rect::new(x, list_y + i as f32 * 32. * s, w, 30. * s),
                    clip: frame.clip,
                },
                cx,
            );
        }
        cx.services.renderer.label(
            frame.layer,
            cx.model.notice.as_deref().unwrap_or("Suggestions are optional; the provider decides availability."),
            Rect::new(x, footer - 44. * s, w, 36. * s),
            12. * s,
            color(if cx.model.notice.is_some() { 0xffb4ab } else { 0x82909f }),
            false,
        );
        for (i, choice) in [ModelChoice::Save, ModelChoice::Presets, ModelChoice::Close].into_iter().enumerate() {
            self.form.button(
                choice,
                Rect::new(x + i as f32 * (w + 8. * s) / 3., footer, (w - 16. * s) / 3., 40. * s),
                choice == ModelChoice::Save,
                false,
                frame,
                cx,
            );
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SettingChoice {
    Save,
    Reload,
    Close,
    Section(usize),
    Previous,
    Next,
    Toggle,
    Reset,
}
pub(in crate::app) struct DaemonDialog {
    pub id: Id,
    pub value: Option<TextField>,
    pub draft: Option<Draft>,
    form: Form<SettingChoice>,
    waiting: bool,
    saving: Option<String>,
    identity: String,
    lineage: Option<String>,
}
impl DaemonDialog {
    pub fn new(cx: &mut Context<'_>) -> Result<Self> {
        let id = Id::new();
        let mut buttons = vec![
            (SettingChoice::Save, "Save"),
            (SettingChoice::Reload, "Reload"),
            (SettingChoice::Close, "Close"),
            (SettingChoice::Previous, "‹"),
            (SettingChoice::Next, "›"),
            (SettingChoice::Toggle, "Toggle"),
            (SettingChoice::Reset, "Reset field"),
        ];
        buttons.extend(SECTIONS.iter().enumerate().map(|(i, label)| (SettingChoice::Section(i), *label)));
        let mut dialog = Self {
            id,
            value: None,
            draft: None,
            form: Form::new(id, &buttons),
            waiting: false,
            saving: None,
            identity: cx.model.identity.clone(),
            lineage: cx.model.account.source_lineage.clone(),
        };
        if let Err(error) = dialog.reload(cx) {
            cx.report(Err(error));
        }
        Ok(dialog)
    }
    fn reload(&mut self, cx: &mut Context<'_>) -> Result<()> {
        cx.model.daemon_settings = None;
        cx.model.request(ClientCommand::GetSettings)?;
        self.draft = None;
        self.value = None;
        self.waiting = true;
        self.saving = None;
        cx.ui.focus = None;
        Ok(())
    }
    fn apply(&mut self) -> Result<()> {
        if let (Some(draft), Some(value)) = (&mut self.draft, &self.value) {
            draft.apply(&value.editor.value)?;
        }
        Ok(())
    }
    pub fn load(&mut self, cx: &mut Context<'_>) -> Result<()> {
        if let Some(draft) = &mut self.draft {
            let definition = draft.definition();
            let text = draft.text()?;
            let editor = if matches!(
                definition.kind,
                Kind::Line | Kind::Number | Kind::Bool | Kind::Model | Kind::OptionalModel | Kind::PromptModel
            ) {
                Editor::line(text)
            } else {
                Editor::new(text)
            };
            self.value = Some(TextField::new(self.id, definition.name, editor));
        }
        cx.ui.focus = None;
        Ok(())
    }
    fn choose(&mut self, choice: SettingChoice, cx: &mut Context<'_>) -> Result<()> {
        if choice == SettingChoice::Close {
            cx.ui.requests.push_back(Request::Close(self.id));
            return Ok(());
        }
        if self.saving.is_some() {
            return Ok(());
        }
        match choice {
            SettingChoice::Reload => self.reload(cx)?,
            SettingChoice::Save => {
                self.apply()?;
                let draft = self.draft.as_ref().ok_or_else(|| anyhow::anyhow!("Load settings first"))?;
                cx.model.notice = None;
                cx.model.settings_result = None;
                self.saving = Some(cx.model.request(ClientCommand::SetSettings {
                    revision: draft.revision,
                    settings: Box::new(draft.document()?),
                })?);
                cx.ui.focus = None;
            }
            SettingChoice::Section(section) => {
                self.apply()?;
                if let Some(d) = &mut self.draft {
                    d.section = section;
                    d.field = 0;
                }
                self.load(cx)?;
            }
            SettingChoice::Previous | SettingChoice::Next => {
                self.apply()?;
                if let Some(d) = &mut self.draft {
                    let n = fields(d.section).len();
                    d.field = (d.field + if choice == SettingChoice::Next { 1 } else { n - 1 }) % n;
                }
                self.load(cx)?;
            }
            SettingChoice::Toggle => {
                if let (Some(d), Some(field)) = (&mut self.draft, &mut self.value) {
                    if d.definition().kind == Kind::PromptOverride {
                        d.inherit = !d.inherit;
                        if d.inherit {
                            field.editor = Editor::new(d.default_prompt().into());
                        }
                    } else {
                        field.editor = Editor::line((field.editor.value != "true").to_string());
                    }
                    cx.ui.focus = None;
                }
            }
            SettingChoice::Reset => {
                if let Some(d) = &mut self.draft {
                    d.reset()?;
                }
                self.load(cx)?;
            }
            SettingChoice::Close => {}
        }
        Ok(())
    }
    pub fn buttons(&self) -> Vec<(&str, Rect)> {
        self.form.buttons.iter().filter_map(|(_, b)| b.control.rect.map(|r| (b.label.as_str(), r))).collect()
    }
}
impl Widget for DaemonDialog {
    fn owns(&self, target: Target, _model: &Controller, _ui: &UiState) -> bool {
        self.form.owns(target) || self.value.as_ref().is_some_and(|f| f.control.target == target)
    }

    fn handle_event(&mut self, event: &Event<'_>, cx: &mut Context<'_>) -> bool {
        if self.identity != cx.model.identity || self.lineage != cx.model.account.source_lineage {
            cx.ui.requests.push_back(Request::Close(self.id));
            return true;
        }
        if matches!(event, Event::Tick(_)) {
            if self.waiting
                && let Some(document) = cx.model.daemon_settings.clone()
            {
                self.waiting = false;
                let result = Draft::new(&document, self.identity.clone()).and_then(|draft| {
                    self.draft = Some(draft);
                    self.load(cx)
                });
                cx.report(result);
            }
            if let Some(request) = &self.saving {
                if cx.model.settings_result.as_ref().is_some_and(|(id, _)| id == request) {
                    let ok = cx.model.settings_result.take().unwrap().1;
                    self.saving = None;
                    if ok && let (Some(d), Some(doc)) = (&mut self.draft, &cx.model.daemon_settings) {
                        d.revision = doc.revision;
                        cx.model.notice = Some("Settings saved".into());
                    }
                    cx.ui.dirty = true;
                } else if cx.model.epoch.is_none() {
                    self.saving = None;
                    cx.model.notice = Some("Save unconfirmed. Reload before saving again; it was not resent.".into());
                    cx.ui.dirty = true;
                }
            }
        }
        let composing = self.value.as_ref().is_some_and(|f| f.editor.composing());
        let choice = match event {
            Event::Back | Event::Key { key: "Escape", .. } if !composing => Some(SettingChoice::Close),
            Event::Submit if !composing => Some(SettingChoice::Save),
            _ => self.form.event(event, &mut self.value.iter_mut().collect::<Vec<_>>(), cx).1,
        };
        if let Some(choice) = choice {
            let result = self.choose(choice, cx);
            cx.report(result);
        }
        true
    }
    fn visit_perframe(&mut self, frame: &mut Frame<'_>, cx: &mut Context<'_>) {
        let b = frame.bounds;
        let s = cx.ui.scale;
        let busy = self.saving.is_some();
        frame.layer.rect(b, color(0x0e141b));
        self.form.begin_frame();
        let w = (b.width - 32. * s).min(900. * s).max(1.);
        let x = b.x + (b.width - w) / 2.;
        let footer = b.y + b.height - 52. * s;
        let compact = b.height / s < 400.;
        let title = self
            .draft
            .as_ref()
            .map(|d| format!("Daemon settings · revision {}", d.revision))
            .unwrap_or_else(|| "Daemon settings".into());
        cx.services.renderer.label(
            frame.layer,
            &title,
            Rect::new(x, b.y + 12. * s, w, 28. * s),
            20. * s,
            color(0xe5eaf0),
            true,
        );
        if let Some(draft) = &self.draft {
            let mut y = b.y + 48. * s;
            if !compact {
                let columns = if w / s >= 600. { SECTIONS.len() } else { 3 };
                let tw = (w - 8. * s * (columns - 1) as f32) / columns as f32;
                for (i, _) in SECTIONS.iter().enumerate() {
                    self.form.button(
                        SettingChoice::Section(i),
                        Rect::new(
                            x + (i % columns) as f32 * (tw + 8. * s),
                            y + (i / columns) as f32 * 36. * s,
                            tw,
                            30. * s,
                        ),
                        i == draft.section,
                        false,
                        frame,
                        cx,
                    );
                }
                y += SECTIONS.len().div_ceil(columns) as f32 * 36. * s + 8. * s;
            }
            let definition = draft.definition();
            let count = fields(draft.section).len();
            if count > 1 {
                self.form.button(SettingChoice::Previous, Rect::new(x, y, 36. * s, 32. * s), false, false, frame, cx);
                self.form.button(
                    SettingChoice::Next,
                    Rect::new(x + w - 36. * s, y, 36. * s, 32. * s),
                    false,
                    false,
                    frame,
                    cx,
                );
            }
            cx.services.renderer.label(
                frame.layer,
                &format!("{}  ({}/{count})", definition.name, draft.field + 1),
                Rect::new(x + 44. * s, y, w - 88. * s, 32. * s),
                15. * s,
                color(0xe5eaf0),
                true,
            );
            y += 40. * s;
            if !compact {
                let help = if definition.kind == Kind::PromptOverride {
                    format!("{}\n{}", draft.prompt_model, definition.help)
                } else {
                    definition.help.into()
                };
                let h = if b.height / s < 480. { 44. } else { 72. } * s;
                cx.services.renderer.label(frame.layer, &help, Rect::new(x, y, w, h), 12. * s, color(0xb7c2ce), false);
                y += h + 8. * s;
            }
            if let Some(field) = &mut self.value {
                field.control.rect = None;
                field.editor.hide();
                let readonly =
                    definition.kind == Kind::Bool || definition.kind == Kind::PromptOverride && draft.inherit;
                field.control.enabled = !busy && !readonly;
                if matches!(definition.kind, Kind::Bool | Kind::PromptOverride) {
                    let label = if definition.kind == Kind::Bool {
                        if field.editor.value == "true" {
                            "✓ Enabled — tap to disable"
                        } else {
                            "Disabled — tap to enable"
                        }
                    } else if draft.inherit {
                        "✓ Inherit default — switch to override"
                    } else {
                        "Model override — switch to inherited default"
                    };
                    self.form.buttons.iter_mut().find(|(a, _)| *a == SettingChoice::Toggle).unwrap().1.label =
                        label.into();
                    self.form.button(SettingChoice::Toggle, Rect::new(x, y, w, 32. * s), false, false, frame, cx);
                    y += 40. * s;
                }
                if definition.kind != Kind::Bool {
                    let available = (footer - if compact { 4. } else { 88. } * s - y).max(1.);
                    let h = if field.editor.single_line { available.min(48. * s) } else { available };
                    field.visit_perframe(
                        &mut Frame { layer: frame.layer, bounds: Rect::new(x, y, w, h), clip: frame.clip },
                        cx,
                    );
                }
            }
            if !compact {
                self.form.button(
                    SettingChoice::Reset,
                    Rect::new(x, footer - 80. * s, 120. * s, 28. * s),
                    false,
                    false,
                    frame,
                    cx,
                );
            }
        } else {
            cx.services.renderer.label(
                frame.layer,
                if self.waiting { "Loading the daemon's settings…" } else { "Connect and reload to edit settings." },
                Rect::new(x, b.y + 72. * s, w, 60. * s),
                15. * s,
                color(0xb7c2ce),
                false,
            );
        }
        if !compact {
            cx.services.renderer.label(
                frame.layer,
                cx.model.notice.as_deref().unwrap_or(if busy {
                    "Saving…"
                } else {
                    "Edits are staged until Save. Reload discards them. Credentials stay on the daemon."
                }),
                Rect::new(x, footer - 42. * s, w, 36. * s),
                12. * s,
                color(0xb7c2ce),
                false,
            );
        }
        for (i, choice) in [SettingChoice::Save, SettingChoice::Reload, SettingChoice::Close].into_iter().enumerate() {
            if busy && choice != SettingChoice::Close {
                continue;
            }
            self.form.button(
                choice,
                Rect::new(x + i as f32 * (w + 8. * s) / 3., footer, (w - 16. * s) / 3., 40. * s),
                choice == SettingChoice::Save,
                false,
                frame,
                cx,
            );
        }
        for (choice, button) in &mut self.form.buttons {
            button.control.enabled = !busy || *choice == SettingChoice::Close;
        }
    }
}
