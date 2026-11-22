# app.yaml

One file describes the whole backend. It has six top-level keys; only `app` is required.

```yaml
app: shelf                       # name: lowercase letters, digits, dashes

server:                          # where to listen
  host: 127.0.0.1                # default 0.0.0.0
  port: 8080                     # default 8080

providers:                       # what plugins use for data and email
  db: { use: sqlite, url: "sqlite://shelf.db" }
  cache: { use: memory }
  kv: { use: memory }
  email: { use: console, from: "Shelf <noreply@localhost>" }

secrets: { use: env }            # where ${secret:NAME} comes from (env is the default)

plugins:                         # what runs, in any order (Keel sorts by dependency)
  - use: auth-core@^0.1          # id, optionally with a semver range
    config: { app_url: "https://shelf.app" }
    degrade_level: error         # optional: how much it matters to /health

routes:                          # guards and validation in front of any route
  "POST /api/books": { guard: [authenticated, "rate-limit:30/min"] }
```

## Sections

| Key | What it does | Details |
| --- | --- | --- |
| `app` | The app's name, used in logs and `/health`. Must match `^[a-z0-9][a-z0-9-]*$`. | |
| `server` | Listen address. | |
| `providers` | The database (`db`), `cache`, key-value store (`kv`) and `email` sender. | [Providers](providers.md) |
| `secrets` | The secret source: `env` (environment plus `.env`) or `vault`. | [Secrets](secrets.md) |
| `plugins` | The plugins to run, each with `use`, optional `config` and `degrade_level`. | [Plugins](plugins.md), [catalog](../plugins/index.md) |
| `routes` | Route rules: `guard` and `validate` per route pattern. | [Routes and guards](routes-and-guards.md) |

## Plugin entries

```yaml
plugins:
  - use: rbac                # any version
  - use: auth-core@^0.1      # must match ^0.1
  - use: resource
    config:                  # checked against the plugin's config.schema.json
      resources: { … }
```

- `use` is the plugin id, optionally `@` and a semver range. A version that doesn't match the compiled-in plugin is an error.
- `config` is validated against the plugin's JSON Schema before anything starts. Missing keys take the schema's defaults; `keel config expand` shows the result.
- A plugin appears at most once. Order doesn't matter: plugins start after the plugins they require.

## Secrets in config

Write `${secret:NAME}` anywhere a string goes, either as the whole value or inside a longer string:

```yaml
providers:
  db: { use: postgres, url: "${secret:DATABASE_URL}" }
```

The file is validated before secrets are resolved, so error messages never contain secret values. A missing secret is reported with every path that uses it.

## Checking and inspecting

```sh
keel config validate       # or `keel check`: every problem, with its path
keel config expand         # bundles expanded, defaults filled in, secrets left as references
keel config pull           # the running server's config, with live changes
```

A broken file lists every problem at once:

```text
error: app.yaml: 3 problem(s) found
  - app: "Team Tasks" does not match "^[a-z0-9][a-z0-9-]*$"
  - plugins[0]: "use" is a required property
  - server: Additional properties are not allowed ('prot' was unexpected)
```

## Files beside app.yaml

| Path | Written by | Contents |
| --- | --- | --- |
| `.env` | you | Secrets as `NAME=value`, loaded by `serve`, `dev`, `check`, `config`, `migrate`, `setup` and `supervise`. Real environment variables win. |
| `.keel/keel.log` | `serve`, `dev`, `supervise` | The server log (also read by the MCP `logs` tool). |
| `.keel/setup.json` | `keel setup done` | Manual setup steps marked done. |
| `.keel/app.yaml.bak` | MCP `apply_config` | The previous `app.yaml`. |
| `.keel/bin/`, `.keel/swaps.jsonl` | `supervise`, `update` | Stored binaries and the swap log. |

The full schema is in the [app.yaml reference](../reference/app-yaml.md).
