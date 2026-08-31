use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result, bail};

#[derive(Clone, Debug)]
pub struct Config {
    pub bind: SocketAddr,
    pub token: Arc<str>,
    pub pi_command: PathBuf,
    pub cwd: PathBuf,
    pub state_path: PathBuf,
    pub session_dir: PathBuf,
    pub telemetry_path: PathBuf,
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

        if !cwd.is_absolute()
            || !state_path.is_absolute()
            || !session_dir.is_absolute()
            || !telemetry_path.is_absolute()
        {
            bail!("Tau paths must be absolute");
        }

        Ok(Self {
            bind,
            token: Arc::from(token),
            pi_command,
            cwd,
            state_path,
            session_dir,
            telemetry_path,
        })
    }
}
