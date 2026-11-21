//! Validation problems with the path they apply to, shared by app and plugin checks.

use std::fmt;

use serde_json::Value;

/// One validation problem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    /// Path such as `plugins[0].use`, or `(root)` for the top level.
    pub path: String,
    pub message: String,
}

impl Issue {
    pub fn new(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for Issue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path, self.message)
    }
}

/// Formats issues as an indented list under a heading.
pub fn format_list(heading: &str, issues: &[Issue]) -> String {
    let mut s = format!("{heading}: {} problem(s) found", issues.len());
    for i in issues {
        s.push_str(&format!("\n  - {i}"));
    }
    s
}

/// Validates `value` against `validator`, returning every problem sorted by path.
pub fn schema_issues(validator: &jsonschema::Validator, value: &Value) -> Vec<Issue> {
    let mut issues: Vec<Issue> = validator
        .iter_errors(value)
        .map(|e| Issue::new(pointer_to_path(&e.instance_path().to_string()), e.to_string()))
        .collect();
    issues.sort_by(|a, b| a.path.cmp(&b.path));
    issues
}

/// `/plugins/0/use` -> `plugins[0].use`
pub fn pointer_to_path(pointer: &str) -> String {
    let mut out = String::new();
    for seg in pointer.split('/').skip(1) {
        let seg = seg.replace("~1", "/").replace("~0", "~");
        if !seg.is_empty() && seg.chars().all(|c| c.is_ascii_digit()) {
            out.push_str(&format!("[{seg}]"));
        } else {
            if !out.is_empty() {
                out.push('.');
            }
            out.push_str(&seg);
        }
    }
    if out.is_empty() {
        "(root)".to_string()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::pointer_to_path;

    #[test]
    fn pointer_paths() {
        assert_eq!(pointer_to_path(""), "(root)");
        assert_eq!(pointer_to_path("/plugins/0/use"), "plugins[0].use");
        assert_eq!(
            pointer_to_path("/routes/~1admin~1**/guard"),
            "routes./admin/**.guard"
        );
    }
}
