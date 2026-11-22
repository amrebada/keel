# 3. Roles and permissions

Shelf has three kinds of users: readers browse the list, librarians add and edit books, and admins do everything. Open `app.yaml` and add the `rbac` plugin under `plugins:`:

```yaml
plugins:
  - use: ping@^0.1
  - use: auth-core@^0.1
  - use: rbac
    config:
      roles:
        admin: { permissions: ["*"] }
        librarian: { permissions: ["books.*"], inherits: [reader] }
        reader: { permissions: ["books.read"] }
      default_role: reader
      admins: [ada@example.com]
```

- `*` grants every permission; `books.*` grants every permission that starts with `books.`.
- `inherits` gives a role everything another role has.
- `default_role` is given to every new user.
- `admins` lists emails that get the `admin` role, at sign-up or, for accounts that already exist like Ada's, when the plugin starts.

Save. `keel dev` applies it live. Check what Ada has now:

```sh
curl -s localhost:8080/rbac/me -H "$ADA"
```

```json
{"user_id":"abfc8077-…","roles":["admin"],"permissions":["*"],"tenant":null}
```

## A second user

Sign up Bob, verify him from the log and log him in:

```sh
curl -s -XPOST localhost:8080/auth/signup \
  -d '{"email":"bob@example.com","password":"another long password"}'
TOKEN=$(grep -o 'verify?token=[0-9a-f]*' .keel/keel.log | tail -1 | cut -d= -f2)
curl -s "localhost:8080/auth/verify?token=$TOKEN"
BOB="Authorization: Bearer $(curl -s -XPOST localhost:8080/auth/login \
  -d '{"email":"bob@example.com","password":"another long password"}' | jq -r .access_token)"
curl -s localhost:8080/rbac/me -H "$BOB"
```

```json
{"user_id":"74102a47-…","roles":["reader"],"permissions":["books.read"],"tenant":null}
```

Bob got `reader` from `default_role`. You'll make him a librarian in the next step, once there is something to librarian.

## Roles in teams

Every role can also be granted inside a tenant (a team or organization). A request that sends `X-Tenant-Id: team-1` gets the user's global roles plus the roles granted in `team-1`. Shelf doesn't need teams; the [rbac page](../plugins/rbac/index.md) shows how they work.

Next: [a books API from YAML](04-books-api.md).
