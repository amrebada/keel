mod describe;
mod dotenv;
mod mcp;
mod ops;
mod output;
mod plugins;
mod scaffold;
mod yaml_edit;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand};
use keel_core::setup::SetupState;
use keel_core::{AppConfig, Host, HostOptions, PluginRegistry, SecretSource};
use keel_providers::LayeredSecrets;
use keel_sdk::capabilities::CapabilityProviders;
use keel_sdk::Manifest;
use output::Out;
use serde_json::{json, Value};

/// Keel: a pluggable Rust backend you configure in YAML.
#[derive(Parser)]
#[command(name = "keel", version)]
struct Cli {
    /// Print one JSON object instead of text (for scripts and AI agents).
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create app.yaml with a development setup (SQLite + memory).
    Init {
        /// Directory to create the app in.
        #[arg(default_value = ".")]
        dir: PathBuf,
        /// App name. Defaults to the directory name.
        #[arg(long)]
        name: Option<String>,
    },
    /// Start the server from app.yaml.
    Serve {
        #[arg(short, long, default_value = "app.yaml")]
        config: PathBuf,
    },
    /// Start the server and restart it whenever app.yaml changes.
    Dev {
        #[arg(short, long, default_value = "app.yaml")]
        config: PathBuf,
    },
    /// Same as `keel config validate`.
    Check {
        #[arg(short, long, default_value = "app.yaml")]
        config: PathBuf,
    },
    /// Validate or expand app.yaml.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Add a compiled-in plugin to app.yaml with a minimal valid config.
    Add {
        /// Plugin id, optionally with a version range: `ping` or `ping@^0.1`.
        plugin: String,
        #[arg(short, long, default_value = "app.yaml")]
        config: PathBuf,
    },
    /// List, describe or check plugins.
    Plugins {
        #[command(subcommand)]
        command: PluginsCommand,
    },
    /// Show which setup steps (accounts, keys, DNS) are done or missing.
    Setup {
        #[command(subcommand)]
        command: SetupCommand,
    },
    /// Plan and run plugin database migrations. Without flags, prints the
    /// plan (a dry run). Destructive migrations need --allow-destructive.
    Migrate {
        #[arg(short, long, default_value = "app.yaml")]
        config: PathBuf,
        /// Run pending additive migrations.
        #[arg(long)]
        apply: bool,
        /// Also run this destructive migration (`<plugin>/<id>`), after
        /// copying the tables it touches to keel_backup_<time>_<table>.
        #[arg(long = "allow-destructive", value_name = "PLUGIN/ID")]
        allow_destructive: Vec<String>,
    },
    /// Serve the Model Context Protocol on stdio for AI clients: search and
    /// describe plugins, read, validate and apply app.yaml, setup status, logs.
    /// Add to Claude Code with `claude mcp add keel -- keel mcp -c /path/to/app.yaml`.
    Mcp {
        #[arg(short, long, default_value = "app.yaml")]
        config: PathBuf,
    },
    /// Create a plugin crate under plugins/ and compile it into this binary.
    NewPlugin {
        /// Plugin id, e.g. `hello` or `stripe-billing`.
        id: String,
    },
}

#[derive(Subcommand)]
enum ConfigCommand {
    /// Check app.yaml, its secrets and its plugins without starting anything.
    Validate {
        #[arg(short, long, default_value = "app.yaml")]
        config: PathBuf,
    },
    /// Print app.yaml with every default filled in. Secrets stay as references.
    Expand {
        #[arg(short, long, default_value = "app.yaml")]
        config: PathBuf,
    },
}

#[derive(Subcommand)]
enum SetupCommand {
    /// Every step of every configured plugin: done, missing, failed or skipped.
    Status {
        #[arg(short, long, default_value = "app.yaml")]
        config: PathBuf,
    },
    /// Mark a manual step done (stored in .keel/setup.json).
    Done {
        /// `<plugin>/<step>`, as `keel setup status` prints it.
        id: String,
        #[arg(short, long, default_value = "app.yaml")]
        config: PathBuf,
    },
    /// Mark a manual step not done again.
    Undo {
        id: String,
        #[arg(short, long, default_value = "app.yaml")]
        config: PathBuf,
    },
}

#[derive(Subcommand)]
enum PluginsCommand {
    /// List the plugins compiled into this binary.
    List,
    /// Show what a plugin needs, its config keys, routes and events.
    Describe {
        id: String,
        /// Short summary for AI agents (about 300 tokens).
        #[arg(long)]
        ai: bool,
    },
    /// Check a plugin's plugin.yaml and config.schema.json.
    Check {
        /// A plugin directory or its plugin.yaml.
        path: PathBuf,
    },
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let out = Out { json: cli.json };
    let serving = matches!(cli.command, Command::Serve { .. } | Command::Dev { .. });
    if serving || std::env::var_os("RUST_LOG").is_some() {
        use tracing_subscriber::layer::SubscriberExt;
        use tracing_subscriber::util::SubscriberInitExt;
        // Servers also log to .keel/keel.log beside app.yaml (read by `keel mcp`).
        let file = match &cli.command {
            Command::Serve { config } | Command::Dev { config } => log_file(config),
            _ => None,
        };
        tracing_subscriber::registry()
            .with(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
            .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
            .with(file.map(|f| {
                tracing_subscriber::fmt::layer()
                    .with_ansi(false)
                    .with_writer(std::sync::Mutex::new(f))
            }))
            .init();
    }

    // Commands that read app.yaml also pick up a `.env` beside it.
    if let Command::Serve { config }
    | Command::Dev { config }
    | Command::Check { config }
    | Command::Migrate { config, .. }
    | Command::Setup {
        command:
            SetupCommand::Status { config }
            | SetupCommand::Done { config, .. }
            | SetupCommand::Undo { config, .. },
    }
    | Command::Config {
        command: ConfigCommand::Validate { config } | ConfigCommand::Expand { config },
    } = &cli.command
    {
        if let Some(path) = dotenv::load_beside(config) {
            if serving {
                eprintln!("keel: loaded {}", path.display());
            }
        }
    }

    let result = match cli.command {
        Command::Init { dir, name } => init(out, &dir, name.as_deref()),
        Command::Serve { config } => serve(&config).await,
        Command::Dev { config } => dev(&config).await,
        Command::Check { config }
        | Command::Config {
            command: ConfigCommand::Validate { config },
        } => validate(out, &config).await,
        Command::Config {
            command: ConfigCommand::Expand { config },
        } => expand(out, &config),
        Command::Add { plugin, config } => add(out, &plugin, &config),
        Command::Plugins {
            command: PluginsCommand::List,
        } => {
            list(out);
            Ok(())
        }
        Command::Plugins {
            command: PluginsCommand::Describe { id, ai },
        } => describe_plugin(out, &id, ai),
        Command::Plugins {
            command: PluginsCommand::Check { path },
        } => check_plugin(out, &path),
        Command::Setup {
            command: SetupCommand::Status { config },
        } => ops::setup_status(out, &config).await,
        Command::Setup {
            command: SetupCommand::Done { id, config },
        } => ops::setup_mark(out, &config, &id, true).await,
        Command::Setup {
            command: SetupCommand::Undo { id, config },
        } => ops::setup_mark(out, &config, &id, false).await,
        Command::Migrate {
            config,
            apply,
            allow_destructive,
        } => ops::migrate(out, &config, apply, &allow_destructive).await,
        Command::Mcp { config } => mcp::run(&config).await,
        Command::NewPlugin { id } => new_plugin(out, &id),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) if e.is::<ops::Reported>() => ExitCode::FAILURE,
        Err(e) => {
            out.failure(&e);
            ExitCode::FAILURE
        }
    }
}

/// `.keel/keel.log` beside app.yaml, appended to; moved to keel.log.1 past 5 MB.
fn log_file(config: &Path) -> Option<std::fs::File> {
    let dir = SetupState::path_for(config).parent()?.to_path_buf();
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join("keel.log");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 5 * 1024 * 1024) {
        let _ = std::fs::rename(&path, dir.join("keel.log.1"));
    }
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .ok()
}

/// Capabilities that need no provider, for static checks.
fn static_providers(config: &AppConfig) -> CapabilityProviders {
    // For checks, pretend each configured slot is available without connecting.
    let mut p = CapabilityProviders {
        secrets: Some(Arc::new(keel_core::EnvSecrets)),
        crypto: Some(Arc::new(keel_providers::DefaultCrypto)),
        http: Some(Arc::new(keel_providers::ReqwestClient::default())),
        ..Default::default()
    };
    if config.providers.contains_key("email") {
        p.email = Some(Arc::new(keel_providers::ConsoleEmail::default()));
    }
    let mem = Arc::new(keel_providers::MemoryStore::new());
    if config.providers.contains_key("cache") {
        p.cache = Some(mem.clone());
    }
    if config.providers.contains_key("kv") {
        p.kv = Some(mem);
    }
    if config.providers.contains_key("db") {
        p.sql = Some(Arc::new(NoSql));
    }
    p
}

/// Stands in for a database during `keel config validate`, which never connects.
struct NoSql;

#[keel_sdk::async_trait]
impl keel_sdk::capabilities::Sql for NoSql {
    fn dialect(&self) -> &'static str {
        "none"
    }
    async fn execute(&self, _: &str, _: &[Value]) -> keel_sdk::capabilities::CapResult<u64> {
        Ok(0)
    }
    async fn query(
        &self,
        _: &str,
        _: &[Value],
    ) -> keel_sdk::capabilities::CapResult<Vec<serde_json::Map<String, Value>>> {
        Ok(vec![])
    }
}

/// The secret source `secrets:` in app.yaml asks for (environment by default).
async fn secrets_for(path: &Path) -> Result<Arc<LayeredSecrets>> {
    let cfg = AppConfig::peek_secrets(path)?;
    let s = keel_providers::secret_source(cfg.as_ref())
        .await
        .map_err(|e| anyhow!(e))?;
    Ok(Arc::new(s))
}

/// Loads app.yaml with its secret source and checks its plugins.
async fn load_checked(path: &Path, registry: &PluginRegistry) -> Result<(AppConfig, Arc<LayeredSecrets>)> {
    let secrets = secrets_for(path).await?;
    let config = AppConfig::load_file(path, secrets.as_ref())?;
    keel_core::plugins::check(&config, registry, &static_providers(&config))?;
    Ok((config, secrets))
}

async fn connect(config: &AppConfig, secrets: Arc<LayeredSecrets>) -> Result<CapabilityProviders> {
    let mut p = keel_providers::connect(&config.providers).await?;
    p.secrets = Some(secrets);
    Ok(p)
}

async fn start(path: &Path, registry: &PluginRegistry) -> Result<Host> {
    let (config, secrets) = load_checked(path, registry).await?;
    let providers = connect(&config, secrets.clone()).await?;
    let options = HostOptions {
        secrets,
        state_file: Some(SetupState::path_for(path)),
    };
    Ok(Host::start_with(config, registry, &providers, options).await?)
}

async fn serve(path: &Path) -> Result<()> {
    let host = start(path, &plugins::builtin()).await?;
    host.serve(async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await?;
    Ok(())
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Runs the server and restarts it when app.yaml changes. A broken edit is
/// reported and the running server keeps going until the file is fixed.
async fn dev(path: &Path) -> Result<()> {
    let registry = plugins::builtin();
    let mut host = start(path, &registry).await?;
    let mut seen = modified(path);
    loop {
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn(host.serve(async {
            let _ = stop_rx.await;
        }));
        loop {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    let _ = stop_tx.send(());
                    server.await??;
                    return Ok(());
                }
                _ = tokio::time::sleep(Duration::from_millis(500)) => {}
            }
            let now = modified(path);
            if now == seen {
                continue;
            }
            seen = now;
            eprintln!("keel dev: {} changed, reloading", path.display());
            match load_checked(path, &registry).await {
                Ok(_) => break,
                Err(e) => eprintln!("keel dev: not reloading, fix app.yaml first:\n{e:#}"),
            }
        }
        let _ = stop_tx.send(());
        server.await??;
        host = match start(path, &registry).await {
            Ok(h) => h,
            Err(e) => {
                eprintln!("keel dev: restart failed:\n{e:#}\nkeel dev: waiting for app.yaml to change");
                loop {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    let now = modified(path);
                    if now != seen {
                        seen = now;
                        if let Ok(h) = start(path, &registry).await {
                            break h;
                        }
                    }
                }
            }
        };
    }
}

async fn validate(out: Out, path: &Path) -> Result<()> {
    let (config, _) = load_checked(path, &plugins::builtin()).await?;
    out.success(
        format!(
            "ok: {} is valid (app '{}', {} provider(s), {} plugin(s), {} route rule(s))",
            path.display(),
            config.app,
            config.providers.len(),
            config.plugins.len(),
            config.routes.len()
        ),
        json!({
            "file": path.display().to_string(),
            "app": config.app,
            "providers": config.providers.iter().map(|(k, v)| (k.clone(), Value::String(v.kind.clone()))).collect::<serde_json::Map<_, _>>(),
            "plugins": config.plugins.iter().map(|p| p.name()).collect::<Vec<_>>(),
        }),
    );
    Ok(())
}

/// Leaves `${secret:NAME}` references as they are.
struct KeepRefs;

impl SecretSource for KeepRefs {
    fn get(&self, name: &str) -> Option<String> {
        Some(format!("${{secret:{name}}}"))
    }
}

fn expand(out: Out, path: &Path) -> Result<()> {
    let config = AppConfig::load_file(path, &KeepRefs)?;
    let registry = plugins::builtin();
    let plugins: Vec<Value> = config
        .plugins
        .iter()
        .map(|p| {
            let cfg = match registry.get(p.name()) {
                Some(pl) => describe::with_defaults(pl.manifest(), &p.config),
                None => p.config.clone(),
            };
            json!({ "use": p.spec, "config": cfg })
        })
        .collect();
    let expanded = json!({
        "app": config.app,
        "server": { "host": config.server.host, "port": config.server.port },
        "providers": config.providers.iter().map(|(k, v)| {
            let mut m = v.settings.clone();
            m.insert("use".into(), Value::String(v.kind.clone()));
            (k.clone(), Value::Object(m))
        }).collect::<serde_json::Map<_, _>>(),
        "plugins": plugins,
        "routes": config.routes.iter().map(|(k, v)| (k.clone(), json!({ "guard": v.guard }))).collect::<serde_json::Map<_, _>>(),
    });
    out.success(
        serde_yaml::to_string(&expanded)?.trim_end(),
        json!({ "config": expanded }),
    );
    Ok(())
}

fn add(out: Out, spec: &str, path: &Path) -> Result<()> {
    let registry = plugins::builtin();
    let id = spec.split_once('@').map_or(spec, |(i, _)| i);
    let plugin = registry.get(id).ok_or_else(|| {
        anyhow!(
            "plugin `{id}` is not in this build. Compiled-in plugins: {}",
            registry.ids().join(", ")
        )
    })?;
    let m = plugin.manifest();
    let text =
        std::fs::read_to_string(path).with_context(|| format!("{}: cannot read file", path.display()))?;
    let current = AppConfig::load_str(&text, &path.display().to_string(), &KeepRefs)?;
    if current.plugins.iter().any(|p| p.name() == id) {
        bail!("`{id}` is already in {}", path.display());
    }
    let spec = if spec.contains('@') {
        spec.to_string()
    } else {
        format!("{id}@^{}", caret(m))
    };
    let config = describe::minimal_config(m);
    let updated = yaml_edit::add_plugin(&text, &spec, &config)?;
    let parsed = AppConfig::load_str(&updated, &path.display().to_string(), &KeepRefs)?;
    keel_core::plugins::check(&parsed, &registry, &static_providers(&parsed))?;
    std::fs::write(path, &updated)?;
    out.success(
        format!(
            "added {spec} to {}\n\n{}",
            path.display(),
            describe::ai_summary(m)
        ),
        json!({ "added": spec, "file": path.display().to_string(), "config": config }),
    );
    Ok(())
}

fn caret(m: &Manifest) -> String {
    if m.version.major > 0 {
        m.version.major.to_string()
    } else {
        format!("0.{}", m.version.minor)
    }
}

fn list(out: Out) {
    let registry = plugins::builtin();
    let mut text = String::new();
    let mut items = Vec::new();
    for p in registry.iter() {
        let m = p.manifest();
        let routes: Vec<String> = m.routes.iter().map(|r| r.to_string()).collect();
        text.push_str(&format!("{} {}  {}\n", m.id, m.version, m.summary));
        if !routes.is_empty() {
            text.push_str(&format!("    routes: {}\n", routes.join(", ")));
        }
        items.push(json!({
            "id": m.id,
            "version": m.version.to_string(),
            "summary": m.summary,
            "capabilities": m.capabilities.iter().map(|c| c.as_str()).collect::<Vec<_>>(),
            "routes": routes,
        }));
    }
    out.success(text.trim_end(), json!({ "plugins": items }));
}

fn describe_plugin(out: Out, id: &str, ai: bool) -> Result<()> {
    let registry = plugins::builtin();
    let p = registry.get(id).ok_or_else(|| {
        anyhow!(
            "plugin `{id}` is not in this build. Compiled-in plugins: {}",
            registry.ids().join(", ")
        )
    })?;
    let m = p.manifest();
    let summary = describe::ai_summary(m);
    if ai {
        out.success(&summary, json!({ "summary": summary }));
    } else {
        let full = describe::manifest_json(m);
        out.success(
            serde_yaml::to_string(&full)?.trim_end(),
            json!({ "plugin": full }),
        );
    }
    Ok(())
}

fn check_plugin(out: Out, path: &Path) -> Result<()> {
    let (dir, yaml_path) = if path.is_dir() {
        (path.to_path_buf(), path.join("plugin.yaml"))
    } else {
        (
            path.parent().unwrap_or(Path::new(".")).to_path_buf(),
            path.to_path_buf(),
        )
    };
    let yaml = std::fs::read_to_string(&yaml_path)
        .map_err(|e| anyhow!("{}: cannot read file: {e}", yaml_path.display()))?;
    let schema_path = dir.join("config.schema.json");
    let schema = schema_path
        .is_file()
        .then(|| std::fs::read_to_string(&schema_path))
        .transpose()?;
    let m = Manifest::from_sources(&yaml, schema.as_deref())?;
    let caps: Vec<&str> = m.capabilities.iter().map(|c| c.as_str()).collect();
    out.success(
        format!(
            "ok: {} {} is valid ({} route(s), {} event(s), capabilities: {})",
            m.id,
            m.version,
            m.routes.len(),
            m.events.len(),
            if caps.is_empty() {
                "none".to_string()
            } else {
                caps.join(", ")
            }
        ),
        json!({ "plugin": describe::manifest_json(&m) }),
    );
    Ok(())
}

fn new_plugin(out: Out, id: &str) -> Result<()> {
    let root = scaffold::find_workspace(&std::env::current_dir()?)?;
    let made = scaffold::new_plugin(&root, id)?;
    let files: Vec<String> = made
        .files
        .iter()
        .map(|f| f.strip_prefix(&root).unwrap_or(f).display().to_string())
        .collect();
    let mut text = format!(
        "created plugin `{id}` ({}) in {}\n",
        made.crate_name,
        made.dir.display()
    );
    for f in &files {
        text.push_str(&format!("  wrote {f}\n"));
    }
    text.push_str(&format!(
        "\nnext:\n  1. keel add {id}\n  2. cargo run -p keel -- dev\n  3. curl localhost:8080/{id}/hello"
    ));
    out.success(
        text,
        json!({ "plugin": id, "crate": made.crate_name, "files": files }),
    );
    Ok(())
}

fn init(out: Out, dir: &Path, name: Option<&str>) -> Result<()> {
    let files = scaffold::init_project(dir, name)?;
    let names: Vec<String> = files.iter().map(|f| f.display().to_string()).collect();
    let mut text = String::new();
    for f in &names {
        text.push_str(&format!("wrote {f}\n"));
    }
    let cd = if dir == Path::new(".") {
        String::new()
    } else {
        format!("cd {} && ", dir.display())
    };
    text.push_str(&format!("\nnext:\n  {cd}keel dev\n  curl localhost:8080/health"));
    out.success(text, json!({ "files": names }));
    Ok(())
}
