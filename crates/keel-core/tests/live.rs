//! Live changes under load: enable, reconfigure, disable and apply while
//! clients keep sending requests, with no failed request.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use keel_core::live::Live;
use keel_core::{HostOptions, MapSecrets, PluginRegistry};
use keel_providers::SqlProvider;
use keel_sdk::capabilities::CapabilityProviders;
use keel_sdk::prelude::*;
use serde_json::json;

/// A second plugin to switch on and off.
struct Echo;

#[async_trait]
impl Plugin for Echo {
    fn manifest(&self) -> &Manifest {
        static M: OnceLock<Manifest> = OnceLock::new();
        M.get_or_init(|| {
            Manifest::from_sources(
                "id: echo\nversion: 0.1.0\nsummary: Echo for live tests.\nprovides:\n  routes: [\"GET /echo\"]\nconfig: config.schema.json\n",
                Some(r#"{"type":"object","additionalProperties":false,"properties":{"word":{"type":"string","default":"hey"}}}"#),
            )
            .unwrap()
        })
    }

    async fn init(&self, ctx: &mut InitContext) -> Result<(), PluginError> {
        let word = ctx
            .config
            .get("word")
            .and_then(Value::as_str)
            .unwrap_or("hey")
            .to_string();
        ctx.routes().get("/echo", move || {
            let w = word.clone();
            async move { Json(json!({ "word": w })) }
        })?;
        Ok(())
    }
}

const APP: &str = "app: live\nplugins:\n  - use: ping\n    config: { message: one }\n";
const TOKEN: &str = "admin-test-token";

async fn start() -> (Live, String) {
    let registry = PluginRegistry::new()
        .register(keel_plugin_ping::Ping::new())
        .register(Echo);
    let providers = CapabilityProviders {
        sql: Some(Arc::new(
            SqlProvider::connect("sqlite::memory:", Some(1)).await.unwrap(),
        )),
        ..Default::default()
    };
    let secrets = HashMap::from([(keel_core::live::ADMIN_TOKEN.to_string(), TOKEN.to_string())]);
    let options = HostOptions {
        secrets: Arc::new(MapSecrets(secrets)),
        state_file: None,
    };
    let live = Live::start_yaml(APP, registry, providers, options).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let l = live.clone();
    tokio::spawn(async move { l.serve_on(listener, std::future::pending()).await.unwrap() });
    (live, base)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn changes_under_load_drop_nothing() {
    let (live, base) = start().await;
    let http = reqwest::Client::new();
    let admin = |m: reqwest::Method, path: &str| {
        http.request(m, format!("{base}/_admin{path}"))
            .bearer_auth(TOKEN)
            .header("x-keel-actor", "test")
    };

    // Load: 8 clients hammering /ping until told to stop.
    let stop = Arc::new(AtomicBool::new(false));
    let (ok, failed) = (Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)));
    let mut load = Vec::new();
    for _ in 0..8 {
        let (http, base, stop, ok, failed) = (
            http.clone(),
            base.clone(),
            stop.clone(),
            ok.clone(),
            failed.clone(),
        );
        load.push(tokio::spawn(async move {
            while !stop.load(Ordering::Relaxed) {
                match http.get(format!("{base}/ping")).send().await {
                    Ok(r) if r.status().is_success() => ok.fetch_add(1, Ordering::Relaxed),
                    _ => failed.fetch_add(1, Ordering::Relaxed),
                };
            }
        }));
    }
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Runtime key: changed in place.
    let r: Value = admin(reqwest::Method::PATCH, "/plugins/ping/config")
        .json(&json!({ "message": "two" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(r["mode"], "live", "{r}");
    let msg: Value = http
        .get(format!("{base}/ping"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(msg["message"], "two");

    // Enable a plugin: a new generation takes over.
    let r = admin(reqwest::Method::POST, "/plugins")
        .json(&json!({ "use": "echo", "config": { "word": "hello" } }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let r: Value = r.json().await.unwrap();
    assert_eq!(
        (r["mode"].as_str(), r["added"].clone()),
        (Some("reload"), json!(["echo"])),
        "{r}"
    );
    let e: Value = http
        .get(format!("{base}/echo"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(e["word"], "hello");
    let msg: Value = http
        .get(format!("{base}/ping"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(msg["message"], "two", "the runtime change survives a reload");

    // A non-runtime key reloads.
    let r: Value = admin(reqwest::Method::PATCH, "/plugins/echo/config")
        .json(&json!({ "word": "bye" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        (r["mode"].as_str(), r["changed"].clone()),
        (Some("reload"), json!(["echo"])),
        "{r}"
    );
    let e: Value = http
        .get(format!("{base}/echo"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(e["word"], "bye");

    // Bad changes are refused and the server keeps running as it was.
    let r = admin(reqwest::Method::PATCH, "/plugins/echo/config")
        .json(&json!({ "nope": 1 }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 422);
    let r = admin(reqwest::Method::POST, "/plugins")
        .json(&json!({ "use": "missing" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 422, "{}", r.text().await.unwrap());
    let r = admin(reqwest::Method::PUT, "/config")
        .body("app: live\nproviders: { cache: { use: memory } }\nplugins: []\n")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 409, "providers need a swap");

    // Disable it again.
    let r: Value = admin(reqwest::Method::DELETE, "/plugins/echo")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(r["removed"], json!(["echo"]), "{r}");
    assert_eq!(
        http.get(format!("{base}/echo")).send().await.unwrap().status(),
        404
    );

    // Whole-file apply, then pull it back.
    let r: Value = admin(reqwest::Method::PUT, "/config")
        .body("app: live\nplugins:\n  - use: ping\n    config: { message: three }\nroutes:\n  /ping: { validate: { query: { type: object } } }\n")
        .send().await.unwrap().json().await.unwrap();
    assert_eq!(r["mode"], "reload", "{r}");
    let pulled: Value = admin(reqwest::Method::GET, "/config")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        pulled["yaml"].as_str().unwrap().contains("message: three"),
        "{pulled}"
    );

    tokio::time::sleep(Duration::from_millis(200)).await;
    stop.store(true, Ordering::Relaxed);
    for l in load {
        l.await.unwrap();
    }
    let (ok, failed) = (ok.load(Ordering::Relaxed), failed.load(Ordering::Relaxed));
    assert!(ok > 100, "load ran: {ok}");
    assert_eq!(failed, 0, "{failed} of {} requests failed", ok + failed);

    // Audit log has every change.
    let a: Value = admin(reqwest::Method::GET, "/audit")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let actions: Vec<&str> = a["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["action"].as_str().unwrap())
        .collect();
    for want in [
        "plugin.configure",
        "plugin.enable",
        "plugin.disable",
        "config.apply",
    ] {
        assert!(actions.contains(&want), "{actions:?}");
    }
    assert_eq!(live.version(), 6);
}

#[tokio::test]
async fn admin_api_needs_the_token() {
    let (_live, base) = start().await;
    let http = reqwest::Client::new();
    let r = http.get(format!("{base}/_admin/plugins")).send().await.unwrap();
    assert_eq!(r.status(), 401);
    let r = http
        .get(format!("{base}/_admin/plugins"))
        .bearer_auth("wrong")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
    let r: Value = http
        .get(format!("{base}/_admin/plugins"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(r["plugins"][0]["runtime_config"], json!(["message"]));
}

#[tokio::test]
async fn version_numbers_survive_a_restart() {
    let db = std::env::temp_dir().join(format!("keel-live-version-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&db);
    let url = format!("sqlite://{}?mode=rwc", db.display());
    let boot = || async {
        let registry = PluginRegistry::new().register(keel_plugin_ping::Ping::new());
        let providers = CapabilityProviders {
            sql: Some(Arc::new(SqlProvider::connect(&url, Some(1)).await.unwrap())),
            ..Default::default()
        };
        let options = HostOptions {
            secrets: Arc::new(MapSecrets(HashMap::new())),
            state_file: None,
        };
        Live::start_yaml(APP, registry, providers, options).await.unwrap()
    };
    let first = boot().await;
    assert_eq!(first.version(), 1);
    let c = first
        .apply_yaml(&APP.replace("one", "two"), "test")
        .await
        .unwrap();
    assert_eq!(c.version, 2);
    drop(first);
    let second = boot().await;
    assert_eq!(second.version(), 3, "numbering carries on after a restart");
    let _ = std::fs::remove_file(&db);
}
