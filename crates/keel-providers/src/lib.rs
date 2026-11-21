//! Keel data providers.
//!
//! `app.yaml` chooses one provider per slot:
//!
//! | Slot | Capability | Providers |
//! | --- | --- | --- |
//! | `db` | `sql` | `postgres`, `mysql`, `sqlite` |
//! | `cache` | `cache` | `redis`, `memory` |
//! | `kv` | `kv` | `redis`, `memory` |
//!
//! Plugins never see which one is in use, so switching SQLite for Postgres
//! is a YAML change.

mod memory;
mod redis_store;
mod sql;

use std::collections::BTreeMap;
use std::sync::Arc;

use keel_core::ProviderConfig;
use keel_sdk::capabilities::{Capability, CapabilityProviders, ProviderInfo};
use serde_json::Value;

pub use memory::MemoryStore;
pub use redis_store::RedisStore;
pub use sql::SqlProvider;

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("providers.{slot}: `{kind}` cannot be used here; use one of: {allowed}")]
    WrongKind {
        slot: String,
        kind: String,
        allowed: &'static str,
    },
    #[error("providers.{slot}: `url` is required for {kind}")]
    MissingUrl { slot: String, kind: String },
    #[error("providers.{slot}: cannot connect to {kind}: {message}")]
    Connect {
        slot: String,
        kind: String,
        message: String,
    },
    #[error("providers.{0}: unknown slot; use db, cache or kv")]
    UnknownSlot(String),
}

/// Connects every provider in `providers` and returns them ready to grant to
/// plugins. Fails on the first provider that cannot connect.
pub async fn connect(
    providers: &BTreeMap<String, ProviderConfig>,
) -> Result<CapabilityProviders, ProviderError> {
    let mut out = CapabilityProviders::default();
    for (slot, cfg) in providers {
        let kind = cfg.kind.as_str();
        let url = || {
            cfg.settings
                .get("url")
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| ProviderError::MissingUrl {
                    slot: slot.clone(),
                    kind: kind.to_string(),
                })
        };
        let prefix = cfg
            .settings
            .get("prefix")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let connect_err = |e: String| ProviderError::Connect {
            slot: slot.clone(),
            kind: kind.to_string(),
            message: e,
        };
        match slot.as_str() {
            "db" => {
                if !matches!(kind, "postgres" | "mysql" | "sqlite") {
                    return Err(ProviderError::WrongKind {
                        slot: slot.clone(),
                        kind: kind.into(),
                        allowed: "postgres, mysql, sqlite",
                    });
                }
                let max = cfg
                    .settings
                    .get("max_connections")
                    .and_then(Value::as_u64)
                    .map(|n| n as u32);
                let p = SqlProvider::connect(&url()?, max).await.map_err(connect_err)?;
                if p.dialect_name() != kind {
                    return Err(connect_err(format!(
                        "the url is for {}, not {kind}",
                        p.dialect_name()
                    )));
                }
                out.sql = Some(Arc::new(p));
                out.info.push(info(slot, kind, Capability::Sql, false));
            }
            "cache" | "kv" => {
                let cap = if slot == "cache" {
                    Capability::Cache
                } else {
                    Capability::Kv
                };
                match kind {
                    "memory" => {
                        let m = Arc::new(MemoryStore::with_prefix(prefix));
                        if cap == Capability::Cache {
                            out.cache = Some(m);
                        } else {
                            out.kv = Some(m);
                        }
                    }
                    "redis" => {
                        let r = Arc::new(RedisStore::connect(&url()?, prefix).await.map_err(connect_err)?);
                        if cap == Capability::Cache {
                            out.cache = Some(r);
                        } else {
                            out.kv = Some(r);
                        }
                    }
                    _ => {
                        return Err(ProviderError::WrongKind {
                            slot: slot.clone(),
                            kind: kind.into(),
                            allowed: "redis, memory",
                        })
                    }
                }
                out.info.push(info(slot, kind, cap, true));
            }
            _ => return Err(ProviderError::UnknownSlot(slot.clone())),
        }
        tracing::info!(slot = %slot, kind = %kind, "provider connected");
    }
    Ok(out)
}

fn info(slot: &str, kind: &str, capability: Capability, hot_load: bool) -> ProviderInfo {
    ProviderInfo {
        slot: slot.to_string(),
        kind: kind.to_string(),
        capability,
        hot_load,
    }
}
