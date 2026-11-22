//! The v1 success test, end to end: the team task app from
//! `examples/team-tasks/app.yaml` runs under `keel supervise` on Postgres and
//! Redis, passes the black-box `acceptance.py`, then takes a live config
//! change and a binary swap under load without dropping a request.
//!
//! Needs `KEEL_TEST_POSTGRES_URL`, `KEEL_TEST_REDIS_URL` and `python3`;
//! skipped otherwise.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use keel_providers::SqlProvider;
use keel_sdk::capabilities::Sql;
use serde_json::{json, Value};

const KEEL: &str = env!("CARGO_BIN_EXE_keel");

struct Supervisor(Child);

impl Drop for Supervisor {
    fn drop(&mut self) {
        unsafe {
            libc::kill(self.0.id() as i32, libc::SIGTERM);
        }
        let _ = self.0.wait();
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn example() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/team-tasks")
}

fn keel(dir: &Path, args: &[&str]) -> (bool, Value) {
    let out = Command::new(KEEL)
        .current_dir(dir)
        .arg("--json")
        .args(args)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    let v = serde_json::from_str(text.lines().last().unwrap_or("null")).unwrap_or(Value::Null);
    (out.status.success(), v)
}

/// `url` with its database name replaced by `name`.
fn with_db(url: &str, name: &str) -> String {
    let (base, query) = url.split_once('?').map_or((url, None), |(b, q)| (b, Some(q)));
    let rest = base.split_once("://").map_or(base, |(_, r)| r);
    let base = match rest.find('/') {
        Some(_) => &base[..base.rfind('/').unwrap()],
        None => base,
    };
    match query {
        Some(q) => format!("{base}/{name}?{q}"),
        None => format!("{base}/{name}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn team_tasks_app_passes_and_updates_live() {
    let (Ok(pg), Ok(redis)) = (
        std::env::var("KEEL_TEST_POSTGRES_URL"),
        std::env::var("KEEL_TEST_REDIS_URL"),
    ) else {
        eprintln!("skipped: set KEEL_TEST_POSTGRES_URL and KEEL_TEST_REDIS_URL");
        return;
    };
    if Command::new("python3").arg("--version").output().is_err() {
        eprintln!("skipped: python3 is not installed");
        return;
    }

    // A fresh database for this run.
    let db = format!("keel_team_tasks_{}", std::process::id());
    let admin = SqlProvider::connect(&pg, Some(1)).await.unwrap();
    let _ = admin.execute(&format!("DROP DATABASE IF EXISTS {db}"), &[]).await;
    admin
        .execute(&format!("CREATE DATABASE {db}"), &[])
        .await
        .unwrap();

    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let (port, mock) = (free_port(), free_port());
    let base = format!("http://127.0.0.1:{port}");
    let app = std::fs::read_to_string(example().join("app.yaml"))
        .unwrap()
        .replace("port: 8080", &format!("port: {port}"));
    std::fs::write(dir.join("app.yaml"), &app).unwrap();
    std::fs::write(
        dir.join(".env"),
        format!(
            "DATABASE_URL={}\nREDIS_URL={redis}\nPUBLIC_URL={base}\nAUTH_JWT_SECRET=team-tasks-test-secret-0123456789abcdef\n\
             OAUTH_GOOGLE_CLIENT_ID=client-tt\nOAUTH_GOOGLE_CLIENT_SECRET=secret-tt\n\
             GOOGLE_AUTH_URL=http://127.0.0.1:{mock}/authorize\nGOOGLE_TOKEN_URL=http://127.0.0.1:{mock}/token\n\
             GOOGLE_USERINFO_URL=http://127.0.0.1:{mock}/userinfo\nKEEL_ADMIN_TOKEN=team-tasks-admin-token\n",
            with_db(&pg, &db)
        ),
    )
    .unwrap();
    let next = dir.join("keel-next");
    std::fs::copy(KEEL, &next).unwrap();

    let (ok, v) = keel(dir, &["migrate", "--apply"]);
    assert!(ok, "migrate: {v}");
    let _sup = Supervisor(
        Command::new(KEEL)
            .current_dir(dir)
            .args(["supervise", "--watch-secs", "3"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    let mut up = false;
    for _ in 0..300 {
        if dir.join(".keel/supervisor.sock").exists()
            && http
                .get(format!("{base}/health"))
                .send()
                .await
                .is_ok_and(|r| r.status().is_success())
        {
            up = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(up, "the app did not start");

    // 1. The black-box acceptance test.
    let script = example().join("acceptance.py");
    let (b, m) = (base.clone(), mock.to_string());
    let out = tokio::task::spawn_blocking(move || {
        Command::new("python3")
            .arg(script)
            .args([&b, &m])
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("checks passed"),
        "acceptance failed:\n{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );

    // 2. Load on a protected, cached resource as the admin.
    let login: Value = http
        .post(format!("{base}/auth/login"))
        .json(&json!({ "email": "boss@example.com", "password": "secret-pass-1" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let token = login["access_token"].as_str().expect("boss logs in").to_string();
    let created = http
        .post(format!("{base}/api/tasks"))
        .bearer_auth(&token)
        .header("x-tenant-id", "load")
        .json(&json!({ "title": "under load" }))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), 201);

    let stop = Arc::new(AtomicBool::new(false));
    let (ok, failed) = (Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)));
    let mut load = Vec::new();
    for i in 0..8 {
        let (http, base, token, stop, ok, failed) = (
            http.clone(),
            base.clone(),
            token.clone(),
            stop.clone(),
            ok.clone(),
            failed.clone(),
        );
        load.push(tokio::spawn(async move {
            let path = if i % 2 == 0 { "/api/tasks" } else { "/health" };
            while !stop.load(Ordering::Relaxed) {
                match http
                    .get(format!("{base}{path}"))
                    .bearer_auth(&token)
                    .header("x-tenant-id", "load")
                    .send()
                    .await
                {
                    Ok(r) if r.status().is_success() => ok.fetch_add(1, Ordering::Relaxed),
                    _ => failed.fetch_add(1, Ordering::Relaxed),
                };
            }
        }));
    }
    let d = dir.to_path_buf();
    let run = move |args: Vec<&'static str>| {
        let d = d.clone();
        tokio::task::spawn_blocking(move || keel(&d, &args))
    };
    tokio::time::sleep(Duration::from_millis(500)).await;

    // 3. A live config change: tasks gain a priority field (a migration and
    // a reload) and the cache TTL changes.
    std::fs::write(
        dir.join("app.yaml"),
        app.replacen("cache_ttl: 30", "cache_ttl: 10", 2).replace(
            "project_id: { type: string }",
            "project_id: { type: string }\n            priority: { type: integer, min: 1, max: 5 }",
        ),
    )
    .unwrap();
    let (applied, r) = run(vec!["apply"]).await.unwrap();
    assert!(applied, "apply: {r}");
    let row = http
        .post(format!("{base}/api/tasks"))
        .bearer_auth(&token)
        .header("x-tenant-id", "load")
        .json(&json!({ "title": "prioritised", "priority": 2 }))
        .send()
        .await
        .unwrap();
    assert_eq!(row.status(), 201, "the new field works after the apply");

    // 4. A zero-downtime binary swap.
    let (swapped, r) = run(vec!["update", "--binary", "keel-next"]).await.unwrap();
    assert!(swapped, "update: {r}");

    tokio::time::sleep(Duration::from_millis(500)).await;
    stop.store(true, Ordering::Relaxed);
    for l in load {
        l.await.unwrap();
    }
    let (ok, failed) = (ok.load(Ordering::Relaxed), failed.load(Ordering::Relaxed));
    assert!(ok > 500, "load ran: {ok}");
    assert_eq!(failed, 0, "{failed} of {} requests failed", ok + failed);

    // The data survived both changes.
    let list: Value = http
        .get(format!("{base}/api/tasks?sort=title"))
        .bearer_auth(&token)
        .header("x-tenant-id", "load")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list["total"], 2, "{list}");

    drop(_sup);
    let _ = admin
        .execute(&format!("DROP DATABASE IF EXISTS {db} WITH (FORCE)"), &[])
        .await;
}

#[test]
fn database_names_are_swapped() {
    assert_eq!(with_db("postgres://u@h:1/postgres", "x"), "postgres://u@h:1/x");
    assert_eq!(with_db("postgres://u@h:1", "x"), "postgres://u@h:1/x");
    assert_eq!(
        with_db("postgres://u@h/db?sslmode=disable", "x"),
        "postgres://u@h/x?sslmode=disable"
    );
}
