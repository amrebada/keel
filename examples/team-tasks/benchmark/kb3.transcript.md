# kb3: build, keel

## Prompt

You are building a backend app for a measured experiment. Work only inside <exp>/kb3.

Goal: make the Team Tasks API described in <exp>/kb3/SPEC.md pass its black-box test, using the Keel framework (a prebuilt binary at ./keel). Do not write application code; Keel apps are configured in app.yaml.

Rules:
- Learn Keel only through its MCP server: `python3 ./mcp.py list` shows the tools, `python3 ./mcp.py <tool> '<json args>'` calls one. Also allowed: `./keel --help` and `./keel <command> --help`, and `./keel config validate -c app.yaml`. Do NOT read any Keel source code, README or docs elsewhere on disk (do not look in /home/user/keel or any other directory outside kb3). This rule is the point of the experiment.
- app.yaml currently only has `app` and `server` (port 18111). Keep that port. Secrets come from the provided .env file in that folder (read it to know the names; do not change it).
- Postgres and Redis are already running; the URLs are in .env.
- Run the server with `./keel serve -c app.yaml` (check --help; run it in the background and stop only your own server when done; `./keel migrate --apply -c app.yaml` may be needed first). Other experiments run in parallel on other ports; leave their processes alone.
- Test with `python3 acceptance.py http://127.0.0.1:18111 18211` (it starts its own mock Google on port 18211). Iterate until every check passes. Do not edit acceptance.py or SPEC.md.
- If Keel itself has a real bug that blocks you, stop and report it precisely instead of working around it.

Report back: whether all checks passed (paste the final summary line), the number of test runs, and any confusing spots. Keep the report under 200 words.

## Prompt

<system-reminder>
Your final report is delivered through SubagentHandback: when your work is complete, call SubagentHandback({message: <your full report>}). The call ends your run, so make it your last step. Only a SubagentHandback call reaches your caller as your result; plain text you write at the end is not delivered. Do not also send the report with SendMessage: your caller would get it twice.
</system-reminder>

**Bash**

```
cd <exp>/kb3 && ls -la && cat SPEC.md app.yaml .env && python3 ./mcp.py list
```

```text
total 29160
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
cd <exp>/kb3 && python3 ./mcp.py search_plugins '{}'; ./keel --help
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
cd <exp>/kb3 && for p in auth-core auth-oauth rbac resource; do python3 ./mcp.py describe_plugin "{\"id\":\"$p\"}"; echo ----; done; python3 ./mcp.py read_config '{}'
```

```text
{
  "ok": true,
  "summary": "auth-core 0.1.0: Email + password accounts with Argon2id, email verification, password reset, magic links, JWT access + rotating refresh tokens, sessions and logout everywhere.\nneeds: host ^0.1; capabilities: sql, crypto, email, secrets, log; plugins: none\napp.yaml providers: db (use: postgres|mysql|sqlite, url); email (use: console|smtp|resend|ses)\nsecrets in config: write \"${secret:NAME}\" (from env or .env beside app.yaml)\nconfig:\n  access_ttl_secs: integer, min 60, max 86400, default 900\n  app_url: string, default \"http://localhost:8080\", Base URL used in links sent by email.\n  links: object, Link templates in emails; {token} is replaced. Point them at your frontend if it handles these pages.\n    magic_link: string, default \"{app_url}/auth/magic-link/verify?token={token}\"\n    reset: string, default \"{app_url}/reset-password?token={token}\"\n    verify: string, default \"{app_url}/auth/verify?token={token}\"\n  magic_link: boolean, default true\n  password_min_length: integer, min 8, max 128, default 8\n  refresh_ttl_secs: integer, min 3600, max 31536000, default 2592000\n  require_verified_email: boolean, default true, Refuse password login until the email is verified.\n  signup: boolean, default true, Allow new accounts through POST /auth/signup.\n  token_ttl_secs: integer, min 300, max 604800, default 86400, Lifetime of verification, reset and magic-link tokens (magic links are capped at 15 minutes).\nroutes: POST /auth/signu
... (8267 more characters)
```

**Bash**

```
cd <exp>/kb3 && cp app.yaml <session> for p in auth-core auth-oauth rbac resource; do python3 ./mcp.py add_plugin "{\"plugin\":\"$p\"}"; done; cat app.yaml; python3 ./mcp.py validate_config '{}'
```

```text
{
  "error": {
    "issues": [
      {
        "message": "`auth-core` needs the `sql` capability; add providers.db (e.g. { use: sqlite, url: \"sqlite://app.db\" })",
        "path": "plugins[0].use"
      },
      {
        "message": "`auth-core` needs the `email` capability; add providers.email (e.g. { use: console })",
        "path": "plugins[0].use"
      }
    ],
    "message": "app.yaml plugins: 2 problem(s) found"
  },
  "ok": false
}
{
  "error": {
    "issues": [
      {
        "message": "`auth-oauth` needs the `sql` capability; add providers.db (e.g. { use: sqlite, url: \"sqlite://app.db\" })",
        "path": "plugins[0].use"
      },
      {
        "message": "`auth-oauth` needs the `cache` capability; add providers.cache (e.g. { use: memory })",
        "path": "plugins[0].use"
      },
      {
        "message": "`auth-oauth` needs plugin `auth-core` (^0.1); add it to plugins",
        "path": "plugins[0].use"
      }
    ],
    "message": "app.yaml plugins: 3 problem(s) found"
  },
  "ok": false
}
{
  "error": {
    "issues": [
      {
        "message": "`rbac` needs the `sql` capability; add providers.db (e.g. { use: sqlite, url: \"sqlite://app.db\" })",
        "path": "plugins[0].use"
      },
      {
        "message": "`rbac` needs plugin `auth-core` (^0.1); add it to plugins",
        "path": "plugins[0].use"
      }
    ],
    "message": "app.yaml plugins: 2 problem(s) found"
  },
  "ok": false
}
{
  "error": {
    "issues": [
      {
        "mess
... (669 more characters)
```

**Bash**

```
cd <exp>/kb3 && cat acceptance.py
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
cd <exp>/kb3 && cat > app.yaml <<'EOF'
app: team-tasks
server: { host: 127.0.0.1, port: 18111 }

providers:
  db: { use: postgres, url: "${secret:DATABASE_URL}" }
  cache: { use: redis, url: "${secret:REDIS_URL}" }
  email: { use: console }

plugins:
  - use: auth-core
    config:
      require_verified_email: false
      app_url: "${secret:PUBLIC_URL}"
  - use: auth-oauth
    config:
      redirect_base_url: "${secret:PUBLIC_URL}"
      providers:
        google:
          auth_url: "${secret:GOOGLE_AUTH_URL}"
          token_url: "${secret:GOOGLE_TOKEN_URL}"
          userinfo_url: "${secret:GOOGLE_USERINFO_URL}"
  - use: rbac
    config:
      admins: [boss@example.com]
      default_role: null
      roles:
        admin: { permissions: ["*"] }
        owner: { permissions: ["projects.*", "tasks.*", "rbac.manage"] }
        member: { permissions: ["projects.read", "tasks.read", "tasks.write"] }
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
EOF
./keel config validate -c app.yaml; python3 ./mcp.py setup_status '{}'
```

```text
ok: app.yaml is valid (app 'team-tasks', 3 provider(s), 4 plugin(s), 0 route rule(s))
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
      "title": "Apple Developer account with a Sign in with Apple key"
    },
    {
      "id": "auth-oauth/facebook-app",
      "kind": "external-account",
      "plugin": "auth-oauth",
      "reason": "`config.providers.facebook` is not set",
      "status": "skipped",
      "title": "Create a Facebook app with Facebook Login"
    },
    {
      "id": "auth-oauth/linkedin-app",
      "kind": "external-account",
      "plugin": "auth-oauth",
      "reason": "`config.providers.linkedin` is not set",
      "status": "skipped",
      "title": "Create a LinkedIn app with Sign In with LinkedIn using OpenID Connect"
... (756 more characters)
```

**Bash**

```
cd <exp>/kb3 && ./keel migrate --apply -c app.yaml 2>&1 | tail -20; ./keel serve --help | head -30
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
cd <exp>/kb3 && (nohup ./keel serve -c app.yaml > <session> 2>&1 & echo $! > <session> for i in 1 2 3 4 5 6 7 8 9 10; do curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:18111/health && break; python3 -c 'import time;time.sleep(1)'; done; echo; python3 acceptance.py http://127.0.0.1:18111 18211
```

```text
200
ok   health -> 200
ok   signup boss -> 201
ok   login boss -> 200
ok   me boss -> 200
ok   me returns boss's email
ok   signup alice -> 201
ok   login alice -> 200
ok   me alice -> 200
ok   me returns alice's email
ok   signup bob -> 201
ok   login bob -> 200
ok   me bob -> 200
ok   me returns bob's email
ok   signup carol -> 201
ok   login carol -> 200
ok   me carol -> 200
ok   me returns carol's email
ok   duplicate signup -> 409
ok   wrong password -> 401
ok   bad token -> 401
ok   google start redirects -> 302
ok   redirect goes to the provider
ok   redirect_uri is the callback
ok   state is sent
ok   google callback signs in -> 200
ok   google user has the provider's email
ok   a callback cannot be replayed
ok   a forged state is refused
ok   admin makes alice owner of acme -> 201
ok   owner alice adds bob to acme -> 201
ok   member bob cannot grant roles -> 403
ok   admin makes carol owner of globex -> 201
ok   projects need a token -> 401
ok   member cannot create a project -> 403
ok   owner creates a project -> 201
ok   project has id and name
ok   project without a name -> 422
ok   member lists projects -> 200
ok   list has the project
ok   another team sees no projects
ok   carol has no role in acme -> 403
ok   admin reads any team -> 200
ok   a new google user has no team role -> 403
ok   member creates task 'Write copy' -> 201
ok   member creates task 'Ship site' -> 201
ok   member creates task 'Plan party' -> 201
ok   task with defaults -> 201
ok   status def
... (543 more characters)
```

**Bash**

```
kill $(cat <session> sleep 1; curl -s -o /dev/null -w '%{http_code}\n' http://127.0.0.1:18111/health
```

```text
Exit code 7
000
```

**SubagentHandback**

```
{
 "message": "All checks passed on the first test run (1 run in total). Final summary line: `all 64 checks passed`\n\nI changed only app.yaml. It now has:\n- **Providers:** postgres, redis and console email, all pointing at the URLs in .env.\n- **auth-core:** `require_verified_email: false`.\n- **auth-oauth:** Google, with the auth, token and userinfo URLs overridden from .env and `redirect_base_url` set to PUBLIC_URL.\n- **rbac:** boss@example.com is the admin, `default_role: null`, and three roles:\n  - admin: `*`\n  - owner: `projects.*`, `tasks.*`, `rbac.manage`\n  - member: `projects.read`, `tasks.read`, `tasks.write`\n- **resource:** `projects` and `tasks` with `scope: tenant`, `cache_ttl: 60`, and the field rules from SPEC.md.\n\nBefore starting the server I ran `keel migrate --apply`. My server is now stopped.\n\nConfusing spots (all minor):\n- No plugin description shows the top-level app.yaml layout (`plugins: [{use, config}]`, `providers.db/cache/email`). I learned it from the error messages `add_plugin` gave.\n- auth-core requires an email provider even though the spec has no email. I used `console`.\n- Nothing says that `OAUTH_GOOGLE_CLIENT_ID`/`OAUTH_GOOGLE_CLIENT_SECRET` and `AUTH_JWT_SECRET` are read from the environment automatically. Only `setup_status` showing them as \"done\" told me no config keys were needed.\n- Two defaults would break this spec without an override: `default_role` is \"member\" and `require_verified_email` is true.\n\nI found no Keel bugs."
}
```

```text
{"success":true,"message":"Report delivered to your caller."}
```
