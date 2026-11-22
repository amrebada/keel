# auth-core

Email and password accounts: sign-up with Argon2id password hashing, email verification, password reset, magic links, JWT access tokens with rotating refresh tokens, sessions, and logout everywhere. Other auth plugins (auth-oauth, auth-mfa, rbac) build on it through its `identity-provider` interface.

```yaml
plugins:
  - use: auth-core
    config:
      app_url: "https://api.shelf.app"
```

| | |
| --- | --- |
| Version | 0.1.0 |
| Capabilities | `sql`, `crypto`, `email`, `secrets`, `log` |
| Requires plugins | none |
| Provides interface | `identity-provider` |
| Degrade level | `error` |
| Setup | `jwt-secret`: set `AUTH_JWT_SECRET` |

## Setup

Generate a signing key and store it as `AUTH_JWT_SECRET`:

```sh
echo "AUTH_JWT_SECRET=$(openssl rand -base64 48)" >> .env
```

The step is checked by the plugin (`plugin-check`), so a key that is too short shows as `failed` with the reason. Until it passes, every auth-core route answers `503 setup_incomplete`. Changing the key signs everyone out.

## Config

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `app_url` | string | `http://localhost:8080` | Base URL used in links sent by email. |
| `links.verify` | string | `{app_url}/auth/verify?token={token}` | Verify-email link. Point it at your frontend if it handles the page. |
| `links.reset` | string | `{app_url}/reset-password?token={token}` | Password reset link (your frontend posts the new password to `/auth/password/reset`). |
| `links.magic_link` | string | `{app_url}/auth/magic-link/verify?token={token}` | Magic link. |
| `signup` | bool | `true` | Allow new accounts through `POST /auth/signup`. |
| `require_verified_email` | bool | `true` | Refuse password login until the email is verified. |
| `magic_link` | bool | `true` | Allow magic-link login. |
| `password_min_length` | integer 8–128 | `8` | Minimum password length. |
| `access_ttl_secs` | integer 60–86400 | `900` | Access token lifetime. |
| `refresh_ttl_secs` | integer 3600–31536000 | `2592000` | Refresh token lifetime (30 days). |
| `token_ttl_secs` | integer 300–604800 | `86400` | Lifetime of verification, reset and magic-link tokens. Magic links are capped at 15 minutes. |

## Routes

| Route | Body or query | Answer |
| --- | --- | --- |
| `POST /auth/signup` | `{email, password, name?}` | `201 {user, verification_sent}` and a verification email |
| `POST /auth/login` | `{email, password}` | tokens, or `{mfa_required: true, mfa_token}` when the user has a second factor |
| `POST /auth/refresh` | `{refresh_token}` | a new token pair; the old refresh token stops working |
| `POST /auth/logout` | `{refresh_token}` | `{logged_out: true}` |
| `POST /auth/logout-all` | bearer token | `{revoked_sessions: n}` |
| `GET /auth/me` | bearer token | `{user}` |
| `GET /auth/verify?token=` | | `{verified: true}` (the link in the email) |
| `POST /auth/verify` | `{token}` | `{verified: true}` |
| `POST /auth/verify/resend` | `{email}` | `202` |
| `POST /auth/password/forgot` | `{email}` | `202` and a reset email |
| `POST /auth/password/reset` | `{token, password}` | `{reset: true, revoked_sessions: n}` |
| `POST /auth/magic-link` | `{email}` | `202` and a login link |
| `GET /auth/magic-link/verify?token=` | | tokens (or `mfa_required`) |
| `POST /auth/magic-link/verify` | `{token}` | tokens (or `mfa_required`) |

Tokens look like:

```json
{"access_token":"eyJ…","refresh_token":"a621…","token_type":"Bearer","expires_in":900}
```

`202` answers are the same whether or not the account exists, so they don't reveal which emails are registered. A user is a JSON object `{id, email, email_verified, name}`.

### Sessions and refresh tokens

Each login starts a session. The access token is a JWT (HS256) with the user id (`sub`) and session id (`sid`); it's also rejected once its session is revoked. Refresh tokens rotate on every use, and presenting a refresh token that was already used revokes the whole session (reuse detection). `logout-all` and a password reset revoke every session of the user.

## Errors

| Code | Status | When |
| --- | --- | --- |
| `email_taken` | 409 | sign-up with an email that has an account |
| `weak_password` | 400 | shorter than `password_min_length` (or over 1024 characters) |
| `signup_disabled` | 403 | `signup: false` |
| `invalid_credentials` | 401 | wrong email or password |
| `email_not_verified` | 403 | password login before verifying, with `require_verified_email` |
| `invalid_token` | 400 / 401 | a bad or expired verification, reset or magic-link token, or an invalid access token |
| `token_expired` | 401 | the access token expired; refresh it |
| `invalid_refresh_token` | 401 | an unknown, expired, revoked or reused refresh token |
| `missing_token` | 401 | no bearer token |

## Events

`auth.user.created {user_id, email}`, `auth.user.verified {user_id}`, `auth.login {user_id, session_id}`, `auth.logout {user_id, all}`, `auth.password.reset {user_id}`.

## Emails

auth-core sends three templates, which you can restyle under `providers.email.templates` (see [Providers](../../concepts/providers.md#templates)):

| Template id | Sent for |
| --- | --- |
| `auth.verify` | sign-up and `verify/resend` |
| `auth.reset` | `password/forgot` (the link works once, for one hour) |
| `auth.magic_link` | `magic-link` |

Every template gets `{{link}}`, `{{email}}` and `{{app_url}}`; `auth.reset` also gets `{{name}}`.

## Tables

`auth_core_users`, `auth_core_sessions`, `auth_core_tokens`, `auth_core_used_refresh`.
