# Admin API

A running `keel serve`, `keel dev` or `keel supervise` serves an admin API under `/_admin`. It's how `keel apply`, `keel plugins enable|disable|set`, `keel config pull` and `keel audit` talk to the server, and you can call it directly from deploy scripts.

## Authentication

The API is off until the `KEEL_ADMIN_TOKEN` secret is set (environment, `.env` or the secret store). Every call sends it as a bearer token:

```sh
curl -s localhost:8080/_admin/stats -H "Authorization: Bearer $KEEL_ADMIN_TOKEN"
```

Without the secret every call answers `503 admin_disabled`; with a missing or wrong token, `401 admin_token`. Send an `X-Keel-Actor` header to name yourself in the audit log (default `admin-api`).

Expose `/_admin` only on a private network, or block it at your proxy.

## Endpoints

| Method and path | Body | Does |
| --- | --- | --- |
| `GET /_admin/config` | | The running config as YAML, with live changes: `{version, generation, pulled_at, yaml}`. Secrets stay as references. |
| `PUT /_admin/config` | `app.yaml` text | Reconcile the server with this whole document (what `keel apply` does). |
| `GET /_admin/plugins` | | Running plugins: `{version, generation, plugins: [{id, use, version, from, runtime_config}]}`. `from` names the bundle a plugin came from. |
| `POST /_admin/plugins` | `{"use": "ping@^0.1", "config": {...}}` | Enable a plugin. |
| `PUT /_admin/plugins/{id}` | `{"config": {...}}` | Replace a plugin's config. |
| `PATCH /_admin/plugins/{id}/config` | `{"key": value}` | Merge keys into a plugin's config; `runtime_config` keys change in place. |
| `DELETE /_admin/plugins/{id}` | | Disable a plugin. |
| `GET /_admin/setup` | | Re-check and return every setup step (same as `keel --json setup status`). |
| `GET /_admin/audit?limit=50` | | The audit log, newest first: `{entries: [...]}`. |
| `GET /_admin/stats` | | `{pid, generation, version, requests, errors, inflight, uptime_secs, plugins}`. |

## Change results

Every change answers:

```json
{"mode":"reload","version":10,"generation":2,"added":[],"removed":["ping"],"changed":[],"migrations":[]}
```

- `mode`: `none` (nothing changed), `live` (changed in place through `configure`) or `reload` (a new generation took over).
- `version` counts config changes; `generation` counts plugin sets started.
- `migrations` lists the migrations the change ran.

A change that fails leaves the server running as it was, and answers:

| Status | Code | When |
| --- | --- | --- |
| `422` | `invalid_config` | the new config, a plugin's config or a route rule is invalid; `issues` lists each problem with its path |
| `422` | `start_failed` | a plugin failed to start (its `init` returned an error) |
| `409` | `migration_blocked` | the change needs a destructive migration; run `keel migrate --allow-destructive` first |
| `409` | `refused` | the change can't be applied live: a provider, `server`, `secrets` or `app` changed |
| `404` | `not_found` | the plugin isn't running |

```json
{"error":{"code":"invalid_config","message":"…","issues":[{"path":"plugins[2].config.roles","message":"…"}]}}
```
