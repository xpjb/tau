use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result, bail};

#[derive(Clone, Debug)]
pub struct Config {
    pub bind: SocketAddr,
    pub token: Arc<str>,
    pub pi_command: PathBuf,
    pub default_model: String,
    pub default_thinking_level: String,
    pub cwd: PathBuf,
    pub state_path: PathBuf,
    pub session_dir: PathBuf,
    pub telemetry_path: PathBuf,
    pub pi_extension_path: PathBuf,
    pub attachment_root: PathBuf,
    pub upload_root: PathBuf,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let token = std::env::var("TAU_TOKEN").context("TAU_TOKEN is required")?;
        if token.chars().count() < 32 {
            bail!("TAU_TOKEN must contain at least 32 characters");
        }
        if token.chars().any(char::is_whitespace) {
            bail!("TAU_TOKEN cannot contain whitespace");
        }

        let bind = std::env::var("TAU_BIND")
            .unwrap_or_else(|_| "127.0.0.1:8787".to_owned())
            .parse()
            .context("TAU_BIND must be an IP address and port")?;
        let pi_command = std::env::var_os("TAU_PI_COMMAND")
            .map(PathBuf::from)
            .unwrap_or_else(|| "/usr/bin/pi".into());
        let default_model = std::env::var("TAU_DEFAULT_MODEL")
            .unwrap_or_else(|_| "openai-codex/gpt-5.6-sol".to_owned());
        let valid_model = default_model
            .split_once('/')
            .is_some_and(|(provider, model_id)| {
                !provider.is_empty()
                    && !model_id.is_empty()
                    && !default_model.chars().any(char::is_whitespace)
            });
        if !valid_model {
            bail!("TAU_DEFAULT_MODEL must be provider/model");
        }
        let default_thinking_level = std::env::var("TAU_DEFAULT_THINKING_LEVEL")
            .unwrap_or_else(|_| "max".to_owned());
        if !matches!(
            default_thinking_level.as_str(),
            "off" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max"
        ) {
            bail!(
                "TAU_DEFAULT_THINKING_LEVEL must be off, minimal, low, medium, high, xhigh, or max"
            );
        }
        let cwd = std::env::var_os("TAU_CWD")
            .map(PathBuf::from)
            .unwrap_or_else(|| "/root".into());
        let state_path = std::env::var_os("TAU_STATE_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|| "/var/lib/tau/state.json".into());
        let session_dir = std::env::var_os("TAU_SESSION_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| "/var/lib/tau/pi-sessions".into());
        let telemetry_path = std::env::var_os("TAU_TELEMETRY_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|| "/var/lib/tau/client-crashes.jsonl".into());
        let pi_extension_path = std::env::var_os("TAU_PI_EXTENSION")
            .map(PathBuf::from)
            .unwrap_or_else(|| "/usr/local/lib/tau/send-media.ts".into());
        let attachment_root = std::env::var_os("TAU_ATTACHMENT_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| "/root/.local/share/tau/outbox".into());
        let upload_root = std::env::var_os("TAU_UPLOAD_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| "/root/.local/share/tau/uploads".into());

        if !cwd.is_absolute()
            || !state_path.is_absolute()
            || !session_dir.is_absolute()
            || !telemetry_path.is_absolute()
            || !pi_extension_path.is_absolute()
            || !attachment_root.is_absolute()
            || !upload_root.is_absolute()
        {
            bail!("Tau paths must be absolute");
        }

        Ok(Self {
            bind,
            token: Arc::from(token),
            pi_command,
            default_model,
            default_thinking_level,
            cwd,
            state_path,
            session_dir,
            telemetry_path,
            pi_extension_path,
            attachment_root,
            upload_root,
        })
    }
}
