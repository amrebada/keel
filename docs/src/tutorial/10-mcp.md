# 10. Let an AI drive it with MCP

Everything you did by hand is also available to an AI assistant through Keel's MCP server. The assistant searches plugins, reads a plugin's short description, proposes a new `app.yaml`, validates it, applies it and checks the setup steps, without opening a shell or reading plugin source code.

## Connect Claude Code

From anywhere:

```sh
claude mcp add keel -- keel mcp -c /path/to/shelf/app.yaml
```

Any MCP client works the same way: the server speaks MCP over stdio, started by `keel mcp -c <app.yaml>`.

## Ask for a change

With `keel dev` running, ask the assistant something like:

> Add Google and GitHub sign-in to Shelf, and two-factor auth.

A good run looks like this, one tool call each:

1. `search_plugins {"query": "login"}` finds `auth-oauth`, `auth-mfa` and `saas-auth`.
2. `describe_plugin {"id": "auth-oauth"}` returns its config keys, routes and setup steps in about 300 tokens.
3. `validate_config {"yaml": "…"}` checks the proposed file; the assistant fixes anything reported.
4. `apply_config {"yaml": "…"}` writes `app.yaml` (the old one is kept in `.keel/app.yaml.bak`). `keel dev` applies it within a second.
5. `setup_status` reports what you have to do: create the Google and GitHub OAuth apps, with the exact redirect URLs, and set `OAUTH_GOOGLE_CLIENT_ID` and the other secrets. The assistant asks you only about those.
6. `health` and `logs` confirm the server is happy.

The assistant writes about ten lines of YAML instead of an OAuth implementation.

## Try the tools without a client

The server is plain JSON-RPC on stdio, so you can poke it from a shell:

```sh
printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"sh","version":"1"}}}' \
  '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"setup_status","arguments":{}}}' \
  | keel mcp -c app.yaml
```

The answer to `id: 2` holds the same JSON as `keel --json setup status`:

```json
{"complete":true,"ok":true,"steps":[{"id":"auth-core/jwt-secret","kind":"secret","plugin":"auth-core",
  "secrets":["AUTH_JWT_SECRET"],"status":"done","title":"Create the key that signs login tokens"}]}
```

The full tool list is in [Using Keel with an AI assistant](../mcp/index.md).

## You're done

You built an API with accounts, roles, two resources, guards, a custom plugin, live changes, a production data layer and a zero-downtime update, and you wrote one plugin's worth of Rust. From here:

- Browse the [plugin catalog](../plugins/index.md) for billing, social login and two-factor auth.
- Read [How Keel works](../concepts/index.md) for the model behind what you just did.
- Read [Writing a plugin](../guides/writing-a-plugin.md) before your next custom plugin.
