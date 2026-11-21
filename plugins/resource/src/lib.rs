//! `resource`: REST CRUD endpoints and tables built from YAML models.
//!
//! Each entry under `config.resources` becomes a table `resource_<name>` and
//! six routes under `/api`. Fields map to columns; the plugin writes one
//! migration per table, system column and field, so adding a field to the
//! YAML adds a migration instead of editing an applied one.
//!
//! Protect the routes with the host's `routes:` rules, e.g.
//! `"POST /api/projects": { guard: ["permission:projects.write"] }`.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::body::Bytes;
use axum::extract::{Path, RawQuery};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use keel_sdk::auth::{bearer, IdentityProvider, IDENTITY_PROVIDER};
use keel_sdk::authz::{Authorizer, AUTHORIZER, TENANT_HEADER};
use keel_sdk::capabilities::{Cache, Log, Sql};
use keel_sdk::interfaces::Interfaces;
use keel_sdk::issues::schema_issues;
use keel_sdk::prelude::*;
use keel_sdk::Emitter;
use serde_json::Map;
use sha2::{Digest, Sha256};

/// Column names a field may not use: system columns and SQL keywords.
const RESERVED: &[&str] = &[
    "id",
    "created_at",
    "updated_at",
    "owner_id",
    "tenant",
    "order",
    "group",
    "select",
    "from",
    "where",
    "user",
    "desc",
    "asc",
    "limit",
    "offset",
    "sort",
    "table",
    "index",
    "key",
    "default",
    "check",
    "references",
    "primary",
    "column",
    "drop",
    "alter",
    "change",
    "rename",
    "modify",
    "and",
    "or",
    "not",
    "null",
    "in",
    "is",
    "like",
    "by",
    "as",
    "on",
    "into",
    "values",
    "set",
    "update",
    "delete",
    "insert",
    "create",
    "unique",
    "join",
    "having",
    "case",
    "when",
    "then",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    String,
    Text,
    Integer,
    Number,
    Boolean,
    Date,
    Datetime,
    Json,
}

impl Kind {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "string" => Kind::String,
            "text" => Kind::Text,
            "integer" => Kind::Integer,
            "number" => Kind::Number,
            "boolean" => Kind::Boolean,
            "date" => Kind::Date,
            "datetime" => Kind::Datetime,
            "json" => Kind::Json,
            _ => return None,
        })
    }

    /// The column type. It depends only on the kind, so changing a field's
    /// rules never changes its migration.
    fn column(self) -> &'static str {
        match self {
            Kind::String => "VARCHAR(255)",
            Kind::Text | Kind::Json => "TEXT",
            Kind::Integer | Kind::Boolean => "BIGINT",
            Kind::Number => "DOUBLE PRECISION",
            Kind::Date | Kind::Datetime => "VARCHAR(40)",
        }
    }

    fn textual(self) -> bool {
        matches!(self, Kind::String | Kind::Text | Kind::Date | Kind::Datetime)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    None,
    Owner,
    Tenant,
}

#[derive(Debug, Clone)]
struct Field {
    name: String,
    kind: Kind,
    required: bool,
    default: Option<Value>,
    unique: bool,
    filter: bool,
    sort: bool,
    spec: Value,
}

/// Who may call a resource's routes. Deny by default: without `access:`
/// every route needs a permission.
#[derive(Debug, Clone, PartialEq, Eq)]
enum AccessMode {
    /// `<name>.read` to list and get, `<name>.write` to create and update,
    /// `<name>.delete` to delete (names configurable); needs auth-core and rbac.
    Permissions {
        read: String,
        write: String,
        delete: String,
    },
    /// Any signed-in user.
    Authenticated,
    /// Anyone; only `routes:` rules in app.yaml restrict it.
    Public,
}

#[derive(Debug, Clone, Copy)]
enum Op {
    Read,
    Write,
    Delete,
}

#[derive(Debug, Clone)]
struct Model {
    name: String,
    fields: Vec<Field>,
    scope: Scope,
    access: AccessMode,
    manage: String,
    page_size: u64,
    max_page_size: u64,
    cache_ttl: u64,
}

impl Model {
    /// Columns usable in filters and sorting: (name, kind, filter, sort).
    fn columns(&self) -> Vec<(String, Kind, bool, bool)> {
        let mut out = vec![
            ("id".to_string(), Kind::String, true, true),
            ("created_at".to_string(), Kind::Integer, true, true),
            ("updated_at".to_string(), Kind::Integer, true, true),
        ];
        if self.scope == Scope::Owner {
            out.push(("owner_id".into(), Kind::String, true, false));
        }
        out.extend(
            self.fields
                .iter()
                .map(|f| (f.name.clone(), f.kind, f.filter, f.sort)),
        );
        out
    }

    fn migrations(&self) -> Vec<Migration> {
        let n = &self.name;
        let t = format!("{{ns}}{n}");
        let mut out = vec![Migration::new(
            format!("{n}_00_table"),
            format!(
                "CREATE TABLE {t} (id VARCHAR(36) NOT NULL PRIMARY KEY, created_at BIGINT NOT NULL, updated_at BIGINT NOT NULL);\n\
                 CREATE INDEX {t}_created ON {t} (created_at)"
            ),
        )];
        match self.scope {
            Scope::Owner => out.push(Migration::new(
                format!("{n}_01_owner"),
                format!("ALTER TABLE {t} ADD COLUMN owner_id VARCHAR(36);\nCREATE INDEX {t}_owner ON {t} (owner_id)"),
            )),
            Scope::Tenant => out.push(Migration::new(
                format!("{n}_02_tenant"),
                format!("ALTER TABLE {t} ADD COLUMN tenant VARCHAR(128);\nCREATE INDEX {t}_tenant ON {t} (tenant)"),
            )),
            Scope::None => {}
        }
        for f in &self.fields {
            out.push(Migration::new(
                format!("{n}_10_{}", f.name),
                format!("ALTER TABLE {t} ADD COLUMN {} {}", f.name, f.kind.column()),
            ));
            if f.unique {
                let unique_cols = match self.scope {
                    Scope::Tenant => format!("tenant, {}", f.name),
                    _ => f.name.clone(),
                };
                out.push(Migration::new(
                    format!("{n}_20_{}_unique", f.name),
                    format!("CREATE UNIQUE INDEX {t}_{}_uq ON {t} ({unique_cols})", f.name),
                ));
            }
        }
        out
    }

    /// JSON Schema of one field's value.
    fn property(f: &Field, nullable: bool) -> Value {
        let s = &f.spec;
        let mut p = Map::new();
        let ty = match f.kind {
            Kind::String | Kind::Text | Kind::Date | Kind::Datetime => Some("string"),
            Kind::Integer => Some("integer"),
            Kind::Number => Some("number"),
            Kind::Boolean => Some("boolean"),
            Kind::Json => None,
        };
        if let Some(ty) = ty {
            // Optional fields also take null; keeping one schema (not anyOf)
            // keeps error messages specific.
            p.insert(
                "type".into(),
                if nullable { json!([ty, "null"]) } else { json!(ty) },
            );
        }
        match f.kind {
            Kind::String => {
                p.insert(
                    "maxLength".into(),
                    s.get("max_length").cloned().unwrap_or(json!(255)),
                );
            }
            Kind::Text => {
                if let Some(v) = s.get("max_length") {
                    p.insert("maxLength".into(), v.clone());
                }
            }
            Kind::Date => {
                p.insert("pattern".into(), json!(r"^\d{4}-\d{2}-\d{2}$"));
            }
            Kind::Datetime => {
                p.insert(
                    "pattern".into(),
                    json!(r"^\d{4}-\d{2}-\d{2}[Tt ]\d{2}:\d{2}(:\d{2}(\.\d+)?)?([Zz]|[+-]\d{2}:\d{2})?$"),
                );
            }
            _ => {}
        }
        for (from, to) in [
            ("min", "minimum"),
            ("max", "maximum"),
            ("min_length", "minLength"),
            ("enum", "enum"),
        ] {
            if let Some(v) = s.get(from) {
                let mut v = v.clone();
                if let (true, Value::Array(a)) = (nullable && to == "enum", &mut v) {
                    a.push(Value::Null);
                }
                p.insert(to.into(), v);
            }
        }
        let mut schema = Value::Object(p);
        if let Some(pat) = s.get("pattern") {
            // pattern only applies to strings, so null still passes.
            schema = json!({ "allOf": [schema, { "pattern": pat }] });
        }
        schema
    }

    fn body_schema(&self, create: bool) -> Value {
        let props: Map<String, Value> = self
            .fields
            .iter()
            .map(|f| (f.name.clone(), Self::property(f, !f.required)))
            .collect();
        let required: Vec<&str> = if create {
            self.fields
                .iter()
                .filter(|f| f.required && f.default.is_none())
                .map(|f| f.name.as_str())
                .collect()
        } else {
            vec![]
        };
        json!({ "type": "object", "additionalProperties": false, "properties": props, "required": required })
    }
}

fn parse_models(config: &Value) -> Result<Vec<Model>, String> {
    let resources = config
        .get("resources")
        .and_then(Value::as_object)
        .ok_or("config.resources is required: a map of resource name to its fields")?;
    let mut out = Vec::new();
    for (name, m) in resources {
        if !valid_name(name) {
            return Err(format!(
                "resource `{name}`: use lowercase letters, digits and _ (start with a letter, at most 40)"
            ));
        }
        let fields_cfg = m
            .get("fields")
            .and_then(Value::as_object)
            .filter(|f| !f.is_empty())
            .ok_or(format!("resource `{name}` needs at least one field under fields"))?;
        let mut fields = Vec::new();
        for (fname, spec) in fields_cfg {
            if !valid_name(fname) {
                return Err(format!(
                    "resources.{name}.fields.{fname}: use lowercase letters, digits and _ (start with a letter)"
                ));
            }
            if RESERVED.contains(&fname.as_str()) {
                return Err(format!(
                    "resources.{name}.fields.{fname}: `{fname}` is reserved (system column or SQL keyword); pick another name"
                ));
            }
            let kind = spec
                .get("type")
                .and_then(Value::as_str)
                .and_then(Kind::parse)
                .ok_or(format!(
                    "resources.{name}.fields.{fname}.type must be one of string, text, integer, number, boolean, date, datetime, json"
                ))?;
            let flag = |k: &str, d: bool| spec.get(k).and_then(Value::as_bool).unwrap_or(d);
            let searchable = !matches!(kind, Kind::Text | Kind::Json);
            if flag("unique", false) && matches!(kind, Kind::Text | Kind::Json) {
                return Err(format!(
                    "resources.{name}.fields.{fname}: text and json fields cannot be unique; use string"
                ));
            }
            fields.push(Field {
                name: fname.clone(),
                kind,
                required: flag("required", false),
                default: spec.get("default").cloned(),
                unique: flag("unique", false),
                filter: flag("filter", searchable),
                sort: flag("sort", searchable) && kind != Kind::Json,
                spec: spec.clone(),
            });
        }
        let scope = match m.get("scope").and_then(Value::as_str).unwrap_or("none") {
            "none" => Scope::None,
            "owner" => Scope::Owner,
            "tenant" => Scope::Tenant,
            s => {
                return Err(format!(
                    "resources.{name}.scope `{s}` must be none, owner or tenant"
                ))
            }
        };
        let perm = |k: &str| {
            m.get("permissions")
                .and_then(|p| p.get(k))
                .and_then(Value::as_str)
                .map(String::from)
                .unwrap_or(format!("{name}.{k}"))
        };
        let access = match m.get("access").and_then(Value::as_str).unwrap_or("permissions") {
            "permissions" => AccessMode::Permissions {
                read: perm("read"),
                write: perm("write"),
                delete: perm("delete"),
            },
            "authenticated" => AccessMode::Authenticated,
            "public" => AccessMode::Public,
            a => {
                return Err(format!(
                    "resources.{name}.access `{a}` must be permissions, authenticated or public"
                ))
            }
        };
        if access == AccessMode::Public && scope == Scope::Owner {
            return Err(format!(
                "resources.{name}: scope owner needs a signed-in user; use access: authenticated or permissions"
            ));
        }
        let num = |k: &str, d: u64| m.get(k).and_then(Value::as_u64).unwrap_or(d);
        let model = Model {
            name: name.clone(),
            fields,
            scope,
            access,
            manage: m
                .get("manage_permission")
                .and_then(Value::as_str)
                .map(String::from)
                .unwrap_or(format!("{name}.manage")),
            page_size: num("page_size", 20).max(1),
            max_page_size: num("max_page_size", 100).max(1),
            cache_ttl: num("cache_ttl", 0),
        };
        if model.page_size > model.max_page_size {
            return Err(format!(
                "resources.{name}: page_size ({}) is larger than max_page_size ({})",
                model.page_size, model.max_page_size
            ));
        }
        for f in &model.fields {
            let schema = Model::property(f, false);
            let v = jsonschema::validator_for(&schema)
                .map_err(|e| format!("resources.{name}.fields.{}: {e}", f.name))?;
            if let Some(d) = &f.default {
                if let Some(i) = schema_issues(&v, d).first() {
                    return Err(format!(
                        "resources.{name}.fields.{}.default does not fit the field: {}",
                        f.name, i.message
                    ));
                }
            }
        }
        out.push(model);
    }
    Ok(out)
}

fn valid_name(s: &str) -> bool {
    let mut c = s.chars();
    matches!(c.next(), Some('a'..='z'))
        && s.len() <= 40
        && c.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

/// A JSON value as the column stores it. `Null` means "write a literal NULL".
fn to_db(kind: Kind, v: &Value) -> Value {
    match (kind, v) {
        (_, Value::Null) => Value::Null,
        (Kind::Boolean, Value::Bool(b)) => json!(*b as i64),
        (Kind::Number, Value::Number(n)) => json!(n.as_f64().unwrap_or_default()),
        (Kind::Json, v) => json!(v.to_string()),
        (_, v) => v.clone(),
    }
}

fn from_db(kind: Kind, v: Value) -> Value {
    match (kind, v) {
        (_, Value::Null) => Value::Null,
        (Kind::Boolean, v) => json!(v.as_i64().map(|n| n != 0).or(v.as_bool()).unwrap_or(false)),
        (Kind::Json, Value::String(s)) => serde_json::from_str(&s).unwrap_or(Value::String(s)),
        (Kind::Number, v) => v.as_f64().map(|f| json!(f)).unwrap_or(v),
        (_, v) => v,
    }
}

/// A query-string filter value as the column stores it.
fn coerce(kind: Kind, field: &str, raw: &str) -> Result<Value, ApiError> {
    let bad = |what: &str| {
        ApiError::bad_request(
            "invalid_filter",
            format!("`{field}` is {what}, so `{raw}` does not fit"),
        )
    };
    Ok(match kind {
        Kind::Integer => json!(raw.parse::<i64>().map_err(|_| bad("an integer"))?),
        Kind::Number => json!(raw.parse::<f64>().map_err(|_| bad("a number"))?),
        Kind::Boolean => match raw {
            "true" | "1" => json!(1),
            "false" | "0" => json!(0),
            _ => return Err(bad("a boolean (true or false)")),
        },
        _ => json!(raw),
    })
}

fn parse_query(q: &str) -> Vec<(String, String)> {
    q.split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            let d = |s: &str| {
                let s = s.replace('+', " ");
                urlencoding::decode(&s).map(|c| c.into_owned()).unwrap_or(s)
            };
            (d(k), d(v))
        })
        .collect()
}

/// Who is asking, as far as the model's scope cares.
#[derive(Debug, Clone)]
enum Access {
    All,
    Owner(String),
    Tenant(String),
}

impl Access {
    fn key(&self) -> String {
        match self {
            Access::All => "all".into(),
            Access::Owner(u) => format!("u:{u}"),
            Access::Tenant(t) => format!("t:{t}"),
        }
    }

    /// WHERE condition for the scope, with its parameter.
    fn condition(&self) -> Option<(&'static str, Value)> {
        match self {
            Access::All => None,
            Access::Owner(u) => Some(("owner_id = ?", json!(u))),
            Access::Tenant(t) => Some(("tenant = ?", json!(t))),
        }
    }
}

struct Built {
    model: Model,
    create: jsonschema::Validator,
    update: jsonschema::Validator,
    create_schema: Value,
}

pub struct Service {
    models: BTreeMap<String, Built>,
    sql: Arc<dyn Sql>,
    cache: Arc<dyn Cache>,
    log: Arc<dyn Log>,
    events: Emitter,
    interfaces: Interfaces,
    ns: String,
}

type Reply = Result<Response, ApiError>;

fn json_reply(status: StatusCode, v: &Value) -> Reply {
    Ok((status, Json(v.clone())).into_response())
}

fn cached_reply(bytes: Vec<u8>) -> Reply {
    Ok((
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        bytes,
    )
        .into_response())
}

fn leap(y: u32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

/// Checks a `YYYY-MM-DD` prefix is a real calendar day, and for datetimes
/// that the time and offset are in range. The schema already checked the
/// shape, so parsing by position is safe.
fn real_date(kind: Kind, s: &str) -> Result<(), String> {
    let n = |r: std::ops::Range<usize>| s.get(r).and_then(|x| x.parse::<u32>().ok()).unwrap_or(0);
    let (y, mo, d) = (n(0..4), n(5..7), n(8..10));
    if !(1..=12).contains(&mo) {
        return Err(format!("`{s}` has month {mo}; months go from 01 to 12"));
    }
    let days = match mo {
        2 if leap(y) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    if !(1..=days).contains(&d) {
        return Err(format!("`{s}` has day {d}; {y}-{mo:02} has {days} days"));
    }
    if kind == Kind::Datetime {
        let (h, mi) = (n(11..13), n(14..16));
        let sec = if s.get(16..17) == Some(":") { n(17..19) } else { 0 };
        if h > 23 || mi > 59 || sec > 59 {
            return Err(format!("`{s}` is not a valid time of day"));
        }
        if let Some(i) = s.get(19..).and_then(|t| t.find(['+', '-']).map(|i| i + 19)) {
            if n(i + 1..i + 3) > 23 || n(i + 4..i + 6) > 59 {
                return Err(format!("`{s}` has an offset out of range"));
            }
        }
    }
    Ok(())
}

fn check_dates(m: &Model, input: &Map<String, Value>) -> Result<(), ApiError> {
    let issues: Vec<Value> = m
        .fields
        .iter()
        .filter(|f| matches!(f.kind, Kind::Date | Kind::Datetime))
        .filter_map(|f| {
            let v = input.get(&f.name)?.as_str()?;
            real_date(f.kind, v)
                .err()
                .map(|msg| json!({ "path": format!("body.{}", f.name), "message": msg }))
        })
        .collect();
    if issues.is_empty() {
        return Ok(());
    }
    Err(ApiError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        "validation_failed",
        format!("{} problem(s) in the request", issues.len()),
    )
    .with("issues", issues))
}

fn validate(v: &jsonschema::Validator, body: &Value) -> Result<(), ApiError> {
    let issues: Vec<Value> = schema_issues(v, body)
        .into_iter()
        .map(|i| {
            let path = if i.path == "(root)" {
                "body".to_string()
            } else {
                format!("body.{}", i.path)
            };
            json!({ "path": path, "message": i.message })
        })
        .collect();
    if issues.is_empty() {
        return Ok(());
    }
    Err(ApiError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        "validation_failed",
        format!("{} problem(s) in the request", issues.len()),
    )
    .with("issues", issues))
}

impl Service {
    fn model(&self, name: &str) -> Result<&Built, ApiError> {
        self.models.get(name).ok_or_else(|| {
            ApiError::not_found(
                "unknown_resource",
                format!(
                    "no resource `{name}`; this app has: {}",
                    self.models.keys().cloned().collect::<Vec<_>>().join(", ")
                ),
            )
        })
    }

    fn table(&self, m: &Model) -> String {
        format!("{}{}", self.ns, m.name)
    }

    /// The signed-in user's id from the bearer token.
    async fn user_of(&self, m: &Model, h: &HeaderMap) -> Result<String, ApiError> {
        let identity = self
            .interfaces
            .get::<dyn IdentityProvider>(IDENTITY_PROVIDER)
            .ok_or_else(|| {
                ApiError::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "needs_auth",
                    format!(
                        "`{}` needs signed-in users, which needs the auth-core plugin (or set access: public)",
                        m.name
                    ),
                )
            })?;
        let token = bearer(h).ok_or_else(|| {
            ApiError::unauthorized(
                "missing_token",
                "send the access token as `Authorization: Bearer <token>`",
            )
        })?;
        Ok(identity.verify_access_token(token).await?.sub)
    }

    /// Enforces the resource's `access` mode for one operation.
    async fn authorize(&self, m: &Model, h: &HeaderMap, op: Op) -> Result<(), ApiError> {
        let perm = match (&m.access, op) {
            (AccessMode::Public, _) => return Ok(()),
            (AccessMode::Authenticated, _) => return self.user_of(m, h).await.map(|_| ()),
            (AccessMode::Permissions { read, .. }, Op::Read) => read,
            (AccessMode::Permissions { write, .. }, Op::Write) => write,
            (AccessMode::Permissions { delete, .. }, Op::Delete) => delete,
        };
        let user = self.user_of(m, h).await?;
        let authz = self
            .interfaces
            .get::<dyn Authorizer>(AUTHORIZER)
            .ok_or_else(|| {
                ApiError::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "needs_rbac",
                    format!(
                        "`{}` checks the `{perm}` permission, which needs the rbac plugin (or set access: authenticated or public)",
                        m.name
                    ),
                )
            })?;
        let tenant = h
            .get(TENANT_HEADER)
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        if authz.has_permission(&user, perm, tenant).await? {
            Ok(())
        } else {
            Err(
                ApiError::forbidden("forbidden", format!("this needs the `{perm}` permission"))
                    .with("permission", perm.clone()),
            )
        }
    }

    async fn access(&self, m: &Model, h: &HeaderMap) -> Result<Access, ApiError> {
        match m.scope {
            Scope::None => Ok(Access::All),
            Scope::Tenant => h
                .get(TENANT_HEADER)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .map(Access::Tenant)
                .ok_or_else(|| {
                    ApiError::bad_request(
                        "missing_tenant",
                        format!("`{}` rows belong to a team; send the X-Tenant-Id header", m.name),
                    )
                }),
            Scope::Owner => {
                let user = self.user_of(m, h).await?;
                if let Some(authz) = self.interfaces.get::<dyn Authorizer>(AUTHORIZER) {
                    if authz.has_permission(&user, &m.manage, None).await? {
                        return Ok(Access::All);
                    }
                }
                Ok(Access::Owner(user))
            }
        }
    }

    fn select_cols(&self, m: &Model) -> String {
        let mut cols = vec!["id".to_string()];
        cols.extend(m.fields.iter().map(|f| f.name.clone()));
        match m.scope {
            Scope::Owner => cols.push("owner_id".into()),
            Scope::Tenant => cols.push("tenant".into()),
            Scope::None => {}
        }
        cols.push("created_at".into());
        cols.push("updated_at".into());
        cols.join(", ")
    }

    fn row_out(&self, m: &Model, mut row: Map<String, Value>) -> Value {
        let mut out = Map::new();
        out.insert("id".into(), row.remove("id").unwrap_or_default());
        for f in &m.fields {
            out.insert(
                f.name.clone(),
                from_db(f.kind, row.remove(&f.name).unwrap_or_default()),
            );
        }
        for k in ["owner_id", "tenant", "created_at", "updated_at"] {
            if let Some(v) = row.remove(k) {
                out.insert(k.into(), v);
            }
        }
        Value::Object(out)
    }

    // ---- cache -----------------------------------------------------------

    fn version_key(&self, m: &Model) -> String {
        format!("{}{}:v", self.ns, m.name)
    }

    async fn cache_key(&self, m: &Model, what: &str) -> Option<String> {
        if m.cache_ttl == 0 {
            return None;
        }
        let v = match self.cache.get(&self.version_key(m)).await {
            Ok(v) => v
                .map(|b| String::from_utf8_lossy(&b).into_owned())
                .unwrap_or_default(),
            Err(e) => {
                self.log.warn(&format!("resource cache unavailable: {e}"));
                return None;
            }
        };
        let digest = Sha256::digest(what.as_bytes());
        let hex: String = digest.iter().take(16).map(|b| format!("{b:02x}")).collect();
        Some(format!("{}{}:{v}:{hex}", self.ns, m.name))
    }

    async fn cached(&self, key: &Option<String>) -> Option<Vec<u8>> {
        match key {
            Some(k) => self.cache.get(k).await.ok().flatten(),
            None => None,
        }
    }

    async fn store(&self, m: &Model, key: &Option<String>, v: &Value) {
        if let Some(k) = key {
            let _ = self
                .cache
                .set(k, v.to_string().into_bytes(), Duration::from_secs(m.cache_ttl))
                .await;
        }
    }

    async fn invalidate(&self, m: &Model) {
        if m.cache_ttl > 0 {
            if let Err(e) = self
                .cache
                .incr(&self.version_key(m), Duration::from_secs(30 * 24 * 3600))
                .await
            {
                self.log
                    .warn(&format!("could not clear the {} cache: {e}", m.name));
            }
        }
    }

    // ---- handlers --------------------------------------------------------

    fn describe(&self) -> Value {
        let resources: Map<String, Value> = self
            .models
            .values()
            .map(|b| {
                let m = &b.model;
                let cols = m.columns();
                (
                    m.name.clone(),
                    json!({
                        "path": format!("/api/{}", m.name),
                        "access": match &m.access {
                            AccessMode::Permissions { read, write, delete } => json!({ "read": read, "write": write, "delete": delete }),
                            AccessMode::Authenticated => json!("authenticated"),
                            AccessMode::Public => json!("public"),
                        },
                        "scope": match m.scope { Scope::None => "none", Scope::Owner => "owner", Scope::Tenant => "tenant" },
                        "fields": m.fields.iter().map(|f| (f.name.clone(), f.spec.clone())).collect::<Map<_, _>>(),
                        "filters": cols.iter().filter(|c| c.2).map(|c| c.0.clone()).collect::<Vec<_>>(),
                        "sort": cols.iter().filter(|c| c.3).map(|c| c.0.clone()).collect::<Vec<_>>(),
                        "page_size": m.page_size,
                        "max_page_size": m.max_page_size,
                        "create_schema": b.create_schema,
                    }),
                )
            })
            .collect();
        json!({
            "resources": resources,
            "query": "filters: field=v, field.ne, .gt, .gte, .lt, .lte, .contains, .in=a,b, .null=true; sort=-created_at,name; limit, offset",
        })
    }

    async fn list(&self, name: &str, h: &HeaderMap, query: Option<String>) -> Reply {
        let b = self.model(name)?;
        let m = &b.model;
        let access = self.access(m, h).await?;
        self.authorize(m, h, Op::Read).await?;
        let raw = query.unwrap_or_default();
        let key = self.cache_key(m, &format!("list|{}|{raw}", access.key())).await;
        if let Some(bytes) = self.cached(&key).await {
            return cached_reply(bytes);
        }
        let cols = m.columns();
        let mut wheres: Vec<String> = Vec::new();
        let mut params: Vec<Value> = Vec::new();
        if let Some((c, p)) = access.condition() {
            wheres.push(c.into());
            params.push(p);
        }
        let mut limit = m.page_size;
        let mut offset = 0u64;
        let mut order: Vec<String> = Vec::new();
        for (k, v) in parse_query(&raw) {
            match k.as_str() {
                "limit" => {
                    limit = v
                        .parse()
                        .ok()
                        .filter(|n| (1..=m.max_page_size).contains(n))
                        .ok_or_else(|| {
                            ApiError::bad_request(
                                "invalid_limit",
                                format!("limit must be a whole number from 1 to {}", m.max_page_size),
                            )
                        })?;
                }
                "offset" => {
                    offset = v.parse().map_err(|_| {
                        ApiError::bad_request("invalid_offset", "offset must be a whole number")
                    })?;
                }
                "sort" => {
                    for part in v.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                        let (desc, field) = match part.strip_prefix('-') {
                            Some(f) => (true, f),
                            None => (false, part),
                        };
                        if !cols.iter().any(|c| c.0 == field && c.3) {
                            return Err(ApiError::bad_request(
                                "invalid_sort",
                                format!(
                                    "cannot sort by `{field}`; sortable: {}",
                                    cols.iter()
                                        .filter(|c| c.3)
                                        .map(|c| c.0.as_str())
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                ),
                            ));
                        }
                        order.push(format!("{field} {}", if desc { "DESC" } else { "ASC" }));
                    }
                }
                _ => {
                    let (field, op) = k.split_once('.').unwrap_or((k.as_str(), "eq"));
                    let col = cols.iter().find(|c| c.0 == field && c.2).ok_or_else(|| {
                        ApiError::bad_request(
                            "unknown_filter",
                            format!(
                                "cannot filter by `{field}`; filters: {}; also limit, offset and sort",
                                cols.iter()
                                    .filter(|c| c.2)
                                    .map(|c| c.0.as_str())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            ),
                        )
                    })?;
                    let kind = col.1;
                    let cmp = |sym: &str,
                               wheres: &mut Vec<String>,
                               params: &mut Vec<Value>|
                     -> Result<(), ApiError> {
                        wheres.push(format!("{field} {sym} ?"));
                        params.push(coerce(kind, field, &v)?);
                        Ok(())
                    };
                    match op {
                        "eq" => cmp("=", &mut wheres, &mut params)?,
                        "ne" => cmp("<>", &mut wheres, &mut params)?,
                        "gt" => cmp(">", &mut wheres, &mut params)?,
                        "gte" => cmp(">=", &mut wheres, &mut params)?,
                        "lt" => cmp("<", &mut wheres, &mut params)?,
                        "lte" => cmp("<=", &mut wheres, &mut params)?,
                        "contains" if kind.textual() => {
                            let esc = v
                                .to_lowercase()
                                .replace('!', "!!")
                                .replace('%', "!%")
                                .replace('_', "!_");
                            wheres.push(format!("LOWER({field}) LIKE ? ESCAPE '!'"));
                            params.push(json!(format!("%{esc}%")));
                        }
                        "in" => {
                            let items: Vec<&str> = v.split(',').map(str::trim).collect();
                            wheres.push(format!(
                                "{field} IN ({})",
                                vec!["?"; items.len()].join(", ")
                            ));
                            for i in items {
                                params.push(coerce(kind, field, i)?);
                            }
                        }
                        "null" => match v.as_str() {
                            "true" | "1" => wheres.push(format!("{field} IS NULL")),
                            "false" | "0" => wheres.push(format!("{field} IS NOT NULL")),
                            _ => {
                                return Err(ApiError::bad_request(
                                    "invalid_filter",
                                    format!("`{field}.null` takes true or false"),
                                ))
                            }
                        },
                        _ => {
                            return Err(ApiError::bad_request(
                                "unknown_operator",
                                format!(
                                    "`{field}.{op}` is not a filter; use {field}, {field}.ne, .gt, .gte, .lt, .lte, .in, .null{}",
                                    if kind.textual() { ", .contains" } else { "" }
                                ),
                            ))
                        }
                    }
                }
            }
        }
        if order.is_empty() {
            order.push("created_at ASC".into());
        }
        order.push("id ASC".into());
        let table = self.table(m);
        let where_sql = if wheres.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", wheres.join(" AND "))
        };
        let total = self
            .sql
            .query(&format!("SELECT COUNT(*) AS n FROM {table}{where_sql}"), &params)
            .await?
            .first()
            .and_then(|r| r.get("n").and_then(Value::as_i64))
            .unwrap_or(0);
        let rows = self
            .sql
            .query(
                &format!(
                    "SELECT {} FROM {table}{where_sql} ORDER BY {} LIMIT {limit} OFFSET {offset}",
                    self.select_cols(m),
                    order.join(", ")
                ),
                &params,
            )
            .await?;
        let items: Vec<Value> = rows.into_iter().map(|r| self.row_out(m, r)).collect();
        let out = json!({ "items": items, "total": total, "limit": limit, "offset": offset });
        self.store(m, &key, &out).await;
        json_reply(StatusCode::OK, &out)
    }

    async fn fetch(&self, m: &Model, id: &str, access: &Access) -> Result<Option<Value>, ApiError> {
        let mut sql = format!(
            "SELECT {} FROM {} WHERE id = ?",
            self.select_cols(m),
            self.table(m)
        );
        let mut params = vec![json!(id)];
        if let Some((c, p)) = access.condition() {
            sql.push_str(&format!(" AND {c}"));
            params.push(p);
        }
        Ok(self
            .sql
            .query(&sql, &params)
            .await?
            .into_iter()
            .next()
            .map(|r| self.row_out(m, r)))
    }

    fn not_found(m: &Model, id: &str) -> ApiError {
        ApiError::not_found("not_found", format!("no {} with id `{id}`", m.name))
    }

    async fn get(&self, name: &str, id: &str, h: &HeaderMap) -> Reply {
        let b = self.model(name)?;
        let m = &b.model;
        let access = self.access(m, h).await?;
        self.authorize(m, h, Op::Read).await?;
        let key = self.cache_key(m, &format!("get|{}|{id}", access.key())).await;
        if let Some(bytes) = self.cached(&key).await {
            return cached_reply(bytes);
        }
        let item = self
            .fetch(m, id, &access)
            .await?
            .ok_or_else(|| Self::not_found(m, id))?;
        self.store(m, &key, &item).await;
        json_reply(StatusCode::OK, &item)
    }

    fn body(bytes: &Bytes) -> Result<Map<String, Value>, ApiError> {
        if bytes.is_empty() {
            return Err(ApiError::bad_request("invalid_body", "send a JSON object"));
        }
        match serde_json::from_slice(bytes) {
            Ok(Value::Object(o)) => Ok(o),
            Ok(_) => Err(ApiError::bad_request(
                "invalid_body",
                "the body must be a JSON object",
            )),
            Err(e) => Err(ApiError::bad_request(
                "invalid_body",
                format!("the JSON body is not valid: {e}"),
            )),
        }
    }

    fn conflict(m: &Model, e: &keel_sdk::CapabilityError) -> Option<ApiError> {
        let msg = e.to_string();
        let lower = msg.to_lowercase();
        if !(lower.contains("unique") || lower.contains("duplicate")) {
            return None;
        }
        let field = m
            .fields
            .iter()
            .filter(|f| f.unique)
            .find(|f| msg.contains(&format!("_{}_uq", f.name)) || msg.contains(&format!(".{}", f.name)))
            .map(|f| f.name.clone());
        Some(match field {
            Some(f) => {
                ApiError::conflict("conflict", format!("another {} has this {f}", m.name)).with("field", f)
            }
            None => ApiError::conflict(
                "conflict",
                format!("another {} has the same unique value", m.name),
            ),
        })
    }

    async fn create(&self, name: &str, h: &HeaderMap, bytes: &Bytes) -> Reply {
        let b = self.model(name)?;
        let m = &b.model;
        let access = self.access(m, h).await?;
        self.authorize(m, h, Op::Write).await?;
        let mut input = Self::body(bytes)?;
        for f in &m.fields {
            if let (false, Some(d)) = (input.contains_key(&f.name), &f.default) {
                input.insert(f.name.clone(), d.clone());
            }
        }
        validate(&b.create, &Value::Object(input.clone()))?;
        check_dates(m, &input)?;
        let id = uuid::Uuid::new_v4().to_string();
        let now = now_ms();
        let mut cols = vec!["id".to_string(), "created_at".into(), "updated_at".into()];
        let mut marks = vec!["?".to_string(), "?".into(), "?".into()];
        let mut params = vec![json!(id), json!(now), json!(now)];
        match (&access, m.scope) {
            (Access::Owner(u), _) => {
                cols.push("owner_id".into());
                marks.push("?".into());
                params.push(json!(u));
            }
            // A manager creating in an owner-scoped resource still owns the row.
            (Access::All, Scope::Owner) => {
                let user = self.caller(h).await?;
                cols.push("owner_id".into());
                marks.push("?".into());
                params.push(json!(user));
            }
            (Access::Tenant(t), _) => {
                cols.push("tenant".into());
                marks.push("?".into());
                params.push(json!(t));
            }
            _ => {}
        }
        for f in &m.fields {
            if let Some(v) = input.get(&f.name) {
                cols.push(f.name.clone());
                let db = to_db(f.kind, v);
                if db.is_null() {
                    marks.push("NULL".into());
                } else {
                    marks.push("?".into());
                    params.push(db);
                }
            }
        }
        let sql = format!(
            "INSERT INTO {} ({}) VALUES ({})",
            self.table(m),
            cols.join(", "),
            marks.join(", ")
        );
        if let Err(e) = self.sql.execute(&sql, &params).await {
            return Err(Self::conflict(m, &e).unwrap_or_else(|| e.into()));
        }
        self.invalidate(m).await;
        let item = self
            .fetch(m, &id, &Access::All)
            .await?
            .ok_or_else(|| ApiError::internal("created row not found"))?;
        self.events
            .emit(
                "resource.created",
                json!({ "resource": m.name, "id": id, "item": item }),
            )
            .await
            .ok();
        json_reply(StatusCode::CREATED, &item)
    }

    async fn caller(&self, h: &HeaderMap) -> Result<String, ApiError> {
        let identity = self
            .interfaces
            .get::<dyn IdentityProvider>(IDENTITY_PROVIDER)
            .ok_or_else(|| ApiError::internal("identity provider missing"))?;
        let token =
            bearer(h).ok_or_else(|| ApiError::unauthorized("missing_token", "send a bearer token"))?;
        Ok(identity.verify_access_token(token).await?.sub)
    }

    async fn update(&self, name: &str, id: &str, h: &HeaderMap, bytes: &Bytes) -> Reply {
        let b = self.model(name)?;
        let m = &b.model;
        let access = self.access(m, h).await?;
        self.authorize(m, h, Op::Write).await?;
        let input = Self::body(bytes)?;
        validate(&b.update, &Value::Object(input.clone()))?;
        check_dates(m, &input)?;
        if input.is_empty() {
            return Err(ApiError::bad_request(
                "empty_update",
                format!("send at least one field of {} to change", m.name),
            ));
        }
        let mut sets = vec!["updated_at = ?".to_string()];
        let mut params = vec![json!(now_ms())];
        for f in &m.fields {
            if let Some(v) = input.get(&f.name) {
                let db = to_db(f.kind, v);
                if db.is_null() {
                    sets.push(format!("{} = NULL", f.name));
                } else {
                    sets.push(format!("{} = ?", f.name));
                    params.push(db);
                }
            }
        }
        let mut sql = format!("UPDATE {} SET {} WHERE id = ?", self.table(m), sets.join(", "));
        params.push(json!(id));
        if let Some((c, p)) = access.condition() {
            sql.push_str(&format!(" AND {c}"));
            params.push(p);
        }
        let n = match self.sql.execute(&sql, &params).await {
            Ok(n) => n,
            Err(e) => return Err(Self::conflict(m, &e).unwrap_or_else(|| e.into())),
        };
        if n == 0 {
            return Err(Self::not_found(m, id));
        }
        self.invalidate(m).await;
        let item = self
            .fetch(m, id, &Access::All)
            .await?
            .ok_or_else(|| Self::not_found(m, id))?;
        self.events
            .emit(
                "resource.updated",
                json!({ "resource": m.name, "id": id, "item": item, "changed": input.keys().collect::<Vec<_>>() }),
            )
            .await
            .ok();
        json_reply(StatusCode::OK, &item)
    }

    async fn delete(&self, name: &str, id: &str, h: &HeaderMap) -> Reply {
        let b = self.model(name)?;
        let m = &b.model;
        let access = self.access(m, h).await?;
        self.authorize(m, h, Op::Delete).await?;
        let mut sql = format!("DELETE FROM {} WHERE id = ?", self.table(m));
        let mut params = vec![json!(id)];
        if let Some((c, p)) = access.condition() {
            sql.push_str(&format!(" AND {c}"));
            params.push(p);
        }
        if self.sql.execute(&sql, &params).await? == 0 {
            return Err(Self::not_found(m, id));
        }
        self.invalidate(m).await;
        self.events
            .emit("resource.deleted", json!({ "resource": m.name, "id": id }))
            .await
            .ok();
        Ok(StatusCode::NO_CONTENT.into_response())
    }
}

#[derive(Default)]
pub struct Resource;

impl Resource {
    pub fn new() -> Self {
        Self
    }
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

#[async_trait]
impl Plugin for Resource {
    fn manifest(&self) -> &Manifest {
        keel_sdk::manifest!()
    }

    fn migrations_for(&self, _dialect: &str, config: &Value) -> Vec<Migration> {
        parse_models(config)
            .map(|ms| ms.iter().flat_map(Model::migrations).collect())
            .unwrap_or_default()
    }

    async fn init(&self, ctx: &mut InitContext) -> Result<(), PluginError> {
        let models = parse_models(&ctx.config).map_err(PluginError::Other)?;
        let mut built = BTreeMap::new();
        for m in models {
            let create_schema = m.body_schema(true);
            let compile = |s: &Value| {
                jsonschema::validator_for(s)
                    .map_err(|e| PluginError::Other(format!("resources.{}: {e}", m.name)))
            };
            let create = compile(&create_schema)?;
            let update = compile(&m.body_schema(false))?;
            built.insert(
                m.name.clone(),
                Built {
                    model: m,
                    create,
                    update,
                    create_schema,
                },
            );
        }
        let caps = &ctx.capabilities;
        let svc = Arc::new(Service {
            models: built,
            sql: caps.sql()?,
            cache: caps.cache()?,
            log: caps.log()?,
            events: ctx.emitter(),
            interfaces: ctx.interfaces(),
            ns: caps.namespace().to_string(),
        });

        let r = ctx.routes();
        r.get("/api", {
            let s = svc.clone();
            move || {
                let s = s.clone();
                async move { json_reply(StatusCode::OK, &s.describe()) }
            }
        })?;
        r.get(
            "/api/{resource}",
            handler!(svc, |s, p: Path<String>, h: HeaderMap, q: RawQuery| s
                .list(&p.0, &h, q.0)
                .await),
        )?;
        r.post(
            "/api/{resource}",
            handler!(svc, |s, p: Path<String>, h: HeaderMap, b: Bytes| s
                .create(&p.0, &h, &b)
                .await),
        )?;
        r.get(
            "/api/{resource}/{id}",
            handler!(svc, |s, p: Path<(String, String)>, h: HeaderMap| s
                .get(&p.0 .0, &p.0 .1, &h)
                .await),
        )?;
        r.patch(
            "/api/{resource}/{id}",
            handler!(svc, |s, p: Path<(String, String)>, h: HeaderMap, b: Bytes| s
                .update(&p.0 .0, &p.0 .1, &h, &b)
                .await),
        )?;
        r.delete(
            "/api/{resource}/{id}",
            handler!(svc, |s, p: Path<(String, String)>, h: HeaderMap| s
                .delete(&p.0 .0, &p.0 .1, &h)
                .await),
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn models(v: Value) -> Result<Vec<Model>, String> {
        parse_models(&json!({ "resources": v }))
    }

    #[test]
    fn migrations_are_one_per_piece() {
        let ms = models(json!({ "projects": { "scope": "owner", "fields": {
            "name": { "type": "string", "required": true, "unique": true },
            "budget": { "type": "number" }
        }}}))
        .unwrap();
        let ids: Vec<String> = ms[0].migrations().into_iter().map(|m| m.id).collect();
        assert_eq!(
            ids,
            [
                "projects_00_table",
                "projects_01_owner",
                "projects_10_budget",
                "projects_10_name",
                "projects_20_name_unique"
            ]
        );
        // Changing rules other than type keeps the field's migration.
        let a = models(json!({ "p": { "fields": { "n": { "type": "string" } } } })).unwrap()[0].migrations();
        let b = models(
            json!({ "p": { "fields": { "n": { "type": "string", "max_length": 20, "required": true } } } }),
        )
        .unwrap()[0]
            .migrations();
        assert_eq!(a, b);
    }

    #[test]
    fn bad_models_are_explained() {
        let e = models(json!({ "p": { "fields": { "order": { "type": "string" } } } })).unwrap_err();
        assert!(e.contains("reserved"), "{e}");
        let e = models(json!({ "p": { "fields": { "n": { "type": "uuid" } } } })).unwrap_err();
        assert!(e.contains("must be one of"), "{e}");
        let e =
            models(json!({ "p": { "fields": { "n": { "type": "integer", "default": "x" } } } })).unwrap_err();
        assert!(e.contains("default"), "{e}");
    }

    #[test]
    fn calendar_dates() {
        assert!(real_date(Kind::Date, "2028-02-29").is_ok());
        assert!(real_date(Kind::Date, "2027-02-29").is_err());
        assert!(real_date(Kind::Date, "2027-13-45")
            .unwrap_err()
            .contains("month 13"));
        assert!(real_date(Kind::Date, "2027-04-31").is_err());
        assert!(real_date(Kind::Datetime, "2027-01-31T23:59:59Z").is_ok());
        assert!(real_date(Kind::Datetime, "2027-01-31T24:00").is_err());
        assert!(real_date(Kind::Datetime, "2027-01-31T10:00:00+25:00").is_err());
        assert!(real_date(Kind::Datetime, "2027-01-31T10:00:00.5-05:30").is_ok());
    }

    #[test]
    fn values_round_trip() {
        assert_eq!(to_db(Kind::Boolean, &json!(true)), json!(1));
        assert_eq!(from_db(Kind::Boolean, json!(1)), json!(true));
        assert_eq!(
            from_db(Kind::Json, to_db(Kind::Json, &json!({"a": [1]}))),
            json!({"a": [1]})
        );
        assert!(
            to_db(Kind::Number, &json!(3)).as_i64().is_none(),
            "numbers bind as floats"
        );
        assert_eq!(
            parse_query("name.contains=a+b%26c&limit=5"),
            [
                ("name.contains".into(), "a b&c".into()),
                ("limit".into(), "5".into())
            ]
        );
    }
}
