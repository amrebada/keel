//! Provider tests. SQLite and memory always run. Postgres, MySQL and Redis
//! run when KEEL_TEST_POSTGRES_URL, KEEL_TEST_MYSQL_URL or KEEL_TEST_REDIS_URL
//! is set (CI sets the Postgres and Redis ones).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use keel_core::{AppConfig, MapSecrets};
use keel_providers::{connect, MemoryStore, RedisStore, SqlProvider};
use keel_sdk::capabilities::{Cache, Capability, Kv, Sql};
use serde_json::json;

async fn sql_roundtrip(db: &dyn Sql, table: &str) {
    db.execute(&format!("DROP TABLE IF EXISTS {table}"), &[])
        .await
        .unwrap();
    db.execute(
        &format!("CREATE TABLE {table} (id INTEGER PRIMARY KEY, name VARCHAR(100) NOT NULL, score DOUBLE PRECISION, active BOOLEAN)"),
        &[],
    )
    .await
    .unwrap();
    let n = db
        .execute(
            &format!("INSERT INTO {table} (id, name, score, active) VALUES (?, ?, ?, ?), (?, ?, ?, ?)"),
            &[
                json!(1),
                json!("ada"),
                json!(9.5),
                json!(true),
                json!(2),
                json!("bob"),
                json!(7.25),
                json!(false),
            ],
        )
        .await
        .unwrap();
    assert_eq!(n, 2);
    let rows = db
        .query(
            &format!("SELECT id, name, score FROM {table} WHERE name = ? OR id = ? ORDER BY id"),
            &[json!("ada"), json!(2)],
        )
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["id"], json!(1));
    assert_eq!(rows[0]["name"], json!("ada"));
    assert_eq!(rows[0]["score"], json!(9.5));
    assert_eq!(rows[1]["name"], json!("bob"));
    let n = db
        .execute(&format!("DELETE FROM {table} WHERE id = ?"), &[json!(1)])
        .await
        .unwrap();
    assert_eq!(n, 1);
    db.ping().await.unwrap();
    db.execute(&format!("DROP TABLE {table}"), &[]).await.unwrap();
}

async fn kv_roundtrip(kv: &dyn Kv, cache: &dyn Cache, prefix: &str) {
    let k = format!("{prefix}k");
    kv.set(&k, b"v1".to_vec()).await.unwrap();
    assert_eq!(kv.get(&k).await.unwrap(), Some(b"v1".to_vec()));
    assert!(kv.delete(&k).await.unwrap());
    assert!(!kv.delete(&k).await.unwrap());
    assert_eq!(kv.get(&k).await.unwrap(), None);

    let c = format!("{prefix}c");
    cache
        .set(&c, b"hot".to_vec(), Duration::from_millis(150))
        .await
        .unwrap();
    assert_eq!(cache.get(&c).await.unwrap(), Some(b"hot".to_vec()));
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(cache.get(&c).await.unwrap(), None, "expired");
    kv.ping().await.unwrap();
}

#[tokio::test]
async fn sqlite_in_memory() {
    let db = SqlProvider::connect("sqlite::memory:", None).await.unwrap();
    assert_eq!(db.dialect(), "sqlite");
    sql_roundtrip(&db, "t_mem").await;
}

#[tokio::test]
async fn sqlite_file_is_created() {
    let dir = std::env::temp_dir().join(format!("keel-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("app.db");
    let db = SqlProvider::connect(&format!("sqlite://{}", path.display()), None)
        .await
        .unwrap();
    sql_roundtrip(&db, "t_file").await;
    assert!(path.exists());
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn memory_store() {
    let m = MemoryStore::new();
    kv_roundtrip(&m, &m, "").await;
}

#[tokio::test]
async fn postgres() {
    let Ok(url) = std::env::var("KEEL_TEST_POSTGRES_URL") else {
        return;
    };
    let db = SqlProvider::connect(&url, None).await.unwrap();
    assert_eq!(db.dialect(), "postgres");
    sql_roundtrip(&db, "keel_t_pg").await;
}

#[tokio::test]
async fn mysql() {
    let Ok(url) = std::env::var("KEEL_TEST_MYSQL_URL") else {
        return;
    };
    let db = SqlProvider::connect(&url, None).await.unwrap();
    assert_eq!(db.dialect(), "mysql");
    sql_roundtrip(&db, "keel_t_my").await;
}

#[tokio::test]
async fn redis() {
    let Ok(url) = std::env::var("KEEL_TEST_REDIS_URL") else {
        return;
    };
    let r = RedisStore::connect(&url, "keel-test:").await.unwrap();
    kv_roundtrip(&r, &r, &format!("{}:", std::process::id())).await;
}

fn providers_config(yaml: &str) -> BTreeMap<String, keel_core::ProviderConfig> {
    AppConfig::load_str(
        &format!("app: t\nproviders:\n{yaml}"),
        "app.yaml",
        &MapSecrets::default(),
    )
    .unwrap()
    .providers
}

#[tokio::test]
async fn connects_from_app_yaml_and_namespaces_each_plugin() {
    let cfg = providers_config("  db: { use: sqlite, url: \"sqlite::memory:\" }\n  cache: { use: memory }\n  kv: { use: memory, prefix: \"app:\" }\n");
    let p = connect(&cfg).await.unwrap();
    let slots: Vec<(&str, &str, Capability, bool)> = p
        .info
        .iter()
        .map(|i| (i.slot.as_str(), i.kind.as_str(), i.capability, i.hot_load))
        .collect();
    assert_eq!(
        slots,
        vec![
            ("cache", "memory", Capability::Cache, true),
            ("db", "sqlite", Capability::Sql, false),
            ("kv", "memory", Capability::Kv, true)
        ]
    );

    let a = p.grant("auth-core", &[Capability::Kv, Capability::Sql]);
    let b = p.grant("billing", &[Capability::Kv]);
    a.kv().unwrap().set("user:1", b"a".to_vec()).await.unwrap();
    assert_eq!(
        b.kv().unwrap().get("user:1").await.unwrap(),
        None,
        "plugins cannot see each other's keys"
    );
    assert_eq!(a.kv().unwrap().get("user:1").await.unwrap(), Some(b"a".to_vec()));
    assert_eq!(a.table("users"), "auth_core_users");
    assert!(b.sql().is_err(), "billing did not declare sql");
    let raw: Arc<dyn Kv> = p.kv.clone().unwrap();
    assert_eq!(
        raw.get("auth-core:user:1").await.unwrap(),
        Some(b"a".to_vec()),
        "stored under the plugin's prefix"
    );
}

#[tokio::test]
async fn reports_bad_providers() {
    let e = connect(&providers_config(
        "  db: { use: postgres, url: \"sqlite::memory:\" }\n",
    ))
    .await
    .unwrap_err();
    assert!(
        e.to_string()
            .contains("providers.db: cannot connect to postgres: the url is for sqlite"),
        "{e}"
    );

    let e = connect(&providers_config(
        "  cache: { use: redis, url: \"redis://127.0.0.1:1\" }\n",
    ))
    .await
    .unwrap_err();
    assert!(
        e.to_string()
            .starts_with("providers.cache: cannot connect to redis"),
        "{e}"
    );
}

#[test]
fn app_schema_checks_provider_slots() {
    let bad = "app: t\nproviders:\n  database: { use: postgres }\n  cache: { use: postgres }\n  kv: { use: redis }\n  db: { use: sqlite }\n";
    let e = AppConfig::load_str(bad, "app.yaml", &MapSecrets::default())
        .unwrap_err()
        .to_string();
    assert!(e.contains("'database' was unexpected"), "{e}");
    assert!(e.contains("providers.cache.use"), "{e}");
    assert!(e.contains("providers.kv: \"url\" is a required property"), "{e}");
    assert!(e.contains("providers.db: \"url\" is a required property"), "{e}");
}
