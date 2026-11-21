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

- A new per-team resource `comments`, with the same routes, query options,
  team rules, caching and error codes as projects and tasks
  (`/api/comments`, `/api/comments/{id}`).
- Fields: `task_id` (string, required), `body` (string, 1 to 250 characters,
  required). Unknown fields are `422`.
- Read: viewer, member, owner, admin. Create and update: member, owner,
  admin. Delete: owner, admin.
