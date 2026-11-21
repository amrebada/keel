//! Loading compiled-in plugins that `app.yaml` asks for.
//!
//! Every check runs before any plugin starts, and every problem is reported
//! at once with its `app.yaml` path: unknown plugin, version mismatch, host
//! incompatibility, missing dependency, unavailable capability, invalid
//! config, or two plugins claiming the same route.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use keel_sdk::capabilities::CapabilityProviders;
use keel_sdk::issues::format_list;
use keel_sdk::{Event, HookBus, InitContext, Issue, Plugin};
use semver::{Version, VersionReq};

use crate::config::AppConfig;

/// The plugins compiled into this binary.
#[derive(Default, Clone)]
pub struct PluginRegistry {
    plugins: BTreeMap<String, Arc<dyn Plugin>>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a plugin. Panics if two plugins share an id, which is a build mistake.
    pub fn register(mut self, plugin: impl Plugin) -> Self {
        let id = plugin.manifest().id.clone();
        assert!(
            !self.plugins.contains_key(&id),
            "two compiled-in plugins share the id `{id}`"
        );
        self.plugins.insert(id, Arc::new(plugin));
        self
    }

    pub fn get(&self, id: &str) -> Option<&Arc<dyn Plugin>> {
        self.plugins.get(id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Arc<dyn Plugin>> {
        self.plugins.values()
    }

    pub fn ids(&self) -> Vec<&str> {
        self.plugins.keys().map(String::as_str).collect()
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{}", format_list("app.yaml plugins", .0))]
pub struct PluginLoadError(pub Vec<Issue>);

/// A plugin that started, with what the host keeps from its init.
pub struct Loaded {
    pub plugin: Arc<dyn Plugin>,
    pub router: axum::Router,
}

type Chosen = Vec<(usize, Arc<dyn Plugin>)>;

/// Checks every plugin listed in `config.plugins` without starting any:
/// existence, versions, host compatibility, capabilities, config, dependencies
/// and route ownership. Returns the plugins in dependency order.
pub fn check(
    config: &AppConfig,
    registry: &PluginRegistry,
    providers: &CapabilityProviders,
) -> Result<Chosen, PluginLoadError> {
    let host_version = Version::parse(keel_sdk::HOST_VERSION).expect("host version is semver");
    let available_caps = providers.available();
    let mut issues = Vec::new();
    let mut chosen: Chosen = Vec::new();
    let mut seen_ids = BTreeMap::new();

    // 1. Each entry on its own.
    for (i, entry) in config.plugins.iter().enumerate() {
        let at = format!("plugins[{i}]");
        let Some(plugin) = registry.get(entry.name()) else {
            let ids = registry.ids();
            let list = if ids.is_empty() {
                "none".to_string()
            } else {
                ids.join(", ")
            };
            issues.push(Issue::new(
                format!("{at}.use"),
                format!(
                    "plugin `{}` is not in this build. Compiled-in plugins: {list}",
                    entry.name()
                ),
            ));
            continue;
        };
        let m = plugin.manifest();
        if let Some(prev) = seen_ids.insert(m.id.clone(), i) {
            issues.push(Issue::new(
                format!("{at}.use"),
                format!("`{}` is already listed at plugins[{prev}]", m.id),
            ));
            continue;
        }
        if let Some(req) = entry.version_req() {
            match VersionReq::parse(req) {
                Ok(r) if !r.matches(&m.version) => issues.push(Issue::new(
                    format!("{at}.use"),
                    format!(
                        "`{}@{req}` asked for, but this build has version {}",
                        m.id, m.version
                    ),
                )),
                Ok(_) => {}
                Err(e) => issues.push(Issue::new(
                    format!("{at}.use"),
                    format!("`{req}` is not a version range: {e}"),
                )),
            }
        }
        if !m.host.matches(&host_version) {
            issues.push(Issue::new(
                format!("{at}.use"),
                format!("`{}` needs host {}, this host is {host_version}", m.id, m.host),
            ));
        }
        for cap in &m.capabilities {
            if !available_caps.contains(cap) {
                issues.push(Issue::new(
                    format!("{at}.use"),
                    format!(
                        "`{}` needs the `{cap}` capability, which this host does not provide yet",
                        m.id
                    ),
                ));
            }
        }
        for issue in m.validate_config(&entry.config) {
            let path = if issue.path == "(root)" {
                format!("{at}.config")
            } else {
                format!("{at}.config.{}", issue.path)
            };
            issues.push(Issue::new(path, issue.message));
        }
        chosen.push((i, plugin.clone()));
    }

    // 2. Across entries: dependencies and route ownership.
    let versions: BTreeMap<&str, &Version> = chosen
        .iter()
        .map(|(_, p)| (p.manifest().id.as_str(), &p.manifest().version))
        .collect();
    let mut route_owner: BTreeMap<String, String> = BTreeMap::new();
    for (i, p) in &chosen {
        let m = p.manifest();
        for (dep, req) in &m.plugins {
            match versions.get(dep.as_str()) {
                None => issues.push(Issue::new(
                    format!("plugins[{i}].use"),
                    format!("`{}` needs plugin `{dep}` ({req}); add it to plugins", m.id),
                )),
                Some(v) if !req.matches(v) => issues.push(Issue::new(
                    format!("plugins[{i}].use"),
                    format!("`{}` needs `{dep}` {req}, but {v} is configured", m.id),
                )),
                Some(_) => {}
            }
        }
        for r in &m.routes {
            let key = r.to_string();
            if let Some(owner) = route_owner.insert(key.clone(), m.id.clone()) {
                issues.push(Issue::new(
                    format!("plugins[{i}].use"),
                    format!("route `{key}` is also provided by `{owner}`"),
                ));
            }
        }
    }

    if !issues.is_empty() {
        return Err(PluginLoadError(issues));
    }
    start_order(&chosen).map_err(|cycle| {
        PluginLoadError(vec![Issue::new(
            "plugins",
            format!("dependency cycle between: {cycle}"),
        )])
    })
}

/// Checks, then starts every plugin listed in `config.plugins`, dependencies first.
pub async fn load(
    config: &AppConfig,
    registry: &PluginRegistry,
    providers: &CapabilityProviders,
    hooks: &Arc<HookBus>,
) -> Result<Vec<Loaded>, PluginLoadError> {
    let order = check(config, registry, providers)?;
    let interfaces = keel_sdk::interfaces::Interfaces::new();
    let mut loaded = Vec::new();
    for (i, plugin) in order {
        let m = plugin.manifest();
        let caps = providers.grant(&m.id, &m.capabilities);
        let config = config.plugins[i].config.clone();
        let mut ctx = InitContext::new(m, config, caps, hooks.clone(), interfaces.clone());
        if let Err(e) = plugin.init(&mut ctx).await {
            return Err(PluginLoadError(vec![Issue::new(
                format!("plugins[{i}]"),
                format!("`{}` failed to start: {e}", m.id),
            )]));
        }
        let routes = ctx.finish();
        let missing = routes.missing();
        if !missing.is_empty() {
            return Err(PluginLoadError(vec![Issue::new(
                format!("plugins[{i}]"),
                format!(
                    "`{}` declares routes it did not register: {}",
                    m.id,
                    missing.join(", ")
                ),
            )]));
        }
        for name in &m.consumes {
            let p = plugin.clone();
            hooks.subscribe(name.clone(), move |e: Event| {
                let p = p.clone();
                async move { p.on_event(&e).await }
            });
        }
        tracing::info!(plugin = %m.id, version = %m.version, routes = m.routes.len(), "plugin started");
        loaded.push(Loaded {
            plugin: plugin.clone(),
            router: routes.into_router(),
        });
    }
    Ok(loaded)
}

fn start_order(chosen: &[(usize, Arc<dyn Plugin>)]) -> Result<Chosen, String> {
    let mut done: BTreeSet<String> = BTreeSet::new();
    let mut order = Vec::new();
    let mut left: Chosen = chosen.to_vec();
    while !left.is_empty() {
        let before = left.len();
        left.retain(|(i, p)| {
            let ready = p.manifest().plugins.keys().all(|d| done.contains(d));
            if ready {
                done.insert(p.manifest().id.clone());
                order.push((*i, p.clone()));
            }
            !ready
        });
        if left.len() == before {
            return Err(left
                .iter()
                .map(|(_, p)| p.manifest().id.clone())
                .collect::<Vec<_>>()
                .join(", "));
        }
    }
    Ok(order)
}
