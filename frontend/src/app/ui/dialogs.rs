use super::{Context, Event, Frame, Id, LegacyDialog, Request, Target, Widget};
use super::controls::{Form, TextField};
use crate::{editor::Editor, render::color, store::Settings};
use sanscale::Rect;
use tau_protocol::*;
use anyhow::Result;

pub(in crate::app) enum TopicEdit { New, Rename(String), Prompt(String), Delete(String) }
pub(in crate::app) enum DialogSpec { Connection, Topic(TopicEdit) }
pub(in crate::app) enum Dialog { Connection(ConnectionDialog), Topic(TopicDialog) }
impl Dialog {
    pub fn new(spec: DialogSpec, cx: &mut Context<'_>) -> Result<Self> {
        cx.model.notice = None;
        Ok(match spec {
            DialogSpec::Connection => Self::Connection(ConnectionDialog::new(cx)),
            DialogSpec::Topic(edit) => Self::Topic(TopicDialog::new(edit, cx)?),
        })
    }
    pub fn id(&self) -> Id { match self { Self::Connection(d) => d.form.id, Self::Topic(d) => d.form.id } }
    pub fn field(&mut self, target: Target) -> Option<&mut TextField> {
        match self {
            Self::Connection(d) => [&mut d.url, &mut d.token].into_iter().find(|f| f.control.target == target),
            Self::Topic(d) => d.fields().into_iter().find(|f| f.control.target == target),
        }
    }
    pub fn field_ref(&self, target: Target) -> Option<&TextField> {
        match self {
            Self::Connection(d) => [&d.url, &d.token].into_iter().find(|f| f.control.target == target),
            Self::Topic(d) => d.name.iter().chain(d.prompt.iter()).find(|f| f.control.target == target),
        }
    }
    #[cfg(test)]
    pub fn fields(&self) -> Vec<&TextField> {
        match self { Self::Connection(d) => vec![&d.url, &d.token], Self::Topic(d) => d.name.iter().chain(d.prompt.iter()).collect() }
    }
    #[cfg(test)]
    pub fn topic_key(&self) -> Option<(&str, &str)> {
        let Self::Topic(d) = self else { return None; };
        Some(match &d.kind {
            TopicKind::New(id) => ("new", id.as_str()), TopicKind::Rename(p) => ("rename", p.id.as_str()),
            TopicKind::Prompt(p) => ("prompt", p.id.as_str()), TopicKind::Delete(p) => ("delete", p.id.as_str()),
            TopicKind::Choice(p) => ("delete-choice", p.id.as_str()),
        })
    }
    #[cfg(test)]
    pub fn buttons(&self) -> Vec<(&str, Rect)> {
        match self {
            Self::Connection(d) => d.form.buttons.iter().filter_map(|(_, b)| b.control.rect.map(|r| (b.label.as_str(), r))).collect(),
            Self::Topic(d) => d.form.buttons.iter().filter_map(|(_, b)| b.control.rect.map(|r| (b.label.as_str(), r))).collect(),
        }
    }
    #[cfg(test)]
    pub fn button(&self, label: &str) -> Option<Rect> {
        match self {
            Self::Connection(d) => d.form.buttons.iter().find_map(|(_, b)| (b.label == label).then_some(b.control.rect).flatten()),
            Self::Topic(d) => d.form.buttons.iter().find_map(|(_, b)| (b.label == label).then_some(b.control.rect).flatten()),
        }
    }
}
impl Widget for Dialog {
    fn handle_event(&mut self, event: &Event<'_>, cx: &mut Context<'_>) -> bool {
        match self { Self::Connection(d) => d.handle_event(event, cx), Self::Topic(d) => d.handle_event(event, cx) }
    }
    fn visit_perframe(&mut self, frame: &mut Frame<'_>, cx: &mut Context<'_>) {
        match self { Self::Connection(d) => d.visit_perframe(frame, cx), Self::Topic(d) => d.visit_perframe(frame, cx) }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ConnectionChoice { Connect, Cancel, Models, Daemon, Refresh, Tools, Main, Diagnostics, Clear, Outbox }
pub(in crate::app) struct ConnectionDialog {
    form: Form<ConnectionChoice>,
    url: TextField,
    token: TextField,
    attempt: Option<String>,
    tools: bool,
}
impl ConnectionDialog {
    fn new(cx: &mut Context<'_>) -> Self {
        let id = Id::new();
        let mut url = TextField::new(id, "Server URL", Editor::line(cx.model.settings.server_url.clone()));
        url.placeholder = "http://vibe:8787".into(); url.size = 16.;
        let mut token = TextField::new(id, "Access token", Editor::line(cx.model.settings.token.clone()));
        token.secret = true; token.placeholder = "Access token".into(); token.size = 16.;
        cx.ui.focus = Some(url.control.target);
        Self { form: Form::new(id, &[(ConnectionChoice::Connect, "Connect"), (ConnectionChoice::Cancel, "Cancel"),
            (ConnectionChoice::Models, "Quick model selection"), (ConnectionChoice::Daemon, "Open settings"), (ConnectionChoice::Refresh, "Refresh models"), (ConnectionChoice::Tools, "More…"),
            (ConnectionChoice::Main, "Back"), (ConnectionChoice::Diagnostics, "Copy connection diagnostics"),
            (ConnectionChoice::Clear, "Clear replica cache (keep local work)"), (ConnectionChoice::Outbox, "Saved actions")]),
            url, token, attempt: None, tools: false }
    }
    fn submit(&mut self, cx: &mut Context<'_>) -> Result<()> {
        if self.attempt.is_some() && cx.model.connection == "Connecting…" { return Ok(()); }
        cx.model.configure(Settings { server_url: self.url.editor.value.clone(), token: self.token.editor.value.clone() })?;
        self.attempt = Some(cx.model.identity.clone());
        self.url.control.enabled = false; self.token.control.enabled = false; cx.ui.focus = None;
        Ok(())
    }
}
impl Widget for ConnectionDialog {
    fn handle_event(&mut self, event: &Event<'_>, cx: &mut Context<'_>) -> bool {
        let composing = self.url.editor.composing() || self.token.editor.composing();
        if matches!(event, Event::Back) && composing {
            self.url.editor.preedit(String::new(), None); self.token.editor.preedit(String::new(), None);
            cx.ui.dirty = true; return true;
        }
        let choice = match event {
            Event::Tick(_) => {
                if let Some(identity) = &self.attempt {
                    if identity != &cx.model.identity { self.attempt = None; }
                    else if cx.model.epoch.is_some() { cx.ui.requests.push_back(Request::Close(self.form.id)); self.attempt = None; }
                    else if cx.model.connection != "Connecting…" { self.attempt = None; }
                    if self.attempt.is_none() {
                        self.url.control.enabled = true; self.token.control.enabled = true; cx.ui.dirty = true;
                    }
                }
                self.form.event(event, &mut [&mut self.url, &mut self.token], cx)
            }
            Event::Back | Event::Key { key: "Escape", .. } if !composing => Some(if self.tools { ConnectionChoice::Main } else { ConnectionChoice::Cancel }),
            Event::Submit | Event::Key { key: "Enter", shift: false, .. } if !composing && !self.tools => Some(ConnectionChoice::Connect),
            _ if self.tools => self.form.event(event, &mut [], cx),
            _ => self.form.event(event, &mut [&mut self.url, &mut self.token], cx),
        };
        match choice {
            Some(ConnectionChoice::Connect) => { let result = self.submit(cx); cx.report(result); }
            Some(ConnectionChoice::Cancel) => cx.ui.requests.push_back(Request::Close(self.form.id)),
            Some(ConnectionChoice::Tools | ConnectionChoice::Main) => {
                self.tools = !self.tools; self.form.begin_frame();
                self.url.control.rect = None; self.token.control.rect = None;
                cx.ui.focus = if self.tools { None } else { Some(self.url.control.target) }; cx.ui.dirty = true;
            }
            Some(ConnectionChoice::Diagnostics) => { cx.services.platform.push(super::PlatformAction::Copy(cx.model.diagnostics())); }
            Some(ConnectionChoice::Clear) => {
                let result = cx.model.clear_replica();
                if result.is_ok() {
                    cx.model.notice = Some("Replica cache cleared. Drafts, attachments and saved intents were preserved.".into());
                    cx.ui.requests.push_back(Request::Close(self.form.id));
                }
                cx.report(result);
            }
            Some(choice) => cx.ui.requests.push_back(Request::Legacy { owner: self.form.id, dialog: match choice {
                ConnectionChoice::Models => LegacyDialog::Models,
                ConnectionChoice::Daemon => LegacyDialog::Daemon,
                ConnectionChoice::Refresh => LegacyDialog::RefreshCatalog,
                ConnectionChoice::Outbox => LegacyDialog::Outbox,
                _ => unreachable!(),
            } }),
            None => {}
        }
        true
    }
    fn visit_perframe(&mut self, frame: &mut Frame<'_>, cx: &mut Context<'_>) {
        let b = frame.bounds; let s = cx.ui.scale;
        self.form.begin_frame();
        if self.tools {
            self.url.control.rect = None; self.token.control.rect = None;
            self.url.editor.hide(); self.token.editor.hide();
            frame.layer.rect(b, color(0x0e141b));
            let w = (b.width - 24. * s).min(520. * s).max(1.); let x = b.x + (b.width - w) / 2.;
            cx.services.renderer.label(frame.layer, "Connection tools", Rect::new(x, b.y + 24. * s, w, 40. * s), 22. * s, color(0xe5eaf0), true);
            for (i, action) in [ConnectionChoice::Outbox, ConnectionChoice::Diagnostics, ConnectionChoice::Clear, ConnectionChoice::Main].into_iter().enumerate() {
                self.form.button(action, Rect::new(x, b.y + (84. + i as f32 * 48.) * s, w, 40. * s), false, false, frame, cx);
            }
            return;
        }
        // The actual fields stay inside the resized activity when the IME is up.
        // Secondary settings remain available after dismissing the keyboard.
        if b.height / s < 520. {
            frame.layer.rect(b, color(0x0e141b));
            let w = (b.width - 24. * s).min(520. * s).max(1.); let x = b.x + (b.width - w) / 2.;
            let h = ((b.height - 86. * s) / 2.).clamp(24. * s, 48. * s);
            for (i, field) in [&mut self.url, &mut self.token].into_iter().enumerate() {
                let y = b.y + 20. * s + i as f32 * (h + 18. * s);
                cx.services.renderer.label(frame.layer, &field.label, Rect::new(x, y - 17. * s, w, 16. * s), 11. * s, color(0xb7c2ce), false);
                field.visit_perframe(&mut Frame { layer: frame.layer, bounds: Rect::new(x, y, w, h), clip: frame.clip }, cx);
            }
            let y = b.y + b.height - 38. * s;
            self.form.button(ConnectionChoice::Cancel, Rect::new(x, y, (w - 8. * s) / 2., 32. * s), false, false, frame, cx);
            if self.attempt.is_none() { self.form.button(ConnectionChoice::Connect, Rect::new(x + (w + 8. * s) / 2., y, (w - 8. * s) / 2., 32. * s), true, false, frame, cx); }
            return;
        }
        let connected = cx.model.epoch.is_some()
            && self.url.editor.value.trim().trim_end_matches('/') == cx.model.settings.server_url
            && self.token.editor.value.trim() == cx.model.settings.token;
        let configured = !cx.model.settings.token.is_empty();
        let connection_error = (!matches!(cx.model.connection.as_str(), "Connected" | "Connecting…" | "Not connected")).then_some(cx.model.connection.as_str());
        let error = cx.model.notice.as_deref().or(connection_error).map(str::to_owned);
        let width = (b.width - 48. * s).min(520. * s).max(240. * s);
        let inner_w = width - 48. * s;
        let intro = "Connect directly to the Tau daemon over your Tailnet.";
        let intro_h = cx.services.renderer.label_height(intro, inner_w, 16. * s, false).max(42. * s);
        let field_offset = 102. * s + intro_h + 18. * s;
        let height = (416. + if configured { 92. } else { 0. } + if error.is_some() { 84. } else { 0. }) * s + field_offset - 146. * s;
        let card = Rect::new(b.x + (b.width - width) / 2., b.y + ((b.height - height) / 2.).max(12. * s), width, height);
        let x = card.x + 24. * s;
        frame.layer.rect(b, color(0x0e141b)); frame.layer.rounded_rect(card, 12. * s, color(0x36343b));
        cx.services.renderer.label(frame.layer, "Tau", Rect::new(x, card.y + 24. * s, inner_w, 44. * s), 32. * s, color(0xe5eaf0), true);
        cx.services.renderer.label(frame.layer, concat!("Version ", env!("CARGO_PKG_VERSION"), " · Beta"), Rect::new(x, card.y + 66. * s, inner_w, 20. * s), 12. * s, color(0xb7c2ce), false);
        cx.services.renderer.label(frame.layer, intro, Rect::new(x, card.y + 102. * s, inner_w, intro_h), 16. * s, color(0xe5eaf0), false);
        let field_y = card.y + field_offset;
        for (i, field) in [&mut self.url, &mut self.token].into_iter().enumerate() {
            let rect = Rect::new(x, field_y + i as f32 * 80. * s, inner_w, 56. * s);
            field.visit_perframe(&mut Frame { layer: frame.layer, bounds: rect, clip: frame.clip }, cx);
            let label = if i == 0 { "Daemon URL" } else { "Access token" };
            let style = sanscale::Style { chain: cx.services.renderer.faces.prose[0], wrap_em: None, align: sanscale::Align::Left, line_spacing: 1. };
            let label_w = cx.services.renderer.text.shape_transient(label, &style)
                .map_or(100. * s, |block| cx.services.renderer.text.measure(block).width_em() * 12. * s + 12. * s);
            frame.layer.rect(Rect::new(x + 12. * s, rect.y - 7. * s, label_w, 16. * s), color(0x36343b));
            cx.services.renderer.label(frame.layer, label, Rect::new(x + 16. * s, rect.y - 8. * s, label_w, 18. * s), 12. * s, color(0xb7c2ce), false);
        }
        let y = field_y + 158. * s;
        if self.attempt.is_none() || cx.model.connection != "Connecting…" {
            self.form.button(ConnectionChoice::Connect, Rect::new(x + inner_w - 104. * s, y, 104. * s, 40. * s), true, false, frame, cx);
        } else {
            cx.services.renderer.label(frame.layer, "Connecting…", Rect::new(x + inner_w - 132. * s, y + 10. * s, 132. * s, 28. * s), 14. * s, color(0xe5eaf0), false);
        }
        self.form.button(ConnectionChoice::Cancel, Rect::new(x + inner_w - 204. * s, y, 88. * s, 40. * s), false, false, frame, cx);
        let mut y = y + 60. * s;
        self.form.button(ConnectionChoice::Models, Rect::new(x, y, inner_w - 80. * s, 32. * s), false, false, frame, cx);
        self.form.button(ConnectionChoice::Tools, Rect::new(x + inner_w - 72. * s, y, 72. * s, 32. * s), false, false, frame, cx); y += 44. * s;
        if configured {
            frame.layer.rect(Rect::new(x, y, inner_w, s), color(0x526170)); y += 12. * s;
            cx.services.renderer.label(frame.layer, "Daemon settings", Rect::new(x, y, inner_w, 26. * s), 16. * s, color(0xe5eaf0), true); y += 28. * s;
            if connected {
                self.form.button(ConnectionChoice::Daemon, Rect::new(x, y, (inner_w - 8. * s) / 2., 32. * s), false, false, frame, cx);
                self.form.button(ConnectionChoice::Refresh, Rect::new(x + (inner_w + 8. * s) / 2., y, (inner_w - 8. * s) / 2., 32. * s), false, false, frame, cx);
            } else {
                cx.services.renderer.label(frame.layer, "Connect to edit daemon, agent and prompt settings.", Rect::new(x, y, inner_w, 36. * s), 14. * s, color(0xe5eaf0), false);
            }
            y += 42. * s;
        }
        if let Some(error) = error {
            cx.services.renderer.label(frame.layer, &error, Rect::new(x, y, inner_w, 72. * s), 14. * s, color(0xffb4ab), false);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TopicChoice { Save, Cancel, Continue, Move, Delete }
enum TopicKind { New(String), Rename(Project), Prompt(Project), Delete(Project), Choice(Project) }
pub(in crate::app) struct TopicDialog {
    form: Form<TopicChoice>, kind: TopicKind, name: Option<TextField>, prompt: Option<TextField>,
    identity: String, lineage: Option<String>, request: Option<String>,
}
impl TopicDialog {
    fn new(edit: TopicEdit, cx: &mut Context<'_>) -> Result<Self> {
        let id = Id::new();
        let project = |id: &str| cx.model.account.projects.iter().find(|p| p.id == id).cloned().ok_or_else(|| anyhow::anyhow!("Topic no longer exists"));
        let (kind, name, prompt) = match edit {
            TopicEdit::New => (TopicKind::New(uuid::Uuid::new_v4().to_string()), Some(String::new()), Some(String::new())),
            TopicEdit::Rename(key) => {
                anyhow::ensure!(key != GENERAL_PROJECT_ID, "General cannot be renamed"); let p = project(&key)?;
                let name = p.name.clone(); (TopicKind::Rename(p), Some(name), None)
            }
            TopicEdit::Prompt(key) => { let p = project(&key)?; let prompt = p.prompt.clone(); (TopicKind::Prompt(p), None, Some(prompt)) }
            TopicEdit::Delete(key) => { anyhow::ensure!(key != GENERAL_PROJECT_ID, "General cannot be deleted"); (TopicKind::Delete(project(&key)?), None, None) }
        };
        let name = name.map(|text| TextField::new(id, "Name", Editor::line(text)));
        let label = if matches!(kind, TopicKind::New(_)) { "Topic prompt (optional)" } else { "Topic prompt" };
        let prompt = prompt.map(|text| TextField::new(id, label, Editor::new(text)));
        cx.ui.focus = name.as_ref().or(prompt.as_ref()).map(|f| f.control.target);
        Ok(Self { form: Form::new(id, &[(TopicChoice::Save, "Save"), (TopicChoice::Cancel, "Cancel"),
            (TopicChoice::Continue, "Continue…"), (TopicChoice::Move, "Move chats to General"), (TopicChoice::Delete, "Delete topic and its chats")]),
            kind, name, prompt, identity: cx.model.identity.clone(), lineage: cx.model.account.source_lineage.clone(), request: None })
    }
    fn fields(&mut self) -> Vec<&mut TextField> { self.name.iter_mut().chain(self.prompt.iter_mut()).collect() }
    fn submit(&mut self, mode: Option<DeleteProjectMode>, cx: &mut Context<'_>) -> Result<()> {
        if self.request.is_some() { return Ok(()); }
        let command = match &self.kind {
            TopicKind::New(id) => ClientCommand::CreateProject { project_id: id.clone(), name: self.name.as_ref().unwrap().editor.value.clone(), prompt: self.prompt.as_ref().unwrap().editor.value.clone() },
            TopicKind::Rename(p) => ClientCommand::UpdateProject { project_id: p.id.clone(), revision: p.revision, name: self.name.as_ref().unwrap().editor.value.clone(), prompt: p.prompt.clone() },
            TopicKind::Prompt(p) => ClientCommand::UpdateProject { project_id: p.id.clone(), revision: p.revision, name: p.name.clone(), prompt: self.prompt.as_ref().unwrap().editor.value.clone() },
            TopicKind::Delete(p) => { self.kind = TopicKind::Choice(p.clone()); self.form.begin_frame(); cx.ui.focus = None; return Ok(()); }
            TopicKind::Choice(p) => {
                let Some(mode) = mode else { return Ok(()); };
                ClientCommand::DeleteProject { project_id: p.id.clone(), revision: p.revision, mode }
            }
        };
        if let ClientCommand::CreateProject { name, prompt, .. } | ClientCommand::UpdateProject { name, prompt, .. } = &command {
            anyhow::ensure!(!name.trim().is_empty() && name.chars().count() <= MAX_PROJECT_NAME_CHARS && !name.chars().any(char::is_control), "Use a topic name of 1–{MAX_PROJECT_NAME_CHARS} characters on one line");
            anyhow::ensure!(prompt.chars().count() <= MAX_PROJECT_PROMPT_CHARS, "Topic prompt is too long");
        }
        cx.model.project_result = None;
        self.request = Some(cx.model.request(command)?); cx.ui.focus = None;
        for field in self.fields() { field.control.enabled = false; }
        Ok(())
    }
    fn title(&self) -> String {
        match &self.kind {
            TopicKind::New(_) => "New topic".into(), TopicKind::Rename(_) => "Rename topic".into(),
            TopicKind::Prompt(p) => format!("{} — topic prompt", p.name),
            TopicKind::Delete(p) => format!("Delete topic “{}”?", p.name),
            TopicKind::Choice(p) => format!("Delete “{}” — what happens to its chats?", p.name),
        }
    }
}
impl Widget for TopicDialog {
    fn handle_event(&mut self, event: &Event<'_>, cx: &mut Context<'_>) -> bool {
        if self.identity != cx.model.identity || self.lineage != cx.model.account.source_lineage {
            cx.ui.requests.push_back(Request::Close(self.form.id)); return true;
        }
        let composing = self.name.iter().chain(self.prompt.iter()).any(|f| f.editor.composing());
        if matches!(event, Event::Back) && composing {
            for field in self.fields() { field.editor.preedit(String::new(), None); }
            cx.ui.dirty = true; return true;
        }
        let choice = match event {
            Event::Tick(_) => {
                if let Some(request) = &self.request {
                    if cx.model.project_result.as_ref().is_some_and(|(id, _)| id == request) {
                        let ok = cx.model.project_result.take().unwrap().1; self.request = None;
                        if ok { cx.ui.requests.push_back(Request::Close(self.form.id)); }
                        for field in self.fields() { field.control.enabled = true; }
                        cx.ui.dirty = true;
                    } else if cx.model.epoch.is_none() {
                        self.request = None; for field in self.fields() { field.control.enabled = true; }
                        cx.model.notice = Some("Topic change unconfirmed. Reconnect and check before trying again; it was not resent.".into()); cx.ui.dirty = true;
                    }
                }
                let fields = &mut self.name.iter_mut().chain(self.prompt.iter_mut()).collect::<Vec<_>>();
                self.form.event(event, fields, cx)
            }
            Event::Back | Event::Key { key: "Escape", .. } if !composing => Some(TopicChoice::Cancel),
            Event::Submit if !composing => Some(TopicChoice::Save),
            _ => {
                let fields = &mut self.name.iter_mut().chain(self.prompt.iter_mut()).collect::<Vec<_>>();
                self.form.event(event, fields, cx)
            }
        };
        match choice {
            Some(TopicChoice::Cancel) => cx.ui.requests.push_back(Request::Close(self.form.id)),
            Some(choice) => {
                let mode = match choice { TopicChoice::Move => Some(DeleteProjectMode::MoveToGeneral), TopicChoice::Delete => Some(DeleteProjectMode::DeleteChats), _ => None };
                let result = self.submit(mode, cx); cx.report(result);
            }
            None => {}
        }
        true
    }
    fn visit_perframe(&mut self, frame: &mut Frame<'_>, cx: &mut Context<'_>) {
        let s = cx.ui.scale; let b = frame.bounds; let busy = self.request.is_some();
        let prompt = self.prompt.is_some();
        let choices = match self.kind { TopicKind::Delete(_) => vec![TopicChoice::Continue, TopicChoice::Cancel],
            TopicKind::Choice(_) => vec![TopicChoice::Move, TopicChoice::Delete, TopicChoice::Cancel], _ => vec![TopicChoice::Save, TopicChoice::Cancel] };
        self.form.begin_frame();
        frame.layer.rect(b, sanscale::Color([0., 0., 0., 0.8]));
        let width = (b.width - 24. * s).min(620. * s).max(1.);
        let height = ((if prompt { 550. } else { 340. }) * s).min((b.height - 24. * s).max(1.));
        let r = Rect::new(b.x + (b.width - width) / 2., b.y + (b.height - height) / 2., width, height);
        frame.layer.rounded_rect(r, 16. * s, color(0x111b25));
        let x = r.x + 18. * s; let w = (r.width - 36. * s).max(1.);
        cx.services.renderer.label(frame.layer, &self.title(), Rect::new(x, r.y + 16. * s, w, 48. * s), 17. * s, color(0xe5eaf0), true);
        let help = if prompt { "New chats capture this prompt. Edits do not change existing chats. Moving a chat here replaces its topic prompt." }
            else if matches!(self.kind, TopicKind::Choice(_)) { "Moving keeps the chats and their history. Deleting chats permanently removes them and cannot be undone." }
            else if matches!(self.kind, TopicKind::Delete(_)) { "Next, choose whether to keep its chats in General or permanently delete them." }
            else { "Changes appear on all connected devices." };
        let help_h = if height / s < 400. && prompt { 0. } else { 52. * s };
        if help_h > 0. { cx.services.renderer.label(frame.layer, help, Rect::new(x, r.y + 65. * s, w, help_h), 12. * s, color(0xb7c2ce), false); }
        let footer = r.y + r.height - 14. * s - choices.len() as f32 * 42. * s;
        let notice_h = if cx.model.notice.is_some() || busy { 42. * s } else { 0. };
        let mut y = r.y + 70. * s + help_h;
        let fields = self.fields(); let field_count = fields.len();
        for (i, field) in fields.into_iter().enumerate() {
            cx.services.renderer.label(frame.layer, &field.label, Rect::new(x, y, w, 18. * s), 11. * s, color(0xb7c2ce), false); y += 20. * s;
            let available = (footer - notice_h - 8. * s - y).max(1.);
            let h = if field.editor.single_line { (40. * s).min((available - (field_count - i - 1) as f32 * 48. * s).max(1.)) } else { available };
            field.control.enabled = !busy;
            field.visit_perframe(&mut Frame { layer: frame.layer, bounds: Rect::new(x, y, w, h), clip: crate::render::intersect(frame.clip, r) }, cx); y += h + 10. * s;
        }
        if notice_h > 0. { cx.services.renderer.label(frame.layer, cx.model.notice.as_deref().unwrap_or("Saving…"), Rect::new(x, footer - notice_h, w, notice_h - 4. * s), 12. * s, color(0xffb4ab), false); }
        for (i, choice) in choices.into_iter().enumerate() {
            if busy && choice != TopicChoice::Cancel { continue; }
            self.form.button(choice, Rect::new(x, footer + i as f32 * 42. * s, w, 36. * s), choice == TopicChoice::Save || choice == TopicChoice::Continue, choice == TopicChoice::Delete, frame, cx);
        }
    }
}

#[cfg(test)]
mod tests;
