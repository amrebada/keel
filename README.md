# Keel

A pluggable Rust backend that AI assistants configure in YAML instead of regenerating code.

Common backend features (auth, roles, CRUD, billing, email) ship as plugins. An AI reads each plugin's short manifest, writes `app.yaml`, and only writes code for the custom business logic. See the v1 spec for the full design.

## Status

Phase 7 of 8 (live operations) is done:

- **Host core (phase 1):** loads and validates `app.yaml`, resolves `${secret:NAME}` from environment variables, runs an event hook bus, serves `/health`.
- **Plugin contract (phase 2):** plugins are Rust crates compiled into the binary. Each has a `plugin.yaml` manifest, a `config.schema.json`, and a type implementing the `Plugin` trait from `keel-sdk`. The host starts only the plugins `app.yaml` lists, after checking versions, dependencies, capabilities, config and route ownership.
- **Data providers (phase 3):** `providers:` in `app.yaml` picks SQL (sqlite, postgres, mysql), cache and key-value (memory or redis). Switching from SQLite to Postgres and Redis is a YAML change. Each plugin gets its own key prefix (`<id>:`) and table prefix (`<id>_`). `/health` reports every provider and plugin.
- **CLI (phase 3):** `init`, `dev` (reloads on `app.yaml` changes and keeps running on a broken edit), `add`, `plugins list/describe --ai`, `config validate/expand`, and `--json` on every command for AI tools.
- **Auth (phase 4):** `auth-core` (email and password, verification, password reset, magic links, JWT access tokens with rotating refresh tokens, logout everywhere), `auth-oauth` (Google, Apple, Facebook, LinkedIn, GitHub, Microsoft and X presets on one OAuth2/OIDC engine, with safe account linking) and `auth-mfa` (TOTP and recovery codes).
- **Setup tracking (phase 4):** each plugin lists the setup steps it needs (create an OAuth app, set a secret). `keel setup status` shows what is missing and how to do it, and the routes a missing step blocks answer `503 setup_incomplete` until it is done.
- **Migration safety (phase 4):** plugin migrations are checked before they run. A migration that drops, renames or deletes is blocked unless you allow it by name, and `keel migrate --apply` backs up the touched tables first.
- **Secret sources and degrade levels (phase 4):** secrets come from env vars and `.env`, or from HashiCorp Vault. Each plugin and provider has a `degrade_level` (info, warn, error); `/health` answers 503 only when an error-level item is down.
- **Roles and guards (phase 5):** the `rbac` plugin defines roles in YAML (with inheritance), per-user permission grants and roles scoped to a tenant (`X-Tenant-Id`). `routes:` in `app.yaml` puts guards (`authenticated`, `role:x`, `permission:x`, `owner:param`, `rate-limit:n/period`, `any(...)`, `all(...)`, plugin guards like `plan:pro`) and JSON Schema validation in front of any route.
- **Business plugins (phase 5):** `billing-stripe` (plans in YAML, Checkout, customer portal, signed webhooks, subscription state per tenant, `plan:x` guard). Email goes through `console`, `smtp`, `resend` or `ses`, with templates in `app.yaml`.
- **MCP (phase 5):** `keel mcp` lets an AI client search plugins, read, validate and apply `app.yaml`, follow setup steps and read logs without a shell.
- **CRUD resources (phase 6):** the `resource` plugin turns models in `app.yaml` into tables, migrations and REST endpoints with validation, filters, sorting, paging, per-owner or per-tenant rows and optional caching. Guard them with `routes:` like any other route.
- **Live operations (phase 7):** enable, disable and reconfigure plugins on a running server with no restart (`keel apply`, `keel plugins enable|disable|set`, the `/_admin` API), with every change in an audit log. `keel supervise` swaps in a new binary under load with zero dropped requests and rolls back automatically when health fails.
- **Bundles (phase 6):** a composer plugin has no code; its manifest expands into other plugins. `saas-auth` brings auth-core, auth-oauth, auth-mfa and rbac with one small config. `keel config expand` shows the result.

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

## Roles, guards and validation

```yaml
plugins:
  - use: rbac
    config:
      roles:
        admin: { permissions: ["*"] }
        owner: { permissions: ["projects.*", "billing.manage", "rbac.manage"], inherits: [member] }
        member: { permissions: ["projects.read"] }
      admins: [you@example.com]        # gets admin on signup
routes:
  /admin/**: { guard: [authenticated, "role:admin"] }
  /users/{id}/**: { guard: ["any(role:admin, owner:id)"] }
  "POST /projects":
    guard: ["permission:projects.write", "rate-limit:30/min"]
    validate: { body: { type: object, required: [name], properties: { name: { type: string, minLength: 1 } } } }
```

Every matching rule applies. Failures answer `401`, `403`, `429`, `402 plan_required` or `422 validation_failed` with a list of issues. Send `X-Tenant-Id: team-1` to use roles granted in that team. `keel config validate` reports a guard whose plugin is missing.

## CRUD resources from YAML

```yaml
plugins:
  - use: resource
    config:
      resources:
        projects:                      # table resource_projects, routes /api/projects
          fields:
            name: { type: string, required: true, max_length: 80 }
            slug: { type: string, required: true, unique: true, pattern: "^[a-z0-9-]+$" }
            status: { type: string, enum: [active, archived], default: active }
            budget: { type: number, min: 0 }
            due: { type: date }
        notes:
          scope: owner                 # rows belong to the signed-in user; notes.manage sees all
          cache_ttl: 60
          access: authenticated        # any signed-in user (default: permissions)
          fields: { body: { type: text, required: true } }
routes:                                # optional extra rules, e.g. rate limits
  "POST /api/projects": { guard: ["rate-limit:30/min"] }
```

Routes are deny by default. With no `access:` key, listing and reading need `projects.read`, create and update need `projects.write`, and delete needs `projects.delete` (rbac permissions; rename them under `permissions: { read, write, delete }`). Set `access: authenticated` for any signed-in user, or `access: public` to open a resource and rely only on `routes:` rules.

Each resource answers `GET /api/<name>` (list), `POST` (create, `201`), `GET`, `PATCH` and `DELETE /api/<name>/{id}`. `GET /api` describes every resource and its create schema.

- Dates are checked against the calendar (`2027-13-45` is a `422`), and every validation issue names the field and the rule it broke.
- Field types: `string` (up to 255), `text`, `integer`, `number`, `boolean`, `date`, `datetime`, `json`. Rules: `required`, `default`, `enum`, `min`, `max`, `min_length`, `max_length`, `pattern`, `unique` (`409 conflict` on a duplicate).
- List queries: `?status=active`, `budget.gte=100`, `.gt`, `.lt`, `.lte`, `.ne`, `name.contains=apo`, `slug.in=a,b`, `due.null=true`, `sort=-created_at,name`, `limit`, `offset`. The answer is `{ items, total, limit, offset }`.
- Scopes: `owner` (needs auth-core) and `tenant` (rows per `X-Tenant-Id`; unique values are per tenant).
- Each table and field is its own migration, so adding a field adds a column. Changing a field's rules needs no migration; to change its type, add a field with a new name.

## Bundles: saas-auth

```yaml
plugins:
  - use: saas-auth@^1
    config:
      social: [google, apple]          # leave empty to skip auth-oauth
      roles: [admin, member]           # admin gets every permission
      admins: [you@example.com]
      with: { auth-core: { password_min_length: 12 } }   # any setting of an included plugin
```

`keel config expand` prints the four plugins this becomes. Each social provider needs its OAuth app credentials as secrets named `OAUTH_<PROVIDER>_CLIENT_ID` and `OAUTH_<PROVIDER>_CLIENT_SECRET`, for example `OAUTH_GOOGLE_CLIENT_ID` and `OAUTH_GOOGLE_CLIENT_SECRET` (Apple uses `OAUTH_APPLE_CLIENT_ID`, `OAUTH_APPLE_TEAM_ID`, `OAUTH_APPLE_KEY_ID` and `OAUTH_APPLE_PRIVATE_KEY`). `keel setup status` lists them with steps. Write your own bundle with a `compose:` block in `plugin.yaml` and `keel_sdk::composer!()` as the whole crate.

## Live changes and zero-downtime updates

Set an admin token (env or `.env`), start the server, and change it while it runs:

```sh
echo "KEEL_ADMIN_TOKEN=$(openssl rand -hex 24)" >> .env
keel serve &                          # or: keel supervise (see below)
keel plugins enable ping --with '{ message: hi }'   # starts it, no restart
keel plugins set ping message=hello   # runtime_config keys change in place
keel plugins disable ping
# edit app.yaml, then reconcile the running server with it:
keel apply
keel config pull --write              # write live changes back to app.yaml
keel audit                            # who changed what, when
```

A change builds a new set of plugins next to the running one and switches requests over once it has started; in-flight requests finish on the old one. A change that fails validation, a dependency check or a migration leaves the server as it was. Providers, `server` and `secrets` need a binary swap or restart. `keel dev` uses the same mechanism, so saving app.yaml applies it live.

Admin API (bearer `KEEL_ADMIN_TOKEN`): `GET|PUT /_admin/config`, `GET|POST /_admin/plugins`, `PUT|DELETE /_admin/plugins/{id}`, `PATCH /_admin/plugins/{id}/config`, `GET /_admin/setup`, `GET /_admin/audit`, `GET /_admin/stats`.

New plugin code ships as a new binary:

```sh
keel supervise                        # owns the port, runs `keel serve` as a child
keel update --binary ./target/release/keel   # or --build
keel status                           # running and previous binary
keel rollback                         # back to the previous binary
```

`keel update` hashes the binary and keeps a copy in `.keel/bin/`, runs `config validate` and additive `migrate --apply` with it, starts it on the shared socket, waits for a healthy `/health`, then lets the old process finish its requests and exit. For 60 seconds (`--watch-secs`) a failing health check, a crash or an error rate over 50% swaps back to the previous binary. Swaps and rollbacks are logged in `.keel/swaps.jsonl`. Unix only.

## Billing with Stripe

```yaml
plugins:
  - use: billing-stripe
    config:
      plans:
        pro: { price: price_123 }
        team: { price: price_456, includes: [pro] }
routes:
  /reports/**: { guard: [authenticated, "plan:pro"] }
```

`POST /billing/checkout {"plan":"pro"}` returns a Checkout URL for the caller's tenant; the webhook at `POST /billing/webhook` records the subscription, and `plan:pro` then passes. `keel setup status` walks through the Stripe account, prices and webhook secret (`stripe listen --forward-to localhost:8080/billing/webhook` locally).

## Email

```yaml
providers:
  email:
    use: resend                       # or console, smtp, ses
    api_key: "${secret:RESEND_API_KEY}"
    from: "Shop <hello@shop.com>"
    templates:
      auth.verify: { subject: "Welcome to Shop", html: "<a href=\"{{link}}\">Confirm your email</a>" }
```

Plugins send with a template id (`auth.verify`, `auth.reset`, `auth.magic_link`); a template in `app.yaml` replaces the subject, text or html it sets. A real provider adds a `providers/email-domain` setup step for SPF and DKIM.

## Connect an AI client (MCP)

```sh
claude mcp add keel -- keel mcp -c /path/to/app.yaml
```

Tools: `search_plugins`, `describe_plugin`, `read_config`, `validate_config`, `apply_config`, `add_plugin`, `setup_status`, `setup_done`, `migrations`, `health`, `logs`. Each returns the same JSON as the CLI's `--json` output. `keel dev` and `keel serve` also log to `.keel/keel.log`, which `logs` reads.

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
| `crates/keel-core` | Host library: config loader, secrets, plugin loading, HTTP host, route guards and validation, setup tracking, migration planner, live changes and admin API |
| `crates/keel-core/schemas/app.schema.json` | JSON Schema for `app.yaml` |
| `crates/keel-providers` | SQL (sqlx), Redis, in-memory, crypto, HTTP, email (console, SMTP, Resend, SES) and Vault providers |
| `crates/keel` | The `keel` binary (`init`, `dev`, `serve`, `supervise`, `update`, `rollback`, `apply`, `audit`, `check`, `add`, `config`, `plugins`, `new-plugin`, `setup`, `migrate`, `mcp`) |
| `plugins/` | Compiled-in plugins (`saas-auth` is a bundle with no code) |
| `examples/app.yaml` | Sample config |

## Development

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
# Postgres, MySQL and Redis tests run when these are set (CI sets them):
# KEEL_TEST_POSTGRES_URL, KEEL_TEST_MYSQL_URL, KEEL_TEST_REDIS_URL
```
