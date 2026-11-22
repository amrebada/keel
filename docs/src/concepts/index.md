# How Keel works

Keel has three layers: a **host** that reads `app.yaml` and serves HTTP, **plugins** that implement features, and **providers** that connect to databases, caches, email and secret stores. Plugins never talk to the outside world directly; they ask the host for capabilities, and the host hands them the providers `app.yaml` chose.

```text
                 app.yaml  ──►  keel (host)
                                 │
        ┌────────────────────────┼─────────────────────────┐
        │                        │                         │
  route rules              plugins (compiled in)       providers
  guards, validation       auth-core, rbac,            sql: sqlite | postgres | mysql
  rate limits              resource, billing-stripe,   cache, kv: memory | redis
                           your own …                  email: console | smtp | resend | ses
        │                        │                     secrets: env + .env | vault
        └──── HTTP request ──────┴──── capabilities ───────┘
```

## What happens at start

When you run `keel serve` (or `dev`, or `supervise`):

1. **Load `app.yaml`.** It's checked against the [app schema](../reference/app-yaml.md). `${secret:NAME}` references are resolved from the [secret source](secrets.md). Every problem is reported at once, with its path.
2. **Expand bundles.** A [composer](bundles.md) such as `saas-auth` is replaced by the plugins it stands for.
3. **Check plugins.** Each entry must name a compiled-in plugin whose version matches the range (`rbac@^0.1`), whose `requires.host` matches the host, whose required plugins are present, whose capabilities have a provider, and whose `config:` passes its JSON Schema. No two plugins may claim the same route. All of this happens before anything starts.
4. **Connect providers** and report each in `/health`.
5. **Run migrations.** Additive migrations run; a [destructive](migrations.md) one stops the start unless you allowed it.
6. **Start plugins in dependency order.** Each gets an `InitContext` with its config, the capabilities it declared, a route registrar limited to its declared routes, an event emitter limited to its declared events, and the shared interface registry.
7. **Evaluate setup steps.** Routes whose [setup step](setup-tracking.md) is missing answer `503 setup_incomplete`.
8. **Serve.** Each request goes through the setup check, then the [route rules](routes-and-guards.md) (guards and validation), then the plugin's handler.

## The manifest is the contract

Every plugin ships a `plugin.yaml`. It lists the capabilities the plugin needs, the plugins it depends on, the routes and events it provides, the events it consumes, its config schema, the config keys that can change live, and the human setup steps it needs. The host enforces it: a plugin that registers an undeclared route, emits an undeclared event or asks for an undeclared capability gets an error. That's what makes the manifest safe to rely on, and why an AI can use a plugin by reading 300 tokens instead of its source.

## Why this saves tokens

An AI building a backend from scratch writes and debugs auth flows, permission checks, CRUD handlers and migrations. With Keel it:

- finds plugins with `keel plugins list --json` or the MCP `search_plugins` tool,
- reads `keel plugins describe <id> --ai`, about 300 tokens per plugin,
- writes a few lines of YAML and validates them with `keel config validate`, which reports every problem with its path so fixes take one round,
- reads `keel setup status` to know exactly which human steps remain,
- writes Rust only for logic that is unique to the app, as a small plugin.

## The pages in this section

- [app.yaml](app-yaml.md): the file and its sections.
- [Plugins](plugins.md): what a plugin is, versions, dependencies, the lifecycle.
- [Providers](providers.md): SQL, cache, key-value and email, and how plugins are isolated.
- [Secrets](secrets.md): `${secret:NAME}`, `.env` and Vault.
- [Setup tracking](setup-tracking.md): human steps a plugin needs.
- [Routes, guards and validation](routes-and-guards.md).
- [Events and interfaces](events-and-interfaces.md): how plugins talk to each other.
- [Migrations](migrations.md): planning, safety and backups.
- [Bundles](bundles.md): composer plugins such as `saas-auth`.
- [Health and degrade levels](health.md).
- [Live changes and binary swaps](live-operations.md).
