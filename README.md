# Keel

A pluggable Rust backend that AI assistants configure in YAML instead of regenerating code.

Common backend features (auth, roles, CRUD, billing, email) ship as plugins. An AI reads each plugin's short manifest, writes `app.yaml`, and only writes code for the custom business logic. See the v1 spec for the full design.

## Status

Phase 1 of 8: the host core. It loads and validates `app.yaml`, resolves `${secret:NAME}` references from environment variables, runs an event hook bus, and serves `/health`. Plugins listed in `app.yaml` are recorded but not loaded until their phases ship.

## Try it

```sh
cargo build
export DATABASE_URL=postgres://localhost/app   # referenced by the example
./target/debug/keel check -c examples/app.yaml # validate only
./target/debug/keel serve -c examples/app.yaml # start on 127.0.0.1:8080
curl localhost:8080/health
```

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
| `crates/keel-core` | Host library: config loader and schema, secrets, hook bus, HTTP host |
| `crates/keel-core/schemas/app.schema.json` | JSON Schema for `app.yaml` |
| `crates/keel` | The `keel` binary (`serve`, `check`) |
| `examples/app.yaml` | Sample config |

## Development

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```
