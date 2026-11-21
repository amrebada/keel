#!/usr/bin/env python3
"""Black-box acceptance test for the follow-up change (see CHANGE.md).

Usage: acceptance_change.py BASE_URL

Run against the app after the change. Only the standard library is used.
Exits 0 when every check passes.
"""

import json
import sys
import urllib.error
import urllib.parse
import urllib.request
import uuid

BASE = sys.argv[1].rstrip("/")
RUN = uuid.uuid4().hex[:8]  # unique emails and teams, so reruns work
CHECKS = 0

# ---- HTTP helpers ---------------------------------------------------------


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *a, **k):
        return None


OPENER = urllib.request.build_opener(NoRedirect)


def call(method, path, body=None, token=None, team=None):
    url = path if path.startswith("http") else BASE + path
    data = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request(url, data=data, method=method)
    if data is not None:
        req.add_header("content-type", "application/json")
    if token:
        req.add_header("authorization", "Bearer " + token)
    if team:
        req.add_header("x-tenant-id", team)
    try:
        with OPENER.open(req, timeout=15) as r:
            raw = r.read()
            return r.status, dict(r.headers), (json.loads(raw) if raw else None)
    except urllib.error.HTTPError as e:
        raw = e.read()
        try:
            parsed = json.loads(raw) if raw else None
        except ValueError:
            parsed = raw.decode(errors="replace")
        return e.code, dict(e.headers), parsed


def check(cond, what, detail=None):
    global CHECKS
    if not cond:
        print(f"FAIL: {what}" + (f"\n  got: {detail}" if detail is not None else ""))
        sys.exit(1)
    CHECKS += 1
    print(f"ok   {what}")


def expect(status, want, what, body=None):
    check(status == want, f"{what} -> {want}", f"{status} {body}")


# ---- the checks -----------------------------------------------------------

def account(email):
    s, _, b = call("POST", "/auth/signup", {"email": email, "password": "secret-pass-1"})
    check(s in (201, 409), f"signup {email}", b)
    s, _, b = call("POST", "/auth/login", {"email": email, "password": "secret-pass-1"})
    expect(s, 200, f"login {email}", b)
    tok = b["access_token"]
    s, _, me = call("GET", "/auth/me", token=tok)
    return me["user"]["id"], tok


boss_id, boss = account("boss@example.com")
alice_id, alice = account(f"alice-{RUN}@example.com")
bob_id, bob = account(f"bob-{RUN}@example.com")
vera_id, vera = account(f"vera-{RUN}@example.com")
carol_id, carol = account(f"carol-{RUN}@example.com")
acme, globex = f"acme-{RUN}", f"globex-{RUN}"

# Roles, including the new viewer role
s, _, b = call("POST", f"/rbac/users/{alice_id}/roles", {"role": "owner", "tenant": acme}, token=boss)
expect(s, 201, "admin makes alice owner of acme", b)
s, _, b = call("POST", f"/rbac/users/{bob_id}/roles", {"role": "member", "tenant": acme}, token=alice)
expect(s, 201, "owner adds bob as member", b)
s, _, b = call("POST", f"/rbac/users/{vera_id}/roles", {"role": "viewer", "tenant": acme}, token=alice)
expect(s, 201, "owner adds vera as viewer", b)
s, _, b = call("POST", f"/rbac/users/{carol_id}/roles", {"role": "owner", "tenant": globex}, token=boss)
expect(s, 201, "admin makes carol owner of globex", b)

# Existing behaviour still works
s, _, p1 = call("POST", "/api/projects", {"name": "Launch"}, token=alice, team=acme)
expect(s, 201, "owner creates a project", p1)
s, _, b = call("POST", "/api/projects", {"name": "Nope"}, token=bob, team=acme)
expect(s, 403, "member still cannot create a project", b)

# Task priority
made = {}
for title, prio in [("Low", 1), ("High", 5), ("Mid", 3)]:
    s, _, t = call("POST", "/api/tasks", {"title": title, "priority": prio, "project_id": p1["id"]}, token=bob, team=acme)
    expect(s, 201, f"task with priority {prio}", t)
    check(t.get("priority") == prio, "priority is returned", t)
    made[title] = t
s, _, t = call("POST", "/api/tasks", {"title": "None"}, token=bob, team=acme)
expect(s, 201, "priority is optional", t)
for bad in [0, 6, "high"]:
    s, _, b = call("POST", "/api/tasks", {"title": "x", "priority": bad}, token=bob, team=acme)
    expect(s, 422, f"priority {bad!r} is refused", b)
s, _, b = call("GET", "/api/tasks?priority=3", token=bob, team=acme)
check(s == 200 and [x["title"] for x in b["items"]] == ["Mid"], "filter by priority", b)
s, _, b = call("GET", "/api/tasks?sort=-priority&priority.null=false", token=bob, team=acme)
check(s == 200 and [x["title"] for x in b["items"]] == ["High", "Mid", "Low"], "sort by priority, highest first", b)
tid = made["Mid"]["id"]
s, _, b = call("PATCH", f"/api/tasks/{tid}", {"priority": 4}, token=bob, team=acme)
check(s == 200 and b["priority"] == 4 and b["title"] == "Mid", "update priority", b)

# Viewer: read only
s, _, b = call("GET", "/api/tasks", token=vera, team=acme)
check(s == 200 and b["total"] == 4, "viewer lists tasks", b)
s, _, b = call("GET", "/api/projects", token=vera, team=acme)
expect(s, 200, "viewer lists projects", b)
s, _, b = call("POST", "/api/tasks", {"title": "x"}, token=vera, team=acme)
expect(s, 403, "viewer cannot create a task", b)
s, _, b = call("PATCH", f"/api/tasks/{tid}", {"priority": 1}, token=vera, team=acme)
expect(s, 403, "viewer cannot update a task", b)
s, _, b = call("POST", f"/rbac/users/{carol_id}/roles", {"role": "member", "tenant": acme}, token=vera)
expect(s, 403, "viewer cannot grant roles", b)

# Comments
s, _, b = call("GET", "/api/comments", team=acme)
expect(s, 401, "comments need a token", b)
s, _, b = call("GET", "/api/comments", token=bob)
expect(s, 400, "comments need a team header", b)
s, _, c1 = call("POST", "/api/comments", {"task_id": tid, "body": "Looks good"}, token=bob, team=acme)
expect(s, 201, "member comments", c1)
check(c1.get("id") and c1["body"] == "Looks good" and c1["task_id"] == tid, "comment has id, body and task_id", c1)
s, _, c2 = call("POST", "/api/comments", {"task_id": made["Low"]["id"], "body": "Later"}, token=alice, team=acme)
expect(s, 201, "owner comments", c2)
for bad, what in [({"task_id": tid}, "no body"), ({"body": "hi"}, "no task_id"),
                  ({"task_id": tid, "body": ""}, "empty body"), ({"task_id": tid, "body": "x" * 251}, "body over 250"),
                  ({"task_id": tid, "body": "hi", "pinned": True}, "unknown field")]:
    s, _, b = call("POST", "/api/comments", bad, token=bob, team=acme)
    expect(s, 422, f"comment with {what}", b)
s, _, b = call("POST", "/api/comments", {"task_id": tid, "body": "hi"}, token=vera, team=acme)
expect(s, 403, "viewer cannot comment", b)
s, _, b = call("GET", f"/api/comments?task_id={tid}", token=vera, team=acme)
check(s == 200 and [x["id"] for x in b["items"]] == [c1["id"]], "viewer reads a task's comments", b)
s, _, b = call("GET", "/api/comments", token=carol, team=globex)
check(s == 200 and b["total"] == 0, "another team sees no comments", b)
s, _, b = call("GET", f"/api/comments/{c1['id']}", token=carol, team=globex)
expect(s, 404, "another team cannot read the comment", b)
s, _, b = call("DELETE", f"/api/comments/{c1['id']}", token=bob, team=acme)
expect(s, 403, "member cannot delete a comment", b)
s, _, b = call("DELETE", f"/api/comments/{c1['id']}", token=alice, team=acme)
expect(s, 204, "owner deletes a comment", b)
s, _, b = call("GET", f"/api/comments?task_id={tid}", token=bob, team=acme)
check(s == 200 and b["total"] == 0, "deleted comment is gone from the list (cache cleared)", b)
s, _, b = call("GET", "/api/comments", token=boss, team=acme)
check(s == 200 and b["total"] == 1, "admin reads any team's comments", b)

print(f"\nall {CHECKS} checks passed")
