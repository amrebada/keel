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
    `GOOGLE_TOKEN_URL` (form POST with code, redirect_uri, client id and
    secret), reads `GOOGLE_USERINFO_URL` with the provider token
    (`{sub, email, email_verified, name}`), creates or finds the user, and
    answers `200` with `{ access_token, ... }`. A state is single use; a
    forged or replayed state or code fails with a `4xx`.

## Roles and teams

- A team is identified by the `X-Tenant-Id` header (any string).
- Roles: `admin` (everything, everywhere), and per team `owner` (manage
  projects and tasks, and grant roles in that team) and `member` (read
  projects, read and write tasks, not delete).
- `boss@example.com` is an admin as soon as that account exists.
- New users have no role anywhere until granted one.
- `POST /rbac/users/{user_id}/roles {role, tenant}`: grants a role in a team.
  Allowed for admins and for owners of that team; `201` when granted, `403`
  otherwise.

## Projects and tasks (per team)

Every request needs a bearer token (`401` without) and `X-Tenant-Id`
(`400` without). Rows belong to the team in the header; other teams never
see them (`404` for a single row, absent from lists). Missing permission:
`403`.

- Projects: `name` (string, 1 to 80, required), `description` (text, optional).
  Read: owner, member, admin. Create, update, delete: owner, admin.
- Tasks: `title` (string, 1 to 200, required), `status` (`todo`, `doing`,
  `done`; default `todo`), `due` (date `YYYY-MM-DD`, a real calendar date),
  `assignee` (string), `project_id` (string), all but title optional.
  Read, create, update: owner, member, admin. Delete: owner, admin.
- Routes, for each of `projects` and `tasks`:
  - `GET /api/<name>`: `200 { items, total, limit, offset }`. Query:
    `<field>=<value>` filters, `<field>.null=true|false`, `sort=<field>` or
    `sort=-<field>` (descending), `limit`, `offset`.
  - `POST /api/<name>`: `201` with the row (`id`, the fields, timestamps).
  - `GET /api/<name>/{id}`: `200` with the row.
  - `PATCH /api/<name>/{id}`: `200` with the updated row; fields left out
    keep their values.
  - `DELETE /api/<name>/{id}`: `204`.
- Invalid bodies (missing required field, unknown field, bad enum value,
  impossible date, too long): `422`.
- Lists and single rows are cached in Redis; writes invalidate the cache, so
  reads after a write are always fresh.

## Operations

- `GET /health`: `200` when the app and its database are up.
