//! Keel plugin `reading-stats`: counts books added and finished, from the
//! `resource` plugin's events, and serves the totals at GET /stats.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use keel_sdk::capabilities::{Kv, Log};
use keel_sdk::{prelude::*, Current, Emitter};

#[derive(Default)]
pub struct ReadingStats {
    goal: Arc<AtomicU64>,
    kv: Current<dyn Kv>,
    log: Current<dyn Log>,
    emitter: Current<Emitter>,
}

impl ReadingStats {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Reads a counter from the key-value store (missing = 0).
async fn count(kv: &dyn Kv, key: &str) -> u64 {
    match kv.get(key).await {
        Ok(Some(bytes)) => String::from_utf8_lossy(&bytes).parse().unwrap_or(0),
        _ => 0,
    }
}

async fn bump(kv: &dyn Kv, key: &str) -> u64 {
    let n = count(kv, key).await + 1;
    let _ = kv.set(key, n.to_string().into_bytes()).await;
    n
}

#[async_trait]
impl Plugin for ReadingStats {
    fn manifest(&self) -> &Manifest {
        keel_sdk::manifest!()
    }

    async fn init(&self, ctx: &mut InitContext) -> Result<(), PluginError> {
        let goal = ctx.config.get("goal").and_then(Value::as_u64).unwrap_or(12);
        self.goal.store(goal, Ordering::Relaxed);

        // Keys are stored as `reading-stats:<key>`, apart from other plugins.
        let kv = ctx.capabilities.kv()?;
        self.kv.set(kv.clone());
        self.log.set(ctx.capabilities.log()?);
        self.emitter.set(Arc::new(ctx.emitter()));

        let goal = self.goal.clone();
        ctx.routes().get("/stats", move || {
            let kv = kv.clone();
            let goal = goal.load(Ordering::Relaxed);
            async move {
                let finished = count(kv.as_ref(), "finished").await;
                Json(json!({
                    "added": count(kv.as_ref(), "added").await,
                    "finished": finished,
                    "goal": goal,
                    "progress": format!("{finished}/{goal}"),
                }))
            }
        })?;
        Ok(())
    }

    async fn on_event(&self, event: &Event) {
        if event.payload["resource"] != "books" {
            return;
        }
        let Some(kv) = self.kv.get() else { return };
        if event.name == "resource.created" {
            bump(kv.as_ref(), "added").await;
        }
        let finished_now = event.payload["item"]["status"] == "done"
            && (event.name == "resource.created"
                || event.payload["changed"]
                    .as_array()
                    .is_some_and(|c| c.iter().any(|f| f == "status")));
        if finished_now {
            let n = bump(kv.as_ref(), "finished").await;
            let goal = self.goal.load(Ordering::Relaxed);
            if n == goal {
                if let Some(log) = self.log.get() {
                    log.info(&format!("reading goal of {goal} reached"));
                }
                if let Some(e) = self.emitter.get() {
                    let _ = e.emit("stats.goal.reached", json!({ "goal": goal })).await;
                }
            }
        }
    }

    async fn configure(&self, patch: Value) -> Result<(), PluginError> {
        match patch.get("goal").and_then(Value::as_u64) {
            Some(g) if g >= 1 => {
                self.goal.store(g, Ordering::Relaxed);
                Ok(())
            }
            _ => Err(PluginError::other("`goal` must be a whole number of at least 1")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_is_valid() {
        let m = ReadingStats::new().manifest().clone();
        assert_eq!(m.id, "reading-stats");
        assert!(m.declares("GET", "/stats"));
        assert!(m.validate_config(&json!({ "goal": 20 })).is_empty());
        assert!(!m.validate_config(&json!({ "goal": 0 })).is_empty());
    }
}
