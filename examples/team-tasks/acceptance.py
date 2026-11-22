#!/usr/bin/env python3
"""Black-box acceptance test for the Team Tasks API (see SPEC.md).

Usage: acceptance.py BASE_URL MOCK_PORT

BASE_URL   where the app listens, e.g. http://127.0.0.1:8080
MOCK_PORT  port for the mock Google provider this script runs. Start the app
           with GOOGLE_AUTH_URL=http://127.0.0.1:MOCK_PORT/authorize,
           GOOGLE_TOKEN_URL=http://127.0.0.1:MOCK_PORT/token,
           GOOGLE_USERINFO_URL=http://127.0.0.1:MOCK_PORT/userinfo,
           OAUTH_GOOGLE_CLIENT_ID=client-tt, OAUTH_GOOGLE_CLIENT_SECRET=secret-tt.

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
TOKENS = {}
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
        self.wfile.write(data)

    def do_GET(self):
        u = urllib.parse.urlparse(self.path)
        q = dict(urllib.parse.parse_qsl(u.query))
        if u.path == "/authorize":
            code = "code-" + uuid.uuid4().hex
            CODES[code] = q.get("redirect_uri")
            # A real provider redirects the browser; the test reads the code.
            return self.reply(200, {"code": code, "state": q.get("state")})
        if u.path == "/userinfo":
            tok = self.headers.get("authorization", "").removeprefix("Bearer ")
            if tok in TOKENS:
                return self.reply(200, PROFILE)
            return self.reply(401, {"error": "invalid_token"})
        self.reply(404, {})

    def do_POST(self):
        n = int(self.headers.get("content-length", 0))
        form = dict(urllib.parse.parse_qsl(self.rfile.read(n).decode()))
        if self.path == "/token":
            code = form.get("code")
            if code not in CODES:
                return self.reply(400, {"error": "invalid_grant"})
            CODES.pop(code)
            tok = "at-" + uuid.uuid4().hex
            TOKENS[tok] = True
            return self.reply(200, {"access_token": tok, "token_type": "bearer"})
        self.reply(404, {})


server = ThreadingHTTPServer(("127.0.0.1", MOCK_PORT), Mock)
threading.Thread(target=server.serve_forever, daemon=True).start()

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

s, _, b = call("GET", "/health")
expect(s, 200, "health")

# Accounts
users = {}
for name in ["boss", "alice", "bob", "carol"]:
    email = "boss@example.com" if name == "boss" else f"{name}-{RUN}@example.com"
    s, _, b = call("POST", "/auth/signup", {"email": email, "password": "secret-pass-1"})
    if name == "boss" and s == 409:  # boss exists from an earlier run
        pass
    else:
        expect(s, 201, f"signup {name}", b)
    s, _, b = call("POST", "/auth/login", {"email": email, "password": "secret-pass-1"})
    expect(s, 200, f"login {name}", b)
    tok = b["access_token"]
    s, _, me = call("GET", "/auth/me", token=tok)
    expect(s, 200, f"me {name}", me)
    check(me["user"]["email"] == email, f"me returns {name}'s email", me)
    users[name] = (me["user"]["id"], tok)
s, _, b = call("POST", "/auth/signup", {"email": f"alice-{RUN}@example.com", "password": "secret-pass-1"})
expect(s, 409, "duplicate signup", b)
s, _, b = call("POST", "/auth/login", {"email": f"alice-{RUN}@example.com", "password": "wrong-pass-9"})
expect(s, 401, "wrong password", b)
s, _, b = call("GET", "/auth/me", token="not-a-token")
expect(s, 401, "bad token", b)

# Google sign-in
s, h, b = call("GET", "/auth/oauth/google/start")
expect(s, 302, "google start redirects", b)
loc = h.get("location") or h.get("Location")
check(loc and loc.startswith(f"http://127.0.0.1:{MOCK_PORT}/authorize"), "redirect goes to the provider", loc)
q = dict(urllib.parse.parse_qsl(urllib.parse.urlparse(loc).query))
check(q.get("redirect_uri", "").endswith("/auth/oauth/google/callback"), "redirect_uri is the callback", q)
check(q.get("state"), "state is sent", q)
s, _, auth = call("GET", loc)
cb = "/auth/oauth/google/callback?" + urllib.parse.urlencode({"code": auth["code"], "state": auth["state"]})
s, _, b = call("GET", cb)
expect(s, 200, "google callback signs in", b)
gtok = b["access_token"]
s, _, me = call("GET", "/auth/me", token=gtok)
check(s == 200 and me["user"]["email"] == PROFILE["email"], "google user has the provider's email", me)
s, _, b = call("GET", cb)
check(s >= 400, "a callback cannot be replayed", s)
s, _, b = call("GET", "/auth/oauth/google/callback?code=x&state=forged")
check(s >= 400, "a forged state is refused", s)

# Teams and roles
boss_id, boss = users["boss"]
alice_id, alice = users["alice"]
bob_id, bob = users["bob"]
carol_id, carol = users["carol"]
acme, globex = f"acme-{RUN}", f"globex-{RUN}"
s, _, b = call("POST", f"/rbac/users/{alice_id}/roles", {"role": "owner", "tenant": acme}, token=boss)
expect(s, 201, "admin makes alice owner of acme", b)
s, _, b = call("POST", f"/rbac/users/{bob_id}/roles", {"role": "member", "tenant": acme}, token=alice)
expect(s, 201, "owner alice adds bob to acme", b)
s, _, b = call("POST", f"/rbac/users/{carol_id}/roles", {"role": "owner", "tenant": globex}, token=bob)
expect(s, 403, "member bob cannot grant roles", b)
s, _, b = call("POST", f"/rbac/users/{carol_id}/roles", {"role": "owner", "tenant": globex}, token=boss)
expect(s, 201, "admin makes carol owner of globex", b)

# Projects
s, _, b = call("GET", "/api/projects", team=acme)
expect(s, 401, "projects need a token", b)
s, _, b = call("POST", "/api/projects", {"name": "Launch"}, token=bob, team=acme)
expect(s, 403, "member cannot create a project", b)
s, _, p1 = call("POST", "/api/projects", {"name": "Launch", "description": "Q1 launch"}, token=alice, team=acme)
expect(s, 201, "owner creates a project", p1)
check(p1.get("id") and p1["name"] == "Launch", "project has id and name", p1)
s, _, b = call("POST", "/api/projects", {"description": "no name"}, token=alice, team=acme)
expect(s, 422, "project without a name", b)
s, _, b = call("GET", "/api/projects", token=bob, team=acme)
expect(s, 200, "member lists projects", b)
check(b["total"] == 1 and b["items"][0]["id"] == p1["id"], "list has the project", b)
s, _, b = call("GET", "/api/projects", token=carol, team=globex)
check(s == 200 and b["total"] == 0, "another team sees no projects", b)
s, _, b = call("GET", "/api/projects", token=carol, team=acme)
expect(s, 403, "carol has no role in acme", b)
s, _, b = call("GET", "/api/projects", token=users["boss"][1], team=acme)
expect(s, 200, "admin reads any team", b)
s, _, b = call("GET", "/api/projects", token=gtok, team=acme)
expect(s, 403, "a new google user has no team role", b)

# Tasks
tasks = []
for title, status, due in [("Write copy", "todo", "2027-03-01"), ("Ship site", "doing", "2027-02-01"), ("Plan party", "done", None)]:
    body = {"title": title, "status": status, "assignee": bob_id, "project_id": p1["id"]}
    if due:
        body["due"] = due
    s, _, t = call("POST", "/api/tasks", body, token=bob, team=acme)
    expect(s, 201, f"member creates task '{title}'", t)
    tasks.append(t)
s, _, t = call("POST", "/api/tasks", {"title": "Default status"}, token=bob, team=acme)
expect(s, 201, "task with defaults", t)
check(t["status"] == "todo", "status defaults to todo", t)
for bad, what in [({"status": "todo"}, "missing title"), ({"title": "x", "status": "later"}, "unknown status"),
                  ({"title": "x", "due": "2027-13-45"}, "impossible date"), ({"title": "x", "owner": 1}, "unknown field")]:
    s, _, b = call("POST", "/api/tasks", bad, token=bob, team=acme)
    expect(s, 422, f"task with {what}", b)
s, _, b = call("GET", "/api/tasks?status=doing", token=bob, team=acme)
check(s == 200 and [x["title"] for x in b["items"]] == ["Ship site"], "filter by status", b)
s, _, b = call("GET", "/api/tasks?sort=due&due.null=false", token=bob, team=acme)
check(s == 200 and [x["title"] for x in b["items"]] == ["Ship site", "Write copy"], "sort by due date", b)
s, _, b = call("GET", "/api/tasks?limit=2&sort=title", token=bob, team=acme)
check(s == 200 and b["total"] == 4 and len(b["items"]) == 2, "paging", b)
tid = tasks[0]["id"]
s, _, b = call("GET", f"/api/tasks/{tid}", token=bob, team=acme)
check(s == 200 and b["title"] == "Write copy", "get one task", b)
s, _, b = call("PATCH", f"/api/tasks/{tid}", {"status": "done"}, token=bob, team=acme)
check(s == 200 and b["status"] == "done" and b["title"] == "Write copy", "member updates a task", b)
s, _, b = call("GET", f"/api/tasks/{tid}", token=bob, team=acme)
check(b["status"] == "done", "reads after a write are fresh (cache cleared)", b)
s, _, b = call("GET", "/api/tasks?status=done", token=bob, team=acme)
check(len(b["items"]) == 2, "lists after a write are fresh", b)
s, _, b = call("GET", f"/api/tasks/{tid}", token=carol, team=globex)
expect(s, 404, "another team cannot see the task", b)
s, _, b = call("DELETE", f"/api/tasks/{tid}", token=bob, team=acme)
expect(s, 403, "member cannot delete", b)
s, _, b = call("DELETE", f"/api/tasks/{tid}", token=alice, team=acme)
expect(s, 204, "owner deletes", b)
s, _, b = call("GET", f"/api/tasks/{tid}", token=bob, team=acme)
expect(s, 404, "deleted task is gone", b)
s, _, b = call("GET", "/api/tasks", token=bob)
expect(s, 400, "tasks need a team header", b)

print(f"\nall {CHECKS} checks passed")
server.shutdown()
