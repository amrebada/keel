//! Live operations: enable, disable and reconfigure plugins while the server
//! runs, with no restart and no dropped requests.
//!
//! A [`Live`] server keeps one *generation* (a started [`Host`] and its
//! router) current. A change builds and starts a new generation next to the
//! running one; when it is ready, new requests go to it, and the old one
//! finishes its in-flight requests before removed plugins get `drain()`.
//! A change that fails at any step leaves the running generation untouched.
//!
//! Keys a plugin lists in `runtime_config` are changed through
//! [`Plugin::configure`] in place when the plugin supports it.
//!
//! The admin API under `/_admin` (bearer `KEEL_ADMIN_TOKEN`) drives this:
//! `GET|PUT /config`, `GET|POST /plugins`, `PUT|DELETE /plugins/{id}`,
//! `PATCH /plugins/{id}/config`, `GET /setup`, `GET /audit`, `GET /stats`.

use std::future::Future;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::body::Bytes;
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch};
use axum::{Json, Router};
use keel_sdk::capabilities::CapabilityProviders;
use keel_sdk::{ApiError, Event, Issue, PluginError};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use tokio::net::TcpListener;
use tower::ServiceExt;

use crate::compose;
use crate::config::{AppConfig, ConfigError};
use crate::plugins::{PluginLoadError, PluginRegistry};
use crate::server::{Host, HostError, HostOptions};

/// Secret that unlocks the admin API.
pub const ADMIN_TOKEN: &str = "KEEL_ADMIN_TOKEN";

/// How long a retired generation may take to finish its requests.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, thiserror::Error)]
pub enum LiveError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Plugins(#[from] PluginLoadError),
    #[error(transparent)]
    Host(#[from] HostError),
    #[error("{0}")]
    Refused(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Invalid(String),
}

impl LiveError {
    fn api(&self) -> ApiError {
        let issues = |list: &[Issue]| -> Vec<Value> {
            list.iter()
                .map(|i| json!({ "path": i.path, "message": i.message }))
                .collect()
        };
        match self {
            LiveError::Config(ConfigError::Invalid { issues: list, .. }) => ApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_config",
                self.to_string(),
            )
            .with("issues", issues(list)),
            LiveError::Plugins(PluginLoadError(list))
            | LiveError::Host(HostError::Plugins(PluginLoadError(list))) => ApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_config",
                self.to_string(),
            )
            .with("issues", issues(list)),
            LiveError::Config(_) | LiveError::Invalid(_) => ApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_config",
                self.to_string(),
            ),
            LiveError::Host(HostError::Migrations(_)) => {
                ApiError::conflict("migration_blocked", self.to_string())
            }
            LiveError::Host(_) => {
                ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "start_failed", self.to_string())
            }
            LiveError::Refused(m) => ApiError::conflict("refused", m.clone()),
            LiveError::NotFound(m) => ApiError::not_found("not_found", m.clone()),
        }
    }
}

/// What a change did.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Change {
    /// `none` (nothing changed), `live` (configure in place) or `reload`
    /// (a new generation took over).
    pub mode: String,
    pub version: u64,
    pub generation: u64,
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub changed: Vec<String>,
    /// Migrations the new generation ran.
    pub migrations: Vec<String>,
}

#[derive(Clone)]
struct Gen {
    number: u64,
    version: u64,
    /// The app.yaml document as written, secret references in place.
    source: Value,
    /// Composers expanded, secrets resolved.
    expanded: AppConfig,
    origins: Vec<Option<String>>,
    host: Arc<Host>,
    router: Router,
    inflight: Arc<AtomicUsize>,
}

struct Inner {
    registry: PluginRegistry,
    providers: CapabilityProviders,
    options: HostOptions,
    current: RwLock<Gen>,
    apply_lock: tokio::sync::Mutex<()>,
    requests: AtomicU64,
    errors: AtomicU64,
    started: Instant,
    /// Rollback drills: `/health` answers 503 after this long.
    drill_unhealthy_after: RwLock<Option<Duration>>,
    /// Shutting down: answers carry `Connection: close`.
    draining: std::sync::atomic::AtomicBool,
}

/// A server whose plugins can change while it runs.
#[derive(Clone)]
pub struct Live(Arc<Inner>);

fn yaml_err(e: impl std::fmt::Display) -> LiveError {
    LiveError::Invalid(format!("app.yaml: not valid YAML: {e}"))
}

/// The newest config version in the audit log, so numbering carries on
/// across restarts and binary swaps; 0 when there is none.
async fn last_version(providers: &CapabilityProviders) -> u64 {
    let Some(sql) = &providers.sql else {
        return 0;
    };
    let rows = sql
        .query(
            "SELECT detail FROM keel_audit WHERE action IN ('config.apply', 'plugin.enable', 'plugin.disable', 'plugin.configure') ORDER BY at DESC, id DESC LIMIT 50",
            &[],
        )
        .await
        .unwrap_or_default();
    rows.iter()
        .filter_map(|r| match r.get("detail") {
            Some(Value::String(d)) => serde_json::from_str::<Value>(d).ok(),
            Some(v) => Some(v.clone()),
            None => None,
        })
        .filter_map(|d| d["version"].as_u64())
        .max()
        .unwrap_or(0)
}

impl Live {
    /// Starts the first generation from the app.yaml document `source`.
    pub async fn start(
        source: Value,
        registry: PluginRegistry,
        providers: CapabilityProviders,
        options: HostOptions,
    ) -> Result<Self, LiveError> {
        let (expanded, origins, host) = build(&source, &registry, &providers, &options).await?;
        let router = host.router();
        let version = last_version(&providers).await + 1;
        let gen = Gen {
            number: 1,
            version,
            source,
            expanded,
            origins,
            host: Arc::new(host),
            router,
            inflight: Arc::default(),
        };
        Ok(Live(Arc::new(Inner {
            registry,
            providers,
            options,
            current: RwLock::new(gen),
            apply_lock: tokio::sync::Mutex::new(()),
            requests: AtomicU64::new(0),
            errors: AtomicU64::new(0),
            started: Instant::now(),
            drill_unhealthy_after: RwLock::new(None),
            draining: std::sync::atomic::AtomicBool::new(false),
        })))
    }

    /// [`Live::start`] from YAML text.
    pub async fn start_yaml(
        yaml: &str,
        registry: PluginRegistry,
        providers: CapabilityProviders,
        options: HostOptions,
    ) -> Result<Self, LiveError> {
        let source: Value = serde_yaml::from_str(yaml).map_err(yaml_err)?;
        Self::start(source, registry, providers, options).await
    }

    fn current(&self) -> Gen {
        self.0.current.read().expect("generation lock").clone()
    }

    /// The running host (current generation).
    pub fn host(&self) -> Arc<Host> {
        self.current().host
    }

    /// The running config, composers expanded.
    pub fn config(&self) -> AppConfig {
        self.current().expanded
    }

    /// The running app.yaml document, with runtime changes and secret
    /// references in place (what `keel config pull` writes).
    pub fn source(&self) -> Value {
        self.current().source
    }

    pub fn version(&self) -> u64 {
        self.current().version
    }

    /// Makes `/health` answer 503 once `after` has passed (rollback drills).
    pub fn drill_unhealthy_after(&self, after: Duration) {
        *self.0.drill_unhealthy_after.write().expect("drill lock") = Some(after);
    }

    fn drilling(&self) -> bool {
        self.0
            .drill_unhealthy_after
            .read()
            .expect("drill lock")
            .is_some_and(|d| self.0.started.elapsed() >= d)
    }

    /// Request counters: (requests, 5xx answers, in flight).
    pub fn stats(&self) -> Value {
        let g = self.current();
        json!({
            "pid": std::process::id(),
            "generation": g.number,
            "version": g.version,
            "requests": self.0.requests.load(Ordering::Relaxed),
            "errors": self.0.errors.load(Ordering::Relaxed),
            "inflight": g.inflight.load(Ordering::Relaxed),
            "uptime_secs": self.0.started.elapsed().as_secs(),
            "plugins": g.expanded.plugins.iter().map(|p| p.name().to_string()).collect::<Vec<_>>(),
        })
    }

    // ---- changes --------------------------------------------------------

    /// Reconciles the server with a whole new app.yaml document.
    pub async fn apply(&self, source: Value, actor: &str) -> Result<Change, LiveError> {
        let _l = self.0.apply_lock.lock().await;
        self.apply_locked(source, actor, "config.apply").await
    }

    /// [`Live::apply`] from YAML text.
    pub async fn apply_yaml(&self, yaml: &str, actor: &str) -> Result<Change, LiveError> {
        let source: Value = serde_yaml::from_str(yaml).map_err(yaml_err)?;
        self.apply(source, actor).await
    }

    async fn apply_locked(&self, source: Value, actor: &str, action: &str) -> Result<Change, LiveError> {
        let old = self.current();
        for key in ["providers", "server", "secrets", "app"] {
            if old.source.get(key) != source.get(key) {
                return Err(LiveError::Refused(format!(
                    "`{key}` changed; providers, server, secrets and app need a restart or a binary swap (keel update), not a live apply"
                )));
            }
        }
        if old.source == source {
            return Ok(Change {
                mode: "none".into(),
                version: old.version,
                generation: old.number,
                ..Default::default()
            });
        }
        let (expanded, origins, host) =
            build(&source, &self.0.registry, &self.0.providers, &self.0.options).await?;
        let mut change = diff(&old.expanded, &expanded);
        change.migrations = host.migrations().iter().map(|m| m.id.clone()).collect();
        let router = host.router();
        let gen = Gen {
            number: old.number + 1,
            version: old.version + 1,
            source,
            expanded,
            origins,
            host: Arc::new(host),
            router,
            inflight: Arc::default(),
        };
        change.mode = "reload".into();
        change.version = gen.version;
        change.generation = gen.number;
        let new_hooks = gen.host.hooks();
        *self.0.current.write().expect("generation lock") = gen;
        tracing::info!(
            generation = change.generation,
            version = change.version,
            added = ?change.added,
            removed = ?change.removed,
            changed = ?change.changed,
            "live change applied"
        );
        new_hooks
            .emit(Event::new(
                "host.reconfigured",
                serde_json::to_value(&change).unwrap_or_default(),
            ))
            .await;
        self.retire(old, &change.removed);
        self.audit(
            actor,
            action,
            "app",
            serde_json::to_value(&change).unwrap_or_default(),
        )
        .await;
        Ok(change)
    }

    /// Lets the old generation finish in-flight requests, then drains the
    /// plugins that are gone.
    fn retire(&self, old: Gen, removed: &[String]) {
        let plugins: Vec<_> = old
            .host
            .plugins()
            .iter()
            .filter(|p| removed.contains(&p.manifest().id))
            .cloned()
            .collect();
        tokio::spawn(async move {
            let start = Instant::now();
            while old.inflight.load(Ordering::Acquire) > 0 && start.elapsed() < DRAIN_TIMEOUT {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            for p in plugins {
                p.drain().await;
            }
        });
    }

    /// Adds a plugin entry (`{ use, config }`) and applies.
    pub async fn enable(&self, entry: Value, actor: &str) -> Result<Change, LiveError> {
        let _l = self.0.apply_lock.lock().await;
        let g = self.current();
        let spec = entry
            .get("use")
            .and_then(Value::as_str)
            .ok_or_else(|| LiveError::Invalid("send { use: <plugin id>[@range], config: {...} }".into()))?;
        let id = spec.split_once('@').map_or(spec, |(i, _)| i);
        if let Some(i) = g.expanded.plugins.iter().position(|p| p.name() == id) {
            return Err(LiveError::Refused(match &g.origins[i] {
                Some(c) => format!("`{id}` is already running as part of `{c}`"),
                None => format!("`{id}` is already enabled"),
            }));
        }
        let mut source = g.source.clone();
        let list = plugins_mut(&mut source);
        let mut e = Map::new();
        e.insert("use".into(), json!(spec));
        if let Some(c) = entry.get("config") {
            e.insert("config".into(), c.clone());
        }
        list.push(Value::Object(e));
        self.apply_locked(source, actor, "plugin.enable").await
    }

    /// Removes a plugin listed directly in app.yaml and applies.
    pub async fn disable(&self, id: &str, actor: &str) -> Result<Change, LiveError> {
        let _l = self.0.apply_lock.lock().await;
        let g = self.current();
        let i = g
            .expanded
            .plugins
            .iter()
            .position(|p| p.name() == id)
            .ok_or_else(|| LiveError::NotFound(format!("`{id}` is not running")))?;
        if let Some(c) = &g.origins[i] {
            return Err(LiveError::Refused(format!(
                "`{id}` is part of `{c}`; disable `{c}` or change its config"
            )));
        }
        let mut source = g.source.clone();
        plugins_mut(&mut source).retain(|p| entry_name(p) != Some(id));
        self.apply_locked(source, actor, "plugin.disable").await
    }

    /// Replaces a plugin's config (`replace`) or merges a patch into it.
    /// Keys the plugin marks `runtime_config` change in place through
    /// `configure` when it supports that; anything else starts a new
    /// generation.
    pub async fn configure(
        &self,
        id: &str,
        patch: Value,
        replace: bool,
        actor: &str,
    ) -> Result<Change, LiveError> {
        let _l = self.0.apply_lock.lock().await;
        let g = self.current();
        let Some(patch_obj) = patch.as_object() else {
            return Err(LiveError::Invalid("the config must be a JSON object".into()));
        };
        let i = g
            .expanded
            .plugins
            .iter()
            .position(|p| p.name() == id)
            .ok_or_else(|| LiveError::NotFound(format!("`{id}` is not running")))?;
        let plugin = self
            .0
            .registry
            .get(id)
            .ok_or_else(|| LiveError::NotFound(format!("`{id}` is not in this build")))?
            .clone();
        let mut merged = g.expanded.plugins[i].config.clone();
        if replace || !merged.is_object() {
            merged = patch.clone();
        } else {
            merge(&mut merged, &patch);
        }
        let issues = plugin.manifest().validate_config(&merged);
        if !issues.is_empty() {
            let at = format!("plugins.{id}.config");
            return Err(LiveError::Plugins(PluginLoadError(
                issues
                    .into_iter()
                    .map(|x| {
                        Issue::new(
                            if x.path == "(root)" {
                                at.clone()
                            } else {
                                format!("{at}.{}", x.path)
                            },
                            x.message,
                        )
                    })
                    .collect(),
            )));
        }

        // The same change in the app.yaml document.
        let mut source = g.source.clone();
        let origin = g.origins[i].clone();
        {
            let list = plugins_mut(&mut source);
            let owner = origin.as_deref().unwrap_or(id);
            let entry = list
                .iter_mut()
                .find(|p| entry_name(p) == Some(owner))
                .ok_or_else(|| LiveError::NotFound(format!("`{owner}` is not in app.yaml")))?;
            let obj = entry.as_object_mut().expect("plugin entries are objects");
            let cfg = obj.entry("config").or_insert_with(|| json!({}));
            let slot = match &origin {
                None => cfg,
                Some(_) => {
                    if !cfg.is_object() {
                        *cfg = json!({});
                    }
                    let with = cfg
                        .as_object_mut()
                        .expect("object")
                        .entry("with")
                        .or_insert_with(|| json!({}));
                    if !with.is_object() {
                        *with = json!({});
                    }
                    with.as_object_mut()
                        .expect("object")
                        .entry(id.to_string())
                        .or_insert_with(|| json!({}))
                }
            };
            if replace || !slot.is_object() {
                *slot = patch.clone();
            } else {
                merge(slot, &patch);
            }
        }

        let runtime = &plugin.manifest().runtime_config;
        let in_place = !replace && !patch_obj.is_empty() && patch_obj.keys().all(|k| runtime.contains(k));
        if in_place {
            match plugin.configure(patch.clone()).await {
                Ok(()) => {
                    let mut ng = g.clone();
                    ng.version += 1;
                    ng.source = source;
                    ng.expanded.plugins[i].config = merged;
                    let change = Change {
                        mode: "live".into(),
                        version: ng.version,
                        generation: ng.number,
                        changed: vec![id.to_string()],
                        ..Default::default()
                    };
                    *self.0.current.write().expect("generation lock") = ng;
                    tracing::info!(
                        plugin = id,
                        version = change.version,
                        "runtime config changed in place"
                    );
                    self.audit(
                        actor,
                        "plugin.configure",
                        id,
                        json!({ "mode": "live", "patch": redact(&patch), "version": change.version }),
                    )
                    .await;
                    return Ok(change);
                }
                Err(PluginError::ConfigureUnsupported) => {}
                Err(e) => return Err(LiveError::Invalid(format!("{id}: {e}"))),
            }
        }
        self.apply_locked(source, actor, "plugin.configure").await
    }

    async fn audit(&self, actor: &str, action: &str, target: &str, detail: Value) {
        if let Some(sql) = &self.0.providers.sql {
            if let Err(e) = crate::migrate::audit(sql.as_ref(), actor, action, target, detail).await {
                tracing::warn!(error = %e, "could not write the audit log");
            }
        }
    }

    /// Recent audit rows, newest first.
    pub async fn audit_log(&self, limit: u32) -> Result<Vec<Value>, ApiError> {
        let Some(sql) = &self.0.providers.sql else {
            return Ok(vec![]);
        };
        let rows = sql
            .query(
                &format!(
                    "SELECT id, at, actor, action, target, detail FROM keel_audit ORDER BY at DESC, id DESC LIMIT {}",
                    limit.clamp(1, 500)
                ),
                &[],
            )
            .await?;
        Ok(rows
            .into_iter()
            .map(|mut r| {
                if let Some(Value::String(d)) = r.get("detail") {
                    if let Ok(v) = serde_json::from_str::<Value>(d) {
                        r.insert("detail".into(), v);
                    }
                }
                Value::Object(r)
            })
            .collect())
    }

    // ---- serving --------------------------------------------------------

    /// The public router: `/_admin/*` plus whatever the current generation serves.
    pub fn router(&self) -> Router {
        Router::new()
            .nest("/_admin", admin_router(self.clone()))
            .fallback(dispatch)
            .with_state(self.clone())
    }

    /// A router for a supervisor on a private loopback port: `/health` and
    /// `/stats`, no token.
    pub fn control_router(&self) -> Router {
        Router::new()
            .route(
                "/stats",
                get(|State(l): State<Live>| async move { Json(l.stats()) }),
            )
            .fallback(dispatch)
            .with_state(self.clone())
    }

    /// Serves on `listener` until `shutdown` completes. Then it stops
    /// accepting (a process sharing the socket keeps accepting), asks
    /// keep-alive clients to reconnect with `Connection: close`, lets in-flight
    /// requests finish, and drains every plugin.
    pub async fn serve_on(
        &self,
        listener: TcpListener,
        shutdown: impl Future<Output = ()> + Send + 'static,
    ) -> std::io::Result<()> {
        let local = listener.local_addr()?;
        let g = self.current();
        tracing::info!(app = %g.expanded.app, %local, plugins = g.host.plugins().len(), "keel host listening");
        let report = g.host.setup_report();
        if !report.complete {
            tracing::warn!(
                steps = %report.incomplete_ids().join(", "),
                "setup incomplete; blocked routes answer 503 until done (run `keel setup status`)"
            );
        }
        let refresher = {
            let live = self.clone();
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_secs(10)).await;
                    let _ = live.host().refresh_setup().await;
                }
            })
        };
        g.host
            .hooks()
            .emit(Event::new(
                "host.started",
                json!({ "app": g.expanded.app, "addr": local.to_string() }),
            ))
            .await;
        let (drain_tx, drain_rx) = tokio::sync::watch::channel(false);
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
        {
            let live = self.clone();
            tokio::spawn(async move {
                shutdown.await;
                live.0.draining.store(true, Ordering::Release);
                let _ = drain_tx.send(true);
                tracing::info!("draining: no new connections; finishing in-flight requests");
                // Give keep-alive clients a request to see `Connection: close`.
                tokio::time::sleep(Duration::from_millis(1000)).await;
                let start = Instant::now();
                while live.current().inflight.load(Ordering::Acquire) > 0 && start.elapsed() < DRAIN_TIMEOUT {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                let _ = stop_tx.send(());
            });
        }
        let listener = axum::serve::ListenerExt::tap_io(
            DrainListener {
                inner: listener,
                draining: drain_rx,
            },
            |_| {},
        );
        let served = axum::serve(
            listener,
            self.router().into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async {
            let _ = stop_rx.await;
        })
        .await;
        refresher.abort();
        served?;
        let g = self.current();
        for p in g.host.plugins() {
            p.drain().await;
        }
        g.host
            .hooks()
            .emit(Event::new("host.stopped", json!({ "app": g.expanded.app })))
            .await;
        Ok(())
    }
}

async fn build(
    source: &Value,
    registry: &PluginRegistry,
    providers: &CapabilityProviders,
    options: &HostOptions,
) -> Result<(AppConfig, Vec<Option<String>>, Host), LiveError> {
    let config = AppConfig::from_value(source.clone(), "app.yaml", options.secrets.as_ref())?;
    let expanded = compose::expand(&config, registry)?;
    let host = Host::start_with(expanded.config.clone(), registry, providers, options.clone()).await?;
    Ok((expanded.config, expanded.origins, host))
}

fn diff(old: &AppConfig, new: &AppConfig) -> Change {
    let find = |c: &AppConfig, id: &str| c.plugins.iter().find(|p| p.name() == id).cloned();
    let mut ch = Change::default();
    for p in &new.plugins {
        match find(old, p.name()) {
            None => ch.added.push(p.name().into()),
            Some(o) if o != *p => ch.changed.push(p.name().into()),
            _ => {}
        }
    }
    for p in &old.plugins {
        if find(new, p.name()).is_none() {
            ch.removed.push(p.name().into());
        }
    }
    if old.routes != new.routes {
        ch.changed.push("routes".into());
    }
    ch
}

fn plugins_mut(source: &mut Value) -> &mut Vec<Value> {
    if !source.is_object() {
        *source = json!({});
    }
    let obj = source.as_object_mut().expect("object");
    let list = obj.entry("plugins").or_insert_with(|| json!([]));
    if !list.is_array() {
        *list = json!([]);
    }
    list.as_array_mut().expect("array")
}

fn entry_name(entry: &Value) -> Option<&str> {
    let spec = entry.get("use")?.as_str()?;
    Some(spec.split_once('@').map_or(spec, |(n, _)| n))
}

fn merge(base: &mut Value, extra: &Value) {
    match (base, extra) {
        (Value::Object(b), Value::Object(e)) => {
            for (k, v) in e {
                match b.get_mut(k) {
                    Some(slot) if v.is_object() => merge(slot, v),
                    _ => {
                        b.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (b, e) => *b = e.clone(),
    }
}

/// Hides values under keys that look secret, for the audit log.
fn redact(v: &Value) -> Value {
    match v {
        Value::Object(o) => Value::Object(
            o.iter()
                .map(|(k, v)| {
                    let lower = k.to_ascii_lowercase();
                    let secret = ["secret", "password", "token", "key"]
                        .iter()
                        .any(|s| lower.contains(s));
                    (k.clone(), if secret { json!("***") } else { redact(v) })
                })
                .collect(),
        ),
        other => other.clone(),
    }
}

// ---- request path ------------------------------------------------------

/// Stops accepting once draining starts, without closing open connections.
struct DrainListener {
    inner: TcpListener,
    draining: tokio::sync::watch::Receiver<bool>,
}

impl axum::serve::Listener for DrainListener {
    type Io = tokio::net::TcpStream;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            if *self.draining.borrow() {
                std::future::pending::<()>().await;
            }
            tokio::select! {
                r = self.inner.accept() => match r {
                    Ok(c) => return c,
                    Err(e) => {
                        tracing::warn!(error = %e, "accept failed");
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                },
                _ = self.draining.changed() => {}
            }
        }
    }

    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        self.inner.local_addr()
    }
}

struct InFlight(Arc<AtomicUsize>);

impl InFlight {
    fn new(c: &Arc<AtomicUsize>) -> Self {
        c.fetch_add(1, Ordering::AcqRel);
        Self(c.clone())
    }
}

impl Drop for InFlight {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

async fn dispatch(State(live): State<Live>, req: Request) -> Response {
    if req.uri().path() == "/health" && live.drilling() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "status": "down", "reason": "rollback drill: --drill-unhealthy-after passed" })),
        )
            .into_response();
    }
    let g = live.current();
    let _guard = InFlight::new(&g.inflight);
    live.0.requests.fetch_add(1, Ordering::Relaxed);
    let res = match g.router.oneshot(req).await {
        Ok(r) => r,
        Err(never) => match never {},
    };
    if res.status().is_server_error() {
        live.0.errors.fetch_add(1, Ordering::Relaxed);
    }
    let mut res = res;
    if live.0.draining.load(Ordering::Acquire) {
        res.headers_mut().insert(
            axum::http::header::CONNECTION,
            axum::http::HeaderValue::from_static("close"),
        );
    }
    res
}

// ---- admin API -----------------------------------------------------------

fn admin_router(live: Live) -> Router<Live> {
    Router::new()
        .route("/config", get(get_config).put(put_config))
        .route("/plugins", get(list_plugins).post(enable_plugin))
        .route(
            "/plugins/{id}",
            axum::routing::put(put_plugin).delete(delete_plugin),
        )
        .route("/plugins/{id}/config", patch(patch_plugin))
        .route("/setup", get(setup))
        .route("/audit", get(audit))
        .route(
            "/stats",
            get(|State(l): State<Live>| async move { Json(l.stats()) }),
        )
        .layer(middleware::from_fn_with_state(live, admin_auth))
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

async fn admin_auth(State(live): State<Live>, req: Request, next: Next) -> Response {
    let Some(token) = live.0.options.secrets.get(ADMIN_TOKEN).filter(|t| !t.is_empty()) else {
        return ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "admin_disabled",
            format!("the admin API is off; set the {ADMIN_TOKEN} secret (env or .env) and restart"),
        )
        .into_response();
    };
    let sent = keel_sdk::auth::bearer(req.headers()).unwrap_or_default();
    if !same(sent.as_bytes(), token.as_bytes()) {
        return ApiError::unauthorized(
            "admin_token",
            format!("send `Authorization: Bearer <{ADMIN_TOKEN}>`"),
        )
        .into_response();
    }
    next.run(req).await
}

fn actor(h: &HeaderMap) -> String {
    h.get("x-keel-actor")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.chars().take(100).collect())
        .unwrap_or_else(|| "admin-api".into())
}

type Reply = Result<Json<Value>, ApiError>;

fn done(r: Result<Change, LiveError>) -> Reply {
    r.map(|c| Json(serde_json::to_value(c).unwrap_or_default()))
        .map_err(|e| e.api())
}

fn json_body(b: &Bytes) -> Result<Value, ApiError> {
    serde_json::from_slice(b)
        .map_err(|e| ApiError::bad_request("invalid_body", format!("the JSON body is not valid: {e}")))
}

async fn get_config(State(live): State<Live>) -> Reply {
    let g = live.current();
    Ok(Json(json!({
        "version": g.version,
        "generation": g.number,
        "pulled_at": now_secs(),
        "yaml": serde_yaml::to_string(&g.source).unwrap_or_default(),
    })))
}

async fn put_config(State(live): State<Live>, h: HeaderMap, body: Bytes) -> Reply {
    let text = std::str::from_utf8(&body)
        .map_err(|_| ApiError::bad_request("invalid_body", "send app.yaml as UTF-8 text"))?;
    done(live.apply_yaml(text, &actor(&h)).await)
}

async fn list_plugins(State(live): State<Live>) -> Reply {
    let g = live.current();
    let list: Vec<Value> = g
        .expanded
        .plugins
        .iter()
        .zip(&g.origins)
        .map(|(p, o)| {
            let m = live.0.registry.get(p.name()).map(|x| x.manifest().clone());
            json!({
                "id": p.name(),
                "use": p.spec,
                "version": m.as_ref().map(|m| m.version.to_string()),
                "from": o,
                "runtime_config": m.as_ref().map(|m| m.runtime_config.clone()).unwrap_or_default(),
            })
        })
        .collect();
    Ok(Json(
        json!({ "version": g.version, "generation": g.number, "plugins": list }),
    ))
}

async fn enable_plugin(State(live): State<Live>, h: HeaderMap, body: Bytes) -> Reply {
    let entry = json_body(&body)?;
    done(live.enable(entry, &actor(&h)).await)
}

#[derive(Deserialize)]
struct PutBody {
    config: Value,
}

async fn put_plugin(State(live): State<Live>, Path(id): Path<String>, h: HeaderMap, body: Bytes) -> Reply {
    let b: PutBody = serde_json::from_slice(&body)
        .map_err(|e| ApiError::bad_request("invalid_body", format!("send {{ \"config\": {{...}} }}: {e}")))?;
    done(live.configure(&id, b.config, true, &actor(&h)).await)
}

async fn delete_plugin(State(live): State<Live>, Path(id): Path<String>, h: HeaderMap) -> Reply {
    done(live.disable(&id, &actor(&h)).await)
}

async fn patch_plugin(State(live): State<Live>, Path(id): Path<String>, h: HeaderMap, body: Bytes) -> Reply {
    let patch = json_body(&body)?;
    done(live.configure(&id, patch, false, &actor(&h)).await)
}

async fn setup(State(live): State<Live>) -> Reply {
    let r = live.host().refresh_setup().await.map_err(ApiError::internal)?;
    Ok(Json(serde_json::to_value(r).unwrap_or_default()))
}

#[derive(Deserialize)]
struct AuditQuery {
    limit: Option<u32>,
}

async fn audit(State(live): State<Live>, q: Query<AuditQuery>) -> Reply {
    let rows = live.audit_log(q.limit.unwrap_or(50)).await?;
    Ok(Json(json!({ "entries": rows })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_and_merges() {
        assert_eq!(
            redact(&json!({ "message": "hi", "api_key": "x", "nested": { "password": 1 } })),
            json!({ "message": "hi", "api_key": "***", "nested": { "password": "***" } })
        );
        let mut b = json!({ "a": { "b": 1 }, "c": [1] });
        merge(&mut b, &json!({ "a": { "d": 2 }, "c": [2] }));
        assert_eq!(b, json!({ "a": { "b": 1, "d": 2 }, "c": [2] }));
        assert!(same(b"abc", b"abc") && !same(b"abc", b"abd") && !same(b"ab", b"abc"));
    }
}
