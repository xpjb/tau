use std::process::ExitCode;

use tracing::error;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .compact()
        .init();

    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args == ["--login-codex"] {
        return match taud::login_codex().await { Ok(()) => ExitCode::SUCCESS, Err(error) => { error!(%error, "Codex login failed"); ExitCode::FAILURE } };
    }
    if !args.is_empty() && !(args.len() == 2 && args[0] == "--import-state") { error!("Usage: taud [--login-codex | --import-state PATH]"); return ExitCode::FAILURE; }

    let config = match taud::Config::from_env() {
        Ok(config) => config,
        Err(error) => {
            error!(%error, "Tau configuration is invalid");
            return ExitCode::FAILURE;
        }
    };
    if args.len() == 2 {
        return match taud::import_state(config,args[1].clone().into()).await {
            Ok(count) => { tracing::info!(count,"Imported legacy sessions into SQLite"); ExitCode::SUCCESS }
            Err(error) => { error!(%error,"Legacy import failed"); ExitCode::FAILURE }
        };
    }
    match taud::run(config).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            error!(error = %error, "Tau daemon stopped");
            ExitCode::FAILURE
        }
    }
}
