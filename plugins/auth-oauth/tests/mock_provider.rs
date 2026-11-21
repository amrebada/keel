//! Social login against a mock OAuth2/OIDC provider that checks the client
//! credentials, redirect URI and PKCE like a real one.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::{Form, Query, State};
use axum::http::{HeaderMap, Request, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine as _;
use http_body_util::BodyExt;
use keel_core::{AppConfig, Host, HostOptions, MapSecrets, PluginRegistry};
use keel_providers::{ConsoleEmail, DefaultCrypto, MemoryStore, ReqwestClient, SqlProvider};
use keel_sdk::capabilities::{CapabilityProviders, HttpClient, HttpRequest};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tower::ServiceExt;

#[derive(Default)]
struct Mock {
    /// The profile the next sign-in returns.
    next: Mutex<Value>,
    /// code -> (challenge, redirect_uri, profile)
    codes: Mutex<HashMap<String, (Option<String>, String, Value)>>,
    /// access token -> profile
    tokens: Mutex<HashMap<String, Value>>,
}

type M = Arc<Mock>;

async fn authorize(State(m): State<M>, Query(q): Query<HashMap<String, String>>) -> Json<Value> {
    assert_eq!(q["response_type"], "code");
    let code = format!("code-{}", m.codes.lock().unwrap().len());
    let profile = m.next.lock().unwrap().clone();
    m.codes.lock().unwrap().insert(
        code.clone(),
        (q.get("code_challenge").cloned(), q["redirect_uri"].clone(), profile),
    );
    Json(json!({ "code": code, "state": q["state"], "client_id": q["client_id"], "scope": q["scope"] }))
}

async fn token(State(m): State<M>, headers: HeaderMap, Form(f): Form<HashMap<String, String>>) -> Result<Json<Value>, StatusCode> {
    let (id, secret) = match headers.get("authorization").and_then(|v| v.to_str().ok()) {
        Some(b) => {
            let raw = String::from_utf8(STANDARD.decode(b.trim_start_matches("Basic ")).unwrap()).unwrap();
            let (i, s) = raw.split_once(':').unwrap();
            (i.to_string(), s.to_string())
        }
        None => (f["client_id"].clone(), f.get("client_secret").cloned().unwrap_or_default()),
    };
    if !id.starts_with("client-") || secret != format!("secret-for-{id}") {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let (challenge, redirect, profile) = m.codes.lock().unwrap().remove(&f["code"]).ok_or(StatusCode::BAD_REQUEST)?;
    if f["redirect_uri"] != redirect {
        return Err(StatusCode::BAD_REQUEST);
    }
    if let Some(c) = challenge {
        let v = f.get("code_verifier").ok_or(StatusCode::BAD_REQUEST)?;
        if URL_SAFE_NO_PAD.encode(Sha256::digest(v.as_bytes())) != c {
            return Err(StatusCode::BAD_REQUEST);
        }
    }
    let at = format!("at-{}", f["code"]);
    m.tokens.lock().unwrap().insert(at.clone(), profile);
    Ok(Json(json!({ "access_token": at, "token_type": "bearer" })))
}

fn profile_of(m: &M, headers: &HeaderMap) -> Result<Value, StatusCode> {
    let at = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or(StatusCode::UNAUTHORIZED)?;
    m.tokens.lock().unwrap().get(at).cloned().ok_or(StatusCode::UNAUTHORIZED)
}

async fn userinfo(State(m): State<M>, h: HeaderMap) -> Result<Json<Value>, StatusCode> {
    profile_of(&m, &h).map(Json)
}

async fn gh_user(State(m): State<M>, h: HeaderMap) -> Result<Json<Value>, StatusCode> {
    let p = profile_of(&m, &h)?;
    Ok(Json(json!({ "id": p["id"], "login": p["login"], "name": p["name"], "email": null })))
}

async fn gh_emails(State(m): State<M>, h: HeaderMap) -> Result<Json<Value>, StatusCode> {
    Ok(Json(profile_of(&m, &h)?["emails"].clone()))
}

async fn mock_server() -> (String, M) {
    let m: M = Arc::default();
    let app = Router::new()
        .route("/authorize", get(authorize))
        .route("/token", post(token))
        .route("/userinfo", get(userinfo))
        .route("/user", get(gh_user))
        .route("/user/emails", get(gh_emails))
        .with_state(m.clone());
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (format!("http://{addr}"), m)
}

struct T {
    router: axum::Router,
    mock: M,
    http: ReqwestClient,
}

async fn setup() -> T {
    let (url, mock) = mock_server().await;
    let mut secrets: HashMap<String, String> = HashMap::new();
    secrets.insert("AUTH_JWT_SECRET".into(), "x".repeat(40));
    for p in ["GOOGLE", "GITHUB", "X"] {
        let id = format!("client-{}", p.to_lowercase());
        secrets.insert(format!("OAUTH_{p}_CLIENT_SECRET"), format!("secret-for-{id}"));
        secrets.insert(format!("OAUTH_{p}_CLIENT_ID"), id);
    }
    let providers = CapabilityProviders {
        sql: Some(Arc::new(SqlProvider::connect("sqlite::memory:", None).await.unwrap())),
        cache: Some(Arc::new(MemoryStore::new())),
        crypto: Some(Arc::new(DefaultCrypto)),
        http: Some(Arc::new(ReqwestClient::default())),
        email: Some(Arc::new(ConsoleEmail::default())),
        secrets: Some(Arc::new(secrets.clone())),
        ..Default::default()
    };
    let yaml = format!(
        "app: t
plugins:
  - use: auth-core
    config: {{ require_verified_email: false }}
  - use: auth-oauth
    config:
      redirect_base_url: https://api.test
      providers:
        google: {{ auth_url: '{url}/authorize', token_url: '{url}/token', userinfo_url: '{url}/userinfo' }}
        github: {{ auth_url: '{url}/authorize', token_url: '{url}/token', userinfo_url: '{url}/user', emails_url: '{url}/user/emails' }}
        x: {{ auth_url: '{url}/authorize', token_url: '{url}/token', userinfo_url: '{url}/userinfo' }}
        microsoft: {{}}
"
    );
    let config = AppConfig::load_str(&yaml, "app.yaml", &MapSecrets::default()).unwrap();
    let registry = PluginRegistry::new()
        .register(keel_plugin_auth_core::AuthCore::new())
        .register(keel_plugin_auth_oauth::AuthOauth::new());
    let host = Host::start_with(
        config,
        &registry,
        &providers,
        HostOptions {
            secrets: Arc::new(MapSecrets(secrets)),
            state_file: None,
        },
    )
    .await
    .unwrap();
    T {
        router: host.router(),
        mock,
        http: ReqwestClient::default(),
    }
}

impl T {
    async fn call(&self, method: &str, uri: &str, body: Option<Value>, bearer: Option<&str>) -> (StatusCode, HeaderMap, Value) {
        let mut req = Request::builder().method(method).uri(uri);
        if let Some(b) = bearer {
            req = req.header("authorization", format!("Bearer {b}"));
        }
        let body = body.map(|b| Body::from(b.to_string())).unwrap_or_default();
        let res = self.router.clone().oneshot(req.body(body).unwrap()).await.unwrap();
        let (s, h) = (res.status(), res.headers().clone());
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (s, h, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    /// start -> provider authorize -> callback. Returns the callback answer
    /// and the callback query, so tests can replay it.
    async fn sign_in(&self, provider: &str, profile: Value) -> (StatusCode, Value, String) {
        *self.mock.next.lock().unwrap() = profile;
        let (s, h, b) = self.call("GET", &format!("/auth/oauth/{provider}/start"), None, None).await;
        assert_eq!(s, StatusCode::FOUND, "{b}");
        let location = h["location"].to_str().unwrap().to_string();
        assert!(location.contains(&format!(
            "redirect_uri={}",
            urlencoding::encode(&format!("https://api.test/auth/oauth/{provider}/callback"))
        )));
        let res = self
            .http
            .send(HttpRequest {
                method: "GET".into(),
                url: location,
                ..Default::default()
            })
            .await
            .unwrap();
        let auth: Value = serde_json::from_slice(&res.body).unwrap();
        let query = format!(
            "code={}&state={}",
            auth["code"].as_str().unwrap(),
            urlencoding::encode(auth["state"].as_str().unwrap())
        );
        let (s, _, b) = self
            .call("GET", &format!("/auth/oauth/{provider}/callback?{query}"), None, None)
            .await;
        (s, b, query)
    }

    async fn me(&self, access: &Value) -> Value {
        let (_, _, b) = self.call("GET", "/auth/me", None, access.as_str()).await;
        b["user"].clone()
    }
}

fn google(sub: &str, email: &str, verified: bool) -> Value {
    json!({ "sub": sub, "email": email, "email_verified": verified, "name": "Gina" })
}

#[tokio::test]
async fn google_creates_then_signs_into_the_same_account() {
    let t = setup().await;
    let (s, b, query) = t.sign_in("google", google("g-1", "Gina@Example.com", true)).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let user = t.me(&b["access_token"]).await;
    assert_eq!(user["email"], "gina@example.com");
    assert_eq!(user["email_verified"], true);
    assert_eq!(user["name"], "Gina");

    let (s, b2, _) = t.sign_in("google", google("g-1", "gina@example.com", true)).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(t.me(&b2["access_token"]).await["id"], user["id"]);

    // A callback can be used once.
    let (s, _, b) = t.call("GET", &format!("/auth/oauth/google/callback?{query}"), None, None).await;
    assert_eq!((s, b["error"]["code"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_state")));
}

#[tokio::test]
async fn verified_email_links_to_the_existing_password_account() {
    let t = setup().await;
    let (s, _, b) = t
        .call("POST", "/auth/signup", Some(json!({ "email": "bob@example.com", "password": "secretpass" })), None)
        .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let password_user = b["user"]["id"].clone();

    // GitHub: email only from /user/emails, primary + verified.
    let gh = json!({ "id": 42, "login": "bob", "name": null, "emails": [
        { "email": "old@example.com", "primary": false, "verified": true },
        { "email": "bob@example.com", "primary": true, "verified": true }
    ]});
    let (s, b, _) = t.sign_in("github", gh).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let user = t.me(&b["access_token"]).await;
    assert_eq!(user["id"], password_user, "linked, not a second account");
    assert_eq!(user["email_verified"], true, "the provider vouched for the email");

    // Unverified email that matches an account: refused, not linked.
    let (s, b, _) = t.sign_in("google", google("g-2", "bob@example.com", false)).await;
    assert_eq!((s, b["error"]["code"].as_str()), (StatusCode::CONFLICT, Some("email_in_use")), "{b}");
}

#[tokio::test]
async fn x_has_no_email_and_uses_basic_auth_and_pkce() {
    let t = setup().await;
    let (s, b, _) = t
        .sign_in("x", json!({ "data": { "id": "x-9", "name": "Xavier", "username": "xav" } }))
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let user = t.me(&b["access_token"]).await;
    assert_eq!(user["email"], Value::Null);
    assert_eq!(user["name"], "Xavier");
}

#[tokio::test]
async fn unknown_disabled_and_unconfigured_providers() {
    let t = setup().await;
    let (s, _, b) = t.call("GET", "/auth/oauth/facebook/start", None, None).await;
    assert_eq!((s, b["error"]["code"].as_str()), (StatusCode::NOT_FOUND, Some("unknown_provider")));

    // Enabled without its secrets: only that provider is blocked.
    let (s, _, b) = t.call("GET", "/auth/oauth/microsoft/start", None, None).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(b["error"]["step"], "auth-oauth/microsoft-app");
    let (s, _, _) = t.call("GET", "/auth/oauth/google/start", None, None).await;
    assert_eq!(s, StatusCode::FOUND);

    let (_, _, b) = t.call("GET", "/auth/oauth/providers", None, None).await;
    let ready: Vec<(String, bool)> = b["providers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| (p["id"].as_str().unwrap().to_string(), p["ready"].as_bool().unwrap()))
        .collect();
    assert!(ready.contains(&("microsoft".into(), false)));
    assert!(ready.contains(&("google".into(), true)));

    let (s, _, b) = t
        .call("GET", "/auth/oauth/google/callback?error=access_denied", None, None)
        .await;
    assert_eq!((s, b["error"]["code"].as_str()), (StatusCode::BAD_REQUEST, Some("provider_denied")));
}
