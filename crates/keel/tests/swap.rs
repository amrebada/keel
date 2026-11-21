//! `keel supervise` end to end: a binary swap and an automatic rollback
//! under constant load, plus a live apply, with zero failed requests.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

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

fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let p = dir.join(name);
    std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    p
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn swap_and_rollback_under_load() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let port = free_port();
    std::fs::write(
        dir.join("app.yaml"),
        format!(
            "app: swap\nserver: {{ host: 127.0.0.1, port: {port} }}\nproviders:\n  db: {{ use: sqlite, url: \"sqlite://app.db?mode=rwc\" }}\nplugins:\n  - use: ping\n    config: {{ message: one }}\n"
        ),
    )
    .unwrap();
    std::fs::write(dir.join(".env"), "KEEL_ADMIN_TOKEN=swap-test-token\n").unwrap();
    // The new build: `KEEL_TEST_NEXT_BINARY` when set (e.g. CI builds another
    // commit), otherwise a copy of this binary with a marker appended, so it
    // is a separate file with its own sha256, as a real new build would be.
    let good = dir.join("keel-next");
    match std::env::var("KEEL_TEST_NEXT_BINARY") {
        Ok(p) => {
            std::fs::copy(p, &good).unwrap();
        }
        Err(_) => {
            let mut bytes = std::fs::read(KEEL).unwrap();
            bytes.extend_from_slice(b"\nkeel-swap-test-next-build\n");
            std::fs::write(&good, bytes).unwrap();
        }
    }
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&good, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let bad = script(
        dir,
        "keel-broken",
        &format!("if [ \"$1\" = serve ]; then exec {KEEL} \"$@\" --drill-unhealthy-after 1; fi\nexec {KEEL} \"$@\""),
    );

    let _sup = Supervisor(
        Command::new(KEEL)
            .current_dir(dir)
            .args(["supervise", "--watch-secs", "4"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let base = format!("http://127.0.0.1:{port}");
    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    let mut up = false;
    for _ in 0..200 {
        if dir.join(".keel/supervisor.sock").exists() && http.get(format!("{base}/ping")).send().await.is_ok()
        {
            up = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(up, "supervisor did not start");

    let stop = Arc::new(AtomicBool::new(false));
    let (ok, failed) = (Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)));
    let mut load = Vec::new();
    for _ in 0..8 {
        let (http, base, stop, ok, failed) = (
            http.clone(),
            base.clone(),
            stop.clone(),
            ok.clone(),
            failed.clone(),
        );
        load.push(tokio::spawn(async move {
            while !stop.load(Ordering::Relaxed) {
                match http.get(format!("{base}/ping")).send().await {
                    Ok(r) if r.status().is_success() => ok.fetch_add(1, Ordering::Relaxed),
                    _ => failed.fetch_add(1, Ordering::Relaxed),
                };
            }
        }));
    }

    let d = dir.to_path_buf();
    let run = move |args: Vec<String>| {
        let d = d.clone();
        tokio::task::spawn_blocking(move || keel(&d, &args.iter().map(String::as_str).collect::<Vec<_>>()))
    };
    let (_, before) = run(vec!["status".into()]).await.unwrap();
    let first_pid = before["running"]["pid"].clone();

    // A good binary: swapped in, watched, kept.
    let (ok1, r) = run(vec![
        "update".into(),
        "--binary".into(),
        good.display().to_string(),
    ])
    .await
    .unwrap();
    assert!(ok1, "{r}");
    assert_eq!(r["previous"].as_str().map(|s| s.len()), Some(64), "{r}");
    let (_, after) = run(vec!["status".into()]).await.unwrap();
    assert_ne!(after["running"]["pid"], first_pid, "a new process serves");
    assert_ne!(
        after["running"]["sha256"], before["running"]["sha256"],
        "the new build runs: {after}"
    );
    assert_eq!(
        after["previous"]["sha256"], before["running"]["sha256"],
        "{after}"
    );

    // A live config change on the new process.
    std::fs::write(
        dir.join("app.yaml"),
        std::fs::read_to_string(dir.join("app.yaml"))
            .unwrap()
            .replace("message: one", "message: two"),
    )
    .unwrap();
    let (ok2, r) = run(vec!["apply".into()]).await.unwrap();
    assert!(ok2, "{r}");
    assert_eq!(r["mode"], "reload");
    let v: Value = http
        .get(format!("{base}/ping"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v["message"], "two");

    // A binary whose health check fails: rolled back automatically.
    let (ok3, r) = run(vec![
        "update".into(),
        "--binary".into(),
        bad.display().to_string(),
    ])
    .await
    .unwrap();
    assert!(!ok3, "{r}");
    assert_eq!(r["rolled_back"], true, "{r}");
    assert!(r["reason"].as_str().unwrap().contains("health"), "{r}");
    let failed_sha = r["failed"].as_str().unwrap().to_string();
    let (_, st) = run(vec!["status".into()]).await.unwrap();
    assert_eq!(st["rejected"]["sha256"], failed_sha.as_str(), "{st}");
    assert_ne!(
        st["previous"]["sha256"],
        failed_sha.as_str(),
        "the broken binary is not a rollback target: {st}"
    );

    tokio::time::sleep(Duration::from_millis(500)).await;
    stop.store(true, Ordering::Relaxed);
    for l in load {
        l.await.unwrap();
    }
    let (ok, failed) = (ok.load(Ordering::Relaxed), failed.load(Ordering::Relaxed));
    assert!(ok > 500, "load ran: {ok}");
    assert_eq!(failed, 0, "{failed} of {} requests failed", ok + failed);
    let log = std::fs::read_to_string(dir.join(".keel/swaps.jsonl")).unwrap();
    assert!(
        log.contains("\"action\":\"swap\"") && log.contains("\"automatic\":true"),
        "{log}"
    );
    assert_eq!(
        log.matches("\"action\":\"swap\"").count(),
        2,
        "both swaps are recorded: {log}"
    );
    let (_, audit) = run(vec!["audit".into()]).await.unwrap();
    let actions: Vec<(String, String)> = audit["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                e["action"].as_str().unwrap_or_default().to_string(),
                e["actor"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect();
    let count = |want: &str| actions.iter().filter(|(a, _)| a == want).count();
    assert_eq!(count("binary.swap"), 2, "swaps are in the audit log: {actions:?}");
    assert!(
        actions
            .iter()
            .any(|(a, who)| a == "binary.rollback" && who == "supervisor"),
        "the automatic rollback is in the audit log: {actions:?}"
    );
    assert!(
        actions
            .iter()
            .any(|(a, who)| a == "binary.swap" && who.starts_with("cli:")),
        "a swap names who asked for it: {actions:?}"
    );
}
