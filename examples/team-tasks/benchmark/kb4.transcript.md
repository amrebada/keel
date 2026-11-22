# kb4: build, keel

## Prompt

You are building a backend app for a measured experiment. Work only inside <exp>/kb4.

Goal: make the Team Tasks API described in <exp>/kb4/SPEC.md pass its black-box test, using the Keel framework (a prebuilt binary at ./keel). Do not write application code; Keel apps are configured in app.yaml.

Rules:
- Learn Keel only through its MCP server: `python3 ./mcp.py list` shows the tools, `python3 ./mcp.py <tool> '<json args>'` calls one. Also allowed: `./keel --help` and `./keel <command> --help`, and `./keel config validate -c app.yaml`. Do NOT read any Keel source code, README or docs elsewhere on disk (do not look in /home/user/keel or any other directory outside kb4). This rule is the point of the experiment.
- app.yaml currently only has `app` and `server` (port 18112). Keep that port. Secrets come from the provided .env file in that folder (read it to know the names; do not change it).
- Postgres and Redis are already running; the URLs are in .env.
- Run the server with `./keel serve -c app.yaml` (check --help; run it in the background and stop only your own server when done; `./keel migrate --apply -c app.yaml` may be needed first). Other experiments run in parallel on other ports; leave their processes alone.
- Test with `python3 acceptance.py http://127.0.0.1:18112 18212` (it starts its own mock Google on port 18212). Iterate until every check passes. Do not edit acceptance.py or SPEC.md.
- If Keel itself has a real bug that blocks you, stop and report it precisely instead of working around it.

Report back: whether all checks passed (paste the final summary line), the number of test runs, and any confusing spots. Keep the report under 200 words.

## Prompt

<system-reminder>
Your final report is delivered through SubagentHandback: when your work is complete, call SubagentHandback({message: <your full report>}). The call ends your run, so make it your last step. Only a SubagentHandback call reaches your caller as your result; plain text you write at the end is not delivered. Do not also send the report with SendMessage: your caller would get it twice.
</system-reminder>

**Bash**

```
cd <exp>/kb4 && ls -la && cat SPEC.md && cat app.yaml && cat .env && python3 ./mcp.py list
```

```text
total 29156
drwxr-xr-x  2 root root     4096 Nov 21 20:20 .
drwxr-xr-x 16 root root     4096 Nov 21 20:20 ..
-rw-r--r--  1 root root      400 Nov 21 20:20 .env
-rw-r--r--  1 root root     3863 Nov 21 20:20 SPEC.md
-rw-r--r--  1 root root    11437 Nov 21 20:20 acceptance.py
-rw-r--r--  1 root root       57 Nov 21 20:20 app.yaml
-rwxr-xr-x  1 root root 29845000 Nov 21 20:20 keel
-rwxr-xr-x  1 root root     1485 Nov 21 20:20 mcp.py
# Team Tasks API: requirements

The v1 success-test app. The same document is handed to an AI building it
with Keel and to an AI building it from scratch; `acceptance.py` is the
black-box test both must pass.

## Stack

- Postgres for data (`DATABASE_URL`), Redis for caching (`REDIS_URL`).
- Listens on the host and port given to you.
- Secrets and settings come from environment variables (a `.env` file is
  provided): `DATABASE_URL`, `REDIS_URL`, `PUBLIC_URL` (the app's own base
  URL), `AUTH_JWT_SECRET`, `OAUTH_GOOGLE_CLIENT_ID`,
  `OAUTH_GOOGLE_CLIENT_SECRET`, and `GOOGLE_AUTH_URL`, `GOOGLE_TOKEN_URL`,
  `GOOGLE_USERINFO_URL` (the Google endpoints; the test points them at a mock).

## Accounts

- `POST /auth/signup {email, password}`: `201` with `{ user: { id, email }, ... }`;
  `409` when the email is taken. No email verification is required to log in.
- `POST /auth/login {email, password}`: `200` with `{ access_token, refresh_token, ... }`;
  `401` on a wrong password.
- `GET /auth/me` with `Authorization: Bearer <access_token>`: `200` with
  `{ u
... (5766 more characters)
```

**Bash**

```
cd <exp>/kb4 && python3 ./mcp.py search_plugins '{"query":""}'; ./keel --help
```

```text
{
  "plugins": [
    {
      "capabilities": [
        "sql",
        "crypto",
        "email",
        "secrets",
        "log"
      ],
      "id": "auth-core",
      "routes": [
        "POST /auth/signup",
        "POST /auth/login",
        "POST /auth/refresh",
        "POST /auth/logout",
        "POST /auth/logout-all",
        "GET /auth/me",
        "GET /auth/verify",
        "POST /auth/verify",
        "POST /auth/verify/resend",
        "POST /auth/password/forgot",
        "POST /auth/password/reset",
        "POST /auth/magic-link",
        "GET /auth/magic-link/verify",
        "POST /auth/magic-link/verify"
      ],
      "summary": "Email + password accounts with Argon2id, email verification, password reset, magic links, JWT access + rotating refresh tokens, sessions and logout everywhere.",
      "tags": [
        "auth",
        "login",
        "signup",
        "password",
        "accounts",
        "users",
        "jwt",
        "sessions",
        "email-verification",
        "magic-link"
      ],
      "version": "0.1.0"
    },
    {
      "capabilities": [
        "sql",
        "crypto",
        "log"
      ],
      "id": "auth-mfa",
      "routes": [
        "GET /auth/mfa/status",
        "POST /auth/mfa/totp/setup",
        "POST /auth/mfa/totp/confirm",
        "POST /auth/mfa/verify",
        "POST /auth/mfa/disable",
        "POST /auth/mfa/recovery-codes"
      ],
      "summary": "Second factor for auth-core logins - TOTP authenticator 
... (6142 more characters)
```

**Bash**

```
cd <exp>/kb4 && for p in auth-core auth-oauth rbac resource saas-auth; do echo "== $p"; python3 ./mcp.py describe_plugin "{\"id\":\"$p\"}"; done; python3 ./mcp.py read_config '{}'
```

```text
== auth-core
{
  "ok": true,
  "summary": "auth-core 0.1.0: Email + password accounts with Argon2id, email verification, password reset, magic links, JWT access + rotating refresh tokens, sessions and logout everywhere.\nneeds: host ^0.1; capabilities: sql, crypto, email, secrets, log; plugins: none\napp.yaml providers: db (use: postgres|mysql|sqlite, url); email (use: console|smtp|resend|ses)\nsecrets in config: write \"${secret:NAME}\" (from env or .env beside app.yaml)\nconfig:\n  access_ttl_secs: integer, min 60, max 86400, default 900\n  app_url: string, default \"http://localhost:8080\", Base URL used in links sent by email.\n  links: object, Link templates in emails; {token} is replaced. Point them at your frontend if it handles these pages.\n    magic_link: string, default \"{app_url}/auth/magic-link/verify?token={token}\"\n    reset: string, default \"{app_url}/reset-password?token={token}\"\n    verify: string, default \"{app_url}/auth/verify?token={token}\"\n  magic_link: boolean, default true\n  password_min_length: integer, min 8, max 128, default 8\n  refresh_ttl_secs: integer, min 3600, max 31536000, default 2592000\n  require_verified_email: boolean, default true, Refuse password login until the email is verified.\n  signup: boolean, default true, Allow new accounts through POST /auth/signup.\n  token_ttl_secs: integer, min 300, max 604800, default 86400, Lifetime of verification, reset and magic-link tokens (magic links are capped at 15 minutes).\nroutes: POS
... (10101 more characters)
```

**Bash**

```
cd <exp>/kb4 && cat acceptance.py
```

```text
#!/usr/bin/env python3
"""Black-box acceptance test for the Team Tasks API (see SPEC.md).

Usage: acceptance.py BASE_URL MOCK_PORT

BASE_URL   where the app listens, e.g. http://127.0.0.1:8080
MOCK_PORT  port for the mock Google provider this script runs. Start the app
           with GOOGLE_AUTH_URL=http://127.0.0.1:MOCK_PORT/authorize,
           GOOGLE_TOKEN_URL=<redacted>
           GOOGLE_USERINFO_URL=http://127.0.0.1:MOCK_PORT/userinfo,
           OAUTH_GOOGLE_CLIENT_ID=client-tt, OAUTH_GOOGLE_CLIENT_SECRET=<redacted>

Only the standard library is used. Exits 0 when every check passes.
"""

import json
import sys
import threading
import urllib.error
import urllib.parse
import urllib.request
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

BASE = sys.argv[1].rstrip("/")
MOCK_PORT = int(sys.argv[2])
RUN = uuid.uuid4().hex[:8]  # unique emails and teams, so reruns work
CHECKS = 0

# ---- mock Google ----------------------------------------------------------

CODES = {}
TOKENS = <redacted>
PROFILE = {"sub": "g-" + RUN, "email": f"gina-{RUN}@example.com", "email_verified": True, "name": "Gina"}


class Mock(BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def reply(self, status, body):
        data = json.dumps(body).encode()
        self.send_response(status)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(data)))
        self.end_headers()
        self.w
... (9925 more characters)
```

**Bash**

```
cd <exp>/kb4 && cat > <session> <<'EOF'
app: team-tasks
server: { host: 127.0.0.1, port: 18112 }
providers:
  db: { use: postgres, url: "${secret:DATABASE_URL}" }
  cache: { use: redis, url: "${secret:REDIS_URL}" }
plugins:
  - id: auth-core
    config:
      app_url: "${secret:PUBLIC_URL}"
      require_verified_email: false
  - id: auth-oauth
    config:
      redirect_base_url: "${secret:PUBLIC_URL}"
      providers:
        google:
          auth_url: "${secret:GOOGLE_AUTH_URL}"
          token_url: "${secret:GOOGLE_TOKEN_URL}"
          userinfo_url: "${secret:GOOGLE_USERINFO_URL}"
  - id: rbac
    config:
      admins: [boss@example.com]
      default_role: null
      roles:
        admin: { permissions: ["*"] }
        owner: { permissions: ["projects.*", "tasks.*", "rbac.manage"] }
        member: { permissions: ["projects.read", "tasks.read", "tasks.write"] }
  - id: resource
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
EOF
python3 - <<'EOF'
import json,subprocess
y=open('<session>
print(subprocess.run(['python3','./mcp.py','validate_config',json.dumps({"yaml":y})],capture_output=True,text=True).stdout)
EOF
```

```text
{
  "error": {
    "issues": [
      {
        "message": "\"use\" is a required property",
        "path": "plugins[0]"
      },
      {
        "message": "Additional properties are not allowed ('id' was unexpected)",
        "path": "plugins[0]"
      },
      {
        "message": "\"use\" is a required property",
        "path": "plugins[1]"
      },
      {
        "message": "Additional properties are not allowed ('id' was unexpected)",
        "path": "plugins[1]"
      },
      {
        "message": "\"use\" is a required property",
        "path": "plugins[2]"
      },
      {
        "message": "Additional properties are not allowed ('id' was unexpected)",
        "path": "plugins[2]"
      },
      {
        "message": "\"use\" is a required property",
        "path": "plugins[3]"
      },
      {
        "message": "Additional properties are not allowed ('id' was unexpected)",
        "path": "plugins[3]"
      }
    ],
    "message": "<exp>/kb4/.keel-candidate.yaml: 8 problem(s) found"
  },
  "ok": false
}
```

**Bash**

```
cd <exp>/kb4 && S=<session> sed -i 's/  - id: /  - use: /' $S/app.yaml && cat > $S/v.py <<'EOF'
import json,subprocess,sys
y=open('<session>
print(subprocess.run(['python3','./mcp.py',sys.argv[1],json.dumps({"yaml":y})],capture_output=True,text=True).stdout)
EOF
python3 $S/v.py validate_config
```

```text
{
  "error": {
    "issues": [
      {
        "message": "`auth-core` needs the `email` capability; add providers.email (e.g. { use: console })",
        "path": "plugins[0].use"
      }
    ],
    "message": "app.yaml plugins: 1 problem(s) found"
  },
  "ok": false
}
```

**Bash**

```
cd <exp>/kb4 && S=<session> sed -i 's/^  cache: { use: redis.*/&\n  email: { use: console }/' $S/app.yaml && python3 $S/v.py validate_config && python3 $S/v.py apply_config && python3 ./mcp.py setup_status '{}'
```

```text
{
  "app": "team-tasks",
  "file": "<exp>/kb4/.keel-candidate.yaml",
  "ok": true,
  "plugins": [
    "auth-core",
    "auth-oauth",
    "rbac",
    "resource"
  ],
  "providers": {
    "cache": "redis",
    "db": "postgres",
    "email": "console"
  }
}

{
  "applied": true,
  "backup": "<exp>/kb4/.keel/app.yaml.bak",
  "file": "<exp>/kb4/app.yaml",
  "validation": {
    "app": "team-tasks",
    "file": "<exp>/kb4/.keel-candidate.yaml",
    "ok": true,
    "plugins": [
      "auth-core",
      "auth-oauth",
      "rbac",
      "resource"
    ],
    "providers": {
      "cache": "redis",
      "db": "postgres",
      "email": "console"
    }
  }
}

{
  "complete": true,
  "ok": true,
  "steps": [
    {
      "id": "auth-core/jwt-secret",
      "kind": "secret",
      "plugin": "auth-core",
      "secrets": [
        "AUTH_JWT_SECRET"
      ],
      "status": "done",
      "title": "Create the key that signs login tokens"
    },
    {
      "id": "auth-oauth/google-client",
      "kind": "external-account",
      "plugin": "auth-oauth",
      "secrets": [
        "OAUTH_GOOGLE_CLIENT_ID",
        "OAUTH_GOOGLE_CLIENT_SECRET"
      ],
      "status": "done",
      "title": "Create a Google OAuth client"
    },
    {
      "id": "auth-oauth/apple-key",
      "kind": "external-account",
      "plugin": "auth-oauth",
      "reason": "`config.providers.apple` is not set",
      "requires_paid_account": true,
      "status": "skipped",
      "title": "Apple Developer account with a 
... (1327 more characters)
```

**Bash**

```
cd <exp>/kb4 && ./keel migrate --apply -c app.yaml 2>&1 | tail -20; ./keel serve --help
```

```text
  additive    resource/tasks_10_assignee  tables: resource_tasks
  additive    resource/tasks_10_due  tables: resource_tasks
  additive    resource/tasks_10_project_id  tables: resource_tasks
  additive    resource/tasks_10_status  tables: resource_tasks
  additive    resource/tasks_10_title  tables: resource_tasks
applied auth-core/001_init
applied auth-core/002_refresh_reuse
applied auth-oauth/001_identities
applied rbac/001_init
applied resource/projects_00_table
applied resource/projects_02_tenant
applied resource/projects_10_description
applied resource/projects_10_name
applied resource/tasks_00_table
applied resource/tasks_02_tenant
applied resource/tasks_10_assignee
applied resource/tasks_10_due
applied resource/tasks_10_project_id
applied resource/tasks_10_status
applied resource/tasks_10_title
Start the server from app.yaml. Plugins can then change live through `keel apply` and the admin API (needs the KEEL_ADMIN_TOKEN secret)

Usage: keel serve [OPTIONS]

Options:
  -c, --config <CONFIG>  [default: app.yaml]
      --json             Print one JSON object instead of text (for scripts and AI agents)
  -h, --help             Print help
```

**Bash**

```
cd <exp>/kb4 && (nohup ./keel serve -c app.yaml > serve.out 2>&1 & echo $! > serve.pid); for i in $(seq 1 20); do curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:18112/health 2>/dev/null | grep -q 200 && break; python3 -c 'import time;time.sleep(0.5)'; done; tail -5 serve.out; python3 acceptance.py http://127.0.0.1:18112 18212 2>&1 | tail -8
```

```text
[2m2026-11-21T20:24:03.910219Z[0m [32m INFO[0m [2mkeel_core::plugins[0m[2m:[0m plugin started [3mplugin[0m[2m=[0mauth-core [3mversion[0m[2m=[0m0.1.0 [3mroutes[0m[2m=[0m14
[2m2026-11-21T20:24:03.910273Z[0m [32m INFO[0m [2mkeel_core::plugins[0m[2m:[0m plugin started [3mplugin[0m[2m=[0mauth-oauth [3mversion[0m[2m=[0m0.1.0 [3mroutes[0m[2m=[0m4
[2m2026-11-21T20:24:03.911370Z[0m [32m INFO[0m [2mkeel_core::plugins[0m[2m:[0m plugin started [3mplugin[0m[2m=[0mrbac [3mversion[0m[2m=[0m0.1.0 [3mroutes[0m[2m=[0m7
[2m2026-11-21T20:24:03.911610Z[0m [32m INFO[0m [2mkeel_core::plugins[0m[2m:[0m plugin started [3mplugin[0m[2m=[0mresource [3mversion[0m[2m=[0m0.1.0 [3mroutes[0m[2m=[0m6
[2m2026-11-21T20:24:03.912766Z[0m [32m INFO[0m [2mkeel_core::live[0m[2m:[0m keel host listening [3mapp[0m[2m=[0mteam-tasks [3mlocal[0m[2m=[0m127.0.0.1:18112 [3mplugins[0m[2m=[0m4
ok   lists after a write are fresh
ok   another team cannot see the task -> 404
ok   member cannot delete -> 403
ok   owner deletes -> 204
ok   deleted task is gone -> 404
ok   tasks need a team header -> 400

all 64 checks passed
```

**Bash**

```
cd <exp>/kb4 && kill $(cat serve.pid) && rm -f serve.pid serve.out && ls -a
```

```text
.
..
.env
.keel
SPEC.md
acceptance.py
app.yaml
keel
mcp.py
```

**SubagentHandback**

```
{
 "message": "All checks passed on the first test run. Final line: `all 64 checks passed`. There was 1 test run.\n\nHow I got there: I used the MCP tools search_plugins, describe_plugin, validate_config, apply_config and setup_status. Then I ran `./keel migrate --apply -c app.yaml` and `./keel serve -c app.yaml` in the background. I've stopped that server since.\n\nThe finished <exp>/kb4/app.yaml uses:\n- **Providers:** postgres for the database, redis for the cache, and console for email.\n- **Plugins:** auth-core (with verified email turned off), auth-oauth (Google, with its endpoints pointed at the .env URLs), rbac and resource.\n- **Roles:** admins is set to boss@example.com and there is no default role. owner gets projects.*, tasks.* and rbac.manage; member gets projects.read, tasks.read and tasks.write.\n- **Resources:** projects and tasks are scoped per team (`scope: tenant`) and cached for 60 seconds.\n\nConfusing spots:\n1. Plugin entries are written as `- use: <id>`, not `- id:`. describe_plugin never shows a plugin entry, so I only found this when validation failed.\n2. auth-core needs an email provider (`providers.email: { use: console }`) even though the app sends no email. Validation explained this clearly.\n3. describe_plugin doesn't say which secret signs the login tokens. setup_status showed it reads AUTH_JWT_SECRET on its own.\n4. It wasn't obvious that a team role holding `rbac.manage` lets an owner grant roles in that team. The manage_permission description in rbac covers it, but you have to read it closely.\n\nI found no Keel bugs."
}
```

```text
{"success":true,"message":"Report delivered to your caller."}
```
