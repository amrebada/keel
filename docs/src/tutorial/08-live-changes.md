# 8. Change the running server

`keel dev` applies your `app.yaml` saves live. Production servers run `keel serve` (or `keel supervise`), and you change them through the **admin API**, which the CLI wraps. Nothing restarts and no request is dropped: Keel starts the new set of plugins next to the running one, switches new requests over, and lets in-flight requests finish on the old one. A change that fails validation, a dependency check or a migration leaves the server as it was.

## Turn on the admin API

The admin API is off until you set a token:

```sh
echo "KEEL_ADMIN_TOKEN=$(openssl rand -hex 24)" >> .env
```

Restart `keel dev` so it reads `.env`. The CLI commands below read the same `.env` and send the token for you.

## Change a plugin setting in place

`goal` is listed in reading-stats' `runtime_config`, so it changes without restarting the plugin:

```sh
keel plugins set reading-stats goal=2
curl -s localhost:8080/stats
```

```text
changed in place, no restart (version 9): reading-stats
```

```json
{"added":0,"finished":0,"goal":2,"progress":"0/2"}
```

Values are read as YAML, so `goal=2` is a number. A key that isn't in `runtime_config` is still applied live, by starting a new generation of plugins next to the running one, as described above.

## Disable and enable plugins

```sh
keel plugins disable ping
curl -s -o /dev/null -w "%{http_code}\n" localhost:8080/ping       # 404
keel plugins enable ping --with '{ message: "hello from shelf" }'
curl -s localhost:8080/ping                                         # {"message":"hello from shelf"}
```

```text
applied live (version 10, generation 2)
  removed: ping
applied live (version 11, generation 3)
  added: ping
```

## The audit log

Every change is recorded with who made it:

```sh
keel audit --limit 5
```

```text
2026-11-22 07:30:46 UTC  plugin.configure   reading-stats  cli:you@laptop  {"mode":"live","patch":{"goal":2},"version":9}
2026-11-22 07:30:46 UTC  plugin.disable     app            cli:you@laptop  {"removed":["ping"],"version":10,…}
2026-11-22 07:30:46 UTC  plugin.enable      app            cli:you@laptop  {"added":["ping"],"version":11,…}
2026-11-22 07:30:36 UTC  config.apply       app            keel dev        {"added":["reading-stats"],"version":7,…}
2026-11-22 07:29:30 UTC  migration.applied  resource/notes_01_owner  host start  {"kind":"additive","tables":["resource_notes"],…}
```

## Keep app.yaml and the server in sync

Live changes don't edit `app.yaml`. You have two ways to reconcile them:

```sh
keel config pull            # print the running config as YAML
keel config pull --write    # overwrite app.yaml with it (comments are not kept)
keel apply                  # or the other way: make the server match app.yaml
```

Run `keel apply` now. Since `app.yaml` doesn't have the `goal: 2` or the new ping message, they go back to the file's values:

```text
applied live (version 12, generation 4)
  changed: ping, reading-stats
```

`keel apply` is how you deploy config changes to a production server: edit `app.yaml`, commit, `keel apply`. Providers, `server` and `secrets` can't change live; they need a restart or a binary swap.

The same operations are HTTP endpoints under `/_admin`, listed in the [admin API reference](../reference/admin-api.md).

Next: [Postgres, Redis and a zero-downtime update](09-production.md).
