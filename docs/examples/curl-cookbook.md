# `mc api serve` — curl cookbook

All examples assume:

```bash
T="correct horse battery staple"
H="Authorization: Bearer $T"
BASE="http://127.0.0.1:5100"
```

Every error response is `application/problem+json` (RFC 7807). See `docs/api.md` §4 for the full `type` URI list.

## Health & spec

```bash
curl -s "$BASE/healthz"                # → "ok"
curl -s "$BASE/readyz"                 # → "ready"
curl -s "$BASE/v1/openapi.json" | jq . # OpenAPI 3.1 spec
```

## Repo metadata

```bash
curl -sH "$H" "$BASE/v1/config"  | jq .
curl -sH "$H" "$BASE/v1/status"  | jq .
```

## Lists & gets

```bash
# Generic list with filter
curl -sH "$H" "$BASE/v1/entities/customer?status=active" | jq .

# Single entity (parsed)
curl -sH "$H" "$BASE/v1/entities/customer/CUST-001" | jq .

# Single entity (raw markdown)
curl -sH "$H" "$BASE/v1/entities/customer/CUST-001/raw"

# Tasks with the full filter set
curl -sGH "$H" "$BASE/v1/tasks" \
  --data-urlencode "status=in-progress" \
  --data-urlencode "project=PROJ-001" \
  --data-urlencode "priority=2" | jq .
```

## Creates

```bash
# Customer
curl -sH "$H" -H "Content-Type: application/json" \
  -X POST "$BASE/v1/customers" \
  -d '{"name":"Acme Inc","status":"active","tags":"enterprise,priority"}' | jq .

# Project (linked to a customer)
curl -sH "$H" -H "Content-Type: application/json" \
  -X POST "$BASE/v1/projects" \
  -d '{"name":"Data Pipeline","owner":"alice","customers":"CUST-001","tags":"ml"}' | jq .

# Task scoped to a customer
curl -sH "$H" -H "Content-Type: application/json" \
  -X POST "$BASE/v1/tasks" \
  -d '{"title":"Smoke test","customer":"CUST-001","priority":2}' | jq .

# Contact for a customer
curl -sH "$H" -H "Content-Type: application/json" \
  -X POST "$BASE/v1/contacts" \
  -d '{"name":"Alice Smith","customer":"CUST-001","role":"VP Eng","email":"alice@acme.example"}' | jq .

# Sprint
curl -sH "$H" -H "Content-Type: application/json" \
  -X POST "$BASE/v1/sprints" \
  -d '{"title":"2026-W05","goal":"Auth module","start_date":"2026-01-27","end_date":"2026-02-07"}' | jq .

# Meeting (today, 14:00, 1h)
curl -sH "$H" -H "Content-Type: application/json" \
  -X POST "$BASE/v1/meetings" \
  -d '{"title":"Weekly Sync","time":"14:00","duration":"1h","customers":"CUST-001"}' | jq .

# Research topic
curl -sH "$H" -H "Content-Type: application/json" \
  -X POST "$BASE/v1/research" \
  -d '{"title":"LLM benchmarks","agents":"claude,gemini","tags":"ai,benchmarks"}' | jq .

# Proposal (BIP/ADR)
curl -sH "$H" -H "Content-Type: application/json" \
  -X POST "$BASE/v1/proposals" \
  -d '{"title":"Use PostgreSQL","type":"architecture","author":"alice"}' | jq .
```

## Task transitions

```bash
# Move TASK-001 to in-progress
curl -sH "$H" -H "Content-Type: application/json" \
  -X POST "$BASE/v1/tasks/TASK-001/move" \
  -d '{"status":"in-progress"}' | jq .

# Complete and assign to a sprint at the same time
curl -sH "$H" -H "Content-Type: application/json" \
  -X POST "$BASE/v1/tasks/TASK-001/move" \
  -d '{"status":"done","sprint":"SPR-001"}' | jq .
```

The path field in the response shows whether the file moved between `todo/` and `done/`.

## Maintenance

```bash
# Rebuild data/*.json indexes
curl -sH "$H" -X POST "$BASE/v1/index" | jq .

# Run validation; ok=false on issues
curl -sH "$H" -X POST "$BASE/v1/validate" | jq .
```

## Auth-failure samples

```bash
# Missing bearer
curl -s -i "$BASE/v1/config" | head -15

# Invalid bearer
curl -s -i -H "Authorization: Bearer wrong" "$BASE/v1/config" | head -15

# Read-only token attempting a write (returns 403 type=forbidden)
curl -sH "Authorization: Bearer read-only-token" -H "Content-Type: application/json" \
  -X POST "$BASE/v1/customers" \
  -d '{"name":"X"}' | jq .
```
