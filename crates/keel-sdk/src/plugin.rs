//! The `Plugin` trait and what the host hands a plugin during `init`.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::handler::Handler;
use axum::routing::{MethodFilter, MethodRouter};
use axum::Router;
use serde::Serialize;
use serde_json::Value;

use crate::async_trait;
use crate::capabilities::{Capabilities, CapabilityError};
use crate::hooks::{Event, HookBus};
use crate::manifest::Manifest;

#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    #[error("route `{method} {path}` is not declared in provides.routes of plugin.yaml")]
    UndeclaredRoute { method: String, path: String },
    #[error("event `{0}` is not declared in provides.events of plugin.yaml")]
    UndeclaredEvent(String),
    #[error(transparent)]
    Capability(#[from] CapabilityError),
    #[error("live config changes are not supported by this plugin")]
    ConfigureUnsupported,
    #[error("{0}")]
    Other(String),
}

impl PluginError {
    pub fn other(message: impl Into<String>) -> Self {
        Self::Other(message.into())
    }
}

/// Health of one plugin, reported under `/health`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Health {
    Ok,
    Degraded { reason: String },
    Down { reason: String },
}

/// A Keel plugin. Every method except [`Plugin::manifest`] has a default, so a
/// minimal plugin only implements `manifest` and `init`.
///
/// Lifecycle, as in the v1 spec:
/// `manifest` -> `init` (on start or enable) -> `handle` (routes) /
/// `on_event` -> `configure` (live config change) -> `drain` (before swap or
/// removal) -> `export_state` / `import_state` (carry state across a swap).
#[async_trait]
pub trait Plugin: Send + Sync + 'static {
    /// The parsed `plugin.yaml`. Use [`crate::manifest!`].
    fn manifest(&self) -> &Manifest;

    /// Called once with the validated config. Register routes and events here.
    async fn init(&self, ctx: &mut InitContext) -> Result<(), PluginError>;

    /// A change to a key listed in `runtime_config`, applied without restart.
    async fn configure(&self, _patch: Value) -> Result<(), PluginError> {
        Err(PluginError::ConfigureUnsupported)
    }

    /// An event listed in `consumes.events`.
    async fn on_event(&self, _event: &Event) {}

    async fn health(&self) -> Health {
        Health::Ok
    }

    /// Finish in-flight work before an update or removal.
    async fn drain(&self) {}

    fn export_state(&self) -> Option<Value> {
        None
    }

    fn import_state(&self, _state: Value) {}
}

/// Routes a plugin registers. Only routes declared in `plugin.yaml` are
/// accepted, so the manifest always tells the truth about the plugin.
pub struct Routes {
    manifest_routes: Vec<(String, String)>,
    by_path: BTreeMap<String, MethodRouter>,
    registered: Vec<(String, String)>,
}

macro_rules! method {
    ($name:ident, $m:literal, $filter:expr) => {
        pub fn $name<H, T>(&mut self, path: &str, handler: H) -> Result<&mut Self, PluginError>
        where
            H: Handler<T, ()>,
            T: 'static,
        {
            self.add($m, $filter, path, handler)
        }
    };
}

impl Routes {
    fn new(manifest: &Manifest) -> Self {
        Self {
            manifest_routes: manifest
                .routes
                .iter()
                .map(|r| (r.method.clone(), r.path.clone()))
                .collect(),
            by_path: BTreeMap::new(),
            registered: Vec::new(),
        }
    }

    method!(get, "GET", MethodFilter::GET);
    method!(post, "POST", MethodFilter::POST);
    method!(put, "PUT", MethodFilter::PUT);
    method!(patch, "PATCH", MethodFilter::PATCH);
    method!(delete, "DELETE", MethodFilter::DELETE);

    fn add<H, T>(
        &mut self,
        method: &str,
        filter: MethodFilter,
        path: &str,
        handler: H,
    ) -> Result<&mut Self, PluginError>
    where
        H: Handler<T, ()>,
        T: 'static,
    {
        let key = (method.to_string(), path.to_string());
        if !self.manifest_routes.contains(&key) {
            return Err(PluginError::UndeclaredRoute {
                method: key.0,
                path: key.1,
            });
        }
        let mr = self.by_path.remove(path).unwrap_or_default().on(filter, handler);
        self.by_path.insert(path.to_string(), mr);
        self.registered.push(key);
        Ok(self)
    }

    /// Declared routes the plugin never registered.
    pub fn missing(&self) -> Vec<String> {
        self.manifest_routes
            .iter()
            .filter(|r| !self.registered.contains(r))
            .map(|(m, p)| format!("{m} {p}"))
            .collect()
    }

    /// Builds the router for everything registered.
    pub fn into_router(self) -> Router {
        self.by_path
            .into_iter()
            .fold(Router::new(), |r, (path, mr)| r.route(&path, mr))
    }
}

/// What a plugin gets in [`Plugin::init`].
pub struct InitContext {
    /// The plugin's `config:` block from `app.yaml`, already validated
    /// against its `config.schema.json`, with secrets resolved.
    pub config: Value,
    pub capabilities: Capabilities,
    routes: Routes,
    hooks: Arc<HookBus>,
    events: Vec<String>,
}

impl InitContext {
    pub fn new(manifest: &Manifest, config: Value, capabilities: Capabilities, hooks: Arc<HookBus>) -> Self {
        Self {
            config,
            capabilities,
            routes: Routes::new(manifest),
            hooks,
            events: manifest.events.clone(),
        }
    }

    pub fn routes(&mut self) -> &mut Routes {
        &mut self.routes
    }

    /// An emitter for events this plugin declared in `provides.events`.
    pub fn emitter(&self) -> Emitter {
        Emitter {
            hooks: self.hooks.clone(),
            allowed: self.events.clone(),
        }
    }

    /// Splits into the parts the host keeps.
    pub fn finish(self) -> Routes {
        self.routes
    }
}

/// Emits events a plugin declared. Cheap to clone into handlers.
#[derive(Clone)]
pub struct Emitter {
    hooks: Arc<HookBus>,
    allowed: Vec<String>,
}

impl Emitter {
    pub async fn emit(&self, name: &str, payload: Value) -> Result<usize, PluginError> {
        if !self.allowed.iter().any(|e| e == name) {
            return Err(PluginError::UndeclaredEvent(name.to_string()));
        }
        Ok(self.hooks.emit(Event::new(name, payload)).await)
    }
}
