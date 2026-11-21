//! The HTTP host: starts plugins from config and serves them.

use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, MethodRouter};
use axum::{Json, Router};
use keel_sdk::capabilities::{Capability, CapabilityProviders};
use keel_sdk::{Event, HookBus, Plugin};
use serde_json::{json, Value};
use tokio::net::TcpListener;

use crate::config::AppConfig;
use crate::plugins::{self, PluginLoadError, PluginRegistry};

#[derive(Clone)]
struct HostState {
    config: Arc<AppConfig>,
    providers: CapabilityProviders,
    plugins: Arc<Vec<Arc<dyn Plugin>>>,
    started: Instant,
}

/// A configured Keel server.
pub struct Host {
    config: Arc<AppConfig>,
    providers: CapabilityProviders,
    hooks: Arc<HookBus>,
    plugins: Vec<Arc<dyn Plugin>>,
    extra: Router,
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
        }
    }

    /// Checks and starts every plugin `config.plugins` lists, taken from `registry`.
    pub async fn start(
        config: AppConfig,
        registry: &PluginRegistry,
        providers: &CapabilityProviders,
    ) -> Result<Self, PluginLoadError> {
        let mut host = Self::new(config);
        host.providers = providers.clone();
        let loaded = plugins::load(&host.config, registry, providers, &host.hooks).await?;
        for l in loaded {
            host.extra = host.extra.merge(l.router);
            host.plugins.push(l.plugin);
        }
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

    /// Adds a route outside any plugin. Panics on a path that is already taken, like axum.
    pub fn route(mut self, path: &str, handler: MethodRouter) -> Self {
        self.extra = self.extra.route(path, handler);
        self
    }

    /// The full router: `/health` plus every plugin route.
    pub fn router(&self) -> Router {
        let state = HostState {
            config: self.config.clone(),
            providers: self.providers.clone(),
            plugins: Arc::new(self.plugins.clone()),
            started: Instant::now(),
        };
        Router::new()
            .route("/health", get(health))
            .with_state(state)
            .merge(self.extra.clone())
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
        tracing::info!(app = %self.config.app, %local, plugins = self.plugins.len(), "keel host listening");
        self.hooks
            .emit(Event::new(
                "host.started",
                json!({ "app": self.config.app, "addr": local.to_string() }),
            ))
            .await;
        axum::serve(listener, router)
            .with_graceful_shutdown(shutdown)
            .await?;
        for p in &self.plugins {
            p.drain().await;
        }
        self.hooks
            .emit(Event::new("host.stopped", json!({ "app": self.config.app })))
            .await;
        Ok(())
    }
}

/// 200 when everything is ok, 503 when a plugin or provider is down, so load
/// balancers and orchestrators stop routing to a degraded instance.
async fn health(State(s): State<HostState>) -> (StatusCode, Json<Value>) {
    let mut plugins = Vec::new();
    let mut all_ok = true;
    for p in s.plugins.iter() {
        let m = p.manifest();
        let h = p.health().await;
        all_ok &= h == keel_sdk::Health::Ok;
        let mut entry = json!({ "id": m.id, "version": m.version.to_string() });
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
                all_ok = false;
                entry["status"] = "down".into();
                entry["reason"] = e.to_string().into();
            }
        }
        providers.push(entry);
    }
    let code = if all_ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    let body = Json(json!({
        "status": if all_ok { "ok" } else { "degraded" },
        "app": s.config.app,
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_secs": s.started.elapsed().as_secs(),
        "plugins": plugins,
        "providers": providers,
    }));
    (code, body)
}
