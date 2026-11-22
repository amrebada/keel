# plugin.yaml reference

The manifest of a plugin. It's validated against `crates/keel-sdk/schemas/plugin.schema.json` and then checked further by `keel_sdk::Manifest`. Check one with `keel plugins check <dir>`.

```yaml
id: invoices                       # required
version: 0.1.0                     # required, semver
summary: One line on what it does. # required
tags: [billing, invoices]
requires:
  host: "^0.1"
  capabilities: [sql, log]
  plugins: { auth-core: "^0.1" }
provides:
  routes: [GET /invoices, POST /invoices]
  events: [invoice.created]
  interfaces: []
consumes:
  events: [billing.subscription.updated]
config: config.schema.json
runtime_config: [currency]
migrations: built-in
setup: []
degrade_level: warn
compose: []                        # only for bundles
```

## Top-level fields

| Field | Required | Rules | Meaning |
| --- | --- | --- | --- |
| `id` | yes | `^[a-z0-9][a-z0-9-]*$` | The name used in `app.yaml` (`use: invoices`). Also the namespace: tables `invoices_*`, keys `invoices:*`. |
| `version` | yes | semver | Matched against `use: invoices@^0.1`. |
| `summary` | yes | | One line. Shown by `plugins list` and searched by MCP `search_plugins`. |
| `tags` | no | | Extra search words (`payments`, `subscriptions`). |
| `requires` | no | | See below. |
| `provides` | no | | See below. |
| `consumes.events` | no | | Events delivered to `Plugin::on_event`. |
| `config` | no | | The config schema file name, by convention `config.schema.json`. `manifest!()` always loads `config.schema.json` beside `plugin.yaml`. |
| `runtime_config` | no | | Config keys that change live through `Plugin::configure`. Others restart the plugin in a new generation. |
| `migrations` | no | | `built-in` when the plugin ships migrations through `Plugin::migrations`. |
| `setup` | no | | Human steps; see [below](#setup). |
| `degrade_level` | no | `info`, `warn`, `error` (default) | How much this plugin being down matters to `/health`. |
| `compose` | no | | Makes the plugin a [bundle](../concepts/bundles.md). |

## requires

| Field | Meaning |
| --- | --- |
| `host` | A semver range the host version (`keel --version`) must match. Default any. |
| `capabilities` | Any of `sql`, `kv`, `cache`, `http-client`, `secrets`, `crypto`, `log`, `email`. The plugin gets only these; each needs its provider configured in `app.yaml` (`sql` → `db`, `kv` → `kv`, `cache` → `cache`, `email` → `email`). |
| `plugins` | Plugin id to semver range. They must be in `app.yaml` and start first. A plugin can't require itself; cycles are errors. |

## provides

| Field | Rules | Meaning |
| --- | --- | --- |
| `routes` | `"<METHOD> /path"`, method one of `GET`, `POST`, `PUT`, `PATCH`, `DELETE`; `{name}` for a path parameter | The only routes the plugin may register, and all of them must be registered. No duplicates. `/health` and `/_admin…` are reserved. Two plugins in one app can't declare the same route. |
| `events` | | The only events the plugin may emit. |
| `interfaces` | | Names the plugin may `provide`: e.g. `identity-provider`, `authorizer`, `guard:plan`. |

## setup

Each entry is one human step:

```yaml
setup:
  - id: google-client                    # required, ^[a-z0-9][a-z0-9-]*$, unique in the plugin
    title: Create a Google OAuth client  # required
    kind: external-account               # required: external-account | secret | dns | webhook | manual
    when: config.providers.google        # only when this config value is set and not false/null
    requires_paid_account: false
    steps:                               # instructions; {config.x} is filled from the plugin's config
      - "Add the redirect URI {config.redirect_base_url}/auth/oauth/google/callback"
    produces: [secret:OAUTH_GOOGLE_CLIENT_ID, secret:OAUTH_GOOGLE_CLIENT_SECRET]
    verify: { type: secret-present }     # secret-present | plugin-check | manual, optional keys: [...]
    blocks: ["/auth/oauth/google/*"]     # routes answering 503 until done
```

| Field | Meaning |
| --- | --- |
| `when` | `config.<path>`. The step applies only when that value exists and isn't `false` or `null`; otherwise it shows as `skipped`. |
| `steps` | Ordered instructions. `{config.a.b}` is replaced with the config value. |
| `produces` | What the user ends up with, as `secret:NAME` (`^secret:[A-Z][A-Z0-9_]*$`). |
| `verify` | Default: `secret-present` for the `produces` secrets, or `manual` if it produces none. `plugin-check` calls `Plugin::check_setup(step_id)`. `keys` overrides which secrets `secret-present` checks. |
| `blocks` | `/path`, `/prefix/*` or `METHOD /path`, with `{name}` for one segment. Default: every route of the plugin. |

`keel setup done <plugin>/<id>` marks a `manual` step done.

## compose

Only for bundles. A list of plugin entries with templates; see [Bundles](../concepts/bundles.md). A composer may not declare routes, capabilities, interfaces or migrations.

```yaml
compose:
  - use: auth-oauth@^0.1
    when: config.social
    config:
      providers: { $keys: "${config.social}" }
```

## Checks beyond the schema

`Manifest::from_sources` also rejects: a `version` or range that isn't valid semver, a plugin that requires itself, duplicate routes, reserved routes, duplicate setup step ids, a composer that includes itself or declares routes, capabilities, interfaces or migrations, and a `config.schema.json` that isn't valid JSON Schema. Every problem is listed with its path.
