//! `keel mcp` over stdio: the handshake, tool listing and tool calls an AI
//! client makes, against a fresh `keel init` app.

use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::{json, Value};

const KEEL: &str = env!("CARGO_BIN_EXE_keel");

fn session(dir: &std::path::Path, calls: &[Value]) -> Vec<Value> {
    let mut child = Command::new(KEEL)
        .args(["mcp", "-c", "app.yaml"])
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    {
        let stdin = child.stdin.as_mut().unwrap();
        for c in calls {
            writeln!(stdin, "{c}").unwrap();
        }
    }
    let out = child.wait_with_output().unwrap();
    String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

fn call(id: u64, name: &str, args: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": "tools/call", "params": { "name": name, "arguments": args } })
}

#[test]
fn ai_client_flow() {
    let tmp = tempfile::tempdir().unwrap();
    let ok = Command::new(KEEL)
        .args(["init", "shop"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(ok.status.success());
    let dir = tmp.path().join("shop");
    let new_yaml = "app: shop\nproviders:\n  db: { use: sqlite, url: \"sqlite://shop.db\" }\n  cache: { use: memory }\n  email: { use: console }\nplugins:\n  - use: auth-core\n  - use: rbac\nroutes:\n  /rbac/users/**: { guard: [\"role:admin\"] }\n";
    let replies = session(
        &dir,
        &[
            json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": { "name": "test", "version": "1" } } }),
            json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
            json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
            call(3, "search_plugins", json!({ "query": "roles" })),
            call(4, "describe_plugin", json!({ "id": "rbac" })),
            call(11, "search_plugins", json!({ "query": "payments" })),
            call(
                5,
                "validate_config",
                json!({ "yaml": "app: shop\nplugins:\n  - use: rbac\n" }),
            ),
            call(6, "apply_config", json!({ "yaml": new_yaml })),
            call(7, "setup_status", json!({})),
            call(8, "read_config", json!({})),
            call(9, "nope", json!({})),
            json!({ "jsonrpc": "2.0", "id": 10, "method": "unknown/method" }),
        ],
    );
    assert_eq!(replies.len(), 11, "the notification gets no reply: {replies:#?}");
    let by_id = |id: u64| replies.iter().find(|r| r["id"] == id).unwrap().clone();
    let r = by_id(1);
    assert_eq!(r["result"]["protocolVersion"], "2025-03-26");
    assert_eq!(r["result"]["serverInfo"]["name"], "keel");
    let names: Vec<String> = by_id(2)["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    for n in [
        "search_plugins",
        "describe_plugin",
        "validate_config",
        "apply_config",
        "setup_status",
        "logs",
    ] {
        assert!(names.contains(&n.to_string()), "{names:?}");
    }
    let found = &by_id(3)["result"]["structuredContent"]["plugins"];
    assert_eq!(found.as_array().unwrap().len(), 1, "{found}");
    assert_eq!(found[0]["id"], "rbac");
    assert_eq!(
        by_id(11)["result"]["structuredContent"]["plugins"][0]["id"],
        "billing-stripe",
        "tags are searched"
    );
    let summary = by_id(4)["result"]["structuredContent"]["summary"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(summary.contains("default_role"), "{summary}");

    // rbac without auth-core is refused, with the reason.
    let r = by_id(5)["result"].clone();
    assert_eq!(r["isError"], true);
    assert!(
        r["content"][0]["text"].as_str().unwrap().contains("auth-core"),
        "{r}"
    );

    let r = by_id(6)["result"].clone();
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(std::fs::read_to_string(dir.join("app.yaml")).unwrap(), new_yaml);
    assert!(dir.join(".keel/app.yaml.bak").exists());

    let steps = &by_id(7)["result"]["structuredContent"]["steps"];
    assert_eq!(steps[0]["id"], "auth-core/jwt-secret", "{steps}");
    assert_eq!(steps[0]["status"], "missing");

    assert_eq!(by_id(8)["result"]["structuredContent"]["yaml"], new_yaml);
    assert_eq!(by_id(9)["error"]["code"], -32602);
    assert_eq!(by_id(10)["error"]["code"], -32601);
}
