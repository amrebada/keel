use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use keel_core::{AppConfig, Host, MapSecrets, PluginRegistry};
use keel_plugin_ping::Ping;
use keel_sdk::capabilities::CapabilityProviders;
use keel_sdk::prelude::*;
use tower::ServiceExt;

/// A plugin built from inline manifest text, for testing the loader.
struct TestPlugin {
    manifest: Manifest,
    register: Vec<&'static str>,
    log: Arc<Mutex<Vec<String>>>,
}

impl TestPlugin {
    fn new(
        yaml: &str,
        schema: Option<&str>,
        register: Vec<&'static str>,
        log: Arc<Mutex<Vec<String>>>,
    ) -> Self {
        Self {
            manifest: Manifest::from_sources(yaml, schema).unwrap(),
            register,
            log,
        }
    }
}

#[async_trait]
impl Plugin for TestPlugin {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    async fn init(&self, ctx: &mut InitContext) -> Result<(), PluginError> {
        self.log
            .lock()
            .unwrap()
            .push(format!("init:{}", self.manifest.id));
        for path in &self.register {
            ctx.routes().get(path, || async { "ok" })?;
        }
        Ok(())
    }

    async fn on_event(&self, event: &Event) {
        self.log
            .lock()
            .unwrap()
            .push(format!("{}<-{}", self.manifest.id, event.name));
    }
}

fn log() -> Arc<Mutex<Vec<String>>> {
    Arc::new(Mutex::new(Vec::new()))
}

fn config(yaml: &str) -> AppConfig {
    AppConfig::load_str(yaml, "app.yaml", &MapSecrets::default()).unwrap()
}

async fn start(yaml: &str, registry: &PluginRegistry) -> Result<Host, String> {
    Host::start(config(yaml), registry, &CapabilityProviders::default())
        .await
        .map_err(|e| e.to_string())
}

async fn call(host: &Host, path: &str) -> (StatusCode, String) {
    let res = host
        .router()
        .oneshot(Request::get(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&body).to_string())
}

#[tokio::test]
async fn starts_a_compiled_in_plugin_and_serves_its_route() {
    let registry = PluginRegistry::new().register(Ping::new());
    let host = start(
        "app: t\nplugins:\n  - use: ping@^0.1\n    config: { message: hello }\n",
        &registry,
    )
    .await
    .unwrap();

    let (status, body) = call(&host, "/ping").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, r#"{"message":"hello"}"#);

    let (_, health) = call(&host, "/health").await;
    let health: Value = serde_json::from_str(&health).unwrap();
    assert_eq!(
        health["plugins"][0],
        json!({ "id": "ping", "version": "0.1.0", "status": "ok", "degrade_level": "error" })
    );
}

#[tokio::test]
async fn plugins_not_listed_in_config_are_not_started() {
    let registry = PluginRegistry::new().register(Ping::new());
    let host = start("app: t\n", &registry).await.unwrap();
    assert_eq!(call(&host, "/ping").await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn reports_every_problem_with_its_path() {
    let l = log();
    let needs_sql = TestPlugin::new(
        "id: db-thing\nversion: 1.0.0\nsummary: x\nrequires: { capabilities: [sql] }\n",
        None,
        vec![],
        l.clone(),
    );
    let registry = PluginRegistry::new().register(Ping::new()).register(needs_sql);
    let err = start(
        "app: t\nplugins:\n  - use: nope\n  - use: ping@^2\n  - use: db-thing\n  - use: ping\n    config: { message: 5 }\n",
        &registry,
    )
    .await
    .err()
    .unwrap();
    assert!(
        err.contains(
            "plugins[0].use: plugin `nope` is not in this build. Compiled-in plugins: db-thing, ping"
        ),
        "{err}"
    );
    assert!(
        err.contains("plugins[1].use: `ping@^2` asked for, but this build has version 0.1.0"),
        "{err}"
    );
    assert!(
        err.contains("plugins[2].use: `db-thing` needs the `sql` capability"),
        "{err}"
    );
    assert!(
        err.contains("plugins[3].use: `ping` is already listed at plugins[1]"),
        "{err}"
    );
    assert!(l.lock().unwrap().is_empty(), "nothing starts when checks fail");
}

#[tokio::test]
async fn rejects_config_that_breaks_the_plugin_schema() {
    let registry = PluginRegistry::new().register(Ping::new());
    let err = start(
        "app: t\nplugins:\n  - use: ping\n    config: { message: 5, extra: 1 }\n",
        &registry,
    )
    .await
    .err()
    .unwrap();
    assert!(err.contains("plugins[0].config.message"), "{err}");
    assert!(
        err.contains("plugins[0].config: Additional properties are not allowed ('extra' was unexpected)"),
        "{err}"
    );
}

#[tokio::test]
async fn starts_dependencies_first_and_checks_their_versions() {
    let l = log();
    let core = TestPlugin::new("id: core\nversion: 1.2.0\nsummary: x\n", None, vec![], l.clone());
    let addon = TestPlugin::new(
        "id: addon\nversion: 1.0.0\nsummary: x\nrequires: { plugins: { core: \"^1\" } }\n",
        None,
        vec![],
        l.clone(),
    );
    let registry = PluginRegistry::new().register(core).register(addon);
    start("app: t\nplugins:\n  - use: addon\n  - use: core\n", &registry)
        .await
        .unwrap();
    assert_eq!(*l.lock().unwrap(), vec!["init:core", "init:addon"]);

    let err = start("app: t\nplugins:\n  - use: addon\n", &registry)
        .await
        .err()
        .unwrap();
    assert!(
        err.contains("`addon` needs plugin `core` (^1); add it to plugins"),
        "{err}"
    );
}

#[tokio::test]
async fn two_plugins_cannot_claim_the_same_route() {
    let l = log();
    let a = TestPlugin::new(
        "id: a\nversion: 1.0.0\nsummary: x\nprovides: { routes: [GET /x] }\n",
        None,
        vec!["/x"],
        l.clone(),
    );
    let b = TestPlugin::new(
        "id: b\nversion: 1.0.0\nsummary: x\nprovides: { routes: [GET /x] }\n",
        None,
        vec!["/x"],
        l.clone(),
    );
    let registry = PluginRegistry::new().register(a).register(b);
    let err = start("app: t\nplugins:\n  - use: a\n  - use: b\n", &registry)
        .await
        .err()
        .unwrap();
    assert!(err.contains("route `GET /x` is also provided by `a`"), "{err}");
}

#[tokio::test]
async fn routes_must_match_the_manifest() {
    let l = log();
    let sneaky = TestPlugin::new(
        "id: sneaky\nversion: 1.0.0\nsummary: x\n",
        None,
        vec!["/secret"],
        l.clone(),
    );
    let registry = PluginRegistry::new().register(sneaky);
    let err = start("app: t\nplugins:\n  - use: sneaky\n", &registry)
        .await
        .err()
        .unwrap();
    assert!(
        err.contains("route `GET /secret` is not declared in provides.routes"),
        "{err}"
    );

    let lazy = TestPlugin::new(
        "id: lazy\nversion: 1.0.0\nsummary: x\nprovides: { routes: [GET /a] }\n",
        None,
        vec![],
        l,
    );
    let registry = PluginRegistry::new().register(lazy);
    let err = start("app: t\nplugins:\n  - use: lazy\n", &registry)
        .await
        .err()
        .unwrap();
    assert!(
        err.contains("declares routes it did not register: GET /a"),
        "{err}"
    );
}

#[tokio::test]
async fn delivers_consumed_events_to_plugins() {
    let l = log();
    let listener = TestPlugin::new(
        "id: listener\nversion: 1.0.0\nsummary: x\nconsumes: { events: [ping.received] }\n",
        None,
        vec![],
        l.clone(),
    );
    let registry = PluginRegistry::new().register(Ping::new()).register(listener);
    let host = start("app: t\nplugins:\n  - use: ping\n  - use: listener\n", &registry)
        .await
        .unwrap();
    call(&host, "/ping").await;
    assert!(
        l.lock().unwrap().contains(&"listener<-ping.received".to_string()),
        "{:?}",
        l.lock().unwrap()
    );
}

#[tokio::test]
async fn ping_supports_live_config_changes() {
    let registry = PluginRegistry::new().register(Ping::new());
    let host = start("app: t\nplugins:\n  - use: ping\n", &registry)
        .await
        .unwrap();
    assert_eq!(call(&host, "/ping").await.1, r#"{"message":"pong"}"#);
    host.plugins()[0]
        .configure(json!({ "message": "changed" }))
        .await
        .unwrap();
    assert_eq!(call(&host, "/ping").await.1, r#"{"message":"changed"}"#);
}
