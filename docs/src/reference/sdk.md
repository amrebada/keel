# Plugin SDK (Rust API)

Plugins depend only on `keel-sdk`. Generate the full API docs locally with:

```sh
cargo doc -p keel-sdk --open
```

This page is the map.

## Prelude

```rust
use keel_sdk::prelude::*;
// async_trait, ApiError, Event, Health, InitContext, Manifest, Migration,
// Plugin, PluginError, axum::{extract, response, Json}, serde_json::{json, Value}
```

## Modules

| Module | What's in it |
| --- | --- |
| `keel_sdk::plugin` | `Plugin` trait, `InitContext`, `Routes`, `Emitter`, `Health`, `PluginError`, `Current<T>` |
| `keel_sdk::manifest` | `Manifest` (parsed `plugin.yaml`), `Route`, `ComposeEntry`, `ManifestError`; `manifest!()` macro at the crate root |
| `keel_sdk::capabilities` | `Capability`, `Capabilities`, and the traits `Sql`, `Kv`, `Cache`, `HttpClient`, `Secrets`, `Crypto`, `Log`, `Email`; `EmailMessage`, `HttpRequest`, `HttpResponse`, `DegradeLevel` |
| `keel_sdk::hooks` | `Event`, `HookBus` |
| `keel_sdk::interfaces` | `Interfaces`, the shared registry |
| `keel_sdk::auth` | `IdentityProvider`, `SecondFactor`, `AuthUser`, `Tokens`, `LoginOutcome`, `Claims`, `bearer(&HeaderMap)`, `IDENTITY_PROVIDER` |
| `keel_sdk::authz` | `Authorizer`, `GuardCheck`, `GuardContext`, `Guard`, `guard_interface(name)`, `AUTHORIZER`, `TENANT_HEADER` |
| `keel_sdk::http` | `ApiError` |
| `keel_sdk::migrations` | `Migration` |
| `keel_sdk::setup` | `SetupStep`, `SetupKind`, `Verify`, `VerifyKind`, `path_matches` |
| `keel_sdk::composer` | `Composer`; `composer!()` macro |
| `keel_sdk::issues` | `Issue` (a path and a message) |

`keel_sdk::HOST_VERSION` is the host version plugins are checked against.

## The Plugin trait

```rust
#[async_trait]
pub trait Plugin: Send + Sync + 'static {
    fn manifest(&self) -> &Manifest;
    async fn init(&self, ctx: &mut InitContext) -> Result<(), PluginError>;

    async fn configure(&self, _patch: Value) -> Result<(), PluginError> { Err(PluginError::ConfigureUnsupported) }
    async fn on_event(&self, _event: &Event) {}
    async fn health(&self) -> Health { Health::Ok }
    async fn drain(&self) {}
    fn export_state(&self) -> Option<Value> { None }
    fn import_state(&self, _state: Value) {}
    fn migrations(&self, _dialect: &str) -> Vec<Migration> { Vec::new() }
    fn migrations_for(&self, dialect: &str, _config: &Value) -> Vec<Migration> { self.migrations(dialect) }
    async fn check_setup(&self, _step_id: &str) -> Result<(), String> { Ok(()) }
}
```

## Capabilities

| Trait | Methods |
| --- | --- |
| `Sql` | `dialect()`, `execute(sql, params) -> rows affected`, `query(sql, params) -> Vec<Map<String, Value>>`, `transaction(statements)`, `ping()`. Write `?` placeholders; params are JSON values. |
| `Kv` | `get(key) -> Option<Vec<u8>>`, `set(key, bytes)`, `delete(key) -> bool` |
| `Cache` | `get`, `set(key, bytes, ttl)`, `delete`, `incr(key, ttl) -> i64` (atomic on Redis) |
| `HttpClient` | `send(HttpRequest { method, url, headers, body }) -> HttpResponse { status, headers, body }` |
| `Secrets` | `get(name) -> Option<String>` |
| `Crypto` | `random_bytes(len)`, `hash_password(pw)` (Argon2id), `verify_password(pw, hash)` |
| `Log` | `info`, `warn`, `error` (tagged with the plugin id) |
| `Email` | `send(EmailMessage { to, subject, text, html, template, vars })` |

Get them from `ctx.capabilities` in `init`: `sql()`, `kv()`, `cache()`, `http_client()`, `secrets()`, `crypto()`, `log()`, `email()`. Each returns `Err(CapabilityError::NotDeclared)` when the manifest doesn't list it. `ctx.capabilities.table("x")` gives the prefixed table name; key-value and cache keys are prefixed for you.

## Errors

`PluginError` (from `init`, `configure`): `UndeclaredRoute`, `UndeclaredEvent`, `UndeclaredInterface`, `MissingInterface`, `Capability`, `ConfigureUnsupported`, `Other` (`PluginError::other("…")`).

`ApiError` (from handlers) renders `{"error": {"code", "message", …details}}`:

```rust
ApiError::bad_request("invalid_currency", "use a 3-letter code")
ApiError::unauthorized("missing_token", "sign in first")
ApiError::forbidden("forbidden", "only owners can do this")
ApiError::not_found("not_found", "no such invoice")
ApiError::conflict("duplicate", "already exists")
ApiError::internal(err)          // 500; logs `err`, hides it from the client
ApiError::new(StatusCode::PAYMENT_REQUIRED, "plan_required", "…").with("plan", "pro")
```

## Macros

- `keel_sdk::manifest!()` embeds `plugin.yaml` and `config.schema.json` from the crate root at compile time and parses them once. It panics on an invalid manifest, which `keel plugins check` and the plugin's tests catch first.
- `keel_sdk::composer!()` builds a `Composer` (a bundle) from the same two files.
