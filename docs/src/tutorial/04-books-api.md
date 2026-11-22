# 4. A books API from YAML

The `resource` plugin turns a model in `app.yaml` into a table, its migrations and a REST API. Add it under `plugins:`:

```yaml
  - use: resource
    config:
      resources:
        books:
          fields:
            title: { type: string, required: true, max_length: 200 }
            author: { type: string, required: true }
            isbn: { type: string, unique: true, pattern: "^[0-9-]{10,17}$" }
            status: { type: string, enum: [want, reading, done], default: want }
            pages: { type: integer, min: 1 }
            finished_on: { type: date }
```

Save, and check it before trusting it:

```sh
keel config validate
```

```text
ok: app.yaml is valid (app 'shelf', 4 provider(s), 4 plugin(s), 0 route rule(s))
```

`keel dev` creates the `resource_books` table (one migration per table and per field) and starts serving:

| Route | What it does | Permission needed |
| --- | --- | --- |
| `GET /api/books` | List, with filters, sort and paging | `books.read` |
| `POST /api/books` | Create (`201`) | `books.write` |
| `GET /api/books/{id}` | Read one | `books.read` |
| `PATCH /api/books/{id}` | Update some fields | `books.write` |
| `DELETE /api/books/{id}` | Delete (`204`) | `books.delete` |
| `GET /api` | Describe every resource and its create schema | none |

Resources are **deny by default**: without a token you get `401`, and without the permission `403`. The permissions are the ones you defined in rbac, which is why `librarian` has `books.*`.

## Create books

Ada is an admin, so she can write:

```sh
curl -s -XPOST localhost:8080/api/books -H "$ADA" \
  -d '{"title":"Dune","author":"Frank Herbert","isbn":"978-0441013593","pages":412}'
```

```json
{"id":"4a788db8-…","title":"Dune","author":"Frank Herbert","isbn":"978-0441013593",
 "pages":412,"status":"want","finished_on":null,"created_at":1795332552587,"updated_at":1795332552587}
```

`status` got its default, and `id`, `created_at` and `updated_at` are added for you.

```sh
curl -s -XPOST localhost:8080/api/books -H "$ADA" \
  -d '{"title":"Piranesi","author":"Susanna Clarke","status":"reading","pages":272}'
```

## Validation and conflicts

Bad input gets a `422` that names every problem:

```sh
curl -s -XPOST localhost:8080/api/books -H "$ADA" \
  -d '{"title":"Untitled","author":"x","status":"lost","pages":0}'
```

```json
{"error":{"code":"validation_failed","message":"2 problem(s) in the request","issues":[
  {"path":"body.pages","message":"0 is less than the minimum of 1"},
  {"path":"body.status","message":"\"lost\" is not one of \"want\", \"reading\" or 2 other candidates"}]}}
```

A duplicate unique value is a `409`:

```sh
curl -s -XPOST localhost:8080/api/books -H "$ADA" \
  -d '{"title":"Dune again","author":"F","isbn":"978-0441013593"}'
```

```json
{"error":{"code":"conflict","field":"isbn","message":"another books has this isbn"}}
```

## Query the list

```sh
curl -s "localhost:8080/api/books?status=want&sort=-pages" -H "$ADA"
curl -s "localhost:8080/api/books?pages.gte=300&title.contains=un" -H "$ADA"
```

```json
{"items":[{"id":"4a788db8-…","title":"Dune", …}],"total":1,"limit":20,"offset":0}
```

Filters: `field=value`, `field.ne`, `.gt`, `.gte`, `.lt`, `.lte`, `.contains`, `.in=a,b` and `.null=true`. Sorting: `sort=-created_at,title`. Paging: `limit` (default 20, at most 100) and `offset`.

## Permissions in action

Bob is a reader. He can list but not create:

```sh
curl -s "localhost:8080/api/books?limit=1" -H "$BOB" | jq '{total}'      # {"total":2}
curl -s -XPOST localhost:8080/api/books -H "$BOB" -d '{"title":"X","author":"Y"}'
```

```json
{"error":{"code":"forbidden","message":"this needs the `books.write` permission","permission":"books.write"}}
```

Ada makes him a librarian through rbac's API (it needs the `rbac.manage` permission, which her `*` covers):

```sh
BOB_ID=$(curl -s localhost:8080/auth/me -H "$BOB" | jq -r .user.id)
curl -s -XPOST localhost:8080/rbac/users/$BOB_ID/roles -H "$ADA" -d '{"role":"librarian"}'
```

```json
{"user_id":"74102a47-…","roles":[{"role":"librarian","tenant":null},{"role":"reader","tenant":null}],"permissions":[]}
```

Roles are checked on every request, so Bob's existing token works right away:

```sh
curl -s -XPOST localhost:8080/api/books -H "$BOB" \
  -d '{"title":"The Hobbit","author":"J.R.R. Tolkien"}' | jq '{id, title}'
```

## Changing the model later

Add a field and save: a new column is added by a new migration. Changing a field's rules (`max_length`, `enum`, `required`) needs no migration. To change a field's type, add a field with a new name. The [resource page](../plugins/resource/index.md) covers caching, `access: public`, custom permission names and more.

Next: [guards, rate limits and validation](05-guards.md).
