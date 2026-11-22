# rbac

Roles and permissions defined in YAML, with inheritance, direct per-user permission grants, and roles scoped to a tenant (a team or organization). It powers the `role:<name>` and `permission:<name>` route guards and the `resource` plugin's permission checks.

```yaml
plugins:
  - use: auth-core
  - use: rbac
    config:
      roles:
        admin: { permissions: ["*"] }
        owner: { permissions: ["projects.*", "billing.manage", "rbac.manage"], inherits: [member] }
        member: { permissions: ["projects.read"] }
      default_role: member
      admins: [you@example.com]
```

| | |
| --- | --- |
| Version | 0.1.0 |
| Capabilities | `sql`, `log` |
| Requires plugins | `auth-core ^0.1` |
| Provides interface | `authorizer` |
| Consumes events | `auth.user.created` (gives the default role) |
| Degrade level | `error` |
| Setup | none |

## Config

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `roles` | map | `{admin: {permissions: ["*"]}, member: {permissions: []}}` | Role name to `{permissions, inherits}`. Names match `^[a-z][a-z0-9_-]*$`. |
| `roles.<r>.permissions` | list | `[]` | Permission names like `projects.read`, a prefix wildcard `projects.*`, or `*` for everything. |
| `roles.<r>.inherits` | list | `[]` | Roles whose permissions this role also has. Must be defined. |
| `default_role` | string or null | `member` | Global role every new user gets; `null` for none. |
| `admins` | list of emails | `[]` | Get `admin_role` globally on sign-up, or at start if the account exists. Bootstraps the first admin. |
| `admin_role` | string | `admin` | The role `admins` get. |
| `manage_permission` | string | `rbac.manage` | Needed to change roles and grants. Held in a tenant, it lets the holder manage that tenant only. |

## How checks work

A user's **effective roles** for a request are their global roles, plus the roles granted in the request's tenant (the `X-Tenant-Id` header), plus every role those inherit. Their **permissions** are the permissions of those roles plus direct grants (global and in the tenant).

A permission `p` is held when any held entry is `*`, equals `p`, or is a prefix wildcard that covers it (`projects.*` covers `projects.read` and `projects.tasks.write`).

Roles and permissions are read on every request, so a change takes effect immediately, without new tokens.

## Routes

All routes need a bearer token. Changing roles or grants needs `manage_permission` globally, or in the tenant being changed.

| Route | Body or query | Does |
| --- | --- | --- |
| `GET /rbac/roles` | | The configured roles with permissions and inherits. |
| `GET /rbac/me` | `X-Tenant-Id` optional | `{user_id, roles, permissions, tenant}`: the caller's effective roles and permissions. |
| `GET /rbac/users/{id}` | `X-Tenant-Id` optional | That user's assignments and grants. |
| `POST /rbac/users/{id}/roles` | `{role, tenant?}` | Assign a role, globally or in `tenant`. |
| `DELETE /rbac/users/{id}/roles/{role}` | `?tenant=` | Remove a role. |
| `POST /rbac/users/{id}/permissions` | `{permission, tenant?}` | Grant a permission directly. |
| `DELETE /rbac/users/{id}/permissions/{permission}` | `?tenant=` | Remove a grant. |

Example: make Bob an `owner` of team `acme`:

```sh
curl -XPOST localhost:8080/rbac/users/$BOB_ID/roles -H "$ADMIN" -d '{"role":"owner","tenant":"acme"}'
curl localhost:8080/rbac/me -H "$BOB" -H "X-Tenant-Id: acme"
```

## Using it from route rules

```yaml
routes:
  /admin/**: { guard: [authenticated, "role:admin"] }
  "POST /projects": { guard: ["permission:projects.write"] }
```

## Using it from a plugin

rbac provides the `authorizer` interface (`keel_sdk::authz::Authorizer`): `roles`, `has_role`, `has_permission` and `assign_role`. A plugin that creates teams can make the creator the team's owner with `assign_role(user_id, "owner", Some(team_id))`.

## Errors

`forbidden` (403, missing `manage_permission`), `unknown_user` (404), `invalid_permission` (400, a malformed permission name), `missing_token` (401).

## Events

`rbac.role.assigned {user_id, role, tenant}`, `rbac.role.removed {user_id, role, tenant}`.

## Tables

`rbac_assignments` (user, role, tenant), `rbac_grants` (user, permission, tenant).
