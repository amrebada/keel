//! The full account flow through the HTTP host, on SQLite always and on
//! Postgres / MySQL when KEEL_TEST_POSTGRES_URL / KEEL_TEST_MYSQL_URL are set.

use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use keel_core::{AppConfig, Host, MapSecrets, PluginRegistry};
use keel_providers::{ConsoleEmail, DefaultCrypto, SqlProvider};
use keel_sdk::capabilities::{CapabilityProviders, Sql};
use serde_json::{json, Value};
use tower::ServiceExt;

const KEY: &str = "0123456789abcdef0123456789abcdef-test-key";

struct App {
    router: axum::Router,
    email: Arc<ConsoleEmail>,
}

async fn app(db_url: &str, with_key: bool) -> App {
    let sql = SqlProvider::connect(db_url, Some(2)).await.unwrap();
    for t in [
        "auth_core_users",
        "auth_core_sessions",
        "auth_core_tokens",
        "auth_core_used_refresh",
    ] {
        let _ = sql.execute(&format!("DROP TABLE {t}"), &[]).await;
    }
    let _ = sql
        .execute("DELETE FROM keel_migrations WHERE plugin = 'auth-core'", &[])
        .await;
    let email = Arc::new(ConsoleEmail::new("test@keel"));
    let mut secrets = HashMap::new();
    if with_key {
        secrets.insert("AUTH_JWT_SECRET".to_string(), KEY.to_string());
    }
    let providers = CapabilityProviders {
        sql: Some(Arc::new(sql)),
        crypto: Some(Arc::new(DefaultCrypto)),
        email: Some(email.clone()),
        secrets: Some(Arc::new(secrets.clone())),
        ..Default::default()
    };
    let config = AppConfig::load_str(
        "app: t\nplugins:\n  - use: auth-core\n    config: { app_url: 'https://app.test' }\n",
        "app.yaml",
        &MapSecrets::default(),
    )
    .unwrap();
    let registry = PluginRegistry::new().register(keel_plugin_auth_core::AuthCore::new());
    let options = keel_core::HostOptions {
        secrets: Arc::new(MapSecrets(secrets)),
        state_file: None,
    };
    let host = Host::start_with(config, &registry, &providers, options)
        .await
        .unwrap();
    App {
        router: host.router(),
        email,
    }
}

impl App {
    async fn call(&self, method: &str, path: &str, body: Value, bearer: Option<&str>) -> (StatusCode, Value) {
        let mut req = Request::builder().method(method).uri(path);
        if let Some(t) = bearer {
            req = req.header("authorization", format!("Bearer {t}"));
        }
        let body = if body.is_null() {
            Body::empty()
        } else {
            Body::from(body.to_string())
        };
        let res = self
            .router
            .clone()
            .oneshot(req.body(body).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    /// The token in the last email whose link contains `marker`.
    fn token_from_email(&self, marker: &str) -> String {
        let sent = self.email.sent();
        let text = &sent
            .iter()
            .rev()
            .find(|m| m.text.contains(marker))
            .expect("email sent")
            .text;
        let start = text.find("token=").unwrap() + 6;
        text[start..]
            .chars()
            .take_while(|c| c.is_ascii_hexdigit())
            .collect()
    }
}

async fn full_flow(db_url: &str) {
    let a = app(db_url, true).await;
    let email = format!("ann+{}@example.com", std::process::id());

    let (s, b) = a
        .call(
            "POST",
            "/auth/signup",
            json!({ "email": email.to_uppercase(), "password": "secretpass", "name": "Ann" }),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    assert_eq!(b["user"]["email"], email);
    assert_eq!(b["user"]["email_verified"], false);

    let (s, b) = a
        .call(
            "POST",
            "/auth/signup",
            json!({ "email": email, "password": "secretpass" }),
            None,
        )
        .await;
    assert_eq!(
        (s, b["error"]["code"].as_str()),
        (StatusCode::CONFLICT, Some("email_taken"))
    );
    let (s, b) = a
        .call(
            "POST",
            "/auth/signup",
            json!({ "email": "x@y.co", "password": "short" }),
            None,
        )
        .await;
    assert_eq!(
        (s, b["error"]["code"].as_str()),
        (StatusCode::BAD_REQUEST, Some("weak_password"))
    );

    let login = json!({ "email": email, "password": "secretpass" });
    let (s, b) = a.call("POST", "/auth/login", login.clone(), None).await;
    assert_eq!(
        (s, b["error"]["code"].as_str()),
        (StatusCode::FORBIDDEN, Some("email_not_verified"))
    );

    let link = a.email.sent().last().unwrap().text.clone();
    assert!(link.contains("https://app.test/auth/verify?token="), "{link}");
    let token = a.token_from_email("/auth/verify?token=");
    let (s, _) = a
        .call("POST", "/auth/verify", json!({ "token": token }), None)
        .await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = a
        .call("POST", "/auth/verify", json!({ "token": token }), None)
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "tokens are single use");

    let (s, b) = a
        .call(
            "POST",
            "/auth/login",
            json!({ "email": email, "password": "wrongpass" }),
            None,
        )
        .await;
    assert_eq!(
        (s, b["error"]["code"].as_str()),
        (StatusCode::UNAUTHORIZED, Some("invalid_credentials"))
    );
    let (s, b) = a.call("POST", "/auth/login", login.clone(), None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let access = b["access_token"].as_str().unwrap().to_string();
    let refresh = b["refresh_token"].as_str().unwrap().to_string();
    assert_eq!(b["token_type"], "Bearer");

    let (s, b) = a.call("GET", "/auth/me", Value::Null, Some(&access)).await;
    assert_eq!(
        (s, b["user"]["email_verified"].as_bool()),
        (StatusCode::OK, Some(true))
    );
    let (s, _) = a.call("GET", "/auth/me", Value::Null, None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);

    let (s, b) = a
        .call("POST", "/auth/refresh", json!({ "refresh_token": refresh }), None)
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let refresh2 = b["refresh_token"].as_str().unwrap().to_string();
    let access2 = b["access_token"].as_str().unwrap().to_string();
    let (s, b) = a
        .call("POST", "/auth/refresh", json!({ "refresh_token": refresh }), None)
        .await;
    assert_eq!(
        (s, b["error"]["code"].as_str()),
        (StatusCode::UNAUTHORIZED, Some("refresh_token_reused")),
        "a rotated refresh token is reuse"
    );
    // Reuse ends the whole session: the newer tokens stop working too.
    let (s, _) = a
        .call(
            "POST",
            "/auth/refresh",
            json!({ "refresh_token": refresh2 }),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    for t in [&access, &access2] {
        let (s, b) = a.call("GET", "/auth/me", Value::Null, Some(t)).await;
        assert_eq!(
            (s, b["error"]["code"].as_str()),
            (StatusCode::UNAUTHORIZED, Some("session_revoked"))
        );
    }

    // Logout ends one session.
    let (_, b) = a.call("POST", "/auth/login", login.clone(), None).await;
    let access = b["access_token"].as_str().unwrap().to_string();
    let refresh = b["refresh_token"].as_str().unwrap().to_string();
    let (s, _) = a
        .call("POST", "/auth/logout", json!({ "refresh_token": refresh }), None)
        .await;
    assert_eq!(s, StatusCode::OK);
    let (s, b) = a.call("GET", "/auth/me", Value::Null, Some(&access)).await;
    assert_eq!(
        (s, b["error"]["code"].as_str()),
        (StatusCode::UNAUTHORIZED, Some("session_revoked"))
    );

    // Two sessions, then logout everywhere.
    let (_, b1) = a.call("POST", "/auth/login", login.clone(), None).await;
    let (_, b2) = a.call("POST", "/auth/login", login.clone(), None).await;
    let t1 = b1["access_token"].as_str().unwrap();
    let (s, b) = a.call("POST", "/auth/logout-all", Value::Null, Some(t1)).await;
    assert_eq!((s, b["revoked_sessions"].as_u64()), (StatusCode::OK, Some(2)));
    let (s, _) = a
        .call("GET", "/auth/me", Value::Null, b2["access_token"].as_str())
        .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);

    // Password reset ends sessions and changes the password.
    let (_, b) = a.call("POST", "/auth/login", login.clone(), None).await;
    let live = b["access_token"].as_str().unwrap().to_string();
    let (s, _) = a
        .call("POST", "/auth/password/forgot", json!({ "email": email }), None)
        .await;
    assert_eq!(s, StatusCode::ACCEPTED);
    let (s, _) = a
        .call(
            "POST",
            "/auth/password/forgot",
            json!({ "email": "nobody@example.com" }),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::ACCEPTED, "no account enumeration");
    let reset = a.token_from_email("reset-password?token=");
    let (s, b) = a
        .call(
            "POST",
            "/auth/password/reset",
            json!({ "token": reset, "password": "brandnewpass" }),
            None,
        )
        .await;
    assert_eq!(
        (s, b["revoked_sessions"].as_u64()),
        (StatusCode::OK, Some(1)),
        "{b}"
    );
    let (s, _) = a.call("GET", "/auth/me", Value::Null, Some(&live)).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, _) = a.call("POST", "/auth/login", login.clone(), None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "old password no longer works");
    let (s, _) = a
        .call(
            "POST",
            "/auth/login",
            json!({ "email": email, "password": "brandnewpass" }),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK);

    // Magic link.
    let (s, _) = a
        .call("POST", "/auth/magic-link", json!({ "email": email }), None)
        .await;
    assert_eq!(s, StatusCode::ACCEPTED);
    let magic = a.token_from_email("magic-link/verify?token=");
    let (s, b) = a
        .call(
            "GET",
            &format!("/auth/magic-link/verify?token={magic}"),
            Value::Null,
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert!(b["access_token"].is_string());
    let (s, _) = a
        .call("POST", "/auth/magic-link/verify", json!({ "token": magic }), None)
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "magic links are single use");

    let (s, b) = a.call("POST", "/auth/login", json!("nope"), None).await;
    assert_eq!(
        (s, b["error"]["code"].as_str()),
        (StatusCode::BAD_REQUEST, Some("invalid_body"))
    );
}

#[tokio::test]
async fn full_flow_sqlite() {
    full_flow("sqlite::memory:").await;
}

#[tokio::test]
async fn full_flow_postgres() {
    match std::env::var("KEEL_TEST_POSTGRES_URL") {
        Ok(url) => full_flow(&url).await,
        Err(_) => eprintln!("skipped: KEEL_TEST_POSTGRES_URL not set"),
    }
}

#[tokio::test]
async fn full_flow_mysql() {
    match std::env::var("KEEL_TEST_MYSQL_URL") {
        Ok(url) => full_flow(&url).await,
        Err(_) => eprintln!("skipped: KEEL_TEST_MYSQL_URL not set"),
    }
}

#[tokio::test]
async fn routes_are_blocked_until_the_signing_key_is_set() {
    let a = app("sqlite::memory:", false).await;
    let (s, b) = a
        .call(
            "POST",
            "/auth/signup",
            json!({ "email": "a@b.co", "password": "secretpass" }),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(b["error"]["code"], "setup_incomplete");
    assert_eq!(b["error"]["step"], "auth-core/jwt-secret");
}
