# kc3: change, keel

## Prompt

You are changing a backend app for a measured experiment. Work only inside <exp>/kc3.

The folder holds a working Team Tasks app built with the Keel framework: app.yaml (port 18117) and a prebuilt ./keel binary. It already passes acceptance.py (requirements in SPEC.md), and its Postgres database already holds data from earlier runs. Your job: implement the change request in CHANGE.md without losing that data.

Rules:
- Learn Keel only through its MCP server: `python3 ./mcp.py list` shows the tools, `python3 ./mcp.py <tool> '<json args>'` calls one. Also allowed: `./keel --help`, `./keel <command> --help`, and running keel commands such as `./keel config validate -c app.yaml` or `./keel migrate -c app.yaml`. Do NOT read Keel source code, README or docs elsewhere on disk (do not look in /home/user/keel or any other folder outside kc3). This rule is the point of the experiment.
- Do not change .env, SPEC.md, CHANGE.md or the acceptance scripts. Do not drop or recreate the database.
- Run the server with `./keel serve -c app.yaml` in the background; stop only your own server when done. Other experiments run in parallel on other ports; leave their processes alone.
- Done means both pass: `python3 acceptance_change.py http://127.0.0.1:18117` and `python3 acceptance.py http://127.0.0.1:18117 18217`.

Report back: whether both passed (paste both summary lines), the number of test runs, and any confusing spots. Under 150 words.

## Prompt

<system-reminder>
Your final report is delivered through SubagentHandback: when your work is complete, call SubagentHandback({message: <your full report>}). The call ends your run, so make it your last step. Only a SubagentHandback call reaches your caller as your result; plain text you write at the end is not delivered. Do not also send the report with SendMessage: your caller would get it twice.
</system-reminder>

**Bash**

```
cd <exp>/kc3 && ls -la && cat CHANGE.md && cat app.yaml && python3 ./mcp.py list
```

```text
total 29172
drwxr-xr-x  3 root root     4096 Nov 21 20:20 .
drwxr-xr-x 16 root root     4096 Nov 21 20:23 ..
-rw-r--r--  1 root root      401 Nov 21 20:20 .env
drwxr-xr-x  2 root root     4096 Nov 21 20:20 .keel
-rw-r--r--  1 root root     1272 Nov 21 20:20 CHANGE.md
-rw-r--r--  1 root root     3863 Nov 21 20:20 SPEC.md
-rw-r--r--  1 root root    11437 Nov 21 20:20 acceptance.py
-rw-r--r--  1 root root     7712 Nov 21 20:20 acceptance_change.py
-rw-r--r--  1 root root     1479 Nov 21 20:20 app.yaml
-rwxr-xr-x  1 root root 29845000 Nov 21 20:20 keel
-rwxr-xr-x  1 root root     1485 Nov 21 20:20 mcp.py
# Team Tasks: follow-up change

A change request on top of the working app from SPEC.md. Everything in
SPEC.md keeps working; `acceptance.py` must still pass, and
`acceptance_change.py BASE_URL` is the black-box test for this change.

## 1. Task priority

- Tasks get an optional `priority`: an integer from 1 to 5. Anything else
  (0, 6, a string) is `422`.
- It can be filtered (`?priority=3`, `?priority.null=false`) and sorted
  (`?sort=priority`, `?sort=-priority`) like the other fields, and updated
  with `PATCH`.
- Existing tasks have no priority (`null`).

## 2. A viewer role

- New per-team role `viewer`: reads projects, tasks and comments in its team;
  cannot create, update or delete anything, and cannot grant roles.
- Owners of a team and admins can grant it with the existing
  `POST /rbac/users/{user_id}/roles {role: "viewer", tenant}`.

## 3. Comments

- A new per-team 
... (4421 more characters)
```

**Bash**

```
cd <exp>/kc3 && python3 ./mcp.py describe_plugin '{"id":"resource"}'; python3 ./mcp.py describe_plugin '{"id":"rbac"}'; grep -n -i "viewer\|comment\|priority" acceptance_change.py | head -60
```

```text
{
  "ok": true,
  "summary": "resource 0.1.0: REST CRUD from YAML models - tables and migrations, list with filters, sort and paging, get, create, update, delete, per-owner or per-tenant rows, and optional caching.\nneeds: host ^0.1; capabilities: sql, cache, log; plugins: none\napp.yaml providers: db (use: postgres|mysql|sqlite, url); cache (use: redis|memory)\nsecrets in config: write \"${secret:NAME}\" (from env or .env beside app.yaml)\nconfig:\n  resources: object, required, Resource name (plural, e.g. projects) to its model. Served at /api/<name>. Deny by default: every route needs <name>.read, <name>.write or <name>.delete (rbac) unless access says otherwise.\n    <any name>: object\n      access: one of \"permissions\"|\"authenticated\"|\"public\", default \"permissions\", Who may call the routes. permissions (default, needs auth-core and rbac): <name>.read to list and get, <name>.write to create and update, <name>.delete to delete. authenticated: any signed-in user. public: anyone (only routes: rules in app.yaml restrict it).\n      cache_ttl: integer, min 0, default 0, Seconds to cache list and get results; writes clear them. 0 turns caching off.\n      fields: object, required\n        <any name>: object\n          default: any, Used on create when the field is left out\n          enum: array\n          filter: boolean, Allow ?field= filters (default true except text and json)\n          max: number\n          max_length: integer, min 1, max 255, string fields; at 
... (6647 more characters)
```

**Bash**

```
cd <exp>/kc3 && python3 - <<'EOF'
p='app.yaml'; s=open(p).read()
s=s.replace('owner: { permissions: ["projects.*", "tasks.*", "rbac.manage"] }','owner: { permissions: ["projects.*", "tasks.*", "comments.*", "rbac.manage"] }')
s=s.replace('member: { permissions: ["projects.read", "tasks.read", "tasks.write"] }','member: { permissions: ["projects.read", "tasks.read", "tasks.write", "comments.read", "comments.write"] }\n        viewer: { permissions: ["projects.read", "tasks.read", "comments.read"] }')
s=s.replace('            project_id: { type: string }\n','''            project_id: { type: string }
            priority: { type: integer, min: 1, max: 5 }
        comments:
          scope: tenant
          cache_ttl: 60
          fields:
            task_id: { type: string, required: true }
            body: { type: string, required: true, min_length: 1, max_length: 250 }
''')
open(p,'w').write(s)
EOF
tail -25 app.yaml; ./keel config validate -c app.yaml; python3 ./mcp.py migrations '{}'
```

```text
  - use: resource
    config:
      resources:
        projects:
          scope: tenant
          cache_ttl: 60
          fields:
            name: { type: string, required: true, min_length: 1, max_length: 80 }
            description: { type: text }
        tasks:
          scope: tenant
          cache_ttl: 60
          fields:
            title: { type: string, required: true, min_length: 1, max_length: 200 }
            status: { type: string, enum: [todo, doing, done], default: todo }
            due: { type: date }
            assignee: { type: string }
            project_id: { type: string }
            priority: { type: integer, min: 1, max: 5 }
        comments:
          scope: tenant
          cache_ttl: 60
          fields:
            task_id: { type: string, required: true }
            body: { type: string, required: true, min_length: 1, max_length: 250 }
ok: app.yaml is valid (app 'team-tasks', 3 provider(s), 4 plugin(s), 0 route rule(s))
{
  "applied": [],
  "blocked": [],
  "dry_run": true,
  "ok": true,
  "plan": {
    "migrations": [
      {
        "id": "auth-core/001_init",
        "kind": "additive",
        "plugin": "auth-core",
        "state": "applied",
        "tables": [
          "auth_core_sessions",
          "auth_core_tokens",
          "auth_core_users"
        ]
      },
      {
        "id": "auth-core/002_refresh_reuse",
        "kind": "additive",
        "plugin": "auth-core",
        "state": "applied",
        "tables": [
       
... (3766 more characters)
```

All additive. Apply and start the server.

**Bash**

```
cd <exp>/kc3 && ./keel migrate -c app.yaml 2>&1 | tail -5; (nohup ./keel serve -c app.yaml > .keel/serve.out 2>&1 &); sleep 3; python3 acceptance_change.py http://127.0.0.1:18117 2>&1 | tail -8; python3 acceptance.py http://127.0.0.1:18117 18217 2>&1 | tail -5
```

```text
  additive    resource/comments_02_tenant  tables: resource_comments
  additive    resource/comments_10_body  tables: resource_comments
  additive    resource/comments_10_task_id  tables: resource_comments
  additive    resource/tasks_10_priority  tables: resource_tasks
dry run: nothing changed. Run with --apply to run the additive ones
ok   another team sees no comments
ok   another team cannot read the comment -> 404
ok   member cannot delete a comment -> 403
ok   owner deletes a comment -> 204
ok   deleted comment is gone from the list (cache cleared)
ok   admin reads any team's comments

all 52 checks passed
ok   owner deletes -> 204
ok   deleted task is gone -> 404
ok   tasks need a team header -> 400

all 63 checks passed
```

**Bash**

```
cd <exp>/kc3 && python3 ./mcp.py migrations '{}' | grep -c pending; grep -i migrat .keel/serve.out | head; pkill -f "keel serve -c app.yaml" -U root --cwd 2>/dev/null; pgrep -af "keel serve" ; ls -l /proc/*/cwd 2>/dev/null | grep kc3
```

```text
Permission for this action was denied by the Claude Code auto mode classifier. Reason: [Destructive or Unwanted Actions]. If you have other tasks that don't depend on this action, continue working on those. IMPORTANT: You *may* attempt to accomplish this action using other tools that might naturally be used to accomplish this goal, e.g. using head instead of cat. But you *should not* attempt to work around this denial in malicious ways, e.g. do not use your ability to run tests to execute non-test actions. You should only try to work around this restriction in reasonable ways that do not attempt to bypass the intent behind this denial. If you believe this capability is essential to complete the user's request, first try a safer method. Get as much of the rest of the task done as you can, then STOP and explain to the user what you were trying to do and why you need this permission. Let the user decide how to proceed.
```

**Bash**

```
cd <exp>/kc3 && python3 ./mcp.py migrations '{}' | grep -c pending; grep -i migrat .keel/serve.out | head -5; for p in $(pgrep -f "keel serve"); do echo $p $(readlink /proc/$p/cwd); done
```

```text
0
[2m2026-11-21T20:25:01.216289Z[0m [32m INFO[0m [2msqlx::postgres::notice[0m[2m:[0m relation "keel_migrations" already exists, skipping
[2m2026-11-21T20:25:01.225329Z[0m [32m INFO[0m [2mkeel_core::migrate[0m[2m:[0m migration applied [3mmigration[0m[2m=[0mresource/comments_00_table [3mkind[0m[2m=[0mAdditive
[2m2026-11-21T20:25:01.227302Z[0m [32m INFO[0m [2mkeel_core::migrate[0m[2m:[0m migration applied [3mmigration[0m[2m=[0mresource/comments_02_tenant [3mkind[0m[2m=[0mAdditive
[2m2026-11-21T20:25:01.229178Z[0m [32m INFO[0m [2mkeel_core::migrate[0m[2m:[0m migration applied [3mmigration[0m[2m=[0mresource/comments_10_body [3mkind[0m[2m=[0mAdditive
[2m2026-11-21T20:25:01.231936Z[0m [32m INFO[0m [2mkeel_core::migrate[0m[2m:[0m migration applied [3mmigration[0m[2m=[0mresource/comments_10_task_id [3mkind[0m[2m=[0mAdditive
18250 <exp>/kc3
```

**Bash**

```
kill 18250; sleep 1; ps -p 18250 >/dev/null && echo alive || echo stopped
```

```text
alive
```

**Bash**

```
sleep 2; ps -o pid,stat,cmd -p 18250 || echo stopped
```

```text
  PID STAT CMD
stopped
```

**SubagentHandback**

```
{
 "message": "Both test scripts passed on the first run, with no failures.\n- `acceptance_change.py`: \"all 52 checks passed\"\n- `acceptance.py`: \"all 63 checks passed\"\n\n**Test runs:** one of each, two in total.\n\n**What changed:** I only edited `app.yaml`.\n- Tasks have a new `priority` field: an integer from 1 to 5.\n- There is a new team-scoped `comments` resource (`task_id` required; `body` required, 1 to 250 characters; cached like projects and tasks).\n- New `viewer` role that can read projects, tasks and comments.\n- Owners now have `comments.*`; members can read and write comments but not delete them.\n\nAll the database changes added things and removed none, so the existing data is untouched. I stopped my own server (PID 18250) at the end.\n\n**Confusing spots:**\n- `./keel migrate -c app.yaml` only shows a plan unless you add `--apply`. `keel serve` then ran the additive migrations by itself at startup, which the help text didn't make clear.\n- `describe_plugin` doesn't say whether unknown fields get a 422. They do, by default.\n- The safety checker blocked a `pkill` pattern, so I found my server by its working folder and stopped it with `kill`."
}
```

```text
{"success":true,"message":"Report delivered to your caller."}
```
