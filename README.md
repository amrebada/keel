# Keel

A pluggable Rust backend that AI assistants configure in YAML instead of regenerating code.

Common backend features (auth, roles, CRUD, billing, email) ship as plugins. An AI reads each plugin's short manifest, writes `app.yaml`, and only writes code for the custom business logic. See the v1 spec for the full design.

## Status

Phase 4 of 8 (auth, setup tracking, migration safety) is done:

- **Host core (phase 1):** loads and validates `app.yaml`, resolves `${secret:NAME}` from environment variables, runs an event hook bus, serves `/health`.
- **Plugin contract (phase 2):** plugins are Rust crates compiled into the binary. Each has a `plugin.yaml` manifest, a `config.schema.json`, and a type implementing the `Plugin` trait from `keel-sdk`. The host starts only the plugins `app.yaml` lists, after checking versions, dependencies, capabilities, config and route ownership.
- **Data providers (phase 3):** `providers:` in `app.yaml` picks SQL (sqlite, postgres, mysql), cache and key-value (memory or redis). Switching from SQLite to Postgres and Redis is a YAML change. Each plugin gets its own key prefix (`<id>:`) and table prefix (`<id>_`). `/health` reports every provider and plugin.
- **CLI (phase 3):** `init`, `dev` (reloads on `app.yaml` changes and keeps running on a broken edit), `add`, `plugins list/describe --ai`, `config validate/expand`, and `--json` on every command for AI tools.
- **Auth (phase 4):** `auth-core` (email and password, verification, password reset, magic links, JWT access tokens with rotating refresh tokens, logout everywhere), `auth-oauth` (Google, Apple, Facebook, LinkedIn, GitHub, Microsoft and X presets on one OAuth2/OIDC engine, with safe account linking) and `auth-mfa` (TOTP and recovery codes).
- **Setup tracking (phase 4):** each plugin lists the setup steps it needs (create an OAuth app, set a secret). `keel setup status` shows what is missing and how to do it, and the routes a missing step blocks answer `503 setup_incomplete` until it is done.
- **Migration safety (phase 4):** plugin migrations are checked before they run. A migration that drops, renames or deletes is blocked unless you allow it by name, and `keel migrate --apply` backs up the touched tables first.
- **Secret sources and degrade levels (phase 4):** secrets come from env vars and `.env`, or from HashiCorp Vault. Each plugin and provider has a `degrade_level` (info, warn, error); `/health` answers 503 only when an error-level item is down.

## Try it

```sh
cargo install --path crates/keel   # or use ./target/debug/keel after cargo build
keel init shop && cd shop          # app.yaml on SQLite + in-memory cache/kv
keel dev                           # serves 127.0.0.1:8080, reloads on edits, loads .env
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

## Add auth

```yaml
providers:
  email: { use: console }          # prints emails (verify links, codes) to the log
plugins:
  - use: auth-core
    config: { app_url: "http://localhost:3000" }
  - use: auth-oauth
    config:
      redirect_base_url: "http://localhost:8080"
      providers: { google: {} }
  - use: auth-mfa
```

```sh
keel setup status                 # lists AUTH_JWT_SECRET, OAUTH_GOOGLE_CLIENT_ID, ... with steps
echo "AUTH_JWT_SECRET=$(openssl rand -hex 32)" >> .env
curl -XPOST localhost:8080/auth/signup -d '{"email":"a@b.co","password":"correct horse battery"}'
curl -XPOST localhost:8080/auth/login  -d '{"email":"a@b.co","password":"correct horse battery"}'
curl localhost:8080/auth/oauth/google/start   # 503 setup_incomplete until the Google secrets are set
```

Manual steps (such as "add the redirect URL in the Google console") are marked with `keel setup done auth-oauth/<step>`, and the state is kept in `.keel/setup.json`.

## Migrations

`keel migrate` prints the plan without changing anything: each pending migration, whether it is additive or destructive, and how many rows it would touch. `keel migrate --apply` runs it. A destructive migration only runs with `--allow-destructive <plugin>/<id>`, and the tables it touches are copied to `keel_backup_<time>_<table>` first. `--apply` exits non-zero while a destructive migration is still blocked. `keel serve` and `keel dev` run additive migrations at start and refuse to start on a blocked one.

## Secrets from Vault

```yaml
secrets: { use: vault, url: "https://vault:8200", mount: secret, path: shop }
```

Keel reads the KV v2 secret at `secret/data/shop` with the token in `VAULT_TOKEN`. Env vars and `.env` still win, so a local override works.

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
| `crates/keel-core` | Host library: config loader, secrets, plugin loading, HTTP host, setup tracking, migration planner |
| `crates/keel-core/schemas/app.schema.json` | JSON Schema for `app.yaml` |
| `crates/keel-providers` | SQL (sqlx), Redis, in-memory, crypto, HTTP, console email and Vault providers |
| `crates/keel` | The `keel` binary (`init`, `dev`, `serve`, `check`, `add`, `config`, `plugins`, `new-plugin`, `setup`, `migrate`) |
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
