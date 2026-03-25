# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Development Commands

```bash
cargo build --release          # Build release binary (target/release/mc)
cargo test                     # Run all tests
cargo fmt --check              # Check formatting
cargo clippy -- -D warnings    # Lint (CI treats warnings as errors)
cargo test <test_name>         # Run a single test by name
```

CI runs fmt, clippy, test, and release build on every push to `main` and every PR. MSRV is 1.88 (pinned in `Cargo.toml` and validated in CI).

## Architecture

**MissionControl (`mc`)** is a Rust CLI for git-based knowledge management. Entities (customers, contacts, projects, meetings, research, tasks, sprints, proposals) are stored as Markdown files with YAML frontmatter in a structured directory tree. Two operating modes are supported:

- **Standalone**: Entire repo is managed by mc. Config at `config/config.yml`. All entity types available.
- **Embedded**: `.mc/` folder inside an existing project. Config at `.mc/config.yml`. Only tasks, meetings, research, sprints, and proposals (no customers/contacts/projects).

### Module Responsibilities

- **`main.rs`** — Entry point. Parses CLI args via clap, loads config, dispatches to command handlers. The `init` command is special-cased before config loading since the config doesn't exist yet.
- **`cli.rs`** — clap derive definitions for all commands and subcommands.
- **`config.rs`** — `RepoMode` enum (Standalone/Embedded), `RawConfig` (parsed from YAML), and `ResolvedConfig` (with absolute paths, mode, and defaults). `find_repo_root()` walks up directories looking for `.mc/config.yml` or `config/config.yml`.
- **`entity.rs`** — `EntityKind` enum (Customer, Contact, Project, Meeting, Research, Task, Sprint, Proposal) with polymorphic methods for labels, prefixes, directory paths, and status values. `EntityId` handles formatted IDs like `CUST-001`. Contacts use discovery-based collection across `customers/*/contacts/` directories (similar to task discovery).
- **`data.rs`** — `EntityRecord` struct and collection functions. `collect_entities()` walks a directory tree and parses each markdown file. `collect_tasks_filtered()` supports multi-dimensional task queries (status, priority, sprint, owner, project, customer). `collect_contacts_filtered()` supports contact queries (status, tag, customer).
- **`frontmatter.rs`** — Splits `---\nYAML\n---\nbody` format, parses/serializes frontmatter, provides field accessors.
- **`html.rs`** — Generates the web dashboard HTML. Layout templates, navigation, status badges, entity detail pages. CSS is embedded from `src/assets/`.
- **`mcp.rs`** — Model Context Protocol server exposing 18 tools and 9 resources for AI assistant integration. Uses `rmcp` crate with schemars for parameter schemas. All descriptions are self-documenting (valid values, defaults, return types).
- **`commands/`** — One file per command (`new.rs`, `list.rs`, `show.rs`, `validate.rs`, `init.rs`, `serve.rs`, `print.rs`, `task.rs`, `index.rs`, `export.rs`, `status.rs`, `mcp.rs`).

### Data Flow

CLI args (clap) → config loading (`find_repo_root` → `load_config`) → entity collection (walk directories → parse frontmatter) → filtering/processing → output (terminal, HTML, JSON, or PDF).

### Key Design Decisions

- **Dual-mode**: Standalone repos use `config/config.yml`; embedded repos use `.mc/config.yml`. The `RepoMode` enum threads through config loading, entity availability, and command dispatch.
- **Config-driven**: All directory paths, ID prefixes, and valid status values come from the config file. Adding a new status or changing a prefix requires only a config change.
- **Polymorphic via enum**: `EntityKind` dispatches behavior (directory, prefix, statuses) rather than using traits or inheritance. Most command handlers work generically over any entity kind.
- **Tasks have special scoping**: Tasks can be global, project-scoped, or customer-scoped, with discovery logic in `entity.rs` that searches multiple directory locations.
- **Contacts are per-customer**: Contacts live in `customers/CUST-NNN-slug/contacts/CONT-NNN-name.md`. IDs are globally sequential across all customers. Discovery walks all customer contact directories.
- **Embedded templates**: The `init` command uses template strings inlined as constants in `commands/init.rs` (not `include_str!`).
- **MCP bridge**: `mcp.rs` mirrors the full CLI surface for AI assistant integration. Each CLI command has a corresponding MCP tool with schemars-annotated parameter structs. Tool and parameter descriptions are AI-friendly — they include valid values, defaults, return types, and examples so agents can use them correctly without external documentation. Resources (e.g. `mc://config`) expose read-only data with descriptions and MIME types.

### Error Handling

`error.rs` defines `McError` (via `thiserror`) and `McResult<T>`. Errors carry optional hints displayed to the user. All commands return `McResult<()>`.

## Release Process

1. Bump version in `Cargo.toml`
2. Commit and push
3. Tag with `v<version>` and push tag — CI cross-compiles for Linux/macOS (amd64+arm64), creates a GitHub Release, and publishes to crates.io
