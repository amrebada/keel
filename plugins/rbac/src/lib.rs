//! `rbac`: roles from YAML with inheritance, per-user permission grants and
//! tenant-scoped roles. Offers the `authorizer` interface the host's
//! `role:x` and `permission:x` guards use.
//!
//! A role or grant has a tenant: `""` means global (applies everywhere),
//! anything else applies only to requests that send that `X-Tenant-Id`.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Bytes;
use axum::extract::{Path, Query};
use axum::http::{HeaderMap, StatusCode};
use keel_sdk::auth::{bearer, IdentityProvider, IDENTITY_PROVIDER};
use keel_sdk::authz::{Authorizer, AUTHORIZER, TENANT_HEADER};
use keel_sdk::capabilities::{Log, Sql};
use keel_sdk::prelude::*;
use keel_sdk::Emitter;
use serde::Deserialize;

#[derive(Debug, Clone, Default)]
struct Role {
    permissions: Vec<String>,
    inherits: Vec<String>,
}

#[derive(Debug, Clone)]
struct Settings {
    roles: BTreeMap<String, Role>,
    default_role: Option<String>,
    admins: Vec<String>,
    admin_role: String,
    manage: String,
}

impl Settings {
    fn from_config(c: &Value) -> Result<Self, String> {
        let roles: BTreeMap<String, Role> = match c.get("roles").and_then(Value::as_object) {
            Some(m) => m
                .iter()
                .map(|(k, v)| {
                    let list = |f: &str| -> Vec<String> {
                        v.get(f)
                            .and_then(Value::as_array)
                            .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
                            .unwrap_or_default()
                    };
                    (
                        k.clone(),
                        Role {
                            permissions: list("permissions"),
                            inherits: list("inherits"),
                        },
                    )
                })
                .collect(),
            None => BTreeMap::from([
                (
                    "admin".to_string(),
                    Role {
                        permissions: vec!["*".into()],
                        inherits: vec![],
                    },
                ),
                ("member".to_string(), Role::default()),
            ]),
        };
        let s = Settings {
            default_role: match c.get("default_role") {
                Some(Value::Null) => None,
                Some(v) => v.as_str().map(String::from),
                None => Some("member".into()),
            },
            admins: c
                .get("admins")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(|e| e.trim().to_ascii_lowercase()))
                        .collect()
                })
                .unwrap_or_default(),
            admin_role: c
                .get("admin_role")
                .and_then(Value::as_str)
                .unwrap_or("admin")
                .into(),
            manage: c
                .get("manage_permission")
                .and_then(Value::as_str)
                .unwrap_or("rbac.manage")
                .into(),
            roles,
        };
        for (name, r) in &s.roles {
            for i in &r.inherits {
                if !s.roles.contains_key(i) {
                    return Err(format!(
                        "role `{name}` inherits `{i}`, which is not defined under roles"
                    ));
                }
            }
        }
        for (what, r) in [
            ("default_role", s.default_role.as_deref()),
            ("admin_role", Some(&s.admin_role)),
        ] {
            if let Some(r) = r {
                if !s.roles.contains_key(r) {
                    return Err(format!("{what} `{r}` is not defined under roles"));
                }
            }
        }
        for name in s.roles.keys() {
            s.expand(name)?;
        }
        Ok(s)
    }

    /// The role and every role it inherits; errors on a cycle.
    fn expand(&self, role: &str) -> Result<BTreeSet<String>, String> {
        let mut out = BTreeSet::new();
        let mut path = Vec::new();
        self.walk(role, &mut out, &mut path)?;
        Ok(out)
    }

    fn walk(&self, role: &str, out: &mut BTreeSet<String>, path: &mut Vec<String>) -> Result<(), String> {
        if path.iter().any(|p| p == role) {
            path.push(role.into());
            return Err(format!("roles inherit in a cycle: {}", path.join(" -> ")));
        }
        if !out.insert(role.into()) {
            return Ok(());
        }
        path.push(role.into());
        if let Some(r) = self.roles.get(role) {
            for i in &r.inherits {
                self.walk(i, out, path)?;
            }
        }
        path.pop();
        Ok(())
    }
}

/// `*` grants all, `posts.*` grants `posts.<anything>`.
fn grants(granted: &str, wanted: &str) -> bool {
    granted == "*"
        || granted == wanted
        || granted
            .strip_suffix(".*")
            .is_some_and(|p| wanted.strip_prefix(p).is_some_and(|r| r.starts_with('.')))
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

pub struct Service {
    settings: Settings,
    sql: Arc<dyn Sql>,
    log: Arc<dyn Log>,
    identity: Arc<dyn IdentityProvider>,
    events: Emitter,
    ns: String,
}

impl Service {
    fn t(&self, name: &str) -> String {
        format!("{}{name}", self.ns)
    }

    /// (role, tenant) rows for a user that apply to `tenant`.
    async fn assigned(&self, user_id: &str, tenant: Option<&str>) -> Result<Vec<(String, String)>, ApiError> {
        let rows = self
            .sql
            .query(
                &format!(
                    "SELECT role, tenant FROM {} WHERE user_id = ? AND (tenant = '' OR tenant = ?)",
                    self.t("assignments")
                ),
                &[json!(user_id), json!(tenant.unwrap_or(""))],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|r| (str_of(r, "role"), str_of(r, "tenant")))
            .collect())
    }

    async fn grants_of(&self, user_id: &str, tenant: Option<&str>) -> Result<Vec<String>, ApiError> {
        let rows = self
            .sql
            .query(
                &format!(
                    "SELECT permission FROM {} WHERE user_id = ? AND (tenant = '' OR tenant = ?)",
                    self.t("grants")
                ),
                &[json!(user_id), json!(tenant.unwrap_or(""))],
            )
            .await?;
        Ok(rows.iter().map(|r| str_of(r, "permission")).collect())
    }

    async fn effective_roles(
        &self,
        user_id: &str,
        tenant: Option<&str>,
    ) -> Result<BTreeSet<String>, ApiError> {
        let mut out = BTreeSet::new();
        for (role, _) in self.assigned(user_id, tenant).await? {
            // A role removed from config no longer counts.
            if let Ok(set) = self.settings.expand(&role) {
                if self.settings.roles.contains_key(&role) {
                    out.extend(set);
                }
            }
        }
        Ok(out)
    }

    async fn permissions(&self, user_id: &str, tenant: Option<&str>) -> Result<BTreeSet<String>, ApiError> {
        let mut out: BTreeSet<String> = self.grants_of(user_id, tenant).await?.into_iter().collect();
        for r in self.effective_roles(user_id, tenant).await? {
            if let Some(role) = self.settings.roles.get(&r) {
                out.extend(role.permissions.iter().cloned());
            }
        }
        Ok(out)
    }

    async fn assign(&self, user_id: &str, role: &str, tenant: &str) -> Result<bool, ApiError> {
        if !self.settings.roles.contains_key(role) {
            return Err(ApiError::bad_request(
                "unknown_role",
                format!("`{role}` is not a role; see GET /rbac/roles"),
            ));
        }
        let exists = self
            .sql
            .query(
                &format!(
                    "SELECT role FROM {} WHERE user_id = ? AND role = ? AND tenant = ?",
                    self.t("assignments")
                ),
                &[json!(user_id), json!(role), json!(tenant)],
            )
            .await?;
        if !exists.is_empty() {
            return Ok(false);
        }
        self.sql
            .execute(
                &format!(
                    "INSERT INTO {} (user_id, role, tenant, created_at) VALUES (?, ?, ?, ?)",
                    self.t("assignments")
                ),
                &[json!(user_id), json!(role), json!(tenant), json!(now())],
            )
            .await?;
        self.events
            .emit(
                "rbac.role.assigned",
                json!({ "user_id": user_id, "role": role, "tenant": tenant_out(tenant) }),
            )
            .await
            .ok();
        Ok(true)
    }

    /// The caller, who must hold the manage permission in `tenant` (or globally).
    async fn manager(&self, headers: &HeaderMap, tenant: &str) -> Result<String, ApiError> {
        let user = self.caller(headers).await?;
        let t = (!tenant.is_empty()).then_some(tenant);
        if !self.has_perm(&user, &self.settings.manage, t).await? {
            return Err(ApiError::forbidden(
                "forbidden",
                format!("changing roles needs the `{}` permission", self.settings.manage),
            ));
        }
        Ok(user)
    }

    async fn caller(&self, headers: &HeaderMap) -> Result<String, ApiError> {
        let token = bearer(headers).ok_or_else(|| {
            ApiError::unauthorized(
                "missing_token",
                "send the access token as `Authorization: Bearer <token>`",
            )
        })?;
        Ok(self.identity.verify_access_token(token).await?.sub)
    }

    async fn has_perm(
        &self,
        user_id: &str,
        permission: &str,
        tenant: Option<&str>,
    ) -> Result<bool, ApiError> {
        Ok(self
            .permissions(user_id, tenant)
            .await?
            .iter()
            .any(|g| grants(g, permission)))
    }

    async fn user_view(&self, user_id: &str) -> Result<Value, ApiError> {
        let roles = self
            .sql
            .query(
                &format!(
                    "SELECT role, tenant FROM {} WHERE user_id = ? ORDER BY tenant, role",
                    self.t("assignments")
                ),
                &[json!(user_id)],
            )
            .await?;
        let grants = self
            .sql
            .query(
                &format!(
                    "SELECT permission, tenant FROM {} WHERE user_id = ? ORDER BY tenant, permission",
                    self.t("grants")
                ),
                &[json!(user_id)],
            )
            .await?;
        Ok(json!({
            "user_id": user_id,
            "roles": roles.iter().map(|r| json!({ "role": str_of(r, "role"), "tenant": tenant_out(&str_of(r, "tenant")) })).collect::<Vec<_>>(),
            "permissions": grants.iter().map(|r| json!({ "permission": str_of(r, "permission"), "tenant": tenant_out(&str_of(r, "tenant")) })).collect::<Vec<_>>(),
        }))
    }

    async fn on_user_created(&self, user_id: &str, email: Option<&str>) {
        let mut roles = Vec::new();
        if let Some(r) = &self.settings.default_role {
            roles.push(r.clone());
        }
        if email.is_some_and(|e| self.settings.admins.contains(&e.to_ascii_lowercase())) {
            roles.push(self.settings.admin_role.clone());
        }
        for r in roles {
            if let Err(e) = self.assign(user_id, &r, "").await {
                self.log
                    .error(&format!("could not give role {r} to new user {user_id}: {e}"));
            }
        }
    }

    /// Gives `admins` that already have an account their role.
    async fn bootstrap_admins(&self) {
        for email in &self.settings.admins {
            match self.identity.user_by_email(email).await {
                Ok(Some(u)) => {
                    if let Ok(true) = self.assign(&u.id, &self.settings.admin_role, "").await {
                        self.log
                            .info(&format!("gave {email} the {} role", self.settings.admin_role));
                    }
                }
                Ok(None) => {}
                Err(e) => self.log.warn(&format!("could not look up admin {email}: {e}")),
            }
        }
    }
}

fn str_of(r: &serde_json::Map<String, Value>, k: &str) -> String {
    r.get(k).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn tenant_out(t: &str) -> Value {
    if t.is_empty() {
        Value::Null
    } else {
        json!(t)
    }
}

fn tenant_of(h: &HeaderMap) -> Option<String> {
    h.get(TENANT_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[async_trait]
impl Authorizer for Service {
    async fn roles(&self, user_id: &str, tenant: Option<&str>) -> Result<Vec<String>, ApiError> {
        Ok(self.effective_roles(user_id, tenant).await?.into_iter().collect())
    }
    async fn has_role(&self, user_id: &str, role: &str, tenant: Option<&str>) -> Result<bool, ApiError> {
        Ok(self.effective_roles(user_id, tenant).await?.contains(role))
    }
    async fn has_permission(
        &self,
        user_id: &str,
        permission: &str,
        tenant: Option<&str>,
    ) -> Result<bool, ApiError> {
        self.has_perm(user_id, permission, tenant).await
    }
    async fn assign_role(&self, user_id: &str, role: &str, tenant: Option<&str>) -> Result<(), ApiError> {
        self.assign(user_id, role, tenant.unwrap_or("")).await.map(|_| ())
    }
}

#[derive(Default)]
pub struct Rbac {
    service: keel_sdk::Current<Service>,
}

impl Rbac {
    pub fn new() -> Self {
        Self::default()
    }
}

#[derive(Deserialize)]
struct RoleBody {
    role: String,
    tenant: Option<String>,
}

#[derive(Deserialize)]
struct PermBody {
    permission: String,
    tenant: Option<String>,
}

#[derive(Deserialize, Default)]
struct TenantQuery {
    tenant: Option<String>,
}

fn body<T: serde::de::DeserializeOwned>(b: &Bytes) -> Result<T, ApiError> {
    serde_json::from_slice(b)
        .map_err(|e| ApiError::bad_request("invalid_body", format!("the JSON body is not valid: {e}")))
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

const MIGRATION: &str = "CREATE TABLE {ns}assignments (
    user_id VARCHAR(36) NOT NULL,
    role VARCHAR(64) NOT NULL,
    tenant VARCHAR(128) NOT NULL,
    created_at BIGINT NOT NULL,
    PRIMARY KEY (user_id, role, tenant)
);
CREATE INDEX {ns}assignments_tenant ON {ns}assignments (tenant);
CREATE TABLE {ns}grants (
    user_id VARCHAR(36) NOT NULL,
    permission VARCHAR(128) NOT NULL,
    tenant VARCHAR(128) NOT NULL,
    created_at BIGINT NOT NULL,
    PRIMARY KEY (user_id, permission, tenant)
)";

#[async_trait]
impl Plugin for Rbac {
    fn manifest(&self) -> &Manifest {
        keel_sdk::manifest!()
    }

    fn migrations(&self, _dialect: &str) -> Vec<Migration> {
        vec![Migration::new("001_init", MIGRATION)]
    }

    async fn init(&self, ctx: &mut InitContext) -> Result<(), PluginError> {
        let settings = Settings::from_config(&ctx.config).map_err(PluginError::Other)?;
        let caps = &ctx.capabilities;
        let svc = Arc::new(Service {
            settings,
            sql: caps.sql()?,
            log: caps.log()?,
            identity: ctx.interface::<dyn IdentityProvider>(IDENTITY_PROVIDER)?,
            events: ctx.emitter(),
            ns: caps.namespace().to_string(),
        });
        let _ = self.service.set(svc.clone());
        ctx.provide::<dyn Authorizer>(AUTHORIZER, svc.clone())?;
        svc.bootstrap_admins().await;

        let r = ctx.routes();
        r.get(
            "/rbac/roles",
            handler!(svc, |s, h: HeaderMap| {
                s.caller(&h).await?;
                let roles: BTreeMap<&String, Value> = s
                    .settings
                    .roles
                    .iter()
                    .map(|(k, r)| (k, json!({ "permissions": r.permissions, "inherits": r.inherits })))
                    .collect();
                ok(json!({ "roles": roles, "default_role": s.settings.default_role }))
            }),
        )?;
        r.get(
            "/rbac/me",
            handler!(svc, |s, h: HeaderMap| {
                let user = s.caller(&h).await?;
                let tenant = tenant_of(&h);
                ok(json!({
                    "user_id": user,
                    "tenant": tenant,
                    "roles": s.effective_roles(&user, tenant.as_deref()).await?,
                    "permissions": s.permissions(&user, tenant.as_deref()).await?,
                }))
            }),
        )?;
        r.get(
            "/rbac/users/{id}",
            handler!(svc, |s, h: HeaderMap, id: Path<String>| {
                let tenant = tenant_of(&h).unwrap_or_default();
                s.manager(&h, &tenant).await?;
                ok(s.user_view(&id).await?)
            }),
        )?;
        r.post(
            "/rbac/users/{id}/roles",
            handler!(svc, |s, h: HeaderMap, id: Path<String>, b: Bytes| {
                let b: RoleBody = body(&b)?;
                let tenant = b.tenant.unwrap_or_default();
                s.manager(&h, &tenant).await?;
                if s.identity.user(&id).await?.is_none() {
                    return Err(ApiError::not_found("unknown_user", "no user with this id"));
                }
                let added = s.assign(&id, &b.role, &tenant).await?;
                Ok::<_, ApiError>((
                    if added {
                        StatusCode::CREATED
                    } else {
                        StatusCode::OK
                    },
                    Json(s.user_view(&id).await?),
                ))
            }),
        )?;
        r.delete(
            "/rbac/users/{id}/roles/{role}",
            handler!(svc, |s,
                           h: HeaderMap,
                           p: Path<(String, String)>,
                           q: Query<TenantQuery>| {
                let (id, role) = p.0;
                let tenant = q.0.tenant.unwrap_or_default();
                s.manager(&h, &tenant).await?;
                let n = s
                    .sql
                    .execute(
                        &format!(
                            "DELETE FROM {} WHERE user_id = ? AND role = ? AND tenant = ?",
                            s.t("assignments")
                        ),
                        &[json!(id), json!(role), json!(tenant)],
                    )
                    .await?;
                if n > 0 {
                    s.events
                        .emit(
                            "rbac.role.removed",
                            json!({ "user_id": id, "role": role, "tenant": tenant_out(&tenant) }),
                        )
                        .await
                        .ok();
                }
                ok(s.user_view(&id).await?)
            }),
        )?;
        r.post(
            "/rbac/users/{id}/permissions",
            handler!(svc, |s, h: HeaderMap, id: Path<String>, b: Bytes| {
                let b: PermBody = body(&b)?;
                let tenant = b.tenant.unwrap_or_default();
                s.manager(&h, &tenant).await?;
                if b.permission.trim().is_empty() {
                    return Err(ApiError::bad_request("invalid_permission", "permission is empty"));
                }
                if s.identity.user(&id).await?.is_none() {
                    return Err(ApiError::not_found("unknown_user", "no user with this id"));
                }
                let t = s.t("grants");
                let exists = s
                    .sql
                    .query(
                        &format!("SELECT permission FROM {t} WHERE user_id = ? AND permission = ? AND tenant = ?"),
                        &[json!(*id), json!(b.permission), json!(tenant)],
                    )
                    .await?;
                if exists.is_empty() {
                    s.sql
                        .execute(
                            &format!("INSERT INTO {t} (user_id, permission, tenant, created_at) VALUES (?, ?, ?, ?)"),
                            &[json!(*id), json!(b.permission), json!(tenant), json!(now())],
                        )
                        .await?;
                }
                Ok::<_, ApiError>((
                    if exists.is_empty() { StatusCode::CREATED } else { StatusCode::OK },
                    Json(s.user_view(&id).await?),
                ))
            }),
        )?;
        r.delete(
            "/rbac/users/{id}/permissions/{permission}",
            handler!(svc, |s,
                           h: HeaderMap,
                           p: Path<(String, String)>,
                           q: Query<TenantQuery>| {
                let (id, perm) = p.0;
                let tenant = q.0.tenant.unwrap_or_default();
                s.manager(&h, &tenant).await?;
                s.sql
                    .execute(
                        &format!(
                            "DELETE FROM {} WHERE user_id = ? AND permission = ? AND tenant = ?",
                            s.t("grants")
                        ),
                        &[json!(id), json!(perm), json!(tenant)],
                    )
                    .await?;
                ok(s.user_view(&id).await?)
            }),
        )?;
        Ok(())
    }

    async fn on_event(&self, event: &Event) {
        if event.name != "auth.user.created" {
            return;
        }
        if let (Some(s), Some(id)) = (
            self.service.get(),
            event.payload.get("user_id").and_then(Value::as_str),
        ) {
            s.on_user_created(id, event.payload.get("email").and_then(Value::as_str))
                .await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcards() {
        assert!(grants("*", "a.b"));
        assert!(grants("posts.*", "posts.write"));
        assert!(grants("posts.*", "posts.comments.delete"));
        assert!(!grants("posts.*", "postsx.write"));
        assert!(!grants("posts.*", "posts"));
        assert!(grants("posts.read", "posts.read"));
    }

    #[test]
    fn inheritance_and_cycles() {
        let s = Settings::from_config(&json!({ "roles": {
            "admin": { "inherits": ["editor"] }, "editor": { "inherits": ["member"], "permissions": ["posts.*"] }, "member": {}
        }}))
        .unwrap();
        assert_eq!(
            s.expand("admin").unwrap().into_iter().collect::<Vec<_>>(),
            ["admin", "editor", "member"]
        );
        let e = Settings::from_config(&json!({ "roles": { "a": { "inherits": ["b"] }, "b": { "inherits": ["a"] } }, "default_role": null, "admin_role": "a" }))
            .unwrap_err();
        assert!(e.contains("cycle"), "{e}");
        let e = Settings::from_config(&json!({ "roles": { "a": {} } })).unwrap_err();
        assert!(e.contains("default_role `member`"), "{e}");
    }
}
