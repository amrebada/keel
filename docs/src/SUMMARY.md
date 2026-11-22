# Summary

[Introduction](introduction.md)

# Start here

- [Getting started](getting-started/index.md)
- [Tutorial: build Shelf, a reading-list API](tutorial/index.md)
  - [1. Create the app](tutorial/01-create-the-app.md)
  - [2. Sign users in](tutorial/02-sign-users-in.md)
  - [3. Roles and permissions](tutorial/03-roles.md)
  - [4. A books API from YAML](tutorial/04-books-api.md)
  - [5. Guards, rate limits and validation](tutorial/05-guards.md)
  - [6. Private notes per user](tutorial/06-private-notes.md)
  - [7. Write your own plugin](tutorial/07-custom-plugin.md)
  - [8. Change the running server](tutorial/08-live-changes.md)
  - [9. Postgres, Redis and a zero-downtime update](tutorial/09-production.md)
  - [10. Let an AI drive it with MCP](tutorial/10-mcp.md)

# Concepts

- [How Keel works](concepts/index.md)
  - [app.yaml](concepts/app-yaml.md)
  - [Plugins](concepts/plugins.md)
  - [Providers: database, cache, key-value, email](concepts/providers.md)
  - [Secrets](concepts/secrets.md)
  - [Setup tracking](concepts/setup-tracking.md)
  - [Routes, guards and validation](concepts/routes-and-guards.md)
  - [Events and interfaces](concepts/events-and-interfaces.md)
  - [Migrations](concepts/migrations.md)
  - [Bundles (composers)](concepts/bundles.md)
  - [Health and degrade levels](concepts/health.md)
  - [Live changes and binary swaps](concepts/live-operations.md)

# Plugins

- [Plugin catalog](plugins/index.md)
  - [auth-core](plugins/auth-core/index.md)
  - [auth-oauth](plugins/auth-oauth/index.md)
  - [auth-mfa](plugins/auth-mfa/index.md)
  - [rbac](plugins/rbac/index.md)
  - [resource](plugins/resource/index.md)
  - [billing-stripe](plugins/billing-stripe/index.md)
  - [saas-auth](plugins/saas-auth/index.md)
  - [ping](plugins/ping/index.md)

# Guides

- [Writing a plugin](guides/writing-a-plugin.md)
- [Going to production](guides/production.md)
- [Using Keel with an AI assistant](mcp/index.md)

# Reference

- [CLI](cli/index.md)
- [app.yaml reference](reference/app-yaml.md)
- [plugin.yaml reference](reference/plugin-yaml.md)
- [Plugin SDK (Rust API)](reference/sdk.md)
- [Admin API](reference/admin-api.md)
- [Errors](reference/errors.md)
- [Repository layout and development](reference/development.md)
