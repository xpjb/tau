//! Settings widgets own staged edits and completion lifetimes. Settings schema,
//! parsing and validation remain in daemon_settings / models, not in the UI tree.
use super::controls::{ButtonStyle, Controls, TextField};
use super::{Context, Controller, Event, Frame, Id, Request, Target, UiState, Widget};
use crate::{
    daemon_settings::{Draft, Kind, SECTIONS, fields},
    editor::Editor,
    render::color,
};
use anyhow::Result;
use sanscale::Rect;
use tau_net::*;

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
    form: Controls<ModelChoice>,
    suggestions: Controls<String>,
    identity: String,
}
impl ModelsDialog {
    fn search_visible(ui: &UiState) -> bool {
        ui.size.1 as f32 / ui.scale >= 320.
    }
    pub fn new(cx: &mut Context<'_>) -> Result<Self> {
        cx.model.warm_model_catalog();
        let id = Id::new();
        let mut models = TextField::new(id, "Quick models", Editor::new(cx.model.model_preferences.text()));
        models.placeholder = "provider/model".into();
        models.size = 16.;
        let mut search = TextField::new(id, "Search model suggestions", Editor::line(String::new()));
        search.placeholder = "Search optional model suggestions".into();
        search.size = 16.;
        cx.ui.focus = Some(models.control.target);
        Ok(Self {
            id,
            models,
            search,
            form: Controls::declared(
                id,
                &[(ModelChoice::Save, "Save"), (ModelChoice::Presets, "Presets"), (ModelChoice::Close, "Cancel")],
            ),
            suggestions: Controls::new(id),
            identity: cx.model.identity.clone(),
        })
    }
    pub fn buttons(&self) -> Vec<(&str, Rect)> {
        self.form
            .items
            .iter()
            .map(|(_, b, _)| b)
            .chain(self.suggestions.items.iter().map(|(_, b, _)| b))
            .filter_map(|b| b.control.rect.map(|r| (b.label.as_str(), r)))
            .collect()
    }
}
impl Widget for ModelsDialog {
    fn update(&mut self, _dt: f32, cx: &mut Context<'_>) {
        if self.identity != cx.model.identity {
            cx.ui.requests.push_back(Request::Close(self.id));
        }
    }
    fn owns(&self, target: Target, _model: &Controller, ui: &UiState) -> bool {
        self.form.owns(target)
            || self.models.control.target == target
            || Self::search_visible(ui) && (self.search.control.target == target || self.suggestions.owns(target))
    }

    fn handle_event(&mut self, event: &Event<'_>, cx: &mut Context<'_>) -> bool {
        if self.identity != cx.model.identity {
            cx.ui.requests.push_back(Request::Close(self.id));
            return true;
        }
        let composing = self.models.editor.composing() || self.search.editor.composing();
        let choice = match event {
            Event::Back | Event::Key { key: "Escape", .. } if !composing => Some(ModelChoice::Close),
            _ => {
                let visible = Self::search_visible(cx.ui);
                let (handled, slug) = if visible { self.suggestions.event(event, cx) } else { (false, None) };
                if let Some(slug) = slug {
                    let result = (|| -> Result<()> {
                        let mut prefs = crate::models::Preferences::parse(&self.models.editor.value)?;
                        if prefs.slugs.contains(&slug) {
                            prefs.slugs.retain(|s| s != &slug);
                        } else {
                            anyhow::ensure!(prefs.slugs.len() < 12, "Choose at most 12 quick models");
                            prefs.slugs.push(slug);
                        }
                        self.models.editor = Editor::new(prefs.text());
                        Ok(())
                    })();
                    self.form.report(result, cx);
                }
                if handled {
                    return true;
                }
                self.form
                    .event_fields(event, std::iter::once(&mut self.models).chain(visible.then_some(&mut self.search)), cx)
                    .1
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
                self.form.report(result, cx);
            }
            None => {}
        }
        true
    }
    fn visit_perframe(&mut self, frame: &mut Frame<'_>, cx: &mut Context<'_>) {
        let b = frame.bounds;
        let s = cx.ui.scale;
        let area = self.form.page(680., "Quick model selection", frame, cx);
        let (x, w, top, footer) = (area.x, area.width, b.y + 12. * s, area.y + area.height);
        let compact = b.height / s < 480.;
        let show_suggestions = Self::search_visible(cx.ui);
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
        frame.visit(Rect::new(x, y, w, h), &mut self.models, cx);
        let search_y = y + h + 12. * s;
        self.search.control.rect = None;
        self.search.editor.hide();
        if show_suggestions {
            frame.visit(Rect::new(x, search_y, w, 36. * s), &mut self.search, cx);
        }
        let list_y = search_y + 44. * s;
        let query = self.search.editor.value.to_lowercase();
        let count = if show_suggestions { ((footer - 52. * s - list_y) / (32. * s)).max(0.) as usize } else { 0 };
        let slugs = cx.model.model_catalog.models.iter()
            .filter(|m| m.value.to_lowercase().contains(&query))
            .take(count).map(|m| m.value.clone()).collect::<Vec<_>>();
        self.suggestions.begin();
        let preferences = crate::models::Preferences::parse(&self.models.editor.value).ok();
        for (i, slug) in slugs.into_iter().enumerate() {
            let label = format!(
                "{} {}",
                if preferences.as_ref().is_some_and(|p| p.slugs.contains(&slug)) { "−" } else { "+" },
                slug
            );
            self.suggestions.button(
                cx,
                frame.layer,
                Rect::new(x, list_y + i as f32 * 32. * s, w, 30. * s),
                &label,
                slug,
                ButtonStyle::Tonal,
                frame.clip,
            );
        }
        self.suggestions.finish(cx);
        frame.feedback(
            Rect::new(x, footer - 44. * s, w, 44. * s),
            "Suggestions are optional; the provider decides availability.",
            cx,
        );
        self.form.row(
            &[
                (ModelChoice::Save, ButtonStyle::Primary),
                (ModelChoice::Presets, ButtonStyle::Tonal),
                (ModelChoice::Close, ButtonStyle::Tonal),
            ],
            Rect::new(x, footer, w, 40. * s),
            frame,
            cx,
        );
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
    form: Controls<SettingChoice>,
    waiting: bool,
    saving: Option<String>,
    identity: String,
    lineage: Option<String>,
}
impl DaemonDialog {
    fn field_visible(&self) -> bool {
        self.draft.as_ref().is_some_and(|d| d.definition().kind != Kind::Bool)
    }
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
            form: Controls::declared(id, &buttons),
            waiting: false,
            saving: None,
            identity: cx.model.identity.clone(),
            lineage: cx.model.account.source_lineage.clone(),
        };
        if let Err(error) = dialog.reload(cx) {
            dialog.form.report(Err(error), cx);
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
        self.form.items.iter().filter_map(|(_, b, _)| b.control.rect.map(|r| (b.label.as_str(), r))).collect()
    }
}
impl Widget for DaemonDialog {
    fn update(&mut self, _dt: f32, cx: &mut Context<'_>) {
        if self.identity != cx.model.identity || self.lineage != cx.model.account.source_lineage {
            cx.ui.requests.push_back(Request::Close(self.id));
            return;
        }
        if self.waiting
            && let Some(document) = cx.model.daemon_settings.clone()
        {
            self.waiting = false;
            let result = Draft::new(&document, self.identity.clone()).and_then(|draft| {
                self.draft = Some(draft);
                self.load(cx)
            });
            self.form.report(result, cx);
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
    fn owns(&self, target: Target, _model: &Controller, _ui: &UiState) -> bool {
        self.form.owns(target)
            || self.field_visible() && self.value.as_ref().is_some_and(|f| f.control.target == target)
    }

    fn handle_event(&mut self, event: &Event<'_>, cx: &mut Context<'_>) -> bool {
        if self.identity != cx.model.identity || self.lineage != cx.model.account.source_lineage {
            cx.ui.requests.push_back(Request::Close(self.id));
            return true;
        }

        let composing = self.value.as_ref().is_some_and(|f| f.editor.composing());
        let choice = match event {
            Event::Back | Event::Key { key: "Escape", .. } if !composing => Some(SettingChoice::Close),
            _ => {
                let visible = self.field_visible();
                self.form.event_fields(event, self.value.iter_mut().filter(|_| visible), cx).1
            }
        };
        if let Some(choice) = choice {
            let result = self.choose(choice, cx);
            self.form.report(result, cx);
        }
        true
    }
    fn visit_perframe(&mut self, frame: &mut Frame<'_>, cx: &mut Context<'_>) {
        let b = frame.bounds;
        let s = cx.ui.scale;
        let busy = self.saving.is_some();
        let compact = b.height / s < 400.;
        let title = self
            .draft
            .as_ref()
            .map(|d| format!("Daemon settings · revision {}", d.revision))
            .unwrap_or_else(|| "Daemon settings".into());
        let area = self.form.page(900., &title, frame, cx);
        let (x, w, footer) = (area.x, area.width, area.y + area.height);
        let feedback = frame.feedback(
            area,
            if busy {
                "Saving…"
            } else if compact {
                ""
            } else {
                "Edits are staged until Save. Reload discards them. Credentials stay on the daemon."
            },
            cx,
        );
        for (_, button, choice) in &mut self.form.items {
            button.control.enabled = !busy || *choice == SettingChoice::Close;
        }
        if let Some(draft) = &self.draft {
            let mut y = b.y + 48. * s;
            if !compact {
                let columns = if w / s >= 600. { SECTIONS.len() } else { 3 };
                let tw = (w - 8. * s * (columns - 1) as f32) / columns as f32;
                for (i, _) in SECTIONS.iter().enumerate() {
                    self.form.paint(
                        SettingChoice::Section(i),
                        Rect::new(
                            x + (i % columns) as f32 * (tw + 8. * s),
                            y + (i / columns) as f32 * 36. * s,
                            tw,
                            30. * s,
                        ),
                        if i == draft.section { ButtonStyle::Primary } else { ButtonStyle::Tonal },
                        frame,
                        cx,
                    );
                }
                y += SECTIONS.len().div_ceil(columns) as f32 * 36. * s + 8. * s;
            }
            let definition = draft.definition();
            let count = fields(draft.section).len();
            if count > 1 {
                self.form.paint(
                    SettingChoice::Previous,
                    Rect::new(x, y, 36. * s, 32. * s),
                    ButtonStyle::Tonal,
                    frame,
                    cx,
                );
                self.form.paint(
                    SettingChoice::Next,
                    Rect::new(x + w - 36. * s, y, 36. * s, 32. * s),
                    ButtonStyle::Tonal,
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
                    self.form.items.iter_mut().find(|(_, _, a)| *a == SettingChoice::Toggle).unwrap().1.label =
                        label.into();
                    self.form.paint(SettingChoice::Toggle, Rect::new(x, y, w, 32. * s), ButtonStyle::Tonal, frame, cx);
                    y += 40. * s;
                }
                if definition.kind != Kind::Bool {
                    let reserve = if compact { feedback + 4. * s } else { (feedback + 40. * s).max(88. * s) };
                    let available = (footer - reserve - y).max(1.);
                    let h = if field.editor.single_line { available.min(48. * s) } else { available };
                    frame.visit(Rect::new(x, y, w, h), field, cx);
                }
            }
            if !compact {
                self.form.paint(
                    SettingChoice::Reset,
                    Rect::new(x, footer - 80. * s, 120. * s, 28. * s),
                    ButtonStyle::Tonal,
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
        self.form.row(
            &[
                (SettingChoice::Save, ButtonStyle::Primary),
                (SettingChoice::Reload, ButtonStyle::Tonal),
                (SettingChoice::Close, ButtonStyle::Tonal),
            ],
            Rect::new(x, footer, w, 40. * s),
            frame,
            cx,
        );
    }
}
