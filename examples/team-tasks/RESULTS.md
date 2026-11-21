# Success test results (2026-11-21)

The v1 success test from the spec: an AI builds the full team task app
twice, once with Keel and once from scratch, and the token counts are
compared. Both builds get the same [SPEC.md](SPEC.md) and must pass the
same black-box [acceptance.py](acceptance.py): 64 checks covering sign-up
with email and Google, roles and permissions per team, and protected CRUD on
Postgres with a Redis cache.

## Setup

- Each build ran as a fresh Claude agent (same model, same tools, same
  instructions apart from the stack).
- **Keel builds** could only learn Keel through its MCP server (`keel mcp`)
  and `keel --help`. They were not allowed to read Keel's source code or
  docs. The deliverable is `app.yaml`.
- **From-scratch build** used Rust with axum, sqlx (Postgres) and redis, with
  no access to Keel. The deliverable is a Cargo project.
- Tokens come from the API usage each agent reported. About 51k tokens of
  every run are the agent's fixed system prompt and tool definitions, so
  "task tokens" below is the context the work itself added on top of that.
- The from-scratch build already shared the same model's knowledge of axum,
  sqlx and redis. Keel is new to the model, so it had to be discovered
  through MCP.

## Results

| | Keel, run 1 | Keel, run 2 | From scratch |
| --- | --- | --- | --- |
| Acceptance | 64/64 | 64/64 | 64/64 |
| Test runs needed | 1 | 1 | 1 |
| What the AI wrote | 45 lines of YAML | 45 lines of YAML | 941 lines of Rust |
| Output tokens (generated) | 6,830 | **3,547** | 21,507 |
| Task tokens (context) | 38,212 | **18,745** | 31,547 |
| Total tokens (with fixed overhead) | 91,008 | **71,258** | 83,675 |
| Model calls | 18 | 9 | 14 |
| Wall time | 79 s | **38 s** | 261 s |

**Run 1** used the phase 7 binary. It used fewer generated tokens than the
from-scratch build, but more context, because the agent had to discover
things by trial and error:
- `describe_plugin` only listed top-level config keys, so the agent pulled
  full JSON schemas to see nested settings.
- Nothing said how to reference a secret. `${REDIS_URL}` passed validation
  and failed only when the server connected.
- Provider spellings (`postgres`, `redis`) were not named anywhere.

**Run 2** used this branch, which fixes all three:
- `describe --ai` now shows the nested config.
- It also names the provider spellings, the `${secret:NAME}` syntax and the
  secrets each plugin reads.
- Validation flags `${NAME}`, `${env:NAME}` and `$NAME`.

With those fixes the Keel build compared with the from-scratch build:
- **84% fewer** generated tokens
- **41% fewer** task tokens
- **15% fewer** total tokens
- about **7 times faster**

## What these numbers do and don't show

- Each column is one run, so expect run-to-run noise of several thousand
  tokens.
- The app is small, so the fixed 51k overhead takes up a large share of
  each total. The generated-token gap grows with the size of the app: the
  from-scratch build writes code per feature, while the Keel build writes a
  few lines of YAML per resource or role.
- Changes after the first build favour Keel even more:
  - A new field is one line of YAML and `keel apply`.
  - In the from-scratch app, the same change means editing the schema, the
    validation, the filters and the handlers.
- The from-scratch app has no:
  - live config changes
  - zero-downtime binary swaps
  - audit log
  - MFA
  - setup tracking
  - per-route guards

  Keel brings all of these without more tokens.

## Live update under load

`crates/keel/tests/team_tasks.rs` runs `examples/team-tasks/app.yaml` under
`keel supervise` on Postgres and Redis. It needs `KEEL_TEST_POSTGRES_URL`,
`KEEL_TEST_REDIS_URL` and `python3`. The test:
1. passes `acceptance.py`;
2. runs 8 clients in a loop, against a protected, cached `GET /api/tasks`
   and against `/health`;
3. while the clients run, applies a config change with `keel apply` (a new
   `priority` field with its migration, plus a different cache TTL);
4. still under load, swaps the binary with `keel update`.

It asserts that not one request fails and that the data survives both
changes.
