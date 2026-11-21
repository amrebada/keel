# sb1: build, scratch

## Prompt

You are building a backend app for a measured experiment. Work only inside <exp>/scratch-build.

Goal: implement the Team Tasks API described in <exp>/scratch-build/SPEC.md from scratch in Rust, so it passes its black-box test.

Rules:
- Use Rust with axum, sqlx (Postgres) and the redis crate, plus whatever other crates you need (argon2, jsonwebtoken, reqwest, etc. are fine). Create a new cargo project in this folder. Crates are cached in ~/.cargo, and crates.io is reachable.
- Do not look at /home/user/keel or any other existing project on disk; write the code yourself. This rule is the point of the experiment.
- Listen on 127.0.0.1:18102. Settings and secrets come from the provided .env file in that folder (read it; do not change it). Postgres and Redis are already running; the URLs are in .env. Create the schema from the app on startup.
- Build in release or debug, run the server in the background, and test with `python3 acceptance.py http://127.0.0.1:18102 18202` (it starts its own mock Google on port 18202). Iterate until every check passes. Do not edit acceptance.py or SPEC.md. Stop the server when done.

Report back: whether all checks passed (paste the final summary line), lines of Rust you wrote (`wc -l src/*.rs`), the number of test iterations, and anything notable. Keep the report under 300 words.

## Prompt

<system-reminder>
Your final report is delivered through SubagentHandback: when your work is complete, call SubagentHandback({message: <your full report>}). The call ends your run, so make it your last step. Only a SubagentHandback call reaches your caller as your result; plain text you write at the end is not delivered. Do not also send the report with SendMessage: your caller would get it twice.
</system-reminder>

**Bash**

```
cd <exp>/scratch-build && ls -la && cat SPEC.md && cat .env && wc -l acceptance.py
```

```text
total 28
drwxr-xr-x 2 root root  4096 Nov 21 20:00 .
drwxr-xr-x 4 root root  4096 Nov 21 20:00 ..
-rw-r--r-- 1 root root   398 Nov 21 20:00 .env
-rw-r--r-- 1 root root  3863 Nov 21 20:00 SPEC.md
-rw-r--r-- 1 root root 11437 Nov 21 20:00 acceptance.py
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
 
... (2980 more characters)
```

**Bash**

```
cat <exp>/scratch-build/acceptance.py; cargo --version; ls ~/.cargo/registry/cache/*/ | grep -E "^(axum|sqlx|redis|argon2|jsonwebtoken|reqwest|tokio|chrono|uuid|serde_json|rand|dotenvy|tower)-[0-9]" | sort | tail -40
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
... (10226 more characters)
```

**Bash**

```
ls ~/.cargo/registry/cache/*/ | grep -E "^(jsonwebtoken|chrono|hmac|sha2|base64|serde|tower-http|password-hash|hex)-[0-9]" | sort
```

```text
base64-0.22.1.crate
base64-0.23.1.crate
hex-0.4.3.crate
hmac-0.12.1.crate
password-hash-0.5.0.crate
serde-1.0.229.crate
sha2-0.10.9.crate
tower-http-0.6.11.crate
```

**Bash**

```
cd <exp>/scratch-build && cargo init --name team_tasks -q 2>&1; cat > Cargo.toml <<'EOF'
[package]
name = "team_tasks"
version = "0.1.0"
edition = "2021"

[dependencies]
axum = "0.8"
tokio = { version = "1", features = ["full"] }
sqlx = { version = "0.8", default-features = false, features = ["runtime-tokio", "postgres", "macros", "chrono"] }
redis = { version = "0.27", features = ["tokio-comp", "connection-manager"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
argon2 = "0.5"
jsonwebtoken = "9"
reqwest = { version = "0.12", default-features = false, features = ["json"] }
chrono = { version = "0.4", features = ["serde"] }
uuid = { version = "1", features = ["v4"] }
dotenvy = "0.15"
rand = "0.8"
urlencoding = "2"
EOF
cargo fetch 2>&1 | tail -3
```

```text
  Downloaded windows-sys v0.61.2
  Downloaded windows-sys v0.48.0
  Downloaded windows-sys v0.52.0
```

**Bash**

```
cd <exp>/scratch-build && sed -i 's/"macros", "chrono"/"macros", "chrono", "json"/' Cargo.toml && mkdir -p src
```

```text
(Bash completed with no output)
```

**Write**

```
{
 "file_path": "<exp>/scratch-build/src/main.rs",
 "content": "mod auth;\nmod crud;\n\nuse axum::{\n    http::StatusCode,\n    response::{IntoResponse, Response},\n    routing::{get, post},\n    Json, Router,\n};\nuse serde_json::{json, Value};\nuse sqlx::PgPool;\n\n#[derive(Clone)]\npub struct Config {\n    pub public_url: String,\n    pub jwt_secret: String,\n    pub google_client_id: String,\n    pub google_client_secret: String,\n    pub google_auth_url: String,\n    pub google_token_url: String,\n    pub google_userinfo_url: String,\n}\n\n#[derive(Clone)]\npub struct AppState {\n    pub db: PgPool,\n    pub redis: redis::aio::ConnectionManager,\n    pub cfg: std::sync::Arc<Config>,\n    pub http: reqwest::Client,\n}\n\npub struct ApiError(pub StatusCode, pub String);\n\nimpl IntoResponse for ApiError {\n    fn into_response(self) -> Response {\n        (self.0, Json(json!({ \"error\": self.1 }))).into_response()\n    }\n}\n\npub fn err(code: StatusCode, msg: impl Into<String>) -> ApiError {\n    ApiError(code, msg.into())\n}\n\nimpl From<sqlx::Error> for ApiError {\n    fn from(e: sqlx::Error) -> Self {\n        eprintln!(\"db error: {e}\");\n        err(StatusCode::INTERNAL_SERVER_ERROR, \"database error\")\n    }\n}\n\nimpl From<redis::RedisError> for ApiError {\n    fn from(e: redis::RedisError) -> Self {\n        eprintln!(\"redis error: {e}\");\n        err(StatusCode::INTERNAL_SERVER_ERROR, \"cache error\")\n    }\n}\n\npub type ApiResult<T> = Result<T, ApiError>;\n\nfn env(name: &str) -> String {\n    std::env::var(name).unwrap_or_else(|_| panic!(\"missing env var {name}\"))\n}\n\nconst SCHEMA: &str = r#\"\nCREATE TABLE IF NOT EXISTS users (\n    id TEXT PRIMARY KEY,\n    email TEXT NOT NULL UNIQUE,\n    password_hash TEXT,\n    google_sub TEXT UNIQUE,\n    name TEXT,\n    created_at TIMESTAMPTZ NOT NULL DEFAULT now()\n);\nCREATE TABLE IF NOT EXISTS roles (\n    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,\n    tenant TEXT NOT NULL,\n    role TEXT NOT NULL,\n    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),\n    PRIMARY KEY (user_id, tenant, role)\n);\nCREATE TABLE IF NOT EXISTS projects (\n    id TEXT PRIMARY KEY,\n    tenant TEXT NOT NULL,\n    name TEXT NOT NULL,\n    description TEXT,\n    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),\n    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()\n);\nCREATE INDEX IF NOT EXISTS projects_tenant ON projects(tenant);\nCREATE TABLE IF NOT EXISTS tasks (\n    id TEXT PRIMARY KEY,\n    tenant TEXT NOT NULL,\n    title TEXT NOT NULL,\n    status TEXT NOT NULL DEFAULT 'todo',\n    due DATE,\n    assignee TEXT,\n    project_id TEXT,\n    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),\n    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()\n);\nCREATE INDEX IF NOT EXISTS tasks_tenant ON tasks(tenant);\n\"#;\n\nasync fn health(axum::extract::State(st): axum::extract::State<AppState>) -> ApiResult<Json<Value>> {\n    sqlx::query(\"SELECT 1\").execute(&st.db).await?;\n    Ok(Json(json!({ \"status\": \"ok\" })))\n}\n\n#[tokio::main]\nasync fn main() {\n    dotenvy::dotenv().ok();\n    let cfg = Config {\n        public_url: env(\"PUBLIC_URL\").trim_end_matches('/').to_string(),\n        jwt_secret: env(\"AUTH_JWT_SECRET\"),\n        google_client_id: env(\"OAUTH_GOOGLE_CLIENT_ID\"),\n        google_client_secret: env(\"OAUTH_GOOGLE_CLIENT_SECRET\"),\n        google_auth_url: env(\"GOOGLE_AUTH_URL\"),\n        google_token_url: env(\"GOOGLE_TOKEN_URL\"),\n        google_userinfo_url: env(\"GOOGLE_USERINFO_URL\"),\n    };\n    let db = sqlx::postgres::PgPoolOptions::new()\n        .max_connections(10)\n        .connect(&env(\"DATABASE_URL\"))\n        .await\n        .expect(\"connect postgres\");\n    sqlx::raw_sql(SCHEMA).execute(&db).await.expect(\"create schema\");\n    let client = redis::Client::open(env(\"REDIS_URL\")).expect(\"redis url\");\n    let redis = redis::aio::ConnectionManager::new(client).await.expect(\"connect redis\");\n    let state = AppState {\n     
```

```text
<tool_use_error>File has not been read yet. Read it first before writing to it.</tool_use_error>
```

**Write**

```
{
 "file_path": "<exp>/scratch-build/src/auth.rs",
 "content": "use crate::{err, ApiError, ApiResult, AppState};\nuse argon2::{\n    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},\n    Argon2,\n};\nuse axum::{\n    body::Bytes,\n    extract::{FromRequestParts, Path, Query, State},\n    http::{header, request::Parts, StatusCode},\n    response::{IntoResponse, Redirect, Response},\n    Json,\n};\nuse jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation};\nuse rand::Rng;\nuse redis::AsyncCommands;\nuse serde::{Deserialize, Serialize};\nuse serde_json::{json, Value};\nuse std::collections::HashMap;\n\npub const ADMIN_EMAIL: &str = \"boss@example.com\";\nconst ACCESS_TTL: i64 = 3600;\nconst REFRESH_TTL: i64 = 30 * 24 * 3600;\n\n#[derive(Serialize, Deserialize)]\nstruct Claims {\n    sub: String,\n    typ: String,\n    exp: i64,\n    iat: i64,\n    jti: String,\n}\n\npub fn parse_json_object(body: &Bytes) -> ApiResult<serde_json::Map<String, Value>> {\n    match serde_json::from_slice::<Value>(body) {\n        Ok(Value::Object(m)) => Ok(m),\n        _ => Err(err(StatusCode::UNPROCESSABLE_ENTITY, \"body must be a JSON object\")),\n    }\n}\n\nfn make_token(st: &AppState, user_id: &str, typ: &str, ttl: i64) -> String {\n    let now = chrono::Utc::now().timestamp();\n    let claims = Claims {\n        sub: user_id.to_string(),\n        typ: typ.to_string(),\n        iat: now,\n        exp: now + ttl,\n        jti: uuid::Uuid::new_v4().to_string(),\n    };\n    jsonwebtoken::encode(\n        &Header::default(),\n        &claims,\n        &EncodingKey::from_secret(st.cfg.jwt_secret.as_bytes()),\n    )\n    .expect(\"encode jwt\")\n}\n\nfn token_pair(st: &AppState, user_id: &str, email: &str) -> Value {\n    json!({\n        \"access_token\": make_token(st, user_id, \"access\", ACCESS_TTL),\n        \"refresh_token\": make_token(st, user_id, \"refresh\", REFRESH_TTL),\n        \"token_type\": \"bearer\",\n        \"expires_in\": ACCESS_TTL,\n        \"user\": { \"id\": user_id, \"email\": email },\n    })\n}\n\n/// The signed-in user, from a bearer access token.\npub struct AuthUser {\n    pub id: String,\n    pub email: String,\n}\n\nimpl AuthUser {\n    pub fn is_admin(&self) -> bool {\n        self.email.eq_ignore_ascii_case(ADMIN_EMAIL)\n    }\n\n    /// Roles this user holds in a team; admins get \"admin\".\n    pub async fn roles_in(&self, st: &AppState, tenant: &str) -> ApiResult<Vec<String>> {\n        let mut roles: Vec<String> =\n            sqlx::query_scalar(\"SELECT role FROM roles WHERE user_id = $1 AND tenant = $2\")\n                .bind(&self.id)\n                .bind(tenant)\n                .fetch_all(&st.db)\n                .await?;\n        if self.is_admin() {\n            roles.push(\"admin\".into());\n        }\n        Ok(roles)\n    }\n}\n\nimpl FromRequestParts<AppState> for AuthUser {\n    type Rejection = ApiError;\n\n    async fn from_request_parts(parts: &mut Parts, st: &AppState) -> Result<Self, ApiError> {\n        let unauth = || err(StatusCode::UNAUTHORIZED, \"missing or invalid token\");\n        let h = parts\n            .headers\n            .get(header::AUTHORIZATION)\n            .and_then(|v| v.to_str().ok())\n            .ok_or_else(unauth)?;\n        let tok = h\n            .strip_prefix(\"Bearer \")\n            .or_else(|| h.strip_prefix(\"bearer \"))\n            .ok_or_else(unauth)?\n            .trim();\n        let data = jsonwebtoken::decode::<Claims>(\n            tok,\n            &DecodingKey::from_secret(st.cfg.jwt_secret.as_bytes()),\n            &Validation::default(),\n        )\n        .map_err(|_| unauth())?;\n        if data.claims.typ != \"access\" {\n            return Err(unauth());\n        }\n        let email: Option<String> = sqlx::query_scalar(\"SELECT email FROM users WHERE id = $1\")\n            .bind(&data.claims.sub)\n            .fetch_optional(&st.db)\n            .await?;\n        let email = email.ok_or
```

```text
File created successfully at: <exp>/scratch-build/src/auth.rs (file state is current in your context — no need to Read it back)
```

**Read**

```
{
 "file_path": "<exp>/scratch-build/src/main.rs"
}
```

```text
1	fn main() {
2	    println!("Hello, world!");
3	}
4	
```

**Write**

```
{
 "file_path": "<exp>/scratch-build/src/main.rs",
 "content": "mod auth;\nmod crud;\n\nuse axum::{\n    http::StatusCode,\n    response::{IntoResponse, Response},\n    routing::{get, post},\n    Json, Router,\n};\nuse serde_json::{json, Value};\nuse sqlx::PgPool;\n\n#[derive(Clone)]\npub struct Config {\n    pub public_url: String,\n    pub jwt_secret: String,\n    pub google_client_id: String,\n    pub google_client_secret: String,\n    pub google_auth_url: String,\n    pub google_token_url: String,\n    pub google_userinfo_url: String,\n}\n\n#[derive(Clone)]\npub struct AppState {\n    pub db: PgPool,\n    pub redis: redis::aio::ConnectionManager,\n    pub cfg: std::sync::Arc<Config>,\n    pub http: reqwest::Client,\n}\n\npub struct ApiError(pub StatusCode, pub String);\n\nimpl IntoResponse for ApiError {\n    fn into_response(self) -> Response {\n        (self.0, Json(json!({ \"error\": self.1 }))).into_response()\n    }\n}\n\npub fn err(code: StatusCode, msg: impl Into<String>) -> ApiError {\n    ApiError(code, msg.into())\n}\n\nimpl From<sqlx::Error> for ApiError {\n    fn from(e: sqlx::Error) -> Self {\n        eprintln!(\"db error: {e}\");\n        err(StatusCode::INTERNAL_SERVER_ERROR, \"database error\")\n    }\n}\n\nimpl From<redis::RedisError> for ApiError {\n    fn from(e: redis::RedisError) -> Self {\n        eprintln!(\"redis error: {e}\");\n        err(StatusCode::INTERNAL_SERVER_ERROR, \"cache error\")\n    }\n}\n\npub type ApiResult<T> = Result<T, ApiError>;\n\nfn env(name: &str) -> String {\n    std::env::var(name).unwrap_or_else(|_| panic!(\"missing env var {name}\"))\n}\n\nconst SCHEMA: &str = r#\"\nCREATE TABLE IF NOT EXISTS users (\n    id TEXT PRIMARY KEY,\n    email TEXT NOT NULL UNIQUE,\n    password_hash TEXT,\n    google_sub TEXT UNIQUE,\n    name TEXT,\n    created_at TIMESTAMPTZ NOT NULL DEFAULT now()\n);\nCREATE TABLE IF NOT EXISTS roles (\n    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,\n    tenant TEXT NOT NULL,\n    role TEXT NOT NULL,\n    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),\n    PRIMARY KEY (user_id, tenant, role)\n);\nCREATE TABLE IF NOT EXISTS projects (\n    id TEXT PRIMARY KEY,\n    tenant TEXT NOT NULL,\n    name TEXT NOT NULL,\n    description TEXT,\n    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),\n    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()\n);\nCREATE INDEX IF NOT EXISTS projects_tenant ON projects(tenant);\nCREATE TABLE IF NOT EXISTS tasks (\n    id TEXT PRIMARY KEY,\n    tenant TEXT NOT NULL,\n    title TEXT NOT NULL,\n    status TEXT NOT NULL DEFAULT 'todo',\n    due DATE,\n    assignee TEXT,\n    project_id TEXT,\n    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),\n    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()\n);\nCREATE INDEX IF NOT EXISTS tasks_tenant ON tasks(tenant);\n\"#;\n\nasync fn health(axum::extract::State(st): axum::extract::State<AppState>) -> ApiResult<Json<Value>> {\n    sqlx::query(\"SELECT 1\").execute(&st.db).await?;\n    Ok(Json(json!({ \"status\": \"ok\" })))\n}\n\n#[tokio::main]\nasync fn main() {\n    dotenvy::dotenv().ok();\n    let cfg = Config {\n        public_url: env(\"PUBLIC_URL\").trim_end_matches('/').to_string(),\n        jwt_secret: env(\"AUTH_JWT_SECRET\"),\n        google_client_id: env(\"OAUTH_GOOGLE_CLIENT_ID\"),\n        google_client_secret: env(\"OAUTH_GOOGLE_CLIENT_SECRET\"),\n        google_auth_url: env(\"GOOGLE_AUTH_URL\"),\n        google_token_url: env(\"GOOGLE_TOKEN_URL\"),\n        google_userinfo_url: env(\"GOOGLE_USERINFO_URL\"),\n    };\n    let db = sqlx::postgres::PgPoolOptions::new()\n        .max_connections(10)\n        .connect(&env(\"DATABASE_URL\"))\n        .await\n        .expect(\"connect postgres\");\n    sqlx::raw_sql(SCHEMA).execute(&db).await.expect(\"create schema\");\n    let client = redis::Client::open(env(\"REDIS_URL\")).expect(\"redis url\");\n    let redis = redis::aio::ConnectionManager::new(client).await.expect(\"connect redis\");\n    let state = AppState {\n     
```

```text
The file <exp>/scratch-build/src/main.rs has been updated successfully. (file state is current in your context — no need to Read it back)
```

**Write**

```
{
 "file_path": "<exp>/scratch-build/src/crud.rs",
 "content": "//! Team-scoped CRUD for projects and tasks, with a Redis read cache.\n\nuse crate::{auth::parse_json_object, auth::AuthUser, err, ApiError, ApiResult, AppState};\nuse axum::{\n    body::Bytes,\n    extract::{FromRequestParts, Path, RawQuery, State},\n    http::{request::Parts, StatusCode},\n    response::{IntoResponse, Response},\n    Json,\n};\nuse redis::AsyncCommands;\nuse serde_json::{json, Map, Value};\n\nconst CACHE_TTL: u64 = 300;\n\n#[derive(Clone, Copy)]\nenum Kind {\n    Str { max: usize },\n    Text,\n    Enum(&'static [&'static str]),\n    Date,\n}\n\nstruct Field {\n    name: &'static str,\n    kind: Kind,\n    required: bool,\n    default: Option<&'static str>,\n}\n\n#[derive(Clone, Copy, PartialEq)]\nenum Action {\n    Read,\n    Create,\n    Update,\n    Delete,\n}\n\nstruct Resource {\n    table: &'static str,\n    fields: &'static [Field],\n    /// Which team roles may perform each action (admin always may).\n    allowed: fn(Action) -> &'static [&'static str],\n}\n\nconst OWNER: &[&str] = &[\"owner\"];\nconst ANY: &[&str] = &[\"owner\", \"member\"];\n\nstatic PROJECTS: Resource = Resource {\n    table: \"projects\",\n    fields: &[\n        Field { name: \"name\", kind: Kind::Str { max: 80 }, required: true, default: None },\n        Field { name: \"description\", kind: Kind::Text, required: false, default: None },\n    ],\n    allowed: |a| if a == Action::Read { ANY } else { OWNER },\n};\n\nstatic TASKS: Resource = Resource {\n    table: \"tasks\",\n    fields: &[\n        Field { name: \"title\", kind: Kind::Str { max: 200 }, required: true, default: None },\n        Field {\n            name: \"status\",\n            kind: Kind::Enum(&[\"todo\", \"doing\", \"done\"]),\n            required: false,\n            default: Some(\"todo\"),\n        },\n        Field { name: \"due\", kind: Kind::Date, required: false, default: None },\n        Field { name: \"assignee\", kind: Kind::Text, required: false, default: None },\n        Field { name: \"project_id\", kind: Kind::Text, required: false, default: None },\n    ],\n    allowed: |a| if a == Action::Delete { OWNER } else { ANY },\n};\n\n/// Columns usable in filters and sorting besides the writable fields.\nconst SYSTEM_COLUMNS: &[&str] = &[\"id\", \"created_at\", \"updated_at\"];\n\nimpl Resource {\n    fn by_name(name: &str) -> ApiResult<&'static Resource> {\n        match name {\n            \"projects\" => Ok(&PROJECTS),\n            \"tasks\" => Ok(&TASKS),\n            _ => Err(err(StatusCode::NOT_FOUND, \"no such resource\")),\n        }\n    }\n\n    fn field(&self, name: &str) -> Option<&Field> {\n        self.fields.iter().find(|f| f.name == name)\n    }\n\n    fn is_column(&self, name: &str) -> bool {\n        self.field(name).is_some() || SYSTEM_COLUMNS.contains(&name)\n    }\n\n    /// SQL placeholder for a bound text parameter, cast to the column's type.\n    fn placeholder(&self, col: &str, n: usize) -> String {\n        match self.field(col).map(|f| f.kind) {\n            Some(Kind::Date) => format!(\"${n}::date\"),\n            _ => format!(\"${n}\"),\n        }\n    }\n}\n\npub struct Tenant(pub String);\n\nimpl FromRequestParts<AppState> for Tenant {\n    type Rejection = ApiError;\n\n    async fn from_request_parts(parts: &mut Parts, _: &AppState) -> Result<Self, ApiError> {\n        parts\n            .headers\n            .get(\"x-tenant-id\")\n            .and_then(|v| v.to_str().ok())\n            .map(|s| s.trim().to_string())\n            .filter(|s| !s.is_empty())\n            .map(Tenant)\n            .ok_or_else(|| err(StatusCode::BAD_REQUEST, \"X-Tenant-Id header is required\"))\n    }\n}\n\nasync fn authorize(st: &AppState, user: &AuthUser, tenant: &str, res: &Resource, action: Action) -> ApiResult<()> {\n    let roles = user.roles_in(st, tenant).await?;\n    let allowed = (res.allowed)(action);\n    if roles.iter().any(|r| r == \"admin\" || allowed.contains(&r.as_
```

```text
File created successfully at: <exp>/scratch-build/src/crud.rs (file state is current in your context — no need to Read it back)
```

**Bash**

```
cd <exp>/scratch-build && cargo build --release 2>&1 | grep -E "^(error|warning)" -A8 | head -80; ls target/release/team_tasks
```

```text
target/release/team_tasks
```

**Bash**

```
cd <exp>/scratch-build && (nohup ./target/release/team_tasks > server.log 2>&1 &) ; for i in $(seq 1 30); do curl -s 127.0.0.1:18102/health && break; sleep 0.5; done; echo; python3 acceptance.py http://127.0.0.1:18102 18202 2>&1 | tail -15; cat server.log
```

```text
{"status":"ok"}
ok   task with unknown field -> 422
ok   filter by status
ok   sort by due date
ok   paging
ok   get one task
ok   member updates a task
ok   reads after a write are fresh (cache cleared)
ok   lists after a write are fresh
ok   another team cannot see the task -> 404
ok   member cannot delete -> 403
ok   owner deletes -> 204
ok   deleted task is gone -> 404
ok   tasks need a team header -> 400

all 64 checks passed
listening on 127.0.0.1:18102
```

**Bash**

```
cd <exp>/scratch-build && python3 acceptance.py http://127.0.0.1:18102 18202 2>&1 | tail -1; pkill -f target/release/team_tasks; sleep 0.5; pgrep -f team_tasks || echo stopped; rm -f server.log; wc -l src/*.rs
```

```text
all 63 checks passed
1860
  352 src/auth.rs
  436 src/crud.rs
  153 src/main.rs
  941 total
```

**Bash**

```
ps aux | grep '[r]elease/team_tasks' || echo none
```

```text
none
```

**SubagentHandback**

```
{
 "message": "All checks passed on the first test run. Final summary line: `all 64 checks passed`. A second run also passed: `all 63 checks passed`. It has one check fewer because the boss account already existed, so the script skips the \"signup boss\" check.\n\n**Lines of Rust (`wc -l src/*.rs`):**\n- src/main.rs: 153\n- src/auth.rs: 352\n- src/crud.rs: 436\n- Total: 941\n\n**Test iterations:** 1, plus 1 rerun to confirm it passes again on the same database. The release build compiled cleanly on the first try, with no errors or warnings.\n\n**Notable:**\n- **Crates:** axum 0.8, sqlx 0.8 (Postgres), redis 0.27 with ConnectionManager, argon2 for passwords, jsonwebtoken 9 for access and refresh tokens, reqwest 0.12, chrono, uuid and dotenvy. jsonwebtoken and chrono were not in the local cache and came from crates.io.\n- **Schema:** the app creates its tables at startup with `CREATE TABLE IF NOT EXISTS` (users, roles, projects, tasks).\n- **Projects and tasks:** one set of handlers serves both, driven by a field list per resource. Each field has a type (string with a max length, text, enum or date) and a required/default flag. The same list is used to:\n  - check bodies and return 422 (unknown field, missing field, bad enum value, impossible date via chrono, too long);\n  - whitelist the fields allowed in filters (`field=value`, `field.null=`) and in `sort`, which keeps the dynamic SQL safe.\n  Rows come back as `to_jsonb(row)` with the tenant column removed.\n- **Cache:** Redis keys include a version number per table and team. Every write bumps that number, so lists and single rows are always fresh after a write. Permissions are checked before the cache is read.\n- **Google sign-in:** the state is stored in Redis for 600s and removed with GETDEL when used, so a replayed or forged state gets 400.\n- **Admin:** an account counts as admin when its email is boss@example.com; this is checked on each request.\n- **Order of errors:** the token is checked before the `X-Tenant-Id` header, so a request with neither gets 401, and one with only a token gets 400.\n\nThe server has been stopped. I did not edit `.env`, `acceptance.py` or `SPEC.md`. The code is in <exp>/scratch-build/src/ and Cargo.toml is in <exp>/scratch-build/."
}
```

```text
{"success":true,"message":"Report delivered to your caller."}
```
