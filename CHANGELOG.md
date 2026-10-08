# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.3.0] - 2026-10-08

### Added
- Milestones (work packages) that group tasks, with a description, start date, deadline, owner, status (`planned`, `active`, `completed`, `cancelled`) and linked projects: `mc new milestone`, `mc list milestones`, `--milestone` on `mc new task`, `mc task set`, `mc list tasks` and `mc task board` (ID, loose ID or unique title; an empty filter lists tasks without a milestone). REST has `POST /v1/milestones` and `milestone` on task create, `PATCH` and `GET /v1/tasks`; MCP has `create_milestone` and `milestone` on `create_task`, `update_task` and `list_tasks`. Repos that have tasks get milestones without a config change. The dashboard has a Gantt chart at `/milestones` (project filter, expandable task rows), a compact chart of open milestones on the overview, and milestone filters, columns and edit controls on the task pages.
- `mc task set <ID>` changes a task's title, status, priority, owner, sprint, due date, projects, customers, tags and dependencies (empty values clear). The same shared update is available as the MCP tool `update_task`, as `PATCH /v1/tasks/{id}` in the REST API, and behind the dashboard's edit form.
- "What next?" for agents: MCP tool `next_tasks` and `GET /v1/tasks/next`, the same queue as `mc task next`, with counts of actionable and blocked tasks.
- `mc --json new …` prints `{id, kind, title|name, path}`.
- `mc list tasks --open`, `--overdue` and `--sort id|priority|due|updated`. Meetings are listed by date and time.
- `mc status` has a COMING UP section with the next meetings.
- `mc validate` reports duplicate IDs, links to entities that don't exist, references written in a non-canonical form, and stale `[[ID|alias]]` names. Each issue has a severity, and only errors fail the run.
- `mc completions <shell>` prints a completion script for bash, zsh, fish, elvish or powershell.
- `mc comment <ID>` with no text opens `$VISUAL` / `$EDITOR`.
- `mc export customer` includes the meetings, tasks and notes elsewhere that link to the customer. `--folder-only` gives the old behaviour.
- `mc index` also writes `data/meetings.json`.
- Dashboard: the edit form changes title, project and customer too, and a new title also rewrites the note's `# Title` heading. Project pages list tasks that mention the project. Contact pages show the customer. Activity entries link to their context and files. The meetings list has a Today divider and a "Needs update" badge. Dependencies are shown one per row with their status. Loose IDs (`task-72`) work in the palette, and `/entity/task-72` redirects to `TASK-072`.

### Changed
- References given to `mc new`, MCP and REST creates (`project`, `customers`, `depends_on`, `supersedes`, `sprint`) accept loose IDs, are stored canonically (`proj-1` → `[[PROJ-001]]`), and must name an existing entity (REST: `404`). A sprint can be given by ID or title and is stored as its ID. Sprint filters match both.
- Statuses are matched case-insensitively and with aliases (`wip`, `doing`, `completed`) everywhere, including `mc new` and the MCP and REST filters. MCP and REST accept loose IDs like the CLI.
- Only `done` and `cancelled` tasks go to `done/`; custom statuses stay in `todo/` (create, move and validate agree).
- `mc new` checks flags before any prompt. A meeting date or due date must be zero-padded `YYYY-MM-DD` on every surface.
- `--color` also applies to `--help` and argument errors, and `TERM=dumb` turns colours off. With `--json`, argument errors are JSON too. Command options are listed before the global ones in `--help`.
- Usage mistakes in `mc api serve` exit with code 2.
- `--json` paths are relative to the repo root on every command, in MCP results and in REST responses (including error details).
- MCP print tools write inside the repo (relative to its root, not the server's working directory). Printed PDFs show `[[ID|alias]]` links as their names and leave out `%% … %%` comments and the mc-links footer.
- `mc index` skips entity kinds the repo doesn't enable.
- `mc init --force` keeps customised templates and `.gitignore` (with a backup). `--project` conflicts with `--embedded`, the name defaults to the folder name, and there are no prompts without a terminal.
- `GET /v1/validate` works with read-only tokens and on `--read-only` servers. Every REST error, including rejected bodies and unknown routes, is `application/problem+json`. Unknown routes are `404` even without a token.
- The OpenAPI spec's version follows the crate version.
- Dashboard: CSS, JS and fonts are served from `/assets/…` with a content hash and immutable caching instead of being inlined in every page. Pages load the entity catalog once per request. The task list shows open work first. Write responses carry the repo version from before the write, so an outside change that landed just before is still noticed.
- Narrow terminal tables shrink their flexible columns down to 4 characters. `$EDITOR` and `$PAGER` values with arguments or spaces work.
- Removed unused dependencies (`utoipa-axum`, `http-body-util`, `mime`).

### Fixed
- `mc task move` no longer deletes a task whose file sat in the wrong folder for its status, and refuses to overwrite a second copy of the task.
- Rewriting frontmatter no longer corrupts YAML that contains quoted `[[…]]` text, keeps CRLF line endings, and no longer merges lines or strips body text when it updates the mc-links footer.
- A status move or other frontmatter rewrite keeps an existing mc-links footer as it is when it already links the same entities (e.g. `[[PROJ-001|Innovation Project]]` for `"[[PROJ-001]]"`, or in another order) instead of rewriting it to the bare links.
- Two processes (CLI, MCP, REST, dashboard) creating entities at the same time no longer get the same ID, and concurrent moves, comments, checklist ticks and edits no longer overwrite each other: every write takes a repo-wide lock. It lives in the git dir of the enclosing git repo (`.git/mc-write.lock`; worktrees and submodules use their own git dir), so an embedded `.mc` in a monorepo subfolder or a nested mc config (e.g. a customer folder with its own `config/config.yml`) shares the repo's one lock. Without git it is `.mc-write.lock` next to the config: `mc init` ignores it, but an existing repo that isn't in git and has its own `.gitignore` should add `.mc-write.lock` to it.
- `atomic_write` follows symlinks and keeps file permissions.
- `mc list contacts --customer` finds contacts by the customer folder they live in.
- `mc show` names a file with broken frontmatter instead of reporting the ID as unknown, and shows mapping fields and `[[ID|alias]]` lists readably.
- `--project` and `--customer` filters accept loose IDs, and unknown IDs fail with a hint instead of an empty list. `--priority` outside 1-4 is a usage error.
- Slugs are the same for NFC and NFD input (`ü` typed or pasted from macOS).
- `move_task` (MCP and REST) refuses IDs that aren't tasks instead of moving the file. Creates refuse kinds the repo doesn't enable.
- Checklist items after an inline code span containing `%%` are found again.
- IDs with four or more digits validate and sort numerically.
- `.MD` files (any case) are read like `.md`.
- YAML errors name the line and column in the file, in `mc validate` and in the "invalid frontmatter" errors of `mc show` and other lookups alike.
- Dashboard: comment authors with markup are escaped instead of refused; undo after a live refresh no longer duplicates a card; live refresh keeps focus and horizontal scroll; "create another" no longer shows a spurious change notice; comments are limited to 128 KiB with a clear error; `Ctrl+K` in text fields is left to macOS; filter selects no longer show a stale value after navigating back; the sidebar search no longer traps Tab; the nav toggle is out of the tab order; stale `[[ID|alias]]` links show the current name; the focus ring is visible on filters; sprint pages find tasks that name the sprint by title; identical titles wrap to two lines on phones; table, timeline and grid layout fixes on phones.

### Security
- The dashboard requires a loopback `Host` on reads as well as writes when there is no `--base-path` (DNS rebinding), and sends `X-Frame-Options`, `frame-ancestors 'none'`, `Referrer-Policy` and `nosniff` (clickjacking). Files served from the repo get a sandboxing CSP, so an SVG can't run scripts. Hidden paths are checked after resolving symlinks.
- The REST API runs argon2 verification off the async runtime with a 2-permit limit and caches failed bearers, so random tokens can't stall the server. The RapiDoc page loads its script with Subresource Integrity.
- MCP `print_*` tools can't write outside the repo.
- Escape sequences in titles, owners and other file values never reach the terminal (`mc list`, `status`, `task`, `validate`, `check`, …).

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
