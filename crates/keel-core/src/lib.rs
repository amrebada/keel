//! Keel host core.
//!
//! Phase 1 (E1) of the v1 spec: load and validate `app.yaml`, resolve
//! `${secret:NAME}` references, run an event hook bus, and serve HTTP with a
//! `/health` endpoint. Plugins plug into this in later phases.

pub mod config;
pub mod hooks;
pub mod secrets;
pub mod server;

pub use config::{AppConfig, ConfigError, Issue, PluginEntry, ProviderConfig, RouteRule, ServerConfig};
pub use hooks::{Event, HookBus};
pub use secrets::{EnvSecrets, MapSecrets, MissingSecret, SecretSource};
pub use server::Host;
