use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use anyhow::{Context, Result, bail};
use serde_json::Value;
use tokio::sync::Mutex;

use crate::state::SessionModel;

pub use tau_protocol::settings::*;

pub(crate) trait SettingsExt {
    fn model(&self, selected: &SessionModel) -> Result<Cow<'_, ModelSettings>>;
    fn validate(&self) -> Result<()>;
    fn system_prompt(&self, selected: &SessionModel, cwd: &Path) -> impl std::future::Future<Output = Result<String>> + Send;
}
impl SettingsExt for Settings {
    fn model(&self, selected: &SessionModel) -> Result<Cow<'_, ModelSettings>> {
        let slug = format!("{}/{}", selected.provider, selected.model_id);
        let parsed: SessionModel = slug.parse().map_err(anyhow::Error::msg)?;
        if parsed != *selected { bail!("Invalid provider/model"); }
        if !self.providers.contains_key(&selected.provider) { bail!("Configure provider {} in daemon settings", selected.provider); }
        Ok(match self.models.iter().find(|model| model.provider == selected.provider && model.id == selected.model_id) {
            Some(model) => Cow::Borrowed(model),
            None => Cow::Owned(ModelSettings { provider: selected.provider.clone(), id: selected.model_id.clone(), ..ModelSettings::default() }),
        })
    }
    fn validate(&self) -> Result<()> {
        if self.schema != 2 { bail!("Unsupported settings schema {}", self.schema); }
        if self.models.len() > 4096 { bail!("Configure at most 4096 model metadata records"); }
        self.model(&self.agent.model)?;
        if let Some(model) = &self.daemon.title_model { self.model(model)?; }
        let mut ids = std::collections::HashSet::new();
        for model in &self.models {
            let slug = format!("{}/{}", model.provider, model.id);
            let parsed: SessionModel = slug.parse().map_err(anyhow::Error::msg)?;
            if parsed.provider != model.provider || model.context_window.is_some_and(|n| !(1024..=100_000_000).contains(&n))
                || !self.providers.contains_key(&model.provider) || !ids.insert((&model.provider, &model.id)) {
                bail!("Invalid or duplicate model {}", model.id);
            }
        }
        for provider in self.providers.values() {
            let url = reqwest::Url::parse(&provider.base_url).context("Invalid provider URL")?;
            if !url.username().is_empty() || url.password().is_some() || url.query().is_some() || url.fragment().is_some()
                || !(url.scheme() == "https" || url.scheme() == "http" && matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))) {
                bail!("Provider URL must use HTTPS (HTTP is allowed only on loopback), without credentials, query, or fragment");
            }
        }
        if std::iter::once(&self.agent.thinking_level).chain(self.agent.model_thinking_levels.values()).any(|level| !LEVELS.contains(&level.as_str())) {
            bail!("Invalid thinking level");
        }
        if self.agent.retry.max_retries > 10 || self.agent.retry.base_delay_ms > 60_000
            || !(1..=3600).contains(&self.agent.http_idle_timeout_seconds)
            || !(1..=3600).contains(&self.agent.compaction.timeout_seconds)
            || self.agent.compaction.reserve_tokens < 256 || self.agent.compaction.keep_recent_tokens > 10_000_000
            || !(1024..=1024 * 1024).contains(&self.agent.max_tool_output_bytes)
            || self.daemon.idle_timeout_seconds > 604800 || !self.agent.shell_path.is_absolute() {
            bail!("Invalid agent limits");
        }
        for slug in self.agent.model_system_prompts.keys() {
            let _: SessionModel = slug.parse().map_err(anyhow::Error::msg)?;
        }
        for text in [&self.daemon.title_prompt, &self.agent.system_prompt].into_iter().chain(self.agent.model_system_prompts.values()) {
            if text.chars().count() > crate::protocol::MAX_PROMPT_CHARS { bail!("Prompt is too long"); }
        }
        if serde_json::to_vec(self)?.len() > 900_000 { bail!("Settings exceed 900 KB"); }
        Ok(())
    }

    async fn system_prompt(&self, selected: &SessionModel, cwd: &Path) -> Result<String> {
        let slug = format!("{}/{}", selected.provider, selected.model_id);
        let mut prompt = self.agent.model_system_prompts.get(&slug).unwrap_or(&self.agent.system_prompt).clone();
        if self.agent.load_agents_files {
            for directory in cwd.ancestors().collect::<Vec<_>>().into_iter().rev() {
                let path = directory.join("AGENTS.md");
                match tokio::fs::read_to_string(&path).await {
                    Ok(text) if text.len() <= 1024 * 1024 => prompt.push_str(&format!("\n\nAGENTS.md instructions ({}):\n{text}", path.display())),
                    Ok(_) => bail!("AGENTS.md instructions {} exceed 1 MB", path.display()),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
                    Err(error) => return Err(error).context("Could not read AGENTS.md instructions"),
                }
            }
        }
        prompt.push_str(&format!("\n\nCurrent working directory: {}", cwd.display()));
        Ok(prompt)
    }
}

#[derive(Clone)]
pub struct SettingsStore { path: PathBuf, value: Arc<RwLock<Settings>>, gate: Arc<Mutex<()>> }
impl SettingsStore {
    pub async fn load(config: &crate::config::Config, title_prompt: String) -> Result<Self> {
        let settings = match tokio::fs::read(&config.settings_path).await {
            Ok(bytes) => {
                let mut value: Value = serde_json::from_slice(&bytes).context("Invalid daemon settings")?;
                if value["schema"] == 1 {
                    let agent = value["agent"].as_object_mut().context("Invalid agent settings")?;
                    if let Some(projects) = agent.remove("projectPrompts")
                        && !projects.as_object().is_some_and(|p| p.is_empty()) {
                        bail!("Legacy directory prompt settings were removed; put their text in the default or model prompt before upgrading");
                    }
                    let mut prompt = match agent.remove("systemPrompt") {
                        Some(Value::String(text)) => text,
                        None | Some(Value::Null) => DEFAULT_SYSTEM_PROMPT.into(),
                        _ => bail!("Invalid default system prompt"),
                    };
                    if let Some(append) = agent.remove("appendSystemPrompt") {
                        let append = append.as_str().context("Invalid append prompt")?;
                        if !append.is_empty() { prompt.push_str("\n\n"); prompt.push_str(append); }
                    }
                    agent.insert("systemPrompt".into(), Value::String(prompt));
                    if let Some(enabled) = agent.remove("loadProjectInstructions") { agent.insert("loadAgentsFiles".into(), enabled); }
                    value["schema"] = Value::from(2);
                }
                serde_json::from_value::<Settings>(value).context("Invalid daemon settings")?
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let mut settings = Settings::default();
                settings.daemon.title_prompt = title_prompt;
                if let Some(dir) = &config.import_pi_dir {
                    let legacy: Value = serde_json::from_slice(&tokio::fs::read(dir.join("settings.json")).await?)?;
                    if let Some(v) = legacy.get("defaultProvider").and_then(Value::as_str) { settings.agent.model.provider = v.into(); }
                    if let Some(v) = legacy.get("defaultModel").and_then(Value::as_str) { settings.agent.model.model_id = v.into(); }
                    if let Some(v) = legacy.get("defaultThinkingLevel").and_then(Value::as_str) { settings.agent.thinking_level = v.into(); }
                    if let Some(v) = legacy.get("steeringMode") { settings.agent.steering_mode = serde_json::from_value(v.clone())?; }
                    if let Some(v) = legacy.get("modelThinkingLevels") { settings.agent.model_thinking_levels = serde_json::from_value(v.clone())?; }
                    if let Some(v) = legacy.get("compaction") { settings.agent.compaction = serde_json::from_value(v.clone())?; }
                    if let Some(v) = legacy.get("retry") {
                        for (field, dest) in [("enabled", "enabled"), ("maxRetries", "maxRetries"), ("baseDelayMs", "baseDelayMs")] {
                            if let Some(value) = v.get(field) {
                                let mut retry = serde_json::to_value(&settings.agent.retry)?;
                                retry[dest] = value.clone(); settings.agent.retry = serde_json::from_value(retry)?;
                            }
                        }
                    }
                    if let Some(v) = legacy.get("shellPath").and_then(Value::as_str) { settings.agent.shell_path = v.into(); }
                    if let Some(v) = legacy.get("shellCommandPrefix").and_then(Value::as_str) { settings.agent.shell_command_prefix = v.into(); }
                    if let Some(text) = optional_text(&dir.join("SYSTEM.md")).await? { settings.agent.system_prompt = text; }
                    if let Some(text) = optional_text(&dir.join("APPEND_SYSTEM.md")).await? && !text.is_empty() {
                        settings.agent.system_prompt.push_str("\n\n"); settings.agent.system_prompt.push_str(&text);
                    }
                    if let Some(text) = optional_text(&dir.join("codex-fast-mode.json")).await? {
                        let raw: Value = serde_json::from_str(&text)?;
                        settings.agent.fast_mode = raw.get("enabled").and_then(Value::as_bool).unwrap_or(false);
                    }
                    if let Some(text) = optional_text(&dir.join("codex-compaction.json")).await? {
                        let raw: Value = serde_json::from_str(&text)?;
                        settings.agent.compaction.native_codex = raw.pointer("/providers/openai-codex").and_then(Value::as_str) == Some("native");
                    }
                    if let Some(text) = optional_text(&dir.join("models-store.json")).await? {
                        let raw: Value = serde_json::from_str(&text)?;
                        let mut models = Vec::new();
                        for provider in settings.providers.keys() {
                            for model in raw.get(provider).and_then(|v| v.get("models")).and_then(Value::as_array).into_iter().flatten() {
                                models.push(ModelSettings { provider: provider.clone(), id: model["id"].as_str().context("Legacy model has no ID")?.into(),
                                    name: model["name"].as_str().unwrap_or_default().into(), context_window: model["contextWindow"].as_u64(),
                                    thinking_level_map: if model.get("reasoning").and_then(Value::as_bool) == Some(false) {
                                        LEVELS.iter().map(|level| ((*level).to_owned(), None)).collect()
                                    } else { model.get("thinkingLevelMap").map(|v| serde_json::from_value(v.clone())).transpose()?.unwrap_or_default() } });
                            }
                        }
                        if !models.is_empty() { settings.models = models; }
                    }
                    settings.validate()?;
                    let auth = config.settings_path.with_file_name("auth.json");
                    if !auth.try_exists()? && let Some(text) = optional_text(&dir.join("auth.json")).await? {
                        let _: Value = serde_json::from_str(&text).context("Invalid legacy credentials (values withheld)")?;
                        atomic_write(&auth, text.as_bytes()).await?;
                    }
                }
                settings.validate()?;
                atomic_write(&config.settings_path, &serde_json::to_vec_pretty(&settings)?).await?;
                settings
            }
            Err(error) => return Err(error).context("Could not read daemon settings"),
        };
        settings.validate()?;
        Ok(Self { path: config.settings_path.clone(), value: Arc::new(RwLock::new(settings)), gate: Arc::new(Mutex::new(())) })
    }
    pub fn get(&self) -> Settings { self.value.read().unwrap_or_else(|e| e.into_inner()).clone() }
    pub async fn set(&self, expected: u64, mut settings: Settings) -> Result<Settings> {
        let _guard = self.gate.lock().await;
        if self.get().revision != expected { bail!("Settings changed; reload before saving"); }
        settings.validate()?;
        settings.revision = expected.checked_add(1).context("Settings revision exhausted")?;
        atomic_write(&self.path, &serde_json::to_vec_pretty(&settings)?).await?;
        *self.value.write().unwrap_or_else(|e| e.into_inner()) = settings.clone();
        Ok(settings)
    }
}

pub async fn optional_text(path: &Path) -> Result<Option<String>> {
    match tokio::fs::read_to_string(path).await {
        Ok(text) => Ok(Some(text)), Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("Could not read {}", path.display())),
    }
}
pub async fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let path = path.to_owned(); let bytes = bytes.to_vec();
    tokio::task::spawn_blocking(move || -> Result<()> {
        use std::io::Write;
        let parent = path.parent().context("File has no parent directory")?;
        std::fs::create_dir_all(parent)?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; file.as_file().set_permissions(std::fs::Permissions::from_mode(0o600))?; }
        file.write_all(&bytes)?; file.as_file().sync_all()?;
        file.persist(&path).map_err(|e| e.error)?;
        std::fs::File::open(parent)?.sync_all()?;
        Ok(())
    }).await?
}
