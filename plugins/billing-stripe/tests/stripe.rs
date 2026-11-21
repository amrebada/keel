//! billing-stripe against a mock Stripe API: checkout for a team, a signed
//! webhook marking it paid, the plan guard, the portal, and cancellation.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use axum::body::Body;
use axum::extract::Path;
use axum::http::{HeaderMap, Request, StatusCode};
use axum::routing::{get, post};
use http_body_util::BodyExt;
use keel_core::{AppConfig, Host, HostOptions, MapSecrets, PluginRegistry};
use keel_plugin_billing_stripe::sign;
use keel_providers::{ConsoleEmail, DefaultCrypto, ReqwestClient, SqlProvider};
use keel_sdk::capabilities::CapabilityProviders;
use keel_sdk::prelude::*;
use serde_json::{json, Value};
use tower::ServiceExt;

type Calls = Arc<Mutex<Vec<(String, String)>>>;

async fn mock_stripe() -> (String, Calls) {
    let calls: Calls = Arc::default();
    let log = |path: &'static str, calls: Calls, reply: Value| {
        post(move |h: HeaderMap, body: String| {
            let calls = calls.clone();
            let reply = reply.clone();
            async move {
                assert_eq!(h["authorization"], "Bearer sk_test_123");
                calls.lock().unwrap().push((path.to_string(), body));
                Json(reply)
            }
        })
    };
    let app = axum::Router::new()
        .route(
            "/v1/customers",
            log("customers", calls.clone(), json!({ "id": "cus_1" })),
        )
        .route(
            "/v1/checkout/sessions",
            log(
                "checkout",
                calls.clone(),
                json!({ "id": "cs_1", "url": "https://checkout.stripe.test/cs_1" }),
            ),
        )
        .route(
            "/v1/billing_portal/sessions",
            log(
                "portal",
                calls.clone(),
                json!({ "url": "https://billing.stripe.test/p_1" }),
            ),
        )
        .route(
            "/v1/balance",
            get(|| async { Json(json!({ "object": "balance" })) }),
        )
        .route(
            "/v1/prices/{id}",
            get(|Path(id): Path<String>| async move {
                if id != "price_missing" {
                    Ok(Json(json!({ "id": id, "recurring": { "interval": "month" } })))
                } else {
                    Err((
                        StatusCode::NOT_FOUND,
                        Json(json!({ "error": { "message": "No such price" } })),
                    ))
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), calls)
}

struct Demo;

#[async_trait]
impl Plugin for Demo {
    fn manifest(&self) -> &Manifest {
        static M: OnceLock<Manifest> = OnceLock::new();
        M.get_or_init(|| {
            Manifest::from_sources(
                "id: demo\nversion: 0.1.0\nsummary: A paid route.\nrequires:\n  host: \"^0.1\"\n  capabilities: []\nprovides:\n  routes: [\"GET /reports\"]\n",
                None,
            )
            .unwrap()
        })
    }
    async fn init(&self, ctx: &mut InitContext) -> Result<(), PluginError> {
        ctx.routes()
            .get("/reports", || async { Json(json!({ "reports": [] })) })?;
        Ok(())
    }
}

struct T {
    router: axum::Router,
    host: Host,
}

async fn app(api: &str, prices: &str) -> T {
    let mut secrets = HashMap::new();
    secrets.insert("AUTH_JWT_SECRET".to_string(), "k".repeat(40));
    secrets.insert("STRIPE_SECRET_KEY".to_string(), "sk_test_123".to_string());
    secrets.insert("STRIPE_WEBHOOK_SECRET".to_string(), "whsec_test".to_string());
    let providers = CapabilityProviders {
        sql: Some(Arc::new(
            SqlProvider::connect("sqlite::memory:", None).await.unwrap(),
        )),
        crypto: Some(Arc::new(DefaultCrypto)),
        http: Some(Arc::new(ReqwestClient::default())),
        email: Some(Arc::new(ConsoleEmail::default())),
        secrets: Some(Arc::new(secrets.clone())),
        ..Default::default()
    };
    let yaml = format!(
        r#"
app: t
plugins:
  - use: auth-core
    config: {{ require_verified_email: false }}
  - use: rbac
    config:
      roles:
        admin: {{ permissions: ["*"] }}
        owner: {{ permissions: ["billing.manage"] }}
        member: {{ permissions: [] }}
      admins: [boss@example.com]
  - use: billing-stripe
    config:
      api_base: "{api}"
      plans:
        pro: {{ price: {prices} }}
        team: {{ price: price_team, includes: [pro] }}
  - use: demo
routes:
  /reports: {{ guard: [authenticated, "plan:pro"] }}
"#
    );
    let config = AppConfig::load_str(&yaml, "app.yaml", &MapSecrets::default()).unwrap();
    let registry = PluginRegistry::new()
        .register(keel_plugin_auth_core::AuthCore::new())
        .register(keel_plugin_rbac::Rbac::new())
        .register(keel_plugin_billing_stripe::BillingStripe::new())
        .register(Demo);
    let opts = HostOptions {
        secrets: Arc::new(MapSecrets(secrets)),
        state_file: None,
    };
    let host = Host::start_with(config, &registry, &providers, opts)
        .await
        .unwrap();
    T {
        router: host.router(),
        host,
    }
}

impl T {
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
        let res = self
            .router
            .clone()
            .oneshot(req.body(body).unwrap())
            .await
            .unwrap();
        let s = res.status();
        let b = res.into_body().collect().await.unwrap().to_bytes();
        (s, serde_json::from_slice(&b).unwrap_or(Value::Null))
    }

    async fn webhook(&self, event: Value, secret: &str) -> (StatusCode, Value) {
        let payload = event.to_string();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let req = Request::builder()
            .method("POST")
            .uri("/billing/webhook")
            .header("stripe-signature", sign(payload.as_bytes(), secret, now))
            .body(Body::from(payload))
            .unwrap();
        let res = self.router.clone().oneshot(req).await.unwrap();
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
                None,
            )
            .await;
        assert_eq!(s, StatusCode::CREATED, "{b}");
        (
            b["user"]["id"].as_str().unwrap().into(),
            b["login"]["access_token"].as_str().unwrap().into(),
        )
    }
}

#[tokio::test]
async fn team_checkout_webhook_guard_and_cancel() {
    let (api, calls) = mock_stripe().await;
    let t = app(&api, "price_pro").await;
    let (_, boss) = t.user("boss@example.com").await;
    let (mia_id, mia) = t.user("mia@example.com").await;

    // Mia owns team-1.
    let (s, b) = t
        .call(
            "POST",
            &format!("/rbac/users/{mia_id}/roles"),
            json!({ "role": "owner", "tenant": "team-1" }),
            Some(&boss),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");

    // Not paid yet: the plan guard answers 402.
    let (s, b) = t
        .call("GET", "/reports", Value::Null, Some(&mia), Some("team-1"))
        .await;
    assert_eq!(
        (s, b["error"]["code"].as_str()),
        (StatusCode::PAYMENT_REQUIRED, Some("plan_required")),
        "{b}"
    );

    // Only an owner can start checkout for the team.
    let (_, other) = t.user("other@example.com").await;
    let (s, b) = t
        .call(
            "POST",
            "/billing/checkout",
            json!({ "plan": "pro" }),
            Some(&other),
            Some("team-1"),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{b}");
    let (s, b) = t
        .call(
            "POST",
            "/billing/checkout",
            json!({ "plan": "gold" }),
            Some(&mia),
            Some("team-1"),
        )
        .await;
    assert_eq!(
        (s, b["error"]["code"].as_str()),
        (StatusCode::BAD_REQUEST, Some("unknown_plan"))
    );
    let (s, b) = t
        .call(
            "POST",
            "/billing/checkout",
            json!({ "plan": "pro" }),
            Some(&mia),
            Some("team-1"),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["url"], "https://checkout.stripe.test/cs_1");
    {
        let calls = calls.lock().unwrap();
        assert_eq!(calls[0].0, "customers");
        assert!(
            calls[0].1.contains("metadata%5Btenant%5D=team-1"),
            "{}",
            calls[0].1
        );
        assert!(calls[0].1.contains("email=mia%40example.com"), "{}", calls[0].1);
        let form = &calls[1].1;
        assert!(form.contains("mode=subscription"), "{form}");
        assert!(form.contains("customer=cus_1"), "{form}");
        assert!(form.contains("client_reference_id=team-1"), "{form}");
        assert!(form.contains("line_items%5B0%5D%5Bprice%5D=price_pro"), "{form}");
    }

    // A forged webhook is refused.
    let completed = json!({
        "id": "evt_1", "type": "checkout.session.completed",
        "data": { "object": { "id": "cs_1", "mode": "subscription", "customer": "cus_1", "subscription": "sub_1",
            "client_reference_id": "team-1", "payment_status": "paid", "metadata": { "tenant": "team-1", "plan": "pro" } } }
    });
    let (s, _) = t.webhook(completed.clone(), "whsec_wrong").await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, b) = t.webhook(completed.clone(), "whsec_test").await;
    assert_eq!((s, &b["handled"]), (StatusCode::OK, &json!(true)), "{b}");
    let (_, b) = t.webhook(completed, "whsec_test").await;
    assert_eq!(b["duplicate"], true, "events are processed once");

    // Paid: the team passes the guard; another team and Mia's personal tenant do not.
    let (s, _) = t
        .call("GET", "/reports", Value::Null, Some(&mia), Some("team-1"))
        .await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = t.call("GET", "/reports", Value::Null, Some(&mia), None).await;
    assert_eq!(s, StatusCode::PAYMENT_REQUIRED);
    let (_, b) = t
        .call(
            "GET",
            "/billing/subscription",
            Value::Null,
            Some(&mia),
            Some("team-1"),
        )
        .await;
    assert_eq!(
        (b["plan"].as_str(), b["status"].as_str(), b["active"].as_bool()),
        (Some("pro"), Some("active"), Some(true)),
        "{b}"
    );
    let (s, _) = t
        .call(
            "POST",
            "/billing/checkout",
            json!({ "plan": "pro" }),
            Some(&mia),
            Some("team-1"),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT);

    // Portal for the team's customer.
    let (s, b) = t
        .call("POST", "/billing/portal", Value::Null, Some(&mia), Some("team-1"))
        .await;
    assert_eq!(
        (s, b["url"].as_str()),
        (StatusCode::OK, Some("https://billing.stripe.test/p_1"))
    );

    // Upgrade to team (includes pro), then cancel.
    let updated = json!({ "id": "evt_2", "type": "customer.subscription.updated", "data": { "object": {
        "id": "sub_1", "customer": "cus_1", "status": "active", "cancel_at_period_end": false, "metadata": {},
        "items": { "data": [ { "price": { "id": "price_team" }, "current_period_end": 1_900_000_000 } ] } } } });
    t.webhook(updated, "whsec_test").await;
    let (_, b) = t
        .call(
            "GET",
            "/billing/subscription",
            Value::Null,
            Some(&mia),
            Some("team-1"),
        )
        .await;
    assert_eq!(
        (b["plan"].as_str(), b["current_period_end"].as_i64()),
        (Some("team"), Some(1_900_000_000)),
        "{b}"
    );
    let (s, _) = t
        .call("GET", "/reports", Value::Null, Some(&mia), Some("team-1"))
        .await;
    assert_eq!(s, StatusCode::OK, "team includes pro");
    let deleted = json!({ "id": "evt_3", "type": "customer.subscription.deleted", "data": { "object": {
        "id": "sub_1", "customer": "cus_1", "status": "canceled", "metadata": { "tenant": "team-1" },
        "items": { "data": [ { "price": { "id": "price_team" } } ] } } } });
    t.webhook(deleted, "whsec_test").await;
    let (s, _) = t
        .call("GET", "/reports", Value::Null, Some(&mia), Some("team-1"))
        .await;
    assert_eq!(s, StatusCode::PAYMENT_REQUIRED);

    // Setup checks: the key works and the configured price exists.
    let report = t.host.refresh_setup().await.unwrap();
    assert!(report.complete, "{:#?}", report.steps);
}

#[tokio::test]
async fn setup_check_reports_a_missing_price() {
    let (api, _) = mock_stripe().await;
    let t = app(&api, "price_missing").await;
    let report = t.host.refresh_setup().await.unwrap();
    let step = report.find("billing-stripe/stripe-prices").unwrap();
    let reason = step.reason.clone().unwrap_or_default();
    assert!(reason.contains("plan `pro`: price price_missing"), "{reason}");
    let (_, token) = t.user("a@example.com").await;
    let (s, b) = t
        .call(
            "POST",
            "/billing/checkout",
            json!({ "plan": "pro" }),
            Some(&token),
            None,
        )
        .await;
    assert_eq!(
        (s, b["error"]["code"].as_str()),
        (StatusCode::SERVICE_UNAVAILABLE, Some("setup_incomplete"))
    );
}
