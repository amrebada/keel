# 1. Create the app

Create the project and start the development server:

```sh
keel init shelf
cd shelf
keel dev
```

```text
wrote shelf/app.yaml
wrote shelf/.env.example
wrote shelf/.gitignore
```

`keel dev` keeps running. It serves `127.0.0.1:8080`, writes its log to the terminal and to `.keel/keel.log`, and applies each save of `app.yaml` to the running server.

In a second terminal, inside `shelf/`:

```sh
curl localhost:8080/ping
```

```json
{"message":"pong"}
```

```sh
curl -s localhost:8080/health | jq '{status, plugins: [.plugins[].id]}'
```

```json
{ "status": "ok", "plugins": ["ping"] }
```

## What's in the folder

| File | Purpose |
| --- | --- |
| `app.yaml` | The whole backend: providers, plugins, route rules. |
| `.env.example` | The secrets you'll likely need. Copy values into `.env`. |
| `.gitignore` | Keeps `.env` and the SQLite files out of git. |
| `shelf.db` | Created on first start: the SQLite database. |
| `.keel/` | Created on first start: the log, setup state, audit log and supervisor state. |

## Look around

Keel's CLI is built to answer questions in a few lines, which is also what makes it cheap for an AI to use:

```sh
keel plugins list              # every plugin compiled into this binary, one line each
keel plugins describe resource --ai
```

`describe --ai` prints the plugin's config keys with types and defaults, its routes, events, dependencies and setup steps. It's the only thing you, or an assistant, need to read before using a plugin.

Next: [sign users in](02-sign-users-in.md).
