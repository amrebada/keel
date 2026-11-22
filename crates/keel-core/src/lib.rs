//! Keel host core.
//!
//! Loads and validates `app.yaml`, resolves `${secret:NAME}` references,
//! starts the compiled-in plugins it lists, runs the event hook bus, and
//! serves HTTP with a `/health` endpoint.

pub mod compose;
pub mod config;
pub mod live;
pub mod migrate;
pub mod plugins;
pub mod routes;
pub mod secrets;
pub mod server;
pub mod setup;

pub use config::{
    AppConfig, ConfigError, Issue, PluginEntry, ProviderConfig, RouteRule, SecretsConfig, ServerConfig,
    ValidateRule,
};
pub use keel_sdk::{Event, HookBus};
pub use plugins::{PluginLoadError, PluginRegistry};
pub use secrets::{EnvSecrets, MapSecrets, MissingSecret, SecretSource};
pub use server::{Host, HostError, HostOptions};
