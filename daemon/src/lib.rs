mod config;
mod manager;
mod pi;
mod protocol;
mod server;
mod state;

use anyhow::{Context, Result, bail};
use tokio::fs;

pub use config::Config;

use manager::AgentManager;
use state::StateStore;

pub async fn run(config: Config) -> Result<()> {
    let cwd = fs::metadata(&config.cwd)
        .await
        .with_context(|| format!("working directory {} is unavailable", config.cwd.display()))?;
    if !cwd.is_dir() {
        bail!("working directory {} is not a directory", config.cwd.display());
    }
    fs::create_dir_all(&config.session_dir)
        .await
        .with_context(|| {
            format!(
                "failed to create Pi session directory {}",
                config.session_dir.display()
            )
        })?;
    fs::create_dir_all(&config.attachment_root)
        .await
        .with_context(|| {
            format!(
                "failed to create attachment directory {}",
                config.attachment_root.display()
            )
        })?;
    if !fs::metadata(&config.pi_extension_path)
        .await
        .is_ok_and(|metadata| metadata.is_file())
    {
        bail!(
            "Pi extension {} is missing",
            config.pi_extension_path.display()
        );
    }
    if let Some(parent) = config.state_path.parent() {
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    if let Some(parent) = config.telemetry_path.parent() {
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let state = StateStore::load(config.state_path.clone()).await?;
    let manager = AgentManager::new(config.clone(), state);
    server::serve(config, manager).await
}
