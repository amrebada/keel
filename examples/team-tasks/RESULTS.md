# Success test results (2026-11-21)

**Headline: an AI working through Keel generated 86% fewer tokens to build
the team task app, and 41% fewer to change it, than the same AI writing it
from scratch.** That is the mean of three runs per side for each task, and
every run passed the same black-box acceptance test.

The spec's v1 success test has an AI build the full team task app with
Keel and from scratch, and compares the token counts. This adds a second
task, a follow-up change to the running app, because most real work on a
backend is changing it rather than building it.

## Tasks

- **Build**: implement [SPEC.md](SPEC.md):
  - sign-up with email and Google;
  - per-team roles;
  - protected CRUD for projects and tasks on Postgres, with a Redis cache.

  Done means [acceptance.py](acceptance.py) passes (64 checks).
- **Change**: implement [CHANGE.md](CHANGE.md) on a working app whose
  database already holds data:
  - a `priority` field on tasks;
  - a read-only `viewer` role;
  - a new `comments` resource.

  Done means [acceptance_change.py](acceptance_change.py) (52 checks) and
  `acceptance.py` both pass.

## Results

Means over three runs per side. Every one of the 12 runs passed its tests
on the first test run.

| | Keel build | Scratch build | Keel change | Scratch change |
| --- | --- | --- | --- | --- |
| **Output tokens (generated)** | **3,187** | **23,525** | **1,947** | **3,326** |
| Task context tokens | 22,166 | 34,348 | 13,794 | 24,791 |
| Model calls | 10 | 15.3 | 8.3 | 7.3 |
| Wall time | 36 s | 234 s | 31 s | 54 s |
| What the AI wrote | 45 lines of YAML | 816 lines of Rust | about 12 lines of YAML | about 55 lines of Rust |

Compared with from scratch, Keel used:
- for the build: 86% fewer output tokens and 35% less task context;
- for the change: 41% fewer output tokens and 44% less task context.

The runs, in order: Keel build kb2-kb4, scratch build sb1-sb3, Keel change
kc1-kc3, scratch change sc1-sc3.

| Run | Output | Task context | Calls | | Run | Output | Task context | Calls |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| kb2 | 3,547 | 18,745 | 9 | | sb1 | 21,507 | 31,547 | 14 |
| kb3 | 2,817 | 23,402 | 10 | | sb2 | 29,238 | 40,033 | 15 |
| kb4 | 3,196 | 24,352 | 11 | | sb3 | 19,829 | 31,465 | 17 |
| kc1 | 2,066 | 14,870 | 9 | | sc1 | 3,498 | 26,740 | 7 |
| kc2 | 1,682 | 13,084 | 7 | | sc2 | 2,977 | 26,431 | 6 |
| kc3 | 2,092 | 13,427 | 9 | | sc3 | 3,502 | 21,201 | 9 |

**An earlier run, kb1, is left out of the means.** It used the phase 7
binary, before the fixes below, and spent 6,830 output tokens and 38,212
task-context tokens (more context than the from-scratch builds). That run
found three gaps:
- `describe_plugin` showed only top-level config keys.
- Nothing documented the `${secret:NAME}` syntax, and wrong forms passed
  validation.
- Provider spellings were never named.

Phase 8 fixed all three. The usage and transcript files for kb1 are still
committed.

## Method

- Every run is a fresh Claude agent with the same model and tools. The
  prompts differ only in the stack.
- **Keel runs** can learn Keel only through `keel mcp` (via a small
  `mcp.py` wrapper) and `keel --help`. They may not read Keel's source code
  or docs.
- **Scratch runs** use Rust with axum, sqlx and redis, and never see Keel.
- **Change runs** each start from a copy of a passing build:
  - Keel runs start from kb2's `app.yaml`;
  - scratch runs start from sb1's code.

  Before each change run, its database was filled by running `acceptance.py`
  against the original app, so the schema has to change in place.
- **Each run is isolated**, with its own Postgres database, Redis database
  and ports.
- **Output tokens** are the tokens the model generated: code, YAML, commands
  and reasoning. They are the main measure.
- **Task context** is the context size at the end of a run minus the size at
  its start. Every run starts with about 51k tokens of fixed system prompt
  and tool definitions, which are the same on both sides and so are left
  out. It counts what the agent read: the spec, tool output, error messages
  and its own messages.
- [benchmark/](benchmark) holds the raw data for every run:
  - `<run>.usage.csv`: per-call API usage (input, cache, output and context
    tokens);
  - `<run>.transcript.md`: a condensed transcript (prompt, every tool call,
    tool results cut at 1,500 characters, secrets and local paths redacted);
  - `summary.json`: the totals per run.

## Limits

- **Three runs per side is a small sample.** Output tokens vary by a few
  thousand between runs, but that is far less than the build gap.
- **The model already knows axum, sqlx and redis well**, while Keel is new
  to it and has to be learned through MCP each time. Keel's advantage would
  likely grow once a model has seen Keel in training. It would shrink if the
  scratch side could reuse a starter template.
- **The change task is small.** Both sides changed only a few lines: about
  12 lines of YAML against about 55 lines of Rust. Most of the change saving
  comes from reading less: the Keel side reads short plugin descriptions,
  while the scratch side reads its own source files.
- **The app is small.** On a larger app, the scratch side has to write and
  later re-read more code per feature. The Keel side's YAML grows by a few
  lines per resource or role.
- **The two apps are not equal.** The from-scratch app has none of the
  following, which the Keel app gets without extra tokens:
  - live config changes;
  - zero-downtime binary swaps;
  - an audit log;
  - MFA;
  - setup tracking;
  - per-route guards.

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
changes. In the phase 8 demo, 0 of 116k requests failed.
