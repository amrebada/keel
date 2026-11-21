//! Setup steps: the human actions a plugin needs (accounts, keys, DNS), each
//! with a check the host can run. Declared under `setup:` in `plugin.yaml`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One step from `setup:` in `plugin.yaml`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct SetupStep {
    pub id: String,
    pub title: String,
    pub kind: SetupKind,
    /// A path into the plugin's config, e.g. `config.providers.google`. The
    /// step applies only when that value is present and not `false`/`null`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<String>,
    /// Instructions for the user, in order. `{config.x}` is replaced with the
    /// plugin's config value.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<String>,
    /// What the user ends up with, e.g. `secret:GOOGLE_CLIENT_ID`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub produces: Vec<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub requires_paid_account: bool,
    /// How the host checks the step. Defaults to `secret-present` for the
    /// secrets in `produces`, or `manual` when it produces none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verify: Option<Verify>,
    /// Routes that answer `503 setup_incomplete` until the step is done:
    /// `/path`, `/prefix/*` or `METHOD /path`. Defaults to every route of
    /// the plugin.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocks: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SetupKind {
    ExternalAccount,
    Secret,
    Dns,
    Webhook,
    Manual,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Verify {
    #[serde(rename = "type")]
    pub kind: VerifyKind,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keys: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum VerifyKind {
    /// Done when every key is a set secret.
    SecretPresent,
    /// Done when [`crate::Plugin::check_setup`] returns `Ok`.
    PluginCheck,
    /// Done when the user runs `keel setup done <id>`.
    Manual,
}

impl SetupStep {
    /// Secret names from `produces`, without the `secret:` prefix.
    pub fn secrets(&self) -> Vec<String> {
        self.produces
            .iter()
            .filter_map(|p| p.strip_prefix("secret:"))
            .map(str::to_string)
            .collect()
    }

    /// The check to run, with defaults applied.
    pub fn verify(&self) -> Verify {
        if let Some(v) = &self.verify {
            if v.kind == VerifyKind::SecretPresent && v.keys.is_empty() {
                return Verify {
                    kind: v.kind,
                    keys: self.secrets(),
                };
            }
            return v.clone();
        }
        let keys = self.secrets();
        if keys.is_empty() {
            Verify {
                kind: VerifyKind::Manual,
                keys,
            }
        } else {
            Verify {
                kind: VerifyKind::SecretPresent,
                keys,
            }
        }
    }

    /// Whether the step applies to this plugin config (see `when`).
    pub fn applies(&self, config: &Value) -> bool {
        let Some(when) = &self.when else {
            return true;
        };
        let mut v = config;
        for key in when.split('.').skip(1) {
            match v.get(key) {
                Some(next) => v = next,
                None => return false,
            }
        }
        !matches!(v, Value::Null | Value::Bool(false))
    }

    /// `steps` with `{config.a.b}` replaced by values from `config`.
    pub fn instructions(&self, config: &Value) -> Vec<String> {
        self.steps.iter().map(|s| fill(s, config)).collect()
    }

    /// Whether this step blocks `method path`. `routes` are the plugin's own
    /// declared routes, used when `blocks` is empty.
    pub fn blocks_request(&self, routes: &[crate::Route], method: &str, path: &str) -> bool {
        if self.blocks.is_empty() {
            return routes
                .iter()
                .any(|r| r.method == method && path_matches(&r.path, path));
        }
        self.blocks.iter().any(|b| match b.split_once(' ') {
            Some((m, p)) => m == method && path_matches(p, path),
            None => path_matches(b, path),
        })
    }
}

fn fill(text: &str, config: &Value) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("{config.") {
        out.push_str(&rest[..start]);
        let after = &rest[start..];
        let Some(end) = after.find('}') else {
            out.push_str(after);
            return out;
        };
        let path = &after[1..end];
        let mut v = config;
        let mut found = true;
        for key in path.split('.').skip(1) {
            match v.get(key) {
                Some(n) => v = n,
                None => {
                    found = false;
                    break;
                }
            }
        }
        match (found, v) {
            (true, Value::String(s)) => out.push_str(s),
            (true, Value::Null) | (false, _) => out.push_str(&after[..=end]),
            (true, other) => out.push_str(&other.to_string()),
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

/// Matches a request path against a pattern: `{name}` matches one segment,
/// a trailing `*` matches the rest.
pub fn path_matches(pattern: &str, path: &str) -> bool {
    if let Some(prefix) = pattern.strip_suffix('*') {
        let prefix = prefix.trim_end_matches('/');
        return segments_match(prefix, path, true);
    }
    segments_match(pattern, path, false)
}

fn segments_match(pattern: &str, path: &str, prefix: bool) -> bool {
    let p: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
    let r: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if r.len() < p.len() || (!prefix && r.len() != p.len()) {
        return false;
    }
    p.iter()
        .zip(&r)
        .all(|(p, r)| (p.starts_with('{') && p.ends_with('}')) || p == r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn step(yaml: &str) -> SetupStep {
        serde_yaml::from_str(yaml).unwrap()
    }

    #[test]
    fn defaults_and_when() {
        let s = step("id: g\ntitle: G\nkind: external-account\nwhen: config.providers.google\nproduces: [secret:A, secret:B]\nsteps: ['Redirect {config.app_url}/cb']");
        assert_eq!(s.verify().kind, VerifyKind::SecretPresent);
        assert_eq!(s.verify().keys, vec!["A", "B"]);
        assert!(s.applies(&json!({"providers": {"google": {}}})));
        assert!(!s.applies(&json!({"providers": {}})));
        assert!(!s.applies(&json!({"providers": {"google": false}})));
        assert_eq!(
            s.instructions(&json!({"app_url": "http://x"})),
            vec!["Redirect http://x/cb"]
        );
        let m = step("id: m\ntitle: M\nkind: manual");
        assert_eq!(m.verify().kind, VerifyKind::Manual);
    }

    #[test]
    fn blocking() {
        let routes = vec![crate::Route {
            method: "POST".into(),
            path: "/auth/login".into(),
        }];
        let all = step("id: a\ntitle: A\nkind: secret");
        assert!(all.blocks_request(&routes, "POST", "/auth/login"));
        assert!(!all.blocks_request(&routes, "GET", "/auth/login"));
        let g = step("id: g\ntitle: G\nkind: secret\nblocks: ['/auth/oauth/google/*', 'GET /x/{id}']");
        assert!(g.blocks_request(&routes, "GET", "/auth/oauth/google/start"));
        assert!(!g.blocks_request(&routes, "GET", "/auth/oauth/github/start"));
        assert!(g.blocks_request(&routes, "GET", "/x/42"));
        assert!(!g.blocks_request(&routes, "POST", "/x/42"));
        assert!(path_matches("/auth/oauth/{p}/start", "/auth/oauth/x/start"));
    }
}
