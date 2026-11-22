# Getting started

This page gets a Keel server running on your machine in a few minutes. The [tutorial](../tutorial/index.md) picks up from here and builds a full API.

## Requirements

- Rust 1.85 or newer (`rustup update stable`)
- `curl` to try the endpoints, and `jq` if you want to follow the tutorial's commands exactly
- Linux or macOS. Everything except `keel supervise` also works on Windows.

You don't need a database to start: new apps use a local SQLite file and in-memory cache.

## Install the CLI

Plugins are compiled into the `keel` binary, so you build it from the repository:

```sh
git clone https://github.com/amrebada/keel
cd keel
cargo install --path crates/keel
keel --version          # keel 0.1.0
```

If you'd rather not install it, `cargo build -p keel` and use `./target/debug/keel`, or put `target/debug` on your `PATH`. Keep the clone: you'll come back to it when you [write your own plugin](../guides/writing-a-plugin.md).

## Create and run an app

```sh
keel init shop          # writes shop/app.yaml, shop/.env.example, shop/.gitignore
cd shop
keel dev
```

`keel dev` serves on `127.0.0.1:8080`, loads `.env` from the app folder, and applies every save of `app.yaml` to the running server without a restart. In another terminal:

```sh
curl localhost:8080/health
curl localhost:8080/ping          # {"message":"pong"}
```

`/health` reports every provider and plugin:

```json
{
  "app": "shop",
  "status": "ok",
  "plugins": [{ "id": "ping", "status": "ok", "version": "0.1.0", "degrade_level": "error" }],
  "providers": [
    { "slot": "db", "kind": "sqlite", "capability": "sql", "status": "ok", "hot_load": false, "degrade_level": "error" },
    { "slot": "cache", "kind": "memory", "capability": "cache", "status": "ok", "hot_load": true, "degrade_level": "warn" }
  ],
  "setup": { "complete": true, "incomplete": [] },
  "uptime_secs": 0,
  "version": "0.1.0"
}
```

## The generated app.yaml

```yaml
app: shop
server:
  host: 127.0.0.1
  port: 8080

providers:
  db: { use: sqlite, url: "sqlite://shop.db" }
  cache: { use: memory }
  kv: { use: memory }
  email: { use: console, from: "shop <noreply@localhost>" }

plugins:
  - use: ping@^0.1
```

- `providers` picks the database, cache, key-value store and email sender. Plugins reach them only through Keel, so switching to Postgres and Redis later is a change here and nothing else.
- `plugins` lists what runs. `ping` is a tiny example plugin; you'll replace it.
- The console email provider prints emails to the server log, which is handy for verify links during development.

## Find and add plugins

```sh
keel plugins list                    # what is compiled into this binary
keel plugins describe auth-core --ai # its config, routes, events and setup steps, briefly
keel add auth-core                   # adds it to app.yaml with a minimal valid config
keel setup status                    # what you still have to do by hand (here: AUTH_JWT_SECRET)
```

## Next

- [Tutorial](../tutorial/index.md): build a reading-list API with accounts, roles, CRUD, a custom plugin, live changes and Postgres.
- [How Keel works](../concepts/index.md): the ideas behind the commands.
- [CLI reference](../cli/index.md): every command and flag.
