# Keel

A pluggable Rust backend that AI assistants configure in YAML instead of regenerating code.

Common backend features (auth, roles, CRUD, billing, email) ship as plugins. An AI reads each plugin's short manifest, writes `app.yaml`, and only writes code for the custom business logic. See the v1 spec for the full design.

## Status

Phase 2 of 8 (plugin contract) is done:

- **Host core (phase 1):** loads and validates `app.yaml`, resolves `${secret:NAME}` from environment variables, runs an event hook bus, serves `/health`.
- **Plugin contract (phase 2):** plugins are Rust crates compiled into the binary. Each has a `plugin.yaml` manifest, a `config.schema.json`, and a type implementing the `Plugin` trait from `keel-sdk`. The host starts only the plugins `app.yaml` lists, after checking versions, dependencies, capabilities, config and route ownership.

## Try it

```sh
cargo build
export DATABASE_URL=postgres://localhost/app   # referenced by the example
./target/debug/keel check -c examples/app.yaml # validate only
./target/debug/keel serve -c examples/app.yaml # start on 127.0.0.1:8080
curl localhost:8080/health
curl localhost:8080/ping
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
| `crates/keel` | The `keel` binary (`serve`, `check`, `plugins`, `new-plugin`) |
| `plugins/` | Compiled-in plugins |
| `examples/app.yaml` | Sample config |

## Development

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```
