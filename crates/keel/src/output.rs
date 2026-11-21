//! Output for people and for AI agents. With `--json`, every command prints
//! one JSON object: `{"ok": true, ...}` or `{"ok": false, "error": {...}}`.

use keel_sdk::Issue;
use serde_json::{json, Value};

#[derive(Clone, Copy)]
pub struct Out {
    pub json: bool,
}

impl Out {
    /// Prints `text` for people, or `data` (with `ok: true` added) for `--json`.
    pub fn success(&self, text: impl AsRef<str>, data: Value) {
        self.report(true, text, data)
    }

    /// Prints a full result; `ok: false` sends the text to stderr.
    pub fn report(&self, ok: bool, text: impl AsRef<str>, data: Value) {
        if self.json {
            let mut data = if data.is_object() {
                data
            } else {
                json!({ "result": data })
            };
            data["ok"] = ok.into();
            println!("{}", serde_json::to_string(&data).unwrap_or_default());
        } else if !ok {
            eprintln!("{}", text.as_ref());
        } else if !text.as_ref().is_empty() {
            println!("{}", text.as_ref());
        }
    }

    pub fn failure(&self, err: &anyhow::Error) {
        if self.json {
            let issues = issues_of(err);
            let mut e = json!({ "message": headline(err) });
            if !issues.is_empty() {
                e["issues"] = issues
                    .iter()
                    .map(|i| json!({ "path": i.path, "message": i.message }))
                    .collect();
            }
            println!("{}", json!({ "ok": false, "error": e }));
        } else {
            eprintln!("error: {err:#}");
        }
    }
}

fn headline(err: &anyhow::Error) -> String {
    err.to_string().lines().next().unwrap_or_default().to_string()
}

/// Structured issues from the error types that carry them.
pub fn issues_of(err: &anyhow::Error) -> Vec<Issue> {
    if let Some(keel_core::ConfigError::Invalid { issues, .. }) = err.downcast_ref() {
        return issues.clone();
    }
    if let Some(keel_core::ConfigError::MissingSecrets { missing, .. }) = err.downcast_ref() {
        return missing
            .iter()
            .map(|m| Issue::new(&m.path, format!("secret {} is not set", m.name)))
            .collect();
    }
    if let Some(e) = err.downcast_ref::<keel_core::PluginLoadError>() {
        return e.0.clone();
    }
    if let Some(e) = err.downcast_ref::<keel_core::HostError>() {
        match e {
            keel_core::HostError::Plugins(p) => return p.0.clone(),
            keel_core::HostError::Migrations(keel_core::migrate::MigrateError::Invalid(i)) => {
                return i.clone()
            }
            _ => {}
        }
    }
    if let Some(keel_core::migrate::MigrateError::Invalid(issues)) = err.downcast_ref() {
        return issues.clone();
    }
    if let Some(keel_sdk::ManifestError::Invalid(issues)) = err.downcast_ref() {
        return issues.clone();
    }
    Vec::new()
}
