use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use keel_core::{AppConfig, Host, HostOptions, MapSecrets, PluginRegistry};
use keel_providers::{ConsoleEmail, DefaultCrypto, SqlProvider};
use keel_sdk::capabilities::CapabilityProviders;
use serde_json::{json, Value};
use tower::ServiceExt;

struct T(axum::Router);

impl T {
    async fn new() -> Self {
        let mut secrets = HashMap::new();
        secrets.insert("AUTH_JWT_SECRET".to_string(), "k".repeat(40));
        let providers = CapabilityProviders {
            sql: Some(Arc::new(
                SqlProvider::connect("sqlite::memory:", None).await.unwrap(),
            )),
            crypto: Some(Arc::new(DefaultCrypto)),
            email: Some(Arc::new(ConsoleEmail::default())),
            secrets: Some(Arc::new(secrets.clone())),
            ..Default::default()
        };
        let config = AppConfig::load_str(
            "app: t\nplugins:\n  - use: auth-core\n    config: { require_verified_email: false }\n  - use: auth-mfa\n    config: { issuer: Shop, recovery_codes: 5 }\n",
            "app.yaml",
            &MapSecrets::default(),
        )
        .unwrap();
        let registry = PluginRegistry::new()
            .register(keel_plugin_auth_mfa::AuthMfa::new())
            .register(keel_plugin_auth_core::AuthCore::new());
        let opts = HostOptions {
            secrets: Arc::new(MapSecrets(secrets)),
            state_file: None,
        };
        T(Host::start_with(config, &registry, &providers, opts)
            .await
            .unwrap()
            .router())
    }

    async fn call(&self, path: &str, body: Value, bearer: Option<&str>) -> (StatusCode, Value) {
        let method = if body.is_null() { "GET" } else { "POST" };
        let mut req = Request::builder().method(method).uri(path);
        if let Some(b) = bearer {
            req = req.header("authorization", format!("Bearer {b}"));
        }
        let body = if body.is_null() {
            Body::empty()
        } else {
            Body::from(body.to_string())
        };
        let res = self.0.clone().oneshot(req.body(body).unwrap()).await.unwrap();
        let s = res.status();
        let b = res.into_body().collect().await.unwrap().to_bytes();
        (s, serde_json::from_slice(&b).unwrap_or(Value::Null))
    }
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

#[tokio::test]
async fn totp_login_recovery_codes_and_lockout() {
    let t = T::new().await;
    let creds = json!({ "email": "mia@example.com", "password": "secretpass" });
    let (s, b) = t.call("/auth/signup", creds.clone(), None).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let access = b["login"]["access_token"].as_str().unwrap().to_string();

    let (_, b) = t.call("/auth/mfa/status", Value::Null, Some(&access)).await;
    assert_eq!(b["enabled"], false);

    let (s, b) = t.call("/auth/mfa/totp/setup", json!({}), Some(&access)).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let secret = b["secret"].as_str().unwrap().to_string();
    assert!(b["otpauth_url"]
        .as_str()
        .unwrap()
        .starts_with("otpauth://totp/Shop%3Amia%40example.com?secret="));

    // Logins do not ask for a code until the device is confirmed.
    let (_, b) = t.call("/auth/login", creds.clone(), None).await;
    assert!(b["access_token"].is_string());

    let code_now = keel_plugin_auth_mfa::totp_code(&secret, now()).unwrap();
    let (s, b) = t
        .call(
            "/auth/mfa/totp/confirm",
            json!({ "code": code_now }),
            Some(&access),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let recovery: Vec<String> = serde_json::from_value(b["recovery_codes"].clone()).unwrap();
    assert_eq!(recovery.len(), 5);

    // Now a login needs the second factor.
    let (_, b) = t.call("/auth/login", creds.clone(), None).await;
    assert_eq!(b["mfa_required"], true, "{b}");
    assert!(b.get("access_token").is_none());
    let mfa = b["mfa_token"].as_str().unwrap().to_string();

    // The code just used cannot be replayed; a wrong code returns a fresh challenge.
    let (s, b) = t
        .call(
            "/auth/mfa/verify",
            json!({ "mfa_token": mfa, "code": code_now }),
            None,
        )
        .await;
    assert_eq!(
        (s, b["error"]["code"].as_str()),
        (StatusCode::UNAUTHORIZED, Some("invalid_code"))
    );
    let mfa = b["error"]["mfa_token"].as_str().unwrap().to_string();
    let next = keel_plugin_auth_mfa::totp_code(&secret, now() + 30).unwrap();
    let (s, b) = t
        .call(
            "/auth/mfa/verify",
            json!({ "mfa_token": mfa, "code": next }),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert!(b["access_token"].is_string());

    // A recovery code works once.
    let (_, b) = t.call("/auth/login", creds.clone(), None).await;
    let (s, b) = t
        .call(
            "/auth/mfa/verify",
            json!({ "mfa_token": b["mfa_token"], "recovery_code": recovery[0].to_uppercase() }),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let fresh = b["access_token"].as_str().unwrap().to_string();
    let (_, b) = t.call("/auth/login", creds.clone(), None).await;
    let (s, _) = t
        .call(
            "/auth/mfa/verify",
            json!({ "mfa_token": b["mfa_token"], "recovery_code": recovery[0] }),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (_, b) = t.call("/auth/mfa/status", Value::Null, Some(&fresh)).await;
    assert_eq!(
        (b["enabled"].as_bool(), b["recovery_codes_left"].as_i64()),
        (Some(true), Some(4))
    );

    // Magic-link logins need it too.
    let (_, b) = t.call("/auth/login", creds.clone(), None).await;
    let mut mfa = b["mfa_token"].as_str().unwrap().to_string();

    // Five wrong codes lock the second factor.
    for _ in 0..5 {
        let (_, b) = t
            .call(
                "/auth/mfa/verify",
                json!({ "mfa_token": mfa, "code": "000000" }),
                None,
            )
            .await;
        mfa = b["error"]["mfa_token"].as_str().unwrap().to_string();
    }
    let right = keel_plugin_auth_mfa::totp_code(&secret, now()).unwrap();
    let (s, b) = t
        .call(
            "/auth/mfa/verify",
            json!({ "mfa_token": mfa, "code": right }),
            None,
        )
        .await;
    assert_eq!(
        (s, b["error"]["code"].as_str()),
        (StatusCode::TOO_MANY_REQUESTS, Some("mfa_locked")),
        "{b}"
    );
}
