# Setup tracking

Many features need something only a human can do: create a Google OAuth app, copy a Stripe key, add a webhook, set up DNS. Keel makes those steps part of the plugin. Each plugin lists its steps under `setup:` in `plugin.yaml`, Keel tracks which are done, and the routes that depend on a missing step answer `503 setup_incomplete` instead of failing halfway.

This is what lets an AI assistant finish a feature: it reads `keel setup status` and asks you only about what's actually missing.

## keel setup status

```sh
keel setup status
```

```text
setup: 1 of 3 done, 2 missing, 0 failed, 0 skipped
  ok   auth-core/jwt-secret  Create the key that signs login tokens
  TODO auth-oauth/google-client  Create a Google OAuth client
         1. Open https://console.cloud.google.com/apis/credentials
         2. Create credentials > OAuth client ID, type Web application
         3. Add the redirect URI http://localhost:8080/auth/oauth/google/callback
         4. Copy the client ID and secret
         next: set OAUTH_GOOGLE_CLIENT_ID, OAUTH_GOOGLE_CLIENT_SECRET
  …
routes blocked by these steps answer 503 setup_incomplete until they are done
```

Each step is one of:

| Status | Meaning |
| --- | --- |
| `done` | The check passed. |
| `missing` | Not done yet; the instructions say how. |
| `failed` | A plugin check ran and failed, with the reason (for example, Stripe rejected the key). |
| `skipped` | The step doesn't apply to this config (its `when` is false, such as the Apple step when Apple sign-in is off). |

`keel --json setup status`, the MCP `setup_status` tool and `GET /_admin/setup` return the same data as JSON. `/health` includes a short summary under `setup`.

## How a step is checked

| `verify.type` | Done when |
| --- | --- |
| `secret-present` (default when the step `produces` secrets) | every listed secret is set |
| `plugin-check` | the plugin's `check_setup(step_id)` returns `Ok` (auth-core checks the JWT secret is long enough; billing-stripe calls Stripe) |
| `manual` (default when the step produces nothing) | you ran `keel setup done <plugin>/<step>` |

Manual marks are stored in `.keel/setup.json`. Undo one with `keel setup undo <plugin>/<step>`.

The running server re-checks every step every 10 seconds, so `keel setup done` and plugin checks take effect without a restart. Secrets in `.env`, however, are only read when the server starts.

## Blocked routes

By default a missing step blocks every route of its plugin. A step can narrow that with `blocks:`, as auth-oauth does per provider so that a missing Google app doesn't block GitHub sign-in:

```yaml
setup:
  - id: google-client
    title: Create a Google OAuth client
    kind: external-account
    when: config.providers.google
    steps:
      - "Add the redirect URI {config.redirect_base_url}/auth/oauth/google/callback"
    produces: [secret:OAUTH_GOOGLE_CLIENT_ID, secret:OAUTH_GOOGLE_CLIENT_SECRET]
    blocks: ["/auth/oauth/google/*"]
```

A blocked request gets:

```json
{"error":{"code":"setup_incomplete","step":"auth-oauth/google-client",
  "message":"this route needs a setup step first: Create a Google OAuth client",
  "hint":"run `keel setup status` to see what is missing"}}
```

## Writing steps

The full format is in the [plugin.yaml reference](../reference/plugin-yaml.md#setup). In short: `id`, `title`, `kind` (`external-account`, `secret`, `dns`, `webhook`, `manual`), optional `when`, `steps` (with `{config.x}` filled in from the plugin's config), `produces`, `requires_paid_account`, `verify` and `blocks`.

Providers can add steps too: a real email provider (`smtp`, `resend`, `ses`) adds `providers/email-domain` for SPF and DKIM, marked done with `keel setup done providers/email-domain`.
