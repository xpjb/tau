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
    if !args.is_empty() { error!("Usage: taud [--login-codex]"); return ExitCode::FAILURE; }

    let config = match taud::Config::from_env() {
        Ok(config) => config,
        Err(error) => {
            error!(%error, "Tau configuration is invalid");
            return ExitCode::FAILURE;
        }
    };
    match taud::run(config).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            error!(error = %error, "Tau daemon stopped");
            ExitCode::FAILURE
        }
    }
}
