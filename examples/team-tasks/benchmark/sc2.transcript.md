# sc2: change, scratch

## Prompt

You are changing a backend app for a measured experiment. Work only inside <exp>/sc2.

The folder holds a working Team Tasks app written in Rust (axum, sqlx Postgres, redis): Cargo.toml and src/, listening on 127.0.0.1:18119, with settings from .env. It already passes acceptance.py (requirements in SPEC.md), and its Postgres database already holds tables and data from earlier runs. Your job: implement the change request in CHANGE.md without losing that data.

Rules:
- Do not look at /home/user/keel or any other folder outside sc2. This rule is the point of the experiment.
- Do not change .env, SPEC.md, CHANGE.md or the acceptance scripts. Do not drop or recreate the database or its tables; existing tables need to be changed in place.
- Build with cargo (target/ already holds a release build), run the server in the background, and stop only your own server when done. Other experiments run in parallel on other ports; leave their processes alone.
- Done means both pass: `python3 acceptance_change.py http://127.0.0.1:18119` and `python3 acceptance.py http://127.0.0.1:18119 18219`.

Report back: whether both passed (paste both summary lines), the number of test runs, and lines changed (a rough count). Under 150 words.

## Prompt

<system-reminder>
Your final report is delivered through SubagentHandback: when your work is complete, call SubagentHandback({message: <your full report>}). The call ends your run, so make it your last step. Only a SubagentHandback call reaches your caller as your result; plain text you write at the end is not delivered. Do not also send the report with SendMessage: your caller would get it twice.
</system-reminder>

**Bash**

```
cd <exp>/sc2 && ls -la && cat CHANGE.md && cat .env && ls src && wc -l src/* && cat Cargo.toml
```

```text
total 116
drwxr-xr-x  4 root root  4096 Nov 21 20:20 .
drwxr-xr-x 16 root root  4096 Nov 21 20:23 ..
-rw-r--r--  1 root root   401 Nov 21 20:20 .env
-rw-r--r--  1 root root  1272 Nov 21 20:20 CHANGE.md
-rw-r--r--  1 root root 63074 Nov 21 20:20 Cargo.lock
-rw-r--r--  1 root root   663 Nov 21 20:20 Cargo.toml
-rw-r--r--  1 root root  3863 Nov 21 20:20 SPEC.md
-rw-r--r--  1 root root 11437 Nov 21 20:20 acceptance.py
-rw-r--r--  1 root root  7712 Nov 21 20:20 acceptance_change.py
drwxr-xr-x  2 root root  4096 Nov 21 20:20 src
drwxr-xr-x  3 root root  4096 Nov 21 20:20 target
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

- A new per-team resource `comments`, with the
... (1455 more characters)
```

**Bash**

```
cd <exp>/sc2 && cat SPEC.md && cat src/main.rs src/crud.rs
```

**Bash**

```
cd <exp>/sc2 && cat src/auth.rs acceptance_change.py
```

```text
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
  `{ user: { id, email, ... } }`; `401` for a missing or bad token.
- Google sign-in (OAuth2 authorization code flow):
  - `GET /auth/oauth/google/start`: `302` to `GOOGLE_AUTH_URL` with
    `response_type=code`, `client_id`, `redirect_uri` =
    `PUBLIC_URL/auth/oauth/google/callback`, `scope` and a random `state`.
  - `GET /auth/oauth/google/callback?code=..&state=..`: exchanges the code at
    `GOOGLE_TOKEN_URL` (form POST with code
... (21907 more characters)
```

```text
use crate::{err, ApiError, ApiResult, AppState};
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use axum::{
    body::Bytes,
    extract::{FromRequestParts, Path, Query, State},
    http::{header, request::Parts, StatusCode},
    response::{IntoResponse, Redirect, Response},
    Json,
};
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation};
use rand::Rng;
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

pub const ADMIN_EMAIL: &str = "boss@example.com";
const ACCESS_TTL: i64 = 3600;
const REFRESH_TTL: i64 = 30 * 24 * 3600;

#[derive(Serialize, Deserialize)]
struct Claims {
    sub: String,
    typ: String,
    exp: i64,
    iat: i64,
    jti: String,
}

pub fn parse_json_object(body: &Bytes) -> ApiResult<serde_json::Map<String, Value>> {
    match serde_json::from_slice::<Value>(body) {
        Ok(Value::Object(m)) => Ok(m),
        _ => Err(err(StatusCode::UNPROCESSABLE_ENTITY, "body must be a JSON object")),
    }
}

fn make_token(st: &AppState, user_id: &str, typ: &str, ttl: i64) -> String {
    let now = chrono::Utc::now().timestamp();
    let claims = Claims {
        sub: user_id.to_string(),
        typ: typ.to_string(),
        iat: now,
        exp: now + ttl,
        jti: uuid::Uuid::new_v4().to_string(),
    };
    jsonwebtoken::encode(
        &Header::default(),
        &claims,
        &EncodingK
... (18566 more characters)
```

**Bash**

```
cd <exp>/sc2 && python3 - <<'EOF'
import re
p='src/main.rs'; s=open(p).read()
s=s.replace("""CREATE INDEX IF NOT EXISTS tasks_tenant ON tasks(tenant);
""","""CREATE INDEX IF NOT EXISTS tasks_tenant ON tasks(tenant);
ALTER TABLE tasks ADD COLUMN IF NOT EXISTS priority INTEGER;
CREATE TABLE IF NOT EXISTS comments (
    id TEXT PRIMARY KEY,
    tenant TEXT NOT NULL,
    task_id TEXT NOT NULL,
    body TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS comments_tenant ON comments(tenant);
""")
open(p,'w').write(s)
p='src/auth.rs'; s=open(p).read()
s=s.replace('matches!(role, "owner" | "member")','matches!(role, "owner" | "member" | "viewer")').replace("role (owner|member)","role (owner|member|viewer)")
open(p,'w').write(s)
p='src/crud.rs'; s=open(p).read()
s=s.replace("""    Date,
}""","""    Date,
    Int { min: i64, max: i64 },
}""",1)
s=s.replace("""const ANY: &[&str] = &["owner", "member"];""","""const ANY: &[&str] = &["owner", "member"];
const READERS: &[&str] = &["owner", "member", "viewer"];""")
s=s.replace("allowed: |a| if a == Action::Read { ANY } else { OWNER },","allowed: |a| if a == Action::Read { READERS } else { OWNER },")
s=s.replace("""        Field { name: "project_id", kind: Kind::Text, required: false, default: None },
    ],
    allowed: |a| if a == Action::Delete { OWNER } else { ANY },
};""","""        Field { name: "project_id", kind: Kind::Text, required: false, default: None },
        Field { name: "priority", kind: Kind::Int { min: 1, max: 5 }, required: false, default: None },
    ],
    allowed: |a| match a {
        Action::Read => READERS,
        Action::Delete => OWNER,
        _ => ANY,
    },
};

static COMMENTS: Resource = Resource {
    table: "comments",
    fields: &[
        Field { name: "task_id", kind: Kind::Text, required: true, default: None },
        Field { name: "body", kind: Kind::Str { max: 250 }, required: true, default: None },
    ],
    allowed: |a| match a {
        Action::Read => READERS,
        Action::Delete => OWNER,
        _ => ANY,
    },
};""")
s=s.replace("""            "tasks" => Ok(&TASKS),""","""            "tasks" => Ok(&TASKS),
            "comments" => Ok(&COMMENTS),""")
s=s.replace("""            Some(Kind::Date) => format!("${n}::date"),""","""            Some(Kind::Date) => format!("${n}::date"),
            Some(Kind::Int { .. }) => format!("${n}::integer"),""")
s=s.replace("""            Some(Value::String(s)) => s.clone(),
            Some(_) =>""","""            Some(Value::Number(n)) if matches!(f.kind, Kind::Int { .. }) => match n.as_i64() {
                Some(i) => i.to_string(),
                None => return Err(unprocessable(format!("'{}' must be an integer", f.name))),
            },
            Some(_) if matches!(f.kind, Kind::Int { .. }) => {
                return Err(unprocessable(format!("'{}' must be an integer", f.name)))
            }
            Some(Value::String(s)) => s.clone(),
            Some(_) =>""")
s=s.replace("""            Kind::Text => {}""","""            Kind::Text => {}
            Kind::Int { min, max } => {
                let ok = value.parse::<i64>().map(|i| (min..=max).contains(&i)).unwrap_or(false);
                if !ok {
                    return Err(unprocessable(format!("'{}' must be an integer from {min} to {max}", f.name)));
                }
            }""")
open(p,'w').write(s)
EOF
grep -n "SELECT\|to_jsonb" src/crud.rs | head; cargo build --release 2>&1 | tail -3
```

```text
257:    format!("SELECT to_jsonb(t) - 'tenant' FROM {} t", res.table)
331:    let count_sql = format!("SELECT count(*) FROM {} t WHERE {where_sql}", res.table);
389:        "INSERT INTO {} ({}) VALUES ({}) RETURNING to_jsonb({}.*) - 'tenant'",
423:        "UPDATE {} SET {}, updated_at = now() WHERE tenant = $1 AND id = $2 RETURNING to_jsonb({}.*) - 'tenant'",
   Compiling team_tasks v0.1.0 (<exp>/sc2)
    Finished `release` profile [optimized] target(s) in 16.94s
```

**Bash**

```
cd <exp>/sc2 && (nohup ./target/release/team_tasks > <exp>/sc2/server.log 2>&1 & echo $! > <exp>/sc2/server.pid); sleep 2; cat server.log; python3 acceptance_change.py http://127.0.0.1:18119 | tail -2; python3 acceptance.py http://127.0.0.1:18119 18219 | tail -2
```

```text
listening on 127.0.0.1:18119

all 52 checks passed

all 63 checks passed
```

**Bash**

```
cd <exp>/sc2 && kill $(cat server.pid); rm server.pid server.log; sleep 1; ss -ltnp | grep 18119 || echo stopped
```

```text
/bin/bash: line 1: ss: command not found
stopped
```

**SubagentHandback**

```
{
 "message": "Both acceptance scripts passed on the first run.\n- `acceptance_change.py`: \"all 52 checks passed\"\n- `acceptance.py`: \"all 63 checks passed\"\n\nTest runs: 1 of each script, 2 in total, with no failures to fix.\n\nLines changed: about 60, in `src/main.rs`, `src/crud.rs` and `src/auth.rs`.\n- **Priority:** an `ALTER TABLE tasks ADD COLUMN IF NOT EXISTS priority INTEGER` was added to the startup schema. Tasks now accept an integer `priority` from 1 to 5 and refuse anything else with 422.\n- **Viewer:** `viewer` can now be granted as a role, and it can only read projects, tasks and comments.\n- **Comments:** a new `comments` table (created only if it doesn't exist) and a `comments` resource that reuses the existing generic CRUD, caching and team rules.\n\nI didn't drop or recreate any tables; the only schema changes are that `ALTER` and the new table. I stopped my own server with the PID I saved when starting it and left all other processes alone."
}
```

```text
{"success":true,"message":"Report delivered to your caller."}
```
