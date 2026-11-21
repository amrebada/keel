//! `billing-stripe`: one Stripe subscription per tenant.
//!
//! The tenant is the `X-Tenant-Id` header (a team), or the signed-in user's
//! own id when the header is absent. Checkout and the customer portal are
//! Stripe-hosted pages; signed webhooks keep the subscription state here,
//! and the `plan:<name>` guard reads it.

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Bytes;
use axum::http::{HeaderMap, StatusCode};
use hmac::{Hmac, Mac};
use keel_sdk::auth::{bearer, IdentityProvider, IDENTITY_PROVIDER};
use keel_sdk::authz::{Authorizer, GuardCheck, GuardContext, AUTHORIZER, TENANT_HEADER};
use keel_sdk::capabilities::{HttpClient, HttpRequest, Log, Secrets, Sql};
use keel_sdk::interfaces::Interfaces;
use keel_sdk::prelude::*;
use keel_sdk::Emitter;
use serde::Deserialize;
use sha2::Sha256;

pub const SECRET_KEY: &str = "STRIPE_SECRET_KEY";
pub const WEBHOOK_SECRET: &str = "STRIPE_WEBHOOK_SECRET";

/// Webhooks older than this are refused (replay protection), as Stripe's SDKs do.
const TOLERANCE_SECS: i64 = 300;

#[derive(Debug, Clone)]
struct Plan {
    price: String,
    name: Option<String>,
    includes: Vec<String>,
}

#[derive(Debug, Clone)]
struct Settings {
    plans: BTreeMap<String, Plan>,
    success_url: String,
    cancel_url: String,
    portal_return_url: String,
    manage: String,
    api_base: String,
}

impl Settings {
    fn from_config(c: &Value) -> Result<Self, String> {
        let s = |k: &str, d: &str| c.get(k).and_then(Value::as_str).unwrap_or(d).to_string();
        let plans: BTreeMap<String, Plan> = c
            .get("plans")
            .and_then(Value::as_object)
            .ok_or("config.plans is required")?
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    Plan {
                        price: v.get("price").and_then(Value::as_str).unwrap_or_default().into(),
                        name: v.get("name").and_then(Value::as_str).map(String::from),
                        includes: v
                            .get("includes")
                            .and_then(Value::as_array)
                            .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
                            .unwrap_or_default(),
                    },
                )
            })
            .collect();
        for (k, p) in &plans {
            for i in &p.includes {
                if !plans.contains_key(i) {
                    return Err(format!("plan `{k}` includes `{i}`, which is not a plan"));
                }
            }
        }
        Ok(Self {
            plans,
            success_url: s("success_url", "http://localhost:3000/billing/success"),
            cancel_url: s("cancel_url", "http://localhost:3000/billing"),
            portal_return_url: s("portal_return_url", "http://localhost:3000/billing"),
            manage: s("manage_permission", "billing.manage"),
            api_base: s("api_base", "https://api.stripe.com")
                .trim_end_matches('/')
                .to_string(),
        })
    }

    fn plan_for_price(&self, price: &str) -> Option<&str> {
        self.plans
            .iter()
            .find(|(_, p)| p.price == price)
            .map(|(k, _)| k.as_str())
    }

    /// Whether having `plan` satisfies a guard for `wanted`.
    fn satisfies(&self, plan: &str, wanted: &str) -> bool {
        if plan == wanted {
            return true;
        }
        let mut seen = vec![plan.to_string()];
        let mut i = 0;
        while i < seen.len() {
            if let Some(p) = self.plans.get(&seen[i]) {
                for inc in &p.includes {
                    if inc == wanted {
                        return true;
                    }
                    if !seen.contains(inc) {
                        seen.push(inc.clone());
                    }
                }
            }
            i += 1;
        }
        false
    }
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

/// Checks a `Stripe-Signature` header (`t=...,v1=...`) for `payload`.
pub fn verify_signature(header: &str, payload: &[u8], secret: &str, now: i64) -> Result<(), String> {
    let mut t = None;
    let mut sigs = Vec::new();
    for part in header.split(',') {
        match part.trim().split_once('=') {
            Some(("t", v)) => t = v.parse::<i64>().ok(),
            Some(("v1", v)) => sigs.push(v.to_string()),
            _ => {}
        }
    }
    let t = t.ok_or("the signature has no timestamp")?;
    if (now - t).abs() > TOLERANCE_SECS {
        return Err("the signature timestamp is too old".into());
    }
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("any key length");
    mac.update(format!("{t}.").as_bytes());
    mac.update(payload);
    let want = mac.finalize().into_bytes();
    let want: String = want.iter().map(|b| format!("{b:02x}")).collect();
    // Constant-time compare of equal-length hex strings.
    let ok = sigs.iter().any(|s| {
        s.len() == want.len() && s.bytes().zip(want.bytes()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
    });
    if ok {
        Ok(())
    } else {
        Err("the signature does not match".into())
    }
}

/// `Stripe-Signature` value for `payload`, for tests and `stripe trigger`-like tools.
pub fn sign(payload: &[u8], secret: &str, t: i64) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("any key length");
    mac.update(format!("{t}.").as_bytes());
    mac.update(payload);
    let hex: String = mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("t={t},v1={hex}")
}

pub struct Service {
    settings: Settings,
    sql: Arc<dyn Sql>,
    http: Arc<dyn HttpClient>,
    secrets: Arc<dyn Secrets>,
    log: Arc<dyn Log>,
    identity: Arc<dyn IdentityProvider>,
    interfaces: Interfaces,
    events: Emitter,
    ns: String,
}

/// A subscription change from a webhook.
struct Update<'a> {
    tenant: &'a str,
    customer: Option<&'a str>,
    subscription: Option<&'a str>,
    plan: Option<&'a str>,
    status: &'a str,
    period_end: Option<i64>,
    cancel_at_end: bool,
}

/// A tenant's subscription as stored.
#[derive(Debug, Clone)]
struct Sub {
    plan: Option<String>,
    status: String,
    current_period_end: Option<i64>,
    cancel_at_period_end: bool,
    customer_id: Option<String>,
}

impl Sub {
    fn active(&self) -> bool {
        matches!(self.status.as_str(), "active" | "trialing")
    }
}

impl Service {
    fn t(&self, n: &str) -> String {
        format!("{}{n}", self.ns)
    }

    fn key(&self) -> Result<String, ApiError> {
        self.secrets
            .get(SECRET_KEY)
            .filter(|k| !k.trim().is_empty())
            .ok_or_else(|| {
                ApiError::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "setup_incomplete",
                    format!("{SECRET_KEY} is not set; run `keel setup status`"),
                )
            })
    }

    /// A form-encoded call to the Stripe API.
    async fn stripe(&self, method: &str, path: &str, form: &[(String, String)]) -> Result<Value, ApiError> {
        let body = form
            .iter()
            .map(|(k, v)| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v)))
            .collect::<Vec<_>>()
            .join("&");
        let res = self
            .http
            .send(HttpRequest {
                method: method.into(),
                url: format!("{}{path}", self.settings.api_base),
                headers: vec![
                    ("authorization".into(), format!("Bearer {}", self.key()?)),
                    ("content-type".into(), "application/x-www-form-urlencoded".into()),
                ],
                body: body.into_bytes(),
            })
            .await
            .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, "stripe_unreachable", e.to_string()))?;
        let v: Value = serde_json::from_slice(&res.body).unwrap_or(Value::Null);
        if !(200..300).contains(&res.status) {
            let msg = v["error"]["message"]
                .as_str()
                .unwrap_or("request failed")
                .to_string();
            self.log
                .warn(&format!("stripe {method} {path}: HTTP {}: {msg}", res.status));
            return Err(
                ApiError::new(StatusCode::BAD_GATEWAY, "stripe_error", format!("Stripe: {msg}"))
                    .with("stripe_status", res.status),
            );
        }
        Ok(v)
    }

    async fn sub(&self, tenant: &str) -> Result<Option<Sub>, ApiError> {
        let rows = self
            .sql
            .query(
                &format!(
                    "SELECT s.plan, s.status, s.current_period_end, s.cancel_at_period_end, c.customer_id
                     FROM {} c LEFT JOIN {} s ON s.tenant = c.tenant WHERE c.tenant = ?",
                    self.t("customers"),
                    self.t("subscriptions")
                ),
                &[json!(tenant)],
            )
            .await?;
        Ok(rows.first().map(|r| Sub {
            plan: r.get("plan").and_then(Value::as_str).map(String::from),
            status: r
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("none")
                .to_string(),
            current_period_end: r.get("current_period_end").and_then(Value::as_i64),
            cancel_at_period_end: r.get("cancel_at_period_end").and_then(Value::as_i64) == Some(1),
            customer_id: r.get("customer_id").and_then(Value::as_str).map(String::from),
        }))
    }

    async fn customer_for(&self, tenant: &str, email: Option<&str>) -> Result<String, ApiError> {
        if let Some(c) = self.sub(tenant).await?.and_then(|s| s.customer_id) {
            return Ok(c);
        }
        let mut form = vec![("metadata[tenant]".to_string(), tenant.to_string())];
        if let Some(e) = email {
            form.push(("email".into(), e.into()));
        }
        let c = self.stripe("POST", "/v1/customers", &form).await?;
        let id = c["id"]
            .as_str()
            .ok_or_else(|| ApiError::internal("Stripe returned no customer id"))?
            .to_string();
        self.sql
            .execute(
                &format!(
                    "INSERT INTO {} (tenant, customer_id, created_at) VALUES (?, ?, ?)",
                    self.t("customers")
                ),
                &[json!(tenant), json!(id), json!(now())],
            )
            .await?;
        Ok(id)
    }

    /// (user id, tenant) for a billing request; checks the caller may manage it.
    async fn tenant_for(&self, headers: &HeaderMap, manage: bool) -> Result<(String, String), ApiError> {
        let token = bearer(headers).ok_or_else(|| {
            ApiError::unauthorized(
                "missing_token",
                "send the access token as `Authorization: Bearer <token>`",
            )
        })?;
        let user = self.identity.verify_access_token(token).await?.sub;
        let tenant = headers
            .get(TENANT_HEADER)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| user.clone());
        if tenant != user {
            let allowed = match self.interfaces.get::<dyn Authorizer>(AUTHORIZER) {
                // Reading the subscription also works with `billing.read`.
                Some(a) => {
                    a.has_permission(&user, &self.settings.manage, Some(&tenant))
                        .await?
                        || (!manage && a.has_permission(&user, "billing.read", Some(&tenant)).await?)
                }
                None => false,
            };
            if !allowed {
                return Err(ApiError::forbidden(
                    "forbidden",
                    format!(
                        "billing for tenant `{tenant}` needs the `{}` permission there (rbac plugin)",
                        self.settings.manage
                    ),
                ));
            }
        }
        Ok((user, tenant))
    }

    async fn upsert(&self, u: Update<'_>) -> Result<(), ApiError> {
        let Update {
            tenant,
            customer,
            subscription,
            plan,
            status,
            period_end,
            cancel_at_end,
        } = u;
        if let Some(c) = customer {
            let have = self
                .sql
                .query(
                    &format!("SELECT tenant FROM {} WHERE tenant = ?", self.t("customers")),
                    &[json!(tenant)],
                )
                .await?;
            if have.is_empty() {
                self.sql
                    .execute(
                        &format!(
                            "INSERT INTO {} (tenant, customer_id, created_at) VALUES (?, ?, ?)",
                            self.t("customers")
                        ),
                        &[json!(tenant), json!(c), json!(now())],
                    )
                    .await?;
            }
        }
        let t = self.t("subscriptions");
        let exists = !self
            .sql
            .query(
                &format!("SELECT tenant FROM {t} WHERE tenant = ?"),
                &[json!(tenant)],
            )
            .await?
            .is_empty();
        let params = [
            json!(subscription.unwrap_or("")),
            json!(plan.unwrap_or("")),
            json!(status),
            json!(period_end.unwrap_or(0)),
            json!(i64::from(cancel_at_end)),
            json!(now()),
            json!(tenant),
        ];
        if exists {
            self.sql
                .execute(
                    &format!(
                        "UPDATE {t} SET subscription_id = ?, plan = ?, status = ?, current_period_end = ?, cancel_at_period_end = ?, updated_at = ? WHERE tenant = ?"
                    ),
                    &params,
                )
                .await?;
        } else {
            self.sql
                .execute(
                    &format!(
                        "INSERT INTO {t} (subscription_id, plan, status, current_period_end, cancel_at_period_end, updated_at, tenant) VALUES (?, ?, ?, ?, ?, ?, ?)"
                    ),
                    &params,
                )
                .await?;
        }
        self.events
            .emit(
                "billing.subscription.updated",
                json!({ "tenant": tenant, "plan": plan, "status": status }),
            )
            .await
            .ok();
        self.log.info(&format!(
            "tenant {tenant}: subscription {status} ({})",
            plan.unwrap_or("-")
        ));
        Ok(())
    }

    async fn tenant_of_customer(&self, customer: &str) -> Result<Option<String>, ApiError> {
        Ok(self
            .sql
            .query(
                &format!("SELECT tenant FROM {} WHERE customer_id = ?", self.t("customers")),
                &[json!(customer)],
            )
            .await?
            .first()
            .and_then(|r| r.get("tenant").and_then(Value::as_str).map(String::from)))
    }

    async fn webhook(&self, headers: &HeaderMap, payload: &[u8]) -> Result<Value, ApiError> {
        let secret = self
            .secrets
            .get(WEBHOOK_SECRET)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                ApiError::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "setup_incomplete",
                    format!("{WEBHOOK_SECRET} is not set"),
                )
            })?;
        let sig = headers
            .get("stripe-signature")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| ApiError::bad_request("bad_signature", "missing Stripe-Signature header"))?;
        verify_signature(sig, payload, &secret, now())
            .map_err(|e| ApiError::bad_request("bad_signature", e))?;
        let event: Value = serde_json::from_slice(payload)
            .map_err(|e| ApiError::bad_request("invalid_body", format!("not JSON: {e}")))?;
        let id = event["id"].as_str().unwrap_or_default();
        let kind = event["type"].as_str().unwrap_or_default();
        let seen = self
            .sql
            .query(
                &format!("SELECT id FROM {} WHERE id = ?", self.t("events")),
                &[json!(id)],
            )
            .await?;
        if !seen.is_empty() {
            return Ok(json!({ "received": true, "duplicate": true }));
        }
        let obj = &event["data"]["object"];
        let handled = match kind {
            "checkout.session.completed" => {
                let tenant = obj["client_reference_id"]
                    .as_str()
                    .or_else(|| obj["metadata"]["tenant"].as_str())
                    .map(String::from);
                let paid = matches!(
                    obj["payment_status"].as_str(),
                    Some("paid" | "no_payment_required")
                );
                match tenant {
                    Some(tenant) if obj["mode"] == "subscription" => {
                        self.upsert(Update {
                            tenant: &tenant,
                            customer: obj["customer"].as_str(),
                            subscription: obj["subscription"].as_str(),
                            plan: obj["metadata"]["plan"].as_str(),
                            status: if paid { "active" } else { "incomplete" },
                            period_end: None,
                            cancel_at_end: false,
                        })
                        .await?;
                        true
                    }
                    _ => false,
                }
            }
            "customer.subscription.created"
            | "customer.subscription.updated"
            | "customer.subscription.deleted" => {
                let customer = obj["customer"].as_str().unwrap_or_default();
                let tenant = match obj["metadata"]["tenant"].as_str() {
                    Some(t) => Some(t.to_string()),
                    None => self.tenant_of_customer(customer).await?,
                };
                let item = &obj["items"]["data"][0];
                let price = item["price"]["id"].as_str().unwrap_or_default();
                let plan = self
                    .settings
                    .plan_for_price(price)
                    .map(String::from)
                    .or_else(|| obj["metadata"]["plan"].as_str().map(String::from));
                let status = if kind.ends_with("deleted") {
                    "canceled"
                } else {
                    obj["status"].as_str().unwrap_or("incomplete")
                };
                // Newer API versions put the period on the item.
                let period_end = obj["current_period_end"]
                    .as_i64()
                    .or_else(|| item["current_period_end"].as_i64());
                match tenant {
                    Some(tenant) => {
                        self.upsert(Update {
                            tenant: &tenant,
                            customer: Some(customer),
                            subscription: obj["id"].as_str(),
                            plan: plan.as_deref(),
                            status,
                            period_end,
                            cancel_at_end: obj["cancel_at_period_end"].as_bool().unwrap_or(false),
                        })
                        .await?;
                        true
                    }
                    None => {
                        self.log
                            .warn(&format!("{kind} for unknown customer {customer}; ignored"));
                        false
                    }
                }
            }
            _ => false,
        };
        self.sql
            .execute(
                &format!(
                    "INSERT INTO {} (id, type, received_at) VALUES (?, ?, ?)",
                    self.t("events")
                ),
                &[json!(id), json!(kind), json!(now())],
            )
            .await?;
        Ok(json!({ "received": true, "handled": handled }))
    }

    fn sub_json(&self, tenant: &str, s: Option<&Sub>) -> Value {
        match s {
            Some(s) => json!({
                "tenant": tenant,
                "plan": s.plan.as_deref().filter(|p| !p.is_empty()),
                "status": s.status,
                "active": s.active(),
                "current_period_end": s.current_period_end.filter(|t| *t > 0),
                "cancel_at_period_end": s.cancel_at_period_end,
            }),
            None => json!({ "tenant": tenant, "plan": null, "status": "none", "active": false }),
        }
    }
}

/// The `plan:<name>` guard: the request's tenant has an active subscription
/// to that plan (or one that includes it). `plan:any` accepts any plan.
struct PlanGuard(Arc<Service>);

#[async_trait]
impl GuardCheck for PlanGuard {
    async fn check(&self, ctx: &GuardContext, wanted: &str) -> Result<(), ApiError> {
        let tenant = match (&ctx.tenant, &ctx.user) {
            (Some(t), _) => t.clone(),
            (None, Some(u)) => u.sub.clone(),
            (None, None) => {
                return Err(ApiError::unauthorized(
                    "missing_token",
                    "sign in (or send X-Tenant-Id) so the plan can be checked",
                ))
            }
        };
        let sub = self.0.sub(&tenant).await?;
        let ok = sub.as_ref().is_some_and(|s| {
            s.active()
                && s.plan.as_deref().is_some_and(|p| {
                    wanted == "any" || wanted.split('|').any(|w| self.0.settings.satisfies(p, w))
                })
        });
        if ok {
            Ok(())
        } else {
            Err(ApiError::new(
                StatusCode::PAYMENT_REQUIRED,
                "plan_required",
                format!("this needs an active `{wanted}` plan; start one with POST /billing/checkout"),
            )
            .with("plan", wanted)
            .with("tenant", tenant))
        }
    }
}

#[derive(Default)]
pub struct BillingStripe {
    service: OnceLock<Arc<Service>>,
}

impl BillingStripe {
    pub fn new() -> Self {
        Self::default()
    }
}

#[derive(Deserialize)]
struct CheckoutBody {
    plan: String,
}

type Reply = Result<(StatusCode, Json<Value>), ApiError>;

fn ok(v: Value) -> Reply {
    Ok((StatusCode::OK, Json(v)))
}

macro_rules! handler {
    ($svc:expr, |$s:ident, $($arg:ident : $t:ty),*| $body:expr) => {{
        let svc = $svc.clone();
        move |$($arg: $t),*| {
            let $s = svc.clone();
            async move { $body }
        }
    }};
}

const MIGRATION: &str = "CREATE TABLE {ns}customers (
    tenant VARCHAR(128) NOT NULL PRIMARY KEY,
    customer_id VARCHAR(255) NOT NULL,
    created_at BIGINT NOT NULL
);
CREATE UNIQUE INDEX {ns}customers_customer ON {ns}customers (customer_id);
CREATE TABLE {ns}subscriptions (
    tenant VARCHAR(128) NOT NULL PRIMARY KEY,
    subscription_id VARCHAR(255) NOT NULL,
    plan VARCHAR(64) NOT NULL,
    status VARCHAR(32) NOT NULL,
    current_period_end BIGINT NOT NULL,
    cancel_at_period_end BIGINT NOT NULL,
    updated_at BIGINT NOT NULL
);
CREATE TABLE {ns}events (
    id VARCHAR(255) NOT NULL PRIMARY KEY,
    type VARCHAR(128) NOT NULL,
    received_at BIGINT NOT NULL
)";

#[async_trait]
impl Plugin for BillingStripe {
    fn manifest(&self) -> &Manifest {
        keel_sdk::manifest!()
    }

    fn migrations(&self, _dialect: &str) -> Vec<Migration> {
        vec![Migration::new("001_init", MIGRATION)]
    }

    async fn check_setup(&self, step_id: &str) -> Result<(), String> {
        let Some(s) = self.service.get() else {
            return Ok(());
        };
        match step_id {
            "stripe-account" => s
                .stripe("GET", "/v1/balance", &[])
                .await
                .map(|_| ())
                .map_err(|e| e.message),
            "stripe-prices" => {
                if s.key().is_err() {
                    return Err(format!("set {SECRET_KEY} first"));
                }
                for (name, p) in &s.settings.plans {
                    let v = s
                        .stripe(
                            "GET",
                            &format!("/v1/prices/{}", urlencoding::encode(&p.price)),
                            &[],
                        )
                        .await
                        .map_err(|e| format!("plan `{name}`: price {}: {}", p.price, e.message))?;
                    if v["recurring"].is_null() {
                        return Err(format!("plan `{name}`: price {} is not recurring", p.price));
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    async fn init(&self, ctx: &mut InitContext) -> Result<(), PluginError> {
        let settings = Settings::from_config(&ctx.config).map_err(PluginError::Other)?;
        let caps = &ctx.capabilities;
        let svc = Arc::new(Service {
            settings,
            sql: caps.sql()?,
            http: caps.http_client()?,
            secrets: caps.secrets()?,
            log: caps.log()?,
            identity: ctx.interface::<dyn IdentityProvider>(IDENTITY_PROVIDER)?,
            interfaces: ctx.interfaces(),
            events: ctx.emitter(),
            ns: caps.namespace().to_string(),
        });
        let _ = self.service.set(svc.clone());
        ctx.provide::<dyn GuardCheck>("guard:plan", Arc::new(PlanGuard(svc.clone())))?;

        let r = ctx.routes();
        r.get(
            "/billing/plans",
            handler!(svc, |s, _h: HeaderMap| {
                let plans: Vec<Value> = s
                    .settings
                    .plans
                    .iter()
                    .map(|(k, p)| json!({ "plan": k, "name": p.name.clone().unwrap_or_else(|| k.clone()), "price": p.price, "includes": p.includes }))
                    .collect();
                ok(json!({ "plans": plans }))
            }),
        )?;
        r.get(
            "/billing/subscription",
            handler!(svc, |s, h: HeaderMap| {
                let (_, tenant) = s.tenant_for(&h, false).await?;
                let sub = s.sub(&tenant).await?;
                ok(s.sub_json(&tenant, sub.as_ref()))
            }),
        )?;
        r.post(
            "/billing/checkout",
            handler!(svc, |s, h: HeaderMap, b: Bytes| {
                let b: CheckoutBody = serde_json::from_slice(&b)
                    .map_err(|e| ApiError::bad_request("invalid_body", format!("send {{\"plan\": \"<name>\"}}: {e}")))?;
                let plan = s.settings.plans.get(&b.plan).ok_or_else(|| {
                    ApiError::bad_request("unknown_plan", format!("`{}` is not a plan; see GET /billing/plans", b.plan))
                })?;
                let (user, tenant) = s.tenant_for(&h, true).await?;
                if let Some(cur) = s.sub(&tenant).await? {
                    if cur.active() {
                        return Err(ApiError::conflict(
                            "already_subscribed",
                            "this tenant already has an active subscription; change it in the portal (POST /billing/portal)",
                        ));
                    }
                }
                let email = s.identity.user(&user).await?.and_then(|u| u.email);
                let customer = s.customer_for(&tenant, email.as_deref()).await?;
                let form: Vec<(String, String)> = [
                    ("mode", "subscription".to_string()),
                    ("customer", customer),
                    ("client_reference_id", tenant.clone()),
                    ("line_items[0][price]", plan.price.clone()),
                    ("line_items[0][quantity]", "1".into()),
                    ("success_url", s.settings.success_url.clone()),
                    ("cancel_url", s.settings.cancel_url.clone()),
                    ("metadata[tenant]", tenant.clone()),
                    ("metadata[plan]", b.plan.clone()),
                    ("subscription_data[metadata][tenant]", tenant.clone()),
                    ("subscription_data[metadata][plan]", b.plan.clone()),
                ]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect();
                let session = s.stripe("POST", "/v1/checkout/sessions", &form).await?;
                ok(json!({ "id": session["id"], "url": session["url"], "tenant": tenant, "plan": b.plan }))
            }),
        )?;
        r.post(
            "/billing/portal",
            handler!(svc, |s, h: HeaderMap| {
                let (_, tenant) = s.tenant_for(&h, true).await?;
                let customer = s.sub(&tenant).await?.and_then(|x| x.customer_id).ok_or_else(|| {
                    ApiError::not_found(
                        "no_customer",
                        "this tenant has no billing account yet; start with POST /billing/checkout",
                    )
                })?;
                let form = vec![
                    ("customer".to_string(), customer),
                    ("return_url".to_string(), s.settings.portal_return_url.clone()),
                ];
                let session = s.stripe("POST", "/v1/billing_portal/sessions", &form).await?;
                ok(json!({ "url": session["url"] }))
            }),
        )?;
        r.post(
            "/billing/webhook",
            handler!(svc, |s, h: HeaderMap, b: Bytes| ok(s.webhook(&h, &b).await?)),
        )?;
        Ok(())
    }

    async fn health(&self) -> Health {
        match self.service.get().map(|s| s.key()) {
            Some(Err(_)) => Health::Degraded {
                reason: format!("{SECRET_KEY} is not set"),
            },
            _ => Health::Ok,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signatures() {
        let body = br#"{"id":"evt_1"}"#;
        let h = sign(body, "whsec_x", 1000);
        assert!(verify_signature(&h, body, "whsec_x", 1100).is_ok());
        assert!(verify_signature(&h, body, "whsec_y", 1100).is_err());
        assert!(verify_signature(&h, b"{}", "whsec_x", 1100).is_err());
        assert!(verify_signature(&h, body, "whsec_x", 2000)
            .unwrap_err()
            .contains("too old"));
        assert!(verify_signature("v1=abc", body, "whsec_x", 1000).is_err());
    }

    #[test]
    fn plan_includes() {
        let s = Settings::from_config(&json!({ "plans": {
            "starter": { "price": "price_1" }, "pro": { "price": "price_2", "includes": ["starter"] },
            "team": { "price": "price_3", "includes": ["pro"] }
        }}))
        .unwrap();
        assert!(s.satisfies("team", "starter"));
        assert!(s.satisfies("pro", "pro"));
        assert!(!s.satisfies("starter", "pro"));
        assert_eq!(s.plan_for_price("price_2"), Some("pro"));
    }
}
