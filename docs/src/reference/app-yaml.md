# app.yaml reference

The schema is `crates/keel-core/schemas/app.schema.json`. Unknown keys are errors everywhere except inside a plugin's `config` (which its own schema checks) and route rule schemas.

## Top level

| Key | Type | Required | Default |
| --- | --- | --- | --- |
| `app` | string, `^[a-z0-9][a-z0-9-]*$` | yes | |
| `server` | object | no | `{host: 0.0.0.0, port: 8080}` |
| `providers` | object with `db`, `cache`, `kv`, `email` | no | none |
| `secrets` | object | no | `{use: env}` |
| `plugins` | list | no | `[]` |
| `routes` | map | no | `{}` |

## server

| Key | Type | Default |
| --- | --- | --- |
| `host` | string | `0.0.0.0` |
| `port` | integer 1–65535 | `8080` |

## providers.db

| Key | Type | Notes |
| --- | --- | --- |
| `use` | `postgres` \| `mysql` \| `sqlite` | required |
| `url` | string | required; `postgres://…`, `mysql://…`, `sqlite://file.db` or `sqlite::memory:` |
| `max_connections` | integer 1–1000 | default 10 |
| `degrade_level` | `info` \| `warn` \| `error` | default `error` |

## providers.cache and providers.kv

| Key | Type | Notes |
| --- | --- | --- |
| `use` | `memory` \| `redis` | required |
| `url` | string | required for `redis`: `redis://host:6379` |
| `prefix` | string | prepended to every key |
| `degrade_level` | `info` \| `warn` \| `error` | default `warn` for cache, `error` for kv |

## providers.email

| Key | Type | Used by |
| --- | --- | --- |
| `use` | `console` \| `smtp` \| `resend` \| `ses` | required |
| `from` | string, e.g. `"Shelf <hi@shelf.app>"` | required for smtp, resend, ses |
| `host`, `port`, `username`, `password` | | smtp (`host` required) |
| `tls` | `starttls` (default) \| `tls` \| `none` | smtp |
| `api_key`, `base_url` | | resend (`api_key` required) |
| `region`, `access_key_id`, `secret_access_key`, `session_token`, `endpoint` | | ses (first three required) |
| `templates` | map of template id to `{subject, text, html}` | all; `{{var}}` placeholders |
| `degrade_level` | `info` \| `warn` \| `error` | default `warn` |

## secrets

| Key | Type | Notes |
| --- | --- | --- |
| `use` | `env` \| `vault` | required |
| `url` | string | Vault address; default `VAULT_ADDR` |
| `token_env` | `^[A-Z][A-Z0-9_]*$` | env var holding the token; default `VAULT_TOKEN` |
| `mount` | string | KV v2 mount; default `secret` |
| `path` | string | required for vault, e.g. `shelf/prod` |
| `namespace` | string | Vault Enterprise namespace |

## plugins[]

| Key | Type | Notes |
| --- | --- | --- |
| `use` | `^[a-z0-9][a-z0-9-]*(@<semver range>)?$` | required, e.g. `rbac` or `rbac@^0.1` |
| `config` | object | checked against the plugin's schema |
| `degrade_level` | `info` \| `warn` \| `error` | overrides the manifest's |

## routes

Keys are patterns `"[METHOD ]/path"` (`*` one segment, `**` the rest, `{name}` a captured segment). Values:

| Key | Type | Notes |
| --- | --- | --- |
| `guard` | list of guard strings | all must pass: `authenticated`, `role:x`, `permission:x`, `owner:param`, `rate-limit:n/period`, `any(...)`, `all(...)`, plugin guards like `plan:x` |
| `validate.body` | JSON Schema | for the JSON body |
| `validate.query` | JSON Schema | for the query string, values coerced to the property types |

See [Routes, guards and validation](../concepts/routes-and-guards.md).

## Secret references

Any string may contain `${secret:NAME}`. Validation runs before resolution, and missing secrets are listed with every path that uses them.
