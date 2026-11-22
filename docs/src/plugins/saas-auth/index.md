# saas-auth

A bundle: everything a SaaS app needs to sign users in (accounts, social login, two-factor and roles) from one small config. It has no code of its own; Keel expands it into [auth-core](../auth-core/index.md), [auth-oauth](../auth-oauth/index.md), [auth-mfa](../auth-mfa/index.md) and [rbac](../rbac/index.md) before anything starts.

```yaml
plugins:
  - use: saas-auth@^1
    config:
      app_url: "https://api.shelf.app"
      app_name: Shelf
      social: [google, github]
      roles: [admin, member]
      admins: [you@example.com]
```

| | |
| --- | --- |
| Version | 1.0.0 |
| Expands into | auth-core, auth-oauth (when `social` is not empty), auth-mfa (when `mfa` is true), rbac |
| Setup | that of the plugins it includes |

## Config

| Key | Default | Becomes |
| --- | --- | --- |
| `app_url` | `http://localhost:8080` | auth-core `app_url` and auth-oauth `redirect_base_url` |
| `app_name` | `Keel` | auth-mfa `issuer` |
| `social` | `[]` | auth-oauth `providers` (one entry each). Any of `google`, `apple`, `facebook`, `linkedin`, `github`, `microsoft`, `x`. Empty leaves auth-oauth out. |
| `success_redirect` | none | auth-oauth `success_redirect` |
| `mfa` | `true` | include auth-mfa |
| `roles` | `[admin, member]` | rbac `roles`. A list of names (`admin_role` gets `*`, the others no permissions until granted) or a full rbac roles map with permissions and inherits. |
| `admin_role` | `admin` | rbac `admin_role` |
| `default_role` | `member` | rbac `default_role` |
| `admins` | `[]` | rbac `admins` |
| `signup` | `true` | auth-core `signup` |
| `require_verified_email` | `true` | auth-core `require_verified_email` |
| `with` | | Any setting of an included plugin, deep-merged: `with: { auth-core: { password_min_length: 12 } }` |

Roles with permissions:

```yaml
  - use: saas-auth@^1
    config:
      mfa: false
      roles:
        admin: { permissions: ["*"] }
        owner: { permissions: ["tasks.*", "rbac.manage"], inherits: [member] }
        member: { permissions: ["tasks.read"] }
```

## See what it becomes

```sh
keel config expand
```

prints the plugin entries it expands into, with every default filled in. `GET /_admin/plugins` shows each running plugin's `from: saas-auth`.

## Setup

Each social provider needs its OAuth app credentials, `OAUTH_<PROVIDER>_CLIENT_ID` and `OAUTH_<PROVIDER>_CLIENT_SECRET` (Apple uses `OAUTH_APPLE_CLIENT_ID`, `OAUTH_APPLE_TEAM_ID`, `OAUTH_APPLE_KEY_ID` and `OAUTH_APPLE_PRIVATE_KEY`), plus auth-core's `AUTH_JWT_SECRET`. `keel setup status` lists them with steps.

How bundles work, and how to write one: [Bundles](../../concepts/bundles.md).
