//! CLI commands that change a running server through its admin API:
//! `keel apply`, `keel config pull`, `keel plugins enable|disable|set` and
//! `keel audit`.

use std::path::Path;

use anyhow::{bail, Context, Result};
use keel_core::live::ADMIN_TOKEN;
use keel_core::{AppConfig, SecretSource};
use reqwest::Method;
use serde_json::{json, Value};

use crate::output::Out;
use crate::{secrets_for, KeepRefs};

pub struct Admin {
    base: String,
    token: String,
    http: reqwest::Client,
}

impl Admin {
    /// The server from `server:` in app.yaml (or `url`), with the admin
    /// token from the app's secret source.
    pub async fn new(config: &Path, url: Option<&str>) -> Result<Self> {
        crate::dotenv::load_beside(config);
        let token = secrets_for(config)
            .await?
            .get(ADMIN_TOKEN)
            .filter(|t| !t.is_empty())
            .with_context(|| {
                format!(
                    "set the {ADMIN_TOKEN} secret (env or .env beside app.yaml) to the value the server uses"
                )
            })?;
        let base = match url {
            Some(u) => u.trim_end_matches('/').to_string(),
            None => {
                let c = AppConfig::load_file(config, &KeepRefs)?;
                let host = match c.server.host.as_str() {
                    "0.0.0.0" | "::" | "[::]" => "127.0.0.1".to_string(),
                    h => h.to_string(),
                };
                format!("http://{host}:{}", c.server.port)
            }
        };
        Ok(Self {
            base,
            token,
            http: reqwest::Client::builder().no_proxy().build()?,
        })
    }

    async fn call(&self, method: Method, path: &str, body: Option<Body>) -> Result<Value> {
        let mut req = self
            .http
            .request(method, format!("{}/_admin{path}", self.base))
            .bearer_auth(&self.token)
            .header("x-keel-actor", actor());
        req = match body {
            Some(Body::Json(v)) => req.json(&v),
            Some(Body::Text(t)) => req.header("content-type", "application/yaml").body(t),
            None => req,
        };
        let res = req
            .send()
            .await
            .with_context(|| format!("cannot reach the server at {}; is it running?", self.base))?;
        let status = res.status();
        let v: Value = res.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            let msg = v["error"]["message"].as_str().unwrap_or("request failed");
            let mut text = format!("{} ({})", msg, status.as_u16());
            if let Some(list) = v["error"]["issues"].as_array() {
                for i in list {
                    text.push_str(&format!(
                        "\n  - {}: {}",
                        i["path"].as_str().unwrap_or_default(),
                        i["message"].as_str().unwrap_or_default()
                    ));
                }
            }
            bail!("{text}");
        }
        Ok(v)
    }
}

enum Body {
    Json(Value),
    Text(String),
}

fn actor() -> String {
    let user = std::env::var("USER").unwrap_or_else(|_| "unknown".into());
    format!("cli:{user}")
}

fn summary(c: &Value) -> String {
    let list = |k: &str| {
        c[k].as_array()
            .map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "))
            .filter(|s| !s.is_empty())
    };
    match c["mode"].as_str() {
        Some("none") => format!("nothing to change (version {})", c["version"]),
        Some("live") => format!(
            "changed in place, no restart (version {}): {}",
            c["version"],
            list("changed").unwrap_or_default()
        ),
        _ => {
            let mut parts = vec![format!(
                "applied live (version {}, generation {})",
                c["version"], c["generation"]
            )];
            for (k, label) in [
                ("added", "added"),
                ("removed", "removed"),
                ("changed", "changed"),
                ("migrations", "migrations"),
            ] {
                if let Some(l) = list(k) {
                    parts.push(format!("  {label}: {l}"));
                }
            }
            parts.join("\n")
        }
    }
}

pub async fn apply(out: Out, config: &Path, url: Option<&str>) -> Result<()> {
    let text =
        std::fs::read_to_string(config).with_context(|| format!("{}: cannot read file", config.display()))?;
    let admin = Admin::new(config, url).await?;
    let c = admin.call(Method::PUT, "/config", Some(Body::Text(text))).await?;
    out.success(summary(&c), c);
    Ok(())
}

pub async fn pull(out: Out, config: &Path, url: Option<&str>, write: bool) -> Result<()> {
    let admin = Admin::new(config, url).await?;
    let v = admin.call(Method::GET, "/config", None).await?;
    let yaml = v["yaml"].as_str().unwrap_or_default().to_string();
    if write {
        std::fs::write(config, &yaml)?;
        out.success(
            format!(
                "wrote the running config (version {}) to {}; comments are not kept",
                v["version"],
                config.display()
            ),
            v,
        );
    } else {
        out.success(yaml.trim_end(), v);
    }
    Ok(())
}

pub async fn enable(out: Out, config: &Path, url: Option<&str>, spec: &str, cfg: Option<&str>) -> Result<()> {
    let mut entry = json!({ "use": spec });
    if let Some(c) = cfg {
        entry["config"] = serde_yaml::from_str::<Value>(c).context("--config must be JSON or YAML")?;
    }
    let admin = Admin::new(config, url).await?;
    let c = admin
        .call(Method::POST, "/plugins", Some(Body::Json(entry)))
        .await?;
    out.success(summary(&c), c);
    Ok(())
}

pub async fn disable(out: Out, config: &Path, url: Option<&str>, id: &str) -> Result<()> {
    let admin = Admin::new(config, url).await?;
    let c = admin
        .call(Method::DELETE, &format!("/plugins/{id}"), None)
        .await?;
    out.success(summary(&c), c);
    Ok(())
}

/// `key=value` pairs; values are read as YAML (`3`, `true`, `[a, b]`, text).
pub async fn set(out: Out, config: &Path, url: Option<&str>, id: &str, pairs: &[String]) -> Result<()> {
    let mut patch = serde_json::Map::new();
    for p in pairs {
        let Some((k, v)) = p.split_once('=') else {
            bail!("`{p}`: use key=value");
        };
        let value: Value = serde_yaml::from_str(v).unwrap_or(Value::String(v.to_string()));
        patch.insert(k.trim().to_string(), value);
    }
    let admin = Admin::new(config, url).await?;
    let c = admin
        .call(
            Method::PATCH,
            &format!("/plugins/{id}/config"),
            Some(Body::Json(Value::Object(patch))),
        )
        .await?;
    out.success(summary(&c), c);
    Ok(())
}

pub async fn audit(out: Out, config: &Path, url: Option<&str>, limit: u32) -> Result<()> {
    let admin = Admin::new(config, url).await?;
    let v = admin
        .call(Method::GET, &format!("/audit?limit={limit}"), None)
        .await?;
    let mut text = String::new();
    if let Some(list) = v["entries"].as_array() {
        for e in list {
            text.push_str(&format!(
                "{}  {:<18} {:<20} {}  {}\n",
                e["at"],
                e["action"].as_str().unwrap_or_default(),
                e["target"].as_str().unwrap_or_default(),
                e["actor"].as_str().unwrap_or_default(),
                e["detail"]
            ));
        }
    }
    if text.is_empty() {
        text = "no entries".into();
    }
    out.success(text.trim_end(), v);
    Ok(())
}
