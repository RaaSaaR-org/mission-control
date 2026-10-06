# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Development Commands

```bash
cargo build --release          # Build release binary (target/release/mc)
cargo test                     # Run all tests
cargo fmt --check              # Check formatting
cargo clippy -- -D warnings    # Lint (CI treats warnings as errors)
cargo clippy --all-targets -- -D warnings  # Also lint tests (keep this clean too)
cargo test <test_name>         # Run a single test by name
```

CI runs fmt, clippy, test, and release build on every push to `main` and every PR. MSRV is 1.88 (pinned in `Cargo.toml` and validated in CI).

## Architecture

**MissionControl (`mc`)** is a Rust CLI for git-based knowledge management. Entities (customers, contacts, projects, meetings, research, tasks, sprints, proposals) are stored as Markdown files with YAML frontmatter in a structured directory tree. Two operating modes are supported:

- **Standalone**: Entire repo is managed by mc. Config at `config/config.yml`. All entity types available.
- **Embedded**: `.mc/` folder inside an existing project. Config at `.mc/config.yml`. Only tasks, meetings, research, sprints, and proposals (no customers/contacts/projects).

### Module Responsibilities

- **`lib.rs`** — Re-exports all modules so integration tests in `tests/` (`cli_ux.rs`, `web*.rs`, `api_integration.rs`) can drive mc in-process.
- **`main.rs`** — Entry point. Parses CLI args via clap, calls `ui::init`, loads config, dispatches to command handlers, and prints errors (JSON on stderr with `--json`) before exiting with `McError::exit_code()`. The `init` command is special-cased before config loading since the config doesn't exist yet.
- **`cli.rs`** — clap derive definitions for all commands and subcommands, plus global flags (`--root`, `-y`, `--json`, `--color`).
- **`cli/ui.rs`** — Terminal presentation: capability detection (color, unicode, width, TTY), glyphs with ASCII fallback, status/priority styling, adaptive `Table`, message helpers (`success`, `info`, `hint`). Piped/non-TTY stdout is plain ASCII with no colors, glyphs, rules, hints or truncation. Respects `NO_COLOR`, `CLICOLOR_FORCE`, `MC_ASCII=1`, `TERM=dumb`, `MC_WIDTH`, `MC_HYPERLINKS` (OSC 8 via `hyperlinks()`/`hyperlink()`; `width_of` ignores OSC 8). Use these helpers instead of hardcoding glyphs.
- **`cli/markdown.rs`** — Terminal Markdown renderer for `mc show` (pulldown-cmark events → wrapped, styled lines): headings, lists/checklists, quotes, boxed code, width-aware tables with box-drawing/ASCII borders, OSC 8 hyperlinks for entity refs and relative files inside the repo (`file://`; `file:` links and paths outside the root are not linked). Drops control characters from the source and again from every parsed event (`&#27;` decodes to ESC); `sanitize` is the helper for any file text printed to a terminal (`mc check` uses it too). `cli/pager.rs` pipes long output through `$MC_PAGER`/`$PAGER`/`less` and opens files for `mc show --open`.
- **`cli/suggest.rs`** — Forgiving input: loose ID parsing (`task-7` → `TASK-007`), status normalization, "did you mean" suggestions, `find_entity` with suggestions.
- **`config.rs`** — `RepoMode` enum (Standalone/Embedded), `RawConfig` (parsed from YAML), and `ResolvedConfig` (with absolute paths, mode, and defaults). `find_repo_root()` walks up directories looking for `.mc/config.yml` or `config/config.yml`.
- **`entity.rs`** — `EntityKind` enum (Customer, Contact, Project, Meeting, Research, Task, Sprint, Proposal) with polymorphic methods for labels, prefixes, directory paths, and status values. `EntityId` handles formatted IDs like `CUST-001`. Contacts use discovery-based collection across `customers/*/contacts/` directories (similar to task discovery). `task_status_folder()` decides `todo/` vs `done/` for a task status (used by both `mc new task` and `mc task move`).
- **`data.rs`** — `EntityRecord` struct and collection functions. `collect_entities()` walks a directory tree and parses each markdown file. `collect_tasks_filtered()` supports multi-dimensional task queries (status, priority, sprint, owner, project, customer). `collect_contacts_filtered()` supports contact queries (status, tag, customer).
- **`frontmatter.rs`** — Splits `---\nYAML\n---\nbody` format, parses/serializes frontmatter, provides field accessors. `split_frontmatter` → `serialize_document` is byte-stable for an unchanged document; use `parse_file` / `update_file` for read-modify-write instead of hand-rolling it.
- **`checklist.rs`** — Markdown task-list items (`- [ ]`) located by pulldown-cmark source offsets (skips code blocks and `%% %%` comments). `set_checked` flips the one state byte, with optional expected state/text that turn a stale edit into `McError::Conflict`. Used by `mc check`, MCP, REST and the dashboard.
- **`comments.rs`** — Comments on tasks and meetings in a trailing `## Comments` section (`### YYYY-MM-DD HH:MM · Author` per comment). `split` separates them from the notes for display; `add` appends (author defaults to `git config user.name`, else `$USER`), staying above the `%% mc-links %%` footer; comment text loses control characters and gets unclosed fences/HTML blocks closed, and `add` refuses to write a comment that wouldn't read back as its own.
- **`util.rs`** — `slugify` (transliterates umlauts/diacritics), `slug_variants` (also the pre-transliteration form, for matching old folders), `atomic_write`, date and list helpers.
- **`template.rs`** — Loads `templates/<kind>.md` and fills fields/placeholders for `mc new`.
- **`html.rs`** — Facade over `src/html/**` for the web dashboard: `catalog` (entity catalog loaded once per request, reference resolution), `format`, `components` (badges, cards, tables), `layout` (app shell), `edit` (quick-create, task edit form, move menu), `notes` (detail body with tickable checklists, comments), `brand`, `markdown`, `search`, and `pages/` (one module per page; `calendar.rs` is the meeting month grid at `/meetings/calendar?month=YYYY-MM`, weeks start Monday, with an agenda fallback on phones; `preview.rs` renders the hover-preview card). CSS/JS/fonts are embedded from `src/assets/` (`app.css`, `app.js`, `fonts/`).
- **`mcp.rs`** — Model Context Protocol server exposing 22 tools and the resources `mc://config` plus `mc://entities/<kind>` per enabled kind. Uses `rmcp` crate with schemars for parameter schemas. All descriptions are self-documenting (valid values, defaults, return types).
- **`api/`** — REST/JSON API for `mc api serve` (axum + utoipa): bearer-token auth (`auth.rs`), RFC 7807 problem-json errors (`error.rs`), handlers mirroring the MCP tools. A per-process write mutex plus a repo lock file serialize writes.
- **`commands/`** — One file per command (`new.rs`, `list.rs`, `show.rs`, `check.rs`, `comment.rs`, `validate.rs`, `init.rs`, `serve.rs`, `print.rs`, `task.rs`, `index.rs`, `export.rs`, `status.rs`, `mcp.rs`, `api.rs`). `serve.rs` builds the dashboard router; `serve/api.rs` holds its JSON endpoints (`/api/palette`, `/api/version`, task get/create/move/PATCH, `/api/entities/{id}/checks` and `/comments`) plus `/api/preview/{id}`, the hover-preview card as an HTML fragment (404 card for unknown IDs); `serve/guard.rs` protects writes (`X-MC-Request` header, same-origin, loopback host). Editing is on locally, off with `--read-only` or behind `--base-path` unless `--allow-edits`.

### Data Flow

CLI args (clap) → config loading (`find_repo_root` → `load_config`) → entity collection (walk directories → parse frontmatter) → filtering/processing → output (terminal, HTML, JSON, or PDF). Collect each kind once per command and filter in memory (e.g. `TaskFilter::matches`, `task::actionable_in`) rather than re-reading files.

### Key Design Decisions

- **Dual-mode**: Standalone repos use `config/config.yml`; embedded repos use `.mc/config.yml`. The `RepoMode` enum threads through config loading, entity availability, and command dispatch.
- **Config-driven**: All directory paths, ID prefixes, and valid status values come from the config file. Adding a new status or changing a prefix requires only a config change.
- **Polymorphic via enum**: `EntityKind` dispatches behavior (directory, prefix, statuses) rather than using traits or inheritance. Most command handlers work generically over any entity kind.
- **Tasks have special scoping**: Tasks can be global, project-scoped, or customer-scoped, with discovery logic in `entity.rs` that searches multiple directory locations. Within each `tasks/` dir, active statuses (backlog, todo, in-progress, review) live in `todo/`, all others in `done/`.
- **Contacts are per-customer**: Contacts live in `customers/CUST-NNN-slug/contacts/CONT-NNN-name.md`. IDs are globally sequential across all customers. Discovery walks all customer contact directories.
- **Embedded templates**: The `init` command uses template strings inlined as constants in `commands/init.rs` (not `include_str!`).
- **Shared write paths**: `mc new`, the MCP tools, the REST API and the dashboard all create and move entities through the same functions in `commands/new.rs` and `commands/task.rs` (checklists: `checklist::set_checked`, comments: `comments::add`), so placement and validation stay identical.
- **MCP bridge**: `mcp.rs` mirrors the full CLI surface for AI assistant integration. Each CLI command has a corresponding MCP tool with schemars-annotated parameter structs. Tool and parameter descriptions are AI-friendly — they include valid values, defaults, return types, and examples so agents can use them correctly without external documentation. Resources (e.g. `mc://config`) expose read-only data with descriptions and MIME types.

### Error Handling

`error.rs` defines `McError` (via `thiserror`) and `McResult<T>`. Errors carry optional hints displayed to the user. All commands return `McResult<()>`. Use typed variants for user-facing errors: `McError::usage(msg, hint)` for invalid input, `McError::not_found(msg, hint)` for missing things, `McError::conflict(msg, hint)` when the file changed under an edit, `McError::not_available(kind, cfg)` for kinds the repo doesn't enable (worded for embedded vs. standalone `paths:`). Mapping:

- **Exit codes**: `0` success; `2` for `Usage`, `InvalidId`, `NotAvailableInMode` (like clap's usage errors); `1` otherwise.
- **REST API** (`api/error.rs`): `Usage` → 400, `NotFound`/`EntityNotFound` → 404, `NotAvailableInMode` → 403, `Conflict` → 409, unknown → 500.
- **MCP** (`tool_err` in `mcp.rs`): `Usage`/`InvalidId`/`EntityNotFound`/`NotAvailableInMode`/`Conflict` → `invalid_params`, `NotFound` → `resource_not_found`, else `internal_error`.

`McError::Other` with message sniffing remains only as a fallback; prefer the typed variants.

## Release Process

1. Bump version in `Cargo.toml`
2. Commit and push
3. Tag with `v<version>` and push tag — CI cross-compiles for Linux/macOS (amd64+arm64), creates a GitHub Release, and publishes to crates.io
