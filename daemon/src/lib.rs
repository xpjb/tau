mod attachments;
mod commands;
mod config;
mod manager;
mod agent;
mod settings;
mod protocol;
mod server;
mod state;
mod transcript;

use anyhow::{Context, Result, bail};
use tokio::fs;

pub use config::Config;

use manager::AgentManager;
use state::StateStore;

pub async fn run(mut config: Config) -> Result<()> {
    let cwd = fs::metadata(&config.cwd)
        .await
        .with_context(|| format!("working directory {} is unavailable", config.cwd.display()))?;
    if !cwd.is_dir() {
        bail!("working directory {} is not a directory", config.cwd.display());
    }
    fs::create_dir_all(&config.attachment_root)
        .await
        .with_context(|| {
            format!(
                "failed to create attachment directory {}",
                config.attachment_root.display()
            )
        })?;
    if let Some(parent) = config.database_path.parent() {
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    if let Some(parent) = config.telemetry_path.parent() {
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let listener = tokio::net::TcpListener::bind(config.bind)
        .await
        .with_context(|| format!("failed to bind {}", config.bind))?;
    config.bind = listener.local_addr()?;
    let state = StateStore::load(config.database_path.clone()).await?;
    let manager = AgentManager::new(config.clone(), state).await?;
    server::serve(config, manager, listener).await
}

#[cfg(test)]
mod agent_test;

pub async fn login_codex() -> Result<()> {
    let settings_path = std::env::var_os("TAU_SETTINGS_PATH").map(std::path::PathBuf::from).unwrap_or_else(|| "/var/lib/tau/settings.json".into());
    if !settings_path.is_absolute() { bail!("TAU_SETTINGS_PATH must be absolute"); }
    let http = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build()?;
    agent::auth::AuthStore::new(settings_path.with_file_name("auth.json"), http).login().await
}

/// Read-only import of deployed Tau 1 metadata/Pi histories; the destination must be empty.
pub async fn import_state(config: Config, path: std::path::PathBuf) -> Result<usize> {
    let legacy: serde_json::Value = serde_json::from_slice(&fs::read(path).await?)?;
    let settings = settings::SettingsStore::load(&config,legacy["title_prompt"].as_str().unwrap_or(state::DEFAULT_TITLE_PROMPT).into()).await?;
    StateStore::load(config.database_path).await?.import_legacy(legacy,settings.get()).await
}

pub async fn export_history(config: Config, session: &str, destination: &std::path::Path) -> Result<()> {
    if !config.database_path.is_file() { bail!("Database does not exist"); }
    StateStore::load(config.database_path).await?.export_history(session,destination).await
}
