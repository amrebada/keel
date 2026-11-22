# 6. Private notes per user

Readers want to keep notes on books that nobody else sees. That's a second resource with `scope: owner`. Add it under `resources:`, next to `books`:

```yaml
        notes:
          scope: owner
          access: authenticated
          fields:
            book_id: { type: string, required: true }
            body: { type: text, required: true }
```

- `scope: owner` stores the creator's user id in an `owner_id` column and limits every list, read, update and delete to the caller's own rows.
- `access: authenticated` lets any signed-in user call the routes, instead of needing `notes.read` and `notes.write` permissions.
- `text` is for long strings; `string` is capped at 255 characters.

Save, then have both users write a note:

```sh
curl -s -XPOST localhost:8080/api/notes -H "$BOB" -d '{"book_id":"x","body":"Bob note"}'
curl -s -XPOST localhost:8080/api/notes -H "$ADA" -d '{"book_id":"x","body":"Ada note"}'
```

```json
{"id":"96add316-…","book_id":"x","body":"Bob note","owner_id":"74102a47-…","created_at":…,"updated_at":…}
```

Each sees only their own:

```sh
curl -s localhost:8080/api/notes -H "$BOB" | jq '[.items[].body]'
```

```json
["Bob note"]
```

Ada is different: holders of the `notes.manage` permission see and change every row, and her `*` includes it.

```sh
curl -s localhost:8080/api/notes -H "$ADA" | jq '[.items[].body]'
```

```json
["Bob note","Ada note"]
```

Rename that permission per resource with `manage_permission`.

The third scope, `scope: tenant`, keeps rows per `X-Tenant-Id` team, with unique values checked per team. That's how the [team-tasks example](https://github.com/amrebada/keel/tree/main/examples/team-tasks) keeps each team's tasks apart.

Next: [write your own plugin](07-custom-plugin.md).
