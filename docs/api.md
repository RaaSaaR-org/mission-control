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
| GET | `/v1/config` | read | Mode, prefixes, valid statuses, configured kinds. |
| GET | `/v1/status` | read | Counts by status per kind + recent activity. |
| GET | `/v1/entities/{kind}` | read | List with optional `?status=&tag=`. |
| GET | `/v1/entities/{kind}/{id}` | read | Parsed entity (frontmatter as JSON + body preview). |
| GET | `/v1/entities/{kind}/{id}/raw` | read | Raw markdown (`text/markdown`). |
| GET | `/v1/tasks` | read | List with full filter set: `status`, `tag`, `project`, `customer`, `priority`, `sprint`, `owner`. |
| POST | `/v1/customers` | write | Create. Body: `{name, owner?, status?, tags?}`. |
| POST | `/v1/projects` | write | `{name, owner?, status?, customers?, tags?}` |
| POST | `/v1/meetings` | write | `{title, date?, time?, duration?, status?, tags?, customers?, projects?, attendees?}` |
| POST | `/v1/research` | write | `{title, owner?, agents?, tags?}` |
| POST | `/v1/tasks` | write | `{title, project?, customer?, owner?, status?, priority?, tags?, sprint?, depends_on?, due_date?}` |
| POST | `/v1/sprints` | write | `{title, owner?, status?, goal?, start_date?, end_date?, projects?, tags?}` |
| POST | `/v1/proposals` | write | `{title, author?, status?, type?, tags?, supersedes?}` |
| POST | `/v1/contacts` | write | `{name, customer, role?, email?, phone?, status?, tags?}` |
| POST | `/v1/tasks/{id}/move` | write | `{status, sprint?}` — also moves the file between `todo/` and `done/` if the status crosses the active boundary. |
| POST | `/v1/index` | write | Rebuild the JSON index files under `data/`. |
| POST | `/v1/validate` | read | Run `mc validate`; returns issues as JSON. |

`kind` accepts singular or plural forms (`customer`/`customers`, `task`/`tasks`, etc.). Comma-separated list fields (`tags`, `customers`, `agents`, etc.) follow the same convention as the CLI.

A successful create returns `201 Created` and `{id, name, path}`. For kinds whose primary field is `title` (meetings, research, tasks, sprints, proposals), the `name` field of the response carries the title — the server normalizes the payload so consumers always see `name`.

---

## 4. Error model (RFC 7807)

Every error is `application/problem+json`:

```json
{
  "type": "https://docs.mc.dev/errors/bad-request",
  "title": "Bad request",
  "status": 400,
  "detail": "Invalid task status 'frob'. Valid statuses: backlog, todo, in-progress, review, done, cancelled"
}
```

Stable `type` URIs:

| `type` | Status | When |
|---|---|---|
| `unauthenticated` | 401 | Missing or invalid bearer token. |
| `forbidden` | 403 | Read-only mode, missing write capability, kind unavailable in repo mode. |
| `bad-request` | 400 | Invalid status, invalid JSON body, unknown entity kind, empty name. |
| `invalid-id` | 400 | ID prefix doesn't match any configured kind. |
| `entity-not-found` | 404 | No entity with that ID. |
| `frontmatter` | 400 | Frontmatter parse error during read-modify-write. |
| `validation` | 422 | `mc validate` found issues (used by CLI; the `/v1/validate` endpoint returns 200 with `ok: false` instead). |
| `not-available` | 403 | Kind not available in this repo mode (e.g. customer in embedded mode). |
| `template-not-found` | 500 | A required template is missing. |
| `already-initialized` | 409 | `mc init` re-init without `--force`. |
| `repo-not-found` | 500 | Repo path no longer exists. |
| `internal` | 500 | Unexpected error — see server logs. |

Some validation messages are reported as `bad-request` instead of `validation` because they originate from per-handler input checks (e.g. `move_task` rejecting an invalid status) rather than the bulk-validate path.

---

## 5. Concurrency

- **Reads** run unsynchronised. `util::atomic_write` makes each individual file's write atomic; readers tolerate the few-millisecond window where a concurrent writer has created a directory but the markdown file inside hasn't landed yet.
- **Writes** acquire a per-server `tokio::sync::Mutex`. One writer at a time. The mutex is held around the full read-modify-write sequence so ID allocation cannot race.

At the rate this API will see (humans + a small fleet of agents), a single mutex is correct and trivially auditable. If contention ever shows up in profiling, the next step is sharding by entity kind. Don't pre-optimise.

### Cross-process safety

At startup, `mc api serve` acquires an exclusive `flock` on `<repo>/.mc-api.lock`. A second instance against the same repo fails fast with a clear error. Without this, two processes would each have an independent mutex and hand out duplicate IDs.

### Bearer-token verification cost

Argon2id verification is intentionally slow (≈30–100 ms each). On every request the server SHA-256-hashes the bearer and looks it up in a small in-memory cache; only never-before-seen bearers pay the argon2 cost. Failed verifications are not cached, so the cache stays bounded by the number of legitimate tokens and an attacker hammering with random bearers cannot grow it.

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
- **General PATCH on frontmatter.** Editing arbitrary fields without going through `mc` opens too many invariants (status/folder sync, ID stability, link integrity). The CLI doesn't do it; the API doesn't do it.
- **WebSocket / SSE.** Polling `/v1/status` is enough at today's scale. Live updates can be added later with a `tokio::sync::broadcast` channel.
- **Tenant scoping.** Lives in a gateway. See §7.
- **Git commits.** Writes are pure FS, just like the CLI. Wire commits into your operator/cron — mc doesn't touch git after `init`.
- **TLS, rate limiting, CORS for public origins.** Reverse-proxy concerns.
