---
id: PROP-012
title: Axum Web Server for Dashboard
status: accepted
type: architecture
author: Stefan Heussner
supersedes: ''
superseded_by: ''
tags:
- web
- server
- architecture
created: 2026-02-09
updated: 2026-02-09
---

# Axum Web Server for Dashboard

## Context

The `mc serve` command needs an HTTP server to host the web dashboard. The server must integrate with the existing synchronous Rust codebase, share configuration state across request handlers, and support routing for entity listing pages, detail pages, and the task board. It should be lightweight and not pull in a heavy framework.

## Options Considered

### Option 1: Actix-web

Mature, high-performance web framework. Feature-rich but heavier than needed for a read-only dashboard. Actor-based concurrency model adds complexity.

### Option 2: Warp

Composable filter-based API. Elegant but the filter composition style can be difficult to read for route-heavy applications.

### Option 3: Axum with Tokio runtime

Built on top of Tokio and Tower, Axum uses standard Rust types (functions as handlers, extractors for parameters). Lightweight, composes well with the Tower ecosystem, and has first-class shared state support via `State` extractor.

## Decision

Use Axum with a Tokio runtime (Option 3). The `commands/serve.rs` module creates a `tokio::runtime::Runtime` on demand (the rest of mc is synchronous) and builds an Axum `Router` with:

- **Shared state** — `Arc<AppState>` containing the `ResolvedConfig`, injected via Axum's `State` extractor
- **Route handlers** — one handler per entity listing page (customers, projects, meetings, research, tasks, sprints, proposals), plus a detail page handler and task board
- **HTML responses** — handlers call `data::collect_entities()` and `html::*` functions, returning `Html<String>`
- **Error handling** — a 404 fallback handler and port-in-use detection with a helpful error message

## Consequences

- The Tokio runtime is created only when `mc serve` runs — all other commands remain synchronous
- Axum's extractor pattern keeps handler signatures clean and testable
- `Arc<ResolvedConfig>` is shared immutably across all handlers — config is loaded once at startup
- The dashboard is read-only and re-reads entity files on every request, ensuring fresh data without caching complexity
- Adding a new entity listing page requires only a new route and handler function following the established pattern
- The Tokio and Axum dependencies increase binary size but are justified by the dashboard feature
