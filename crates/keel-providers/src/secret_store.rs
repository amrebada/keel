//! Secret sources chosen under `secrets:` in `app.yaml`.
//!
//! - `env` (default): process environment, which the CLI fills from `.env`
//!   beside `app.yaml` without overriding variables already set.
//! - `vault`: HashiCorp Vault KV v2. Every key at `<mount>/<path>` is read
//!   once at start. Environment variables still win, so one secret can be
//!   overridden locally.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use keel_core::{EnvSecrets, MapSecrets, SecretSource, SecretsConfig};
use serde_json::Value;

/// Looks secrets up in each layer in order; the first hit wins.
pub struct LayeredSecrets {
    layers: Vec<Arc<dyn SecretSource>>,
    /// e.g. `env`, `env + vault (secret/myapp)`.
    pub description: String,
}

impl SecretSource for LayeredSecrets {
    fn get(&self, name: &str) -> Option<String> {
        self.layers.iter().find_map(|l| l.get(name))
    }
}

impl keel_sdk::capabilities::Secrets for LayeredSecrets {
    fn get(&self, name: &str) -> Option<String> {
        SecretSource::get(self, name)
    }
}

/// Builds the secret source for `cfg`. Vault errors say what to fix.
pub async fn secret_source(cfg: Option<&SecretsConfig>) -> Result<LayeredSecrets, String> {
    let env: Arc<dyn SecretSource> = Arc::new(EnvSecrets);
    let Some(cfg) = cfg.filter(|c| c.kind != "env") else {
        return Ok(LayeredSecrets {
            layers: vec![env],
            description: "env".into(),
        });
    };
    match cfg.kind.as_str() {
        "vault" => {
            let vault = read_vault(cfg).await?;
            let mount = cfg.mount.as_deref().unwrap_or("secret");
            let path = cfg.path.as_deref().unwrap_or_default();
            Ok(LayeredSecrets {
                layers: vec![env, Arc::new(vault)],
                description: format!("env + vault ({mount}/{path})"),
            })
        }
        other => Err(format!(
            "secrets.use: unknown secret source `{other}`; use env or vault"
        )),
    }
}

async fn read_vault(cfg: &SecretsConfig) -> Result<MapSecrets, String> {
    let addr = cfg
        .url
        .clone()
        .or_else(|| std::env::var("VAULT_ADDR").ok())
        .ok_or("secrets.url: set the Vault address here or in VAULT_ADDR")?;
    let token_env = cfg.token_env.as_deref().unwrap_or("VAULT_TOKEN");
    let token = std::env::var(token_env)
        .map_err(|_| format!("secrets: the Vault token is read from {token_env}, which is not set"))?;
    let mount = cfg.mount.as_deref().unwrap_or("secret").trim_matches('/');
    let path = cfg.path.as_deref().unwrap_or_default().trim_matches('/');
    let url = format!("{}/v1/{mount}/data/{path}", addr.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;
    let mut req = client.get(&url).header("X-Vault-Token", token);
    if let Some(ns) = &cfg.namespace {
        req = req.header("X-Vault-Namespace", ns);
    }
    let res = req
        .send()
        .await
        .map_err(|e| format!("secrets: cannot reach Vault at {addr}: {e}"))?;
    let status = res.status();
    if !status.is_success() {
        let hint = match status.as_u16() {
            403 => "the token is invalid or lacks read access to this path",
            404 => "nothing is stored at this mount and path",
            _ => "Vault refused the request",
        };
        return Err(format!(
            "secrets: Vault answered {status} for {mount}/{path}: {hint}"
        ));
    }
    let body: Value = res
        .json()
        .await
        .map_err(|e| format!("secrets: Vault sent an unreadable answer: {e}"))?;
    let data = body
        .pointer("/data/data")
        .and_then(Value::as_object)
        .ok_or("secrets: Vault answer has no data.data; is the mount KV version 2?")?;
    let map: HashMap<String, String> = data
        .iter()
        .map(|(k, v)| {
            let v = match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            (k.clone(), v)
        })
        .collect();
    tracing::info!(keys = map.len(), %mount, %path, "secrets loaded from Vault");
    Ok(MapSecrets(map))
}
