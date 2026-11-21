//! Loading and validating `app.yaml`.
//!
//! Pipeline: parse YAML, validate against the embedded JSON Schema (so every
//! bad key is reported with its path), resolve `${secret:NAME}` references,
//! then deserialize into [`AppConfig`]. Validation runs before secrets are
//! resolved, so error messages never contain secret values.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::OnceLock;

use serde::Deserialize;
use serde_json::Value;

use keel_sdk::issues::{format_list, schema_issues};
pub use keel_sdk::Issue;

use crate::secrets::{self, MissingSecret, SecretSource};

const APP_SCHEMA: &str = include_str!("../schemas/app.schema.json");

/// The whole backend, as described by `app.yaml`.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    pub app: String,
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub providers: BTreeMap<String, ProviderConfig>,
    #[serde(default)]
    pub plugins: Vec<PluginEntry>,
    #[serde(default)]
    pub routes: BTreeMap<String, RouteRule>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
        }
    }
}

fn default_host() -> String {
    "0.0.0.0".to_string()
}

fn default_port() -> u16 {
    8080
}

/// A data provider (sql, cache, kv) chosen in YAML, e.g. `{ use: postgres, url: ... }`.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ProviderConfig {
    #[serde(rename = "use")]
    pub kind: String,
    #[serde(flatten)]
    pub settings: serde_json::Map<String, Value>,
}

/// One entry under `plugins:`, e.g. `{ use: auth-core@^1, config: {...} }`.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PluginEntry {
    #[serde(rename = "use")]
    pub spec: String,
    #[serde(default = "empty_object")]
    pub config: Value,
}

fn empty_object() -> Value {
    Value::Object(Default::default())
}

impl PluginEntry {
    /// Plugin id without the version range, e.g. `auth-core`.
    pub fn name(&self) -> &str {
        self.spec.split_once('@').map_or(self.spec.as_str(), |(n, _)| n)
    }

    /// Version range after `@`, if any, e.g. `^1`.
    pub fn version_req(&self) -> Option<&str> {
        self.spec.split_once('@').map(|(_, v)| v)
    }
}

/// Guards that apply to a route pattern. Enforced from phase 5 (E8).
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RouteRule {
    #[serde(default)]
    pub guard: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("{file}: cannot read file: {source}")]
    Read {
        file: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{file}: not valid YAML: {message}")]
    Parse { file: String, message: String },
    #[error("{}", format_issues(file, issues))]
    Invalid { file: String, issues: Vec<Issue> },
    #[error("{}", format_missing(file, missing))]
    MissingSecrets {
        file: String,
        missing: Vec<MissingSecret>,
    },
}

fn format_issues(file: &str, issues: &[Issue]) -> String {
    format_list(file, issues)
}

fn format_missing(file: &str, missing: &[MissingSecret]) -> String {
    let mut s = format!("{file}: {} secret(s) not set", missing.len());
    for m in missing {
        s.push_str(&format!("\n  - {} (used at {})", m.name, m.path));
    }
    s.push_str("\n  Set them as environment variables, e.g. export NAME=value");
    s
}

impl AppConfig {
    /// Reads and loads a config file.
    pub fn load_file(path: impl AsRef<Path>, secrets: &dyn SecretSource) -> Result<Self, ConfigError> {
        let path = path.as_ref();
        let file = path.display().to_string();
        let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
            file: file.clone(),
            source,
        })?;
        Self::load_str(&text, &file, secrets)
    }

    /// Loads config from YAML text. `file` is only used in error messages.
    pub fn load_str(yaml: &str, file: &str, secrets: &dyn SecretSource) -> Result<Self, ConfigError> {
        let mut value: Value = serde_yaml::from_str(yaml).map_err(|e| ConfigError::Parse {
            file: file.to_string(),
            message: e.to_string(),
        })?;

        let issues = validate(&value);
        if !issues.is_empty() {
            return Err(ConfigError::Invalid {
                file: file.to_string(),
                issues,
            });
        }

        let missing = secrets::resolve(&mut value, secrets);
        if !missing.is_empty() {
            return Err(ConfigError::MissingSecrets {
                file: file.to_string(),
                missing,
            });
        }

        serde_json::from_value(value).map_err(|e| ConfigError::Invalid {
            file: file.to_string(),
            issues: vec![Issue {
                path: "(root)".into(),
                message: e.to_string(),
            }],
        })
    }
}

fn validator() -> &'static jsonschema::Validator {
    static V: OnceLock<jsonschema::Validator> = OnceLock::new();
    V.get_or_init(|| {
        let schema: Value = serde_json::from_str(APP_SCHEMA).expect("embedded app schema is valid JSON");
        jsonschema::validator_for(&schema).expect("embedded app schema compiles")
    })
}

/// Checks a parsed config against the app schema and returns every problem.
pub fn validate(value: &Value) -> Vec<Issue> {
    schema_issues(validator(), value)
}
