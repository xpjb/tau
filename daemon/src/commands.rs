use crate::settings::SettingsExt;
use std::{sync::Arc, collections::BTreeMap};
use anyhow::{Result, bail};
use serde_json::json;
use crate::manager::{AgentManager, PromptOutcome, SessionRuntime};
use crate::protocol::{PromptDisposition, SessionStatus, SlashCommand, SlashCommandArgument, SlashCommandSource};
use crate::state::SessionModel;

impl AgentManager {
    pub async fn commands(&self, id: &str) -> Result<Vec<SlashCommand>> {
        self.runtime(id).await?;
        let settings = self.inner.settings.get();
        let mut models = settings.models.iter().map(|m| (format!("{}/{}",m.provider,m.id), Some(m.name.clone())))
            .collect::<BTreeMap<_,_>>();
        for provider in settings.providers.keys() {
            for (id, window) in self.inner.catalog.models(&settings, provider) {
                models.insert(format!("{provider}/{id}"), Some(format!("{} token context", window)));
            }
        }
        Ok([
            ("compact", "Compact session context", "[instructions]", vec![]),
            ("model", "Select the model (also the default for new chats)", "<provider/model>", models.into_iter().map(|(value, description)| SlashCommandArgument { value, description }).collect()),
            ("thinking", "Set this chat's thinking level", "<level>", crate::settings::LEVELS.iter().map(|level| SlashCommandArgument { value:(*level).into(), description:None }).collect()),
            ("name", "Rename this chat", "<title>", vec![]),
            ("fast", "Set Codex priority service for subsequent turns", "<on|off|status>", ["on","off","status"].into_iter().map(|v| SlashCommandArgument { value:v.into(), description:None }).collect()),
        ].into_iter().map(|(name, description, hint, arguments)| SlashCommand { name:name.into(), description:Some(description.into()), source:SlashCommandSource::Builtin, argument_hint:Some(hint.into()), arguments }).collect())
    }
    pub(crate) async fn run_builtin_command(&self, id: &str, runtime: &Arc<SessionRuntime>, name: &str, arguments: &str) -> Result<PromptOutcome> {
        let mut content = runtime.content.lock().await;
        if content.agent.as_ref().unwrap().running && matches!(name, "model" | "thinking" | "compact") { bail!("Stop the current run before changing /{name}"); }
        let notice = match name {
            "model" => {
                let model: SessionModel = arguments.parse().map_err(anyhow::Error::msg)?;
                let mut settings = self.inner.settings.get(); settings.model(&model)?;
                settings.agent.model = model.clone();
                self.set_settings(settings.revision, settings).await?;
                let settings = self.inner.settings.get();
                let level = settings.agent.model_thinking_levels.get(arguments).unwrap_or(&settings.agent.thinking_level).clone();
                content.append(id,json!({"type":"model_change","provider":model.provider,"modelId":model.model_id,"thinkingLevel":level})).await?;
                self.schedule_catalog(&model.provider);
                let usage = self.context_usage(&settings, &model, None);
                self.set_runtime_state(id, runtime, SessionStatus::Idle, None, Some(usage));
                format!("Model set to {arguments}. New chats will use it too.")
            }
            "thinking" => {
                if !crate::settings::LEVELS.contains(&arguments) { bail!("Usage: /thinking <off|minimal|low|medium|high|xhigh|max>"); }
                content.append(id, json!({"type":"thinking_level_change","thinkingLevel":arguments})).await?;
                content.agent.as_mut().unwrap().thinking = arguments.into(); format!("Thinking level set to {arguments}.")
            }
            "name" => {
                if arguments.trim().is_empty() || arguments.chars().count() > crate::protocol::MAX_TITLE_CHARS || arguments.contains(['\n','\r']) { bail!("Usage: /name <title>"); }
                self.inner.state.rename(id, arguments.into(), false).await?; format!("Chat renamed to {arguments}.")
            }
            "fast" => {
                let mut settings = self.inner.settings.get();
                match arguments { "on" => settings.agent.fast_mode = true, "off" => settings.agent.fast_mode = false, "status" => {}, _ => bail!("Usage: /fast <on|off|status>") }
                if arguments != "status" { settings = self.set_settings(settings.revision, settings).await?; }
                format!("Codex priority service is {}.", if settings.agent.fast_mode { "on" } else { "off" })
            }
            "compact" => {
                let agent = content.agent.as_mut().unwrap(); agent.running = true; agent.cancel = tokio_util::sync::CancellationToken::new();
                drop(content);
                let result = self.compact(id, runtime, arguments).await;
                content = runtime.content.lock().await;
                content.agent.as_mut().unwrap().running = false;
                self.set_runtime_state(id, runtime, SessionStatus::Idle, None, Some(None));
                result?; "Context compacted.".into()
            }
            _ => bail!("Unknown command /{name}"),
        };
        drop(content); self.broadcast_sessions().await;
        Ok(PromptOutcome { disposition:PromptDisposition::Handled, notice:Some(notice) })
    }
}
