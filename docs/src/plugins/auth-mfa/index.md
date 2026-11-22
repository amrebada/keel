# auth-mfa

A second factor for auth-core logins: TOTP codes from authenticator apps (Google Authenticator, 1Password, Authy and the like) and one-time recovery codes. Codes can't be replayed, and too many wrong codes lock the second factor for five minutes.

```yaml
plugins:
  - use: auth-core
  - use: auth-mfa
    config:
      issuer: Shelf
```

| | |
| --- | --- |
| Version | 0.1.0 |
| Capabilities | `sql`, `crypto`, `log` |
| Requires plugins | `auth-core ^0.1` |
| Degrade level | `error` |
| Setup | none |

## Config

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `issuer` | string | `Keel` | Name shown in authenticator apps. |
| `recovery_codes` | integer 5–20 | `10` | Recovery codes generated at a time. |
| `max_failures` | integer 3–20 | `5` | Wrong codes in a row before a 5-minute lock. |

## Turning it on (signed-in user)

1. `POST /auth/mfa/totp/setup` with the bearer token returns `{secret, otpauth_url}`. Show `otpauth_url` as a QR code, or the secret for manual entry.
2. `POST /auth/mfa/totp/confirm {"code": "123456"}` with a current code turns it on and returns the recovery codes. They are shown only this once.

## Logging in with it

Once a user has a second factor, every login (password, magic link, social) answers:

```json
{"mfa_required": true, "mfa_token": "…"}
```

instead of tokens. Finish with either:

```sh
curl -XPOST localhost:8080/auth/mfa/verify -d '{"mfa_token":"…","code":"123456"}'
curl -XPOST localhost:8080/auth/mfa/verify -d '{"mfa_token":"…","recovery_code":"…"}'
```

Both answer the normal token pair. A recovery code works once.

## Routes

| Route | Body | Answer |
| --- | --- | --- |
| `GET /auth/mfa/status` | bearer | `{enabled, recovery_codes_left}` |
| `POST /auth/mfa/totp/setup` | bearer | `{secret, otpauth_url, next}` |
| `POST /auth/mfa/totp/confirm` | bearer, `{code}` | turns it on; returns recovery codes |
| `POST /auth/mfa/verify` | `{mfa_token, code}` or `{mfa_token, recovery_code}` | tokens |
| `POST /auth/mfa/disable` | bearer, `{code}` | turns it off |
| `POST /auth/mfa/recovery-codes` | bearer, `{code}` | `{recovery_codes}`: a new set, replacing the old one |

## Errors

| Code | Status | When |
| --- | --- | --- |
| `invalid_code` | 401 | wrong or reused TOTP code, or a wrong or used recovery code |
| `mfa_locked` | 429 | `max_failures` wrong codes in a row; try again in 5 minutes |
| `invalid_mfa_token` | 401 | unknown or expired `mfa_token` |
| `mfa_not_set_up` | 400 | confirming before setup |

## Events

`auth.mfa.enabled {user_id}`, `auth.mfa.disabled {user_id}`.

## Tables

`auth_mfa_totp`, `auth_mfa_recovery`.
