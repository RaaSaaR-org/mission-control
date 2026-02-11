# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added
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
