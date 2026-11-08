//! Ephemeral, source-bound device sign-in. Tokens stay on the daemon; neither
//! browser approval nor reconnect automatically repeats a model/tool operation.
use super::controls::{ButtonStyle, Controls};
use super::{Context, Controller, Event, Frame, Id, Request, Target, UiState, Widget};
use crate::{app::PlatformAction, render::color};
use sanscale::Rect;
use std::time::{Duration, Instant};
use tau_net::{ClientCommand, CodexLogin, QueueOperation, CODEX_LOGIN_URL};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Choice { Open, Copy, Retry, Cancel, Close, Resume }
pub(in crate::app) struct CodexDialog {
    pub id: Id,
    controls: Controls<Choice>,
    state: Option<CodexLogin>,
    request: Option<(String, Instant)>,
    login_id: Option<String>,
    next_poll: Instant,
    session: Option<String>,
    identity: String,
    lineage: Option<String>,
}
impl CodexDialog {
    pub fn new(session: Option<String>, cx: &mut Context<'_>) -> Self {
        let id = Id::new();
        cx.ui.focus = None;
        cx.model.codex_login_required = None;
        let mut dialog = Self {
            id, controls: Controls::declared(id, &[
                (Choice::Open, "Open Codex sign-in"), (Choice::Copy, "Copy code"),
                (Choice::Retry, "Try again"), (Choice::Cancel, "Cancel sign-in"),
                (Choice::Close, "Close"), (Choice::Resume, "Resume chat"),
            ]),
            state: None, request: None, login_id: None, next_poll: Instant::now(), session,
            identity: cx.model.identity.clone(), lineage: cx.model.account.source_lineage.clone(),
        };
        dialog.send(ClientCommand::StartCodexLogin, cx);
        dialog
    }
    fn current(&self, cx: &Context<'_>) -> bool {
        self.identity == cx.model.identity && self.lineage == cx.model.account.source_lineage
    }
    fn send(&mut self, command: ClientCommand, cx: &mut Context<'_>) {
        match cx.model.request(command) {
            Ok(id) => self.request = Some((id, Instant::now())),
            Err(error) => { self.state = Some(CodexLogin::Failed { message:error.to_string() }); self.request = None; }
        }
        cx.ui.dirty = true;
    }
    fn text(&self, connected: bool) -> String {
        match &self.state {
            None => "Getting your Codex sign-in link…".into(),
            Some(CodexLogin::Pending { user_code, .. }) => format!(
                "1. Open the sign-in page:\n{CODEX_LOGIN_URL}\n\n2. Enter this code:\n{user_code}\n\n{}\n\nThis signs this beta into your Codex account separately. Stable's login is not changed. Only enter this code on the page above; don't share it.",
                if connected { "Waiting for browser approval. The code expires within 15 minutes." } else { "Disconnected. Approval can finish in your browser; Tau will check when it reconnects." }),
            Some(CodexLogin::Complete) => "Signed in to Codex.\n\nStable's login is unchanged. No message or tool operation has been replayed. Resume your chat when you're ready.".into(),
            Some(CodexLogin::Failed { message }) => format!("{message}\n\nTry again to get a sign-in code. No terminal command is needed."),
            Some(CodexLogin::Cancelled) => "Sign-in cancelled. Your existing credentials were not replaced.".into(),
        }
    }
    #[cfg(test)]
    pub fn buttons(&self) -> Vec<(&str, Rect)> {
        self.controls.items.iter().filter_map(|(_, b, _)| b.control.rect.map(|r| (b.label.as_str(), r))).collect()
    }
}
impl Widget for CodexDialog {
    fn update(&mut self, _dt: f32, cx: &mut Context<'_>) {
        if !self.current(cx) { cx.ui.requests.push_back(Request::Close(self.id)); return; }
        if let Some((id, _)) = &self.request
            && cx.model.codex_login_result.as_ref().is_some_and(|(got, _)| got == id) {
            let (_, result) = cx.model.codex_login_result.take().unwrap();
            self.request = None;
            self.state = Some(match result {
                Ok(state) => state,
                Err(message) => CodexLogin::Failed { message },
            });
            if let Some(CodexLogin::Pending { login_id, .. }) = &self.state { self.login_id = Some(login_id.clone()); }
            self.next_poll = Instant::now() + Duration::from_secs(2);
            cx.ui.dirty = true;
        }
        if self.request.as_ref().is_some_and(|(_, at)| at.elapsed() > Duration::from_secs(35)) {
            self.request = None;
            self.state = Some(CodexLogin::Failed { message:"The sign-in request timed out. Check your connection and try again.".into() });
            cx.ui.dirty = true;
        }
        if cx.model.epoch.is_none() {
            if self.request.take().is_some() && self.state.is_none() {
                self.state = Some(CodexLogin::Failed { message:"Connect to Tau, then try again.".into() });
                cx.ui.dirty = true;
            }
        } else if self.request.is_none() && matches!(self.state, Some(CodexLogin::Pending { .. })) && Instant::now() >= self.next_poll {
            self.send(ClientCommand::GetCodexLogin { login_id:self.login_id.clone().unwrap() }, cx);
        }
    }
    fn owns(&self, target: Target, _: &Controller, _: &UiState) -> bool { self.controls.owns(target) }
    fn handle_event(&mut self, event: &Event<'_>, cx: &mut Context<'_>) -> bool {
        if !self.current(cx) { cx.ui.requests.push_back(Request::Close(self.id)); return true; }
        let choice = match event {
            Event::Back | Event::Key { key:"Escape", .. } => Some(Choice::Close),
            _ => self.controls.event(event, cx).1,
        };
        match choice {
            Some(Choice::Close) => cx.ui.requests.push_back(Request::Close(self.id)),
            Some(Choice::Open) if matches!(self.state, Some(CodexLogin::Pending { .. })) => {
                // Never open a provider-supplied arbitrary URL or put the code in a URL.
                cx.services.platform.push(PlatformAction::OpenUrl(CODEX_LOGIN_URL.into()));
            }
            Some(Choice::Copy) => if let Some(CodexLogin::Pending { user_code, .. }) = &self.state {
                cx.services.platform.push(PlatformAction::Copy(user_code.clone()));
            },
            Some(Choice::Retry) if self.request.is_none() => {
                self.state = None;
                self.send(ClientCommand::StartCodexLogin, cx);
            }
            Some(Choice::Cancel) => if let Some(login_id) = &self.login_id {
                self.send(ClientCommand::CancelCodexLogin { login_id:login_id.clone() }, cx);
            },
            Some(Choice::Resume) if matches!(self.state, Some(CodexLogin::Complete)) => {
                let result = (|| -> anyhow::Result<()> {
                    let session_id = self.session.as_ref().ok_or_else(|| anyhow::anyhow!("Select a chat to resume"))?;
                    let chat = cx.model.chats.get(session_id).ok_or_else(|| anyhow::anyhow!("Chat no longer available"))?;
                    cx.model.control(ClientCommand::QueueControl {
                        session_id:session_id.clone(), generation:chat.feed.generation.clone(),
                        operation:QueueOperation::Resume { run_id:chat.feed.queue.run_id.clone() },
                    })?;
                    cx.ui.requests.push_back(Request::Close(self.id));
                    Ok(())
                })();
                self.controls.report(result, cx);
            }
            _ => {},
        }
        true
    }
    fn visit_perframe(&mut self, frame: &mut Frame<'_>, cx: &mut Context<'_>) {
        let s = cx.ui.scale;
        let area = self.controls.page(620., "Sign in to Codex", frame, cx);
        let text = self.text(cx.model.epoch.is_some());
        let pending = matches!(self.state, Some(CodexLogin::Pending { .. }));
        let complete = matches!(self.state, Some(CodexLogin::Complete));
        let mut buttons = vec![];
        if pending { buttons.extend([(Choice::Open, ButtonStyle::Primary), (Choice::Copy, ButtonStyle::Tonal), (Choice::Cancel, ButtonStyle::Tonal)]); }
        else if complete && self.session.is_some() { buttons.push((Choice::Resume, ButtonStyle::Primary)); }
        else if !complete && self.request.is_none() { buttons.push((Choice::Retry, ButtonStyle::Primary)); }
        buttons.push((Choice::Close, ButtonStyle::Tonal));
        let button_h = 38. * s;
        let footer = area.y + area.height + 40. * s - buttons.len() as f32 * (button_h + 6. * s);
        let available = (footer - area.y - 8. * s).max(1.);
        let size = if available / s < 330. { 14. } else { 16. } * s;
        cx.services.renderer.clipped_label(frame.layer, &text, Rect::new(area.x, area.y, area.width, available), size, color(0xe5eaf0), false, frame.clip);
        for (_, button, choice) in &mut self.controls.items {
            button.control.enabled = match choice {
                Choice::Retry | Choice::Resume => cx.model.epoch.is_some() && self.request.is_none(),
                Choice::Cancel => cx.model.epoch.is_some(),
                _ => true,
            };
        }
        self.controls.stack(&buttons, Rect::new(area.x, footer, area.width, buttons.len() as f32 * (button_h + 6. * s)), frame, cx);
    }
}

#[cfg(all(test, not(target_os = "android")))]
#[path = "../../../tests/unit/app/ui/codex_login.rs"]
mod tests;
