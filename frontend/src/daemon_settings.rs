use anyhow::{Context, Result};
use serde_json::{Value, json};
use tau_protocol::{SessionModel, settings::Settings};

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Text,
    Line,
    Number,
    Bool,
    Json,
    Model,
    OptionalModel,
    PromptModel,
    PromptOverride,
}
pub struct Field {
    pub name: &'static str,
    pub path: &'static str,
    pub kind: Kind,
    pub help: &'static str,
}
macro_rules! field {
    ($name:literal,$path:literal,$kind:ident,$help:literal) => {
        Field {
            name: $name,
            path: $path,
            kind: Kind::$kind,
            help: $help,
        }
    };
}
pub const SECTIONS: &[&str] = &[
    "Daemon",
    "Agent",
    "Prompts",
    "Providers",
    "Model metadata",
];
pub fn fields(section: usize) -> &'static [Field] {
    match section {
        0 => &[
            field!(
                "Generate titles with the model",
                "/daemon/generateTitles",
                Bool,
                "Optional extra provider request. Off uses the first nonempty prompt line. Title generation never delays prompt acknowledgement."
            ),
            field!(
                "Title model",
                "/daemon/titleModel",
                OptionalModel,
                "Optional provider/model used only for titles. Leave empty to use the chat model. This does not change the chat or new-chat model."
            ),
            field!(
                "Title prompt",
                "/daemon/titlePrompt",
                Text,
                "Use {text} for the first user message. Exact whitespace is preserved. Empty text is intentional."
            ),
            field!(
                "Idle runtime timeout (seconds)",
                "/daemon/idleTimeoutSeconds",
                Number,
                "Evicts idle in-memory agents, not saved history. Zero disables eviction. This is not the provider-cache TTL."
            ),
        ],
        1 => &[
            field!(
                "Last chosen / new-chat model",
                "/agent/model",
                Model,
                "provider/model. Explicit chat model choices update this default. Quick-select favorites do not."
            ),
            field!(
                "Default thinking level",
                "/agent/thinkingLevel",
                Line,
                "off, minimal, low, medium, high, xhigh or max. The provider determines support; optional model metadata can map levels."
            ),
            field!(
                "Per-model thinking defaults",
                "/agent/modelThinkingLevels",
                Json,
                "JSON object mapping provider/model to a thinking level."
            ),
            field!(
                "Queued prompt mode",
                "/agent/steeringMode",
                Line,
                "all or one-at-a-time. Native queue boundaries are turns."
            ),
            field!(
                "Codex priority service",
                "/agent/fastMode",
                Bool,
                "Applies to subsequent Codex requests and may change billing."
            ),
            field!(
                "Retry policy",
                "/agent/retry",
                Json,
                "enabled, maxRetries and baseDelayMs. Started responses/tools are not blindly replayed."
            ),
            field!(
                "Compaction policy",
                "/agent/compaction",
                Json,
                "enabled, reserveTokens, keepRecentTokens, nativeCodex and timeoutSeconds. Saved history is retained."
            ),
            field!(
                "HTTP idle timeout (seconds)",
                "/agent/httpIdleTimeoutSeconds",
                Number,
                "Maximum idle time while waiting for provider data."
            ),
            field!(
                "Tool output limit (bytes)",
                "/agent/maxToolOutputBytes",
                Number,
                "Large tool output is kept in a daemon-side file; the model receives a bounded result."
            ),
            field!(
                "Shell executable",
                "/agent/shellPath",
                Line,
                "Absolute daemon-side executable path, such as /bin/bash."
            ),
            field!(
                "Shell command prefix",
                "/agent/shellCommandPrefix",
                Text,
                "Exact daemon-side shell prefix. This is executable configuration, not a prompt."
            ),
        ],
        2 => &[
            field!(
                "Default system prompt",
                "/agent/systemPrompt",
                Text,
                "Used when a model has no override. This is saved text, not a hidden fallback. Empty text is intentional. Tools are separate."
            ),
            field!(
                "Model to edit",
                "",
                PromptModel,
                "Enter any exact provider/model ID, then use the next field to edit its prompt. No model catalog is required."
            ),
            field!(
                "Model system prompt",
                "/agent/modelSystemPrompts",
                PromptOverride,
                "Model override → default prompt. Inherit uses the saved default; an empty override replaces it with nothing. Tools and AGENTS.md context remain separate."
            ),
            field!(
                "Load AGENTS.md files",
                "/agent/loadAgentsFiles",
                Bool,
                "Read AGENTS.md from the filesystem root down to the working directory. This adds file context; it is not another prompt fallback."
            ),
            field!(
                "All model prompt overrides",
                "/agent/modelSystemPrompts",
                Json,
                "Optional JSON view: provider/model keys map to exact prompt strings. An empty string is an override; remove the key to inherit the default."
            ),
        ],
        3 => &[field!(
            "Provider endpoints",
            "/providers",
            Json,
            "JSON provider records: api, baseUrl, apiKeyEnv and webSearch. apiKeyEnv is a variable NAME, never an API key. Credentials stay on the daemon."
        )],
        _ => &[field!(
            "Optional model metadata",
            "/models",
            Json,
            "Optional name and thinkingLevelMap suggestions for each provider/id. Not an allowlist. Context limits come only from Tau's saved authenticated provider catalog. Refresh it from Connection settings."
        )],
    }
}
pub struct Draft {
    pub identity: String,
    pub revision: u64,
    pub section: usize,
    pub field: usize,
    pub inherit: bool,
    pub prompt_model: String,
    value: Value,
}
impl Draft {
    pub fn new(settings: &Settings, identity: String) -> Result<Self> {
        Ok(Self {
            identity,
            revision: settings.revision,
            section: 2,
            field: 0,
            inherit: false,
            prompt_model: format!("{}/{}", settings.agent.model.provider, settings.agent.model.model_id),
            value: serde_json::to_value(settings)?,
        })
    }
    pub fn default_prompt(&self) -> &str {
        self.value["agent"]["systemPrompt"].as_str().unwrap_or_default()
    }
    pub fn definition(&self) -> &'static Field {
        &fields(self.section)[self.field]
    }
    pub fn text(&mut self) -> Result<String> {
        let definition = self.definition();
        if definition.kind == Kind::PromptModel { return Ok(self.prompt_model.clone()); }
        let value = self.value.pointer(definition.path).context("Missing settings field")?;
        if definition.kind == Kind::PromptOverride {
            let prompt = value.get(&self.prompt_model).and_then(Value::as_str);
            self.inherit = prompt.is_none();
            return Ok(prompt.unwrap_or_else(|| self.default_prompt()).to_owned());
        }
        Ok(match definition.kind {
            Kind::Model | Kind::OptionalModel if !value.is_null() => format!(
                "{}/{}", value["provider"].as_str().unwrap_or_default(), value["modelId"].as_str().unwrap_or_default()
            ),
            Kind::OptionalModel => String::new(),
            Kind::Text | Kind::Line => value.as_str().unwrap_or_default().into(),
            Kind::Json => serde_json::to_string_pretty(value)?,
            _ => value.to_string(),
        })
    }
    pub fn apply(&mut self, text: &str) -> Result<()> {
        let field = self.definition();
        if field.kind == Kind::PromptModel {
            let _: SessionModel = text.trim().parse().map_err(anyhow::Error::msg)?;
            self.prompt_model = text.trim().into();
            return Ok(());
        }
        let mut candidate = self.value.clone();
        let dest = candidate.pointer_mut(field.path).context("Missing settings field")?;
        if field.kind == Kind::PromptOverride {
            let prompts = dest.as_object_mut().context("Invalid prompt overrides")?;
            if self.inherit { prompts.remove(&self.prompt_model); }
            else { prompts.insert(self.prompt_model.clone(), json!(text)); }
        } else {
            *dest = match field.kind {
                Kind::Text | Kind::Line => json!(text),
                Kind::Number => json!(text.trim().parse::<u64>().context("Enter a non-negative whole number")?),
                Kind::Bool => json!(text.parse::<bool>().context("Expected true or false")?),
                Kind::Json => serde_json::from_str(text).context("Invalid JSON; edits have not been saved")?,
                Kind::OptionalModel if text.trim().is_empty() => Value::Null,
                Kind::Model | Kind::OptionalModel => {
                    let model: SessionModel = text.trim().parse().map_err(anyhow::Error::msg)?;
                    serde_json::to_value(model)?
                }
                Kind::PromptModel | Kind::PromptOverride => unreachable!(),
            };
        }
        let settings: Settings = serde_json::from_value(candidate).context("Invalid settings field")?;
        self.value = serde_json::to_value(settings)?;
        Ok(())
    }
    pub fn reset(&mut self) -> Result<String> {
        match self.definition().kind {
            Kind::PromptOverride => {
                self.inherit = true;
                self.apply("")?;
            }
            Kind::PromptModel => {
                self.prompt_model = format!("{}/{}", self.value["agent"]["model"]["provider"].as_str().unwrap_or_default(),
                    self.value["agent"]["model"]["modelId"].as_str().unwrap_or_default());
            }
            _ => {
                let defaults = serde_json::to_value(Settings::default())?;
                let path = self.definition().path;
                *self.value.pointer_mut(path).context("Missing settings field")? = defaults.pointer(path).context("Missing default")?.clone();
            }
        }
        self.text()
    }
    pub fn document(&self) -> Result<Settings> {
        Ok(serde_json::from_value(self.value.clone())?)
    }
}
