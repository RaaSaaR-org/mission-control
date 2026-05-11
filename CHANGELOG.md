# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

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
