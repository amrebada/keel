//! saas-auth expands into its plugins, which then pass the host's checks.

use std::sync::Arc;

use keel_core::{AppConfig, MapSecrets, PluginRegistry};
use keel_providers::{ConsoleEmail, DefaultCrypto, MemoryStore, ReqwestClient, SqlProvider};
use keel_sdk::capabilities::CapabilityProviders;
use serde_json::json;

fn registry() -> PluginRegistry {
    PluginRegistry::new()
        .register(keel_plugin_auth_core::AuthCore::new())
        .register(keel_plugin_auth_oauth::AuthOauth::new())
        .register(keel_plugin_auth_mfa::AuthMfa::new())
        .register(keel_plugin_rbac::Rbac::new())
        .register(keel_plugin_saas_auth::SaasAuth::new())
}

fn load(yaml: &str) -> AppConfig {
    AppConfig::load_str(yaml, "app.yaml", &MapSecrets::default()).unwrap()
}

async fn providers() -> CapabilityProviders {
    CapabilityProviders {
        sql: Some(Arc::new(
            SqlProvider::connect("sqlite::memory:", None).await.unwrap(),
        )),
        cache: Some(Arc::new(MemoryStore::new())),
        crypto: Some(Arc::new(DefaultCrypto)),
        email: Some(Arc::new(ConsoleEmail::default())),
        http: Some(Arc::new(ReqwestClient::default())),
        secrets: Some(Arc::new(std::collections::HashMap::<String, String>::new())),
        ..Default::default()
    }
}

#[tokio::test]
async fn expands_into_its_plugins() {
    let config = load(
        r#"
app: t
plugins:
  - use: saas-auth@^1
    config:
      social: [google, apple]
      roles: [admin, member, editor]
      admins: [boss@example.com]
      app_url: https://api.shop.test
      with: { auth-core: { password_min_length: 12 } }
"#,
    );
    let e = keel_core::compose::expand(&config, &registry()).unwrap();
    let names: Vec<&str> = e.config.plugins.iter().map(|p| p.name()).collect();
    assert_eq!(names, ["auth-core", "auth-oauth", "auth-mfa", "rbac"]);
    assert!(e.origins.iter().all(|o| o.as_deref() == Some("saas-auth")));
    let cfg = |i: usize| e.config.plugins[i].config.clone();
    assert_eq!(
        cfg(0),
        json!({ "app_url": "https://api.shop.test", "signup": true, "require_verified_email": true, "password_min_length": 12 })
    );
    assert_eq!(
        cfg(1),
        json!({ "redirect_base_url": "https://api.shop.test", "providers": { "google": {}, "apple": {} } })
    );
    assert_eq!(cfg(2), json!({ "issuer": "Keel" }));
    assert_eq!(
        cfg(3),
        json!({
            "roles": { "admin": { "permissions": ["*"] }, "member": { "permissions": [] }, "editor": { "permissions": [] } },
            "default_role": "member", "admin_role": "admin", "admins": ["boss@example.com"]
        })
    );
    keel_core::plugins::check(&e.config, &registry(), &providers().await).unwrap();
}

#[tokio::test]
async fn small_config_and_mistakes() {
    let e = keel_core::compose::expand(
        &load("app: t\nplugins:\n  - use: saas-auth\n    config: { mfa: false }\n"),
        &registry(),
    )
    .unwrap();
    let names: Vec<&str> = e.config.plugins.iter().map(|p| p.name()).collect();
    assert_eq!(names, ["auth-core", "rbac"], "no social, no mfa");

    let err = keel_core::compose::expand(&load("app: t\nplugins:\n  - use: saas-auth@^2\n"), &registry())
        .unwrap_err()
        .to_string();
    assert!(err.contains("this build has version 1.0.0"), "{err}");

    let err = keel_core::compose::expand(
        &load("app: t\nplugins:\n  - use: saas-auth\n    config: { social: [myspace], with: { billing: {} } }\n"),
        &registry(),
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("plugins[0].config.social"), "{err}");
    assert!(err.contains("does not include `billing`"), "{err}");

    let err = keel_core::compose::expand(
        &load("app: t\nplugins:\n  - use: saas-auth\n  - use: rbac\n"),
        &registry(),
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("`rbac` comes from `saas-auth`"), "{err}");
}
