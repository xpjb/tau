use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Mutex;

use crate::state::SessionModel;

pub const DEFAULT_SYSTEM_PROMPT: &str = "You are an expert coding assistant operating inside Tau. Help the user by reading files, executing commands, editing code, and writing new files. Be concise and show paths clearly. Use bash for listing and searching files. Use send_image or send_file to deliver artifacts from the Tau outbox. Use flag_it for new actionable findings outside the current task; state the location and impact, omit secrets, and continue the current task.";
pub const LEVELS: &[&str] = &["off", "minimal", "low", "medium", "high", "xhigh", "max"];

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct Settings {
    pub schema: u32,
    pub revision: u64,
    pub daemon: DaemonSettings,
    pub agent: AgentSettings,
    pub providers: BTreeMap<String, ProviderSettings>,
    pub models: Vec<ModelSettings>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct DaemonSettings {
    pub title_prompt: String,
    pub idle_timeout_seconds: u64,
}
impl Default for DaemonSettings {
    fn default() -> Self { Self { title_prompt: crate::state::DEFAULT_TITLE_PROMPT.into(), idle_timeout_seconds: 3600 } }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct AgentSettings {
    pub model: SessionModel,
    pub thinking_level: String,
    pub model_thinking_levels: BTreeMap<String, String>,
    pub steering_mode: SteeringMode,
    pub system_prompt: Option<String>,
    pub append_system_prompt: String,
    pub project_prompts: BTreeMap<PathBuf, PromptOverride>,
    pub load_project_instructions: bool,
    pub fast_mode: bool,
    pub retry: RetrySettings,
    pub compaction: CompactionSettings,
    pub shell_path: PathBuf,
    pub shell_command_prefix: String,
    pub http_idle_timeout_seconds: u64,
    pub max_tool_output_bytes: usize,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SteeringMode { All, OneAtATime }
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct PromptOverride { pub system_prompt: Option<String>, pub append_system_prompt: String }
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct RetrySettings { pub enabled: bool, pub max_retries: u32, pub base_delay_ms: u64 }
impl Default for RetrySettings {
    fn default() -> Self { Self { enabled: true, max_retries: 3, base_delay_ms: 2000 } }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct CompactionSettings {
    pub enabled: bool,
    pub reserve_tokens: u64,
    pub keep_recent_tokens: u64,
    pub native_codex: bool,
    pub timeout_seconds: u64,
}
impl Default for CompactionSettings {
    fn default() -> Self { Self { enabled: true, reserve_tokens: 16384, keep_recent_tokens: 20000, native_codex: true, timeout_seconds: 600 } }
}
impl Default for AgentSettings {
    fn default() -> Self { Self {
        model: SessionModel { provider: "openai-codex".into(), model_id: "gpt-6-astra".into() },
        thinking_level: "max".into(), model_thinking_levels: BTreeMap::new(), steering_mode: SteeringMode::All,
        system_prompt: None, append_system_prompt: String::new(), project_prompts: BTreeMap::new(),
        load_project_instructions: true, fast_mode: false, retry: RetrySettings::default(), compaction: CompactionSettings::default(),
        shell_path: "/bin/bash".into(), shell_command_prefix: String::new(), http_idle_timeout_seconds: 300, max_tool_output_bytes: 50 * 1024,
    } }
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Api { Codex, ChatCompletions }
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ProviderSettings {
    pub api: Api,
    pub base_url: String,
    pub api_key_env: Option<String>,
    #[serde(default)] pub web_search: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ModelSettings {
    pub provider: String,
    pub id: String,
    pub name: String,
    pub context_window: u64,
    #[serde(default)] pub thinking_level_map: BTreeMap<String, Option<String>>,
}
impl Default for Settings {
    fn default() -> Self { Self {
        schema: 1, revision: 0, daemon: DaemonSettings::default(), agent: AgentSettings::default(),
        providers: BTreeMap::from([
            ("openai-codex".into(), ProviderSettings { api: Api::Codex, base_url: "https://chatgpt.com/backend-api/codex".into(), api_key_env: None, web_search: true }),
            ("openrouter".into(), ProviderSettings { api: Api::ChatCompletions, base_url: "https://openrouter.ai/api/v1".into(), api_key_env: Some("OPENROUTER_API_KEY".into()), web_search: true }),
        ]),
        models: vec![ModelSettings { provider: "openai-codex".into(), id: "gpt-6-astra".into(), name: "GPT-6 Astra".into(), context_window: 272000, thinking_level_map: BTreeMap::new() }],
    } }
}
impl Settings {
    pub fn model(&self, selected: &SessionModel) -> Result<&ModelSettings> {
        self.models.iter().find(|model| model.provider == selected.provider && model.id == selected.model_id)
            .with_context(|| format!("Model {}/{} is not in daemon settings", selected.provider, selected.model_id))
    }
    pub fn validate(&self) -> Result<()> {
        if self.schema != 1 { bail!("Unsupported settings schema {}", self.schema); }
        if self.models.is_empty() || self.models.len() > 4096 { bail!("Configure 1–4096 models"); }
        self.model(&self.agent.model)?;
        let mut ids = std::collections::HashSet::new();
        for model in &self.models {
            if model.id.is_empty() || model.context_window < 1024 || model.context_window > 100_000_000
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
        for (path, prompt) in &self.agent.project_prompts {
            if !path.is_absolute() { bail!("Project prompt paths must be absolute"); }
            for text in [prompt.system_prompt.as_deref().unwrap_or_default(), &prompt.append_system_prompt] {
                if text.chars().count() > crate::protocol::MAX_PROMPT_CHARS { bail!("Project prompt is too long"); }
            }
        }
        for text in [&self.daemon.title_prompt, self.agent.system_prompt.as_deref().unwrap_or_default(), &self.agent.append_system_prompt] {
            if text.chars().count() > crate::protocol::MAX_PROMPT_CHARS { bail!("Prompt is too long"); }
        }
        if serde_json::to_vec(self)?.len() > 900_000 { bail!("Settings exceed 900 KB"); }
        Ok(())
    }

    pub async fn system_prompt(&self, cwd: &Path) -> Result<String> {
        let project = self.agent.project_prompts.iter().filter(|(path, _)| cwd.starts_with(path))
            .max_by_key(|(path, _)| path.components().count()).map(|(_, prompt)| prompt);
        let mut prompt = project.and_then(|p| p.system_prompt.as_ref()).or(self.agent.system_prompt.as_ref())
            .map(String::as_str).unwrap_or(DEFAULT_SYSTEM_PROMPT).to_owned();
        prompt.push_str(&format!("\n\n{}", self.agent.append_system_prompt));
        if let Some(project) = project { prompt.push_str(&format!("\n\n{}", project.append_system_prompt)); }
        if self.agent.load_project_instructions {
            for directory in cwd.ancestors().collect::<Vec<_>>().into_iter().rev() {
                let path = directory.join("AGENTS.md");
                match tokio::fs::read_to_string(&path).await {
                    Ok(text) if text.len() <= 1024 * 1024 => prompt.push_str(&format!("\n\nProject instructions ({}):\n{text}", path.display())),
                    Ok(_) => bail!("Project instructions {} exceed 1 MB", path.display()),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
                    Err(error) => return Err(error).context("Could not read project instructions"),
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
        let mut settings = match tokio::fs::read(&config.settings_path).await {
            Ok(bytes) => serde_json::from_slice::<Settings>(&bytes).context("Invalid daemon settings")?,
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
                    for (file, append) in [("SYSTEM.md", false), ("APPEND_SYSTEM.md", true)] {
                        if let Some(text) = optional_text(&dir.join(file)).await? {
                            if append { settings.agent.append_system_prompt = text; } else { settings.agent.system_prompt = Some(text); }
                        }
                    }
                    let project = config.cwd.join(".pi");
                    let override_prompt = PromptOverride { system_prompt: optional_text(&project.join("SYSTEM.md")).await?,
                        append_system_prompt: optional_text(&project.join("APPEND_SYSTEM.md")).await?.unwrap_or_default() };
                    if override_prompt.system_prompt.is_some() || !override_prompt.append_system_prompt.is_empty() {
                        settings.agent.project_prompts.insert(config.cwd.clone(), override_prompt);
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
                                    name: model["name"].as_str().unwrap_or_default().into(), context_window: model["contextWindow"].as_u64().unwrap_or(128000),
                                    thinking_level_map: model.get("thinkingLevelMap").map(|v| serde_json::from_value(v.clone())).transpose()?.unwrap_or_default() });
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
        // Revision zero is a valid initial revision; every edit uses compare-and-swap.
        settings.schema = 1;
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
