//! Setup tracking: which human actions each plugin still needs.
//!
//! Each step from a plugin's `setup:` is `done`, `missing`, `failed` (with the
//! error) or `skipped` (its `when` is false). Manual steps are marked done
//! with `keel setup done <plugin>/<id>` and stored in `.keel/setup.json`
//! beside `app.yaml`, so they survive restarts. Routes a step blocks answer
//! `503 setup_incomplete` until it is done.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use keel_sdk::setup::{SetupKind, VerifyKind};
use keel_sdk::{ApiError, Plugin};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::AppConfig;
use crate::secrets::SecretSource;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StepState {
    Done,
    Missing,
    Failed,
    Skipped,
}

/// One step as reported by `keel setup status` and `/health`.
#[derive(Debug, Clone, Serialize)]
pub struct StepStatus {
    /// `<plugin>/<step>`, e.g. `auth-oauth/google-client`.
    pub id: String,
    pub plugin: String,
    pub title: String,
    pub kind: SetupKind,
    pub status: StepState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub instructions: Vec<String>,
    /// Secrets the step produces, set or not.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub secrets: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub missing_secrets: Vec<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub requires_paid_account: bool,
    /// How to finish it: set secrets, or `keel setup done <id>`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
}

impl StepStatus {
    pub fn incomplete(&self) -> bool {
        matches!(self.status, StepState::Missing | StepState::Failed)
    }
}

/// Every step of every configured plugin.
#[derive(Debug, Clone, Default, Serialize)]
pub struct SetupReport {
    pub complete: bool,
    pub steps: Vec<StepStatus>,
    /// For blocking: which routes each incomplete step blocks.
    #[serde(skip)]
    blocking: Vec<Blocker>,
}

#[derive(Clone)]
struct Blocker {
    step: String,
    title: String,
    plugin: Arc<dyn Plugin>,
    index: usize,
}

impl std::fmt::Debug for Blocker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Blocker").field("step", &self.step).finish()
    }
}

impl SetupReport {
    /// The incomplete step that blocks `method path`, as a 503 error.
    pub fn blocked(&self, method: &str, path: &str) -> Option<ApiError> {
        self.blocking.iter().find_map(|b| {
            let m = b.plugin.manifest();
            m.setup[b.index].blocks_request(&m.routes, method, path).then(|| {
                ApiError::new(
                    axum::http::StatusCode::SERVICE_UNAVAILABLE,
                    "setup_incomplete",
                    format!("this route needs a setup step first: {}", b.title),
                )
                .with("step", b.step.clone())
                .with("hint", "run `keel setup status` to see what is missing")
            })
        })
    }

    pub fn incomplete_ids(&self) -> Vec<String> {
        self.steps
            .iter()
            .filter(|s| s.incomplete())
            .map(|s| s.id.clone())
            .collect()
    }

    pub fn find(&self, id: &str) -> Option<&StepStatus> {
        self.steps.iter().find(|s| s.id == id)
    }
}

/// Manual steps the user marked done. Stored as JSON.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SetupState {
    /// Step id -> unix seconds when it was marked done.
    #[serde(default)]
    pub done: BTreeMap<String, u64>,
}

impl SetupState {
    /// `.keel/setup.json` in the directory of `app.yaml`.
    pub fn path_for(config_file: &Path) -> PathBuf {
        let dir = config_file
            .parent()
            .filter(|d| !d.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        dir.join(".keel").join("setup.json")
    }

    pub fn load(path: &Path) -> std::io::Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).map_err(|e| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("{}: {e}", path.display()),
                )
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e),
        }
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string_pretty(self).expect("state serializes");
        std::fs::write(path, text + "\n")
    }

    pub fn mark_done(&mut self, id: &str) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default();
        self.done.insert(id.to_string(), now);
    }

    pub fn unmark(&mut self, id: &str) -> bool {
        self.done.remove(id).is_some()
    }
}

/// Evaluates every step. `plugins` are the configured plugins; `run_checks`
/// runs `plugin-check` steps, which only makes sense on started plugins.
pub async fn evaluate(
    config: &AppConfig,
    plugins: &[Arc<dyn Plugin>],
    secrets: &dyn SecretSource,
    state: &SetupState,
    run_checks: bool,
) -> SetupReport {
    let mut report = SetupReport::default();
    for plugin in plugins {
        let m = plugin.manifest();
        let empty = Value::Object(Default::default());
        let plugin_config = config
            .plugins
            .iter()
            .find(|e| e.name() == m.id)
            .map(|e| &e.config)
            .unwrap_or(&empty);
        for (index, step) in m.setup.iter().enumerate() {
            let id = format!("{}/{}", m.id, step.id);
            let verify = step.verify();
            let produced = step.secrets();
            let mut status = StepStatus {
                id: id.clone(),
                plugin: m.id.clone(),
                title: step.title.clone(),
                kind: step.kind,
                status: StepState::Done,
                reason: None,
                instructions: Vec::new(),
                secrets: produced,
                missing_secrets: Vec::new(),
                requires_paid_account: step.requires_paid_account,
                next: None,
            };
            if !step.applies(plugin_config) {
                status.status = StepState::Skipped;
                status.reason = step.when.as_ref().map(|w| format!("`{w}` is not set"));
                status.secrets.clear();
                report.steps.push(status);
                continue;
            }
            match verify.kind {
                VerifyKind::SecretPresent => {
                    let missing: Vec<String> = verify
                        .keys
                        .iter()
                        .filter(|k| secrets.get(k).is_none_or(|v| v.trim().is_empty()))
                        .cloned()
                        .collect();
                    if !missing.is_empty() {
                        status.status = StepState::Missing;
                        status.next = Some(format!(
                            "set {} (env, .env or the secret store)",
                            missing.join(", ")
                        ));
                        status.missing_secrets = missing;
                    }
                }
                VerifyKind::Manual => {
                    if !state.done.contains_key(&id) {
                        status.status = StepState::Missing;
                        status.next = Some(format!("when finished, run `keel setup done {id}`"));
                    }
                }
                VerifyKind::PluginCheck => {
                    let missing: Vec<String> = status
                        .secrets
                        .iter()
                        .filter(|k| secrets.get(k).is_none_or(|v| v.trim().is_empty()))
                        .cloned()
                        .collect();
                    if !missing.is_empty() {
                        status.status = StepState::Missing;
                        status.next = Some(format!("set {}", missing.join(", ")));
                        status.missing_secrets = missing;
                    } else if run_checks {
                        if let Err(e) = plugin.check_setup(&step.id).await {
                            status.status = StepState::Failed;
                            status.reason = Some(e);
                        }
                    }
                }
            }
            if status.incomplete() {
                status.instructions = step.instructions(plugin_config);
                report.blocking.push(Blocker {
                    step: id,
                    title: step.title.clone(),
                    plugin: plugin.clone(),
                    index,
                });
            }
            report.steps.push(status);
        }
    }
    report.complete = report.steps.iter().all(|s| !s.incomplete());
    report
}
