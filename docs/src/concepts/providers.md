# Providers: database, cache, key-value, email

Providers are the connections Keel makes for your plugins. You pick them under `providers:` in `app.yaml`; plugins only see a capability (`sql`, `cache`, `kv`, `email`) and never know which product is behind it. Moving from SQLite to Postgres, or from memory to Redis, changes `app.yaml` and nothing else.

```yaml
providers:
  db:    { use: postgres, url: "${secret:DATABASE_URL}", max_connections: 20 }
  cache: { use: redis, url: "${secret:REDIS_URL}" }
  kv:    { use: redis, url: "${secret:REDIS_URL}", prefix: "kv:" }
  email: { use: resend, api_key: "${secret:RESEND_API_KEY}", from: "Shelf <hi@shelf.app>" }
```

| Slot | Capability | `use` | Settings |
| --- | --- | --- | --- |
| `db` | `sql` | `sqlite`, `postgres`, `mysql` | `url` (required), `max_connections` (default 10), `degrade_level` |
| `cache` | `cache` | `memory`, `redis` | `url` (redis), `prefix`, `degrade_level` |
| `kv` | `kv` | `memory`, `redis` | `url` (redis), `prefix`, `degrade_level` |
| `email` | `email` | `console`, `smtp`, `resend`, `ses` | see below |

Four more capabilities need no provider config: `http-client` (outbound HTTPS), `crypto` (random bytes, Argon2id password hashing), `secrets` (read secrets by name from the [secret source](secrets.md)) and `log` (structured logs tagged with the plugin id).

A plugin that requires a capability with no provider is refused at start: `keel config validate` says which plugin needs which slot.

## SQL

- **SQLite**: `url: "sqlite://shelf.db"` creates the file on first use. `sqlite::memory:` works for tests.
- **Postgres**: `url: "postgres://user:pass@host:5432/db"`.
- **MySQL**: `url: "mysql://user:pass@host:3306/db"`.

The url's scheme must match `use`. Plugins write SQL with `?` placeholders, and the provider rewrites them for the database in use. Plugins that ship migrations provide one set per dialect.

## Cache and key-value

Both are byte stores. The cache has time-to-live and atomic counters (used by `rate-limit` guards and resource caching); the key-value store is durable. With `memory`, both live in the process and are lost on restart, and rate limits aren't shared between instances. Use Redis in production. `prefix` prepends a string to every key, useful when cache and kv share one Redis.

Every plugin's keys are also prefixed with its id, so `reading-stats` writing `finished` stores `kv:reading-stats:finished` in the example above.

## Email

```yaml
email:
  use: smtp                       # console | smtp | resend | ses
  from: "Shelf <hi@shelf.app>"
  host: smtp.example.com
  port: 587
  username: "${secret:SMTP_USER}"
  password: "${secret:SMTP_PASSWORD}"
  tls: starttls                   # starttls (default, 587) | tls (465) | none
```

| `use` | Required settings |
| --- | --- |
| `console` | none. Prints each email to the log instead of sending it. For development. |
| `smtp` | `host`, `from`; optional `port`, `username`, `password`, `tls` |
| `resend` | `api_key`, `from`; optional `base_url` |
| `ses` | `region`, `access_key_id`, `secret_access_key`, `from`; optional `session_token`, `endpoint` |

A real sender adds a `providers/email-domain` setup step reminding you to set up SPF and DKIM for your domain.

### Templates

Plugins send emails by template id (`auth.verify`, `auth.reset`, `auth.magic_link`). Override any of them in `app.yaml`; `{{name}}` placeholders are filled from the plugin's variables, such as `{{link}}`:

```yaml
email:
  use: resend
  api_key: "${secret:RESEND_API_KEY}"
  from: "Shelf <hi@shelf.app>"
  templates:
    auth.verify:
      subject: "Welcome to Shelf"
      html: "<p>Confirm your email: <a href=\"{{link}}\">{{link}}</a></p>"
```

A template sets `subject`, `text` and/or `html`; anything it leaves out keeps the plugin's default.

## Health

Each provider appears in `/health` with its status and `degrade_level`. By default losing `db` or `kv` is an error (503) and losing `cache` or `email` is a warning (200, `degraded`). Override with `degrade_level:` on the provider. See [Health and degrade levels](health.md).

## Changing providers on a running server

Providers can't change through a live apply; they need a restart (`keel dev` restarts itself when you edit them) or a [binary swap](live-operations.md).
