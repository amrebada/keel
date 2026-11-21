//! The plugins compiled into this `keel` binary.
//!
//! `keel new-plugin` adds a line between the markers below; you can also edit
//! it by hand. A plugin listed here only runs when `app.yaml` lists it.

use keel_core::PluginRegistry;

pub fn builtin() -> PluginRegistry {
    PluginRegistry::new()
        // keel:plugins
        .register(keel_plugin_auth_core::AuthCore::new())
        .register(keel_plugin_auth_mfa::AuthMfa::new())
        .register(keel_plugin_auth_oauth::AuthOauth::new())
        .register(keel_plugin_ping::Ping::new())
    // keel:plugins:end
}
