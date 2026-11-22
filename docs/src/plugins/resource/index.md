# resource

REST CRUD from YAML models. Each resource becomes a table, its migrations and six routes with validation, filters, sorting, paging, optional per-owner or per-tenant rows, and optional caching.

```yaml
plugins:
  - use: resource
    config:
      resources:
        projects:                      # table resource_projects, routes /api/projects
          fields:
            name: { type: string, required: true, max_length: 80 }
            slug: { type: string, required: true, unique: true, pattern: "^[a-z0-9-]+$" }
            status: { type: string, enum: [active, archived], default: active }
            budget: { type: number, min: 0 }
            due: { type: date }
        notes:
          scope: owner                 # rows belong to the signed-in user
          access: authenticated        # any signed-in user may call the routes
          cache_ttl: 60
          fields:
            body: { type: text, required: true }
```

| | |
| --- | --- |
| Version | 0.1.0 |
| Capabilities | `sql`, `cache`, `log` |
| Requires plugins | none (auth-core for anything but `access: public`; rbac for `access: permissions`) |
| Degrade level | `error` |
| Setup | none |

## Routes

For each resource `<name>`:

| Route | Does | Default permission |
| --- | --- | --- |
| `GET /api/<name>` | list: `{items, total, limit, offset}` | `<name>.read` |
| `POST /api/<name>` | create; `201` with the row | `<name>.write` |
| `GET /api/<name>/{id}` | read one | `<name>.read` |
| `PATCH /api/<name>/{id}` | update the fields sent | `<name>.write` |
| `DELETE /api/<name>/{id}` | delete; `204` | `<name>.delete` |

And `GET /api` describes every resource: its access rules, create schema and the query syntax. It's a good thing to hand a frontend developer or an AI.

Every row has `id` (a UUID), its fields, `created_at` and `updated_at` (Unix milliseconds), plus `owner_id` or `tenant` when scoped.

## Resource options

| Key | Default | Meaning |
| --- | --- | --- |
| `fields` | required | Field name to its definition (below). Names match `^[a-z][a-z0-9_]{0,39}$`; `id`, `created_at`, `updated_at`, `owner_id`, `tenant` and SQL keywords are reserved. |
| `access` | `permissions` | Who may call the routes: `permissions` (rbac permissions per operation), `authenticated` (any signed-in user) or `public` (anyone; restrict with `routes:` rules). |
| `permissions` | `<name>.read`, `<name>.write`, `<name>.delete` | Rename the permissions: `{ read, write, delete }`. |
| `scope` | `none` | `owner`: rows belong to the user who created them. `tenant`: rows belong to the `X-Tenant-Id` team. |
| `manage_permission` | `<name>.manage` | With `scope: owner`, holders see and change every row. |
| `page_size` | `20` | Default `limit`. |
| `max_page_size` | `100` | Largest `limit` accepted. |
| `cache_ttl` | `0` | Seconds to cache list and get results in the `cache` provider; any write clears them. `0` turns caching off. |

The name is the URL segment and the table suffix; use a plural (`projects`).

## Fields

| Key | Applies to | Meaning |
| --- | --- | --- |
| `type` | all, required | `string` (up to 255 characters), `text` (long), `integer`, `number`, `boolean`, `date` (`YYYY-MM-DD`, checked against the calendar), `datetime` (RFC 3339), `json` |
| `required` | all | must be present on create (default `false`) |
| `default` | all | used on create when the field is left out |
| `enum` | all | allowed values |
| `min`, `max` | numbers | bounds |
| `min_length`, `max_length` | strings | lengths (`max_length` at most 255 for `string`) |
| `pattern` | strings | a regular expression the value must match |
| `unique` | all | no two rows share a value (per tenant with `scope: tenant`); a duplicate answers `409 conflict` |
| `filter` | all | allow `?field=` filters (default true except `text` and `json`) |
| `sort` | all | allow `?sort=field` (default true except `text` and `json`) |

Fields that aren't `required` accept `null`. Unknown fields in a request are rejected.

## Querying lists

| Query | Meaning |
| --- | --- |
| `?status=active` | equals |
| `?budget.gte=100`, `.gt`, `.lt`, `.lte`, `.ne` | comparisons |
| `?name.contains=apo` | substring |
| `?slug.in=a,b` | one of |
| `?due.null=true` | is null (or `false`: is not null) |
| `?sort=-created_at,name` | sort; `-` for descending |
| `?limit=50&offset=100` | paging |

## Access and scopes

- **`access: permissions`** (the default) is deny by default: a request needs a valid token (`401` otherwise) and the operation's permission from rbac (`403` otherwise), checked in the `X-Tenant-Id` tenant when sent. Without rbac in `app.yaml` these routes answer `503 needs_rbac`.
- **`access: authenticated`** needs only a valid token. Without auth-core: `503 needs_auth`.
- **`access: public`** checks nothing; add `routes:` rules for what you need.
- **`scope: owner`** stores the creator in `owner_id` and limits every operation to the caller's rows, except for holders of `manage_permission`.
- **`scope: tenant`** stores the `X-Tenant-Id` header in `tenant` and limits every operation to that team's rows. A request without the header answers `400 missing_tenant`.

Route rules in `app.yaml` add to these checks, for example rate limits or `role:admin` on delete.

## Changing models

Each table and each field is its own migration (`projects_00_table`, `projects_10_name`, `projects_20_slug_unique`), so:

- adding a field adds a column;
- changing a field's rules (`max_length`, `enum`, `required`, `default`, `pattern`) needs no migration;
- to change a field's type, add a field with a new name;
- removing a field from YAML stops serving it but leaves the column.

## Errors

| Code | Status | When |
| --- | --- | --- |
| `validation_failed` | 422 | the body breaks a field rule; `issues` names each field and rule |
| `conflict` | 409 | a `unique` value already exists (`field` says which) |
| `not_found` | 404 | no row with that id (or not visible to the caller) |
| `missing_token` | 401 | a token is needed |
| `forbidden` | 403 | the permission is missing (`permission` says which) |
| `missing_tenant` | 400 | `scope: tenant` without `X-Tenant-Id` |
| `invalid_offset` | 400 | a bad `offset` |
| `needs_auth`, `needs_rbac` | 503 | `access` needs a plugin that isn't in `app.yaml` |

## Events

`resource.created {resource, id, item}`, `resource.updated {resource, id, item, changed}`, `resource.deleted {resource, id}`.

## Tables

`resource_<name>` per resource.
