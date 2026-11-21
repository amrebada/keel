mod plugins;
mod scaffold;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use keel_core::{AppConfig, EnvSecrets, Host};
use keel_sdk::capabilities::CapabilityProviders;
use keel_sdk::Manifest;

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
    /// Validate app.yaml, its secrets and its plugins without starting anything.
    Check {
        #[arg(short, long, default_value = "app.yaml")]
        config: PathBuf,
    },
    /// Work with plugins.
    Plugins {
        #[command(subcommand)]
        command: PluginsCommand,
    },
    /// Create a plugin crate under plugins/ and compile it into this binary.
    NewPlugin {
        /// Plugin id, e.g. `hello` or `stripe-billing`.
        id: String,
    },
}

#[derive(Subcommand)]
enum PluginsCommand {
    /// List the plugins compiled into this binary.
    List,
    /// Check a plugin's plugin.yaml and config.schema.json.
    Check {
        /// A plugin directory or its plugin.yaml.
        path: PathBuf,
    },
}

fn providers() -> CapabilityProviders {
    CapabilityProviders {
        secrets: Some(Arc::new(EnvSecrets)),
        ..Default::default()
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let result = match Cli::parse().command {
        Command::Serve { config } => serve(&config).await,
        Command::Check { config } => check(&config),
        Command::Plugins {
            command: PluginsCommand::List,
        } => {
            list();
            Ok(())
        }
        Command::Plugins {
            command: PluginsCommand::Check { path },
        } => check_plugin(&path),
        Command::NewPlugin { id } => new_plugin(&id),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

async fn serve(path: &Path) -> anyhow::Result<()> {
    let config = AppConfig::load_file(path, &EnvSecrets)?;
    let host = Host::start(config, &plugins::builtin(), &providers()).await?;
    let shutdown = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    host.serve(shutdown).await?;
    Ok(())
}

fn check(path: &Path) -> anyhow::Result<()> {
    let config = AppConfig::load_file(path, &EnvSecrets)?;
    keel_core::plugins::check(&config, &plugins::builtin(), &providers())?;
    println!(
        "ok: {} is valid (app '{}', {} provider(s), {} plugin(s), {} route rule(s))",
        path.display(),
        config.app,
        config.providers.len(),
        config.plugins.len(),
        config.routes.len()
    );
    Ok(())
}

fn list() {
    for p in plugins::builtin().iter() {
        let m = p.manifest();
        let routes: Vec<String> = m.routes.iter().map(|r| r.to_string()).collect();
        println!("{} {}  {}", m.id, m.version, m.summary);
        if !routes.is_empty() {
            println!("    routes: {}", routes.join(", "));
        }
    }
}

fn check_plugin(path: &Path) -> anyhow::Result<()> {
    let (dir, yaml_path) = if path.is_dir() {
        (path.to_path_buf(), path.join("plugin.yaml"))
    } else {
        (
            path.parent().unwrap_or(Path::new(".")).to_path_buf(),
            path.to_path_buf(),
        )
    };
    let yaml = std::fs::read_to_string(&yaml_path)
        .map_err(|e| anyhow::anyhow!("{}: cannot read file: {e}", yaml_path.display()))?;
    let schema_path = dir.join("config.schema.json");
    let schema = schema_path
        .is_file()
        .then(|| std::fs::read_to_string(&schema_path))
        .transpose()?;
    let m = Manifest::from_sources(&yaml, schema.as_deref())?;
    println!(
        "ok: {} {} is valid ({} route(s), {} event(s), capabilities: {})",
        m.id,
        m.version,
        m.routes.len(),
        m.events.len(),
        if m.capabilities.is_empty() {
            "none".to_string()
        } else {
            m.capabilities
                .iter()
                .map(|c| c.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        }
    );
    Ok(())
}

fn new_plugin(id: &str) -> anyhow::Result<()> {
    let root = scaffold::find_workspace(&std::env::current_dir()?)?;
    let out = scaffold::new_plugin(&root, id)?;
    println!(
        "created plugin `{id}` ({}) in {}",
        out.crate_name,
        out.dir.display()
    );
    for f in &out.files {
        println!("  wrote {}", f.strip_prefix(&root).unwrap_or(f).display());
    }
    println!("\nnext:\n  1. add `- use: {id}` under plugins: in your app.yaml");
    println!("  2. cargo run -p keel -- serve -c app.yaml");
    println!("  3. curl localhost:8080/{id}/hello");
    Ok(())
}
