# 9. Postgres, Redis and a zero-downtime update

SQLite and in-memory stores are good for development. In production you want Postgres for data and Redis for the cache (rate limits, resource caching) and key-value store (the reading-stats counters). In Keel that's a provider change; no plugin changes.

## Start Postgres and Redis

With Docker:

```sh
docker run -d --name shelf-pg -e POSTGRES_PASSWORD=shelf -e POSTGRES_DB=shelf -p 5432:5432 postgres:16
docker run -d --name shelf-redis -p 6379:6379 redis:7
```

## Switch providers

Stop `keel dev`. In `app.yaml`, replace the three development providers (the generated file has these lines as comments):

```yaml
providers:
  db: { use: postgres, url: "${secret:DATABASE_URL}" }
  cache: { use: redis, url: "${secret:REDIS_URL}" }
  kv: { use: redis, url: "${secret:REDIS_URL}", prefix: "kv:" }
  email: { use: console, from: "shelf <noreply@localhost>" }
```

Connection strings are secrets, so they're `${secret:NAME}` references. Validation tells you when one is missing:

```sh
keel config validate
```

```text
error: app.yaml: 3 secret(s) not set
  - REDIS_URL (used at providers.cache.url)
  - DATABASE_URL (used at providers.db.url)
  - REDIS_URL (used at providers.kv.url)
  Set them in the environment or .env (e.g. NAME=value), or in the secret store under `secrets:`
```

```sh
echo "DATABASE_URL=postgres://postgres:shelf@127.0.0.1:5432/shelf" >> .env
echo "REDIS_URL=redis://127.0.0.1:6379" >> .env
keel config validate     # ok: app.yaml is valid (app 'shelf', 4 provider(s), 5 plugin(s), 3 route rule(s))
```

## Plan the migrations

`keel migrate` is a dry run. It lists every pending migration and whether it's additive or destructive:

```sh
keel migrate
```

```text
migrations: 15 pending
  additive    auth-core/001_init  tables: auth_core_sessions, auth_core_tokens, auth_core_users
  additive    auth-core/002_refresh_reuse  tables: auth_core_used_refresh
  additive    rbac/001_init  tables: rbac_assignments, rbac_grants
  additive    resource/books_00_table  tables: resource_books
  additive    resource/books_10_author  tables: resource_books
  …
  additive    resource/notes_10_book_id  tables: resource_notes
dry run: nothing changed. Run with --apply to run the additive ones
```

```sh
keel migrate --apply
```

Every plugin's tables carry its prefix (`auth_core_`, `rbac_`, `resource_`), so plugins never collide. `keel serve` also runs additive migrations at start; a destructive one (a drop, a rename, a delete) is never run unless you allow it by name with `--allow-destructive <plugin>/<id>`, and the touched tables are backed up first. See [Migrations](../concepts/migrations.md).

The new database is empty, so sign Ada up again when you test. `admins:` makes her an admin as soon as she signs up.

## Run under the supervisor

`keel serve` runs the server. `keel supervise` runs it as a child process that owns the port, so you can swap the binary later without dropping a request:

```sh
keel supervise
```

```sh
curl -s localhost:8080/health | jq -c '{status, providers: [.providers[] | "\(.slot)=\(.kind):\(.status)"]}'
```

```json
{"status":"ok","providers":["cache=redis:ok","db=postgres:ok","email=console:ok","kv=redis:ok"]}
```

```sh
keel status
```

```text
running pid 8066, binary 73b371dbb4be (…/shelf/.keel/bin/keel-73b371dbb4be)
no previous binary to roll back to
```

## Ship new code with zero downtime

Say you changed reading-stats and rebuilt Keel. Swap the running server to the new binary:

```sh
keel update --binary ~/keel/target/release/keel
```

Or let Keel build it: from your Keel clone, `keel update --build -c ~/shelf/app.yaml` runs `cargo build --release --bin keel` and uses the result.

```text
verify: sha256 73b371dbb4be53de…
check: config validate with the new binary
migrate: additive migrations with the new binary
warm-up: starting 73b371dbb4be on the shared socket
swap: pid 8101 takes new requests
drain: pid 8066 finishes in-flight requests and exits
watch: checking health and errors for 60s
ok: running 73b371dbb4be
```

For 60 seconds after the swap (`--watch-secs` on `keel supervise`), a failing `/health`, a crash or an error rate over 50% swaps back to the previous binary on its own. You can also go back by hand with `keel rollback`. Swaps and rollbacks are logged in `.keel/swaps.jsonl`. This is Unix only.

## The finished app.yaml

```yaml
app: shelf
server:
  host: 127.0.0.1
  port: 8080

providers:
  db: { use: postgres, url: "${secret:DATABASE_URL}" }
  cache: { use: redis, url: "${secret:REDIS_URL}" }
  kv: { use: redis, url: "${secret:REDIS_URL}", prefix: "kv:" }
  email: { use: console, from: "shelf <noreply@localhost>" }

plugins:
  - use: ping@^0.1
  - use: auth-core@^0.1
  - use: rbac
    config:
      roles:
        admin: { permissions: ["*"] }
        librarian: { permissions: ["books.*"], inherits: [reader] }
        reader: { permissions: ["books.read"] }
      default_role: reader
      admins: [ada@example.com]
  - use: resource
    config:
      resources:
        books:
          fields:
            title: { type: string, required: true, max_length: 200 }
            author: { type: string, required: true }
            isbn: { type: string, unique: true, pattern: "^[0-9-]{10,17}$" }
            status: { type: string, enum: [want, reading, done], default: want }
            pages: { type: integer, min: 1 }
            finished_on: { type: date }
        notes:
          scope: owner
          access: authenticated
          fields:
            book_id: { type: string, required: true }
            body: { type: text, required: true }
  - use: reading-stats@^0.1

routes:
  "POST /api/books":
    guard: ["rate-limit:3/min"]
  "DELETE /api/books/{id}":
    guard: ["role:admin"]
  "GET /api/books":
    validate:
      query: { type: object, properties: { limit: { type: integer, maximum: 50 } } }
```

It's also in the repository as [examples/shelf/app.yaml](https://github.com/amrebada/keel/blob/main/examples/shelf/app.yaml). Before a real launch you'd also swap the console email provider for SMTP, Resend or SES, and raise the rate limit. [Going to production](../guides/production.md) has the checklist.

Next: [let an AI drive it with MCP](10-mcp.md).
