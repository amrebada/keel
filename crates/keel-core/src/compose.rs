//! Expands composer plugins (bundles) in `app.yaml` into the plugins they
//! list, before any check or start.
//!
//! In a composer's `compose:` templates:
//! - `"${config.a.b}"` (the whole string) is replaced by that value of the
//!   composer's config (defaults filled in); inside a longer string it is
//!   interpolated as text. Keys whose value ends up null are dropped.
//! - `when: config.x` keeps the entry only when the value is set and not
//!   false, empty or zero.
//! - `{ $keys: "${config.list}", value: {...}, overrides: { name: {...} } }`
//!   turns a list of names into a map (each name gets `value`, merged with
//!   its `overrides` entry); a map passes through unchanged.
//!
//! Every composer also accepts `with: { <plugin id>: {...} }` in its config,
//! deep-merged into that plugin's generated config, for settings the
//! composer does not expose.

use serde_json::{Map, Value};

use keel_sdk::Issue;

use crate::config::{AppConfig, PluginEntry};
use crate::plugins::{PluginLoadError, PluginRegistry};

/// `config` with every composer replaced, and for each resulting plugin
/// entry the composer it came from, if any.
#[derive(Debug, Clone)]
pub struct Expanded {
    pub config: AppConfig,
    pub origins: Vec<Option<String>>,
}

/// Replaces composer entries with the plugins they expand into.
pub fn expand(config: &AppConfig, registry: &PluginRegistry) -> Result<Expanded, PluginLoadError> {
    let mut issues = Vec::new();
    let mut plugins = Vec::new();
    let mut origins: Vec<Option<String>> = Vec::new();
    for (i, entry) in config.plugins.iter().enumerate() {
        expand_entry(
            entry,
            &format!("plugins[{i}]"),
            None,
            registry,
            0,
            &mut plugins,
            &mut origins,
            &mut issues,
        );
    }
    // A plugin both listed and brought in by a composer.
    let mut first: std::collections::BTreeMap<String, usize> = Default::default();
    for (i, p) in plugins.iter().enumerate() {
        if let Some(prev) = first.insert(p.name().to_string(), i) {
            let from = origins[i].as_ref().or(origins[prev].as_ref());
            if let Some(c) = from {
                issues.push(Issue::new(
                    "plugins",
                    format!(
                        "`{}` comes from `{c}` and is listed again; remove the extra entry and put its settings under `{c}`'s config as with: {{ {}: {{ ... }} }}",
                        p.name(),
                        p.name()
                    ),
                ));
            }
        }
    }
    if !issues.is_empty() {
        return Err(PluginLoadError(issues));
    }
    let mut out = config.clone();
    out.plugins = plugins;
    Ok(Expanded { config: out, origins })
}

#[allow(clippy::too_many_arguments)]
fn expand_entry(
    entry: &PluginEntry,
    at: &str,
    origin: Option<&str>,
    registry: &PluginRegistry,
    depth: usize,
    plugins: &mut Vec<PluginEntry>,
    origins: &mut Vec<Option<String>>,
    issues: &mut Vec<Issue>,
) {
    let composer = registry
        .get(entry.name())
        .map(|p| p.manifest())
        .filter(|m| m.is_composer());
    let Some(m) = composer else {
        plugins.push(entry.clone());
        origins.push(origin.map(String::from));
        return;
    };
    if depth > 4 {
        issues.push(Issue::new(
            format!("{at}.use"),
            "composers nest more than 4 deep; check for a cycle",
        ));
        return;
    }
    if let Some(req) = entry.version_req() {
        match semver::VersionReq::parse(req) {
            Ok(r) if !r.matches(&m.version) => {
                issues.push(Issue::new(
                    format!("{at}.use"),
                    format!(
                        "`{}@{req}` asked for, but this build has version {}",
                        m.id, m.version
                    ),
                ));
                return;
            }
            Ok(_) => {}
            Err(e) => {
                issues.push(Issue::new(
                    format!("{at}.use"),
                    format!("`{req}` is not a version range: {e}"),
                ));
                return;
            }
        }
    }
    let mut cfg = entry.config.as_object().cloned().unwrap_or_default();
    let with = cfg.remove("with").unwrap_or(Value::Object(Map::new()));
    let cfg = Value::Object(cfg);
    let before = issues.len();
    for i in m.validate_config(&cfg) {
        let p = if i.path == "(root)" {
            format!("{at}.config")
        } else {
            format!("{at}.config.{}", i.path)
        };
        issues.push(Issue::new(p, i.message));
    }
    let Some(with) = with.as_object() else {
        issues.push(Issue::new(
            format!("{at}.config.with"),
            "must map plugin ids to config, e.g. with: { auth-core: { ... } }",
        ));
        return;
    };
    for k in with.keys() {
        if !m.compose.iter().any(|c| c.id() == k) {
            issues.push(Issue::new(
                format!("{at}.config.with.{k}"),
                format!(
                    "`{}` does not include `{k}`; it includes: {}",
                    m.id,
                    m.compose.iter().map(|c| c.id()).collect::<Vec<_>>().join(", ")
                ),
            ));
        }
    }
    if issues.len() > before {
        return;
    }
    let ctx = serde_json::json!({ "config": m.config_with_defaults(&cfg) });
    let origin = origin.unwrap_or(&m.id).to_string();
    for (j, c) in m.compose.iter().enumerate() {
        if let Some(w) = &c.when {
            if !truthy(lookup(&ctx, w)) {
                continue;
            }
        }
        let mut config = match render(&c.config, &ctx, None) {
            Ok(Value::Null) => Value::Object(Map::new()),
            Ok(v) => v,
            Err(e) => {
                issues.push(Issue::new(
                    format!("{at}.use"),
                    format!("`{}` compose[{j}] ({}): {e}", m.id, c.id()),
                ));
                continue;
            }
        };
        if let Some(extra) = with.get(c.id()) {
            merge(&mut config, extra);
        }
        let generated = PluginEntry {
            spec: c.spec.clone(),
            config,
            degrade_level: None,
        };
        expand_entry(
            &generated,
            &format!("{at} ({} > {})", m.id, c.id()),
            Some(&origin),
            registry,
            depth + 1,
            plugins,
            origins,
            issues,
        );
    }
}

/// Value at a dotted path such as `config.social`.
fn lookup<'a>(ctx: &'a Value, path: &str) -> Option<&'a Value> {
    path.trim().split('.').try_fold(ctx, |v, k| v.get(k))
}

fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) | Some(Value::Bool(false)) => false,
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Number(n)) => n.as_f64() != Some(0.0),
        Some(_) => true,
    }
}

/// `${path}` alone becomes the value; inside text it is interpolated.
fn render_str(s: &str, ctx: &Value, item: Option<&str>) -> Result<Value, String> {
    let get = |path: &str| -> Value {
        if path == "item" {
            return item.map(Value::from).unwrap_or(Value::Null);
        }
        lookup(ctx, path).cloned().unwrap_or(Value::Null)
    };
    if let Some(inner) = s.strip_prefix("${").and_then(|r| r.strip_suffix('}')) {
        if !inner.contains("${") && !inner.contains('}') {
            return Ok(get(inner));
        }
    }
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find("${") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 2..];
        let end = after
            .find('}')
            .ok_or_else(|| format!("unclosed `${{` in `{s}`"))?;
        match get(&after[..end]) {
            Value::Null => {}
            Value::String(v) => out.push_str(&v),
            v => out.push_str(&v.to_string()),
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(Value::String(out))
}

fn render(t: &Value, ctx: &Value, item: Option<&str>) -> Result<Value, String> {
    Ok(match t {
        Value::String(s) => render_str(s, ctx, item)?,
        Value::Array(a) => Value::Array(a.iter().map(|v| render(v, ctx, item)).collect::<Result<_, _>>()?),
        Value::Object(o) if o.contains_key("$keys") => {
            let source = render(&o["$keys"], ctx, item)?;
            match source {
                Value::Null => Value::Null,
                Value::Object(m) => Value::Object(m),
                Value::Array(names) => {
                    let template = o.get("value").cloned().unwrap_or(Value::Object(Map::new()));
                    let overrides = match o.get("overrides") {
                        Some(ov) => render(ov, ctx, item)?,
                        None => Value::Null,
                    };
                    let mut out = Map::new();
                    for n in names {
                        let name = n
                            .as_str()
                            .ok_or_else(|| format!("$keys items must be names, got {n}"))?;
                        let mut v = render(&template, ctx, Some(name))?;
                        if let Some(extra) = overrides.get(name) {
                            merge(&mut v, extra);
                        }
                        out.insert(name.to_string(), v);
                    }
                    Value::Object(out)
                }
                other => return Err(format!("$keys needs a list or a map, got {other}")),
            }
        }
        Value::Object(o) => {
            let mut out = Map::new();
            for (k, v) in o {
                let key = match render_str(k, ctx, item)? {
                    Value::String(s) => s,
                    Value::Null => continue,
                    other => other.to_string(),
                };
                let v = render(v, ctx, item)?;
                if !v.is_null() {
                    out.insert(key, v);
                }
            }
            Value::Object(out)
        }
        other => other.clone(),
    })
}

/// Deep-merges `extra` into `base`: objects merge, anything else replaces.
fn merge(base: &mut Value, extra: &Value) {
    match (base, extra) {
        (Value::Object(b), Value::Object(e)) => {
            for (k, v) in e {
                match b.get_mut(k) {
                    Some(slot) => merge(slot, v),
                    None => {
                        b.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (b, e) => *b = e.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn templates() {
        let ctx = json!({ "config": { "social": ["google", "apple"], "roles": ["admin", "member"], "admin": "admin", "url": "https://x.test", "none": null } });
        let t = json!({
            "providers": { "$keys": "${config.social}" },
            "roles": { "$keys": "${config.roles}", "value": { "permissions": [] }, "overrides": { "${config.admin}": { "permissions": ["*"] } } },
            "base": "${config.url}/auth",
            "gone": "${config.none}",
            "list": "${config.social}"
        });
        assert_eq!(
            render(&t, &ctx, None).unwrap(),
            json!({
                "providers": { "google": {}, "apple": {} },
                "roles": { "admin": { "permissions": ["*"] }, "member": { "permissions": [] } },
                "base": "https://x.test/auth",
                "list": ["google", "apple"]
            })
        );
        assert!(truthy(lookup(&ctx, "config.social")));
        assert!(!truthy(lookup(&ctx, "config.none")));
        assert!(!truthy(lookup(&ctx, "config.missing")));
        let mut b = json!({ "a": { "b": 1, "c": 2 } });
        merge(&mut b, &json!({ "a": { "c": 3 }, "d": true }));
        assert_eq!(b, json!({ "a": { "b": 1, "c": 3 }, "d": true }));
    }
}
