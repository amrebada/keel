//! `keel setup` and `keel migrate`.

use std::path::Path;
use std::sync::Arc;

use anyhow::{anyhow, bail, Result};
use keel_core::migrate::{self, Kind, State};
use keel_core::setup::{self, SetupReport, SetupState, StepState};
use keel_sdk::setup::VerifyKind;
use keel_sdk::Plugin;
use serde_json::json;

use crate::output::Out;
use crate::{connect, load_checked, plugins};

/// Setup report without starting anything. `plugin-check` steps are only
/// checked for their secrets here; the running server runs the full check.
async fn report(path: &Path) -> Result<SetupReport> {
    let registry = plugins::builtin();
    let (config, secrets) = load_checked(path, &registry).await?;
    let list: Vec<Arc<dyn Plugin>> = config
        .plugins
        .iter()
        .filter_map(|e| registry.get(e.name()).cloned())
        .collect();
    let state = SetupState::load(&SetupState::path_for(path))?;
    Ok(setup::evaluate(&config, &list, secrets.as_ref(), &state, false).await)
}

pub async fn setup_status(out: Out, path: &Path) -> Result<()> {
    let r = report(path).await?;
    let count = |s: StepState| r.steps.iter().filter(|x| x.status == s).count();
    let (done, missing, failed, skipped) = (
        count(StepState::Done),
        count(StepState::Missing),
        count(StepState::Failed),
        count(StepState::Skipped),
    );
    let mut text = if r.steps.is_empty() {
        "setup: no plugin in app.yaml needs setup steps".to_string()
    } else {
        format!(
            "setup: {done} of {} done, {missing} missing, {failed} failed, {skipped} skipped",
            r.steps.len() - skipped
        )
    };
    for s in &r.steps {
        let mark = match s.status {
            StepState::Done => "ok  ",
            StepState::Missing => "TODO",
            StepState::Failed => "FAIL",
            StepState::Skipped => "skip",
        };
        let paid = if s.requires_paid_account {
            " (needs a paid account)"
        } else {
            ""
        };
        text.push_str(&format!("\n  {mark} {}  {}{paid}", s.id, s.title));
        if s.incomplete() {
            if let Some(reason) = &s.reason {
                text.push_str(&format!("\n         why: {reason}"));
            }
            for (i, step) in s.instructions.iter().enumerate() {
                text.push_str(&format!("\n         {}. {step}", i + 1));
            }
            if let Some(next) = &s.next {
                text.push_str(&format!("\n         next: {next}"));
            }
        }
    }
    if !r.complete {
        text.push_str("\nroutes blocked by these steps answer 503 setup_incomplete until they are done");
    }
    out.success(text, serde_json::to_value(&r)?);
    Ok(())
}

pub async fn setup_mark(out: Out, path: &Path, id: &str, done: bool) -> Result<()> {
    let registry = plugins::builtin();
    let (config, _) = load_checked(path, &registry).await?;
    let Some((plugin_id, step_id)) = id.split_once('/') else {
        bail!("`{id}`: use <plugin>/<step>, as `keel setup status` prints it");
    };
    if plugin_id == "providers" {
        let sends = config
            .providers
            .get("email")
            .is_some_and(|e| ["resend", "ses", "smtp"].contains(&e.kind.as_str()));
        if step_id != "email-domain" || !sends {
            bail!("`{id}` is not a setup step; provider steps: providers/email-domain (with a smtp, resend or ses email provider)");
        }
        return save_mark(out, path, id, done);
    }
    let plugin = config
        .plugins
        .iter()
        .find(|e| e.name() == plugin_id)
        .and_then(|e| registry.get(e.name()))
        .ok_or_else(|| anyhow!("`{plugin_id}` is not a plugin in {}", path.display()))?;
    let step = plugin
        .manifest()
        .setup
        .iter()
        .find(|s| s.id == step_id)
        .ok_or_else(|| {
            let ids: Vec<String> = plugin
                .manifest()
                .setup
                .iter()
                .map(|s| format!("{plugin_id}/{}", s.id))
                .collect();
            anyhow!(
                "`{id}` is not a setup step. Steps of {plugin_id}: {}",
                ids.join(", ")
            )
        })?;
    if step.verify().kind != VerifyKind::Manual {
        bail!(
            "`{id}` is checked automatically ({}); set its secrets instead of marking it done",
            match step.verify().kind {
                VerifyKind::SecretPresent => format!("needs {}", step.verify().keys.join(", ")),
                _ => "by the plugin".to_string(),
            }
        );
    }
    save_mark(out, path, id, done)
}

fn save_mark(out: Out, path: &Path, id: &str, done: bool) -> Result<()> {
    let state_path = SetupState::path_for(path);
    let mut state = SetupState::load(&state_path)?;
    if done {
        state.mark_done(id);
    } else {
        state.unmark(id);
    }
    state.save(&state_path)?;
    let word = if done { "done" } else { "not done" };
    out.success(
        format!("marked {id} {word} (saved in {})", state_path.display()),
        json!({ "id": id, "done": done, "file": state_path.display().to_string() }),
    );
    Ok(())
}

pub async fn migrate(out: Out, path: &Path, apply: bool, allow: &[String]) -> Result<()> {
    let registry = plugins::builtin();
    let (config, secrets) = load_checked(path, &registry).await?;
    let providers = connect(&config, secrets).await?;
    let Some(sql) = providers.sql.clone() else {
        bail!("no database: add providers.db to {}", path.display());
    };
    let order: Vec<Arc<dyn Plugin>> = keel_core::plugins::check(&config, &registry, &providers)?
        .into_iter()
        .map(|(_, p)| p)
        .collect();
    let mut plan = migrate::plan(sql.as_ref(), &order, allow).await?;
    for a in allow {
        match plan.migrations.iter().find(|m| &m.id == a) {
            None => bail!("`{a}` is not a migration of any configured plugin; see `keel migrate`"),
            Some(m) if m.kind != Kind::Destructive => bail!("`{a}` is additive; --apply runs it"),
            Some(m) if m.state == State::Applied => bail!("`{a}` already ran"),
            Some(_) => {}
        }
    }
    migrate::dry_run(sql.as_ref(), &mut plan).await;

    let pending: Vec<_> = plan.pending().collect();
    let mut text = if pending.is_empty() {
        format!("migrations: all {} applied, nothing to do", plan.migrations.len())
    } else {
        format!("migrations: {} pending", pending.len())
    };
    for m in &pending {
        let tag = match (m.kind, m.state) {
            (Kind::Additive, State::Pending) => "additive   ",
            (Kind::Destructive, State::Pending) => "DESTRUCTIVE (allowed)",
            (Kind::Destructive, _) => "DESTRUCTIVE",
            (Kind::Additive, _) => "waiting    ",
        };
        text.push_str(&format!("\n  {tag} {}  tables: {}", m.id, m.tables.join(", ")));
        for r in &m.reasons {
            text.push_str(&format!("\n      - {r}"));
        }
        for (t, n) in &m.row_counts {
            let n = n.map_or("table missing".to_string(), |n| format!("{n} rows"));
            text.push_str(&format!("\n      {t}: {n} now"));
        }
        if m.kind == Kind::Destructive && m.state == State::Blocked {
            text.push_str(&format!(
                "\n      blocked: run `keel migrate --allow-destructive {}` to back up the tables and run it",
                m.id
            ));
        } else if m.kind == Kind::Additive && m.state == State::Blocked {
            text.push_str("\n      waits for the destructive migration before it");
        }
    }

    let mut applied = Vec::new();
    if apply || !allow.is_empty() {
        applied = migrate::apply(sql.as_ref(), &plan, "keel migrate").await?;
        for a in &applied {
            text.push_str(&format!("\napplied {}", a.id));
            for b in &a.backups {
                text.push_str(&format!("\n  backup table: {b}"));
            }
        }
        if applied.is_empty() {
            text.push_str("\nnothing ran");
        }
    } else if !pending.is_empty() {
        text.push_str("\ndry run: nothing changed. Run with --apply to run the additive ones");
    }
    let ran = apply || !allow.is_empty();
    let blocked: Vec<String> = plan
        .migrations
        .iter()
        .filter(|m| m.state == State::Blocked)
        .map(|m| m.id.clone())
        .collect();
    let data = json!({ "plan": plan, "applied": applied, "dry_run": !ran, "blocked": blocked });
    if ran && !blocked.is_empty() {
        text.push_str(&format!(
            "
error: {} migration(s) still blocked: {}",
            blocked.len(),
            blocked.join(", ")
        ));
        out.report(false, text, data);
        return Err(Reported.into());
    }
    out.success(text, data);
    Ok(())
}

/// The command already printed its result and failed; exit non-zero without
/// printing again.
#[derive(Debug)]
pub struct Reported;

impl std::fmt::Display for Reported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("already reported")
    }
}

impl std::error::Error for Reported {}
