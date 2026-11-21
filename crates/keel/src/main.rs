use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use keel_core::{AppConfig, EnvSecrets, Host};

/// Keel: a pluggable Rust backend you configure in YAML.
#[derive(Parser)]
#[command(name = "keel", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Start the server from app.yaml.
    Serve {
        /// Path to the config file.
        #[arg(short, long, default_value = "app.yaml")]
        config: PathBuf,
    },
    /// Validate app.yaml and its secrets without starting the server.
    Check {
        #[arg(short, long, default_value = "app.yaml")]
        config: PathBuf,
    },
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cli = Cli::parse();
    let (path, serve) = match cli.command {
        Command::Serve { config } => (config, true),
        Command::Check { config } => (config, false),
    };

    let config = match AppConfig::load_file(&path, &EnvSecrets) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

    if !serve {
        println!(
            "ok: {} is valid (app '{}', {} provider(s), {} plugin(s), {} route rule(s))",
            path.display(),
            config.app,
            config.providers.len(),
            config.plugins.len(),
            config.routes.len()
        );
        return ExitCode::SUCCESS;
    }

    let shutdown = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    match Host::new(config).serve(shutdown).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: server failed: {e}");
            ExitCode::FAILURE
        }
    }
}
