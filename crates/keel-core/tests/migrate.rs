use std::sync::{Arc, Mutex};

use keel_core::migrate::{self, Kind, State};
use keel_sdk::capabilities::{CapResult, Sql};
use keel_sdk::prelude::*;
use serde_json::Map;

/// A plugin whose migrations a test can change.
struct Mig {
    list: Mutex<Vec<Migration>>,
}

impl Mig {
    fn new(list: Vec<Migration>) -> Arc<Self> {
        Arc::new(Self { list: Mutex::new(list) })
    }
}

#[async_trait]
impl Plugin for Mig {
    fn manifest(&self) -> &Manifest {
        static M: std::sync::OnceLock<Manifest> = std::sync::OnceLock::new();
        M.get_or_init(|| {
            Manifest::from_sources(
                "id: notes\nversion: 1.0.0\nsummary: test\nrequires: { capabilities: [sql] }\n",
                None,
            )
            .unwrap()
        })
    }
    async fn init(&self, _: &mut InitContext) -> Result<(), PluginError> {
        Ok(())
    }
    fn migrations(&self, _: &str) -> Vec<Migration> {
        self.list.lock().unwrap().clone()
    }
}

/// An in-memory SQLite through the real provider.
async fn db() -> keel_providers::SqlProvider {
    keel_providers::SqlProvider::connect("sqlite::memory:", None).await.unwrap()
}

async fn count(sql: &dyn Sql, q: &str) -> i64 {
    let rows: CapResult<Vec<Map<String, Value>>> = sql.query(q, &[]).await;
    rows.unwrap()[0]["n"].as_i64().unwrap()
}

#[tokio::test]
async fn additive_runs_destructive_waits_for_permission() {
    let sql = db().await;
    let p = Mig::new(vec![
        Migration::new("001_notes", "CREATE TABLE {ns}notes (id VARCHAR(36) PRIMARY KEY, body TEXT, legacy TEXT)"),
        Migration::new("002_seed", "INSERT INTO {ns}notes (id, body) VALUES ('1', 'hi'), ('2', 'there')"),
    ]);
    let plugins: Vec<Arc<dyn Plugin>> = vec![p.clone()];
    let done = migrate::run_at_start(&sql, &plugins).await.unwrap();
    assert_eq!(done.len(), 2);
    assert_eq!(count(&sql, "SELECT COUNT(*) AS n FROM notes_notes").await, 2);

    // A destructive migration is refused at start, with the command to run.
    p.list
        .lock()
        .unwrap()
        .push(Migration::new("003_drop_legacy", "ALTER TABLE {ns}notes DROP COLUMN legacy"));
    p.list
        .lock()
        .unwrap()
        .push(Migration::new("004_index", "CREATE INDEX {ns}notes_body ON {ns}notes (body)"));
    let err = migrate::run_at_start(&sql, &plugins).await.unwrap_err().to_string();
    assert!(err.contains("notes/003_drop_legacy"), "{err}");
    assert!(err.contains("keel migrate --allow-destructive notes/003_drop_legacy"), "{err}");

    // The dry run shows what it touches and the rows there now; the index
    // after it waits.
    let mut plan = migrate::plan(&sql, &plugins, &[]).await.unwrap();
    migrate::dry_run(&sql, &mut plan).await;
    let m3 = plan.migrations.iter().find(|m| m.id == "notes/003_drop_legacy").unwrap();
    assert_eq!((m3.kind, m3.state), (Kind::Destructive, State::Blocked));
    assert_eq!(m3.row_counts, vec![("notes_notes".to_string(), Some(2))]);
    let m4 = plan.migrations.iter().find(|m| m.id == "notes/004_index").unwrap();
    assert_eq!(m4.state, State::Blocked);
    assert!(migrate::apply(&sql, &plan, "test").await.unwrap().is_empty());

    // Allowed: backup first, then run, then the waiting one.
    let allow = vec!["notes/003_drop_legacy".to_string()];
    let plan = migrate::plan(&sql, &plugins, &allow).await.unwrap();
    let done = migrate::apply(&sql, &plan, "test").await.unwrap();
    assert_eq!(done.len(), 2);
    assert_eq!(done[0].backups.len(), 1);
    let backup = &done[0].backups[0];
    assert!(backup.starts_with("keel_backup_") && backup.ends_with("_notes_notes"));
    assert_eq!(count(&sql, &format!("SELECT COUNT(*) AS n FROM {backup}")).await, 2);
    assert!(sql.query("SELECT legacy FROM notes_notes", &[]).await.is_err());

    // Everything is audited.
    let audit = count(&sql, "SELECT COUNT(*) AS n FROM keel_audit WHERE action = 'migration.applied'").await;
    assert_eq!(audit, 4);
}

#[tokio::test]
async fn edited_and_foreign_migrations_are_refused() {
    let sql = db().await;
    let p = Mig::new(vec![Migration::new("001", "CREATE TABLE {ns}a (id INT)")]);
    let plugins: Vec<Arc<dyn Plugin>> = vec![p.clone()];
    migrate::run_at_start(&sql, &plugins).await.unwrap();

    *p.list.lock().unwrap() = vec![Migration::new("001", "CREATE TABLE {ns}a (id BIGINT)")];
    let err = migrate::run_at_start(&sql, &plugins).await.unwrap_err().to_string();
    assert!(err.contains("was changed after it ran"), "{err}");

    *p.list.lock().unwrap() = vec![
        Migration::new("001", "CREATE TABLE {ns}a (id INT)"),
        Migration::new("002", "CREATE TABLE auth_core_users_copy (id INT)"),
    ];
    let err = migrate::run_at_start(&sql, &plugins).await.unwrap_err().to_string();
    assert!(err.contains("outside its namespace `notes_`"), "{err}");
}
