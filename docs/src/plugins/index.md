# Plugin catalog

These plugins are compiled into `keel` 0.1. Each page lists the plugin's config (with defaults), routes, events, dependencies and setup steps. The same information, in short form, comes from `keel plugins describe <id> --ai`.

| Plugin | What it gives you | Needs |
| --- | --- | --- |
| [auth-core](auth-core/index.md) | Email and password accounts, email verification, password reset, magic links, JWT access tokens with rotating refresh tokens, sessions, logout everywhere | `db`, `email`; secret `AUTH_JWT_SECRET` |
| [auth-oauth](auth-oauth/index.md) | Social login with Google, Apple, Facebook, LinkedIn, GitHub, Microsoft and X, linked by verified email | auth-core, `db`, `cache`; an OAuth app per provider |
| [auth-mfa](auth-mfa/index.md) | Two-factor auth: TOTP authenticator apps and recovery codes, with replay protection and lockout | auth-core, `db` |
| [rbac](rbac/index.md) | Roles and permissions in YAML, inheritance, per-user grants, roles per tenant; powers `role:` and `permission:` guards | auth-core, `db` |
| [resource](resource/index.md) | REST CRUD from YAML models: tables, migrations, validation, filters, sorting, paging, per-owner or per-tenant rows, caching | `db`, `cache`; auth-core and rbac for protected access |
| [billing-stripe](billing-stripe/index.md) | Stripe subscriptions per tenant: plans in YAML, Checkout, customer portal, signed webhooks, `plan:` guard | auth-core, `db`; a Stripe account |
| [saas-auth](saas-auth/index.md) | Bundle: auth-core, auth-oauth, auth-mfa and rbac from one small config | as its parts |
| [ping](ping/index.md) | Example plugin: `GET /ping` with a configurable message | nothing |

## Choosing

- **Accounts only**: auth-core.
- **Accounts plus social login, 2FA and roles**: saas-auth, or the four plugins individually when you want full control of each config.
- **A data API**: resource, plus auth-core and rbac unless it's public.
- **Paid plans**: billing-stripe, then guard routes with `plan:<name>`.
- **Something none of these do**: [write a plugin](../guides/writing-a-plugin.md).

## Common conventions

- Every error is `{"error": {"code": "...", "message": "...", ...details}}` with a stable snake_case `code`. See [Errors](../reference/errors.md).
- Routes that need a signed-in user read `Authorization: Bearer <access token>` from auth-core.
- Routes that work per team read the tenant from `X-Tenant-Id`.
- Plugin tables are prefixed with the plugin id (`auth_core_users`, `rbac_assignments`, `resource_books`).
