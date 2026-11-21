//! `keel mcp`: a Model Context Protocol server over stdio, so an AI client
//! (Claude Code, Claude Desktop, ...) can search plugins, read and change
//! `app.yaml`, and follow setup without a shell.
//!
//! Each tool runs this same `keel` binary with `--json` and returns that
//! output, so MCP and the CLI always agree.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use serde_json::{json, Value};

const PROTOCOL: &str = "2025-06-18";
const SUPPORTED: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

struct Server {
    exe: PathBuf,
    /// Absolute path to app.yaml.
    config: PathBuf,
}

fn tool(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": { "type": "object", "properties": properties, "required": required, "additionalProperties": false }
    })
}

fn tools() -> Vec<Value> {
    vec![
        tool(
            "search_plugins",
            "Search the plugins compiled into this Keel build by id or summary. Empty query lists all. Start here to find what to add instead of writing code.",
            json!({ "query": { "type": "string", "description": "Words to match, e.g. 'stripe' or 'roles'" } }),
            &[],
        ),
        tool(
            "describe_plugin",
            "A plugin's config keys with defaults, routes, events, interfaces and setup steps, in about 300 tokens. Read it before adding the plugin to app.yaml.",
            json!({ "id": { "type": "string" } }),
            &["id"],
        ),
        tool(
            "read_config",
            "The current app.yaml text, plus the expanded config with defaults filled in.",
            json!({}),
            &[],
        ),
        tool(
            "validate_config",
            "Check app.yaml (or a proposed new version passed as yaml) against every schema, secret and plugin rule. Returns each problem with its path. Writes nothing.",
            json!({ "yaml": { "type": "string", "description": "A full proposed app.yaml; omit to check the current file" } }),
            &[],
        ),
        tool(
            "apply_config",
            "Replace app.yaml with yaml after it passes validation (the old file is kept as .keel/app.yaml.bak). A running `keel dev` reloads it within a second.",
            json!({ "yaml": { "type": "string", "description": "The full new app.yaml" } }),
            &["yaml"],
        ),
        tool(
            "add_plugin",
            "Add a compiled-in plugin to app.yaml with a minimal valid config, keeping comments.",
            json!({ "plugin": { "type": "string", "description": "Plugin id, optionally with a version range, e.g. 'rbac' or 'rbac@^0.1'" } }),
            &["plugin"],
        ),
        tool(
            "setup_status",
            "Every setup step (accounts, API keys, DNS, webhooks) of the configured plugins and providers: done, missing, failed or skipped, with instructions and the secrets each needs. Ask the user only about missing or failed steps.",
            json!({}),
            &[],
        ),
        tool(
            "setup_done",
            "Mark a manual setup step done (or not done) after the user confirms they did it.",
            json!({ "id": { "type": "string", "description": "<plugin>/<step> as setup_status prints it" }, "done": { "type": "boolean", "default": true } }),
            &["id"],
        ),
        tool(
            "migrations",
            "Plan plugin database migrations (dry run with row counts), or run the additive ones with apply. Destructive migrations are never run from here; the user runs `keel migrate --allow-destructive` themselves.",
            json!({ "apply": { "type": "boolean", "default": false } }),
            &[],
        ),
        tool(
            "health",
            "GET /health of the running server (from server.host and server.port in app.yaml): status of every plugin and provider and incomplete setup steps.",
            json!({}),
            &[],
        ),
        tool(
            "logs",
            "The last lines of the server log (.keel/keel.log, written by `keel dev` and `keel serve`).",
            json!({ "lines": { "type": "integer", "minimum": 1, "maximum": 1000, "default": 100 }, "contains": { "type": "string", "description": "Only lines containing this text" } }),
            &[],
        ),
    ]
}

impl Server {
    fn dir(&self) -> PathBuf {
        self.config
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    }

    /// Runs `keel --json <args>` and returns (ok, parsed JSON).
    fn keel(&self, args: &[&str]) -> Result<(bool, Value)> {
        let out = Command::new(&self.exe)
            .arg("--json")
            .args(args)
            .current_dir(self.dir())
            .env_remove("RUST_LOG")
            .output()
            .with_context(|| format!("running {}", self.exe.display()))?;
        let text = String::from_utf8_lossy(&out.stdout);
        let v: Value = text
            .lines()
            .rev()
            .find_map(|l| serde_json::from_str(l).ok())
            .unwrap_or_else(|| {
                json!({ "ok": false, "error": { "message": String::from_utf8_lossy(&out.stderr).trim().to_string() } })
            });
        let ok = out.status.success() && v.get("ok").and_then(Value::as_bool).unwrap_or(true);
        Ok((ok, v))
    }

    fn config_arg(&self) -> String {
        self.config.display().to_string()
    }

    /// Validates proposed YAML from a temporary file beside app.yaml, so
    /// relative paths (SQLite files, .env) mean the same thing.
    fn validate_text(&self, yaml: &str) -> Result<(bool, Value)> {
        let candidate = self.dir().join(".keel-candidate.yaml");
        std::fs::write(&candidate, yaml)?;
        let r = self.keel(&["config", "validate", "-c", &candidate.display().to_string()]);
        let _ = std::fs::remove_file(&candidate);
        r
    }

    fn call(&self, name: &str, args: &Value) -> Result<(bool, Value)> {
        let s = |k: &str| args.get(k).and_then(Value::as_str);
        let c = self.config_arg();
        match name {
            "search_plugins" => {
                let (ok, v) = self.keel(&["plugins", "list"])?;
                let q = s("query").unwrap_or("").to_lowercase();
                let words: Vec<&str> = q.split_whitespace().collect();
                let list: Vec<Value> = v["plugins"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|p| {
                        let hay = format!(
                            "{} {}",
                            p["id"].as_str().unwrap_or(""),
                            p["summary"].as_str().unwrap_or("")
                        )
                        .to_lowercase();
                        words.iter().all(|w| hay.contains(w))
                    })
                    .collect();
                Ok((ok, json!({ "plugins": list })))
            }
            "describe_plugin" => self.keel(&["plugins", "describe", s("id").unwrap_or(""), "--ai"]),
            "read_config" => {
                let text = std::fs::read_to_string(&self.config)
                    .with_context(|| format!("reading {}", self.config.display()))?;
                let (_, expanded) = self.keel(&["config", "expand", "-c", &c])?;
                Ok((true, json!({ "file": c, "yaml": text, "expanded": expanded })))
            }
            "validate_config" => match s("yaml") {
                Some(y) => self.validate_text(y),
                None => self.keel(&["config", "validate", "-c", &c]),
            },
            "apply_config" => {
                let yaml = s("yaml").unwrap_or("");
                let (ok, v) = self.validate_text(yaml)?;
                if !ok {
                    return Ok((false, json!({ "applied": false, "validation": v })));
                }
                let backup = self.dir().join(".keel").join("app.yaml.bak");
                std::fs::create_dir_all(self.dir().join(".keel"))?;
                if self.config.exists() {
                    std::fs::copy(&self.config, &backup)?;
                }
                std::fs::write(&self.config, yaml)?;
                Ok((
                    true,
                    json!({ "applied": true, "file": c, "backup": backup.display().to_string(), "validation": v }),
                ))
            }
            "add_plugin" => self.keel(&["add", s("plugin").unwrap_or(""), "-c", &c]),
            "setup_status" => self.keel(&["setup", "status", "-c", &c]),
            "setup_done" => {
                let done = args.get("done").and_then(Value::as_bool).unwrap_or(true);
                self.keel(&[
                    "setup",
                    if done { "done" } else { "undo" },
                    s("id").unwrap_or(""),
                    "-c",
                    &c,
                ])
            }
            "migrations" => {
                if args.get("apply").and_then(Value::as_bool).unwrap_or(false) {
                    self.keel(&["migrate", "-c", &c, "--apply"])
                } else {
                    self.keel(&["migrate", "-c", &c])
                }
            }
            "health" => self.health(),
            "logs" => {
                let n = args
                    .get("lines")
                    .and_then(Value::as_u64)
                    .unwrap_or(100)
                    .clamp(1, 1000) as usize;
                let path = self.dir().join(".keel").join("keel.log");
                let text = match std::fs::read_to_string(&path) {
                    Ok(t) => t,
                    Err(_) => {
                        return Ok((
                            false,
                            json!({ "error": { "message": format!("no log at {}; start the server with `keel dev`", path.display()) } }),
                        ))
                    }
                };
                let filter = s("contains");
                let lines: Vec<&str> = text
                    .lines()
                    .filter(|l| filter.is_none_or(|f| l.contains(f)))
                    .collect();
                let tail = lines[lines.len().saturating_sub(n)..].join("\n");
                Ok((true, json!({ "file": path.display().to_string(), "lines": tail })))
            }
            other => anyhow::bail!("unknown tool `{other}`"),
        }
    }

    fn health(&self) -> Result<(bool, Value)> {
        let (_, v) = self.keel(&["config", "expand", "-c", &self.config_arg()])?;
        let cfg = v.get("config").unwrap_or(&v);
        let host = cfg["server"]["host"].as_str().unwrap_or("127.0.0.1");
        let host = if host == "0.0.0.0" { "127.0.0.1" } else { host };
        let port = cfg["server"]["port"].as_u64().unwrap_or(8080);
        let url = format!("http://{host}:{port}/health");
        let rt = tokio::runtime::Handle::current();
        let res = tokio::task::block_in_place(|| {
            rt.block_on(async {
                let client = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(5))
                    .build()?;
                let r = client.get(&url).send().await?;
                let status = r.status().as_u16();
                let body: Value = r.json().await.unwrap_or(Value::Null);
                Ok::<_, reqwest::Error>((status, body))
            })
        });
        Ok(match res {
            Ok((status, body)) => (
                status < 500,
                json!({ "url": url, "http_status": status, "health": body }),
            ),
            Err(e) => (
                false,
                json!({ "url": url, "error": { "message": format!("server not reachable: {e}; start it with `keel dev`") } }),
            ),
        })
    }

    fn handle(&self, msg: &Value) -> Option<Value> {
        let id = msg.get("id").cloned();
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        // Notifications get no answer.
        let id = id?;
        let result: Result<Value, (i64, String)> = match method {
            "initialize" => {
                let asked = params["protocolVersion"].as_str().unwrap_or(PROTOCOL);
                let version = if SUPPORTED.contains(&asked) {
                    asked
                } else {
                    PROTOCOL
                };
                Ok(json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": { "listChanged": false } },
                    "serverInfo": { "name": "keel", "title": "Keel backend", "version": env!("CARGO_PKG_VERSION") },
                    "instructions": format!(
                        "Keel builds a backend from app.yaml ({}) and compiled-in plugins. Prefer configuring plugins over writing code: search_plugins, describe_plugin, then add_plugin or apply_config. After any change run setup_status and ask the user only about missing or failed steps.",
                        self.config.display()
                    ),
                }))
            }
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tools() })),
            "tools/call" => {
                let name = params["name"].as_str().unwrap_or("");
                let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
                if !tools().iter().any(|t| t["name"] == name) {
                    Err((-32602, format!("unknown tool `{name}`")))
                } else {
                    let (ok, v) = match self.call(name, &args) {
                        Ok(r) => r,
                        Err(e) => (false, json!({ "error": { "message": format!("{e:#}") } })),
                    };
                    Ok(json!({
                        "content": [{ "type": "text", "text": serde_json::to_string_pretty(&v).unwrap_or_default() }],
                        "structuredContent": v,
                        "isError": !ok,
                    }))
                }
            }
            "resources/list" => Ok(json!({ "resources": [] })),
            "prompts/list" => Ok(json!({ "prompts": [] })),
            _ => Err((-32601, format!("method `{method}` is not supported"))),
        };
        Some(match result {
            Ok(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
            Err((code, message)) => {
                json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
            }
        })
    }
}

/// Serves MCP on stdin/stdout until stdin closes.
pub async fn run(config: &Path) -> Result<()> {
    let config = if config.is_absolute() {
        config.to_path_buf()
    } else {
        std::env::current_dir()?.join(config)
    };
    let server = Server {
        exe: std::env::current_exe().context("finding the keel binary")?,
        config,
    };
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(Value::Array(batch)) => {
                let out: Vec<Value> = batch.iter().filter_map(|m| server.handle(m)).collect();
                (!out.is_empty()).then_some(Value::Array(out))
            }
            Ok(msg) => server.handle(&msg),
            Err(e) => Some(
                json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": format!("parse error: {e}") } }),
            ),
        };
        if let Some(r) = reply {
            writeln!(stdout, "{r}")?;
            stdout.flush()?;
        }
    }
    Ok(())
}
