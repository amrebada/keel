# Live changes and binary swaps

Keel changes a running server in two ways:

- **Live changes** (config): enable, disable or reconfigure plugins, and change route rules, in-process with no restart.
- **Binary swaps** (code): run a new `keel` binary, with new or updated plugins, under a supervisor that hands over the port with no dropped requests and rolls back on failure.

## Live changes

A running server keeps one **generation**: a started set of plugins and its router. A change:

1. validates the new config (schema, secrets, plugins, versions, dependencies, route rules);
2. runs additive migrations (a destructive one refuses the change);
3. starts a new generation next to the running one;
4. switches new requests to it;
5. lets in-flight requests finish on the old generation (up to 30 seconds), then calls `drain()` on removed plugins.

Any failure before step 4 leaves the running generation untouched. A change that only touches keys a plugin lists in `runtime_config` skips the new generation and calls the plugin's `configure()` in place.

What can't change live: `providers`, `server`, `secrets` and `app`. Those need a restart or a binary swap; the change is refused with that message. (`keel dev` restarts itself for them.)

### Ways to make a live change

| Tool | Command |
| --- | --- |
| Edit while developing | save `app.yaml` under `keel dev` |
| Reconcile with the file | `keel apply` |
| Enable a plugin | `keel plugins enable rbac --with '{ default_role: member }'` |
| Disable a plugin | `keel plugins disable ping` |
| Change keys | `keel plugins set ping message=hello` (values read as YAML) |
| Read back | `keel config pull`, `keel config pull --write` |
| HTTP | the [admin API](../reference/admin-api.md) under `/_admin` |

The CLI commands find the server from `server.host` and `server.port` in `app.yaml` (override with `--url`) and authenticate with the `KEEL_ADMIN_TOKEN` secret. Without that secret the admin API is off.

### Audit log

Every change is written to the `keel_audit` table in the database, with the actor (`cli:<user>@<host>`, `keel dev`, `host start`, `admin-api`), the action and the details. Read it with `keel audit` or `GET /_admin/audit`:

```text
2026-11-22 07:30:46 UTC  plugin.configure   reading-stats  cli:you@laptop  {"mode":"live","patch":{"goal":2},"version":9}
2026-11-22 07:30:36 UTC  config.apply       app            keel dev        {"added":["reading-stats"],…}
2026-11-22 07:29:30 UTC  migration.applied  resource/notes_01_owner  host start  {"kind":"additive",…}
```

## Binary swaps

New plugin code means a new binary. `keel supervise` owns the listening socket and runs `keel serve` as a child; `keel update` swaps the child:

```sh
keel supervise                                  # instead of keel serve
keel update --binary ./target/release/keel      # or --build
keel status                                     # running and previous binary
keel rollback                                   # back to the previous binary
```

`keel update` steps, each printed as it happens:

1. **verify**: hash the binary and keep a copy in `.keel/bin/`;
2. **check**: run `config validate` with the new binary;
3. **migrate**: run additive migrations with the new binary;
4. **warm-up**: start it on the shared socket and wait for a healthy `/health`;
5. **swap**: the new process takes new requests;
6. **drain**: the old process finishes in-flight requests and exits;
7. **watch**: for `--watch-secs` (default 60, set on `keel supervise`), a failing health check, a crash or an error rate over 50% swaps back to the previous binary automatically.

Swaps and rollbacks are recorded in `.keel/swaps.jsonl`. Binary swaps are Unix only.

The team-tasks test (`crates/keel/tests/team_tasks.rs`) runs a live config change and a binary swap under load and checks that no request fails.
