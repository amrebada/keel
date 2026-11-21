//! Loads `.env` next to app.yaml so `${secret:NAME}` works in local dev.

use std::path::{Path, PathBuf};

/// Parses `KEY=value` lines. Skips blanks and `#` comments, accepts a leading
/// `export `, and strips one pair of matching quotes around the value.
pub fn parse(text: &str) -> Vec<(String, String)> {
    let mut vars = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            continue;
        }
        let value = value.trim();
        let value = ['"', '\'']
            .iter()
            .find_map(|q| value.strip_prefix(*q).and_then(|v| v.strip_suffix(*q)))
            .unwrap_or(value);
        vars.push((key.to_string(), value.to_string()));
    }
    vars
}

/// Sets variables from `.env` beside `config`. Variables already set in the
/// environment win. Returns the file read, if any.
pub fn load_beside(config: &Path) -> Option<PathBuf> {
    let dir = config
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let path = dir.join(".env");
    let text = std::fs::read_to_string(&path).ok()?;
    for (key, value) in parse(&text) {
        if std::env::var_os(&key).is_none() {
            std::env::set_var(key, value);
        }
    }
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_forms() {
        let vars = parse("# c\n\nA=1\nexport B = \"two words\"\nC='x=y'\nbad line\n=nokey\n");
        assert_eq!(
            vars,
            vec![
                ("A".into(), "1".into()),
                ("B".into(), "two words".into()),
                ("C".into(), "x=y".into())
            ]
        );
    }
}
