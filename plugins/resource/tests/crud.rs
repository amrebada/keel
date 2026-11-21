//! A YAML-defined `projects` resource end to end over HTTP: list, filter,
//! sort, page, create, update, delete, behind rbac permission guards; plus
//! owner- and tenant-scoped resources and caching. SQLite always, Postgres
//! when KEEL_TEST_POSTGRES_URL is set.

use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use keel_core::{AppConfig, Host, HostOptions, MapSecrets, PluginRegistry};
use keel_providers::{ConsoleEmail, DefaultCrypto, MemoryStore, SqlProvider};
use keel_sdk::capabilities::{CapabilityProviders, Sql};
use serde_json::{json, Value};
use tower::ServiceExt;

const APP: &str = r#"
app: t
plugins:
  - use: auth-core
    config: { require_verified_email: false }
  - use: rbac
    config:
      roles:
        admin: { permissions: ["*"] }
        editor: { permissions: ["projects.*"], inherits: [member] }
        member: { permissions: ["projects.read"] }
      admins: [boss@example.com]
  - use: resource
    config:
      resources:
        projects:
          fields:
            name: { type: string, required: true, min_length: 2, max_length: 80 }
            slug: { type: string, required: true, unique: true, pattern: "^[a-z0-9-]+$" }
            status: { type: string, enum: [active, archived], default: active }
            budget: { type: number, min: 0 }
            stars: { type: integer, default: 0 }
            public: { type: boolean, default: false }
            due: { type: date }
            meta: { type: json }
        notes:
          scope: owner
          cache_ttl: 60
          fields:
            body: { type: text, required: true }
        tasks:
          scope: tenant
          fields:
            title: { type: string, required: true, unique: true }
routes:
  "GET /api/projects": { guard: ["permission:projects.read"] }
  "GET /api/projects/*": { guard: ["permission:projects.read"] }
  "POST /api/projects": { guard: ["permission:projects.write"] }
  "PATCH /api/projects/*": { guard: ["permission:projects.write"] }
  "DELETE /api/projects/*": { guard: ["permission:projects.delete"] }
"#;

struct T(axum::Router);

impl T {
    async fn new(db: &str) -> Self {
        let sql = SqlProvider::connect(db, Some(2)).await.unwrap();
        for t in [
            "auth_core_users",
            "auth_core_sessions",
            "auth_core_tokens",
            "auth_core_used_refresh",
            "rbac_assignments",
            "rbac_grants",
            "resource_projects",
            "resource_notes",
            "resource_tasks",
        ] {
            let _ = sql.execute(&format!("DROP TABLE {t}"), &[]).await;
        }
        let _ = sql
            .execute(
                "DELETE FROM keel_migrations WHERE plugin IN ('auth-core', 'rbac', 'resource')",
                &[],
            )
            .await;
        let mut secrets = HashMap::new();
        secrets.insert("AUTH_JWT_SECRET".to_string(), "k".repeat(40));
        let providers = CapabilityProviders {
            sql: Some(Arc::new(sql)),
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
            .register(keel_plugin_resource::Resource::new());
        let opts = HostOptions {
            secrets: Arc::new(MapSecrets(secrets)),
            state_file: None,
        };
        let host = Host::start_with(config, &registry, &providers, opts)
            .await
            .unwrap();
        T(host.router())
    }

    async fn call(&self, method: &str, path: &str, body: Value, token: Option<&str>) -> (StatusCode, Value) {
        self.call_t(method, path, body, token, None).await
    }

    async fn call_t(
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
            req = req.header("content-type", "application/json");
            Body::from(body.to_string())
        };
        let res = self.0.clone().oneshot(req.body(body).unwrap()).await.unwrap();
        let s = res.status();
        let b = res.into_body().collect().await.unwrap().to_bytes();
        (s, serde_json::from_slice(&b).unwrap_or(Value::Null))
    }

    async fn user(&self, email: &str) -> (String, String) {
        let (s, b) = self
            .call(
                "POST",
                "/auth/signup",
                json!({ "email": email, "password": "secretpass" }),
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

fn names(b: &Value) -> Vec<String> {
    b["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["name"].as_str().unwrap().to_string())
        .collect()
}

async fn crud(db: &str) {
    let t = T::new(db).await;
    let (_, boss) = t.user("boss@example.com").await;
    let (ed_id, ed) = t.user("ed@example.com").await;
    let (_, mia) = t.user("mia@example.com").await;
    let (s, b) = t
        .call(
            "POST",
            &format!("/rbac/users/{ed_id}/roles"),
            json!({ "role": "editor" }),
            Some(&boss),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");

    // Guards: anonymous 401, member can read but not write.
    let (s, _) = t.call("GET", "/api/projects", Value::Null, None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, b) = t.call("GET", "/api/projects", Value::Null, Some(&mia)).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["total"], 0);
    let (s, b) = t
        .call(
            "POST",
            "/api/projects",
            json!({ "name": "Apollo", "slug": "apollo" }),
            Some(&mia),
        )
        .await;
    assert_eq!((s, code(&b)), (StatusCode::FORBIDDEN, Some("forbidden")), "{b}");

    // Create with defaults applied.
    let (s, apollo) = t
        .call(
            "POST",
            "/api/projects",
            json!({ "name": "Apollo", "slug": "apollo", "budget": 1200.5, "due": "2027-01-31", "meta": { "color": "red" } }),
            Some(&ed),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{apollo}");
    assert_eq!(apollo["status"], "active");
    assert_eq!(apollo["stars"], 0);
    assert_eq!(apollo["public"], false);
    assert_eq!(apollo["budget"], 1200.5);
    assert_eq!(apollo["meta"], json!({ "color": "red" }));
    assert!(apollo["created_at"].as_i64().unwrap() > 0);
    let apollo_id = apollo["id"].as_str().unwrap().to_string();
    for (name, slug, budget, stars, public) in [
        ("Gemini", "gemini", 300.0, 5, true),
        ("Mercury", "mercury", 50.0, 9, true),
        ("Artemis", "artemis", 9000.0, 2, false),
    ] {
        let (s, b) = t
            .call(
                "POST",
                "/api/projects",
                json!({ "name": name, "slug": slug, "budget": budget, "stars": stars, "public": public }),
                Some(&ed),
            )
            .await;
        assert_eq!(s, StatusCode::CREATED, "{b}");
    }

    // Validation and unique values.
    let (s, b) = t
        .call(
            "POST",
            "/api/projects",
            json!({ "name": "X", "slug": "Bad Slug", "owner": 1 }),
            Some(&ed),
        )
        .await;
    assert_eq!(
        (s, code(&b)),
        (StatusCode::UNPROCESSABLE_ENTITY, Some("validation_failed")),
        "{b}"
    );
    assert!(b["error"]["issues"].as_array().unwrap().len() >= 3, "{b}");
    let (s, b) = t
        .call(
            "POST",
            "/api/projects",
            json!({ "name": "Apollo 2", "slug": "apollo" }),
            Some(&ed),
        )
        .await;
    assert_eq!((s, code(&b)), (StatusCode::CONFLICT, Some("conflict")), "{b}");
    assert_eq!(b["error"]["field"], "slug");

    // List: filters, sort and paging.
    let get = |q: &str| {
        let path = format!("/api/projects{q}");
        let (t, mia) = (&t, mia.clone());
        async move { t.call("GET", &path, Value::Null, Some(&mia)).await }
    };
    let (_, b) = get("").await;
    assert_eq!(names(&b), ["Apollo", "Gemini", "Mercury", "Artemis"]);
    let (_, b) = get("?public=true&sort=-stars").await;
    assert_eq!(names(&b), ["Mercury", "Gemini"]);
    let (_, b) = get("?budget.gte=300&sort=name").await;
    assert_eq!(names(&b), ["Apollo", "Artemis", "Gemini"]);
    let (_, b) = get("?name.contains=MER").await;
    assert_eq!(names(&b), ["Mercury"]);
    let (_, b) = get("?slug.in=gemini,artemis&sort=-name").await;
    assert_eq!(names(&b), ["Gemini", "Artemis"]);
    let (_, b) = get("?due.null=false").await;
    assert_eq!(names(&b), ["Apollo"]);
    let (_, b) = get("?sort=name&limit=2&offset=1").await;
    assert_eq!(
        (names(&b), b["total"].clone(), b["limit"].clone()),
        (vec!["Artemis".to_string(), "Gemini".into()], json!(4), json!(2))
    );
    let (s, b) = get("?colour=red").await;
    assert_eq!((s, code(&b)), (StatusCode::BAD_REQUEST, Some("unknown_filter")));
    let (s, b) = get("?meta=1").await;
    assert_eq!(
        (s, code(&b)),
        (StatusCode::BAD_REQUEST, Some("unknown_filter")),
        "json is not filterable"
    );
    let (s, b) = get("?stars.gt=lots").await;
    assert_eq!((s, code(&b)), (StatusCode::BAD_REQUEST, Some("invalid_filter")));
    let (s, b) = get("?limit=1000").await;
    assert_eq!((s, code(&b)), (StatusCode::BAD_REQUEST, Some("invalid_limit")));

    // Get, update, delete.
    let one = format!("/api/projects/{apollo_id}");
    let (s, b) = t.call("GET", &one, Value::Null, Some(&mia)).await;
    assert_eq!((s, b["slug"].as_str()), (StatusCode::OK, Some("apollo")));
    let (s, b) = t
        .call(
            "PATCH",
            &one,
            json!({ "status": "archived", "due": null, "stars": 7 }),
            Some(&ed),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(
        (b["status"].as_str(), b["due"].clone(), b["stars"].clone()),
        (Some("archived"), Value::Null, json!(7))
    );
    assert_eq!(b["name"], "Apollo", "fields left out keep their value");
    let (s, b) = t
        .call("PATCH", &one, json!({ "status": "gone" }), Some(&ed))
        .await;
    assert_eq!(
        (s, code(&b)),
        (StatusCode::UNPROCESSABLE_ENTITY, Some("validation_failed"))
    );
    let (s, b) = t.call("PATCH", &one, json!({ "name": null }), Some(&ed)).await;
    assert_eq!(
        s,
        StatusCode::UNPROCESSABLE_ENTITY,
        "required fields cannot be cleared: {b}"
    );
    let (s, _) = t.call("PATCH", &one, json!({ "stars": 1 }), Some(&mia)).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = t.call("DELETE", &one, Value::Null, Some(&ed)).await;
    assert_eq!(s, StatusCode::NO_CONTENT, "editor has projects.*");
    let (s, b) = t.call("GET", &one, Value::Null, Some(&mia)).await;
    assert_eq!((s, code(&b)), (StatusCode::NOT_FOUND, Some("not_found")));
    let (s, _) = t.call("DELETE", &one, Value::Null, Some(&boss)).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (_, b) = get("").await;
    assert_eq!(b["total"], 3);

    // Describe.
    let (s, b) = t.call("GET", "/api", Value::Null, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        b["resources"]["projects"]["create_schema"]["required"],
        json!(["name", "slug"])
    );
    assert_eq!(b["resources"]["notes"]["scope"], "owner");

    // Owner scope: each user sees their own notes; admins (notes.manage via *) see all.
    let (s, b) = t
        .call("POST", "/api/notes", json!({ "body": "mia's note" }), Some(&mia))
        .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let mia_note = b["id"].as_str().unwrap().to_string();
    let (_, b) = t.call("GET", "/api/notes", Value::Null, Some(&mia)).await;
    assert_eq!(b["total"], 1, "cached list for mia: {b}");
    t.call("POST", "/api/notes", json!({ "body": "ed's note" }), Some(&ed))
        .await;
    let (_, b) = t.call("GET", "/api/notes", Value::Null, Some(&ed)).await;
    assert_eq!(
        (b["total"].clone(), b["items"][0]["body"].clone()),
        (json!(1), json!("ed's note"))
    );
    let (_, b) = t.call("GET", "/api/notes", Value::Null, Some(&boss)).await;
    assert_eq!(b["total"], 2, "admin sees every note");
    let (s, _) = t
        .call("GET", &format!("/api/notes/{mia_note}"), Value::Null, Some(&ed))
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "other users' rows are invisible");
    let (s, _) = t
        .call(
            "DELETE",
            &format!("/api/notes/{mia_note}"),
            Value::Null,
            Some(&ed),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = t.call("GET", "/api/notes", Value::Null, None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    // Writes clear the cache.
    let (s, _) = t
        .call(
            "PATCH",
            &format!("/api/notes/{mia_note}"),
            json!({ "body": "edited" }),
            Some(&mia),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let (_, b) = t.call("GET", "/api/notes", Value::Null, Some(&mia)).await;
    assert_eq!(b["items"][0]["body"], "edited");

    // Tenant scope: rows and unique values are per team.
    let (s, b) = t.call("GET", "/api/tasks", Value::Null, None).await;
    assert_eq!((s, code(&b)), (StatusCode::BAD_REQUEST, Some("missing_tenant")));
    for team in ["acme", "globex"] {
        let (s, b) = t
            .call_t(
                "POST",
                "/api/tasks",
                json!({ "title": "Ship it" }),
                None,
                Some(team),
            )
            .await;
        assert_eq!(s, StatusCode::CREATED, "{b}");
        assert_eq!(b["tenant"], team);
    }
    let (s, _) = t
        .call_t(
            "POST",
            "/api/tasks",
            json!({ "title": "Ship it" }),
            None,
            Some("acme"),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT);
    let (_, b) = t
        .call_t("GET", "/api/tasks", Value::Null, None, Some("acme"))
        .await;
    assert_eq!(b["total"], 1);

    let (s, b) = t.call("GET", "/api/widgets", Value::Null, None).await;
    assert_eq!((s, code(&b)), (StatusCode::NOT_FOUND, Some("unknown_resource")));
}

#[tokio::test]
async fn crud_sqlite() {
    crud("sqlite::memory:").await;
}

#[tokio::test]
async fn crud_postgres() {
    match std::env::var("KEEL_TEST_POSTGRES_URL") {
        Ok(url) => crud(&url).await,
        Err(_) => eprintln!("skipped: KEEL_TEST_POSTGRES_URL not set"),
    }
}

#[tokio::test]
async fn adding_a_field_adds_a_migration() {
    let sql = SqlProvider::connect("sqlite::memory:", Some(1)).await.unwrap();
    let sql: Arc<dyn Sql> = Arc::new(sql);
    let plugin: Arc<dyn keel_sdk::Plugin> = Arc::new(keel_plugin_resource::Resource::new());
    let v1 = json!({ "resources": { "items": { "fields": { "name": { "type": "string" } } } } });
    let v2 = json!({ "resources": { "items": { "fields": { "name": { "type": "string", "required": true }, "qty": { "type": "integer" } } } } });
    let plan = keel_core::migrate::plan_with(sql.as_ref(), &[(plugin.clone(), v1)], &[])
        .await
        .unwrap();
    assert_eq!(plan.pending().count(), 2, "{plan:?}");
    keel_core::migrate::apply(sql.as_ref(), &plan, "test")
        .await
        .unwrap();
    let plan = keel_core::migrate::plan_with(sql.as_ref(), &[(plugin, v2)], &[])
        .await
        .unwrap();
    let ids: Vec<&str> = plan.pending().map(|m| m.id.as_str()).collect();
    assert_eq!(
        ids,
        ["resource/items_10_qty"],
        "only the new field; changing `required` needs nothing"
    );
}
