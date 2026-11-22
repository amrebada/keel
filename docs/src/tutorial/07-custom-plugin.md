# 7. Write your own plugin

Plugins cover what most apps share. The part that's unique to Shelf goes in a plugin of its own. You'll write `reading-stats`: it listens to the `resource` plugin's events, counts books added and finished, and serves the totals at `GET /stats` with progress toward a reading goal.

This step uses everything a plugin can do: a manifest, a config schema, a route, a capability (key-value storage), consumed and emitted events, and a config value that changes live.

Plugins are compiled into the `keel` binary, so you work in your clone of the Keel repository.

## Scaffold it

```sh
cd ~/keel                      # your clone
keel new-plugin reading-stats
```

```text
created plugin `reading-stats` (keel-plugin-reading-stats) in /home/you/keel/plugins/reading-stats
  wrote plugins/reading-stats/Cargo.toml
  wrote plugins/reading-stats/plugin.yaml
  wrote plugins/reading-stats/config.schema.json
  wrote plugins/reading-stats/src/lib.rs
  wrote crates/keel/Cargo.toml
  wrote crates/keel/src/plugins.rs
```

The last two lines register the crate in the binary. The template is a working "hello" plugin; replace its three files.

## The manifest: plugin.yaml

The manifest is the plugin's contract. Keel enforces it: the plugin can only register the routes, emit the events and use the capabilities listed here, so the file is always an accurate description for a human or an AI.

```yaml
# plugins/reading-stats/plugin.yaml
id: reading-stats
version: 0.1.0
summary: Counts books added and finished on the shelf, and reports progress toward a reading goal.
tags: [stats, reading, example]
requires:
  host: "^0.1"
  capabilities: [kv, log]
  plugins:
    resource: "^0.1"
provides:
  routes:
    - GET /stats
  events: [stats.goal.reached]
consumes:
  events: [resource.created, resource.updated]
config: config.schema.json
runtime_config: [goal]
```

- `requires.capabilities`: the plugin gets a key-value store and a logger, nothing else. No SQL, no network, no secrets.
- `requires.plugins`: Keel refuses to start it without `resource`, and starts it after `resource`.
- `consumes.events`: Keel calls the plugin's `on_event` for these.
- `runtime_config`: `goal` can change on a running server without restarting the plugin.

## The config schema: config.schema.json

The `config:` block in `app.yaml` is checked against this JSON Schema before the plugin starts. Defaults appear in `keel config expand` and `describe`.

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "additionalProperties": false,
  "properties": {
    "goal": { "type": "integer", "minimum": 1, "default": 12, "description": "Books to finish." }
  }
}
```

## The code: src/lib.rs

```rust
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
```

The pieces, in the order Keel uses them:

1. `manifest()` returns the parsed `plugin.yaml` and `config.schema.json`, embedded at compile time by `keel_sdk::manifest!()`.
2. `init()` runs when the plugin starts, and again on a live reconfigure. `ctx.config` is the validated config with secrets resolved. `ctx.capabilities.kv()` fails if `kv` isn't in the manifest. `ctx.routes().get("/stats", …)` fails if the route isn't declared. Handlers are ordinary [axum](https://docs.rs/axum) handlers.
3. `on_event()` receives the events listed under `consumes.events`. The `resource` plugin's payload is `{resource, id, item}`, plus `changed` (the field names) on updates.
4. `configure()` handles a change to a `runtime_config` key on a running server.
5. `Current<T>` holds what `init` built; a later `init` replaces it.

The full trait is in the [SDK reference](../reference/sdk.md).

## Check, test and build

```sh
keel plugins check plugins/reading-stats
cargo test -p keel-plugin-reading-stats
cargo install --path crates/keel           # rebuild keel with the new plugin
```

```text
ok: reading-stats 0.1.0 is valid (1 route(s), 1 event(s), capabilities: kv, log)
test tests::manifest_is_valid ... ok
```

## Use it in Shelf

A new plugin is new code, so restart `keel dev` with the new binary (Ctrl-C, then `keel dev`). Then, in `shelf/`:

```sh
keel add reading-stats
curl -s localhost:8080/stats
```

```json
{"added":0,"finished":0,"goal":12,"progress":"0/12"}
```

Add a book and finish it:

```sh
ID=$(curl -s -XPOST localhost:8080/api/books -H "$ADA" \
  -d '{"title":"Circe","author":"Madeline Miller"}' | jq -r .id)
curl -s -XPATCH localhost:8080/api/books/$ID -H "$ADA" \
  -d '{"status":"done","finished_on":"2026-11-20"}' | jq '{status, finished_on}'
curl -s localhost:8080/stats
```

```json
{"added":1,"finished":1,"goal":12,"progress":"1/12"}
```

The counters live in the `kv` provider. With `kv: { use: memory }` they reset when the server restarts; with Redis (step 9) they persist.

`GET /stats` is public. To restrict it, add a rule such as `/stats: { guard: [authenticated] }` to `routes:`; no plugin code changes.

The finished plugin is also in the repository at [examples/shelf/reading-stats](https://github.com/amrebada/keel/tree/main/examples/shelf/reading-stats).

Next: [change the running server](08-live-changes.md).
