//! Capabilities: the only way plugins reach databases, caches, the network,
//! secrets and email. Swapping Postgres for SQLite is a change in `app.yaml`,
//! never in a plugin.
//!
//! Phase 2 defines the traits and provides `secrets` and `log`. Data
//! providers (`sql`, `kv`, `cache`) arrive in phase 3, `email` in phase 5.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Map, Value};

use crate::async_trait;

/// Names used in `requires.capabilities`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Capability {
    Sql,
    Kv,
    Cache,
    HttpClient,
    Secrets,
    Crypto,
    Log,
    Email,
}

impl Capability {
    pub const ALL: [Capability; 8] = [
        Capability::Sql,
        Capability::Kv,
        Capability::Cache,
        Capability::HttpClient,
        Capability::Secrets,
        Capability::Crypto,
        Capability::Log,
        Capability::Email,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Capability::Sql => "sql",
            Capability::Kv => "kv",
            Capability::Cache => "cache",
            Capability::HttpClient => "http-client",
            Capability::Secrets => "secrets",
            Capability::Crypto => "crypto",
            Capability::Log => "log",
            Capability::Email => "email",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.as_str() == s)
    }
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum CapabilityError {
    #[error("capability `{0}` is not listed in requires.capabilities of plugin.yaml")]
    NotDeclared(Capability),
    #[error("capability `{0}` is not available in this host")]
    NotAvailable(Capability),
    #[error("{0}")]
    Failed(String),
}

pub type CapResult<T> = Result<T, CapabilityError>;

/// SQL database. Rows are JSON objects keyed by column name.
#[async_trait]
pub trait Sql: Send + Sync {
    /// `postgres`, `mysql` or `sqlite`. Write `?` placeholders; the provider
    /// rewrites them for the database in use.
    fn dialect(&self) -> &'static str;
    async fn ping(&self) -> CapResult<()> {
        self.query("SELECT 1", &[]).await.map(|_| ())
    }
    async fn execute(&self, sql: &str, params: &[Value]) -> CapResult<u64>;
    async fn query(&self, sql: &str, params: &[Value]) -> CapResult<Vec<Map<String, Value>>>;
    /// Runs statements in one transaction where the database allows it
    /// (MySQL commits DDL implicitly). The default runs them one by one.
    async fn transaction(&self, statements: &[String]) -> CapResult<()> {
        for s in statements {
            self.execute(s, &[]).await?;
        }
        Ok(())
    }
}

/// Durable key-value store.
#[async_trait]
pub trait Kv: Send + Sync {
    async fn ping(&self) -> CapResult<()> {
        self.get("__keel_ping").await.map(|_| ())
    }
    async fn get(&self, key: &str) -> CapResult<Option<Vec<u8>>>;
    async fn set(&self, key: &str, value: Vec<u8>) -> CapResult<()>;
    async fn delete(&self, key: &str) -> CapResult<bool>;
}

/// Cache with time-to-live.
#[async_trait]
pub trait Cache: Send + Sync {
    async fn ping(&self) -> CapResult<()> {
        self.get("__keel_ping").await.map(|_| ())
    }
    async fn get(&self, key: &str) -> CapResult<Option<Vec<u8>>>;
    async fn set(&self, key: &str, value: Vec<u8>, ttl: Duration) -> CapResult<()>;
    async fn delete(&self, key: &str) -> CapResult<bool>;
}

/// Outbound HTTP request.
#[derive(Debug, Clone, Default)]
pub struct HttpRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, Default)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[async_trait]
pub trait HttpClient: Send + Sync {
    async fn send(&self, request: HttpRequest) -> CapResult<HttpResponse>;
}

/// Secrets by name, scoped to what the plugin may read.
pub trait Secrets: Send + Sync {
    fn get(&self, name: &str) -> Option<String>;
}

pub trait Crypto: Send + Sync {
    fn random_bytes(&self, len: usize) -> Vec<u8>;
    fn hash_password(&self, password: &str) -> CapResult<String>;
    fn verify_password(&self, password: &str, hash: &str) -> CapResult<bool>;
}

/// Structured logging, tagged with the plugin id.
pub trait Log: Send + Sync {
    fn info(&self, message: &str);
    fn warn(&self, message: &str);
    fn error(&self, message: &str);
}

#[derive(Debug, Clone, Default)]
pub struct EmailMessage {
    pub to: Vec<String>,
    pub subject: String,
    pub text: String,
    pub html: Option<String>,
}

#[async_trait]
pub trait Email: Send + Sync {
    async fn send(&self, message: EmailMessage) -> CapResult<()>;
}

/// The capabilities a plugin was granted. Asking for one the plugin did not
/// declare in `plugin.yaml` is an error, so manifests stay truthful.
#[derive(Clone, Default)]
pub struct Capabilities {
    declared: Vec<Capability>,
    namespace: String,
    pub(crate) sql: Option<Arc<dyn Sql>>,
    pub(crate) kv: Option<Arc<dyn Kv>>,
    pub(crate) cache: Option<Arc<dyn Cache>>,
    pub(crate) http: Option<Arc<dyn HttpClient>>,
    pub(crate) secrets: Option<Arc<dyn Secrets>>,
    pub(crate) crypto: Option<Arc<dyn Crypto>>,
    pub(crate) log: Option<Arc<dyn Log>>,
    pub(crate) email: Option<Arc<dyn Email>>,
}

macro_rules! getter {
    ($name:ident, $field:ident, $cap:expr, $t:ty) => {
        pub fn $name(&self) -> CapResult<Arc<$t>> {
            if !self.declared.contains(&$cap) {
                return Err(CapabilityError::NotDeclared($cap));
            }
            self.$field
                .clone()
                .ok_or(CapabilityError::NotAvailable($cap))
        }
    };
}

impl Capabilities {
    getter!(sql, sql, Capability::Sql, dyn Sql);
    getter!(kv, kv, Capability::Kv, dyn Kv);
    getter!(cache, cache, Capability::Cache, dyn Cache);
    getter!(http_client, http, Capability::HttpClient, dyn HttpClient);
    getter!(secrets, secrets, Capability::Secrets, dyn Secrets);
    getter!(crypto, crypto, Capability::Crypto, dyn Crypto);
    getter!(log, log, Capability::Log, dyn Log);
    getter!(email, email, Capability::Email, dyn Email);

    pub fn declared(&self) -> &[Capability] {
        &self.declared
    }

    /// Prefix for this plugin's SQL tables, e.g. `auth_core_`. Key-value and
    /// cache keys are prefixed automatically; SQL tables must use this.
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// `namespace()` + `name`, e.g. `table("users")` -> `auth_core_users`.
    pub fn table(&self, name: &str) -> String {
        format!("{}{name}", self.namespace)
    }
}

/// How much an item being down matters to `/health`: only `error` turns the
/// whole host `down` (HTTP 503); `info` and `warn` make it `degraded` (200).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum DegradeLevel {
    Info,
    Warn,
    #[default]
    Error,
}

impl DegradeLevel {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "info" => Some(Self::Info),
            "warn" => Some(Self::Warn),
            "error" => Some(Self::Error),
            _ => None,
        }
    }

    /// Default for a provider slot: losing the database or key-value store
    /// breaks the app; losing the cache or email does not.
    pub fn for_slot(slot: &str) -> Self {
        match slot {
            "cache" | "email" => Self::Warn,
            _ => Self::Error,
        }
    }
}

/// A configured data provider, as reported in `/health` and by the CLI.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ProviderInfo {
    /// The key in `app.yaml` providers: `db`, `cache` or `kv`.
    pub slot: String,
    /// `postgres`, `mysql`, `sqlite`, `redis` or `memory`.
    pub kind: String,
    pub capability: Capability,
    /// Whether it can be switched or reconnected live (phase 7).
    pub hot_load: bool,
    pub degrade_level: DegradeLevel,
}

/// What the host can provide. The host builds one per plugin with
/// [`CapabilityProviders::grant`], limited to what the plugin declared.
#[derive(Clone, Default)]
pub struct CapabilityProviders {
    pub info: Vec<ProviderInfo>,
    pub sql: Option<Arc<dyn Sql>>,
    pub kv: Option<Arc<dyn Kv>>,
    pub cache: Option<Arc<dyn Cache>>,
    pub http: Option<Arc<dyn HttpClient>>,
    pub secrets: Option<Arc<dyn Secrets>>,
    pub crypto: Option<Arc<dyn Crypto>>,
    pub email: Option<Arc<dyn Email>>,
}

impl CapabilityProviders {
    pub fn available(&self) -> Vec<Capability> {
        let mut v = vec![Capability::Log];
        let pairs = [
            (Capability::Sql, self.sql.is_some()),
            (Capability::Kv, self.kv.is_some()),
            (Capability::Cache, self.cache.is_some()),
            (Capability::HttpClient, self.http.is_some()),
            (Capability::Secrets, self.secrets.is_some()),
            (Capability::Crypto, self.crypto.is_some()),
            (Capability::Email, self.email.is_some()),
        ];
        v.extend(pairs.into_iter().filter(|(_, ok)| *ok).map(|(c, _)| c));
        v.sort();
        v
    }

    /// Capabilities for one plugin: only what it declared, with a log tagged
    /// with its id.
    pub fn grant(&self, plugin_id: &str, declared: &[Capability]) -> Capabilities {
        let has = |c| declared.contains(&c);
        let ns = format!("{plugin_id}:");
        Capabilities {
            declared: declared.to_vec(),
            namespace: format!("{}_", plugin_id.replace('-', "_")),
            sql: self.sql.clone().filter(|_| has(Capability::Sql)),
            kv: self.kv.clone().filter(|_| has(Capability::Kv)).map(|inner| {
                Arc::new(PrefixedKv {
                    inner,
                    prefix: ns.clone(),
                }) as Arc<dyn Kv>
            }),
            cache: self
                .cache
                .clone()
                .filter(|_| has(Capability::Cache))
                .map(|inner| {
                    Arc::new(PrefixedCache {
                        inner,
                        prefix: ns.clone(),
                    }) as Arc<dyn Cache>
                }),
            http: self.http.clone().filter(|_| has(Capability::HttpClient)),
            secrets: self.secrets.clone().filter(|_| has(Capability::Secrets)),
            crypto: self.crypto.clone().filter(|_| has(Capability::Crypto)),
            log: has(Capability::Log).then(|| Arc::new(TracingLog(plugin_id.to_string())) as Arc<dyn Log>),
            email: self.email.clone().filter(|_| has(Capability::Email)),
        }
    }
}

/// Keeps each plugin's keys apart: `auth-core` writing `session:1` stores
/// `auth-core:session:1`.
struct PrefixedKv {
    inner: Arc<dyn Kv>,
    prefix: String,
}

#[async_trait]
impl Kv for PrefixedKv {
    async fn ping(&self) -> CapResult<()> {
        self.inner.ping().await
    }
    async fn get(&self, key: &str) -> CapResult<Option<Vec<u8>>> {
        self.inner.get(&format!("{}{key}", self.prefix)).await
    }
    async fn set(&self, key: &str, value: Vec<u8>) -> CapResult<()> {
        self.inner.set(&format!("{}{key}", self.prefix), value).await
    }
    async fn delete(&self, key: &str) -> CapResult<bool> {
        self.inner.delete(&format!("{}{key}", self.prefix)).await
    }
}

struct PrefixedCache {
    inner: Arc<dyn Cache>,
    prefix: String,
}

#[async_trait]
impl Cache for PrefixedCache {
    async fn ping(&self) -> CapResult<()> {
        self.inner.ping().await
    }
    async fn get(&self, key: &str) -> CapResult<Option<Vec<u8>>> {
        self.inner.get(&format!("{}{key}", self.prefix)).await
    }
    async fn set(&self, key: &str, value: Vec<u8>, ttl: Duration) -> CapResult<()> {
        self.inner.set(&format!("{}{key}", self.prefix), value, ttl).await
    }
    async fn delete(&self, key: &str) -> CapResult<bool> {
        self.inner.delete(&format!("{}{key}", self.prefix)).await
    }
}

/// `log` capability backed by `tracing`.
struct TracingLog(String);

impl Log for TracingLog {
    fn info(&self, message: &str) {
        tracing::info!(plugin = %self.0, "{message}");
    }
    fn warn(&self, message: &str) {
        tracing::warn!(plugin = %self.0, "{message}");
    }
    fn error(&self, message: &str) {
        tracing::error!(plugin = %self.0, "{message}");
    }
}

/// Fixed secrets, useful in tests.
impl Secrets for HashMap<String, String> {
    fn get(&self, name: &str) -> Option<String> {
        HashMap::get(self, name).cloned()
    }
}

impl fmt::Debug for CapabilityProviders {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CapabilityProviders")
            .field("info", &self.info)
            .field("available", &self.available())
            .finish()
    }
}
