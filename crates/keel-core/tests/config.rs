use keel_core::{AppConfig, ConfigError, MapSecrets};

const GOOD: &str = r#"
app: team-tasks
server: { port: 9090 }
providers:
  db: { use: postgres, url: "postgres://u:${secret:DB_PASS}@localhost/app" }
plugins:
  - use: auth-core@^1
    config: { session: { type: jwt } }
  - use: rbac
routes:
  /admin/**: { guard: [authenticated, "role:admin"] }
"#;

fn secrets() -> MapSecrets {
    MapSecrets::new([("DB_PASS", "s3cret")])
}

#[test]
fn loads_a_valid_config() {
    let c = AppConfig::load_str(GOOD, "app.yaml", &secrets()).unwrap();
    assert_eq!(c.app, "team-tasks");
    assert_eq!(c.server.port, 9090);
    assert_eq!(c.server.host, "0.0.0.0");
    assert_eq!(c.plugins[0].name(), "auth-core");
    assert_eq!(c.plugins[0].version_req(), Some("^1"));
    assert_eq!(c.plugins[1].name(), "rbac");
    assert_eq!(c.plugins[1].version_req(), None);
    assert_eq!(c.routes["/admin/**"].guard, vec!["authenticated", "role:admin"]);
}

#[test]
fn resolves_secrets_inside_strings() {
    let c = AppConfig::load_str(GOOD, "app.yaml", &secrets()).unwrap();
    assert_eq!(c.providers["db"].kind, "postgres");
    assert_eq!(
        c.providers["db"].settings["url"],
        "postgres://u:s3cret@localhost/app"
    );
}

#[test]
fn reports_missing_secrets_with_their_path() {
    let err = AppConfig::load_str(GOOD, "app.yaml", &MapSecrets::default()).unwrap_err();
    match &err {
        ConfigError::MissingSecrets { missing, .. } => {
            assert_eq!(missing.len(), 1);
            assert_eq!(missing[0].name, "DB_PASS");
            assert_eq!(missing[0].path, "providers.db.url");
        }
        other => panic!("unexpected error: {other}"),
    }
    assert!(err.to_string().contains("DB_PASS (used at providers.db.url)"));
}

#[test]
fn reports_every_bad_key_with_its_path() {
    let bad = r#"
app: team-tasks
server: { prot: 80, port: 70000 }
plugins:
  - config: {}
  - use: "Bad Name"
colour: blue
"#;
    let err = AppConfig::load_str(bad, "app.yaml", &secrets()).unwrap_err();
    let ConfigError::Invalid { issues, .. } = &err else {
        panic!("unexpected error: {err}")
    };
    let paths: Vec<&str> = issues.iter().map(|i| i.path.as_str()).collect();
    assert!(paths.contains(&"(root)"), "{paths:?}"); // unknown `colour`
    assert!(paths.contains(&"server"), "{paths:?}"); // unknown `prot`
    assert!(paths.contains(&"server.port"), "{paths:?}"); // out of range
    assert!(paths.contains(&"plugins[0]"), "{paths:?}"); // missing `use`
    assert!(paths.contains(&"plugins[1].use"), "{paths:?}"); // bad pattern
    let text = err.to_string();
    assert!(text.contains("colour"), "{text}");
    assert!(text.contains("prot"), "{text}");
}

#[test]
fn errors_do_not_leak_secret_values() {
    let bad = "app: x\nserver: { port: \"${secret:PORT}\" }\n";
    let err = AppConfig::load_str(bad, "app.yaml", &MapSecrets::new([("PORT", "hunter2")])).unwrap_err();
    assert!(!err.to_string().contains("hunter2"));
}

#[test]
fn reports_yaml_syntax_errors() {
    let err = AppConfig::load_str("app: [unclosed", "app.yaml", &secrets()).unwrap_err();
    assert!(matches!(err, ConfigError::Parse { .. }), "{err}");
}

#[test]
fn requires_app_name() {
    let err = AppConfig::load_str("server: { port: 80 }", "app.yaml", &secrets()).unwrap_err();
    assert!(err.to_string().contains("app"), "{err}");
}

#[test]
fn example_config_is_valid() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/app.yaml");
    let c = AppConfig::load_file(
        path,
        &MapSecrets::new([("DATABASE_URL", "postgres://localhost/x")]),
    )
    .unwrap();
    assert_eq!(c.app, "team-tasks");
}
