# kc1: change, keel

## Prompt

You are changing a backend app for a measured experiment. Work only inside <exp>/kc1.

The folder holds a working Team Tasks app built with the Keel framework: app.yaml (port 18115) and a prebuilt ./keel binary. It already passes acceptance.py (requirements in SPEC.md), and its Postgres database already holds data from earlier runs. Your job: implement the change request in CHANGE.md without losing that data.

Rules:
- Learn Keel only through its MCP server: `python3 ./mcp.py list` shows the tools, `python3 ./mcp.py <tool> '<json args>'` calls one. Also allowed: `./keel --help`, `./keel <command> --help`, and running keel commands such as `./keel config validate -c app.yaml` or `./keel migrate -c app.yaml`. Do NOT read Keel source code, README or docs elsewhere on disk (do not look in /home/user/keel or any other folder outside kc1). This rule is the point of the experiment.
- Do not change .env, SPEC.md, CHANGE.md or the acceptance scripts. Do not drop or recreate the database.
- Run the server with `./keel serve -c app.yaml` in the background; stop only your own server when done. Other experiments run in parallel on other ports; leave their processes alone.
- Done means both pass: `python3 acceptance_change.py http://127.0.0.1:18115` and `python3 acceptance.py http://127.0.0.1:18115 18215`.

Report back: whether both passed (paste both summary lines), the number of test runs, and any confusing spots. Under 150 words.

## Prompt

<system-reminder>
Your final report is delivered through SubagentHandback: when your work is complete, call SubagentHandback({message: <your full report>}). The call ends your run, so make it your last step. Only a SubagentHandback call reaches your caller as your result; plain text you write at the end is not delivered. Do not also send the report with SendMessage: your caller would get it twice.
</system-reminder>

**Bash**

```
cd <exp>/kc1 && ls -la && cat CHANGE.md && cat app.yaml && ls -R | head -50
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
... (1935 more characters)
```

**Bash**

```
cd <exp>/kc1 && ls -la .keel; python3 ./mcp.py list
```

```text
total 12
drwxr-xr-x 2 root root 4096 Nov 21 20:20 .
drwxr-xr-x 3 root root 4096 Nov 21 20:20 ..
-rw-r--r-- 1 root root 2626 Nov 21 20:20 keel.log
search_plugins: Search the plugins compiled into this Keel build by id, summary and tags (e.g. payments, login, roles, email). Every word must match; empty query lists all. Start here to find what to add instead of writing code.
  args: {"query": {"description": "Words to match, e.g. 'stripe' or 'roles'", "type": "string"}}
describe_plugin: A plugin's config keys with defaults, routes, events, interfaces and setup steps, in about 300 tokens. Read it before adding the plugin to app.yaml.
  args: {"id": {"type": "string"}}
read_config: The current app.yaml text, plus the expanded config with defaults filled in.
  args: {}
validate_config: Check app.yaml (or a proposed new version passed as yaml) against every schema, secret and plugin rule. Returns each problem with its path. Writes nothing.
  args: {"yaml": {"description": "A full proposed app.yaml; omit to check the current file", "type": "string"}}
apply_config: Replace app.yaml with yaml after it passes validation (the old file is kept as .keel/app.yaml.bak). A running `keel dev` reloads it within a second.
  args: {"yaml": {"description": "The full new app.yaml", "type": "string"}}
add_plugin: Add a compiled-in plugin to app.yaml with a minimal valid config, keeping comments.
  args: {"plugin": {"description": "Plugin id, optionally with a version range, e.g. 'rbac' or 'rbac@^0.1
... (1208 more characters)
```

**Bash**

```
cd <exp>/kc1 && python3 ./mcp.py describe_plugin '{"id":"resource"}'; python3 ./mcp.py describe_plugin '{"id":"rbac"}'
```

```text
{
  "ok": true,
  "summary": "resource 0.1.0: REST CRUD from YAML models - tables and migrations, list with filters, sort and paging, get, create, update, delete, per-owner or per-tenant rows, and optional caching.\nneeds: host ^0.1; capabilities: sql, cache, log; plugins: none\napp.yaml providers: db (use: postgres|mysql|sqlite, url); cache (use: redis|memory)\nsecrets in config: write \"${secret:NAME}\" (from env or .env beside app.yaml)\nconfig:\n  resources: object, required, Resource name (plural, e.g. projects) to its model. Served at /api/<name>. Deny by default: every route needs <name>.read, <name>.write or <name>.delete (rbac) unless access says otherwise.\n    <any name>: object\n      access: one of \"permissions\"|\"authenticated\"|\"public\", default \"permissions\", Who may call the routes. permissions (default, needs auth-core and rbac): <name>.read to list and get, <name>.write to create and update, <name>.delete to delete. authenticated: any signed-in user. public: anyone (only routes: rules in app.yaml restrict it).\n      cache_ttl: integer, min 0, default 0, Seconds to cache list and get results; writes clear them. 0 turns caching off.\n      fields: object, required\n        <any name>: object\n          default: any, Used on create when the field is left out\n          enum: array\n          filter: boolean, Allow ?field= filters (default true except text and json)\n          max: number\n          max_length: integer, min 1, max 255, string fields; at 
... (3043 more characters)
```

**Bash**

```
cd <exp>/kc1 && cat acceptance_change.py | head -200; cat .keel/keel.log | tail -20
```

```text
#!/usr/bin/env python3
"""Black-box acceptance test for the follow-up change (see CHANGE.md).

Usage: acceptance_change.py BASE_URL

Run against the app after the change. Only the standard library is used.
Exits 0 when every check passes.
"""

import json
import sys
import urllib.error
import urllib.parse
import urllib.request
import uuid

BASE = sys.argv[1].rstrip("/")
RUN = uuid.uuid4().hex[:8]  # unique emails and teams, so reruns work
CHECKS = 0

# ---- HTTP helpers ---------------------------------------------------------


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *a, **k):
        return None


OPENER = urllib.request.build_opener(NoRedirect)


def call(method, path, body=None, token=None, team=None):
    url = path if path.startswith("http") else BASE + path
    data = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request(url, data=data, method=method)
    if data is not None:
        req.add_header("content-type", "application/json")
    if token:
        req.add_header("authorization", "Bearer " + token)
    if team:
        req.add_header("x-tenant-id", team)
    try:
        with OPENER.open(req, timeout=15) as r:
            raw = r.read()
            return r.status, dict(r.headers), (json.loads(raw) if raw else None)
    except urllib.error.HTTPError as e:
        raw = e.read()
        try:
            parsed = json.loads(raw) if raw else None
        except ValueError:
            parse
... (7545 more characters)
```

Now edit app.yaml.

**Bash**

```
cd <exp>/kc1 && python3 - <<'EOF'
s=open('app.yaml').read()
s=s.replace('''        owner: { permissions: ["projects.*", "tasks.*", "rbac.manage"] }
        member: { permissions: ["projects.read", "tasks.read", "tasks.write"] }''','''        owner: { permissions: ["projects.*", "tasks.*", "comments.*", "rbac.manage"] }
        member: { permissions: ["projects.read", "tasks.read", "tasks.write", "comments.read", "comments.write"] }
        viewer: { permissions: ["projects.read", "tasks.read", "comments.read"] }''')
s=s.replace('''            project_id: { type: string }''','''            project_id: { type: string }
            priority: { type: integer, min: 1, max: 5 }
        comments:
          scope: tenant
          cache_ttl: 60
          fields:
            task_id: { type: string, required: true }
            body: { type: string, required: true, min_length: 1, max_length: 250 }''')
open('app.yaml','w').write(s)
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
cd <exp>/kc1 && python3 ./mcp.py migrations '{"apply":true}' | grep -E '"applied"|blocked|ok' | head; (nohup ./keel serve -c app.yaml > <session> 2>&1 & echo $! > <session> for i in $(seq 20); do curl -s 127.0.0.1:18115/health >/dev/null && break; python3 -c 'import time;time.sleep(0.5)'; done; curl -s 127.0.0.1:18115/health | head -c 400
```

```text
  "applied": [
  "blocked": [],
  "ok": true,
        "state": "applied",
          "auth_core_tokens",
        "state": "applied",
        "state": "applied",
        "state": "applied",
        "state": "applied",
        "state": "applied",
{"app":"team-tasks","plugins":[{"degrade_level":"error","id":"auth-core","status":"ok","version":"0.1.0"},{"degrade_level":"warn","id":"auth-oauth","status":"ok","version":"0.1.0"},{"degrade_level":"error","id":"rbac","status":"ok","version":"0.1.0"},{"degrade_level":"error","id":"resource","status":"ok","version":"0.1.0"}],"providers":[{"capability":"cache","degrade_level":"warn","hot_load":true,
```

**Bash**

```
cd <exp>/kc1 && python3 acceptance_change.py http://127.0.0.1:18115 | tail -3; python3 acceptance.py http://127.0.0.1:18115 18215 | tail -3
```

```text
ok   admin reads any team's comments

all 52 checks passed
ok   tasks need a team header -> 400

all 63 checks passed
```

**Bash**

```
kill $(cat <session> sleep 1; ss -ltnp | grep 18115 || echo stopped
```

```text
/bin/bash: line 1: ss: command not found
stopped
```

**SubagentHandback**

```
{
 "message": "Both acceptance tests passed on the first run.\n- `acceptance_change.py`: \"all 52 checks passed\"\n- `acceptance.py`: \"all 63 checks passed\"\n\nTest runs: 2 (each script ran once).\n\nChanges, all in `<exp>/kc1/app.yaml`:\n- **Priority:** tasks got `priority: { type: integer, min: 1, max: 5 }`.\n- **Comments:** a new `comments` resource with `scope: tenant`, `cache_ttl: 60`, a required `task_id` and a required `body` of 1 to 250 characters.\n- **Roles:** a new `viewer` role with `projects.read`, `tasks.read` and `comments.read`. Members also got `comments.read` and `comments.write`, and owners got `comments.*`.\n\nThe config validated. The MCP `migrations` dry run showed only additive steps, so I applied them and kept the existing data.\n\nI stopped my server afterwards; `ss` isn't installed, so I couldn't confirm port 18115 closed.\n\nConfusing spots: almost none. `describe_plugin` doesn't say whether integer `min`/`max` and filters like `?priority.null=false` work. The tests showed they do."
}
```

```text
{"success":true,"message":"Report delivered to your caller."}
```
