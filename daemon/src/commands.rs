use std::collections::HashSet;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tracing::{debug, warn};

use crate::manager::{
    AgentManager, INTERNAL_FORK_COMMAND, SessionRuntime, bounded, session_model_from_pi_model,
};
use crate::pi::RpcProcess;
use crate::manager::PromptOutcome;
use crate::protocol::{
    PromptDisposition, SessionStatus, SlashCommand, SlashCommandArgument, SlashCommandSource,
    MAX_TITLE_CHARS,
};
use crate::state::SessionModel;

impl AgentManager {
    pub(crate) async fn run_builtin_command(
        &self,
        id: &str,
        runtime: &SessionRuntime,
        process: &RpcProcess,
        name: &str,
        arguments: &str,
        previous_status: SessionStatus,
    ) -> Result<PromptOutcome> {
        self.set_runtime_state(id, runtime, SessionStatus::Running, None, None);
        let mut accepted = false;
        let result: Result<String> = async {
            match name {
                "compact" => {
                    let mut command = json!({ "type": "compact" });
                    if !arguments.is_empty() {
                        command.as_object_mut().expect("command is an object").insert(
                            "customInstructions".to_owned(),
                            Value::String(arguments.to_owned()),
                        );
                    }
                    process.request_unbounded(command).await?;
                    accepted = true;
                    self.inner.state.touch(id).await?;
                    Ok("Context compacted.".to_owned())
                }
                "model" => {
                    let (provider, model_id) = arguments.split_once('/').filter(
                        |(provider, model_id)| !provider.is_empty() && !model_id.is_empty(),
                    ).context("Usage: /model <provider/model>")?;
                    let response = process.request(json!({
                        "type": "set_model",
                        "provider": provider,
                        "modelId": model_id,
                        "persist": true,
                    })).await?;
                    accepted = true;
                    let model = response
                        .get("data")
                        .and_then(session_model_from_pi_model)
                        .unwrap_or_else(|| SessionModel {
                            provider: provider.to_owned(),
                            model_id: model_id.to_owned(),
                        });
                    self.inner.state.set_model(id, model).await?;
                    self.inner.state.touch(id).await?;
                    Ok(format!("Model set to {provider}/{model_id}. New chats will use it too."))
                }
                "thinking" => {
                    if arguments.is_empty() || arguments.chars().any(char::is_whitespace) {
                        bail!("Usage: /thinking <level>");
                    }
                    process.request(json!({
                        "type": "set_thinking_level",
                        "level": arguments,
                    })).await?;
                    accepted = true;
                    self.inner.state.touch(id).await?;
                    Ok(format!("Thinking level set to {arguments}."))
                }
                "name" => {
                    let title = arguments.trim();
                    if title.is_empty() {
                        bail!("Usage: /name <title>");
                    }
                    if title.contains('\n') || title.contains('\r') {
                        bail!("session title must be one line");
                    }
                    if title.chars().count() > MAX_TITLE_CHARS {
                        bail!("session title is too long");
                    }
                    process.request(json!({
                        "type": "set_session_name",
                        "name": title,
                    })).await?;
                    accepted = true;
                    self.inner.state.rename(id, title.to_owned()).await?;
                    Ok(format!("Chat renamed to {title}."))
                }
                _ => bail!("unsupported Tau command /{name}"),
            }
        }
        .await;
        let notice = match result {
            Ok(notice) => notice,
            Err(error) if accepted => {
                warn!(session = id, %error, "command accepted; session metadata refresh was delayed");
                format!("/{name} accepted; metadata refresh is delayed.")
            }
            Err(error) => {
                let status = if process.is_alive() {
                    if previous_status == SessionStatus::Running {
                        SessionStatus::Running
                    } else {
                        SessionStatus::Idle
                    }
                } else {
                    SessionStatus::Error
                };
                self.set_runtime_state(
                    id,
                    runtime,
                    status,
                    (status == SessionStatus::Error).then(|| bounded(&error.to_string(), 240)),
                    None,
                );
                return Err(error);
            }
        };
        if let Err(error) = self.persist_session_file(id, process).await {
            warn!(session = id, %error, "command accepted; session path refresh was delayed");
        }
        if let Err(error) = self.refresh_runtime_status(id, runtime, process).await {
            warn!(session = id, %error, "command accepted; status refresh was delayed");
        }
        self.broadcast_sessions().await;
        Ok(PromptOutcome {
            disposition: PromptDisposition::Handled,
            notice: Some(notice),
        })
    }

    pub(crate) async fn load_slash_commands(
        &self,
        runtime: &SessionRuntime,
        process: &Arc<RpcProcess>,
        refresh: bool,
    ) -> Result<Vec<SlashCommand>> {
        if !refresh && let Some(commands) = runtime.content.lock().await.commands.clone() {
            return Ok(commands);
        }

        let response = process
            .request(json!({ "type": "get_commands" }))
            .await
            .context("Pi could not list slash commands")?;
        let records = response
            .get("data")
            .and_then(|data| data.get("commands"))
            .and_then(Value::as_array)
            .context("Pi command response had no commands")?;
        let mut commands = records
            .iter()
            .filter_map(|record| {
                let name = record.get("name")?.as_str()?.trim();
                if name.is_empty()
                    || name == INTERNAL_FORK_COMMAND
                    || name.chars().count() > 128
                    || name.chars().any(char::is_whitespace)
                {
                    return None;
                }
                let source = match record.get("source")?.as_str()? {
                    "extension" => SlashCommandSource::Extension,
                    "prompt" => SlashCommandSource::Prompt,
                    "skill" => SlashCommandSource::Skill,
                    _ => return None,
                };
                Some(SlashCommand {
                    name: name.to_owned(),
                    description: record
                        .get("description")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|description| !description.is_empty())
                        .map(|description| bounded(description, 240)),
                    source,
                    argument_hint: None,
                    arguments: Vec::new(),
                })
            })
            .collect::<Vec<_>>();
        let names = commands
            .iter()
            .map(|command| command.name.as_str())
            .collect::<HashSet<_>>();

        let model_arguments = if names.contains("model") {
            Vec::new()
        } else {
            match process
                .request(json!({ "type": "get_available_models" }))
                .await
            {
                Ok(response) => {
                    let mut arguments = response
                        .get("data")
                        .and_then(|data| data.get("models"))
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(|model| {
                            let provider = model.get("provider")?.as_str()?;
                            let model_id = model.get("id")?.as_str()?;
                            if provider.is_empty() || model_id.is_empty() {
                                return None;
                            }
                            Some(SlashCommandArgument {
                                value: bounded(&format!("{provider}/{model_id}"), 240),
                                description: model
                                    .get("name")
                                    .and_then(Value::as_str)
                                    .filter(|name| !name.is_empty())
                                    .map(|name| bounded(name, 160)),
                            })
                        })
                        .collect::<Vec<_>>();
                    arguments.sort_by(|left, right| left.value.cmp(&right.value));
                    arguments.dedup_by(|left, right| left.value == right.value);
                    arguments
                }
                Err(error) => {
                    debug!(%error, "Pi model completion is unavailable");
                    Vec::new()
                }
            }
        };
        let thinking_arguments = if names.contains("thinking") {
            Vec::new()
        } else {
            match process
                .request(json!({ "type": "get_available_thinking_levels" }))
                .await
            {
                Ok(response) => response
                    .get("data")
                    .and_then(|data| data.get("levels"))
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .filter(|level| !level.is_empty() && !level.chars().any(char::is_whitespace))
                    .map(|level| SlashCommandArgument {
                        value: level.to_owned(),
                        description: None,
                    })
                    .collect(),
                Err(error) => {
                    debug!(%error, "Pi thinking-level completion is unavailable");
                    Vec::new()
                }
            }
        };
        drop(names);

        for command in [
            SlashCommand {
                name: "compact".to_owned(),
                description: Some("Manually compact the session context".to_owned()),
                source: SlashCommandSource::Builtin,
                argument_hint: Some("[instructions]".to_owned()),
                arguments: Vec::new(),
            },
            SlashCommand {
                name: "model".to_owned(),
                description: Some("Select the Pi model".to_owned()),
                source: SlashCommandSource::Builtin,
                argument_hint: Some("<provider/model>".to_owned()),
                arguments: model_arguments,
            },
            SlashCommand {
                name: "thinking".to_owned(),
                description: Some("Set the Pi thinking level".to_owned()),
                source: SlashCommandSource::Builtin,
                argument_hint: Some("<level>".to_owned()),
                arguments: thinking_arguments,
            },
            SlashCommand {
                name: "name".to_owned(),
                description: Some("Set the Tau and Pi session name".to_owned()),
                source: SlashCommandSource::Builtin,
                argument_hint: Some("<title>".to_owned()),
                arguments: Vec::new(),
            },
        ] {
            if commands.iter().all(|existing| existing.name != command.name) {
                commands.push(command);
            }
        }
        commands.sort_by(|left, right| left.name.cmp(&right.name));
        let mut content = runtime.content.lock().await;
        if content.process.as_ref().is_some_and(|current| Arc::ptr_eq(current, process)) {
            content.commands = Some(commands.clone());
        }
        Ok(commands)
    }
}
