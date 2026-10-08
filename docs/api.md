# `mc api serve` — REST API

A bearer-authenticated HTTP/JSON surface that mirrors the MCP tool set. Lets non-LLM clients (custom UIs, mobile apps, CI bots, the swarm gateway) drive mc without forking a process per call.

The API is **single-tenant**: it serves one repository, one process, one mutex. Multi-tenant scoping is not built in — it belongs in a thin gateway in front (see [§7](#7-pattern-multi-tenant-scoping-via-a-gateway)).

---

## 1. Quick start

The fastest path — zero token setup, just for local development:

```bash
mc init /tmp/demo --name DemoCo
mc --root /tmp/demo api serve --insecure-dev-token
```

The server prints the random bearer token to stderr at startup. Open the
interactive docs at <http://127.0.0.1:5100/v1/docs> in your browser.

For anything beyond local dev, use a tokens file:

```bash
HASH=$(mc api hash-token "correct horse battery staple")
cat > /tmp/demo/tokens.yml <<EOF
tokens:
  - name: deploy-bot
    hash: "$HASH"
    capabilities: [read, write]
EOF
mc --root /tmp/demo api serve --tokens-file /tmp/demo/tokens.yml --port 5100
```

Then:

```bash
T="correct horse battery staple"
curl -s http://127.0.0.1:5100/healthz
curl -s -H "Authorization: Bearer $T" http://127.0.0.1:5100/v1/config
curl -s -H "Authorization: Bearer $T" -H "Content-Type: application/json" \
  -X POST http://127.0.0.1:5100/v1/customers \
  -d '{"name":"Acme","status":"active"}'
```

Useful URLs once the server is running:

| URL | What |
|---|---|
| `/v1/docs` | Interactive API docs (RapiDoc, no auth) |
| `/v1/openapi.json` | OpenAPI 3.1 spec (no auth) |
| `/healthz` | Liveness probe (no auth) |
| `/readyz` | Readiness probe (no auth) |

---

## 2. Authentication

Every authenticated request carries `Authorization: Bearer <secret>`. Tokens are compared against argon2id hashes loaded once at startup from `--tokens-file`.

A bearer is verified with argon2 only the first time it is seen; after that its result (match or failure) is cached in memory. First-sight verifications run off the async workers, at most two at a time, so a flood of random bearers can't stall other requests; if a request waits more than 5 s for a slot it gets `503 unavailable` and may retry.

### Token file format (YAML)

```yaml
tokens:
  - name: deploy-bot         # appears in logs, not in responses
    hash: $argon2id$v=19$m=19456,t=2,p=1$...
    capabilities: [read, write]
  - name: read-only-dashboard
    hash: $argon2id$v=19$m=19456,t=2,p=1$...
    capabilities: [read]
```

- `capabilities` is `[read]`, `[read, write]`, or omitted (defaults to `[read]`).
- An empty `tokens:` list is a startup error — you must define at least one.
- Tokens are not hot-reloaded today. To rotate, edit the file and restart the server. SIGHUP-reload is on the roadmap.

### Generating a hash

```bash
mc api hash-token "your-secret"
# or pipe from stdin:
echo -n "your-secret" | mc api hash-token
```

### `--read-only` flag

Even a token with `[read, write]` cannot write when the server runs with `--read-only`. Use this for sidecars that should never mutate the repo.

### `--insecure-dev-token` flag

Mutually exclusive with `--tokens-file`. Generates a random read+write bearer token at startup, prints it to stderr with a loud warning, and serves until you SIGTERM. Single-token, in-memory, never written to disk.

For local development only. Anyone reading the terminal stream gets full access to the repo.

### Bind address

Default `127.0.0.1`. Only set `--bind 0.0.0.0` behind a trusted reverse proxy that terminates TLS and enforces network policy. The API has no built-in TLS or rate limiting.

---

## 3. Endpoints

| Method | Path | Capability | Description |
|---|---|---|---|
| GET | `/healthz` | none | Liveness — process is alive. |
| GET | `/readyz` | none | Readiness — repo root is accessible. |
| GET | `/v1/openapi.json` | none | OpenAPI 3.1 spec, generated from the typed handlers. |
| GET | `/v1/config` | read | Repo `name` (`brand.name`, else `site.name`), mode, prefixes, valid statuses, configured path keys, `available_kinds`. |
| GET | `/v1/status` | read | Counts by status per kind + recent activity. |
| GET | `/v1/entities/{kind}` | read | List with optional `?status=&tag=`. Each entry is the frontmatter plus `_kind` and `_source` (file path relative to the repo root). |
| GET | `/v1/entities/{kind}/{id}` | read | Parsed entity: `{kind, id, source_path, frontmatter, body_preview}`, `source_path` relative to the repo root. |
| GET | `/v1/entities/{kind}/{id}/raw` | read | Raw markdown (`text/markdown`). |
| GET | `/v1/tasks` | read | List with full filter set: `status`, `tag`, `project`, `customer`, `priority`, `sprint`, `milestone`, `owner`. `milestone` takes an ID or unique title; an empty value (`?milestone=`) lists tasks without a milestone, and an unknown milestone is a `404`. Entries are shaped like `/v1/entities/task`. |
| GET | `/v1/tasks/next` | read | What to work on next, like `mc task next`: open tasks (`todo`, `backlog`) whose dependencies are done or cancelled, best first (todo before backlog, then priority, due date, ID). Optional `?project=&customer=&owner=&limit=` (default 5). Returns `{tasks, actionable, blocked}`: the tasks shaped like `/v1/tasks`, the number of actionable tasks before `limit`, and the number of open tasks waiting on dependencies. |
| POST | `/v1/customers` | write | Create. Body: `{name, owner?, status?, tags?}`. |
| POST | `/v1/projects` | write | `{name, owner?, status?, customers?, tags?}` |
| POST | `/v1/meetings` | write | `{title, date?, time?, duration?, status?, tags?, customers?, projects?, attendees?}` |
| POST | `/v1/research` | write | `{title, owner?, agents?, tags?}` |
| POST | `/v1/tasks` | write | `{title, project?, customer?, owner?, status?, priority?, tags?, sprint?, milestone?, depends_on?, due_date?}` |
| POST | `/v1/sprints` | write | `{title, owner?, status?, goal?, start_date?, end_date?, projects?, tags?}` |
| POST | `/v1/milestones` | write | `{title, description?, start_date?, due_date?, owner?, status?, projects?}` — a milestone (work package) that groups tasks. Status defaults to `planned`; `due_date` may not be before `start_date`. Assign tasks with `milestone` on `POST /v1/tasks` or `PATCH /v1/tasks/{id}`; list milestones with `GET /v1/entities/milestones`. |
| POST | `/v1/proposals` | write | `{title, author?, status?, type?, tags?, supersedes?}` |
| POST | `/v1/contacts` | write | `{name, customer, role?, email?, phone?, status?, tags?}` |
| PATCH | `/v1/tasks/{id}` | write | `{title?, status?, priority?, owner?, sprint?, milestone?, due_date?, projects?, customers?, tags?, depends_on?}` — change only the given fields (same as `mc task set` and the MCP `update_task` tool). Empty values clear (`""`, `[]`); list fields replace the whole list; `project`/`customer` are accepted for `projects`/`customers`. Validated like a create (references must exist, `404` otherwise) and nothing is written if a value is invalid; unknown fields are a `400`. A status change moves the file like `move`. Returns `{id, changed, old_status, new_status, path}`, `changed` listing the fields whose value actually changed. |
| POST | `/v1/tasks/{id}/move` | write | `{status, sprint?}` — also moves the file between `todo/` and `done/` if the status crosses the active boundary. Returns `{id, old_status, new_status, path}`. Only tasks can be moved. |
| GET | `/v1/entities/{kind}/{id}/checklist` | read | `- [ ]` checklist items `{id, items: [{index, line, checked, text}], done, total}` (code blocks and task/meeting comments excluded). |
| POST | `/v1/entities/{kind}/{id}/checklist/{item}` | write | `{checked?, expect_text?}` — tick (default) or untick item `item` (1-based). Only the box character changes. A stale `expect_text` is `409 conflict`. |
| POST | `/v1/entities/{kind}/{id}/comments` | write | `{text, author?}` — comment on a task or meeting; appended under `## Comments` as `### YYYY-MM-DD HH:MM · Author`. `201` with `{id, comment, count, path}`. |
| POST | `/v1/index` | write | Rebuild the JSON index files under `data/`. |
| GET, POST | `/v1/validate` | read | Run `mc validate`; returns issues as JSON. Read-only tokens and `--read-only` servers may call it with either method. |

`kind` accepts singular or plural forms (`customer`/`customers`, `task`/`tasks`, etc.). IDs in paths are forgiving like the CLI's: case and zero-padding don't matter and a bare number takes the path's kind (`/v1/entities/task/task-7`, `/v1/entities/task/7` and `/v1/tasks/TASK-0007/move` all mean `TASK-007`). Statuses in `move` and in `?status=` filters ignore case and `_`/space for `-`, and accept the CLI's aliases (`doing`/`wip` → `in-progress`, `completed` → `done`, `canceled` → `cancelled`). References in create and PATCH bodies (`project`, `customer(s)`, `projects`, `depends_on`, `supersedes`, `sprint`, `milestone`) are just as forgiving and are stored canonically (`task-1` → `TASK-001`; a sprint or milestone may be given by ID or unique title and is stored as its ID); they must name an existing entity.

Every `path`, `source_path` and `_source` in a response is relative to the repo root (`tasks/todo/TASK-001-x.md`), so responses don't reveal where the repo lives on the server; error details name files the same way. Cross-reference fields (`customers`, `projects`, `depends_on`, `sprint`, `milestone`, …) come without wiki-link brackets (`PROJ-001`, not `[[PROJ-001]]`). List fields (`tags`, `customers`, `projects`, `attendees`, `agents`, `depends_on`) accept either a comma-separated string (`"a,b"`, the CLI convention) or a JSON array of strings (`["a", "b"]`). Both forms are split on commas, so items cannot contain a comma.

Create bodies are validated before anything is written:

- Omitted or blank `status` falls back to the kind's first configured status; any other value must be one of the configured statuses.
- Dates (`date`, `start_date`, `end_date`, `due_date`) must be `YYYY-MM-DD`; a sprint's `end_date` and a milestone's `due_date` may not be before its `start_date`.
- `priority` must be 1-4.
- Referenced scopes (`project`/`customer` on tasks, `customer` on contacts) must exist. IDs may be given plain (`CUST-001`) or wiki-linked (`[[CUST-001]]`).
- Two meetings with the same date and title get distinct files (`…-standup.md`, `…-standup-2.md`) instead of overwriting each other.

Violations return `400 bad-request` with a message starting with `Invalid …` (or `404` for a missing scope).

A successful create returns `201 Created` and `{id, name, path}` (`path` relative to the repo root). For kinds whose primary field is `title` (meetings, research, tasks, sprints, milestones, proposals), the `name` field of the response carries the title — the server normalizes the payload so consumers always see `name`.

---

## 4. Error model (RFC 7807)

Every error is `application/problem+json` — including a body that isn't valid JSON or misses a field, a query parameter of the wrong type, an unknown path, a wrong method (with an `Allow` header), an oversized body and a timeout:

```json
{
  "type": "https://docs.mc.dev/errors/bad-request",
  "title": "Bad request",
  "status": 400,
  "detail": "'frob' is not a valid task status (valid statuses: backlog, todo, in-progress, review, done, cancelled)"
}
```

Stable `type` URIs:

| `type` | Status | When |
|---|---|---|
| `unauthenticated` | 401 | Missing or invalid bearer token. |
| `forbidden` | 403 | Read-only mode, missing write capability, kind unavailable in repo mode. |
| `bad-request` | 400 | Invalid status, priority or date, a body or query string that can't be read (bad JSON, missing or mistyped field), unknown entity kind, empty name. |
| `invalid-id` | 400 | ID prefix doesn't match any configured kind (the detail suggests the closest one). |
| `method-not-allowed` | 405 | The path exists but not for this method. |
| `payload-too-large` | 413 | Body over 64 KiB. |
| `unsupported-media-type` | 415 | A body sent without `Content-Type: application/json`. |
| `timeout` | 408 | The request took longer than 30 s. |
| `unavailable` | 503 | Too many first-sight token verifications waiting, or `/readyz` found the repo unreachable. |
| `entity-not-found` | 404 | No entity with that ID. |
| `not-found` | 404 | Another requested thing does not exist (e.g. a research report file, or a `project`/`customer`/`sprint`/`milestone`/`depends_on` reference in a create, `PATCH` or task filter), or no endpoint at that path. |
| `frontmatter` | 400 | Frontmatter parse error during read-modify-write. |
| `validation` | 422 | `mc validate` found issues (used by CLI; the `/v1/validate` endpoint returns 200 with `ok: false` instead). |
| `not-available` | 403 | Kind not enabled in this repo (customer in embedded mode, or a kind missing from `paths:` in a standalone config). |
| `template-not-found` | 500 | A required template is missing. |
| `already-initialized` | 409 | `mc init` re-init without `--force`. |
| `conflict` | 409 | A checklist item's text no longer matches `expect_text` (the file changed). |
| `repo-not-found` | 500 | Repo path no longer exists. |
| `internal` | 500 | Unexpected error — see server logs. |

Some validation messages are reported as `bad-request` instead of `validation` because they originate from per-handler input checks (e.g. `move_task` rejecting an invalid status) rather than the bulk-validate path.

---

## 5. Concurrency

- **Reads** run unsynchronised. `util::atomic_write` makes each individual file's write atomic; readers tolerate the few-millisecond window where a concurrent writer has created a directory but the markdown file inside hasn't landed yet.
- **Writes** acquire a per-server `tokio::sync::Mutex`. One writer at a time. The mutex is held around the full read-modify-write sequence so ID allocation cannot race; the repo-wide write lock below covers writers in other processes.

At the rate this API will see (humans + a small fleet of agents), a single mutex is correct and trivially auditable. If contention ever shows up in profiling, the next step is sharding by entity kind. Don't pre-optimise.

### Cross-process safety

At startup, `mc api serve` acquires an exclusive `flock` on `<repo>/.mc-api.lock` (`<repo>/.mc/.mc-api.lock` in embedded repos; `mc init` git-ignores it). A second instance against the same repo fails fast with a clear error.

Writes from other processes (the CLI, `mc mcp`, the dashboard) are serialized by the repo-wide write lock in `src/lock.rs`: every shared write function (`create_*` from ID allocation until the file lands, task moves, comments, checklist ticks, frontmatter updates) holds an exclusive `flock` on `.git/mc-write.lock` (or `.mc-write.lock` / `.mc/.mc-write.lock` without a `.git` directory), so no two writers hand out the same ID.

### Bearer-token verification cost

Argon2id verification is intentionally slow (≈30–100 ms each). On every request the server SHA-256-hashes the bearer and looks it up in a small in-memory cache; only never-before-seen bearers pay the argon2 cost. That cost runs off the async runtime (`spawn_blocking`) behind a 2-permit semaphore; a request that waits more than 5 s for a permit gets `503 unavailable`. Failed bearers go into a bounded negative cache (4096 entries, then cleared), so repeating a bad token skips argon2 and random bearers cannot grow memory.

### Body and timeout limits

- Request bodies are capped at **64 KiB**. Even the heaviest entity create is under 4 KiB; the limit rules out an OOM via a multi-megabyte JSON body.
- Each request has a **30 s** timeout. A client that fails to send the full body in that window is dropped before it can hold the write mutex (slowloris guard).

### Embedded mode restrictions

In embedded mode (`.mc/`), the kinds `customer`, `project`, and `contact` are not available. Lists return `403 not-available`; creates return `403`.

---

## 6. Operations

- **Bind**: defaults to `127.0.0.1`. Set `0.0.0.0` only behind a trusted reverse proxy (Traefik, nginx) that terminates TLS.
- **TLS**: not built in. Use a reverse proxy.
- **Logging**: human format by default; `--log-format json` for structured output. Each request emits an `http` span with `method`, `path`, `request_id`, and the response status at INFO. Set `RUST_LOG=mc::api=debug` for more.
- **Health probes**: `/healthz` for liveness (always 200), `/readyz` for readiness (200 when the repo root is a directory; 503 otherwise). Both bypass auth.
- **Request ID**: every request gets `X-Request-Id` (UUIDv4); the header is propagated to the response. Use it to correlate client errors with server logs.
- **Graceful shutdown**: SIGINT or SIGTERM closes the listener after in-flight requests finish.
- **Single-instance per repo**: enforced by an exclusive `flock` on `<repo>/.mc-api.lock`; a second instance fails fast at startup.
- **Body limit**: 64 KiB per request.
- **Request timeout**: 30 s.
- **What to monitor**: `/v1/openapi.json` is the canonical surface — pin a snapshot in CI and assert it doesn't drift unintentionally. Watch the lib's `argon2` crate in audits — it's the hot path on first-sight requests.

---

## 7. Pattern: multi-tenant scoping via a gateway

mc is single-tenant by design. To serve N tenants from one mc instance, run a small gateway in front. The gateway holds per-tenant tokens and rewrites requests so each tenant sees only its own subtree.

### Recommended split

```
        ┌─────────────────────┐
        │  mc api serve       │   loopback only, single internal token
        │  127.0.0.1:5100     │
        └──────────▲──────────┘
                   │
        ┌──────────┴──────────┐
        │  mc-gateway         │   per-tenant tokens, scopes requests,
        │  ClusterIP:8080     │   forwards with internal token
        └──────────▲──────────┘
                   │
            tenant clients
```

### Gateway responsibilities

1. **Authenticate** the inbound request against per-tenant token storage (e.g. K8s Secret).
2. **Map** the token to a `{slug, role, customer_id}`. Roles: `admin` (Kira, full access) and `tenant` (one customer).
3. **Scope** the request before forwarding:
   - **Lists** (`GET /v1/entities/task` etc.): inject `customer=CUST-NNN-<slug>` query param, drop user-supplied conflicting filters.
   - **Creates** (`POST /v1/tasks` etc.): require `customer=CUST-NNN-<slug>` in the body; reject otherwise.
   - **Single GETs** (`GET /v1/entities/customer/CUST-001`): proxy upstream, then verify the response's `customer` (or own ID) field matches the slug. Return `404` (not `403`) for cross-tenant — avoids existence leaks.
   - **Customer / Project / Contact creates**: tenant tokens cannot create these. Return `403`.
4. **Forward** to `mc api serve` with the gateway's internal token. The internal token has `[read, write]` always.
5. **Log** every request with `slug`, `method`, `path`, `status`, `duration_ms`.

### Sketch (Go, ~30 lines)

```go
func (g *Gateway) Handle(w http.ResponseWriter, r *http.Request) {
    tok := bearer(r)
    binding, ok := g.tokens.Lookup(tok)
    if !ok { http.Error(w, "unauth", 401); return }

    if binding.Role == "tenant" {
        switch {
        case r.Method == "GET" && strings.HasPrefix(r.URL.Path, "/v1/tasks"):
            // Force customer filter on lists.
            q := r.URL.Query()
            q.Set("customer", binding.CustomerID)
            r.URL.RawQuery = q.Encode()
        case r.Method == "POST" && r.URL.Path == "/v1/tasks":
            // Force customer field in body.
            if err := injectField(r, "customer", binding.CustomerID); err != nil {
                http.Error(w, err.Error(), 400); return
            }
        case strings.HasPrefix(r.URL.Path, "/v1/entities/customer"),
             strings.HasPrefix(r.URL.Path, "/v1/entities/project"),
             strings.HasPrefix(r.URL.Path, "/v1/entities/contact"):
            if r.Method != "GET" || !strings.HasSuffix(r.URL.Path, "/"+binding.CustomerID) {
                http.Error(w, "cross-tenant", 404); return
            }
        }
    }

    r.Header.Set("Authorization", "Bearer "+g.upstreamToken)
    g.proxy.ServeHTTP(w, r)
}
```

Don't reuse the user's bearer when forwarding upstream — replace it with the gateway's internal token. Tenant identity belongs in the gateway's audit log, not the upstream's.

---

## 8. Intentionally not in the API

- **DELETE.** mc has no delete operation today. Removing a markdown file by hand still works; if you need automated removal, do it in your repo tooling (and commit it).
- **General PATCH on frontmatter.** Editing arbitrary fields without going through `mc` opens too many invariants (status/folder sync, ID stability, link integrity). Only the typed, validated task fields of `PATCH /v1/tasks/{id}` can be changed (the same set as `mc task set`); IDs, slugs and other kinds' fields are edited in the files.
- **WebSocket / SSE.** Polling `/v1/status` is enough at today's scale. Live updates can be added later with a `tokio::sync::broadcast` channel.
- **Tenant scoping.** Lives in a gateway. See §7.
- **Git commits.** Writes are pure FS, just like the CLI. Wire commits into your operator/cron — mc doesn't touch git after `init`.
- **TLS, rate limiting, CORS for public origins.** Reverse-proxy concerns.
