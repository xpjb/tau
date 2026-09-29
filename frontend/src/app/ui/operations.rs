//! Short, instance-bound forms. The field and each choice live on the dialog;
//! submission uses the captured destination, never a current-chat field index.
use super::controls::{ButtonStyle, Form, TextField};
use super::{Context, Controller, DialogSpec, Event, Frame, Id, Request, Target, UiState, Widget};
use crate::{editor::Editor, render::color};
use anyhow::Result;
use sanscale::Rect;
use tau_protocol::*;

pub(in crate::app) enum Operation {
    Rename(String),
    Delete(String),
    Refresh,
    Agent(String, String),
    Queue { session: String, id: String, revision: u64, text: String },
    Link(String),
    Outbox(usize),
    Inspect(String),
    ForgetControl(String),
    ForgetRecovered(String),
    Review(String),
}
#[derive(Clone, PartialEq, Eq)]
enum Choice {
    Submit,
    Close,
    Page(usize),
    Inspect(String),
    Copy(String),
    Check(String),
    Retry(String),
    Forget(String),
}
pub(in crate::app) struct OperationDialog {
    pub id: Id,
    pub title: String,
    pub value: Option<TextField>,
    form: Form<Choice>,
    operation: Operation,
    identity: String,
    lineage: Option<String>,
}
impl OperationDialog {
    pub fn new(operation: Operation, cx: &mut Context<'_>) -> Result<Self> {
        let id = Id::new();
        let (title, field, submit) = match &operation {
            Operation::Rename(session) => ("Rename chat".into(), Some(("Title", Editor::line(cx.model.account.sessions.iter().find(|s| &s.id == session).map(|s| s.title.clone()).unwrap_or_default()))), "Save"),
            Operation::Delete(_) => ("Permanently delete this chat and its files?".into(), None, "Delete permanently"),
            Operation::Refresh => {
                let provider = cx.model.daemon_settings.as_ref().map(|s| s.agent.model.provider.clone())
                    .or_else(|| cx.model.account.sessions.iter().find(|s| Some(&s.id) == cx.model.account.selected.as_ref()).and_then(|s| s.model.as_ref().map(|m| m.provider.clone())))
                    .unwrap_or_else(|| "openai-codex".into());
                ("Refresh model catalog".into(), Some(("Provider name", Editor::line(provider))), "Refresh")
            }
            Operation::Agent(session, command) => {
                let chat = cx.model.account.sessions.iter().find(|s| &s.id == session);
                let (label, value) = match command.as_str() {
                    "model" => ("provider/model", chat.and_then(|s| s.model.as_ref()).map(|m| format!("{}/{}", m.provider, m.model_id)).unwrap_or_default()),
                    "thinking" => ("off / minimal / low / medium / high / xhigh / max", chat.and_then(|s| s.thinking_level.clone()).unwrap_or_default()),
                    "fast" => ("on / off / status", String::new()),
                    _ => ("Optional compaction instructions", String::new()),
                };
                (format!("Chat {command}"), Some((label, Editor::line(value))), "Apply")
            }
            Operation::Queue { text, .. } => ("Edit queued message".into(), Some(("Message", Editor::new(text.clone()))), "Save"),
            Operation::Link(url) => {
                let parsed = url::Url::parse(url)?;
                anyhow::ensure!(matches!(parsed.scheme(), "https" | "http" | "mailto"), "Only web and mail links can be opened");
                (format!("Open link?\n{url}"), None, "Open")
            }
            Operation::Outbox(_) => ("Saved immutable actions".into(), None, ""),
            Operation::Inspect(key) => (format!("FixtureChoice {key}"), None, ""),
            Operation::ForgetControl(_) => ("Forget this saved intent? This does NOT undo or cancel a daemon action.".into(), None, "Forget locally"),
            Operation::ForgetRecovered(_) => ("Forget this local chat, its drafts and files? This does not undo or cancel source work. Saved daemon actions remain in Settings.".into(), None, "Forget local chat"),
            Operation::Review(_) => ("Restored history may omit external effects or paid work. Inspect those outcomes first. This acknowledgment only permits future explicit execution; it does not resume or resend anything.".into(), None, "Allow future explicit execution"),
        };
        let mut buttons = Vec::new();
        match &operation {
            Operation::Outbox(page) => {
                for (key, saved) in cx.model.account.pending_controls.iter().skip(page * 5).take(5) {
                    let kind = serde_json::to_value(&saved.request.command)?
                        .get("type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_owned();
                    buttons.push((
                        Choice::Inspect(key.clone()),
                        format!(
                            "{kind} · {}",
                            if saved.blocked {
                                "needs reconciliation"
                            } else if saved.accepted {
                                "accepted"
                            } else {
                                "unconfirmed"
                            }
                        ),
                    ));
                }
                if *page > 0 {
                    buttons.push((Choice::Page(page - 1), "Previous".into()));
                }
                if (page + 1) * 5 < cx.model.account.pending_controls.len() {
                    buttons.push((Choice::Page(page + 1), "Next".into()));
                }
            }
            Operation::Inspect(key) => {
                let saved = cx
                    .model
                    .account
                    .pending_controls
                    .get(key)
                    .ok_or_else(|| anyhow::anyhow!("FixtureChoice already reconciled"))?;
                buttons.extend([
                    (Choice::Copy(serde_json::to_string_pretty(&saved.request)?), "Copy complete saved intent".into()),
                    (Choice::Check(key.clone()), "Check daemon receipt (no execution)".into()),
                    (Choice::Retry(key.clone()), "Explicitly retry original ID".into()),
                    (Choice::Forget(key.clone()), "Forget local intent…".into()),
                    (Choice::Page(0), "Back".into()),
                ]);
            }
            _ => buttons.push((Choice::Submit, submit.into())),
        }
        buttons.push((
            Choice::Close,
            if matches!(operation, Operation::Outbox(_) | Operation::Inspect(_)) { "Close" } else { "Cancel" }.into(),
        ));
        let form = Form::new(id, &buttons.iter().map(|(a, s)| (a.clone(), s.as_str())).collect::<Vec<_>>());
        let value = field.map(|(label, e)| TextField::new(id, label, e));
        cx.ui.focus = value.as_ref().map(|f| f.control.target);
        Ok(Self {
            id,
            title,
            value,
            form,
            operation,
            identity: cx.model.identity.clone(),
            lineage: cx.model.account.source_lineage.clone(),
        })
    }
    fn submit(&mut self, cx: &mut Context<'_>) -> Result<()> {
        let value = self.value.as_ref().map(|f| f.editor.value.clone()).unwrap_or_default();
        match &self.operation {
            Operation::Rename(id) => {
                cx.model.request(ClientCommand::RenameSession { session_id: id.clone(), title: value })?;
            }
            Operation::Delete(id) => {
                cx.model.request(ClientCommand::DeleteSession { session_id: id.clone() })?;
            }
            Operation::Refresh => {
                let provider = value.trim();
                anyhow::ensure!(
                    !provider.is_empty() && provider.len() <= 120 && !provider.chars().any(char::is_whitespace),
                    "Enter a configured provider name"
                );
                cx.model.request(ClientCommand::RefreshModelCatalog { provider: provider.into() })?;
                cx.model.notice = Some(format!("Refreshing {provider} model catalog…").into());
            }
            Operation::Agent(session, command) => {
                cx.model.ensure_chat(session)?;
                cx.model.control(ClientCommand::Prompt {
                    session_id: session.clone(),
                    text: format!("/{command} {value}"),
                })?;
            }
            Operation::Queue { session, id, revision, .. } => {
                let generation = cx
                    .model
                    .chats
                    .get(session)
                    .ok_or_else(|| anyhow::anyhow!("Chat no longer available"))?
                    .feed
                    .generation
                    .clone();
                cx.model.control(ClientCommand::QueueControl {
                    session_id: session.clone(),
                    generation,
                    operation: QueueOperation::Edit { request_id: id.clone(), revision: *revision, text: value },
                })?;
            }
            Operation::Link(url) => cx.services.platform.push(super::PlatformAction::OpenUrl(url.clone())),
            Operation::ForgetControl(id) => cx.model.forget_control(id)?,
            Operation::ForgetRecovered(id) => cx.model.forget_missing_chat(id)?,
            Operation::Review(id) => {
                cx.model.request(ClientCommand::ReviewRestore { session_id: id.clone() })?;
            }
            Operation::Outbox(_) | Operation::Inspect(_) => {}
        }
        cx.ui.requests.push_back(Request::Close(self.id));
        Ok(())
    }
    pub fn buttons(&self) -> Vec<(&str, Rect)> {
        self.form.buttons.iter().filter_map(|(_, b)| b.control.rect.map(|r| (b.label.as_str(), r))).collect()
    }
}
impl Widget for OperationDialog {
    fn owns(&self, target: Target, _model: &Controller, _ui: &UiState) -> bool {
        self.form.owns(target) || self.value.as_ref().is_some_and(|f| f.control.target == target)
    }

    fn handle_event(&mut self, event: &Event<'_>, cx: &mut Context<'_>) -> bool {
        if self.identity != cx.model.identity || self.lineage != cx.model.account.source_lineage {
            cx.ui.requests.push_back(Request::Close(self.id));
            return true;
        }
        let composing = self.value.as_ref().is_some_and(|f| f.editor.composing());
        let choice = match event {
            Event::Back | Event::Key { key: "Escape", .. } if !composing => Some(Choice::Close),
            Event::Submit if !composing => Some(Choice::Submit),
            Event::Key { key: "Enter", shift: false, .. }
                if !composing && !cx.ui.mobile && matches!(self.operation, Operation::Rename(_)) =>
            {
                Some(Choice::Submit)
            }
            _ => self.form.event(event, self.value.iter_mut(), cx).1,
        };
        let result = match choice {
            Some(Choice::Close) => {
                cx.ui.requests.push_back(Request::Close(self.id));
                Ok(())
            }
            Some(Choice::Submit) => self.submit(cx),
            Some(Choice::Page(page)) => {
                cx.ui.requests.push_back(Request::Replace {
                    owner: self.id,
                    spec: DialogSpec::Operation(Operation::Outbox(page)),
                });
                Ok(())
            }
            Some(Choice::Inspect(key)) => {
                let operation = Operation::Inspect(key);
                cx.ui.requests.push_back(Request::Replace { owner: self.id, spec: DialogSpec::Operation(operation) });
                Ok(())
            }
            Some(Choice::Forget(key)) => {
                cx.ui.requests.push_back(Request::Replace {
                    owner: self.id,
                    spec: DialogSpec::Operation(Operation::ForgetControl(key)),
                });
                Ok(())
            }
            Some(Choice::Copy(text)) => {
                cx.services.platform.push(super::PlatformAction::Copy(text));
                Ok(())
            }
            Some(Choice::Check(key)) => cx.model.check_control(&key).map(|()| {
                cx.model.notice = Some("Checking the original operation; nothing is being reexecuted".into());
            }),
            Some(Choice::Retry(key)) => cx.model.retry_control(&key).map(|()| {
                cx.model.notice = Some(
                    "Submitted the original immutable ID; uncertain effects are not automatically repeated".into(),
                );
            }),
            None => return true,
        };
        self.form.report(result, cx);
        true
    }
    fn visit_perframe(&mut self, frame: &mut Frame<'_>, cx: &mut Context<'_>) {
        let b = frame.bounds;
        let s = cx.ui.scale;
        self.form.begin_frame();
        frame.layer.rect(b, sanscale::Color([0., 0., 0., 0.8]));
        let w = (b.width - 24. * s).min(620. * s).max(1.);
        let x = b.x + (b.width - w) / 2.;
        let title_h = cx.services.renderer.label_height(&self.title, w - 40. * s, 17. * s, true).max(36. * s);
        let count = self.form.buttons.len();
        let feedback = cx.model.notice.as_ref().filter(|n| n.download.is_none()).map(|n| n.to_string());
        let feedback_h = feedback.as_ref().map_or(0., |text| {
            (cx.services.renderer.label_height(text, w - 40. * s, 13. * s, false) + 12. * s).min(100. * s)
        });
        let button_h = (36. * s).min((b.height - title_h - 50. * s).max(count as f32) / count as f32);
        let field_h = if self.value.is_some() {
            (if matches!(self.operation, Operation::Queue { .. }) { 180. } else { 48. }) * s
        } else {
            0.
        };
        let height = (title_h
            + 40. * s
            + field_h
            + feedback_h
            + count as f32 * (button_h + 6. * s)
            + if self.value.is_some() { 24. * s } else { 0. })
        .min(b.height - 16. * s);
        let y = b.y + (b.height - height) / 2.;
        frame.layer.rounded_rect(Rect::new(x, y, w, height), 16. * s, color(0x111b25));
        cx.services.renderer.label(
            frame.layer,
            &self.title,
            Rect::new(x + 20. * s, y + 12. * s, w - 40. * s, title_h),
            17. * s,
            color(0xe5eaf0),
            true,
        );
        let footer = y + height - 10. * s - count as f32 * (button_h + 6. * s);
        if let Some(field) = &mut self.value {
            let top = y + title_h + 20. * s;
            field.labeled(
                Rect::new(x + 20. * s, top, w - 40. * s, (footer - feedback_h - top - 4. * s).max(1.)),
                frame,
                cx,
            );
        }
        frame.feedback(Rect::new(x + 20. * s, footer - feedback_h, w - 40. * s, feedback_h), "", cx);
        // These are the fixed owned choices; don't build a list to look them up again.
        for (i, (choice, button)) in self.form.buttons.iter_mut().enumerate() {
            button.style = if *choice == Choice::Submit { ButtonStyle::Primary } else { ButtonStyle::Tonal };
            frame.visit(
                Rect::new(x + 20. * s, footer + i as f32 * (button_h + 6. * s), w - 40. * s, button_h),
                button,
                cx,
            );
        }
    }
}
