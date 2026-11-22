//! Keel plugin SDK.
//!
//! A Keel plugin is a Rust crate with three parts:
//! - `plugin.yaml`, the manifest an AI reads to use the plugin ([`Manifest`]),
//! - `config.schema.json`, the JSON Schema for its `config:` block in `app.yaml`,
//! - a type implementing [`Plugin`].
//!
//! Plugins are compiled into the host binary. They talk to the outside world
//! only through capabilities ([`capabilities`]) and events ([`HookBus`]).

pub mod auth;
pub mod authz;
pub mod capabilities;
pub mod composer;
pub mod hooks;
pub mod http;
pub mod interfaces;
pub mod issues;
pub mod manifest;
pub mod migrations;
pub mod plugin;
pub mod setup;

pub use async_trait::async_trait;
pub use capabilities::{Capabilities, Capability, CapabilityError};
pub use hooks::{Event, HookBus};
pub use http::ApiError;
pub use issues::Issue;
pub use manifest::{ComposeEntry, Manifest, ManifestError, Route};
pub use migrations::Migration;
pub use plugin::{Emitter, Health, InitContext, Plugin, PluginError, Routes};
pub use setup::SetupStep;

/// Re-exports plugin crates commonly need, so they only depend on `keel-sdk`.
pub mod prelude {
    pub use crate::{
        async_trait, ApiError, Event, Health, InitContext, Manifest, Migration, Plugin, PluginError,
    };
    pub use axum::{extract, response, Json};
    pub use serde_json::{json, Value};
}

/// The host version, used to check `requires.host` in manifests.
pub const HOST_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Loads a plugin's `plugin.yaml` and `config.schema.json` at compile time.
///
/// ```ignore
/// fn manifest() -> &'static keel_sdk::Manifest { keel_sdk::manifest!() }
/// ```
/// Panics on first use if the manifest is invalid; `keel plugin check`
/// and the plugin's own tests catch that before it ships.
#[macro_export]
macro_rules! manifest {
    () => {{
        static M: ::std::sync::OnceLock<$crate::Manifest> = ::std::sync::OnceLock::new();
        M.get_or_init(|| {
            $crate::Manifest::from_sources(
                include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/plugin.yaml")),
                Some(include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/config.schema.json"
                ))),
            )
            .unwrap_or_else(|e| panic!("invalid plugin manifest in {}: {e}", env!("CARGO_PKG_NAME")))
        })
    }};
}
