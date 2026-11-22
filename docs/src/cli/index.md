# CLI

```text
keel [--json] <command> [options]
```

`--json` works on every command: it prints one JSON object instead of text, for scripts and AI agents. Commands that read `app.yaml` take `-c, --config <path>` (default `app.yaml`) and load `.env` beside it.

## Create and run

| Command | Does |
| --- | --- |
| `keel init [dir] [--name <name>]` | Create `app.yaml` (SQLite, memory cache and kv, console email, the `ping` plugin), `.env.example` and `.gitignore` in `dir` (default `.`). The app name defaults to the directory name. Refuses to overwrite an existing `app.yaml`. |
| `keel dev` | Serve and apply every save of `app.yaml` live. A broken edit is reported and the server keeps running; a provider or `server` change restarts it. Logs to stderr and `.keel/keel.log`. |
| `keel serve` | Serve from `app.yaml` (runs additive migrations first). Changes come through `keel apply` and the admin API. |
| `keel supervise [--binary <path>] [--watch-secs 60]` | Own the port and run `keel serve` as a child, so `keel update` can swap binaries. Unix only. |

## Config

| Command | Does |
| --- | --- |
| `keel config validate` / `keel check` | Check `app.yaml`, its secrets, plugins, versions, dependencies and route rules without starting anything. Exit code 1 with every problem listed. |
| `keel config expand` | Print `app.yaml` with bundles expanded and defaults filled in. Secrets stay as references. |
| `keel config pull [--url <url>] [--write]` | Print the running server's config with live changes; `--write` overwrites `app.yaml` (comments are not kept). |
| `keel add <plugin>[@range]` | Add a compiled-in plugin to `app.yaml` with a minimal valid config, keeping comments, and print its description. |

## Plugins

| Command | Does |
| --- | --- |
| `keel plugins list` | The plugins compiled into this binary: id, version, summary, tags, routes. |
| `keel plugins describe <id> [--ai]` | Config keys with types and defaults, routes, events, interfaces, dependencies, secrets and setup steps. `--ai` is a compact form of about 300 tokens. |
| `keel plugins check <dir or plugin.yaml>` | Validate a plugin's manifest and config schema. |
| `keel plugins enable <plugin>[@range] [--with '<yaml>']` | Start a plugin on the running server. |
| `keel plugins disable <id>` | Stop a plugin on the running server. |
| `keel plugins set <id> key=value…` | Change config keys of a running plugin; values are YAML. `runtime_config` keys change in place. |
| `keel new-plugin <id>` | Inside the Keel repository: create `plugins/<id>` from a template and register it in `crates/keel`. |

`enable`, `disable` and `set` take `--url` (default from `server.host` and `server.port`) and need `KEEL_ADMIN_TOKEN`.

## Setup

| Command | Does |
| --- | --- |
| `keel setup status` | Every setup step of the configured plugins and providers: done, missing, failed or skipped, with instructions and the secrets each needs. |
| `keel setup done <plugin>/<step>` | Mark a manual step done (stored in `.keel/setup.json`). |
| `keel setup undo <plugin>/<step>` | Mark it not done again. |

## Database

| Command | Does |
| --- | --- |
| `keel migrate` | Dry run: every pending migration, additive or destructive, and the tables it touches. |
| `keel migrate --apply` | Run pending additive migrations. Exits non-zero while a destructive one is blocked. |
| `keel migrate --apply --allow-destructive <plugin>/<id>` | Also run that destructive migration, after copying its tables to `keel_backup_<time>_<table>`. Repeat the flag for several. |

## Live operations

| Command | Does |
| --- | --- |
| `keel apply [--url <url>]` | Reconcile the running server with `app.yaml`. |
| `keel audit [--limit 30] [--url <url>]` | Recent changes: config, plugins, migrations, binary swaps and rollbacks. |
| `keel update --binary <path>` / `--build` | Swap the supervised server to a new binary: verify, check config, migrate, warm up, swap, drain, watch, and roll back on failure. `--build` runs `cargo build --release --bin keel` in the current directory. |
| `keel rollback` | Swap back to the previous binary. |
| `keel status` | The running binary (hash, pid) and the previous one. |

## AI

| Command | Does |
| --- | --- |
| `keel mcp` | Serve the Model Context Protocol on stdio. See [Using Keel with an AI assistant](../mcp/index.md). |

## Environment

| Variable | Used for |
| --- | --- |
| `KEEL_ADMIN_TOKEN` | Turns on the admin API; the CLI sends it for live commands. |
| `RUST_LOG` | Log filter, e.g. `RUST_LOG=keel_core=debug` (default `info`). |
| `VAULT_ADDR`, `VAULT_TOKEN` | Vault secret source defaults. |
| anything you reference as `${secret:NAME}` | secrets |

## Exit codes

`0` on success, `1` on any error (invalid config, failed check, refused change, blocked destructive migration). With `--json`, success prints `{"ok": true, …}` and failure `{"ok": false, "error": {"message", "issues"?}}`, where `issues` lists `{path, message}` for config problems.
