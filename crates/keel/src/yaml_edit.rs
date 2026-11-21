//! Small, comment-preserving edits to `app.yaml`.

use anyhow::{bail, Result};
use serde_json::Value;

/// Adds `- use: spec` (and `config:` when not empty) to the top-level
/// `plugins:` list, keeping the rest of the file as written.
pub fn add_plugin(text: &str, spec: &str, config: &Value) -> Result<String> {
    let mut entry = vec![format!("- use: {spec}")];
    if config.as_object().is_some_and(|m| !m.is_empty()) {
        entry.push(format!("  config: {}", serde_json::to_string(config)?));
    }

    let lines: Vec<&str> = text.lines().collect();
    let Some(start) = lines.iter().position(|l| l.starts_with("plugins:")) else {
        let mut out = text.trim_end().to_string();
        out.push_str("\n\nplugins:\n");
        for e in &entry {
            out.push_str(&format!("  {e}\n"));
        }
        return Ok(out);
    };

    let rest = lines[start]["plugins:".len()..].trim();
    let rest = rest.split('#').next().unwrap_or("").trim();
    let empty_flow = rest == "[]";
    if !rest.is_empty() && !empty_flow {
        bail!(
            "app.yaml lists plugins inline (`plugins: {rest}`); write them as a block list to use `keel add`"
        );
    }

    // The block ends at the next top-level key.
    let mut end = start + 1;
    while end < lines.len()
        && (lines[end].is_empty() || lines[end].starts_with(' ') || lines[end].starts_with('#'))
    {
        end += 1;
    }
    // Insert after the last non-blank line of the block.
    let mut insert_at = end;
    while insert_at > start + 1 && lines[insert_at - 1].trim().is_empty() {
        insert_at -= 1;
    }
    let indent = lines[start + 1..end]
        .iter()
        .find(|l| l.trim_start().starts_with("- "))
        .map(|l| &l[..l.len() - l.trim_start().len()])
        .unwrap_or("  ")
        .to_string();

    let mut out: Vec<String> = Vec::with_capacity(lines.len() + 2);
    for (i, l) in lines.iter().enumerate() {
        if i == insert_at {
            out.extend(entry.iter().map(|e| format!("{indent}{e}")));
        }
        if i == start && empty_flow {
            out.push("plugins:".to_string());
        } else {
            out.push(l.to_string());
        }
    }
    if insert_at == lines.len() {
        out.extend(entry.iter().map(|e| format!("{indent}{e}")));
    }
    Ok(out.join("\n") + "\n")
}

#[cfg(test)]
mod tests {
    use super::add_plugin;
    use serde_json::json;

    #[test]
    fn appends_to_an_existing_block_keeping_comments() {
        let src = "app: x\n# plugins we use\nplugins:\n    - use: ping   # example\n\nroutes: {}\n";
        let out = add_plugin(src, "hello@^0.1", &json!({ "greeting": "Hi" })).unwrap();
        assert_eq!(
            out,
            "app: x\n# plugins we use\nplugins:\n    - use: ping   # example\n    - use: hello@^0.1\n      config: {\"greeting\":\"Hi\"}\n\nroutes: {}\n"
        );
    }

    #[test]
    fn creates_the_block_when_missing_or_empty() {
        assert_eq!(
            add_plugin("app: x\n", "ping", &json!({})).unwrap(),
            "app: x\n\nplugins:\n  - use: ping\n"
        );
        assert_eq!(
            add_plugin("app: x\nplugins: []\n", "ping", &json!({})).unwrap(),
            "app: x\nplugins:\n  - use: ping\n"
        );
        assert_eq!(
            add_plugin("plugins:\n  - use: a\n", "b", &json!({})).unwrap(),
            "plugins:\n  - use: a\n  - use: b\n"
        );
    }

    #[test]
    fn refuses_inline_lists() {
        assert!(add_plugin("plugins: [{ use: a }]\n", "b", &json!({})).is_err());
    }
}
