# Tutorial: build Shelf, a reading-list API

In this tutorial you build **Shelf**, a backend for a shared reading list. By the end it has:

- email sign-up with verification, login, JWT access tokens and refresh tokens
- roles: `admin`, `librarian` and `reader`, with the first admin set from YAML
- a `books` REST API with validation, unique ISBNs, filters, sorting and paging
- rate limits, an admin-only delete and query validation, all in YAML
- private notes that each user sees only for themselves
- a custom plugin you write in Rust that counts finished books from events
- live configuration changes with no restart, and an audit log
- Postgres and Redis instead of SQLite and memory, and a zero-downtime binary update
- an MCP connection so an AI assistant can make the next change for you

You write about 60 lines of YAML and one 100-line plugin. Everything else comes from Keel's plugins.

## Before you start

Finish [Getting started](../getting-started/index.md) so that `keel --version` works. You also need `curl`, `jq` and `openssl`. Step 9 needs Postgres and Redis; Docker is the easiest way to get them.

Each step builds on the previous one. Keep `keel dev` running in one terminal and run the commands in another. Responses shown here were captured from a real run, with ids and timestamps shortened.

## Steps

1. [Create the app](01-create-the-app.md)
2. [Sign users in](02-sign-users-in.md)
3. [Roles and permissions](03-roles.md)
4. [A books API from YAML](04-books-api.md)
5. [Guards, rate limits and validation](05-guards.md)
6. [Private notes per user](06-private-notes.md)
7. [Write your own plugin](07-custom-plugin.md)
8. [Change the running server](08-live-changes.md)
9. [Postgres, Redis and a zero-downtime update](09-production.md)
10. [Let an AI drive it with MCP](10-mcp.md)

The finished `app.yaml` is at the end of [step 9](09-production.md#the-finished-appyaml).
