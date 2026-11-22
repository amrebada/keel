# kb2: build, keel

## Prompt

You are building a backend app for a measured experiment. Work only inside <exp>/keel-build2.

Goal: make the Team Tasks API described in <exp>/keel-build2/SPEC.md pass its black-box test, using the Keel framework (a prebuilt binary at ./keel). Do not write application code; Keel apps are configured in app.yaml.

Rules:
- Learn Keel only through its MCP server: `python3 ./mcp.py list` shows the tools, `python3 ./mcp.py <tool> '<json args>'` calls one. Also allowed: `./keel --help` and `./keel <command> --help`, and `./keel config validate -c app.yaml`. Do NOT read any Keel source code, README or docs elsewhere on disk (do not look in /home/user/keel, <exp>/keel-build or any other directory outside keel-build2). This rule is the point of the experiment.
- app.yaml currently only has `app` and `server` (port 18103). Keep that port. Secrets come from the provided .env file in that folder (read it to know the names; do not change it).
- Postgres and Redis are already running; the URLs are in .env.
- Run the server with `./keel serve -c app.yaml` (check --help; run it in the background and stop it when done; `./keel migrate --apply -c app.yaml` may be needed first).
- Test with `python3 acceptance.py http://127.0.0.1:18103 18203` (it starts its own mock Google on port 18203). Iterate until every check passes. Do not edit acceptance.py or SPEC.md.
- If Keel itself has a real bug that blocks you, stop and report it precisely instead of working around it.

Report back: whether all checks passed (paste the final summary line), the final app.yaml, the number of iterations, and any confusing spots. Keep the report under 300 words.

## Prompt

<system-reminder>
Your final report is delivered through SubagentHandback: when your work is complete, call SubagentHandback({message: <your full report>}). The call ends your run, so make it your last step. Only a SubagentHandback call reaches your caller as your result; plain text you write at the end is not delivered. Do not also send the report with SendMessage: your caller would get it twice.
</system-reminder>

**Bash**

```
cd <exp>/keel-build2 && ls -la && cat SPEC.md app.yaml .env && python3 ./mcp.py list
```

```text
total 29156
drwxr-xr-x 2 root root     4096 Nov 21 20:10 .
drwxr-xr-x 5 root root     4096 Nov 21 20:10 ..
-rw-r--r-- 1 root root      399 Nov 21 20:10 .env
-rw-r--r-- 1 root root     3863 Nov 21 20:10 SPEC.md
-rw-r--r-- 1 root root    11437 Nov 21 20:10 acceptance.py
-rw-r--r-- 1 root root       57 Nov 21 20:10 app.yaml
-rwxr-xr-x 1 root root 29845000 Nov 21 20:10 keel
-rwxr-xr-x 1 root root     1485 Nov 21 20:10 mcp.py
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
  `{ user: { i
... (5757 more characters)
```

**Bash**

```
cd <exp>/keel-build2 && python3 mcp.py search_plugins '{"query":""}'; ./keel --help
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
cd <exp>/keel-build2 && for p in auth-core auth-oauth rbac resource; do echo "=== $p"; python3 mcp.py describe_plugin "{\"id\":\"$p\"}"; done
```

```text
=== auth-core
{
  "ok": true,
  "summary": "auth-core 0.1.0: Email + password accounts with Argon2id, email verification, password reset, magic links, JWT access + rotating refresh tokens, sessions and logout everywhere.\nneeds: host ^0.1; capabilities: sql, crypto, email, secrets, log; plugins: none\napp.yaml providers: db (use: postgres|mysql|sqlite, url); email (use: console|smtp|resend|ses)\nsecrets in config: write \"${secret:NAME}\" (from env or .env beside app.yaml)\nconfig:\n  access_ttl_secs: integer, min 60, max 86400, default 900\n  app_url: string, default \"http://localhost:8080\", Base URL used in links sent by email.\n  links: object, Link templates in emails; {token} is replaced. Point them at your frontend if it handles these pages.\n    magic_link: string, default \"{app_url}/auth/magic-link/verify?token={token}\"\n    reset: string, default \"{app_url}/reset-password?token={token}\"\n    verify: string, default \"{app_url}/auth/verify?token={token}\"\n  magic_link: boolean, default true\n  password_min_length: integer, min 8, max 128, default 8\n  refresh_ttl_secs: integer, min 3600, max 31536000, default 2592000\n  require_verified_email: boolean, default true, Refuse password login until the email is verified.\n  signup: boolean, default true, Allow new accounts through POST /auth/signup.\n  token_ttl_secs: integer, min 300, max 604800, default 86400, Lifetime of verification, reset and magic-link tokens (magic links are capped at 15 minutes).\nroutes: PO
... (7963 more characters)
```

**Bash**

```
cd <exp>/keel-build2 && cp app.yaml <session> for p in auth-core auth-oauth rbac resource; do python3 mcp.py add_plugin "{\"plugin\":\"$p\"}"; done; cat app.yaml; python3 mcp.py setup_status '{}'
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
... (613 more characters)
```

**Bash**

```
cd <exp>/keel-build2 && cat > <session> <<'EOF'
app: team-tasks
server: { host: 127.0.0.1, port: 18103 }
providers:
  db: { use: postgres, url: "${secret:DATABASE_URL}" }
  cache: { use: redis, url: "${secret:REDIS_URL}" }
  email: { use: console }
plugins:
  - use: auth-core
    config:
      app_url: "${secret:PUBLIC_URL}"
      require_verified_email: false
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
python3 - <<'EOF'
import json,subprocess
y=open('<session>
print(subprocess.run(['python3','mcp.py','validate_config',json.dumps({"yaml":y})],capture_output=True,text=True).stdout)
EOF
```

```text
{
  "app": "team-tasks",
  "file": "<exp>/keel-build2/.keel-candidate.yaml",
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
```

**Bash**

```
cd <exp>/keel-build2 && cp <session> app.yaml && ./keel config validate -c app.yaml; python3 mcp.py setup_status '{}'; ./keel migrate --help; ./keel serve --help
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
... (1722 more characters)
```

**Bash**

```
cd <exp>/keel-build2 && ./keel migrate --apply -c app.yaml 2>&1 | tail -20; (nohup ./keel serve -c app.yaml > <session> 2>&1 &) ; sleep 3; curl -s 127.0.0.1:18103/health; echo; python3 acceptance.py http://127.0.0.1:18103 18203 2>&1 | tail -40
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
{"app":"team-tasks","plugins":[{"degrade_level":"error","id":"auth-core","status":"ok","version":"0.1.0"},{"degrade_level":"warn","id":"auth-oauth","status":"ok","version":"0.1.0"},{"degrade_level":"error","id":"rbac","status":"ok","version":"0.1.0"},{"degrade_level":"error","id":"resource","status":"ok","version":"0.1.0"}],"providers":[{"capability":"cache","degrade_level":"warn","hot_load":true,"kind":"redis","slot":"cache","status":"ok"},{"capability":"sql","degrade_level":"error","hot_load":false,"kind":"postgres","slot":"db","status":"ok"},{"capability":"email","degrade_level":"warn","hot_load":true,"kind":"console","slot":"email","status":"ok"}],"setup":{"complete":true,"
... (1417 more characters)
```

**Bash**

```
pkill -f "keel serve -c app.yaml"; sleep 1; pgrep -fa "keel serve" || echo stopped
```

```text
15221 /home/user/keel/target/release/keel serve
```

**SubagentHandback**

```
{
 "message": "All 64 acceptance checks passed on the first test run. Final line: `all 64 checks passed`\n\n**Iterations:** 1 test run, with no fixes needed. Before it I made 1 validate_config call and 1 migrate --apply.\n\n**Final app.yaml** (<exp>/keel-build2/app.yaml):\n```yaml\napp: team-tasks\nserver: { host: 127.0.0.1, port: 18103 }\nproviders:\n  db: { use: postgres, url: \"${secret:DATABASE_URL}\" }\n  cache: { use: redis, url: \"${secret:REDIS_URL}\" }\n  email: { use: console }\nplugins:\n  - use: auth-core\n    config: { app_url: \"${secret:PUBLIC_URL}\", require_verified_email: false }\n  - use: auth-oauth\n    config:\n      redirect_base_url: \"${secret:PUBLIC_URL}\"\n      providers:\n        google:\n          auth_url: \"${secret:GOOGLE_AUTH_URL}\"\n          token_url: \"${secret:GOOGLE_TOKEN_URL}\"\n          userinfo_url: \"${secret:GOOGLE_USERINFO_URL}\"\n  - use: rbac\n    config:\n      admins: [boss@example.com]\n      default_role: null\n      roles:\n        admin: { permissions: [\"*\"] }\n        owner: { permissions: [\"projects.*\", \"tasks.*\", \"rbac.manage\"] }\n        member: { permissions: [\"projects.read\", \"tasks.read\", \"tasks.write\"] }\n  - use: resource\n    config:\n      resources:\n        projects:\n          scope: tenant\n          cache_ttl: 60\n          fields:\n            name: { type: string, required: true, min_length: 1, max_length: 80 }\n            description: { type: text }\n        tasks:\n          scope: tenant\n          cache_ttl: 60\n          fields:\n            title: { type: string, required: true, min_length: 1, max_length: 200 }\n            status: { type: string, enum: [todo, doing, done], default: todo }\n            due: { type: date }\n            assignee: { type: string }\n            project_id: { type: string }\n```\n(The file on disk is the same, in block style rather than the inline maps above.)\n\n**Confusing spots:**\n- **Secret names:** `describe_plugin` never says what the JWT secret or the Google client id/secret are called (`AUTH_JWT_SECRET`, `OAUTH_GOOGLE_CLIENT_*`). They only show up in `setup_status`, and only once the plugin is configured. I guessed right because `.env` already used those names.\n- **`add_plugin` on an empty app:** it fails until providers exist, so you can't add plugins one at a time to an empty app. I had to write the whole yaml by hand.\n- **Tenant roles:** `describe_plugin` doesn't explain how grants scoped to one team combine with the global `admin` role. The only hint is one line saying `manage_permission` \"held in a tenant lets the holder manage that tenant only\". I had to trust it.\n\nNo Keel bugs found. I stopped my server. A different `keel serve` (pid 15221, from /home/user/keel/target/release) was already running and is not mine, so I left it alone."
}
```

```text
{"success":true,"message":"Report delivered to your caller."}
```
