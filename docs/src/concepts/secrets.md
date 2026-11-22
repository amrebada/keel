# Secrets

Secrets never appear in `app.yaml`. You reference them as `${secret:NAME}`, and Keel looks them up when it loads the file. Plugins that declare the `secrets` capability read secrets by name too (auth-core reads `AUTH_JWT_SECRET`, billing-stripe reads `STRIPE_SECRET_KEY`).

```yaml
providers:
  db: { use: postgres, url: "${secret:DATABASE_URL}" }
```

## Where secrets come from

`secrets:` in `app.yaml` picks the source. With no `secrets:` block, it's `env`.

### env (default)

Secrets are environment variables. Commands that read `app.yaml` (`serve`, `dev`, `supervise`, `check`, `config validate|expand`, `migrate`, `setup`) also load a `.env` file beside it:

```sh
# .env
AUTH_JWT_SECRET=6f1c…
DATABASE_URL=postgres://postgres:shelf@127.0.0.1:5432/shelf
```

Variables already set in the shell win over `.env`, so a one-off override works: `DATABASE_URL=… keel migrate`. `.env` is read when the process starts; restart `keel dev` or `keel serve` after editing it. `keel init` adds `.env` to `.gitignore` and writes a `.env.example`.

### vault

```yaml
secrets:
  use: vault
  url: "https://vault.internal:8200"   # default: the VAULT_ADDR environment variable
  mount: secret                         # KV v2 mount, default `secret`
  path: shelf/prod                      # required
  token_env: VAULT_TOKEN                # env var holding the token, default VAULT_TOKEN
  namespace: team-a                     # Vault Enterprise namespace, optional
```

Keel reads every key of the KV v2 secret at `<mount>/data/<path>` once at start. Environment variables and `.env` still win, so you can override one value locally. A Vault error at start says what to fix (address, token, path).

## Missing secrets

`keel config validate` lists every missing secret and where it's used:

```text
error: app.yaml: 3 secret(s) not set
  - REDIS_URL (used at providers.cache.url)
  - DATABASE_URL (used at providers.db.url)
  - REDIS_URL (used at providers.kv.url)
  Set them in the environment or .env (e.g. NAME=value), or in the secret store under `secrets:`
```

Secrets a plugin reads itself, rather than through `${secret:…}`, are tracked as [setup steps](setup-tracking.md): `keel setup status` lists them with instructions, and the routes that need them answer `503 setup_incomplete` until they're set. `keel plugins describe <id>` lists the secrets a plugin reads.

## Good practice

- Generate signing keys with `openssl rand -hex 32` or `openssl rand -base64 48`.
- Changing `AUTH_JWT_SECRET` signs everyone out.
- `keel config expand` and `keel config pull` keep `${secret:…}` references; they never print values.
- Use Vault (or your platform's secret injection into environment variables) in production rather than a `.env` file on disk.
