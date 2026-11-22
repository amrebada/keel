# Keel

Keel is a backend framework written in Rust. You describe your backend in one YAML file, `app.yaml`, and Keel runs it. Sign-up and login, social login, two-factor auth, roles, CRUD APIs, Stripe billing and email already exist as **plugins**. You turn them on and configure them; you don't write them again.

It was built for a specific problem: when an AI assistant builds a backend, most of its tokens go into writing the same login, permission and CRUD code that every app needs. With Keel the assistant reads a plugin's short manifest, writes a few lines of YAML, and spends its effort only on the parts that are unique to your app.

```yaml
app: shelf
providers:
  db: { use: postgres, url: "${secret:DATABASE_URL}" }
  cache: { use: redis, url: "${secret:REDIS_URL}" }
  email: { use: resend, api_key: "${secret:RESEND_API_KEY}", from: "Shelf <hi@shelf.app>" }
plugins:
  - use: saas-auth
    config: { social: [google], roles: [admin, member], admins: [you@example.com] }
  - use: resource
    config:
      resources:
        books:
          fields:
            title: { type: string, required: true }
            author: { type: string, required: true }
routes:
  "POST /api/books": { guard: ["rate-limit:30/min"] }
```

That file is a working API with email and Google sign-in, two-factor auth, roles, a `books` table with validated REST endpoints, and a rate limit.

## What you get

- **Plugins compiled into one binary.** Each plugin is a Rust crate with a manifest (`plugin.yaml`) and a JSON Schema for its config. The host refuses to start a plugin whose config, dependencies, routes or capabilities don't check out, and tells you every problem with its path.
- **Pluggable data layer.** SQLite, Postgres or MySQL for SQL, memory or Redis for cache and key-value, and console, SMTP, Resend or SES for email. Switching is a YAML change; plugins never see the difference.
- **Setup tracking.** Plugins declare the human steps they need (create a Google OAuth app, add a Stripe webhook). `keel setup status` lists what is missing and how to do it, and the routes that depend on a missing step answer `503 setup_incomplete` instead of failing in odd ways.
- **Guards and validation in YAML.** Put `authenticated`, `role:admin`, `permission:books.write`, `owner:id`, `rate-limit:30/min`, `plan:pro` or a JSON Schema in front of any route.
- **Live changes.** Enable, disable and reconfigure plugins on a running server with no restart and no dropped requests. Ship new plugin code as a new binary that is swapped in under load and rolled back automatically if health fails.
- **Built for AI tools.** Every CLI command has `--json`, `keel plugins describe --ai` fits a plugin in about 300 tokens, and `keel mcp` exposes the whole workflow as MCP tools.

## Does it actually save tokens?

The [team-tasks success test](https://github.com/amrebada/keel/tree/main/examples/team-tasks) had an AI build the same app (email and Google sign-in, per-team roles, protected CRUD on Postgres with a Redis cache) three times through Keel's MCP server and three times from scratch in Rust, and then make the same follow-up change both ways. Every run passed the same acceptance tests. Keel used **86% fewer output tokens for the build** and **41% fewer for the change**. The [results file](https://github.com/amrebada/keel/blob/main/examples/team-tasks/RESULTS.md) has the method and the caveats.

## Where to go next

- New here? Follow [Getting started](getting-started/index.md), then the [tutorial](tutorial/index.md), which builds a real API step by step. Every command in it was run against the current code.
- Want the model? Read [How Keel works](concepts/index.md).
- Looking for a plugin's options? See the [plugin catalog](plugins/index.md).
- Writing your own plugin? See [Writing a plugin](guides/writing-a-plugin.md).
- Using Claude Code or another MCP client? See [Using Keel with an AI assistant](mcp/index.md).

## Status

Keel is at version 0.1 (v1 of the design: all eight phases are done). It runs on Linux and macOS; the zero-downtime binary swap (`keel supervise`) is Unix only. Plugins are compiled in, so adding a new plugin means building a new `keel` binary from the repository.
