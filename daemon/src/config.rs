use std::net::{SocketAddr, SocketAddrV4};
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result, bail};

#[derive(Clone, Debug)]
pub struct Config {
    pub bind: SocketAddr,
    pub transfer_bind: SocketAddrV4,
    pub token: Arc<str>,
    pub settings_path: PathBuf,
    pub import_pi_dir: Option<PathBuf>,
    pub cwd: PathBuf,
    pub state_path: PathBuf,
    pub session_dir: PathBuf,
    pub telemetry_path: PathBuf,
    pub attachment_root: PathBuf,
    pub upload_root: PathBuf,
    pub title_command: Option<String>,
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
        let transfer_bind = std::env::var("TAU_TRANSFER_BIND")
            .unwrap_or_else(|_| "127.0.0.1:8788".to_owned())
            .parse()
            .context("TAU_TRANSFER_BIND must be an IPv4 address and UDP port")?;
        let settings_path = std::env::var_os("TAU_SETTINGS_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|| "/var/lib/tau/settings.json".into());
        let import_pi_dir = std::env::var_os("TAU_IMPORT_PI_DIR").map(PathBuf::from);
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
        let attachment_root = std::env::var_os("TAU_ATTACHMENT_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| "/root/.local/share/tau/outbox".into());
        let upload_root = std::env::var_os("TAU_UPLOAD_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| "/root/.local/share/tau/uploads".into());
        let title_command = std::env::var("TAU_TITLE_COMMAND").ok().filter(|c| !c.is_empty());

        if !cwd.is_absolute()
            || !state_path.is_absolute()
            || !session_dir.is_absolute()
            || !telemetry_path.is_absolute()
            || !settings_path.is_absolute()
            || import_pi_dir.as_ref().is_some_and(|path| !path.is_absolute())
            || !attachment_root.is_absolute()
            || !upload_root.is_absolute()
        {
            bail!("Tau paths must be absolute");
        }

        Ok(Self {
            bind,
            transfer_bind,
            token: Arc::from(token),
            settings_path,
            import_pi_dir,
            cwd,
            state_path,
            session_dir,
            telemetry_path,
            attachment_root,
            upload_root,
            title_command,
        })
    }
}
