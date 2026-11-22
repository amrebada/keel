//! Migration safety (E4).
//!
//! Every plugin migration is classified before it runs:
//! - additive: CREATE TABLE/INDEX, ALTER TABLE ... ADD, INSERT, UPDATE with WHERE.
//!   These run automatically at start.
//! - destructive: DROP, column drop/rename/type change, TRUNCATE, any DELETE,
//!   UPDATE without WHERE, and anything not recognized. These never run at
//!   start; they need `keel migrate --allow-destructive <plugin>/<id>`, which
//!   prints a dry-run plan with row counts and copies the affected tables to
//!   `keel_backup_<time>_<table>` first.
//!
//! A migration may only touch tables and indexes in its plugin's namespace
//! (`{ns}` in the SQL). Applied migrations are recorded in `keel_migrations`
//! with a checksum, and every run is written to `keel_audit`.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use keel_sdk::capabilities::Sql;
use keel_sdk::{Issue, Plugin};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Additive,
    Destructive,
}

/// What a migration's SQL does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Classified {
    pub kind: Kind,
    /// Why it is destructive, one line per statement.
    pub reasons: Vec<String>,
    /// Tables the SQL touches.
    pub tables: Vec<String>,
    /// Tables or indexes outside the plugin's namespace.
    pub outside_namespace: Vec<String>,
}

/// Splits on `;` outside quotes and comments; drops empty statements.
pub fn split_statements(sql: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut chars = sql.chars().peekable();
    while let Some(c) = chars.next() {
        match quote {
            Some(q) => {
                cur.push(c);
                if c == q {
                    quote = None;
                }
            }
            None => match c {
                '\'' | '"' | '`' => {
                    quote = Some(c);
                    cur.push(c);
                }
                '-' if chars.peek() == Some(&'-') => {
                    for n in chars.by_ref() {
                        if n == '\n' {
                            cur.push('\n');
                            break;
                        }
                    }
                }
                ';' => {
                    if !cur.trim().is_empty() {
                        out.push(cur.trim().to_string());
                    }
                    cur.clear();
                }
                _ => cur.push(c),
            },
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

/// Words of a statement: identifiers unquoted, punctuation split off.
fn words(stmt: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let flush = |cur: &mut String, out: &mut Vec<String>| {
        if !cur.is_empty() {
            out.push(std::mem::take(cur));
        }
    };
    let mut in_str = false;
    for c in stmt.chars() {
        if in_str {
            if c == '\'' {
                in_str = false;
            }
            continue;
        }
        match c {
            '\'' => {
                flush(&mut cur, &mut out);
                in_str = true;
                out.push("'…'".into());
            }
            '"' | '`' | '[' | ']' => {}
            '(' | ')' | ',' => {
                flush(&mut cur, &mut out);
                out.push(c.to_string());
            }
            c if c.is_whitespace() => flush(&mut cur, &mut out),
            c => cur.push(c),
        }
    }
    flush(&mut cur, &mut out);
    out
}

fn upper(w: &[String]) -> Vec<String> {
    w.iter().map(|s| s.to_ascii_uppercase()).collect()
}

/// The identifier after skipping optional `IF [NOT] EXISTS` at `i`.
fn ident_at(w: &[String], u: &[String], mut i: usize) -> Option<String> {
    if u.get(i).map(String::as_str) == Some("IF") {
        i += 1;
        if u.get(i).map(String::as_str) == Some("NOT") {
            i += 1;
        }
        i += 1; // EXISTS
    }
    w.get(i).map(|s| s.trim_end_matches(';').to_string())
}

fn after(u: &[String], word: &str) -> Option<usize> {
    u.iter().position(|x| x == word).map(|i| i + 1)
}

/// Classifies `sql` (with `{ns}` already replaced by `ns`).
pub fn classify(sql: &str, ns: &str) -> Classified {
    let mut reasons = Vec::new();
    let mut tables = BTreeSet::new();
    let mut names = BTreeSet::new(); // tables and indexes, for the namespace check
    for stmt in split_statements(sql) {
        let w = words(&stmt);
        let u = upper(&w);
        let first: Vec<&str> = u.iter().take(3).map(String::as_str).collect();
        let short = stmt.split_whitespace().collect::<Vec<_>>().join(" ");
        let short: String = short.chars().take(80).collect();
        match first.as_slice() {
            ["CREATE", "TABLE", ..] => {
                if let Some(t) = ident_at(&w, &u, 2) {
                    tables.insert(t.clone());
                    names.insert(t);
                }
                if u.contains(&"AS".to_string()) && u.contains(&"SELECT".to_string()) {
                    // CREATE TABLE x AS SELECT reads other tables: fine, additive.
                }
            }
            ["CREATE", "INDEX", ..] | ["CREATE", "UNIQUE", "INDEX"] => {
                let at = if first[1] == "UNIQUE" { 3 } else { 2 };
                if let Some(ix) = ident_at(&w, &u, at) {
                    names.insert(ix);
                }
                if let Some(i) = after(&u, "ON") {
                    if let Some(t) = w.get(i) {
                        tables.insert(t.clone());
                        names.insert(t.clone());
                    }
                }
            }
            ["ALTER", "TABLE", ..] => {
                if let Some(t) = ident_at(&w, &u, 2) {
                    tables.insert(t.clone());
                    names.insert(t);
                }
                let rest: Vec<&str> = u.iter().skip(3).map(String::as_str).collect();
                let has = |k: &str| rest.contains(&k);
                if has("DROP") {
                    reasons.push(format!("drops a column or constraint: {short}"));
                } else if has("RENAME") {
                    reasons.push(format!("renames a table or column: {short}"));
                } else if has("MODIFY") || has("CHANGE") || (has("ALTER") && (has("TYPE") || has("COLUMN"))) {
                    reasons.push(format!("changes a column definition: {short}"));
                } else if !has("ADD") {
                    reasons.push(format!("unrecognized ALTER TABLE: {short}"));
                }
            }
            ["DROP", ..] => {
                let what = first.get(1).copied().unwrap_or("");
                if let Some(n) = ident_at(&w, &u, 2) {
                    names.insert(n.clone());
                    if what == "TABLE" {
                        tables.insert(n);
                    }
                }
                if let Some(i) = after(&u, "ON") {
                    if let Some(t) = w.get(i) {
                        tables.insert(t.clone());
                        names.insert(t.clone());
                    }
                }
                reasons.push(format!("drops {}: {short}", what.to_ascii_lowercase()));
            }
            ["TRUNCATE", ..] => {
                let at = if first.get(1) == Some(&"TABLE") { 2 } else { 1 };
                if let Some(t) = w.get(at) {
                    tables.insert(t.clone());
                    names.insert(t.clone());
                }
                reasons.push(format!("deletes every row: {short}"));
            }
            ["DELETE", ..] => {
                if let Some(t) = after(&u, "FROM").and_then(|i| w.get(i)) {
                    tables.insert(t.clone());
                    names.insert(t.clone());
                }
                if u.contains(&"WHERE".to_string()) {
                    reasons.push(format!("deletes rows: {short}"));
                } else {
                    reasons.push(format!("deletes every row (no WHERE): {short}"));
                }
            }
            ["UPDATE", ..] => {
                if let Some(t) = w.get(1) {
                    tables.insert(t.clone());
                    names.insert(t.clone());
                }
                if !u.contains(&"WHERE".to_string()) {
                    reasons.push(format!("rewrites every row (no WHERE): {short}"));
                }
            }
            ["INSERT", ..] => {
                if let Some(t) = after(&u, "INTO").and_then(|i| w.get(i)) {
                    tables.insert(t.clone());
                    names.insert(t.clone());
                }
            }
            _ => reasons.push(format!("unrecognized statement, treated as destructive: {short}")),
        }
        if let Some(i) = after(&u, "REFERENCES") {
            if let Some(t) = w.get(i) {
                names.insert(t.clone());
            }
        }
    }
    let outside_namespace = names.iter().filter(|n| !n.starts_with(ns)).cloned().collect();
    Classified {
        kind: if reasons.is_empty() {
            Kind::Additive
        } else {
            Kind::Destructive
        },
        reasons,
        tables: tables.into_iter().collect(),
        outside_namespace,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Applied,
    Pending,
    /// Destructive and not allowed, or after one that is.
    Blocked,
}

/// One migration in a plan.
#[derive(Debug, Clone, Serialize)]
pub struct Planned {
    /// `<plugin>/<id>`.
    pub id: String,
    pub plugin: String,
    pub kind: Kind,
    pub state: State,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub reasons: Vec<String>,
    pub tables: Vec<String>,
    /// Rows now in each touched table (dry run, destructive only).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub row_counts: Vec<(String, Option<i64>)>,
    #[serde(skip)]
    statements: Vec<String>,
    #[serde(skip)]
    checksum: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Plan {
    pub migrations: Vec<Planned>,
}

impl Plan {
    pub fn pending(&self) -> impl Iterator<Item = &Planned> {
        self.migrations.iter().filter(|m| m.state != State::Applied)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MigrateError {
    #[error("{}", keel_sdk::issues::format_list("migrations", .0))]
    Invalid(Vec<Issue>),
    #[error("migrations: {0}")]
    Sql(String),
}

impl From<keel_sdk::CapabilityError> for MigrateError {
    fn from(e: keel_sdk::CapabilityError) -> Self {
        Self::Sql(e.to_string())
    }
}

const TRACKING: [&str; 2] = [
    "CREATE TABLE IF NOT EXISTS keel_migrations (plugin VARCHAR(100) NOT NULL, id VARCHAR(200) NOT NULL, \
     checksum VARCHAR(64) NOT NULL, kind VARCHAR(20) NOT NULL, applied_at BIGINT NOT NULL, PRIMARY KEY (plugin, id))",
    "CREATE TABLE IF NOT EXISTS keel_audit (id VARCHAR(40) NOT NULL PRIMARY KEY, at BIGINT NOT NULL, \
     actor VARCHAR(200) NOT NULL, action VARCHAR(100) NOT NULL, target VARCHAR(300) NOT NULL, detail TEXT)",
];

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

fn namespace(plugin_id: &str) -> String {
    format!("{}_", plugin_id.replace('-', "_"))
}

/// Builds the plan for `plugins` (in start order). `allow` lists destructive
/// migrations the user allowed, as `<plugin>/<id>`.
pub async fn plan(
    sql: &dyn Sql,
    plugins: &[Arc<dyn Plugin>],
    allow: &[String],
) -> Result<Plan, MigrateError> {
    let with: Vec<(Arc<dyn Plugin>, Value)> = plugins.iter().map(|p| (p.clone(), Value::Null)).collect();
    plan_with(sql, &with, allow).await
}

/// [`plan`] for plugins whose migrations depend on their `app.yaml` config
/// (e.g. `resource` builds tables from YAML models).
pub async fn plan_with(
    sql: &dyn Sql,
    plugins: &[(Arc<dyn Plugin>, Value)],
    allow: &[String],
) -> Result<Plan, MigrateError> {
    for t in TRACKING {
        sql.execute(t, &[]).await?;
    }
    let applied = sql
        .query("SELECT plugin, id, checksum FROM keel_migrations", &[])
        .await?;
    let applied: Vec<(String, String, String)> = applied
        .into_iter()
        .map(|r| {
            let s = |k: &str| r.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
            (s("plugin"), s("id"), s("checksum"))
        })
        .collect();
    let mut issues = Vec::new();
    let mut out = Plan::default();
    for (plugin, config) in plugins {
        let m = plugin.manifest();
        let mut list = plugin.migrations_for(sql.dialect(), config);
        if list.is_empty() {
            continue;
        }
        if !m.capabilities.contains(&keel_sdk::Capability::Sql) {
            issues.push(Issue::new(
                m.id.clone(),
                "has migrations but does not declare the `sql` capability",
            ));
            continue;
        }
        list.sort_by(|a, b| a.id.cmp(&b.id));
        let ns = namespace(&m.id);
        let mut blocked = false;
        for mig in list {
            let id = format!("{}/{}", m.id, mig.id);
            let body = mig.up.replace("{ns}", &ns);
            let checksum = hex(&Sha256::digest(mig.up.as_bytes()));
            let c = classify(&body, &ns);
            if !c.outside_namespace.is_empty() {
                issues.push(Issue::new(
                    id.clone(),
                    format!(
                        "touches {} outside its namespace `{ns}`; a plugin may only change its own tables",
                        c.outside_namespace.join(", ")
                    ),
                ));
            }
            let done = applied.iter().find(|(p, i, _)| p == &m.id && i == &mig.id);
            let state = match done {
                Some((_, _, sum)) => {
                    if sum != &checksum {
                        issues.push(Issue::new(
                            id.clone(),
                            "was changed after it ran; add a new migration instead of editing one that shipped",
                        ));
                    }
                    State::Applied
                }
                None if blocked => State::Blocked,
                None if c.kind == Kind::Destructive && !allow.contains(&id) => {
                    blocked = true;
                    State::Blocked
                }
                None => State::Pending,
            };
            out.migrations.push(Planned {
                id,
                plugin: m.id.clone(),
                kind: c.kind,
                state,
                reasons: c.reasons,
                tables: c.tables,
                row_counts: Vec::new(),
                statements: split_statements(&body),
                checksum,
            });
        }
    }
    if !issues.is_empty() {
        return Err(MigrateError::Invalid(issues));
    }
    Ok(out)
}

/// Fills row counts for every destructive, not yet applied migration.
pub async fn dry_run(sql: &dyn Sql, plan: &mut Plan) {
    for m in plan.migrations.iter_mut() {
        if m.state == State::Applied || m.kind != Kind::Destructive {
            continue;
        }
        for t in &m.tables {
            let n = sql
                .query(&format!("SELECT COUNT(*) AS n FROM {t}"), &[])
                .await
                .ok()
                .and_then(|rows| rows.first().and_then(|r| r.get("n")).and_then(Value::as_i64));
            m.row_counts.push((t.clone(), n));
        }
    }
}

/// What [`apply`] did.
#[derive(Debug, Clone, Serialize)]
pub struct Applied {
    pub id: String,
    pub kind: Kind,
    /// Backup tables made before a destructive migration.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub backups: Vec<String>,
}

/// Runs every `Pending` migration in `plan`, in order. Destructive ones get
/// a backup of each touched table first. Each migration and its record run
/// in one transaction where the database allows it.
pub async fn apply(sql: &dyn Sql, plan: &Plan, actor: &str) -> Result<Vec<Applied>, MigrateError> {
    let mut done = Vec::new();
    for m in plan.migrations.iter().filter(|m| m.state == State::Pending) {
        let mut backups = Vec::new();
        if m.kind == Kind::Destructive {
            let stamp = now();
            for t in &m.tables {
                let exists = sql
                    .query(&format!("SELECT COUNT(*) AS n FROM {t}"), &[])
                    .await
                    .is_ok();
                if !exists {
                    continue;
                }
                let name = format!("keel_backup_{stamp}_{t}");
                sql.execute(&format!("CREATE TABLE {name} AS SELECT * FROM {t}"), &[])
                    .await?;
                backups.push(name);
            }
        }
        let mig_id = m.id.split_once('/').map(|(_, i)| i).unwrap_or(&m.id);
        let mut stmts = m.statements.clone();
        stmts.push(format!(
            "INSERT INTO keel_migrations (plugin, id, checksum, kind, applied_at) VALUES ('{}', '{}', '{}', '{}', {})",
            esc(&m.plugin),
            esc(mig_id),
            m.checksum,
            if m.kind == Kind::Additive { "additive" } else { "destructive" },
            now()
        ));
        sql.transaction(&stmts).await.map_err(|e| {
            MigrateError::Sql(format!(
                "{} failed, nothing from it was kept where the database allows: {e}",
                m.id
            ))
        })?;
        audit(
            sql,
            actor,
            "migration.applied",
            &m.id,
            json!({ "kind": m.kind, "tables": m.tables, "reasons": m.reasons, "backups": backups }),
        )
        .await?;
        tracing::info!(migration = %m.id, kind = ?m.kind, "migration applied");
        done.push(Applied {
            id: m.id.clone(),
            kind: m.kind,
            backups,
        });
    }
    Ok(done)
}

/// Writes one row to `keel_audit`.
pub async fn audit(
    sql: &dyn Sql,
    actor: &str,
    action: &str,
    target: &str,
    detail: Value,
) -> Result<(), MigrateError> {
    sql.execute(
        "INSERT INTO keel_audit (id, at, actor, action, target, detail) VALUES (?, ?, ?, ?, ?, ?)",
        &[
            json!(uuid::Uuid::new_v4().to_string()),
            json!(now()),
            json!(actor),
            json!(action),
            json!(target),
            json!(detail.to_string()),
        ],
    )
    .await?;
    Ok(())
}

/// At start: applies additive migrations, refuses to continue past a
/// destructive one.
pub async fn run_at_start(sql: &dyn Sql, plugins: &[Arc<dyn Plugin>]) -> Result<Vec<Applied>, MigrateError> {
    let with: Vec<(Arc<dyn Plugin>, Value)> = plugins.iter().map(|p| (p.clone(), Value::Null)).collect();
    run_at_start_with(sql, &with).await
}

/// [`run_at_start`] with each plugin's config.
pub async fn run_at_start_with(
    sql: &dyn Sql,
    plugins: &[(Arc<dyn Plugin>, Value)],
) -> Result<Vec<Applied>, MigrateError> {
    let plan = plan_with(sql, plugins, &[]).await?;
    let blocked: Vec<Issue> = plan
        .migrations
        .iter()
        .filter(|m| m.state == State::Blocked && m.kind == Kind::Destructive)
        .map(|m| {
            Issue::new(
                m.id.clone(),
                format!(
                    "is destructive ({}) and never runs automatically. Review with `keel migrate`, then run `keel migrate --allow-destructive {}`",
                    m.reasons.join("; "),
                    m.id
                ),
            )
        })
        .collect();
    if !blocked.is_empty() {
        return Err(MigrateError::Invalid(blocked));
    }
    apply(sql, &plan, "host start").await
}

fn esc(s: &str) -> String {
    s.replace('\'', "''")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NS: &str = "auth_core_";

    #[test]
    fn splits_outside_quotes_and_comments() {
        let s = split_statements(
            "CREATE TABLE a (x TEXT DEFAULT ';'); -- c; here\nINSERT INTO a VALUES ('x;y');;",
        );
        assert_eq!(s.len(), 2, "{s:?}");
        assert!(s[1].starts_with("INSERT"));
    }

    #[test]
    fn additive() {
        let c = classify(
            "CREATE TABLE IF NOT EXISTS auth_core_users (id VARCHAR(36) PRIMARY KEY);
             CREATE UNIQUE INDEX auth_core_users_email ON auth_core_users (email);
             ALTER TABLE auth_core_users ADD COLUMN name TEXT;
             INSERT INTO auth_core_users (id) VALUES ('1');
             UPDATE auth_core_users SET name = 'x' WHERE name IS NULL",
            NS,
        );
        assert_eq!(c.kind, Kind::Additive, "{:?}", c.reasons);
        assert_eq!(c.tables, vec!["auth_core_users"]);
        assert!(c.outside_namespace.is_empty());
    }

    #[test]
    fn destructive() {
        for sql in [
            "DROP TABLE auth_core_old",
            "ALTER TABLE auth_core_users DROP COLUMN legacy",
            "ALTER TABLE auth_core_users RENAME COLUMN a TO b",
            "ALTER TABLE auth_core_users ALTER COLUMN a TYPE INT",
            "ALTER TABLE auth_core_users MODIFY a INT",
            "TRUNCATE auth_core_users",
            "DELETE FROM auth_core_users",
            "DELETE FROM auth_core_users WHERE id = '1'",
            "UPDATE auth_core_users SET a = 1",
            "GRANT ALL ON auth_core_users TO x",
        ] {
            let c = classify(sql, NS);
            assert_eq!(c.kind, Kind::Destructive, "{sql}");
            assert_eq!(c.reasons.len(), 1, "{sql}");
        }
    }

    #[test]
    fn namespace_is_enforced() {
        let c = classify(
            "CREATE TABLE auth_core_x (u VARCHAR(36) REFERENCES billing_customers(id)); DROP TABLE billing_plans",
            NS,
        );
        assert_eq!(c.outside_namespace, vec!["billing_customers", "billing_plans"]);
    }
}
