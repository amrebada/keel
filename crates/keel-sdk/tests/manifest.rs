use keel_sdk::{Manifest, ManifestError};
use serde_json::json;

fn err(yaml: &str) -> String {
    match Manifest::from_sources(yaml, None) {
        Err(ManifestError::Invalid(issues)) => issues
            .iter()
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join("\n"),
        Err(e) => e.to_string(),
        Ok(_) => panic!("expected an error for:\n{yaml}"),
    }
}

#[test]
fn parses_a_full_manifest() {
    let m = Manifest::from_sources(
        r#"
id: auth-oauth
version: 1.2.0
summary: Social login
requires:
  host: ">=0.1, <2"
  capabilities: [sql, cache, secrets]
  plugins: { auth-core: "^1" }
provides:
  interfaces: [identity-provider]
  routes:
    - GET /auth/{provider}/start
    - GET /auth/{provider}/callback
  events: [user.linked]
consumes: { events: [user.created] }
config: config.schema.json
runtime_config: [providers.google.enabled]
"#,
        Some(r#"{"type":"object","properties":{"providers":{"type":"object"}}}"#),
    )
    .unwrap();
    assert_eq!(m.id, "auth-oauth");
    assert_eq!(m.version.to_string(), "1.2.0");
    assert_eq!(m.capabilities.len(), 3);
    assert!(m.plugins.contains_key("auth-core"));
    assert!(m.declares("GET", "/auth/{provider}/start"));
    assert!(m.validate_config(&json!({ "providers": {} })).is_empty());
    assert!(!m.validate_config(&json!({ "providers": 1 })).is_empty());
}

#[test]
fn reports_schema_problems_with_paths() {
    let e = err("id: Bad_Id\nversion: one\nrequires: { capabilities: [sql, teleport] }\nprovides: { routes: [FETCH /x] }\ncolour: blue\n");
    assert!(e.contains("(root): \"summary\" is a required property"), "{e}");
    assert!(
        e.contains("(root): Additional properties are not allowed ('colour' was unexpected)"),
        "{e}"
    );
    assert!(e.contains("id: \"Bad_Id\" does not match"), "{e}");
    assert!(e.contains("version: \"one\" does not match"), "first pass reports a bad version: {e}");
    assert!(e.contains("requires.capabilities[1]"), "{e}");
    assert!(e.contains("provides.routes[0]"), "{e}");
}

#[test]
fn reports_semantic_problems() {
    let e = err("id: x\nversion: 1.0.0\nsummary: s\nrequires: { host: \"!!\", plugins: { x: \"^1\", y: \"nope\" } }\nprovides: { routes: [GET /a, GET /a, GET /health] }\n");
    assert!(e.contains("requires.host: not a version range"), "{e}");
    assert!(
        e.contains("requires.plugins.x: a plugin cannot require itself"),
        "{e}"
    );
    assert!(e.contains("requires.plugins.y: not a version range"), "{e}");
    assert!(
        e.contains("provides.routes[1]: `GET /a` is declared twice"),
        "{e}"
    );
    assert!(
        e.contains("provides.routes[2]: `/health` is reserved by the host"),
        "{e}"
    );
}

#[test]
fn rejects_a_broken_config_schema() {
    let e = Manifest::from_sources("id: x\nversion: 1.0.0\nsummary: s\n", Some("{ not json")).unwrap_err();
    assert!(
        e.to_string().starts_with("config.schema.json: not valid JSON"),
        "{e}"
    );
    let e =
        Manifest::from_sources("id: x\nversion: 1.0.0\nsummary: s\n", Some(r#"{"type": 5}"#)).unwrap_err();
    assert!(e.to_string().contains("not a valid JSON Schema"), "{e}");
}

#[test]
fn plugin_without_schema_takes_no_config() {
    let m = Manifest::from_sources("id: x\nversion: 1.0.0\nsummary: s\n", None).unwrap();
    assert!(m.validate_config(&json!({})).is_empty());
    assert!(!m.validate_config(&json!({ "a": 1 })).is_empty());
}
