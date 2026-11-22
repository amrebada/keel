# auth-oauth

Social login through OAuth 2.0 and OpenID Connect, with presets for **Google, Apple, Facebook, LinkedIn, GitHub, Microsoft and X**. Identities are linked to auth-core accounts by verified email, and logins go through auth-core, so sessions, refresh tokens and two-factor auth work the same as for passwords.

```yaml
plugins:
  - use: auth-core
  - use: auth-oauth
    config:
      redirect_base_url: "https://api.shelf.app"
      success_redirect: "https://shelf.app/signed-in"
      providers:
        google: {}
        github: {}
```

| | |
| --- | --- |
| Version | 0.1.0 |
| Capabilities | `sql`, `http-client`, `secrets`, `cache`, `crypto`, `log` |
| Requires plugins | `auth-core ^0.1` |
| Degrade level | `warn` (social login down doesn't take the API down) |
| Setup | one step per enabled provider |

## Config

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `redirect_base_url` | string | `http://localhost:8080` | Public base URL of this API. Callbacks are `{redirect_base_url}/auth/oauth/<provider>/callback`. |
| `success_redirect` | string | none | Frontend URL to send the browser to after login, with the result in the URL fragment. Without it the callback answers JSON. |
| `link_by_email` | bool | `true` | Sign a provider identity into an existing account with the same email, when the provider says the email is verified. |
| `providers.<id>` | object | `{}` | Enable a provider: `google`, `apple`, `facebook`, `linkedin`, `github`, `microsoft`, `x`. |
| `providers.<id>.scopes` | list | the preset's | Override the requested scopes. |
| `providers.<id>.auth_url`, `token_url`, `userinfo_url`, `emails_url` | string | the preset's | Override endpoints, for tests or self-hosted providers. |

## Setup

Each enabled provider adds a step. `keel setup status` prints the exact steps with your redirect URL filled in.

| Provider | Secrets |
| --- | --- |
| Google | `OAUTH_GOOGLE_CLIENT_ID`, `OAUTH_GOOGLE_CLIENT_SECRET` |
| Apple (paid developer account) | `OAUTH_APPLE_CLIENT_ID` (the Services ID), `OAUTH_APPLE_TEAM_ID`, `OAUTH_APPLE_KEY_ID`, `OAUTH_APPLE_PRIVATE_KEY` (the .p8 contents) |
| Facebook | `OAUTH_FACEBOOK_CLIENT_ID`, `OAUTH_FACEBOOK_CLIENT_SECRET` |
| LinkedIn | `OAUTH_LINKEDIN_CLIENT_ID`, `OAUTH_LINKEDIN_CLIENT_SECRET` |
| GitHub | `OAUTH_GITHUB_CLIENT_ID`, `OAUTH_GITHUB_CLIENT_SECRET` |
| Microsoft (Entra) | `OAUTH_MICROSOFT_CLIENT_ID`, `OAUTH_MICROSOFT_CLIENT_SECRET` |
| X | `OAUTH_X_CLIENT_ID`, `OAUTH_X_CLIENT_SECRET` (X doesn't share email addresses) |

A missing step blocks only that provider's routes (`/auth/oauth/<id>/*`), so GitHub works while Google is still being set up.

## Routes

| Route | Does |
| --- | --- |
| `GET /auth/oauth/providers` | Enabled providers: `{providers: [{id, label, start, ready}]}`. `ready` is false until the client id is set. |
| `GET /auth/oauth/{provider}/start` | Redirects the browser to the provider, with a one-time `state` (and PKCE where supported). |
| `GET /auth/oauth/{provider}/callback` | Where the provider sends the browser back. Exchanges the code, reads the profile, finds or creates the account, logs in. |
| `POST /auth/oauth/{provider}/callback` | The same, for providers that post the result (Apple). |

### The flow

1. Your frontend sends the browser to `/auth/oauth/google/start`.
2. The user signs in at Google, which redirects to `/auth/oauth/google/callback`.
3. Keel finds the account: a known Google identity signs into its account; otherwise, if Google says the email is verified and an account has that email, the identity is linked to it (`link_by_email`); otherwise a new account is created with the email marked verified.
4. Without `success_redirect` the callback answers the same JSON as `/auth/login`. With it, the browser goes to `https://shelf.app/signed-in#access_token=…&refresh_token=…&expires_in=900&token_type=Bearer`, or `#mfa_required=true&mfa_token=…`, or `#error=…&error_description=…`.

## Errors

`invalid_state` (the `state` is unknown or expired, often a reused or slow callback), `missing_code` (the provider sent no code), and `unknown_provider` (404) for a provider that isn't enabled.

## Events

`auth.oauth.linked {provider, user_id}` when an identity is linked to an account.

## Tables

`auth_oauth_identities`: provider, subject and user id.
