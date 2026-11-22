# Going to production

A checklist for running a Keel app for real.

## Build

```sh
cargo build --release -p keel        # target/release/keel, with every plugin you compiled in
```

Ship that binary with your `app.yaml`. There's nothing else to deploy: plugins, migrations and schemas are inside the binary.

## Data

- [ ] `db` is Postgres or MySQL, not SQLite, if more than one instance runs.
- [ ] `cache` is Redis, so rate limits and resource caching are shared between instances.
- [ ] `kv` is Redis (or another durable store), so plugin state survives restarts.
- [ ] `keel migrate` shows the plan you expect; run `keel migrate --apply` in your deploy step, or let `keel serve` run additive migrations at start.
- [ ] Destructive migrations are run deliberately with `--allow-destructive`, and you know where the `keel_backup_*` tables are.

## Secrets

- [ ] Secrets come from your platform's environment injection or from Vault (`secrets: { use: vault, … }`), not a `.env` file on disk.
- [ ] `AUTH_JWT_SECRET` is at least 32 random bytes and stored safely; rotating it signs everyone out.
- [ ] `KEEL_ADMIN_TOKEN` is set only if you use live changes, and `/_admin` isn't reachable from the internet.
- [ ] `keel config validate` passes in the target environment (it checks every secret is set).

## Setup steps

- [ ] `keel setup status` shows everything `done`. OAuth redirect URLs, Stripe webhooks and email domains point at production URLs (`app_url`, `redirect_base_url`, `public_url`).
- [ ] The email provider is `smtp`, `resend` or `ses`, the sending domain has SPF and DKIM (`providers/email-domain`), and the `auth.*` templates carry your branding.

## Network

- [ ] `server.host` is `0.0.0.0` (the default) inside a container, behind a TLS-terminating proxy or load balancer.
- [ ] The proxy sets `X-Forwarded-For`, which rate limits use to tell anonymous clients apart.
- [ ] The load balancer's health check uses `GET /health` (503 only when an `error`-level item is down).
- [ ] Degrade levels match what really matters to your app (see [Health](../concepts/health.md)).

## Access rules

- [ ] Every resource uses `access: permissions` or `authenticated` unless it's meant to be public.
- [ ] Write routes have rate limits in `routes:`.
- [ ] `admins:` in rbac lists only the people who should be admins; it grants the role at every start.

## Deploys

- **Config changes**: commit `app.yaml`, then `keel apply` against the running server. Changes to `providers`, `server` or `secrets` need a restart or a binary swap.
- **Code changes**: run under `keel supervise` and deploy with `keel update --binary <new>`. It validates, migrates, warms up, swaps with no dropped requests, and rolls back automatically for 60 seconds if health fails. Or use your platform's rolling deploys with `keel serve`, since `/health` and additive start-up migrations make each instance safe to roll.
- **Audit**: `keel audit` shows every config change, migration and binary swap with its actor.

## Logs

The server logs to stderr (and `.keel/keel.log` beside `app.yaml`). Set `RUST_LOG` to adjust, for example `RUST_LOG=info,keel_core=debug`. Plugin logs are tagged with the plugin id.
