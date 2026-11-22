# Using Keel with an AI assistant

Keel was designed for AI-assisted backend work. The assistant should configure plugins instead of writing code, and spend tokens only on what's unique to your app. Keel gives it two ways in: an **MCP server** and **`--json` on every CLI command**.

## Connect over MCP

```sh
claude mcp add keel -- keel mcp -c /path/to/app.yaml
```

For other MCP clients, configure a stdio server with the command `keel mcp -c /path/to/app.yaml`. The server tells the client how to work with it:

> Keel builds a backend from app.yaml and compiled-in plugins. Prefer configuring plugins over writing code: search_plugins, describe_plugin, then add_plugin or apply_config. After any change run setup_status and ask the user only about missing or failed steps.

## Tools

| Tool | Arguments | Does |
| --- | --- | --- |
| `search_plugins` | `query` (words; empty lists all) | Search compiled-in plugins by id, summary and tags (e.g. `payments`, `login`, `roles`, `email`). |
| `describe_plugin` | `id` | A plugin's config keys with defaults, routes, events, interfaces and setup steps, in about 300 tokens. |
| `read_config` | | The current `app.yaml` text and the expanded config with defaults. |
| `validate_config` | `yaml` (optional) | Check `app.yaml`, or a proposed version, against every schema, secret and plugin rule. Writes nothing. |
| `apply_config` | `yaml` | Replace `app.yaml` after it passes validation; the old file is kept as `.keel/app.yaml.bak`. A running `keel dev` reloads it within a second. |
| `add_plugin` | `plugin` (`rbac` or `rbac@^0.1`) | Add a plugin with a minimal valid config, keeping comments. |
| `setup_status` | | Every setup step: done, missing, failed or skipped, with instructions and secrets. |
| `setup_done` | `id` (`<plugin>/<step>`), `done` (default true) | Mark a manual step done after the user confirms it. |
| `migrations` | `apply` (default false) | Plan migrations (dry run with row counts), or run the additive ones. Destructive migrations are never run from here. |
| `health` | | `GET /health` of the running server. |
| `logs` | `lines` (default 100), `contains` | The last lines of `.keel/keel.log`. |

Each tool returns the same JSON as the matching CLI command with `--json`.

### Guard rails

- `apply_config` validates first and refuses an invalid file, so the assistant can't break a running `keel dev`.
- Destructive migrations can't run through MCP; a person runs `keel migrate --allow-destructive` themselves.
- Secret values never pass through the tools: config keeps `${secret:…}` references, and setup steps only say which secrets are missing.

## A typical session

> "Add Stripe billing with a pro plan, and make /reports need it."

1. `search_plugins {"query": "payments"}` → `billing-stripe`.
2. `describe_plugin {"id": "billing-stripe"}` → config keys (`plans`, `public_url`…), the `plan:` guard, three setup steps.
3. `validate_config` with the new `app.yaml`:
   ```yaml
   plugins:
     - use: billing-stripe
       config: { plans: { pro: { price: price_123 } } }
   routes:
     /reports/**: { guard: [authenticated, "plan:pro"] }
   ```
4. `apply_config` with the same text.
5. `setup_status` → "create a Stripe account and set `STRIPE_SECRET_KEY`; create a price for `pro`; add the webhook and set `STRIPE_WEBHOOK_SECRET`". The assistant relays exactly those to you.
6. After you set them: `health` and `logs`.

## Without MCP: the CLI

Every command takes `--json` and prints one object, `{"ok": true, …}` or `{"ok": false, "error": {"message", "issues"}}`. The commands an assistant needs most:

```sh
keel --json plugins list
keel plugins describe resource --ai
keel add rbac
keel --json config validate
keel config expand
keel --json setup status
keel --json migrate
```

Validation lists every problem with its path at once, so a fix takes one round trip instead of one per error.

## Tips for prompting

- Point the assistant at `keel plugins list` (or `search_plugins`) before it writes any code.
- Ask it to put app-specific logic in a small plugin (see [Writing a plugin](../guides/writing-a-plugin.md)) rather than in a separate service.
- Have it finish every change with `setup_status`, so you only hear about what you actually need to do.
- For a measured comparison of token use with and without Keel, see the [team-tasks results](https://github.com/amrebada/keel/blob/main/examples/team-tasks/RESULTS.md).
