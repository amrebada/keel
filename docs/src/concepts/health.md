# Health and degrade levels

`GET /health` reports every plugin and provider. It's reserved by the host (no plugin can claim it) and needs no token.

```json
{
  "status": "degraded",
  "app": "shelf",
  "version": "0.1.0",
  "uptime_secs": 3605,
  "plugins": [
    { "id": "auth-core", "version": "0.1.0", "degrade_level": "error", "status": "ok" },
    { "id": "billing-stripe", "version": "0.1.0", "degrade_level": "warn", "status": "down", "reason": "…" }
  ],
  "providers": [
    { "slot": "db", "kind": "postgres", "capability": "sql", "hot_load": false, "degrade_level": "error", "status": "ok" },
    { "slot": "cache", "kind": "redis", "capability": "cache", "hot_load": true, "degrade_level": "warn", "status": "down", "reason": "…" }
  ],
  "setup": { "complete": false, "incomplete": ["auth-oauth/google-client"] }
}
```

## Status and HTTP code

| `status` | HTTP | When |
| --- | --- | --- |
| `ok` | 200 | everything is up |
| `degraded` | 200 | something is down or degraded, but nothing at level `error` is down |
| `down` | 503 | a plugin or provider at level `error` is down |

Load balancers and `keel supervise` use the HTTP code, so only failures that really break the app take an instance out of rotation.

## Degrade levels

Every plugin and provider has a `degrade_level`: `info`, `warn` or `error`.

- Plugins set a default in `plugin.yaml` (`error` if they don't). auth-oauth and billing-stripe default to `warn`: losing social login or billing shouldn't take the whole API down.
- Providers default to `error` for `db` and `kv`, and `warn` for `cache` and `email`.
- Override either in `app.yaml`:

```yaml
providers:
  cache: { use: redis, url: "${secret:REDIS_URL}", degrade_level: info }
plugins:
  - use: billing-stripe
    degrade_level: error            # billing is critical for this app
    config: { plans: { pro: { price: price_123 } } }
```

Providers are pinged on each `/health` call. Plugins report through `Plugin::health()`, returning `Ok`, `Degraded { reason }` or `Down { reason }`.

## Setup

`setup.incomplete` lists the [setup steps](setup-tracking.md) still missing. Incomplete setup doesn't change `status`; the affected routes answer `503 setup_incomplete` on their own.
