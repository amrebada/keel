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
                    let m = i["message"].as_str().unwrap_or_default();
                    // Skip issues the message already says.
                    if m.is_empty() || msg.contains(m) {
                        continue;
                    }
                    text.push_str(&format!(
                        "\n  - {}: {}",
                        i["path"].as_str().unwrap_or_default(),
                        m
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

/// `cli:<user>@<host>` for the audit log.
pub(crate) fn actor() -> String {
    let from_cmd = |cmd: &str| {
        std::process::Command::new(cmd)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let user = ["USER", "LOGNAME", "USERNAME"]
        .iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
        .or_else(|| from_cmd("whoami"))
        .unwrap_or_else(|| "unknown".into());
    let host = ["HOSTNAME", "COMPUTERNAME"]
        .iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
        .or_else(|| {
            std::fs::read_to_string("/etc/hostname")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
        .or_else(|| from_cmd("hostname"));
    match host {
        Some(h) => format!("cli:{user}@{h}"),
        None => format!("cli:{user}"),
    }
}

/// Unix seconds as `YYYY-MM-DD HH:MM:SS UTC`.
fn utc(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02} UTC",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
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
                e["at"]
                    .as_i64()
                    .or_else(|| e["at"].as_str().and_then(|s| s.parse().ok()))
                    .map(utc)
                    .unwrap_or_else(|| e["at"].to_string()),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_dates() {
        assert_eq!(utc(0), "1970-01-01 00:00:00 UTC");
        assert_eq!(utc(951_782_400), "2000-02-29 00:00:00 UTC");
        assert_eq!(utc(1_795_290_691), "2026-11-21 19:51:31 UTC");
    }

    #[test]
    fn actor_names_user() {
        assert!(actor().starts_with("cli:"));
    }
}
