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
    /// Optional read-only Codex credential source for side-by-side operation.
    pub codex_auth_source: Option<PathBuf>,
    pub cwd: PathBuf,
    pub database_path: PathBuf,
    pub telemetry_path: PathBuf,
    pub attachment_root: PathBuf,
    pub upload_root: PathBuf,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        if std::env::var_os("TAU_STATE_PATH").is_some() || std::env::var_os("TAU_SESSION_DIR").is_some() {
            bail!("Tau 2 uses TAU_DATABASE_PATH, not TAU_STATE_PATH/TAU_SESSION_DIR. Remove those variables; use --import-state PATH for legacy Tau 1 history.");
        }
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
        let codex_auth_source = std::env::var_os("TAU_CODEX_AUTH_SOURCE").map(PathBuf::from);
        let cwd = std::env::var_os("TAU_CWD")
            .map(PathBuf::from)
            .unwrap_or_else(|| "/root".into());
        let database_path = std::env::var_os("TAU_DATABASE_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|| "/var/lib/tau/tau.sqlite3".into());
        let telemetry_path = std::env::var_os("TAU_TELEMETRY_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|| "/var/lib/tau/client-crashes.jsonl".into());
        let attachment_root = std::env::var_os("TAU_ATTACHMENT_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| "/root/.local/share/tau/outbox".into());
        let upload_root = std::env::var_os("TAU_UPLOAD_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| "/root/.local/share/tau/uploads".into());

        if !cwd.is_absolute()
            || !database_path.is_absolute()
            || !telemetry_path.is_absolute()
            || !settings_path.is_absolute()
            || import_pi_dir.as_ref().is_some_and(|path| !path.is_absolute())
            || codex_auth_source.as_ref().is_some_and(|path| !path.is_absolute())
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
            codex_auth_source,
            cwd,
            database_path,
            telemetry_path,
            attachment_root,
            upload_root,
        })
    }
}
