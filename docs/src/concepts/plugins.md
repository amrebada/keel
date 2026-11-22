# Plugins

A plugin is a Rust crate compiled into the `keel` binary. It has three parts:

| File | Purpose |
| --- | --- |
| `plugin.yaml` | The manifest: id, version, summary, what it requires, what it provides, its setup steps. The only file a person or an AI needs to read to use the plugin. |
| `config.schema.json` | JSON Schema for the plugin's `config:` block in `app.yaml`, with defaults and descriptions. |
| `src/lib.rs` | A type implementing the `keel_sdk::Plugin` trait. |

A plugin compiled into the binary does nothing until `app.yaml` lists it. `keel plugins list` shows what's compiled in.

## Compiled in, not loaded at runtime

Keel doesn't load plugins from shared libraries or WebAssembly. Plugins are ordinary crates, linked into one binary, which keeps them fast, type-checked against the SDK, and simple to deploy. The cost is that adding new plugin code means building a new binary. Keel makes that cheap at runtime: [enable, disable and reconfigure](live-operations.md) need no restart, and a new binary is swapped in under load with automatic rollback.

## What the manifest declares

```yaml
id: reading-stats
version: 0.1.0
summary: Counts books added and finished, and reports progress toward a goal.
tags: [stats, reading]
requires:
  host: "^0.1"                     # host versions it works with
  capabilities: [kv, log]          # what it may use
  plugins: { resource: "^0.1" }    # what must run first
provides:
  routes: [GET /stats]             # the only routes it may register
  events: [stats.goal.reached]     # the only events it may emit
  interfaces: []                   # typed services it offers other plugins
consumes:
  events: [resource.created]       # delivered to on_event
config: config.schema.json
runtime_config: [goal]             # keys that change live via configure()
migrations: built-in               # it ships SQL migrations
setup: []                          # human steps it needs
degrade_level: error               # default weight in /health
```

The host checks the manifest at build time (`keel plugins check`), at start, and on every call:

- registering a route not in `provides.routes` fails with `UndeclaredRoute`;
- emitting an event not in `provides.events` fails with `UndeclaredEvent`;
- asking for a capability not in `requires.capabilities` fails with `NotDeclared`;
- offering an interface not in `provides.interfaces` fails with `UndeclaredInterface`.

Routes under `/health` and `/_admin` are reserved for the host. Every field is described in the [plugin.yaml reference](../reference/plugin-yaml.md).

## Lifecycle

```text
manifest → init → (requests, on_event) → configure → drain → (export_state → import_state)
```

| Method | When it's called |
| --- | --- |
| `manifest()` | Always. Returns the parsed `plugin.yaml`. |
| `init(ctx)` | On start, on enable, and when a live change rebuilds the plugin. Register routes, read config, grab capabilities. |
| `on_event(event)` | For each event listed in `consumes.events`. |
| `configure(patch)` | A live change that touches only `runtime_config` keys. |
| `health()` | For `/health`: `Ok`, `Degraded { reason }` or `Down { reason }`. |
| `drain()` | Before the plugin is removed or the process swaps out. |
| `export_state()` / `import_state()` | Carry in-memory state across a swap. |
| `migrations(dialect)` / `migrations_for(dialect, config)` | SQL migrations for `postgres`, `mysql` or `sqlite`. |
| `check_setup(step_id)` | Runs a setup step with `verify: { type: plugin-check }`. |

Only `manifest` and `init` are required.

## Versions and dependencies

- `use: rbac@^0.1` in `app.yaml` must match the compiled-in plugin's version.
- `requires.host` must match the host version (`keel --version`).
- `requires.plugins` must all be listed in `app.yaml` with matching versions; they start first. A cycle is an error.
- Two plugins may not declare the same route.

All of these are checked before anything starts, and reported together with their `app.yaml` paths.

## Isolation

Plugins share one process but not their data:

- SQL tables carry the plugin's prefix (`auth-core` → `auth_core_users`). Migrations may only touch tables in their own prefix.
- Cache and key-value keys are prefixed automatically (`auth-core:session:1`).
- A plugin gets only the capabilities it declared.
- Plugins reach each other only through [events and interfaces](events-and-interfaces.md).

## Kinds of plugins

- **Feature plugins** such as `auth-core` and `resource` have code, routes and usually tables.
- **Bundles** (composers) such as `saas-auth` have no code. They expand into other plugins with config derived from their own. See [Bundles](bundles.md).

To write one, see [Writing a plugin](../guides/writing-a-plugin.md).
