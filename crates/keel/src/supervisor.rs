//! `keel supervise`: owns the listening socket and runs `keel serve` as a
//! child that inherits it. `keel update` and `keel rollback` talk to it over
//! `.keel/supervisor.sock` to swap in a new binary with zero dropped
//! requests:
//!
//! 1. verify: hash the binary and keep a copy in `.keel/bin/`;
//! 2. check: `<new> config validate`, then `<new> migrate --apply`
//!    (additive only; a blocked destructive migration stops here);
//! 3. warm up: start the new child on the same socket and wait for its
//!    ready line and a healthy `/health` on its private control port;
//! 4. swap: SIGTERM the old child, which stops accepting and finishes its
//!    in-flight requests (the shared socket keeps accepting meanwhile);
//! 5. watch: for `--watch-secs`, a failing health check, a crash or an error
//!    rate over 50% swaps back to the previous binary.
//!
//! Every swap, rollback and restart is appended to `.keel/swaps.jsonl`.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

use crate::output::Out;

const READY_TIMEOUT: Duration = Duration::from_secs(60);
const DRAIN_TIMEOUT: Duration = Duration::from_secs(35);

fn keel_dir(config: &Path) -> PathBuf {
    keel_core::setup::SetupState::path_for(config)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(".keel"))
}

pub fn socket_path(config: &Path) -> PathBuf {
    keel_dir(config).join("supervisor.sock")
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

fn sha256_file(p: &Path) -> Result<String> {
    let bytes = std::fs::read(p).with_context(|| format!("cannot read {}", p.display()))?;
    Ok(Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// Copies a binary into `.keel/bin/keel-<sha12>` so a rollback target
/// survives rebuilds. Returns (stored path, full sha256).
fn store_binary(config: &Path, src: &Path) -> Result<(PathBuf, String)> {
    let sha = sha256_file(src)?;
    let dir = keel_dir(config).join("bin");
    std::fs::create_dir_all(&dir)?;
    let dst = dir.join(format!("keel-{}", &sha[..12]));
    if !dst.exists() {
        let tmp = dir.join(format!(".keel-{}.tmp", &sha[..12]));
        std::fs::copy(src, &tmp).with_context(|| format!("cannot copy {}", src.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
        }
        std::fs::rename(&tmp, &dst)?;
    }
    Ok((dst, sha))
}

struct Proc {
    child: Child,
    pid: u32,
    control: String,
    bin: PathBuf,
    sha: String,
}

fn signal(pid: u32, sig: i32) {
    // SAFETY: plain kill(2) on a child we started.
    unsafe {
        libc::kill(pid as i32, sig);
    }
}

/// Starts `<bin> serve` on the shared socket and waits for its ready line.
async fn spawn(bin: &Path, sha: &str, config: &Path, fd: i32) -> Result<Proc> {
    let mut cmd = Command::new(bin);
    cmd.arg("serve")
        .arg("-c")
        .arg(config)
        .env("KEEL_LISTEN_FD", "3")
        .env("KEEL_CONTROL", "127.0.0.1:0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    // SAFETY: only async-signal-safe calls between fork and exec.
    unsafe {
        cmd.pre_exec(move || {
            if fd == 3 {
                let flags = libc::fcntl(3, libc::F_GETFD);
                if flags < 0 || libc::fcntl(3, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
            } else if libc::dup2(fd, 3) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = cmd
        .spawn()
        .with_context(|| format!("cannot start {}", bin.display()))?;
    let pid = child.id().unwrap_or_default();
    let stdout = child.stdout.take().expect("piped stdout");
    let mut lines = BufReader::new(stdout).lines();
    let ready = tokio::time::timeout(READY_TIMEOUT, async {
        while let Some(line) = lines.next_line().await? {
            if let Some(rest) = line.strip_prefix("KEEL_READY ") {
                return Ok::<_, std::io::Error>(Some(
                    serde_json::from_str::<Value>(rest).unwrap_or_default(),
                ));
            }
            println!("{line}");
        }
        Ok(None)
    })
    .await;
    let info = match ready {
        Ok(Ok(Some(v))) => v,
        Ok(_) => {
            let status = child.wait().await.ok();
            bail!(
                "{} exited before it was ready ({}); see .keel/keel.log",
                bin.display(),
                status.map(|s| s.to_string()).unwrap_or_default()
            );
        }
        Err(_) => {
            let _ = child.kill().await;
            bail!(
                "{} was not ready within {}s",
                bin.display(),
                READY_TIMEOUT.as_secs()
            );
        }
    };
    tokio::spawn(async move {
        while let Ok(Some(line)) = lines.next_line().await {
            println!("{line}");
        }
    });
    Ok(Proc {
        child,
        pid,
        control: info["control"].as_str().unwrap_or_default().to_string(),
        bin: bin.to_path_buf(),
        sha: sha.to_string(),
    })
}

fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .no_proxy()
        .build()
        .expect("http client")
}

async fn health(p: &Proc) -> Result<(), String> {
    match http().get(format!("http://{}/health", p.control)).send().await {
        Ok(r) if r.status().is_success() => Ok(()),
        Ok(r) => Err(format!("/health answered {}", r.status().as_u16())),
        Err(e) => Err(format!("/health did not answer: {e}")),
    }
}

async fn stats(p: &Proc) -> Option<(u64, u64)> {
    let v: Value = http()
        .get(format!("http://{}/stats", p.control))
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;
    Some((v["requests"].as_u64()?, v["errors"].as_u64()?))
}

/// SIGTERM, then wait for the drain; SIGKILL after the timeout.
async fn retire(mut p: Proc) {
    signal(p.pid, libc::SIGTERM);
    if tokio::time::timeout(DRAIN_TIMEOUT, p.child.wait()).await.is_err() {
        let _ = p.child.kill().await;
    }
}

struct State {
    current: Option<Proc>,
    /// (binary, sha) that ran before the current one.
    previous: Option<(PathBuf, String)>,
}

struct Sup {
    config: PathBuf,
    fd: i32,
    watch: Duration,
    state: Mutex<State>,
    _listener: std::net::TcpListener,
}

impl Sup {
    fn record(&self, entry: Value) {
        let path = keel_dir(&self.config).join("swaps.jsonl");
        let mut e = entry;
        e["at"] = json!(now());
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            use std::io::Write;
            let _ = writeln!(f, "{e}");
        }
    }

    /// Runs `<bin> <args>` and fails with its output when it fails.
    async fn check(&self, bin: &Path, args: &[&str]) -> Result<()> {
        let out = Command::new(bin)
            .args(args)
            .arg("-c")
            .arg(&self.config)
            .output()
            .await
            .with_context(|| format!("cannot run {}", bin.display()))?;
        if !out.status.success() {
            bail!(
                "`{} {}` failed:\n{}{}",
                bin.display(),
                args.join(" "),
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
        }
        Ok(())
    }

    /// Starts `bin` next to the current child, then retires the current one.
    async fn switch(&self, st: &mut State, bin: &Path, sha: &str, say: &Say) -> Result<()> {
        say.step("warm-up", format!("starting {} on the shared socket", short(sha)));
        let new = spawn(bin, sha, &self.config, self.fd).await?;
        if let Err(e) = health(&new).await {
            retire(new).await;
            bail!("the new process is not healthy: {e}");
        }
        say.step("swap", format!("pid {} takes new requests", new.pid));
        if let Some(old) = st.current.replace(new) {
            say.step(
                "drain",
                format!("pid {} finishes in-flight requests and exits", old.pid),
            );
            st.previous = Some((old.bin.clone(), old.sha.clone()));
            tokio::spawn(retire(old));
        }
        Ok(())
    }

    /// Watches the current child; returns why it failed, if it did.
    async fn watch(&self, st: &mut State, say: &Say) -> Option<String> {
        if self.watch.is_zero() {
            return None;
        }
        say.step(
            "watch",
            format!("checking health and errors for {}s", self.watch.as_secs()),
        );
        let deadline = tokio::time::Instant::now() + self.watch;
        let mut bad = 0;
        let mut base = None;
        while tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let p = st.current.as_mut()?;
            if let Ok(Some(status)) = p.child.try_wait() {
                return Some(format!("the new process exited ({status})"));
            }
            match health(p).await {
                Ok(()) => bad = 0,
                Err(e) => {
                    bad += 1;
                    if bad >= 3 {
                        return Some(format!("health check failed 3 times in a row: {e}"));
                    }
                }
            }
            if let Some((req, err)) = stats(p).await {
                let (r0, e0) = *base.get_or_insert((req, err));
                let (dr, de) = (req.saturating_sub(r0), err.saturating_sub(e0));
                if dr >= 20 && de * 2 > dr {
                    return Some(format!("{de} of {dr} requests failed with 5xx"));
                }
            }
        }
        None
    }

    async fn swap(&self, src: &Path, say: &Say) -> Result<Value> {
        let mut st = self.state.lock().await;
        say.step("verify", format!("hashing {}", src.display()));
        let (bin, sha) = store_binary(&self.config, src)?;
        let from = st.current.as_ref().map(|p| p.sha.clone()).unwrap_or_default();
        say.step("verify", format!("sha256 {sha}"));
        say.step("check", "config validate with the new binary".into());
        self.check(&bin, &["config", "validate"]).await?;
        say.step("migrate", "additive migrations with the new binary".into());
        self.check(&bin, &["migrate", "--apply"]).await?;
        self.switch(&mut st, &bin, &sha, say).await?;
        if let Some(reason) = self.watch(&mut st, say).await {
            say.step("rollback", reason.clone());
            let Some((prev, prev_sha)) = st.previous.clone() else {
                bail!("{reason}; no previous binary to roll back to");
            };
            self.switch(&mut st, &prev, &prev_sha, say).await?;
            st.previous = Some((bin, sha.clone()));
            let r = json!({ "action": "rollback", "from": sha, "to": prev_sha, "reason": reason, "automatic": true });
            self.record(r.clone());
            return Ok(
                json!({ "ok": false, "rolled_back": true, "reason": reason, "running": prev_sha, "failed": sha }),
            );
        }
        let r = json!({ "action": "swap", "from": from, "to": sha });
        self.record(r);
        Ok(json!({ "ok": true, "running": sha, "previous": from }))
    }

    async fn rollback(&self, say: &Say) -> Result<Value> {
        let prev = self.state.lock().await.previous.clone();
        let Some((bin, sha)) = prev else {
            bail!("no previous binary to roll back to");
        };
        say.step("rollback", format!("back to {}", short(&sha)));
        let mut st = self.state.lock().await;
        let from = st.current.as_ref().map(|p| p.sha.clone()).unwrap_or_default();
        self.switch(&mut st, &bin, &sha, say).await?;
        self.record(json!({ "action": "rollback", "from": from, "to": sha, "automatic": false }));
        Ok(json!({ "ok": true, "running": sha, "previous": from }))
    }

    async fn status(&self) -> Value {
        match self.state.try_lock() {
            Ok(st) => json!({
                "ok": true,
                "running": st.current.as_ref().map(|p| json!({ "pid": p.pid, "sha256": p.sha, "binary": p.bin, "control": p.control })),
                "previous": st.previous.as_ref().map(|(b, s)| json!({ "sha256": s, "binary": b })),
                "watch_secs": self.watch.as_secs(),
            }),
            Err(_) => json!({ "ok": true, "busy": true }),
        }
    }
}

fn short(sha: &str) -> &str {
    &sha[..sha.len().min(12)]
}

/// Progress lines for the client, in order.
#[derive(Clone)]
struct Say(tokio::sync::mpsc::UnboundedSender<String>);

impl Say {
    fn step(&self, step: &str, message: String) {
        eprintln!("keel supervise: {step}: {message}");
        let _ = self
            .0
            .send(json!({ "step": step, "message": message }).to_string());
    }
}

async fn handle(sup: Arc<Sup>, stream: UnixStream) {
    let (r, mut w) = stream.into_split();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let writer = tokio::spawn(async move {
        while let Some(line) = rx.recv().await {
            if w.write_all(format!("{line}\n").as_bytes()).await.is_err() {
                break;
            }
        }
    });
    let say = Say(tx);
    let mut lines = BufReader::new(r).lines();
    let Ok(Some(line)) = lines.next_line().await else {
        return;
    };
    let req: Value = serde_json::from_str(&line).unwrap_or_default();
    let result = match req["cmd"].as_str() {
        Some("swap") => match req["binary"].as_str() {
            Some(b) => sup.swap(Path::new(b), &say).await,
            None => Err(anyhow!("swap needs a binary")),
        },
        Some("rollback") => sup.rollback(&say).await,
        Some("status") => Ok(sup.status().await),
        _ => Err(anyhow!("unknown command")),
    };
    let mut done = match result {
        Ok(v) => v,
        Err(e) => json!({ "ok": false, "error": format!("{e:#}") }),
    };
    done["done"] = json!(true);
    let _ = say.0.send(done.to_string());
    drop(say);
    let _ = writer.await;
}

/// `keel supervise`.
pub async fn run(config: &Path, binary: Option<PathBuf>, watch_secs: u64) -> Result<()> {
    use std::os::fd::AsRawFd;
    let config = std::fs::canonicalize(config).with_context(|| format!("{}: not found", config.display()))?;
    let app = keel_core::AppConfig::load_file(&config, &crate::KeepRefs)?;
    let addr = format!("{}:{}", app.server.host, app.server.port);
    let listener = std::net::TcpListener::bind(&addr).with_context(|| format!("cannot listen on {addr}"))?;
    let fd = listener.as_raw_fd();
    let src = match binary {
        Some(b) => b,
        None => std::env::current_exe()?,
    };
    let (bin, sha) = store_binary(&config, &src)?;
    let first = spawn(&bin, &sha, &config, fd).await?;
    eprintln!(
        "keel supervise: listening on {addr}, pid {} running {}",
        first.pid,
        short(&sha)
    );
    let sup = Arc::new(Sup {
        config: config.clone(),
        fd,
        watch: Duration::from_secs(watch_secs),
        state: Mutex::new(State {
            current: Some(first),
            previous: None,
        }),
        _listener: listener,
    });
    let sock = socket_path(&config);
    let _ = std::fs::remove_file(&sock);
    let control = UnixListener::bind(&sock).with_context(|| format!("cannot bind {}", sock.display()))?;

    // Crash recovery: restart the current binary if its process dies.
    {
        let sup = sup.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                let Ok(mut st) = sup.state.try_lock() else {
                    continue;
                };
                let exited = st
                    .current
                    .as_mut()
                    .and_then(|p| p.child.try_wait().ok().flatten());
                if let Some(status) = exited {
                    let p = st.current.take().expect("checked");
                    eprintln!("keel supervise: pid {} exited ({status}); restarting", p.pid);
                    match spawn(&p.bin, &p.sha, &sup.config, sup.fd).await {
                        Ok(n) => {
                            sup.record(
                                json!({ "action": "restart", "to": p.sha, "reason": status.to_string() }),
                            );
                            st.current = Some(n);
                        }
                        Err(e) => {
                            eprintln!("keel supervise: restart failed: {e:#}");
                            st.current = Some(p);
                        }
                    }
                }
            }
        });
    }

    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            _ = term.recv() => break,
            c = control.accept() => {
                if let Ok((stream, _)) = c {
                    tokio::spawn(handle(sup.clone(), stream));
                }
            }
        }
    }
    eprintln!("keel supervise: stopping");
    let _ = std::fs::remove_file(&sock);
    let mut st = sup.state.lock().await;
    if let Some(p) = st.current.take() {
        retire(p).await;
    }
    Ok(())
}

/// Sends one command to a running supervisor and prints its progress.
pub async fn send(out: Out, config: &Path, cmd: Value) -> Result<()> {
    let sock = socket_path(config);
    let stream = UnixStream::connect(&sock).await.with_context(|| {
        format!(
            "no supervisor at {}; start the server with `keel supervise -c {}`",
            sock.display(),
            config.display()
        )
    })?;
    let (r, mut w) = stream.into_split();
    w.write_all(format!("{cmd}\n").as_bytes()).await?;
    let mut lines = BufReader::new(r).lines();
    while let Some(line) = lines.next_line().await? {
        let v: Value = serde_json::from_str(&line).unwrap_or_default();
        if v["done"] == json!(true) {
            let ok = v["ok"] == json!(true);
            let text = if let Some(e) = v["error"].as_str() {
                format!("failed, nothing changed: {e}")
            } else if v["rolled_back"] == json!(true) {
                format!(
                    "rolled back: {}. Still running {}",
                    v["reason"].as_str().unwrap_or_default(),
                    short(v["running"].as_str().unwrap_or_default())
                )
            } else if v["busy"] == json!(true) {
                "the supervisor is busy with a swap".to_string()
            } else if v["running"].is_object() {
                let r = &v["running"];
                let mut t = format!(
                    "running pid {}, binary {} ({})",
                    r["pid"],
                    short(r["sha256"].as_str().unwrap_or_default()),
                    r["binary"].as_str().unwrap_or_default()
                );
                if let Some(p) = v["previous"]["sha256"].as_str() {
                    t.push_str(&format!(
                        "\nprevious {} (keel rollback goes back to it)",
                        short(p)
                    ));
                }
                t
            } else if v.get("running").is_some_and(|r| r.is_string()) {
                format!("ok: running {}", short(v["running"].as_str().unwrap_or_default()))
            } else {
                serde_json::to_string_pretty(&v).unwrap_or_default()
            };
            out.report(ok, text, v);
            return if ok {
                Ok(())
            } else {
                Err(crate::ops::Reported.into())
            };
        }
        if !out.json {
            println!(
                "{}: {}",
                v["step"].as_str().unwrap_or_default(),
                v["message"].as_str().unwrap_or_default()
            );
        }
    }
    bail!("the supervisor closed the connection without an answer")
}
