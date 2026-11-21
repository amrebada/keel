# Keel

A pluggable Rust backend that AI assistants configure in YAML instead of regenerating code.

Common backend features (auth, roles, CRUD, billing, email) ship as plugins. An AI reads each plugin's short manifest, writes `app.yaml`, and only writes code for the custom business logic. See the v1 spec for the full design.

## Status

Phase 3 of 8 (data providers and CLI) is done:

- **Host core (phase 1):** loads and validates `app.yaml`, resolves `${secret:NAME}` from environment variables, runs an event hook bus, serves `/health`.
- **Plugin contract (phase 2):** plugins are Rust crates compiled into the binary. Each has a `plugin.yaml` manifest, a `config.schema.json`, and a type implementing the `Plugin` trait from `keel-sdk`. The host starts only the plugins `app.yaml` lists, after checking versions, dependencies, capabilities, config and route ownership.
- **Data providers (phase 3):** `providers:` in `app.yaml` picks SQL (sqlite, postgres, mysql), cache and key-value (memory or redis). Switching from SQLite to Postgres and Redis is a YAML change. Each plugin gets its own key prefix (`<id>:`) and table prefix (`<id>_`). `/health` reports every provider.
- **CLI (phase 3):** `init`, `dev` (reloads on `app.yaml` changes and keeps running on a broken edit), `add`, `plugins list/describe --ai`, `config validate/expand`, and `--json` on every command for AI tools.

## Try it

```sh
cargo install --path crates/keel   # or use ./target/debug/keel after cargo build
keel init shop && cd shop          # app.yaml on SQLite + in-memory cache/kv
keel dev                           # serves 127.0.0.1:8080, reloads on edits
curl localhost:8080/health
curl localhost:8080/ping
```

Move to Postgres and Redis by editing `providers:` (the commented lines in the generated `app.yaml`):

```yaml
providers:
  db:    { use: postgres, url: "${secret:DATABASE_URL}" }
  cache: { use: redis, url: "${secret:REDIS_URL}" }
  kv:    { use: redis, url: "${secret:REDIS_URL}", prefix: "kv:" }
```

Commands an AI uses to stay cheap on tokens:

```sh
keel plugins list --json        # one line per plugin
keel plugins describe ping --ai # config, routes, events, setup steps in a few lines
keel add ping                   # adds the entry with its minimal config, keeps comments
keel config validate            # every problem with its path
keel config expand              # app.yaml with defaults filled in, secrets left as refs
```

## Write a plugin

```sh
cargo run -p keel -- new-plugin hello     # creates plugins/hello and compiles it in
# add `- use: hello` under plugins: in app.yaml, then
cargo run -p keel -- serve -c app.yaml
curl localhost:8080/hello/hello
```

A plugin is three files:

| File | Purpose |
| --- | --- |
| `plugin.yaml` | Manifest: id, version, summary, required capabilities and plugins, routes, events. The only file an AI needs to read to use the plugin. |
| `config.schema.json` | JSON Schema for the plugin's `config:` block in `app.yaml`. |
| `src/lib.rs` | A type implementing `keel_sdk::Plugin`. Routes it registers must be declared in `plugin.yaml`. |

`keel plugins check plugins/hello` validates a manifest, and `keel plugins list` shows what is compiled in. `plugins/ping` is a complete small example.

A broken config lists every problem with its path:

```text
error: app.yaml: 3 problem(s) found
  - app: "Team Tasks" does not match "^[a-z0-9][a-z0-9-]*$"
  - plugins[0]: "use" is a required property
  - server: Additional properties are not allowed ('prot' was unexpected)
```

## Layout

| Path | What it is |
| --- | --- |
| `crates/keel-sdk` | What plugins depend on: `Plugin` trait, manifest format, capability traits, hook bus |
| `crates/keel-sdk/schemas/plugin.schema.json` | JSON Schema for `plugin.yaml` |
| `crates/keel-core` | Host library: config loader, secrets, plugin loading, HTTP host |
| `crates/keel-core/schemas/app.schema.json` | JSON Schema for `app.yaml` |
| `crates/keel-providers` | SQL (sqlx), Redis and in-memory providers |
| `crates/keel` | The `keel` binary (`init`, `dev`, `serve`, `check`, `add`, `config`, `plugins`, `new-plugin`) |
| `plugins/` | Compiled-in plugins |
| `examples/app.yaml` | Sample config |

## Development

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
# Postgres, MySQL and Redis tests run when these are set (CI sets them):
# KEEL_TEST_POSTGRES_URL, KEEL_TEST_MYSQL_URL, KEEL_TEST_REDIS_URL
```
