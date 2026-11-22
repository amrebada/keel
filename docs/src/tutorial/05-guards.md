# 5. Guards, rate limits and validation

Plugins decide what their routes do. `routes:` in `app.yaml` decides who may call them. Each rule matches a route pattern and can add **guards** and **validation** in front of it, for any plugin's routes.

Add a top-level `routes:` block to `app.yaml`:

```yaml
routes:
  "POST /api/books":
    guard: ["rate-limit:3/min"]
  "DELETE /api/books/{id}":
    guard: ["role:admin"]
  "GET /api/books":
    validate:
      query: { type: object, properties: { limit: { type: integer, maximum: 50 } } }
```

Save. The log shows the change:

```text
keel dev: applied live (version 5; added [], removed [], changed ["resource", "routes"])
```

## Rate limit

Three creates a minute per user is low on purpose, so you can see it:

```sh
for i in 1 2 3 4; do
  curl -s -o /dev/null -w "%{http_code} " -XPOST localhost:8080/api/books -H "$BOB" \
    -d "{\"title\":\"Book $i\",\"author\":\"A\"}"
done
```

```text
201 201 201 429
```

```json
{"error":{"code":"rate_limited","message":"at most 3 requests per 60s; try again later","limit":3,"period_secs":60}}
```

Limits count per signed-in user, or per client address for anonymous calls, and use the cache provider, so they're shared across instances once the cache is Redis.

## Admin-only delete

`librarian` has `books.delete` through `books.*`, so the resource plugin would let Bob delete. The route rule adds a second check, and **every** matching rule must pass:

```sh
ID=$(curl -s "localhost:8080/api/books?title=Book%201" -H "$BOB" | jq -r '.items[0].id')
curl -s -XDELETE localhost:8080/api/books/$ID -H "$BOB"
```

```json
{"error":{"code":"forbidden","message":"this needs the `admin` role","role":"admin"}}
```

```sh
curl -s -o /dev/null -w "%{http_code}\n" -XDELETE localhost:8080/api/books/$ID -H "$ADA"   # 204
```

## Query validation

```sh
curl -s "localhost:8080/api/books?limit=500" -H "$BOB"
```

```json
{"error":{"code":"validation_failed","message":"1 problem(s) in the request","issues":[
  {"path":"query.limit","message":"500 is greater than the maximum of 50"}]}}
```

Query values are converted to the property's type before checking. `validate.body` takes a JSON Schema for the request body in the same way.

## Everything a guard can be

| Guard | Passes when |
| --- | --- |
| `authenticated` | the request has a valid access token |
| `role:admin` | the user has the role (globally or in the `X-Tenant-Id` tenant) |
| `permission:books.write` | the user has the permission through a role or a direct grant |
| `owner:id` | the user's id equals the `{id}` segment of the pattern |
| `rate-limit:30/min` | the caller made fewer than 30 calls this minute (`s`, `min`, `hour`, `day`, or e.g. `10min`) |
| `any(role:admin, owner:id)` | any of the inner guards passes |
| `all(...)` | all of the inner guards pass |
| `plan:pro` | offered by billing-stripe: the tenant's subscription includes `pro` |

Patterns: `"[METHOD ]/path"`, where `*` matches one segment, `**` the rest, and `{name}` captures a segment. `keel config validate` rejects a guard it can't parse, and one that needs a plugin you haven't added. More in [Routes, guards and validation](../concepts/routes-and-guards.md).

Next: [private notes per user](06-private-notes.md).
