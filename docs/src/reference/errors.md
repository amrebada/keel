# Errors

Every Keel route, built-in or from a plugin, answers errors in one shape:

```json
{"error": {"code": "forbidden", "message": "this needs the `books.write` permission", "permission": "books.write"}}
```

- `code` is stable, machine-readable snake_case. Branch on it, not on `message`.
- `message` is a sentence for people, often with the fix.
- Extra fields add details (`permission`, `field`, `issues`, `step`, `limit`…).

Validation errors carry `issues`, a list of `{path, message}`:

```json
{"error":{"code":"validation_failed","message":"2 problem(s) in the request","issues":[
  {"path":"body.pages","message":"0 is less than the minimum of 1"},
  {"path":"body.status","message":"\"lost\" is not one of \"want\", \"reading\" or 2 other candidates"}]}}
```

## Host

| Code | Status | Meaning |
| --- | --- | --- |
| `setup_incomplete` | 503 | A setup step this route needs is missing (`step`, `hint`). Run `keel setup status`. |
| `missing_token` | 401 | A guard needs a signed-in user and no bearer token was sent. |
| `token_expired` | 401 | The access token expired; call `/auth/refresh`. |
| `invalid_token` | 401 | The access token is invalid or its session was revoked. |
| `forbidden` | 403 | A `role:` or `permission:` guard failed (`role` or `permission`). |
| `not_owner` | 403 | An `owner:` guard failed. |
| `rate_limited` | 429 | A `rate-limit:` guard failed (`limit`, `period_secs`). |
| `validation_failed` | 422 | `validate.body` or `validate.query` failed (`issues`). |
| `invalid_body` | 400 | The JSON body couldn't be parsed. |
| `body_too_large` | 413 | A body over 1 MB on a route with `validate.body`. |
| `internal` | 500 | An unexpected error; the cause is in the server log. |

## Admin API

| Code | Status | Meaning |
| --- | --- | --- |
| `admin_disabled` | 503 | `KEEL_ADMIN_TOKEN` isn't set. |
| `admin_token` | 401 | Missing or wrong admin token. |
| `invalid_config` | 422 | The change is invalid (`issues`). |
| `start_failed` | 422 | A plugin failed to start. |
| `migration_blocked` | 409 | The change needs a destructive migration. |
| `refused` | 409 | The change can't apply live (providers, server, secrets, app). |
| `not_found` | 404 | The plugin isn't running. |

## Plugins

| Plugin | Codes |
| --- | --- |
| auth-core | `email_taken` 409, `weak_password` 400, `signup_disabled` 403, `invalid_credentials` 401, `email_not_verified` 403, `invalid_token` 400/401, `token_expired` 401, `invalid_refresh_token` 401 |
| auth-oauth | `unknown_provider` 404, `invalid_state` 400, `missing_code` 400 |
| auth-mfa | `invalid_code` 401, `mfa_locked` 429, `invalid_mfa_token` 401, `mfa_not_set_up` 400 |
| rbac | `forbidden` 403, `unknown_user` 404, `invalid_permission` 400 |
| resource | `validation_failed` 422, `conflict` 409 (`field`), `not_found` 404, `forbidden` 403, `missing_tenant` 400, `invalid_offset` 400, `needs_auth` 503, `needs_rbac` 503 |
| billing-stripe | `plan_required` 402 (`plan`), `unknown_plan` 400, `already_subscribed` 409, `no_customer` 404, `bad_signature` 400, `stripe_error` 502, `stripe_unreachable` 502 |

Each plugin's page explains when its codes happen.

## CLI

With `--json`, a failed command prints `{"ok": false, "error": {"message": "…", "issues": [{"path", "message"}]}}` and exits with status 1. Without it, the error goes to stderr:

```text
error: app.yaml: 3 problem(s) found
  - app: "Team Tasks" does not match "^[a-z0-9][a-z0-9-]*$"
  - plugins[0]: "use" is a required property
  - server: Additional properties are not allowed ('prot' was unexpected)
```
