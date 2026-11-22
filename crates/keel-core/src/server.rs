//! The HTTP host: starts plugins from config and serves them.

use std::future::Future;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, MethodRouter};
use axum::{Json, Router};
use keel_sdk::capabilities::{Capability, CapabilityProviders, DegradeLevel};
use keel_sdk::interfaces::Interfaces;
use keel_sdk::{Event, HookBus, Plugin};
use serde_json::{json, Value};
use tokio::net::TcpListener;

use crate::config::AppConfig;
use crate::migrate::{self, MigrateError};
use crate::plugins::{self, PluginLoadError, PluginRegistry};
use crate::routes::{self, RouteRules};
use crate::secrets::{EnvSecrets, SecretSource};
use crate::setup::{self, SetupReport, SetupState};

/// Why a host could not start.
#[derive(Debug, thiserror::Error)]
pub enum HostError {
    #[error(transparent)]
    Plugins(#[from] PluginLoadError),
    #[error(transparent)]
    Migrations(#[from] MigrateError),
    #[error("{0}")]
    State(String),
    #[error("{}", .0.iter().map(|i| format!("{}: {}", i.path, i.message)).collect::<Vec<_>>().join("; "))]
    Routes(Vec<keel_sdk::Issue>),
}

/// How a host finds secrets and setup state.
#[derive(Clone)]
pub struct HostOptions {
    /// Used by setup tracking (`secret-present` checks).
    pub secrets: Arc<dyn SecretSource>,
    /// `.keel/setup.json`; `None` keeps manual steps in memory only.
    pub state_file: Option<PathBuf>,
}

impl Default for HostOptions {
    fn default() -> Self {
        Self {
            secrets: Arc::new(EnvSecrets),
            state_file: None,
        }
    }
}

#[derive(Clone)]
struct HostState {
    config: Arc<AppConfig>,
    providers: CapabilityProviders,
    plugins: Arc<Vec<Arc<dyn Plugin>>>,
    setup: Arc<RwLock<SetupReport>>,
    started: Instant,
}

/// A configured Keel server.
pub struct Host {
    config: Arc<AppConfig>,
    providers: CapabilityProviders,
    hooks: Arc<HookBus>,
    plugins: Vec<Arc<dyn Plugin>>,
    extra: Router,
    options: HostOptions,
    setup: Arc<RwLock<SetupReport>>,
    migrations: Vec<migrate::Applied>,
    interfaces: Interfaces,
    rules: Option<Arc<RouteRules>>,
}

impl Host {
    /// A host with no plugins started (plugin entries in config are ignored).
    pub fn new(config: AppConfig) -> Self {
        Self {
            config: Arc::new(config),
            providers: CapabilityProviders::default(),
            hooks: Arc::new(HookBus::new()),
            plugins: Vec::new(),
            extra: Router::new(),
            options: HostOptions::default(),
            setup: Arc::new(RwLock::new(SetupReport::default())),
            migrations: Vec::new(),
            interfaces: Interfaces::new(),
            rules: None,
        }
    }

    /// [`Host::start_with`] with secrets from the environment and no state file.
    pub async fn start(
        config: AppConfig,
        registry: &PluginRegistry,
        providers: &CapabilityProviders,
    ) -> Result<Self, HostError> {
        Self::start_with(config, registry, providers, HostOptions::default()).await
    }

    /// Checks every plugin `config.plugins` lists, runs their additive
    /// migrations, starts them, and evaluates setup steps.
    pub async fn start_with(
        config: AppConfig,
        registry: &PluginRegistry,
        providers: &CapabilityProviders,
        options: HostOptions,
    ) -> Result<Self, HostError> {
        let config = crate::compose::expand(&config, registry)?.config;
        let mut host = Self::new(config);
        host.providers = providers.clone();
        host.options = options;
        let order = plugins::check(&host.config, registry, providers)?;
        if let Some(sql) = &providers.sql {
            let ordered: Vec<(Arc<dyn Plugin>, Value)> = order
                .iter()
                .map(|(i, p)| (p.clone(), host.config.plugins[*i].config.clone()))
                .collect();
            host.migrations = migrate::run_at_start_with(sql.as_ref(), &ordered).await?;
        }
        let loaded = plugins::load(&host.config, registry, providers, &host.hooks, &host.interfaces).await?;
        for l in loaded {
            host.extra = host.extra.merge(l.router);
            host.plugins.push(l.plugin);
        }
        if !host.config.routes.is_empty() {
            let rules = RouteRules::build(&host.config.routes, &host.interfaces, providers.cache.clone())
                .map_err(HostError::Routes)?;
            host.rules = Some(Arc::new(rules));
        }
        host.refresh_setup().await?;
        Ok(host)
    }

    pub fn config(&self) -> &AppConfig {
        &self.config
    }

    pub fn hooks(&self) -> Arc<HookBus> {
        self.hooks.clone()
    }

    /// Started plugins, in start order.
    pub fn plugins(&self) -> &[Arc<dyn Plugin>] {
        &self.plugins
    }

    /// Interfaces the started plugins offer (identity-provider, authorizer, ...).
    pub fn interfaces(&self) -> &Interfaces {
        &self.interfaces
    }

    /// Migrations applied while starting.
    pub fn migrations(&self) -> &[migrate::Applied] {
        &self.migrations
    }

    /// The latest setup report.
    pub fn setup_report(&self) -> SetupReport {
        self.setup.read().expect("setup lock").clone()
    }

    /// Re-evaluates every setup step (secrets, plugin checks, manual marks).
    pub async fn refresh_setup(&self) -> Result<SetupReport, HostError> {
        let report = evaluate(&self.config, &self.plugins, &self.options).await?;
        *self.setup.write().expect("setup lock") = report.clone();
        Ok(report)
    }

    /// Adds a route outside any plugin. Panics on a path that is already taken, like axum.
    pub fn route(mut self, path: &str, handler: MethodRouter) -> Self {
        self.extra = self.extra.route(path, handler);
        self
    }

    /// The full router: `/health` plus every plugin route. Plugin routes that
    /// an incomplete setup step blocks answer `503 setup_incomplete`.
    pub fn router(&self) -> Router {
        let state = HostState {
            config: self.config.clone(),
            providers: self.providers.clone(),
            plugins: Arc::new(self.plugins.clone()),
            setup: self.setup.clone(),
            started: Instant::now(),
        };
        let mut plugin_routes = self.extra.clone();
        if let Some(rules) = &self.rules {
            plugin_routes =
                plugin_routes.layer(middleware::from_fn_with_state(rules.clone(), routes::middleware));
        }
        let plugin_routes =
            plugin_routes.layer(middleware::from_fn_with_state(self.setup.clone(), setup_gate));
        Router::new()
            .route("/health", get(health))
            .with_state(state)
            .merge(plugin_routes)
    }

    /// Binds `server.host:server.port` and serves until `shutdown` completes.
    pub async fn serve(self, shutdown: impl Future<Output = ()> + Send + 'static) -> std::io::Result<()> {
        let addr = format!("{}:{}", self.config.server.host, self.config.server.port);
        let listener = TcpListener::bind(&addr).await?;
        self.serve_on(listener, shutdown).await
    }

    /// Serves on an existing listener (tests bind port 0).
    pub async fn serve_on(
        self,
        listener: TcpListener,
        shutdown: impl Future<Output = ()> + Send + 'static,
    ) -> std::io::Result<()> {
        let local: SocketAddr = listener.local_addr()?;
        let router = self.router();
        let report = self.setup_report();
        tracing::info!(app = %self.config.app, %local, plugins = self.plugins.len(), "keel host listening");
        if !report.complete {
            tracing::warn!(
                steps = %report.incomplete_ids().join(", "),
                "setup incomplete; blocked routes answer 503 until done (run `keel setup status`)"
            );
        }
        // Pick up secrets and `keel setup done` without a restart.
        let refresher = {
            let (config, plugins, options, setup) = (
                self.config.clone(),
                self.plugins.clone(),
                self.options.clone(),
                self.setup.clone(),
            );
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_secs(10)).await;
                    if let Ok(r) = evaluate(&config, &plugins, &options).await {
                        *setup.write().expect("setup lock") = r;
                    }
                }
            })
        };
        self.hooks
            .emit(Event::new(
                "host.started",
                json!({ "app": self.config.app, "addr": local.to_string() }),
            ))
            .await;
        let served = axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(shutdown)
        .await;
        refresher.abort();
        served?;
        for p in &self.plugins {
            p.drain().await;
        }
        self.hooks
            .emit(Event::new("host.stopped", json!({ "app": self.config.app })))
            .await;
        Ok(())
    }
}

async fn evaluate(
    config: &AppConfig,
    plugins: &[Arc<dyn Plugin>],
    options: &HostOptions,
) -> Result<SetupReport, HostError> {
    let state = match &options.state_file {
        Some(path) => SetupState::load(path).map_err(|e| HostError::State(e.to_string()))?,
        None => SetupState::default(),
    };
    Ok(setup::evaluate(config, plugins, options.secrets.as_ref(), &state, true).await)
}

async fn setup_gate(State(setup): State<Arc<RwLock<SetupReport>>>, req: Request, next: Next) -> Response {
    let blocked = {
        let report = setup.read().expect("setup lock");
        if report.complete {
            None
        } else {
            report.blocked(req.method().as_str(), req.uri().path())
        }
    };
    match blocked {
        Some(e) => e.into_response(),
        None => next.run(req).await,
    }
}

/// `ok`: everything up. `degraded`: something is down or degraded, but
/// nothing at the `error` level is down (HTTP 200). `down`: an error-level
/// plugin or provider is down (HTTP 503).
async fn health(State(s): State<HostState>) -> (StatusCode, Json<Value>) {
    let mut any_issue = false;
    let mut any_error = false;
    let mut plugins = Vec::new();
    for p in s.plugins.iter() {
        let m = p.manifest();
        let level = s
            .config
            .plugins
            .iter()
            .find(|e| e.name() == m.id)
            .and_then(|e| e.degrade_level)
            .unwrap_or(m.degrade_level);
        let h = p.health().await;
        match &h {
            keel_sdk::Health::Ok => {}
            keel_sdk::Health::Degraded { .. } => any_issue = true,
            keel_sdk::Health::Down { .. } => {
                any_issue = true;
                any_error |= level == DegradeLevel::Error;
            }
        }
        let mut entry = json!({ "id": m.id, "version": m.version.to_string(), "degrade_level": level });
        if let (Value::Object(e), Ok(Value::Object(h))) = (&mut entry, serde_json::to_value(&h)) {
            e.extend(h);
        }
        plugins.push(entry);
    }
    let mut providers = Vec::new();
    for info in &s.providers.info {
        let ping = match info.capability {
            Capability::Sql => match &s.providers.sql {
                Some(p) => p.ping().await,
                None => Ok(()),
            },
            Capability::Cache => match &s.providers.cache {
                Some(p) => p.ping().await,
                None => Ok(()),
            },
            Capability::Kv => match &s.providers.kv {
                Some(p) => p.ping().await,
                None => Ok(()),
            },
            _ => Ok(()),
        };
        let mut entry = serde_json::to_value(info).unwrap_or_default();
        match ping {
            Ok(()) => entry["status"] = "ok".into(),
            Err(e) => {
                any_issue = true;
                any_error |= info.degrade_level == DegradeLevel::Error;
                entry["status"] = "down".into();
                entry["reason"] = e.to_string().into();
            }
        }
        providers.push(entry);
    }
    let (status, code) = if any_error {
        ("down", StatusCode::SERVICE_UNAVAILABLE)
    } else if any_issue {
        ("degraded", StatusCode::OK)
    } else {
        ("ok", StatusCode::OK)
    };
    let setup = {
        let r = s.setup.read().expect("setup lock");
        json!({ "complete": r.complete, "incomplete": r.incomplete_ids() })
    };
    let body = Json(json!({
        "status": status,
        "app": s.config.app,
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_secs": s.started.elapsed().as_secs(),
        "plugins": plugins,
        "providers": providers,
        "setup": setup,
    }));
    (code, body)
}
