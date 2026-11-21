//! Secret resolution for `${secret:NAME}` references in config.
//!
//! Secrets never appear inline in `app.yaml`. v1 reads them from environment
//! variables; other backends (Vault, AWS) implement [`SecretSource`] later.

use std::collections::HashMap;

use serde_json::Value;

/// Somewhere secrets can be looked up by name.
pub trait SecretSource: Send + Sync {
    fn get(&self, name: &str) -> Option<String>;
}

/// Reads secrets from process environment variables.
#[derive(Debug, Default, Clone, Copy)]
pub struct EnvSecrets;

impl SecretSource for EnvSecrets {
    fn get(&self, name: &str) -> Option<String> {
        std::env::var(name).ok()
    }
}

/// Fixed in-memory secrets, mainly for tests and demos.
#[derive(Debug, Default, Clone)]
pub struct MapSecrets(pub HashMap<String, String>);

impl MapSecrets {
    pub fn new<I, K, V>(pairs: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        Self(pairs.into_iter().map(|(k, v)| (k.into(), v.into())).collect())
    }
}

impl SecretSource for MapSecrets {
    fn get(&self, name: &str) -> Option<String> {
        self.0.get(name).cloned()
    }
}

/// A `${secret:NAME}` reference that could not be resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingSecret {
    /// Secret name, e.g. `DATABASE_URL`.
    pub name: String,
    /// Where in the config it was referenced, e.g. `providers.db.url`.
    pub path: String,
}

const PREFIX: &str = "${secret:";

/// Replaces every `${secret:NAME}` in string values of `value`, in place.
/// Returns every reference that could not be resolved.
pub fn resolve(value: &mut Value, source: &dyn SecretSource) -> Vec<MissingSecret> {
    let mut missing = Vec::new();
    walk(value, source, String::new(), &mut missing);
    missing
}

fn walk(value: &mut Value, source: &dyn SecretSource, path: String, missing: &mut Vec<MissingSecret>) {
    match value {
        Value::String(s) if s.contains(PREFIX) => {
            *s = substitute(s, source, &path, missing);
        }
        Value::Array(items) => {
            for (i, item) in items.iter_mut().enumerate() {
                walk(item, source, format!("{path}[{i}]"), missing);
            }
        }
        Value::Object(map) => {
            for (key, item) in map.iter_mut() {
                let child = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                walk(item, source, child, missing);
            }
        }
        _ => {}
    }
}

fn substitute(
    input: &str,
    source: &dyn SecretSource,
    path: &str,
    missing: &mut Vec<MissingSecret>,
) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find(PREFIX) {
        out.push_str(&rest[..start]);
        let after = &rest[start + PREFIX.len()..];
        match after.find('}') {
            Some(end) => {
                let name = &after[..end];
                match source.get(name) {
                    Some(v) => out.push_str(&v),
                    None => {
                        missing.push(MissingSecret {
                            name: name.to_string(),
                            path: path.to_string(),
                        });
                        out.push_str(&rest[start..start + PREFIX.len() + end + 1]);
                    }
                }
                rest = &after[end + 1..];
            }
            None => {
                // Unterminated reference: keep the text as-is.
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

impl keel_sdk::capabilities::Secrets for EnvSecrets {
    fn get(&self, name: &str) -> Option<String> {
        std::env::var(name).ok()
    }
}

impl keel_sdk::capabilities::Secrets for MapSecrets {
    fn get(&self, name: &str) -> Option<String> {
        self.0.get(name).cloned()
    }
}
