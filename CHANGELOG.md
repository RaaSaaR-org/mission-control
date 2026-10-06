# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.2.0] - 2026-10-06

### Added
- Redesigned web dashboard ("Flight Ops"): new component layer, light/dark themes with a manual toggle, bundled Archivo font, status lamps, "Next six weeks" flight-plan strip on the overview, phone/print/high-contrast support. `brand.css` overrides still apply.
- Dashboard editing: drag tasks between board columns (plus a keyboard move menu with undo), task edit form, quick-create (`c`), ⌘K command palette, keyboard shortcuts (`?`), live refresh when files change on disk.
- Hover/focus preview cards for entity links (`/api/preview/{id}`).
- Meeting calendar view (`/meetings/calendar`): month grid with ISO weeks, optional sprints & deadlines overlay, agenda list on phones.
- Toggle Markdown checkboxes from detail pages; `mc check`, MCP tool and REST endpoint for the same.
- Comments on tasks and meetings, stored in a `## Comments` section of the entity file; `mc comment`, MCP tool and REST endpoint.
- `mc show` renders Markdown in the terminal (tables, lists, checkboxes, code), with OSC 8 clickable links to referenced entities and files, a links/backlinks footer, and a pager for long output. `--raw` forces raw Markdown.
- `/files/...` route so relative links in notes work in the dashboard.
- `--read-only` and `--allow-edits` flags for `mc serve` (editing is off behind `--base-path` unless allowed).
- CLI: "did you mean" suggestions, forgiving ID parsing (`task-7` → `TASK-007`), global `--root` and `-y`, `NO_COLOR`/`MC_ASCII`/`MC_WIDTH` support.

### Changed
- Usage errors (unknown status, invalid ID, entity type not enabled) exit with code 2 instead of 1.
- Piped/non-TTY output is plain ASCII without colors or glyphs; "no results" messages go to stderr.
- `mc status` and `mc index` hide entity types not enabled in the config.
- REST API maps usage errors to 400, missing entities to 404 and disabled kinds to 403 (was 500); MCP returns `invalid_params` / `resource_not_found`.
- `mc new`, MCP and REST validate dates, priorities (1–4) and sprint date ranges; MCP rejects unknown entity kinds and print templates.
- `slugify` transliterates umlauts and diacritics; research agent folder names are lowercase slugs.
- `mc show --full` is accepted but no longer needed (rendered view shows the whole body).
- Web dashboard code split from a single `html.rs` into `src/html/**`.

### Fixed
- `site.name` from the config is used as the dashboard name.
- `mc task move` no longer adds a blank line to the frontmatter on every move.
- `mc new task --status done` files the task under `done/`.
- Crash (SIGPIPE) when piping output, e.g. `mc list tasks | head`.
- Customer export finds folders with umlauts.
- "Not available in embedded mode" error no longer shown in standalone repos.

### Security
- Raw HTML in notes is sanitised in the dashboard (XSS).
- Dashboard writes require an `X-MC-Request` header and a same-origin, loopback request.
- Control characters are stripped from terminal output and comment text, so notes can't inject terminal escape sequences.

## [0.1.14] - 2026-05-11

### Added
- `mc api serve` — bearer-authenticated HTTP/JSON API mirroring the MCP tool surface (versioned `/v1`, OpenAPI 3.1 spec at `/v1/openapi.json`, RFC 7807 error responses, request-id propagation, structured tracing at INFO).
- `mc api serve --insecure-dev-token` — generate a random read+write token at startup for zero-friction local dev.
- `mc api serve --read-only` — reject every non-GET regardless of token capabilities.
- `/v1/docs` — interactive RapiDoc viewer rendering the OpenAPI spec, no auth required.
- `mc api hash-token` — generate argon2id hashes for the tokens file.
- Cross-process safety: exclusive `flock` on `<repo>/.mc-api.lock`; a second `mc api serve` against the same repo fails fast.
- SHA-256 fast-path cache for bearer verification — argon2 only runs on first-sight bearers, subsequent requests hit a hashmap.
- 64 KiB request-body limit and 30 s request timeout (slowloris/oversized-body DoS guards).
- `docs/api.md` — full API reference including a multi-tenant gateway pattern for downstream consumers.
- `docs/examples/curl-cookbook.md`, `docs/examples/tokens.example.yml`.
- Crate is now a hybrid lib+bin so integration tests under `tests/` can drive the API in-process.
- Embedded mode for `.mc/` inside existing projects
- CLAUDE.md with architecture and build documentation
- CHANGELOG.md

### Changed
- Rewrote README with quickstart and task management focus

## [0.1.1] - 2026-02-01

### Added
- `mc init` command to bootstrap new repositories
- Sprint entity type

### Changed
- Inlined templates in `init.rs` instead of `include_str!`
- Simplified README, reduced duplication with CLAUDE.md

## [0.1.0] - 2026-01-31

### Added
- Initial release
- Entity types: customers, projects, meetings, research, tasks
- Markdown files with YAML frontmatter storage
- CLI commands: new, list, show, validate, serve, print, export, status, index
- Web dashboard with HTML generation
- PDF export via genpdf
- MCP server for AI assistant integration
- Standalone repo mode with `config/config.yml`

[Unreleased]: https://github.com/RaaSaaR-org/mission-control/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/RaaSaaR-org/mission-control/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/RaaSaaR-org/mission-control/releases/tag/v0.1.0
