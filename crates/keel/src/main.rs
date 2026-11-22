mod describe;
mod dotenv;
mod live_ops;
mod mcp;
mod ops;
mod output;
mod plugins;
mod scaffold;
#[cfg(unix)]
mod supervisor;
mod yaml_edit;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand};
use keel_core::live::Live;
use keel_core::setup::SetupState;
use keel_core::{AppConfig, HostOptions, PluginRegistry, SecretSource};
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
    /// Start the server from app.yaml. Plugins can then change live through
    /// `keel apply` and the admin API (needs the KEEL_ADMIN_TOKEN secret).
    Serve {
        #[arg(short, long, default_value = "app.yaml")]
        config: PathBuf,
        /// Rollback drill: make /health answer 503 after this many seconds.
        #[arg(long, hide = true, value_name = "SECS")]
        drill_unhealthy_after: Option<u64>,
    },
    /// Start the server and apply app.yaml edits live as you save.
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
    /// Reconcile the running server with app.yaml, live: plugins start,
    /// stop or reconfigure with no restart and no dropped requests.
    Apply {
        #[arg(short, long, default_value = "app.yaml")]
        config: PathBuf,
        /// Server URL; defaults to server.host and server.port in app.yaml.
        #[arg(long)]
        url: Option<String>,
    },
    /// Recent changes from the server's audit log (who, what, when).
    Audit {
        #[arg(short, long, default_value = "app.yaml")]
        config: PathBuf,
        #[arg(long)]
        url: Option<String>,
        #[arg(long, default_value_t = 30)]
        limit: u32,
    },
    /// Run the server under a supervisor that can swap in a new binary with
    /// zero dropped requests (`keel update`) and roll back (`keel rollback`).
    Supervise {
        #[arg(short, long, default_value = "app.yaml")]
        config: PathBuf,
        /// Binary to run; defaults to this one.
        #[arg(long)]
        binary: Option<PathBuf>,
        /// After a swap, watch health and errors this long and roll back on failure.
        #[arg(long, default_value_t = 60)]
        watch_secs: u64,
    },
    /// Swap the supervised server to a new binary: verify, check config,
    /// migrate, warm up, swap, watch, and roll back automatically on failure.
    Update {
        #[arg(short, long, default_value = "app.yaml")]
        config: PathBuf,
        /// The new keel binary.
        #[arg(long, conflicts_with = "build")]
        binary: Option<PathBuf>,
        /// Run `cargo build --release --bin keel` and use target/release/keel.
        #[arg(long)]
        build: bool,
    },
    /// Swap the supervised server back to the previous binary.
    Rollback {
        #[arg(short, long, default_value = "app.yaml")]
        config: PathBuf,
    },
    /// What the supervisor is running (binary hash, pid, previous binary).
    Status {
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
    /// Print app.yaml with bundles (composers such as saas-auth) expanded into
    /// their plugins and every default filled in. Secrets stay as references.
    Expand {
        #[arg(short, long, default_value = "app.yaml")]
        config: PathBuf,
    },
    /// Print the running server's config, with live changes, as app.yaml.
    Pull {
        #[arg(short, long, default_value = "app.yaml")]
        config: PathBuf,
        #[arg(long)]
        url: Option<String>,
        /// Overwrite app.yaml with it (comments are not kept).
        #[arg(long)]
        write: bool,
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
    /// Start a plugin on the running server (no restart).
    Enable {
        /// Plugin id, optionally with a version range.
        plugin: String,
        /// Its config as JSON or YAML, e.g. '{ message: hi }'.
        #[arg(long = "with", value_name = "CONFIG")]
        with: Option<String>,
        #[arg(short, long, default_value = "app.yaml")]
        config: PathBuf,
        #[arg(long)]
        url: Option<String>,
    },
    /// Stop a plugin on the running server (no restart).
    Disable {
        id: String,
        #[arg(short, long, default_value = "app.yaml")]
        config: PathBuf,
        #[arg(long)]
        url: Option<String>,
    },
    /// Change config keys of a running plugin: `keel plugins set ping message=hi`.
    Set {
        id: String,
        /// key=value pairs; values are read as YAML.
        #[arg(required = true)]
        pairs: Vec<String>,
        #[arg(short, long, default_value = "app.yaml")]
        config: PathBuf,
        #[arg(long)]
        url: Option<String>,
    },
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let out = Out { json: cli.json };
    let serving = matches!(
        cli.command,
        Command::Serve { .. } | Command::Dev { .. } | Command::Supervise { .. }
    );
    if serving || std::env::var_os("RUST_LOG").is_some() {
        use tracing_subscriber::layer::SubscriberExt;
        use tracing_subscriber::util::SubscriberInitExt;
        // Servers also log to .keel/keel.log beside app.yaml (read by `keel mcp`).
        let file = match &cli.command {
            Command::Serve { config, .. } | Command::Dev { config } | Command::Supervise { config, .. } => {
                log_file(config)
            }
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
    if let Command::Serve { config, .. }
    | Command::Dev { config }
    | Command::Check { config }
    | Command::Migrate { config, .. }
    | Command::Supervise { config, .. }
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
        Command::Serve {
            config,
            drill_unhealthy_after,
        } => serve(&config, drill_unhealthy_after).await,
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
        Command::Apply { config, url } => live_ops::apply(out, &config, url.as_deref()).await,
        Command::Audit { config, url, limit } => live_ops::audit(out, &config, url.as_deref(), limit).await,
        Command::Config {
            command: ConfigCommand::Pull { config, url, write },
        } => live_ops::pull(out, &config, url.as_deref(), write).await,
        Command::Plugins {
            command:
                PluginsCommand::Enable {
                    plugin,
                    with,
                    config,
                    url,
                },
        } => live_ops::enable(out, &config, url.as_deref(), &plugin, with.as_deref()).await,
        Command::Plugins {
            command: PluginsCommand::Disable { id, config, url },
        } => live_ops::disable(out, &config, url.as_deref(), &id).await,
        Command::Plugins {
            command:
                PluginsCommand::Set {
                    id,
                    pairs,
                    config,
                    url,
                },
        } => live_ops::set(out, &config, url.as_deref(), &id, &pairs).await,
        Command::Supervise {
            config,
            binary,
            watch_secs,
        } => supervise(&config, binary, watch_secs).await,
        Command::Update {
            config,
            binary,
            build,
        } => update(out, &config, binary, build).await,
        Command::Rollback { config } => {
            supervisor_cmd(
                out,
                &config,
                json!({ "cmd": "rollback", "actor": live_ops::actor() }),
            )
            .await
        }
        Command::Status { config } => supervisor_cmd(out, &config, json!({ "cmd": "status" })).await,
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

#[cfg(unix)]
async fn supervise(config: &Path, binary: Option<PathBuf>, watch_secs: u64) -> Result<()> {
    supervisor::run(config, binary, watch_secs).await
}

#[cfg(unix)]
async fn supervisor_cmd(out: Out, config: &Path, cmd: Value) -> Result<()> {
    supervisor::send(out, config, cmd).await
}

#[cfg(not(unix))]
async fn supervise(_: &Path, _: Option<PathBuf>, _: u64) -> Result<()> {
    bail!("keel supervise needs a Unix system (Linux or macOS)")
}

#[cfg(not(unix))]
async fn supervisor_cmd(_: Out, _: &Path, _: Value) -> Result<()> {
    bail!("the supervisor needs a Unix system (Linux or macOS)")
}

async fn update(out: Out, config: &Path, binary: Option<PathBuf>, build: bool) -> Result<()> {
    let binary = match (binary, build) {
        (Some(b), _) => b,
        (None, true) => {
            eprintln!("keel update: cargo build --release --bin keel");
            let status = tokio::process::Command::new("cargo")
                .args(["build", "--release", "--bin", "keel"])
                .status()
                .await
                .context("cannot run cargo")?;
            if !status.success() {
                bail!("the build failed; nothing changed");
            }
            PathBuf::from("target/release/keel")
        }
        (None, false) => bail!("pass --binary <path to the new keel> or --build"),
    };
    let binary =
        std::fs::canonicalize(&binary).with_context(|| format!("{}: not found", binary.display()))?;
    supervisor_cmd(
        out,
        config,
        json!({ "cmd": "swap", "binary": binary, "actor": live_ops::actor() }),
    )
    .await
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
    let config = keel_core::compose::expand(&config, registry)?.config;
    keel_core::plugins::check(&config, registry, &static_providers(&config))?;
    Ok((config, secrets))
}

async fn connect(config: &AppConfig, secrets: Arc<LayeredSecrets>) -> Result<CapabilityProviders> {
    let mut p = keel_providers::connect(&config.providers).await?;
    p.secrets = Some(secrets);
    Ok(p)
}

async fn start_live(path: &Path, registry: PluginRegistry) -> Result<Live> {
    let (config, secrets) = load_checked(path, &registry).await?;
    let providers = connect(&config, secrets.clone()).await?;
    let options = HostOptions {
        secrets,
        state_file: Some(SetupState::path_for(path)),
    };
    let text =
        std::fs::read_to_string(path).with_context(|| format!("{}: cannot read file", path.display()))?;
    Ok(Live::start_yaml(&text, registry, providers, options).await?)
}

/// Completes on Ctrl-C or SIGTERM (what a supervisor sends to drain).
async fn stop_signal() {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

/// The listener: inherited from a supervisor (`KEEL_LISTEN_FD`) or bound
/// from `server:` in app.yaml.
async fn listener_for(live: &Live) -> Result<tokio::net::TcpListener> {
    #[cfg(unix)]
    if let Some(fd) = std::env::var("KEEL_LISTEN_FD")
        .ok()
        .and_then(|v| v.parse::<i32>().ok())
    {
        use std::os::fd::FromRawFd;
        // SAFETY: the supervisor passes an open listening socket as this fd.
        let std_listener = unsafe { std::net::TcpListener::from_raw_fd(fd) };
        std_listener.set_nonblocking(true)?;
        return Ok(tokio::net::TcpListener::from_std(std_listener)?);
    }
    let c = live.config();
    let addr = format!("{}:{}", c.server.host, c.server.port);
    tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("cannot listen on {addr}"))
}

async fn serve(path: &Path, drill_unhealthy_after: Option<u64>) -> Result<()> {
    let live = start_live(path, plugins::builtin()).await?;
    if let Some(secs) = drill_unhealthy_after {
        tracing::warn!(secs, "rollback drill: /health will answer 503");
        live.drill_unhealthy_after(Duration::from_secs(secs));
    }
    let listener = listener_for(&live).await?;
    // Supervised: a private loopback port for health checks, then a ready line.
    if let Ok(addr) = std::env::var("KEEL_CONTROL") {
        let control = tokio::net::TcpListener::bind(&addr).await?;
        let local = control.local_addr()?;
        let router = live.control_router();
        tokio::spawn(async move {
            let _ = axum::serve(control, router).await;
        });
        use std::io::Write;
        println!(
            "KEEL_READY {}",
            json!({ "control": local.to_string(), "pid": std::process::id(), "version": env!("CARGO_PKG_VERSION") })
        );
        let _ = std::io::stdout().flush();
    }
    live.serve_on(listener, stop_signal()).await?;
    Ok(())
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Runs the server and applies app.yaml edits live (no restart). Changes a
/// live apply cannot make (providers, server) restart the server. A broken
/// edit is reported and the running server keeps going.
async fn dev(path: &Path) -> Result<()> {
    let mut seen = modified(path);
    loop {
        let live = start_live(path, plugins::builtin()).await?;
        let listener = listener_for(&live).await?;
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
        let server = {
            let live = live.clone();
            tokio::spawn(async move {
                live.serve_on(listener, async {
                    let _ = stop_rx.await;
                })
                .await
            })
        };
        let restart = loop {
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
            let Ok(text) = std::fs::read_to_string(path) else {
                continue;
            };
            match live.apply_yaml(&text, "keel dev").await {
                Ok(c) if c.mode == "none" => {}
                Ok(c) => eprintln!(
                    "keel dev: applied live (version {}; added {:?}, removed {:?}, changed {:?})",
                    c.version, c.added, c.removed, c.changed
                ),
                Err(keel_core::live::LiveError::Refused(m)) => {
                    eprintln!("keel dev: {m}; restarting");
                    break true;
                }
                Err(e) => eprintln!("keel dev: not applied, fix app.yaml first:\n{e}"),
            }
        };
        if restart {
            let _ = stop_tx.send(());
            server.await??;
        }
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
    let expanded = keel_core::compose::expand(&config, &registry)?;
    let config = expanded.config;
    let plugins: Vec<Value> = config
        .plugins
        .iter()
        .zip(&expanded.origins)
        .map(|(p, origin)| {
            let cfg = match registry.get(p.name()) {
                Some(pl) => describe::with_defaults(pl.manifest(), &p.config),
                None => p.config.clone(),
            };
            let mut e = json!({ "use": p.spec, "config": cfg });
            if let Some(o) = origin {
                e["from"] = json!(o);
            }
            e
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
    let parsed = keel_core::compose::expand(&parsed, &registry)?.config;
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
        if !m.tags.is_empty() {
            text.push_str(&format!("    tags: {}\n", m.tags.join(", ")));
        }
        if !routes.is_empty() {
            text.push_str(&format!("    routes: {}\n", routes.join(", ")));
        }
        items.push(json!({
            "id": m.id,
            "version": m.version.to_string(),
            "summary": m.summary,
            "tags": m.tags,
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
