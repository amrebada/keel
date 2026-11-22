# Routes, guards and validation

Plugins own their routes. `routes:` in `app.yaml` decides who may call them and what a request must look like, for any plugin, without touching plugin code.

```yaml
routes:
  /admin/**: { guard: [authenticated, "role:admin"] }
  /users/{id}/**: { guard: ["any(role:admin, owner:id)"] }
  "POST /projects":
    guard: ["permission:projects.write", "rate-limit:30/min"]
    validate:
      body:
        type: object
        required: [name]
        properties: { name: { type: string, minLength: 1 } }
  "GET /api/projects":
    validate:
      query: { type: object, properties: { limit: { type: integer, maximum: 50 } } }
  /reports/**: { guard: [authenticated, "plan:pro"] }
```

## Patterns

A key is `"[METHOD ]/path"`:

| Part | Matches |
| --- | --- |
| `GET ` (optional) | only that method (`GET`, `POST`, `PUT`, `PATCH`, `DELETE`, `HEAD`, `OPTIONS`); without it, any method |
| `/literal` | that segment |
| `*` | exactly one segment |
| `{name}` | one segment, captured for `owner:name` |
| `**` | the rest of the path, including nothing; only as the last segment |

**Every rule whose pattern matches applies**, and all must pass. So a broad rule (`/api/**: { guard: [authenticated] }`) and a narrow one (`"DELETE /api/books/{id}": { guard: ["role:admin"] }`) combine.

## Guards

A rule's `guard` is a list; every entry must pass.

| Guard | Passes when | Fails with |
| --- | --- | --- |
| `authenticated` | the request has a valid access token (`Authorization: Bearer …`) | `401 missing_token` or the identity provider's error |
| `role:<name>` | the user has the role, globally or in the request's tenant | `403 forbidden` |
| `permission:<name>` | the user has the permission through a role, an inherited role or a direct grant | `403 forbidden` |
| `owner:<param>` | the user's id equals the `{param}` captured by the pattern | `403 not_owner` |
| `rate-limit:<n>/<period>` | the caller made fewer than `n` requests in the current window | `429 rate_limited` |
| `any(g1, g2, …)` | at least one inner guard passes | the first inner failure |
| `all(g1, g2, …)` | every inner guard passes | the first failure |
| `<name>:<arg>` | a guard offered by a plugin, e.g. `plan:pro` from billing-stripe | the plugin's error, e.g. `402 plan_required` |

Details:

- `role:` and `permission:` need the [rbac](../plugins/rbac/index.md) plugin, and any guard that needs a user needs [auth-core](../plugins/auth-core/index.md). `keel config validate` reports a guard whose plugin is missing.
- **Tenant**: send `X-Tenant-Id: team-1` to evaluate roles and permissions granted in `team-1`, on top of global ones.
- **Rate limits** count per signed-in user, or per client address for anonymous requests (the first `X-Forwarded-For` entry when present, so put Keel behind a proxy that sets it). Periods: `s`, `min`, `hour`, `day`, optionally with a count (`10min`). Windows are fixed and stored in the `cache` provider, so use Redis to share them across instances.
- Guards nest: `"any(role:admin, all(permission:posts.write, owner:id))"`.

## Validation

`validate.body` and `validate.query` are JSON Schemas.

- `body` is checked against the JSON request body (up to 1 MB). Invalid JSON is `400 invalid_body`.
- `query` is checked against the query string as an object. Values are converted to the type the schema gives the property (`?limit=10` becomes the integer `10`).

A failure answers `422`:

```json
{"error":{"code":"validation_failed","message":"1 problem(s) in the request",
  "issues":[{"path":"query.limit","message":"500 is greater than the maximum of 50"}]}}
```

## Order of checks

For each request the host checks, in order:

1. the plugin's [setup steps](setup-tracking.md) for this route (`503 setup_incomplete`),
2. the guards of every matching rule,
3. validation of every matching rule,
4. then calls the plugin's handler, which may do its own checks (the `resource` plugin checks `books.read` and similar by itself).

## Writing a guard in a plugin

A plugin can offer a guard kind. It implements `keel_sdk::authz::GuardCheck`, provides it as the interface `guard:<name>`, and lists that under `provides.interfaces`:

```yaml
# plugin.yaml
provides:
  interfaces: ["guard:plan"]
```

```rust
use keel_sdk::authz::{guard_interface, GuardCheck, GuardContext};

struct PlanGuard;

#[async_trait]
impl GuardCheck for PlanGuard {
    async fn check(&self, ctx: &GuardContext, arg: &str) -> Result<(), ApiError> {
        // ctx.user, ctx.tenant, ctx.params, ctx.method, ctx.path, ctx.client
        todo!()
    }
}

// in init():
ctx.provide::<dyn GuardCheck>(&guard_interface("plan"), Arc::new(PlanGuard))?;
```

Route rules can then use `plan:pro`.
