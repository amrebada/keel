# kc2: change, keel

## Prompt

You are changing a backend app for a measured experiment. Work only inside <exp>/kc2.

The folder holds a working Team Tasks app built with the Keel framework: app.yaml (port 18116) and a prebuilt ./keel binary. It already passes acceptance.py (requirements in SPEC.md), and its Postgres database already holds data from earlier runs. Your job: implement the change request in CHANGE.md without losing that data.

Rules:
- Learn Keel only through its MCP server: `python3 ./mcp.py list` shows the tools, `python3 ./mcp.py <tool> '<json args>'` calls one. Also allowed: `./keel --help`, `./keel <command> --help`, and running keel commands such as `./keel config validate -c app.yaml` or `./keel migrate -c app.yaml`. Do NOT read Keel source code, README or docs elsewhere on disk (do not look in /home/user/keel or any other folder outside kc2). This rule is the point of the experiment.
- Do not change .env, SPEC.md, CHANGE.md or the acceptance scripts. Do not drop or recreate the database.
- Run the server with `./keel serve -c app.yaml` in the background; stop only your own server when done. Other experiments run in parallel on other ports; leave their processes alone.
- Done means both pass: `python3 acceptance_change.py http://127.0.0.1:18116` and `python3 acceptance.py http://127.0.0.1:18116 18216`.

Report back: whether both passed (paste both summary lines), the number of test runs, and any confusing spots. Under 150 words.

## Prompt

<system-reminder>
Your final report is delivered through SubagentHandback: when your work is complete, call SubagentHandback({message: <your full report>}). The call ends your run, so make it your last step. Only a SubagentHandback call reaches your caller as your result; plain text you write at the end is not delivered. Do not also send the report with SendMessage: your caller would get it twice.
</system-reminder>

**Bash**

```
cd <exp>/kc2 && ls -la && cat CHANGE.md && cat app.yaml && python3 ./mcp.py list
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
cd <exp>/kc2 && python3 ./mcp.py describe_plugin '{"id":"resource"}'; python3 ./mcp.py describe_plugin '{"id":"rbac"}'; cat acceptance_change.py | head -150
```

```text
{
  "ok": true,
  "summary": "resource 0.1.0: REST CRUD from YAML models - tables and migrations, list with filters, sort and paging, get, create, update, delete, per-owner or per-tenant rows, and optional caching.\nneeds: host ^0.1; capabilities: sql, cache, log; plugins: none\napp.yaml providers: db (use: postgres|mysql|sqlite, url); cache (use: redis|memory)\nsecrets in config: write \"${secret:NAME}\" (from env or .env beside app.yaml)\nconfig:\n  resources: object, required, Resource name (plural, e.g. projects) to its model. Served at /api/<name>. Deny by default: every route needs <name>.read, <name>.write or <name>.delete (rbac) unless access says otherwise.\n    <any name>: object\n      access: one of \"permissions\"|\"authenticated\"|\"public\", default \"permissions\", Who may call the routes. permissions (default, needs auth-core and rbac): <name>.read to list and get, <name>.write to create and update, <name>.delete to delete. authenticated: any signed-in user. public: anyone (only routes: rules in app.yaml restrict it).\n      cache_ttl: integer, min 0, default 0, Seconds to cache list and get results; writes clear them. 0 turns caching off.\n      fields: object, required\n        <any name>: object\n          default: any, Used on create when the field is left out\n          enum: array\n          filter: boolean, Allow ?field= filters (default true except text and json)\n          max: number\n          max_length: integer, min 1, max 255, string fields; at 
... (9663 more characters)
```

**Bash**

```
cd <exp>/kc2 && python3 - <<'EOF'
p='app.yaml'; s=open(p).read()
s=s.replace('owner: { permissions: ["projects.*", "tasks.*", "rbac.manage"] }','owner: { permissions: ["projects.*", "tasks.*", "comments.*", "rbac.manage"] }')
s=s.replace('member: { permissions: ["projects.read", "tasks.read", "tasks.write"] }','member: { permissions: ["projects.read", "tasks.read", "tasks.write", "comments.read", "comments.write"] }\n        viewer: { permissions: ["projects.read", "tasks.read", "comments.read"] }')
s=s.replace('            project_id: { type: string }','''            project_id: { type: string }
            priority: { type: integer, min: 1, max: 5 }
        comments:
          scope: tenant
          cache_ttl: 60
          fields:
            task_id: { type: string, required: true }
            body: { type: string, required: true, min_length: 1, max_length: 250 }''')
open(p,'w').write(s)
EOF
./keel config validate -c app.yaml && python3 ./mcp.py migrations '{}'
```

```text
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
          "auth_core_used_refresh"
        ]
      },
      {
        "id": "auth-oauth/001_identities",
        "kind": "additive",
        "plugin": "auth-oauth",
        "state": "applied",
        "tables": [
          "auth_oauth_identities"
        ]
      },
      {
        "id": "rbac/001_init",
        "kind": "additive",
        "plugin": "rbac",
        "state": "applied",
        "tables": [
          "rbac_assignments",
          "rbac_grants"
        ]
      },
      {
        "id": "resource/comments_00_table",
        "kind": "additive",
        "plugin": "resource",
        "state": "pending",
        "tables": [
          "resource_comments"
        ]
      },
      {
        "id": "resource/comments_02_tenant",
        "kind": "additive",
        "plugin": "resource",
        "state": "pending",
        "tables": [
          "resource_comments"
        ]
      }
... (2880 more characters)
```

All additive. Apply and start the server.

**Bash**

```
cd <exp>/kc2 && python3 ./mcp.py migrations '{"apply":true}' | grep -E '"ok"|applied"|blocked' | head; (nohup ./keel serve -c app.yaml > serve.out 2>&1 &) ; sleep 3; python3 acceptance_change.py http://127.0.0.1:18116 | tail -3; python3 acceptance.py http://127.0.0.1:18116 18216 | tail -3
```

```text
  "applied": [
  "blocked": [],
  "ok": true,
        "state": "applied",
        "state": "applied",
        "state": "applied",
        "state": "applied",
        "state": "applied",
        "state": "applied",
        "state": "applied",
ok   admin reads any team's comments

all 52 checks passed
ok   tasks need a team header -> 400

all 63 checks passed
```

**Bash**

```
cd <exp>/kc2 && pkill -f "keel serve -c app.yaml" -U root --cwd . 2>/dev/null; for p in $(pgrep -f "keel serve -c app.yaml"); do [ "$(readlink /proc/$p/cwd)" = "$PWD" ] && kill $p; done; sleep 1; pgrep -af keel | head; rm -f serve.out
```

```text
18205 [keel] <defunct>
18250 ./keel serve -c app.yaml
18292 [keel] <defunct>
```

**Bash**

```
readlink /proc/18250/cwd
```

```text
<exp>/kc3
```

**SubagentHandback**

```
{
 "message": "Both acceptance scripts passed on the first test run:\n- `acceptance_change.py`: \"all 52 checks passed\"\n- `acceptance.py`: \"all 63 checks passed\"\n\nTest runs: 1 of each script.\n\nThe change was three edits to app.yaml:\n- **Priority:** tasks got `priority: { type: integer, min: 1, max: 5 }`.\n- **Comments:** a new tenant-scoped `comments` resource (`cache_ttl` 60) with `task_id` (required string) and `body` (required string, 1 to 250 characters).\n- **Roles:** `owner` got `comments.*`, `member` got `comments.read` and `comments.write`, and a new `viewer` role got `projects.read`, `tasks.read` and `comments.read`.\n\n`keel config validate` passed. The MCP `migrations` dry run showed only additive steps (the new comments table and the priority column), which I then applied with `apply`. No data was dropped.\n\nNothing was confusing: `describe_plugin` for `resource` and `rbac` covered everything I needed.\n\nMy server has stopped. The other `keel serve` still running is from the kc3 experiment, and I left it alone."
}
```

```text
{"success":true,"message":"Report delivered to your caller."}
```
