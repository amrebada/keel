//! Route rules from `app.yaml`: guards and request validation applied in
//! front of plugin routes.
//!
//! ```yaml
//! routes:
//!   /admin/**: { guard: [authenticated, "role:admin"] }
//!   "POST /notes": { guard: ["rate-limit:20/min"], validate: { body: { type: object, required: [title] } } }
//!   /users/{id}/**: { guard: ["any(role:admin, owner:id)"] }
//! ```
//!
//! Every rule whose pattern matches applies, in file order.

use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use keel_sdk::auth::{bearer, Claims, IdentityProvider, IDENTITY_PROVIDER};
use keel_sdk::authz::{
    guard_interface, Authorizer, Guard, GuardCheck, GuardContext, AUTHORIZER, TENANT_HEADER,
};
use keel_sdk::capabilities::Cache;
use keel_sdk::interfaces::Interfaces;
use keel_sdk::issues::schema_issues;
use keel_sdk::{ApiError, Issue};
use serde_json::{json, Map, Value};

use crate::config::RouteRule;

/// Largest JSON body a `validate.body` rule reads.
const MAX_BODY: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq)]
enum Seg {
    Lit(String),
    Param(String),
    One,
    Rest,
}

#[derive(Debug, Clone)]
struct Pattern {
    method: Option<String>,
    segs: Vec<Seg>,
}

impl Pattern {
    fn parse(key: &str) -> Result<Self, String> {
        let key = key.trim();
        let (method, path) = match key.split_once(' ') {
            Some((m, p)) => (Some(m.to_ascii_uppercase()), p.trim()),
            None => (None, key),
        };
        if let Some(m) = &method {
            if !["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"].contains(&m.as_str()) {
                return Err(format!("`{m}` is not an HTTP method"));
            }
        }
        let path = path
            .strip_prefix('/')
            .ok_or_else(|| format!("`{path}` must start with /"))?;
        let mut segs = Vec::new();
        let parts: Vec<&str> = if path.is_empty() {
            vec![]
        } else {
            path.split('/').collect()
        };
        for (i, s) in parts.iter().enumerate() {
            segs.push(match *s {
                "**" if i + 1 == parts.len() => Seg::Rest,
                "**" => return Err("`**` can only be the last segment".into()),
                "*" => Seg::One,
                s if s.starts_with('{') && s.ends_with('}') && s.len() > 2 => {
                    Seg::Param(s[1..s.len() - 1].into())
                }
                s if s.contains(['{', '}', '*']) => return Err(format!("segment `{s}` is not valid")),
                s => Seg::Lit(s.into()),
            });
        }
        Ok(Self { method, segs })
    }

    fn params(&self) -> impl Iterator<Item = &str> {
        self.segs.iter().filter_map(|s| match s {
            Seg::Param(p) => Some(p.as_str()),
            _ => None,
        })
    }

    fn matches(&self, method: &str, path: &str) -> Option<BTreeMap<String, String>> {
        if let Some(m) = &self.method {
            if !m.eq_ignore_ascii_case(method) {
                return None;
            }
        }
        let path = path.trim_start_matches('/');
        let parts: Vec<&str> = if path.is_empty() {
            vec![]
        } else {
            path.split('/').collect()
        };
        let mut params = BTreeMap::new();
        let mut i = 0;
        for seg in &self.segs {
            match seg {
                Seg::Rest => return Some(params),
                _ if i >= parts.len() => return None,
                Seg::Lit(l) if l != parts[i] => return None,
                Seg::Lit(_) | Seg::One => {}
                Seg::Param(p) => {
                    params.insert(p.clone(), percent_decode(parts[i]));
                }
            }
            i += 1;
        }
        (i == parts.len()).then_some(params)
    }
}

struct Rule {
    key: String,
    pattern: Pattern,
    guards: Vec<Guard>,
    body: Option<jsonschema::Validator>,
    query: Option<(jsonschema::Validator, Value)>,
}

fn compile(routes: &BTreeMap<String, RouteRule>) -> (Vec<Rule>, Vec<Issue>) {
    let mut rules = Vec::new();
    let mut issues = Vec::new();
    for (key, rule) in routes {
        let at = |p: &str| format!("routes[\"{key}\"]{p}");
        let pattern = match Pattern::parse(key) {
            Ok(p) => p,
            Err(e) => {
                issues.push(Issue::new(at(""), e));
                continue;
            }
        };
        let mut guards = Vec::new();
        for (i, g) in rule.guard.iter().enumerate() {
            match Guard::parse(g) {
                Ok(g) => {
                    g.walk(&mut |g| {
                        if let Guard::Owner(p) = g {
                            if !pattern.params().any(|x| x == p) {
                                issues.push(Issue::new(
                                    at(&format!(".guard[{i}]")),
                                    format!("owner:{p} needs `{{{p}}}` in the route pattern"),
                                ));
                            }
                        }
                    });
                    guards.push(g);
                }
                Err(e) => issues.push(Issue::new(at(&format!(".guard[{i}]")), e)),
            }
        }
        let schema = |s: &Value, p: &str, issues: &mut Vec<Issue>| match jsonschema::validator_for(s) {
            Ok(v) => Some(v),
            Err(e) => {
                issues.push(Issue::new(at(p), format!("not a valid JSON Schema: {e}")));
                None
            }
        };
        let v = rule.validate.as_ref();
        let body = v
            .and_then(|v| v.body.as_ref())
            .and_then(|s| schema(s, ".validate.body", &mut issues));
        let query = v
            .and_then(|v| v.query.as_ref())
            .and_then(|s| schema(s, ".validate.query", &mut issues).map(|v| (v, s.clone())));
        rules.push(Rule {
            key: key.clone(),
            pattern,
            guards,
            body,
            query,
        });
    }
    (rules, issues)
}

/// Problems with the `routes:` block that config validation reports.
pub fn check(routes: &BTreeMap<String, RouteRule>) -> Vec<Issue> {
    compile(routes).1
}

/// Guards whose plugin or provider is missing. `has` answers whether an
/// interface (e.g. `authorizer`, `guard:plan`) is offered.
fn needs(rules: &[Rule], has: &dyn Fn(&str) -> bool, has_cache: bool) -> Vec<Issue> {
    let mut issues = Vec::new();
    for r in rules {
        for (i, g) in r.guards.iter().enumerate() {
            let at = format!("routes[\"{}\"].guard[{i}]", r.key);
            g.walk(&mut |g| {
                let need = match g {
                    Guard::Authenticated | Guard::Owner(_) if !has(IDENTITY_PROVIDER) => {
                        Some(format!("`{g}` needs the auth-core plugin (identity-provider)"))
                    }
                    Guard::Role(_) | Guard::Permission(_) if !has(AUTHORIZER) || !has(IDENTITY_PROVIDER) => {
                        Some(format!("`{g}` needs the auth-core and rbac plugins"))
                    }
                    Guard::RateLimit { .. } if !has_cache => {
                        Some(format!("`{g}` needs a cache provider (providers.cache)"))
                    }
                    Guard::Custom { name, .. } if !has(&guard_interface(name)) => Some(format!(
                        "no plugin offers the `{name}` guard; built-in guards are authenticated, role, permission, owner and rate-limit"
                    )),
                    _ => None,
                };
                if let Some(m) = need {
                    issues.push(Issue::new(at.clone(), m));
                }
            });
        }
    }
    issues
}

/// [`needs`] before plugins start, from the interfaces their manifests offer.
pub fn check_needs(routes: &BTreeMap<String, RouteRule>, offered: &[String], has_cache: bool) -> Vec<Issue> {
    let (rules, _) = compile(routes);
    needs(&rules, &|n| offered.iter().any(|x| x == n), has_cache)
}

/// Compiled rules plus what their guards call.
pub struct RouteRules {
    rules: Vec<Rule>,
    identity: Option<Arc<dyn IdentityProvider>>,
    authorizer: Option<Arc<dyn Authorizer>>,
    custom: HashMap<String, Arc<dyn GuardCheck>>,
    cache: Option<Arc<dyn Cache>>,
}

impl RouteRules {
    /// Compiles the rules and checks that every guard has what it needs:
    /// `identity-provider` (auth-core) for user guards, `authorizer` (rbac)
    /// for roles and permissions, a cache for rate limits, and the plugin
    /// that offers each custom guard.
    pub fn build(
        routes: &BTreeMap<String, RouteRule>,
        interfaces: &Interfaces,
        cache: Option<Arc<dyn Cache>>,
    ) -> Result<Self, Vec<Issue>> {
        let (rules, mut issues) = compile(routes);
        let names = interfaces.names();
        issues.extend(needs(&rules, &|n| names.iter().any(|x| x == n), cache.is_some()));
        let identity = interfaces.get::<dyn IdentityProvider>(IDENTITY_PROVIDER);
        let authorizer = interfaces.get::<dyn Authorizer>(AUTHORIZER);
        let mut custom = HashMap::new();
        for r in &rules {
            for g in &r.guards {
                g.walk(&mut |g| {
                    if let Guard::Custom { name, .. } = g {
                        if let Some(c) = interfaces.get::<dyn GuardCheck>(&guard_interface(name)) {
                            custom.insert(name.clone(), c);
                        }
                    }
                });
            }
        }
        if !issues.is_empty() {
            return Err(issues);
        }
        Ok(Self {
            rules,
            identity,
            authorizer,
            custom,
            cache,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    async fn enforce(&self, req: Request) -> Result<Request, ApiError> {
        let method = req.method().as_str().to_string();
        let path = req.uri().path().to_string();
        let matched: Vec<(&Rule, BTreeMap<String, String>)> = self
            .rules
            .iter()
            .filter_map(|r| r.pattern.matches(&method, &path).map(|p| (r, p)))
            .collect();
        if matched.is_empty() {
            return Ok(req);
        }
        let headers = req.headers();
        let token = bearer(headers).map(str::to_string);
        let tenant = headers
            .get(TENANT_HEADER)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let client = headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(',').next())
            .map(|s| s.trim().to_string())
            .or_else(|| {
                req.extensions()
                    .get::<ConnectInfo<SocketAddr>>()
                    .map(|c| c.0.ip().to_string())
            });
        let mut user = UserSlot::new(token);
        for (rule, params) in &matched {
            if rule.guards.is_empty() {
                continue;
            }
            let mut ctx = GuardContext {
                method: method.clone(),
                path: path.clone(),
                user: None,
                tenant: tenant.clone(),
                params: params.clone(),
                client: client.clone(),
            };
            for (i, g) in rule.guards.iter().enumerate() {
                let scope = format!("{}#{i}", rule.key);
                self.eval(g, &scope, &mut ctx, &mut user).await?;
            }
        }
        let mut req = req;
        for (rule, _) in &matched {
            if let Some((v, schema)) = &rule.query {
                let q = query_value(req.uri().query().unwrap_or(""), schema);
                check_value(v, &q, "query")?;
            }
        }
        if matched.iter().any(|(r, _)| r.body.is_some()) {
            let (parts, body) = req.into_parts();
            let bytes = axum::body::to_bytes(body, MAX_BODY).await.map_err(|_| {
                ApiError::new(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "body_too_large",
                    format!("the body is larger than {MAX_BODY} bytes"),
                )
            })?;
            let value: Value = if bytes.iter().all(u8::is_ascii_whitespace) {
                return Err(validation_error(vec![Issue::new(
                    "body",
                    "a JSON body is required",
                )]));
            } else {
                serde_json::from_slice(&bytes).map_err(|e| {
                    ApiError::bad_request("invalid_body", format!("the JSON body is not valid: {e}"))
                })?
            };
            for (r, _) in matched.iter().filter(|(r, _)| r.body.is_some()) {
                check_value(r.body.as_ref().expect("filtered"), &value, "body")?;
            }
            req = Request::from_parts(parts, Body::from(bytes));
        }
        Ok(req)
    }

    fn eval<'a>(
        &'a self,
        g: &'a Guard,
        scope: &'a str,
        ctx: &'a mut GuardContext,
        user: &'a mut UserSlot,
    ) -> Pin<Box<dyn Future<Output = Result<(), ApiError>> + Send + 'a>> {
        Box::pin(async move {
            match g {
                Guard::Authenticated => {
                    self.user(ctx, user).await?;
                    Ok(())
                }
                Guard::Owner(param) => {
                    let c = self.user(ctx, user).await?;
                    if ctx.params.get(param) == Some(&c.sub) {
                        Ok(())
                    } else {
                        Err(ApiError::forbidden("not_owner", "only the owner can do this"))
                    }
                }
                Guard::Role(role) => {
                    let c = self.user(ctx, user).await?;
                    let a = self.authorizer.as_ref().expect("checked at build");
                    if a.has_role(&c.sub, role, ctx.tenant.as_deref()).await? {
                        Ok(())
                    } else {
                        Err(
                            ApiError::forbidden("forbidden", format!("this needs the `{role}` role"))
                                .with("role", role.as_str()),
                        )
                    }
                }
                Guard::Permission(p) => {
                    let c = self.user(ctx, user).await?;
                    let a = self.authorizer.as_ref().expect("checked at build");
                    if a.has_permission(&c.sub, p, ctx.tenant.as_deref()).await? {
                        Ok(())
                    } else {
                        Err(
                            ApiError::forbidden("forbidden", format!("this needs the `{p}` permission"))
                                .with("permission", p.as_str()),
                        )
                    }
                }
                Guard::RateLimit { max, period_secs } => {
                    // A valid token counts per user; otherwise per client address.
                    let who = match self.user(ctx, user).await {
                        Ok(c) => format!("u:{}", c.sub),
                        Err(_) => format!("ip:{}", ctx.client.as_deref().unwrap_or("unknown")),
                    };
                    let cache = self.cache.as_ref().expect("checked at build");
                    let n = cache
                        .incr(
                            &format!("keel:rl:{scope}:{who}"),
                            Duration::from_secs(*period_secs),
                        )
                        .await?;
                    if n as u64 > *max {
                        Err(ApiError::new(
                            StatusCode::TOO_MANY_REQUESTS,
                            "rate_limited",
                            format!("at most {max} requests per {period_secs}s; try again later"),
                        )
                        .with("limit", *max)
                        .with("period_secs", *period_secs))
                    } else {
                        Ok(())
                    }
                }
                Guard::All(list) => {
                    for g in list {
                        self.eval(g, scope, ctx, user).await?;
                    }
                    Ok(())
                }
                Guard::Any(list) => {
                    let mut first = None;
                    for g in list {
                        match self.eval(g, scope, ctx, user).await {
                            Ok(()) => return Ok(()),
                            Err(e) => {
                                first.get_or_insert(e);
                            }
                        }
                    }
                    Err(first.expect("any() has at least one guard"))
                }
                Guard::Custom { name, arg } => {
                    // Plugin guards see the user when a valid token was sent.
                    let _ = self.user(ctx, user).await;
                    self.custom
                        .get(name)
                        .expect("checked at build")
                        .check(ctx, arg)
                        .await
                }
            }
        })
    }

    /// The signed-in user, verifying the token once per request.
    async fn user(&self, ctx: &mut GuardContext, slot: &mut UserSlot) -> Result<Claims, ApiError> {
        if slot.result.is_none() {
            let r = match (&slot.token, &self.identity) {
                (None, _) => Err(ApiError::unauthorized(
                    "missing_token",
                    "send the access token as `Authorization: Bearer <token>`",
                )),
                (Some(_), None) => Err(ApiError::unauthorized(
                    "missing_token",
                    "sign-in is not configured",
                )),
                (Some(t), Some(idp)) => idp.verify_access_token(t).await,
            };
            slot.result = Some(r);
        }
        let r = slot.result.clone().expect("set above");
        if let Ok(c) = &r {
            ctx.user = Some(c.clone());
        }
        r
    }
}

struct UserSlot {
    token: Option<String>,
    result: Option<Result<Claims, ApiError>>,
}

impl UserSlot {
    fn new(token: Option<String>) -> Self {
        Self { token, result: None }
    }
}

/// Axum middleware running [`RouteRules`].
pub async fn middleware(State(rules): State<Arc<RouteRules>>, req: Request, next: Next) -> Response {
    match rules.enforce(req).await {
        Ok(req) => next.run(req).await,
        Err(e) => e.into_response(),
    }
}

fn validation_error(issues: Vec<Issue>) -> ApiError {
    let list: Vec<Value> = issues
        .iter()
        .map(|i| json!({ "path": i.path, "message": i.message }))
        .collect();
    ApiError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        "validation_failed",
        format!("{} problem(s) in the request", list.len()),
    )
    .with("issues", list)
}

fn check_value(v: &jsonschema::Validator, value: &Value, root: &str) -> Result<(), ApiError> {
    let issues: Vec<Issue> = schema_issues(v, value)
        .into_iter()
        .map(|mut i| {
            i.path = if i.path == "(root)" {
                root.to_string()
            } else {
                format!("{root}.{}", i.path)
            };
            i
        })
        .collect();
    if issues.is_empty() {
        Ok(())
    } else {
        Err(validation_error(issues))
    }
}

/// Query string to a JSON object, turning values into numbers or booleans
/// where the schema's property asks for them. Repeated keys become arrays
/// when the property is an array.
fn query_value(q: &str, schema: &Value) -> Value {
    let props = schema.get("properties");
    let kind = |k: &str| -> String {
        props
            .and_then(|p| p.get(k))
            .and_then(|p| p.get("type"))
            .and_then(Value::as_str)
            .unwrap_or("string")
            .to_string()
    };
    let item_kind = |k: &str| -> String {
        props
            .and_then(|p| p.get(k))
            .and_then(|p| p.get("items"))
            .and_then(|p| p.get("type"))
            .and_then(Value::as_str)
            .unwrap_or("string")
            .to_string()
    };
    let coerce = |v: String, t: &str| -> Value {
        match t {
            "integer" => v.parse::<i64>().map(Value::from).unwrap_or(Value::String(v)),
            "number" => v.parse::<f64>().map(Value::from).unwrap_or(Value::String(v)),
            "boolean" => match v.as_str() {
                "true" | "1" => Value::Bool(true),
                "false" | "0" => Value::Bool(false),
                _ => Value::String(v),
            },
            _ => Value::String(v),
        }
    };
    let mut out = Map::new();
    for pair in q.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let (k, v) = (percent_decode(k), percent_decode(v));
        if kind(&k) == "array" {
            let it = coerce(v, &item_kind(&k));
            match out.entry(k).or_insert_with(|| Value::Array(vec![])) {
                Value::Array(a) => a.push(it),
                _ => unreachable!(),
            }
        } else {
            let t = kind(&k);
            out.insert(k, coerce(v, &t));
        }
    }
    Value::Object(out)
}

fn percent_decode(s: &str) -> String {
    let s = s.replace('+', " ");
    urlencoding::decode(&s).map(|c| c.into_owned()).unwrap_or(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns() {
        let p = Pattern::parse("/admin/**").unwrap();
        assert!(p.matches("GET", "/admin").is_some());
        assert!(p.matches("GET", "/admin/users/1").is_some());
        assert!(p.matches("GET", "/administrator").is_none());
        let p = Pattern::parse("POST /users/{id}/notes/*").unwrap();
        assert_eq!(
            p.matches("POST", "/users/u1/notes/n2")
                .unwrap()
                .get("id")
                .map(String::as_str),
            Some("u1")
        );
        assert!(p.matches("GET", "/users/u1/notes/n2").is_none());
        assert!(p.matches("POST", "/users/u1/notes").is_none());
        assert!(Pattern::parse("admin").is_err());
        assert!(Pattern::parse("/a/**/b").is_err());
        assert!(Pattern::parse("FETCH /a").is_err());
    }

    #[test]
    fn query_coercion() {
        let schema = json!({ "properties": { "limit": { "type": "integer" }, "tag": { "type": "array", "items": { "type": "string" } }, "on": { "type": "boolean" } } });
        assert_eq!(
            query_value("limit=10&tag=a&tag=b%20c&on=true&q=x", &schema),
            json!({ "limit": 10, "tag": ["a", "b c"], "on": true, "q": "x" })
        );
    }

    #[test]
    fn config_issues() {
        let routes: BTreeMap<String, RouteRule> = serde_json::from_value(json!({
            "/a/{id}": { "guard": ["owner:id", "owner:user", "role"] },
            "/b": { "validate": { "body": { "type": 5 } } }
        }))
        .unwrap();
        let issues = check(&routes);
        let text: Vec<String> = issues
            .iter()
            .map(|i| format!("{}: {}", i.path, i.message))
            .collect();
        assert_eq!(issues.len(), 3, "{text:#?}");
        assert!(text[0].contains("owner:user needs `{user}`"), "{text:#?}");
    }
}
