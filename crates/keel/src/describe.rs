//! Compact plugin descriptions. `describe --ai` is what an agent reads
//! instead of source code, so it stays short: one line per fact.

use keel_sdk::Manifest;
use serde_json::{json, Map, Value};

/// A short plain-text summary, typically under 300 tokens.
pub fn ai_summary(m: &Manifest) -> String {
    let mut s = format!("{} {}: {}\n", m.id, m.version, m.summary);
    let caps: Vec<&str> = m.capabilities.iter().map(|c| c.as_str()).collect();
    let deps: Vec<String> = m.plugins.iter().map(|(id, r)| format!("{id} {r}")).collect();
    s.push_str(&format!(
        "needs: host {}; capabilities: {}; plugins: {}\n",
        m.host,
        join_or_none(&caps),
        join_or_none(&deps)
    ));
    let providers: Vec<String> = m
        .capabilities
        .iter()
        .filter_map(|c| provider_hint(c.as_str()))
        .collect();
    if !providers.is_empty() {
        s.push_str(&format!("app.yaml providers: {}\n", providers.join("; ")));
    }
    s.push_str("secrets in config: write \"${secret:NAME}\" (from env or .env beside app.yaml)\n");
    let keys = config_keys(m);
    if keys.is_empty() {
        s.push_str("config: none\n");
    } else {
        s.push_str("config:\n");
        for k in keys {
            s.push_str(&format!("  {k}\n"));
        }
    }
    if m.is_composer() {
        let parts: Vec<String> = m
            .compose
            .iter()
            .map(|c| match &c.when {
                Some(w) => format!("{} (when {w})", c.spec),
                None => c.spec.clone(),
            })
            .collect();
        s.push_str(&format!(
            "bundle: expands into {}; see the result with keel config expand; tune an included plugin with config.with.<plugin id>\n",
            parts.join(", ")
        ));
    }
    let routes: Vec<String> = m.routes.iter().map(|r| r.to_string()).collect();
    s.push_str(&format!("routes: {}\n", join_or_none(&routes)));
    s.push_str(&format!(
        "events out: {}; in: {}\n",
        join_or_none(&m.events),
        join_or_none(&m.consumes)
    ));
    if !m.interfaces.is_empty() {
        s.push_str(&format!("provides interfaces: {}\n", m.interfaces.join(", ")));
    }
    s.push_str(&format!(
        "setup steps: {}\n",
        if m.setup.is_empty() {
            "none".into()
        } else {
            m.setup.len().to_string()
        }
    ));
    s.push_str(&format!("add with: keel add {}@^{}", m.id, caret(m)));
    s
}

/// Which `providers:` entry a capability needs, with the accepted `use:` values.
fn provider_hint(cap: &str) -> Option<String> {
    let (key, uses) = match cap {
        "sql" => ("db", "postgres|mysql|sqlite, url"),
        "cache" => ("cache", "redis|memory"),
        "kv" => ("kv", "redis|memory"),
        "email" => ("email", "console|smtp|resend|ses"),
        _ => return None,
    };
    Some(format!("{key} (use: {uses})"))
}

fn caret(m: &Manifest) -> String {
    if m.version.major > 0 {
        m.version.major.to_string()
    } else {
        format!("0.{}", m.version.minor)
    }
}

fn join_or_none<S: AsRef<str>>(items: &[S]) -> String {
    if items.is_empty() {
        "none".to_string()
    } else {
        items.iter().map(|s| s.as_ref()).collect::<Vec<_>>().join(", ")
    }
}

/// One line per config key: `name: type, required, default x (live)`.
/// Nested objects, maps and list items are indented under their key, so an
/// agent sees the whole shape without the JSON schema.
fn config_keys(m: &Manifest) -> Vec<String> {
    let Some(schema) = &m.config_schema else {
        return vec![];
    };
    let mut out = Vec::new();
    walk(m, schema, "", 0, &mut out);
    out
}

fn walk(m: &Manifest, schema: &Value, prefix: &str, depth: usize, out: &mut Vec<String>) {
    let indent = "  ".repeat(depth);
    let required: Vec<&str> = schema["required"]
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    if let Some(props) = schema["properties"].as_object() {
        for (name, p) in props {
            let path = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}.{name}")
            };
            let mut parts = vec![type_of(p)];
            if required.contains(&name.as_str()) {
                parts.push("required".into());
            }
            for (k, label) in [
                ("minimum", "min"),
                ("maximum", "max"),
                ("minLength", "min length"),
                ("maxLength", "max length"),
            ] {
                if let Some(v) = p.get(k) {
                    parts.push(format!("{label} {v}"));
                }
            }
            if let Some(d) = p.get("default") {
                parts.push(format!("default {d}"));
            }
            if let Some(desc) = p.get("description").and_then(Value::as_str) {
                parts.push(desc.to_string());
            }
            let live = if m
                .runtime_config
                .iter()
                .any(|k| *k == path || k.starts_with(&format!("{path}.")))
            {
                " (live)"
            } else {
                ""
            };
            out.push(format!("{indent}{name}: {}{live}", parts.join(", ")));
            if depth < 5 {
                walk(m, p, &path, depth + 1, out);
            }
        }
    }
    if depth >= 5 {
        return;
    }
    if let Some(extra) = schema.get("additionalProperties").filter(|v| v.is_object()) {
        out.push(format!("{indent}<any name>: {}", type_of(extra)));
        walk(m, extra, &format!("{prefix}.*"), depth + 1, out);
    }
    if let Some(items) = schema.get("items").filter(|v| v["properties"].is_object()) {
        out.push(format!("{indent}- each item:"));
        walk(m, items, prefix, depth + 1, out);
    }
}

fn type_of(p: &Value) -> String {
    if let Some(e) = p.get("enum").and_then(Value::as_array) {
        return format!(
            "one of {}",
            e.iter().map(|v| v.to_string()).collect::<Vec<_>>().join("|")
        );
    }
    match p.get("type") {
        Some(Value::String(t)) if t == "array" => match p["items"].get("type").and_then(Value::as_str) {
            Some(it) => format!("array of {it}"),
            None => "array".into(),
        },
        Some(Value::String(t)) => t.clone(),
        Some(Value::Array(ts)) => ts.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("|"),
        _ => "any".into(),
    }
}

/// The whole manifest as JSON.
pub fn manifest_json(m: &Manifest) -> Value {
    json!({
        "id": m.id,
        "version": m.version.to_string(),
        "summary": m.summary,
        "tags": m.tags,
        "requires": {
            "host": m.host.to_string(),
            "capabilities": m.capabilities.iter().map(|c| c.as_str()).collect::<Vec<_>>(),
            "plugins": m.plugins.iter().map(|(k, v)| (k.clone(), Value::String(v.to_string()))).collect::<Map<_, _>>(),
        },
        "provides": {
            "interfaces": m.interfaces,
            "routes": m.routes.iter().map(|r| r.to_string()).collect::<Vec<_>>(),
            "events": m.events,
        },
        "consumes": { "events": m.consumes },
        "config_schema": m.config_schema,
        "runtime_config": m.runtime_config,
        "setup": m.setup,
        "compose": m.compose.iter().map(|c| json!({ "use": c.spec, "when": c.when, "config": c.config })).collect::<Vec<_>>(),
    })
}

/// A minimal config for `keel add`: required keys that have defaults.
pub fn minimal_config(m: &Manifest) -> Value {
    let mut out = Map::new();
    if let Some(schema) = &m.config_schema {
        let required = schema["required"].as_array().cloned().unwrap_or_default();
        for key in required.iter().filter_map(Value::as_str) {
            if let Some(d) = schema["properties"][key].get("default") {
                out.insert(key.to_string(), d.clone());
            }
        }
    }
    Value::Object(out)
}

/// The plugin's config with schema defaults filled in for missing top-level keys.
pub fn with_defaults(m: &Manifest, config: &Value) -> Value {
    m.config_with_defaults(config)
}
