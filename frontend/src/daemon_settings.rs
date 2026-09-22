use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use tau_protocol::settings::Settings;

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Text,
    Line,
    Number,
    Bool,
    Json,
    Model,
    Nullable,
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
    "System",
    "Projects",
    "Providers",
    "Models",
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
                "Title prompt",
                "/daemon/titlePrompt",
                Text,
                "Exact whitespace is preserved. Empty text is intentional."
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
                "off, minimal, low, medium, high, xhigh or max. Actual support comes from the model catalog."
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
                "System prompt replacement",
                "/agent/systemPrompt",
                Nullable,
                "Built-in means null. Custom empty text means an intentionally empty replacement. They are not equivalent."
            ),
            field!(
                "Append system prompt",
                "/agent/appendSystemPrompt",
                Text,
                "Appended exactly after the replacement or built-in system prompt."
            ),
            field!(
                "Load project instructions",
                "/agent/loadProjectInstructions",
                Bool,
                "Read AGENTS.md from the filesystem root down to the daemon working directory."
            ),
        ],
        3 => &[field!(
            "Project prompt overrides",
            "/agent/projectPrompts",
            Json,
            "JSON keyed by absolute project path. Values have systemPrompt (null inherits; empty replaces with nothing) and appendSystemPrompt."
        )],
        4 => &[field!(
            "Provider endpoints",
            "/providers",
            Json,
            "JSON provider records: api, baseUrl, apiKeyEnv and webSearch. apiKeyEnv is a variable NAME, never an API key. Credentials stay on the daemon."
        )],
        _ => &[field!(
            "Model catalog",
            "/models",
            Json,
            "JSON model records: provider, id, name, contextWindow and thinkingLevelMap. Only configure capabilities confirmed by the provider."
        )],
    }
}
pub struct Draft {
    pub identity: String,
    pub revision: u64,
    pub section: usize,
    pub field: usize,
    pub builtin: bool,
    default_prompt: String,
    value: Value,
}
impl Draft {
    pub fn new(settings: &Settings, default_prompt: String, identity: String) -> Result<Self> {
        Ok(Self {
            identity,
            revision: settings.revision,
            section: 0,
            field: 0,
            builtin: false,
            default_prompt,
            value: serde_json::to_value(settings)?,
        })
    }
    pub fn builtin_text(&self) -> &str {
        &self.default_prompt
    }
    pub fn definition(&self) -> &'static Field {
        &fields(self.section)[self.field]
    }
    pub fn text(&mut self) -> Result<String> {
        let definition = self.definition();
        let value = self
            .value
            .pointer(definition.path)
            .context("Missing settings field")?;
        self.builtin = definition.kind == Kind::Nullable && value.is_null();
        Ok(match definition.kind {
            Kind::Nullable if self.builtin => self.default_prompt.clone(),
            Kind::Model => format!(
                "{}/{}",
                value["provider"].as_str().unwrap_or_default(),
                value["modelId"].as_str().unwrap_or_default()
            ),
            Kind::Text | Kind::Line | Kind::Nullable => value.as_str().unwrap_or_default().into(),
            Kind::Json => serde_json::to_string_pretty(value)?,
            _ => value.to_string(),
        })
    }
    pub fn apply(&mut self, text: &str) -> Result<()> {
        let field = self.definition();
        let value = match field.kind {
            Kind::Nullable if self.builtin => Value::Null,
            Kind::Text | Kind::Line | Kind::Nullable => json!(text),
            Kind::Number => json!(
                text.trim()
                    .parse::<u64>()
                    .context("Enter a non-negative whole number")?
            ),
            Kind::Bool => json!(text.parse::<bool>().context("Expected true or false")?),
            Kind::Json => {
                serde_json::from_str(text).context("Invalid JSON; edits have not been saved")?
            }
            Kind::Model => {
                let (provider, model) =
                    text.trim().split_once('/').context("Use provider/model")?;
                ensure!(
                    self.value["models"].as_array().is_some_and(|models| models
                        .iter()
                        .any(|m| m["provider"] == provider && m["id"] == model)),
                    "That model is not in this daemon's catalog"
                );
                json!({"provider":provider,"modelId":model})
            }
        };
        let mut candidate = self.value.clone();
        *candidate
            .pointer_mut(field.path)
            .context("Missing settings field")? = value;
        // Preserve all unedited fields; reject misspelled/secret JSON properties.
        let _: Settings =
            serde_json::from_value(candidate.clone()).context("Invalid settings field")?;
        self.value = candidate;
        Ok(())
    }
    pub fn reset(&mut self) -> Result<String> {
        let defaults = serde_json::to_value(Settings::default())?;
        let path = self.definition().path;
        *self
            .value
            .pointer_mut(path)
            .context("Missing settings field")? =
            defaults.pointer(path).context("Missing default")?.clone();
        self.text()
    }
    pub fn document(&self) -> Result<Settings> {
        Ok(serde_json::from_value(self.value.clone())?)
    }
}
