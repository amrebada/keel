# Writing a plugin

Write a plugin when your app needs behavior no existing plugin provides. The [tutorial's step 7](../tutorial/07-custom-plugin.md) walks through a complete one; this guide is the checklist and the details.

## 1. Scaffold

Inside your clone of the Keel repository:

```sh
keel new-plugin invoices          # or: cargo run -p keel -- new-plugin invoices
```

This creates `plugins/invoices/` (`Cargo.toml`, `plugin.yaml`, `config.schema.json`, `src/lib.rs`) and registers the crate in `crates/keel/Cargo.toml` and `crates/keel/src/plugins.rs`, between the `keel:plugins` markers. Ids are lowercase letters, digits and dashes, starting with a letter. The crate is `keel-plugin-invoices` and the type `Invoices`.

The template is a working plugin with one route, `GET /invoices/hello`.

## 2. Write the manifest

Declare everything the plugin will do. The host enforces it at runtime, so start here.

```yaml
id: invoices
version: 0.1.0
summary: Invoices per team with PDF links and a monthly total.   # one line; it's what search shows
tags: [billing, invoices, finance]
requires:
  host: "^0.1"
  capabilities: [sql, log]
  plugins:
    auth-core: "^0.1"
provides:
  routes:
    - GET /invoices
    - POST /invoices
    - GET /invoices/{id}
  events: [invoice.created]
consumes:
  events: [billing.subscription.updated]
config: config.schema.json
runtime_config: [currency]
migrations: built-in
degrade_level: warn
setup:
  - id: pdf-key
    title: Create an API key for the PDF service
    kind: external-account
    steps: ["Sign up at https://pdf.example.com", "Copy the API key"]
    produces: [secret:PDF_API_KEY]
    blocks: ["GET /invoices/{id}"]
```

Every field is described in the [plugin.yaml reference](../reference/plugin-yaml.md). Check it any time:

```sh
keel plugins check plugins/invoices
```

## 3. Write the config schema

`config.schema.json` is a JSON Schema (draft 2020-12) for the plugin's `config:` block. Give every key a `default` and a `description`: `describe --ai` shows them, and `keel config expand` fills defaults in. Use `"additionalProperties": false` so typos are caught.

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "additionalProperties": false,
  "properties": {
    "currency": { "type": "string", "pattern": "^[A-Z]{3}$", "default": "USD", "description": "ISO currency code." }
  }
}
```

A plugin without a schema accepts only an empty config.

## 4. Implement the Plugin trait

```rust
use std::sync::Arc;

use keel_sdk::auth::{bearer, IdentityProvider, IDENTITY_PROVIDER};
use keel_sdk::capabilities::Sql;
use keel_sdk::prelude::*;
use axum::http::HeaderMap;

#[derive(Default)]
pub struct Invoices;

impl Invoices {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Plugin for Invoices {
    fn manifest(&self) -> &Manifest {
        keel_sdk::manifest!()
    }

    async fn init(&self, ctx: &mut InitContext) -> Result<(), PluginError> {
        let sql: Arc<dyn Sql> = ctx.capabilities.sql()?;
        let table = ctx.capabilities.table("invoices");          // "invoices_invoices"
        let idp = ctx.interface::<dyn IdentityProvider>(IDENTITY_PROVIDER)?;

        ctx.routes().get("/invoices", move |h: HeaderMap| {
            let (sql, table, idp) = (sql.clone(), table.clone(), idp.clone());
            async move {
                let token = bearer(&h).ok_or_else(|| ApiError::unauthorized("missing_token", "sign in first"))?;
                let user = idp.verify_access_token(token).await?.sub;
                let rows = sql
                    .query(&format!("SELECT id, total FROM {table} WHERE user_id = ?"), &[json!(user)])
                    .await
                    .map_err(ApiError::internal)?;
                Ok::<_, ApiError>(Json(json!({ "items": rows })))
            }
        })?;
        // … register POST /invoices and GET /invoices/{id} the same way:
        // Keel refuses to start a plugin that declares a route it doesn't register.
        Ok(())
    }

    fn migrations(&self, _dialect: &str) -> Vec<Migration> {
        vec![Migration::new(
            "001_init",
            "CREATE TABLE {ns}invoices (
                id VARCHAR(36) NOT NULL PRIMARY KEY,
                user_id VARCHAR(36) NOT NULL,
                total BIGINT NOT NULL,
                created_at BIGINT NOT NULL
            )",
        )]
    }
}
```

The template's `Cargo.toml` depends only on `keel-sdk`, whose prelude re-exports `Json`, `extract` and `response` from axum. To use other axum types such as `HeaderMap`, add `axum.workspace = true` under `[dependencies]`.

What `InitContext` gives you:

| Item | Use |
| --- | --- |
| `ctx.config` | The validated `config:` (a `serde_json::Value`), secrets resolved. Defaults aren't filled in; read with a fallback. |
| `ctx.capabilities.sql()`, `.kv()`, `.cache()`, `.http_client()`, `.secrets()`, `.crypto()`, `.log()`, `.email()` | The capabilities you declared. |
| `ctx.capabilities.table("x")` | Your prefixed table name. Use it in every query. |
| `ctx.routes().get/post/put/patch/delete(path, handler)` | Register a declared route. Handlers are [axum](https://docs.rs/axum/0.8) handlers; the path syntax is axum's (`/invoices/{id}`). |
| `ctx.emitter()` | An `Emitter` for your declared events. Cheap to clone into handlers. |
| `ctx.provide(name, Arc<T>)` | Offer a declared interface to other plugins. |
| `ctx.interface::<T>(name)` | Get an interface from a plugin you require. |
| `ctx.interfaces()` | The registry, for optional interfaces looked up per request. |

Return errors as `ApiError` so every route answers `{"error": {"code", "message"}}`: `ApiError::bad_request(code, msg)`, `unauthorized`, `forbidden`, `not_found`, `conflict`, `internal(cause)` (logs the cause, hides it from the client), or `ApiError::new(status, code, msg).with("field", value)`.

The full API is in the [SDK reference](../reference/sdk.md).

## 5. Optional hooks

- `on_event(&self, event)` for `consumes.events`.
- `configure(&self, patch)` to apply `runtime_config` keys in place; return `PluginError::ConfigureUnsupported` (the default) to have Keel restart the plugin instead.
- `health(&self)` returning `Health::Ok`, `Degraded { reason }` or `Down { reason }`.
- `check_setup(&self, step_id)` for steps with `verify: { type: plugin-check }`.
- `drain`, `export_state`, `import_state` for in-flight work and in-memory state across swaps.
- `migrations_for(&self, dialect, config)` when migrations depend on config.

`init` can run more than once (live reconfigure), so keep what it builds in a `keel_sdk::Current<T>` rather than a `OnceLock`.

## 6. Test

The template includes a manifest test. Add your own; `keel_sdk` types work without a server:

```rust
#[test]
fn manifest_is_valid() {
    let m = Invoices::new().manifest().clone();
    assert!(m.declares("GET", "/invoices"));
    assert!(m.validate_config(&json!({ "currency": "EUR" })).is_empty());
    assert!(!m.validate_config(&json!({ "currency": "euro" })).is_empty());
}
```

For HTTP-level tests, look at `crates/keel-core/tests/server.rs` and the plugins' own tests, which start a host in-process with SQLite in memory.

```sh
cargo test -p keel-plugin-invoices
cargo clippy -p keel-plugin-invoices --all-targets -- -D warnings
```

## 7. Use it

```sh
cargo install --path crates/keel      # or keel update --build for a supervised server
keel add invoices
```

## Checklist

- [ ] `summary` is one clear line and `tags` cover the words people will search for.
- [ ] Every config key has a default and a description.
- [ ] Every route, event, interface and capability the code uses is declared, and every declared route is registered (Keel refuses to start a plugin that declares a route it doesn't register).
- [ ] SQL uses `ctx.capabilities.table()` / `{ns}` and `?` placeholders.
- [ ] Anything a human must do is a `setup` step, with `blocks` narrowed to the routes that need it.
- [ ] Errors use stable snake_case codes.
- [ ] `keel plugins check` passes, and `keel plugins describe <id> --ai` reads well.
