//! rbac with the host's route guards: roles, tenant roles, grants, owner,
//! rate limits and request validation, end to end over HTTP.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use keel_core::{AppConfig, Host, HostOptions, MapSecrets, PluginRegistry};
use keel_providers::{ConsoleEmail, DefaultCrypto, MemoryStore, SqlProvider};
use keel_sdk::capabilities::CapabilityProviders;
use keel_sdk::prelude::*;
use serde_json::{json, Value};
use tower::ServiceExt;

/// A plugin with routes for the guards to protect.
struct Demo;

#[async_trait]
impl Plugin for Demo {
    fn manifest(&self) -> &Manifest {
        static M: OnceLock<Manifest> = OnceLock::new();
        M.get_or_init(|| {
            Manifest::from_sources(
                "id: demo\nversion: 0.1.0\nsummary: Routes for guard tests.\nrequires:\n  host: \"^0.1\"\n  capabilities: []\nprovides:\n  routes: [\"GET /admin/stats\", \"GET /users/{id}/profile\", \"POST /notes\", \"GET /notes\", \"GET /teams/{team}/settings\"]\n",
                None,
            )
            .unwrap()
        })
    }

    async fn init(&self, ctx: &mut InitContext) -> Result<(), PluginError> {
        let r = ctx.routes();
        r.get("/admin/stats", || async { Json(json!({ "users": 2 })) })?;
        r.get("/users/{id}/profile", |p: extract::Path<String>| async move {
            Json(json!({ "id": p.0 }))
        })?;
        r.post("/notes", |b: axum::body::Bytes| async move {
            let v: Value = serde_json::from_slice(&b).unwrap();
            Json(json!({ "created": v }))
        })?;
        r.get("/notes", || async { Json(json!({ "notes": [] })) })?;
        r.get("/teams/{team}/settings", || async { Json(json!({ "ok": true })) })?;
        Ok(())
    }
}

const APP: &str = r#"
app: t
plugins:
  - use: auth-core
    config: { require_verified_email: false }
  - use: rbac
    config:
      roles:
        admin: { permissions: ["*"] }
        team-owner: { permissions: ["team.settings", "rbac.manage"], inherits: [member] }
        member: { permissions: ["notes.read"] }
      admins: [boss@example.com]
  - use: demo
routes:
  /admin/**: { guard: [authenticated, "role:admin"] }
  /users/{id}/**: { guard: ["any(role:admin, owner:id)"] }
  "GET /notes": { guard: ["permission:notes.read"], validate: { query: { type: object, properties: { limit: { type: integer, maximum: 50 } }, additionalProperties: false } } }
  "POST /notes":
    guard: [authenticated, "rate-limit:3/min"]
    validate: { body: { type: object, required: [title], properties: { title: { type: string, minLength: 1 } } } }
  /teams/{team}/**: { guard: ["permission:team.settings"] }
"#;

struct T(axum::Router);

impl T {
    async fn new() -> Self {
        let mut secrets = HashMap::new();
        secrets.insert("AUTH_JWT_SECRET".to_string(), "k".repeat(40));
        let providers = CapabilityProviders {
            sql: Some(Arc::new(
                SqlProvider::connect("sqlite::memory:", None).await.unwrap(),
            )),
            cache: Some(Arc::new(MemoryStore::new())),
            crypto: Some(Arc::new(DefaultCrypto)),
            email: Some(Arc::new(ConsoleEmail::default())),
            secrets: Some(Arc::new(secrets.clone())),
            ..Default::default()
        };
        let config = AppConfig::load_str(APP, "app.yaml", &MapSecrets::default()).unwrap();
        let registry = PluginRegistry::new()
            .register(keel_plugin_auth_core::AuthCore::new())
            .register(keel_plugin_rbac::Rbac::new())
            .register(Demo);
        let opts = HostOptions {
            secrets: Arc::new(MapSecrets(secrets)),
            state_file: None,
        };
        let host = Host::start_with(config, &registry, &providers, opts)
            .await
            .unwrap();
        T(host.router())
    }

    async fn call(
        &self,
        method: &str,
        path: &str,
        body: Value,
        token: Option<&str>,
        tenant: Option<&str>,
    ) -> (StatusCode, Value) {
        let mut req = Request::builder().method(method).uri(path);
        if let Some(t) = token {
            req = req.header("authorization", format!("Bearer {t}"));
        }
        if let Some(t) = tenant {
            req = req.header("x-tenant-id", t);
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

    /// Signs up and returns (user id, access token).
    async fn user(&self, email: &str) -> (String, String) {
        let (s, b) = self
            .call(
                "POST",
                "/auth/signup",
                json!({ "email": email, "password": "secretpass" }),
                None,
                None,
            )
            .await;
        assert_eq!(s, StatusCode::CREATED, "{b}");
        (
            b["user"]["id"].as_str().unwrap().to_string(),
            b["login"]["access_token"].as_str().unwrap().to_string(),
        )
    }
}

fn code(b: &Value) -> Option<&str> {
    b["error"]["code"].as_str()
}

#[tokio::test]
async fn roles_tenants_owner_rate_limit_and_validation() {
    let t = T::new().await;
    let (boss_id, boss) = t.user("boss@example.com").await;
    let (mia_id, mia) = t.user("mia@example.com").await;

    // Admin route: anonymous 401, member 403, admin 200.
    let (s, b) = t.call("GET", "/admin/stats", Value::Null, None, None).await;
    assert_eq!((s, code(&b)), (StatusCode::UNAUTHORIZED, Some("missing_token")));
    let (s, b) = t.call("GET", "/admin/stats", Value::Null, Some(&mia), None).await;
    assert_eq!((s, code(&b)), (StatusCode::FORBIDDEN, Some("forbidden")), "{b}");
    let (s, _) = t
        .call("GET", "/admin/stats", Value::Null, Some(&boss), None)
        .await;
    assert_eq!(s, StatusCode::OK);
    let (s, b) = t
        .call("GET", "/admin/stats", Value::Null, Some("nope"), None)
        .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "{b}");

    // Owner or admin.
    let me = format!("/users/{mia_id}/profile");
    let (s, _) = t.call("GET", &me, Value::Null, Some(&mia), None).await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = t.call("GET", &me, Value::Null, Some(&boss), None).await;
    assert_eq!(s, StatusCode::OK);
    let (s, b) = t
        .call(
            "GET",
            &format!("/users/{boss_id}/profile"),
            Value::Null,
            Some(&mia),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{b}");

    // Default role gives notes.read; query validation.
    let (s, _) = t
        .call("GET", "/notes?limit=10", Value::Null, Some(&mia), None)
        .await;
    assert_eq!(s, StatusCode::OK);
    let (s, b) = t
        .call("GET", "/notes?limit=99&x=1", Value::Null, Some(&mia), None)
        .await;
    assert_eq!(
        (s, code(&b)),
        (StatusCode::UNPROCESSABLE_ENTITY, Some("validation_failed"))
    );
    assert_eq!(b["error"]["issues"].as_array().unwrap().len(), 2, "{b}");

    // Body validation, then the rate limit (3 per minute).
    let (s, b) = t
        .call("POST", "/notes", json!({ "title": "" }), Some(&mia), None)
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{b}");
    assert_eq!(b["error"]["issues"][0]["path"], "body.title", "{b}");
    let (s, b) = t
        .call("POST", "/notes", json!({ "title": "hi" }), Some(&mia), None)
        .await;
    assert_eq!(
        (s, &b["created"]["title"]),
        (StatusCode::OK, &json!("hi")),
        "body reaches the handler"
    );
    let (s, _) = t
        .call("POST", "/notes", json!({ "title": "hi" }), Some(&mia), None)
        .await;
    assert_eq!(s, StatusCode::OK);
    let (s, b) = t
        .call("POST", "/notes", json!({ "title": "hi" }), Some(&mia), None)
        .await;
    assert_eq!(
        (s, code(&b)),
        (StatusCode::TOO_MANY_REQUESTS, Some("rate_limited"))
    );
    let (s, _) = t
        .call("POST", "/notes", json!({ "title": "hi" }), Some(&boss), None)
        .await;
    assert_eq!(s, StatusCode::OK, "limits are per user");

    // Tenant roles: owner of team-a only.
    let (s, b) = t
        .call(
            "POST",
            &format!("/rbac/users/{mia_id}/roles"),
            json!({ "role": "team-owner", "tenant": "team-a" }),
            Some(&mia),
            None,
        )
        .await;
    assert_eq!(
        s,
        StatusCode::FORBIDDEN,
        "members cannot give themselves roles: {b}"
    );
    let (s, b) = t
        .call(
            "POST",
            &format!("/rbac/users/{mia_id}/roles"),
            json!({ "role": "team-owner", "tenant": "team-a" }),
            Some(&boss),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let (s, _) = t
        .call(
            "GET",
            "/teams/a/settings",
            Value::Null,
            Some(&mia),
            Some("team-a"),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = t
        .call(
            "GET",
            "/teams/b/settings",
            Value::Null,
            Some(&mia),
            Some("team-b"),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = t
        .call("GET", "/teams/a/settings", Value::Null, Some(&mia), None)
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "tenant roles need the tenant header");

    let (_, b) = t
        .call("GET", "/rbac/me", Value::Null, Some(&mia), Some("team-a"))
        .await;
    assert_eq!(b["roles"], json!(["member", "team-owner"]), "{b}");

    // A tenant manager can manage their tenant only.
    let (s, _) = t
        .call(
            "POST",
            &format!("/rbac/users/{boss_id}/permissions"),
            json!({ "permission": "x.y", "tenant": "team-a" }),
            Some(&mia),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    let (s, _) = t
        .call(
            "POST",
            &format!("/rbac/users/{boss_id}/permissions"),
            json!({ "permission": "x.y" }),
            Some(&mia),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, b) = t
        .call(
            "POST",
            &format!("/rbac/users/{mia_id}/roles"),
            json!({ "role": "ghost" }),
            Some(&boss),
            None,
        )
        .await;
    assert_eq!((s, code(&b)), (StatusCode::BAD_REQUEST, Some("unknown_role")));

    let (s, b) = t
        .call(
            "DELETE",
            &format!("/rbac/users/{mia_id}/roles/team-owner?tenant=team-a"),
            Value::Null,
            Some(&boss),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (s, _) = t
        .call(
            "GET",
            "/teams/a/settings",
            Value::Null,
            Some(&mia),
            Some("team-a"),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn guards_without_their_plugins_stop_the_start() {
    let providers = CapabilityProviders {
        sql: Some(Arc::new(
            SqlProvider::connect("sqlite::memory:", None).await.unwrap(),
        )),
        ..Default::default()
    };
    let config = AppConfig::load_str(
        "app: t\nplugins:\n  - use: demo\nroutes:\n  /admin/**: { guard: [\"role:admin\", \"plan:pro\", \"rate-limit:1/s\"] }\n",
        "app.yaml",
        &MapSecrets::default(),
    )
    .unwrap();
    let registry = PluginRegistry::new().register(Demo);
    let err = match Host::start(config, &registry, &providers).await {
        Err(e) => e.to_string(),
        Ok(_) => panic!("should not start"),
    };
    assert!(err.contains("needs the auth-core and rbac plugins"), "{err}");
    assert!(err.contains("no plugin offers the `plan` guard"), "{err}");
    assert!(err.contains("needs a cache provider"), "{err}");
}
