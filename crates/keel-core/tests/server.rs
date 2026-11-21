use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::get;
use http_body_util::BodyExt;
use keel_core::{AppConfig, Event, Host, MapSecrets};
use serde_json::Value;
use tower::ServiceExt;

fn config() -> AppConfig {
    AppConfig::load_str(
        "app: team-tasks\nproviders:\n  cache: { use: memory }\nplugins:\n  - use: auth-core@^1\n",
        "app.yaml",
        &MapSecrets::default(),
    )
    .unwrap()
}

async fn get_json(router: axum::Router, path: &str) -> (StatusCode, Value) {
    let res = router
        .oneshot(Request::get(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

#[tokio::test]
async fn health_reports_app_and_configured_parts() {
    let (status, body) = get_json(Host::new(config()).router(), "/health").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "ok");
    assert_eq!(body["app"], "team-tasks");
    assert_eq!(body["plugins"], serde_json::json!([]));
    assert_eq!(
        body["providers"],
        serde_json::json!([]),
        "Host::new connects no providers"
    );
}

#[tokio::test]
async fn added_routes_are_served() {
    let host = Host::new(config()).route(
        "/hello",
        get(|| async { axum::Json(serde_json::json!({"hi": true})) }),
    );
    let (status, body) = get_json(host.router(), "/hello").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["hi"], true);
}

#[tokio::test]
async fn unknown_route_is_404() {
    let (status, _) = get_json(Host::new(config()).router(), "/nope").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn serves_over_tcp_and_emits_lifecycle_events() {
    let host = Host::new(config());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let s = seen.clone();
    host.hooks().subscribe("*", move |e: Event| {
        let s = s.clone();
        async move { s.lock().unwrap().push(e.name) }
    });

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn(host.serve_on(listener, async {
        let _ = rx.await;
    }));

    // Plain HTTP/1.1 request over a raw socket, to avoid an HTTP client dependency.
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut sock = tokio::net::TcpStream::connect(addr).await.unwrap();
    sock.write_all(b"GET /health HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut buf = String::new();
    sock.read_to_string(&mut buf).await.unwrap();
    assert!(buf.starts_with("HTTP/1.1 200"), "{buf}");
    assert!(buf.contains("\"app\":\"team-tasks\""), "{buf}");

    tx.send(()).unwrap();
    task.await.unwrap().unwrap();
    assert_eq!(*seen.lock().unwrap(), vec!["host.started", "host.stopped"]);
}

struct DownKv;

#[async_trait::async_trait]
impl keel_sdk::capabilities::Kv for DownKv {
    async fn get(&self, _: &str) -> keel_sdk::capabilities::CapResult<Option<Vec<u8>>> {
        Err(keel_sdk::capabilities::CapabilityError::Failed(
            "connection refused".into(),
        ))
    }
    async fn set(&self, _: &str, _: Vec<u8>) -> keel_sdk::capabilities::CapResult<()> {
        unreachable!()
    }
    async fn delete(&self, _: &str) -> keel_sdk::capabilities::CapResult<bool> {
        unreachable!()
    }
}

async fn health_with_down_kv(level: &str) -> (StatusCode, Value) {
    use keel_sdk::capabilities::{Capability, CapabilityProviders, DegradeLevel, ProviderInfo};
    let config = AppConfig::load_str(
        "app: team-tasks\nproviders:\n  kv: { use: redis, url: redis://x }\n",
        "app.yaml",
        &MapSecrets::default(),
    )
    .unwrap();
    let providers = CapabilityProviders {
        kv: Some(Arc::new(DownKv)),
        info: vec![ProviderInfo {
            slot: "kv".into(),
            kind: "redis".into(),
            capability: Capability::Kv,
            hot_load: true,
            degrade_level: DegradeLevel::parse(level).unwrap(),
        }],
        ..Default::default()
    };
    let host = Host::start(config, &keel_core::PluginRegistry::new(), &providers)
        .await
        .unwrap();
    get_json(host.router(), "/health").await
}

#[tokio::test]
async fn health_is_503_only_when_an_error_level_item_is_down() {
    let (status, body) = health_with_down_kv("error").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["status"], "down");
    assert_eq!(body["providers"][0]["status"], "down");
    assert_eq!(body["providers"][0]["reason"], "connection refused");

    for level in ["warn", "info"] {
        let (status, body) = health_with_down_kv(level).await;
        assert_eq!(status, StatusCode::OK, "{level}");
        assert_eq!(body["status"], "degraded", "{level}");
    }
}
