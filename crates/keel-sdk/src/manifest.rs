//! `plugin.yaml`: the manifest that describes a plugin to the host and to an AI.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use semver::{Version, VersionReq};
use serde::Deserialize;
use serde_json::Value;

use crate::capabilities::{Capability, DegradeLevel};
use crate::issues::{format_list, schema_issues, Issue};
use crate::setup::SetupStep;

const PLUGIN_SCHEMA: &str = include_str!("../schemas/plugin.schema.json");

/// A parsed, checked plugin manifest.
#[derive(Debug, Clone)]
pub struct Manifest {
    pub id: String,
    pub version: Version,
    pub summary: String,
    /// Extra search words (`keel plugins list`, MCP `search_plugins`).
    pub tags: Vec<String>,
    /// Host versions this plugin works with. Defaults to any.
    pub host: VersionReq,
    pub capabilities: Vec<Capability>,
    /// Other plugins this one needs, by id, with a version range.
    pub plugins: BTreeMap<String, VersionReq>,
    pub interfaces: Vec<String>,
    pub routes: Vec<Route>,
    pub events: Vec<String>,
    pub consumes: Vec<String>,
    /// JSON Schema for the plugin's `config:` block, if it has one.
    pub config_schema: Option<Value>,
    pub runtime_config: Vec<String>,
    pub migrations: Option<String>,
    /// Human actions the plugin needs (accounts, keys), tracked by `keel setup`.
    pub setup: Vec<SetupStep>,
    /// Default `/health` weight; `app.yaml` can override it per plugin.
    pub degrade_level: DegradeLevel,
    /// Non-empty for a composer: the plugin entries it expands into.
    pub compose: Vec<ComposeEntry>,
}

/// One `compose:` entry: a plugin entry template.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ComposeEntry {
    /// `id` or `id@range`.
    #[serde(rename = "use")]
    pub spec: String,
    /// Path such as `config.social`; the entry is kept only when it is set.
    #[serde(default)]
    pub when: Option<String>,
    /// Config template.
    #[serde(default)]
    pub config: Value,
}

impl ComposeEntry {
    pub fn id(&self) -> &str {
        self.spec.split_once('@').map_or(self.spec.as_str(), |(n, _)| n)
    }
}

/// A route the plugin declares, e.g. `GET /hello`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Route {
    pub method: String,
    pub path: String,
}

impl std::fmt::Display for Route {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.method, self.path)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("plugin.yaml: not valid YAML: {0}")]
    Parse(String),
    #[error("{}", format_list("plugin.yaml", .0))]
    Invalid(Vec<Issue>),
    #[error("config.schema.json: {0}")]
    ConfigSchema(String),
}

#[derive(Deserialize)]
struct Raw {
    id: String,
    version: String,
    summary: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    requires: RawRequires,
    #[serde(default)]
    provides: RawProvides,
    #[serde(default)]
    consumes: RawConsumes,
    #[serde(default)]
    #[allow(dead_code)]
    config: Option<String>,
    #[serde(default)]
    runtime_config: Vec<String>,
    #[serde(default)]
    migrations: Option<String>,
    #[serde(default)]
    setup: Vec<SetupStep>,
    #[serde(default)]
    degrade_level: DegradeLevel,
    #[serde(default)]
    compose: Vec<ComposeEntry>,
}

#[derive(Deserialize, Default)]
struct RawRequires {
    host: Option<String>,
    #[serde(default)]
    capabilities: Vec<String>,
    #[serde(default)]
    plugins: BTreeMap<String, String>,
}

#[derive(Deserialize, Default)]
struct RawProvides {
    #[serde(default)]
    interfaces: Vec<String>,
    #[serde(default)]
    routes: Vec<String>,
    #[serde(default)]
    events: Vec<String>,
}

#[derive(Deserialize, Default)]
struct RawConsumes {
    #[serde(default)]
    events: Vec<String>,
}

fn validator() -> &'static jsonschema::Validator {
    static V: OnceLock<jsonschema::Validator> = OnceLock::new();
    V.get_or_init(|| {
        let schema: Value =
            serde_json::from_str(PLUGIN_SCHEMA).expect("embedded plugin schema is valid JSON");
        jsonschema::validator_for(&schema).expect("embedded plugin schema compiles")
    })
}

impl Manifest {
    /// Parses and checks `plugin.yaml` text, plus the optional config schema text.
    pub fn from_sources(plugin_yaml: &str, config_schema: Option<&str>) -> Result<Self, ManifestError> {
        let value: Value =
            serde_yaml::from_str(plugin_yaml).map_err(|e| ManifestError::Parse(e.to_string()))?;
        let mut issues = schema_issues(validator(), &value);
        if !issues.is_empty() {
            return Err(ManifestError::Invalid(issues));
        }
        let raw: Raw = serde_json::from_value(value).map_err(|e| ManifestError::Parse(e.to_string()))?;

        let version = Version::parse(&raw.version)
            .map_err(|e| issues.push(Issue::new("version", format!("not a semver version: {e}"))))
            .ok();
        let host = match &raw.requires.host {
            None => Some(VersionReq::STAR),
            Some(h) => VersionReq::parse(h)
                .map_err(|e| issues.push(Issue::new("requires.host", format!("not a version range: {e}"))))
                .ok(),
        };
        let mut plugins = BTreeMap::new();
        for (id, req) in &raw.requires.plugins {
            match VersionReq::parse(req) {
                Ok(r) => {
                    plugins.insert(id.clone(), r);
                }
                Err(e) => issues.push(Issue::new(
                    format!("requires.plugins.{id}"),
                    format!("not a version range: {e}"),
                )),
            }
            if id == &raw.id {
                issues.push(Issue::new(
                    format!("requires.plugins.{id}"),
                    "a plugin cannot require itself",
                ));
            }
        }
        let capabilities: Vec<Capability> = raw
            .requires
            .capabilities
            .iter()
            .filter_map(|c| Capability::parse(c))
            .collect();

        let mut routes = Vec::new();
        let mut seen = BTreeSet::new();
        for (i, r) in raw.provides.routes.iter().enumerate() {
            let (method, path) = r.split_once(' ').expect("schema checks the route pattern");
            let route = Route {
                method: method.to_string(),
                path: path.to_string(),
            };
            if !seen.insert(route.clone()) {
                issues.push(Issue::new(
                    format!("provides.routes[{i}]"),
                    format!("`{r}` is declared twice"),
                ));
            }
            if path == "/health" || path.starts_with("/_admin") {
                issues.push(Issue::new(
                    format!("provides.routes[{i}]"),
                    format!("`{path}` is reserved by the host"),
                ));
            }
            routes.push(route);
        }

        let mut step_ids = BTreeSet::new();
        for (i, step) in raw.setup.iter().enumerate() {
            if !step_ids.insert(step.id.as_str()) {
                issues.push(Issue::new(
                    format!("setup[{i}].id"),
                    format!("`{}` is used by another step", step.id),
                ));
            }
        }

        for (i, c) in raw.compose.iter().enumerate() {
            if c.id() == raw.id {
                issues.push(Issue::new(
                    format!("compose[{i}].use"),
                    "a composer cannot include itself",
                ));
            }
            if let Some((_, req)) = c.spec.split_once('@') {
                if let Err(e) = VersionReq::parse(req) {
                    issues.push(Issue::new(
                        format!("compose[{i}].use"),
                        format!("not a version range: {e}"),
                    ));
                }
            }
        }
        if !raw.compose.is_empty()
            && (!raw.provides.routes.is_empty()
                || !raw.requires.capabilities.is_empty()
                || !raw.provides.interfaces.is_empty()
                || raw.migrations.is_some())
        {
            issues.push(Issue::new(
                "compose",
                "a composer only expands into other plugins; move routes, capabilities, interfaces and migrations to a plugin it includes",
            ));
        }

        let config_schema = match config_schema {
            None => None,
            Some(text) => {
                let v: Value = serde_json::from_str(text)
                    .map_err(|e| ManifestError::ConfigSchema(format!("not valid JSON: {e}")))?;
                jsonschema::validator_for(&v)
                    .map_err(|e| ManifestError::ConfigSchema(format!("not a valid JSON Schema: {e}")))?;
                Some(v)
            }
        };

        if !issues.is_empty() {
            issues.sort_by(|a, b| a.path.cmp(&b.path));
            return Err(ManifestError::Invalid(issues));
        }

        Ok(Manifest {
            id: raw.id,
            version: version.expect("checked above"),
            summary: raw.summary,
            tags: raw.tags,
            host: host.expect("checked above"),
            capabilities,
            plugins,
            interfaces: raw.provides.interfaces,
            routes,
            events: raw.provides.events,
            consumes: raw.consumes.events,
            config_schema,
            runtime_config: raw.runtime_config,
            migrations: raw.migrations,
            setup: raw.setup,
            degrade_level: raw.degrade_level,
            compose: raw.compose,
        })
    }

    /// Checks `config` against the plugin's config schema. A plugin without a
    /// schema accepts only an empty config.
    pub fn validate_config(&self, config: &Value) -> Vec<Issue> {
        match &self.config_schema {
            Some(schema) => match jsonschema::validator_for(schema) {
                Ok(v) => schema_issues(&v, config),
                Err(e) => vec![Issue::new(
                    "(root)",
                    format!("plugin config schema is invalid: {e}"),
                )],
            },
            None => match config {
                Value::Object(m) if m.is_empty() => vec![],
                Value::Null => vec![],
                _ => vec![Issue::new("(root)", "this plugin takes no config")],
            },
        }
    }

    /// `config` with schema defaults filled in for missing top-level keys.
    pub fn config_with_defaults(&self, config: &Value) -> Value {
        let mut out = config.as_object().cloned().unwrap_or_default();
        if let Some(props) = self
            .config_schema
            .as_ref()
            .and_then(|s| s["properties"].as_object())
        {
            for (k, p) in props {
                if let (false, Some(d)) = (out.contains_key(k), p.get("default")) {
                    out.insert(k.clone(), d.clone());
                }
            }
        }
        Value::Object(out)
    }

    /// Whether this is a composer (a bundle of other plugins, no code).
    pub fn is_composer(&self) -> bool {
        !self.compose.is_empty()
    }

    /// Whether this plugin declares `route`.
    pub fn declares(&self, method: &str, path: &str) -> bool {
        self.routes.iter().any(|r| r.method == method && r.path == path)
    }
}
