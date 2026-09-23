use std::{collections::BTreeMap, path::PathBuf};
use serde::{Deserialize, Serialize};
use crate::SessionModel;

pub const DEFAULT_TITLE_PROMPT: &str = include_str!("../../scripts/title_prompt.txt");

pub const DEFAULT_SYSTEM_PROMPT: &str = "You are an expert coding assistant operating inside Tau. Help the user by reading files, executing commands, editing code, and writing new files. Be concise and show paths clearly. Use bash for listing and searching files. Use send_file to deliver local artifacts; staging and inline image detection are automatic. Use image_generation when offered, otherwise generate_image, to generate or edit images through Codex; output is delivered automatically. Use flag_it for new actionable findings outside the current task; state the location and impact, omit secrets, and continue the current task.";
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
    pub generate_titles: bool,
    pub title_model: Option<SessionModel>,
    pub idle_timeout_seconds: u64,
}
impl Default for DaemonSettings {
    fn default() -> Self { Self { title_prompt: DEFAULT_TITLE_PROMPT.into(), generate_titles:false, title_model:None, idle_timeout_seconds: 3600 } }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct AgentSettings {
    pub model: SessionModel,
    pub thinking_level: String,
    pub model_thinking_levels: BTreeMap<String, String>,
    pub steering_mode: SteeringMode,
    pub system_prompt: String,
    pub model_system_prompts: BTreeMap<String, String>,
    pub load_agents_files: bool,
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
        system_prompt: DEFAULT_SYSTEM_PROMPT.into(), model_system_prompts: BTreeMap::new(),
        load_agents_files: true, fast_mode: false, retry: RetrySettings::default(), compaction: CompactionSettings::default(),
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
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ModelSettings {
    pub provider: String,
    pub id: String,
    #[serde(default)] pub name: String,
    #[serde(default)] pub context_window: Option<u64>,
    #[serde(default)] pub thinking_level_map: BTreeMap<String, Option<String>>,
}
impl Default for Settings {
    fn default() -> Self { Self {
        schema: 2, revision: 0, daemon: DaemonSettings::default(), agent: AgentSettings::default(),
        providers: BTreeMap::from([
            ("openai-codex".into(), ProviderSettings { api: Api::Codex, base_url: "https://chatgpt.com/backend-api/codex".into(), api_key_env: None, web_search: true }),
            ("openrouter".into(), ProviderSettings { api: Api::ChatCompletions, base_url: "https://openrouter.ai/api/v1".into(), api_key_env: Some("OPENROUTER_API_KEY".into()), web_search: true }),
        ]),
        models: vec![ModelSettings { provider: "openai-codex".into(), id: "gpt-6-astra".into(), name: "GPT-6 Astra".into(), context_window: Some(272000), thinking_level_map: BTreeMap::new() }],
    } }
}
