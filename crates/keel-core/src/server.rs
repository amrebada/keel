//! The HTTP host: builds the router from config and serves it.

use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use axum::extract::State;
use axum::routing::{get, MethodRouter};
use axum::{Json, Router};
use serde_json::{json, Value};
use tokio::net::TcpListener;

use crate::config::AppConfig;
use crate::hooks::{Event, HookBus};

#[derive(Clone)]
struct HostState {
    config: Arc<AppConfig>,
    started: Instant,
}

/// A configured Keel server. Plugins add routes with [`Host::route`] before it starts.
pub struct Host {
    config: Arc<AppConfig>,
    hooks: Arc<HookBus>,
    extra: Router,
}

impl Host {
    pub fn new(config: AppConfig) -> Self {
        Self {
            config: Arc::new(config),
            hooks: Arc::new(HookBus::new()),
            extra: Router::new(),
        }
    }

    pub fn config(&self) -> &AppConfig {
        &self.config
    }

    pub fn hooks(&self) -> Arc<HookBus> {
        self.hooks.clone()
    }

    /// Adds a route. Panics on a path that is already taken, like axum.
    pub fn route(mut self, path: &str, handler: MethodRouter) -> Self {
        self.extra = self.extra.route(path, handler);
        self
    }

    /// The full router: `/health` plus every added route.
    pub fn router(&self) -> Router {
        let state = HostState {
            config: self.config.clone(),
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
        tracing::info!(app = %self.config.app, %local, "keel host listening");
        self.hooks
            .emit(Event::new(
                "host.started",
                json!({ "app": self.config.app, "addr": local.to_string() }),
            ))
            .await;
        axum::serve(listener, router)
            .with_graceful_shutdown(shutdown)
            .await?;
        self.hooks
            .emit(Event::new("host.stopped", json!({ "app": self.config.app })))
            .await;
        Ok(())
    }
}

async fn health(State(s): State<HostState>) -> Json<Value> {
    Json(json!({
        "status": "ok",
        "app": s.config.app,
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_secs": s.started.elapsed().as_secs(),
        "plugins_configured": s.config.plugins.iter().map(|p| p.name()).collect::<Vec<_>>(),
        "providers_configured": s.config.providers.keys().collect::<Vec<_>>(),
    }))
}
