# 2. Sign users in

Add the `auth-core` plugin:

```sh
keel add auth-core
```

```text
added auth-core@^0.1 to app.yaml
```

`keel add` appends `- use: auth-core@^0.1` under `plugins:`, keeps your comments, and prints the plugin's description. Because `keel dev` is running, the plugin starts right away:

```text
keel dev: applied live (version 2; added ["auth-core"], removed [], changed [])
```

## A missing setup step

Try to sign up:

```sh
curl -s -XPOST localhost:8080/auth/signup \
  -d '{"email":"ada@example.com","password":"correct horse battery"}'
```

```json
{"error":{"code":"setup_incomplete","step":"auth-core/jwt-secret",
  "message":"this route needs a setup step first: Create the key that signs login tokens",
  "hint":"run `keel setup status` to see what is missing"}}
```

`auth-core` signs tokens with a secret you have to provide. Instead of failing in an odd way, its routes answer `503 setup_incomplete` until the step is done. Ask Keel what's missing:

```sh
keel setup status
```

```text
setup: 0 of 1 done, 1 missing, 0 failed, 0 skipped
  TODO auth-core/jwt-secret  Create the key that signs login tokens
         1. Generate 32+ random bytes, e.g. `openssl rand -base64 48`
         2. Store it as AUTH_JWT_SECRET (environment, .env or the secret store). Changing it logs everyone out.
         next: set AUTH_JWT_SECRET
routes blocked by these steps answer 503 setup_incomplete until they are done
```

Add the secret to `.env`:

```sh
echo "AUTH_JWT_SECRET=$(openssl rand -hex 32)" >> .env
keel setup status        # setup: 1 of 1 done
```

The server reads `.env` when it starts, so **stop `keel dev` (Ctrl-C) and start it again**. Edits to `app.yaml` apply live; edits to `.env` need a restart.

## Sign up and verify the email

```sh
curl -s -XPOST localhost:8080/auth/signup \
  -d '{"email":"ada@example.com","password":"correct horse battery"}'
```

```json
{"user":{"id":"abfc8077-…","email":"ada@example.com","email_verified":false,"name":null},"verification_sent":true}
```

Password login is refused until the email is verified (`require_verified_email` defaults to `true`):

```json
{"error":{"code":"email_not_verified","message":"confirm your email first; POST /auth/verify/resend sends the link again"}}
```

The `console` email provider prints the email in the `keel dev` log instead of sending it:

```text
INFO keel_providers::builtin: email (console provider, not sent):
Confirm your email address by opening this link:

http://localhost:8080/auth/verify?token=9ffa7e70…

If you did not sign up, ignore this email. from=shelf <noreply@localhost> to=ada@example.com subject=Confirm your email
```

Open the link, or pull the token out of the log:

```sh
TOKEN=$(grep -o 'verify?token=[0-9a-f]*' .keel/keel.log | tail -1 | cut -d= -f2)
curl -s "localhost:8080/auth/verify?token=$TOKEN"      # {"verified":true}
```

## Log in

```sh
curl -s -XPOST localhost:8080/auth/login \
  -d '{"email":"ada@example.com","password":"correct horse battery"}' | tee ada.json
```

```json
{"access_token":"eyJhbGciOiJIUzI1NiIs…","refresh_token":"a6217a45…","token_type":"Bearer","expires_in":900}
```

The access token is a JWT that lasts 15 minutes (`access_ttl_secs`). The refresh token lasts 30 days and rotates: `POST /auth/refresh {"refresh_token": "..."}` returns a new pair, and reusing an old refresh token revokes the session. Keep the access token in a variable:

```sh
ADA="Authorization: Bearer $(jq -r .access_token ada.json)"
curl -s localhost:8080/auth/me -H "$ADA"
```

```json
{"user":{"id":"abfc8077-…","email":"ada@example.com","email_verified":true,"name":null}}
```

If a later command answers `401`, the access token has expired; log in again and reset `ADA`.

## What else auth-core does

Without more config you also have password reset (`/auth/password/forgot`, `/auth/password/reset`), magic links (`/auth/magic-link`), `logout` and `logout-all`. Social login and two-factor are separate plugins, [auth-oauth](../plugins/auth-oauth/index.md) and [auth-mfa](../plugins/auth-mfa/index.md). The [auth-core page](../plugins/auth-core/index.md) lists every route and option.

Next: [roles and permissions](03-roles.md).
