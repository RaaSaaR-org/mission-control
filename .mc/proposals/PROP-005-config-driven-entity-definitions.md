---
id: PROP-005
title: Config-Driven Entity Definitions
status: accepted
type: architecture
author: Stefan Heussner
supersedes: ''
superseded_by: ''
tags:
- core
- architecture
- config
created: 2026-02-09
updated: 2026-02-09
---

# Config-Driven Entity Definitions

## Context

MissionControl manages multiple entity types (customers, projects, meetings, research, tasks, sprints, proposals), each with its own directory path, ID prefix, and set of valid statuses. These definitions could be hardcoded in the source or externalized to configuration. The choice affects how easily users can customize their setup and how maintainable the system is as entity types evolve.

## Options Considered

### Option 1: Hardcoded constants
Define all paths, prefixes, and statuses as constants in source code. Simple and type-safe but requires recompilation for any change. Users cannot customize their setup.

### Option 2: Convention-based with overrides
Use hardcoded defaults with optional config overrides. Reduces config boilerplate for the common case but creates two sources of truth and makes it harder to reason about what values are active.

### Option 3: Fully config-driven
All directory paths, ID prefixes, and valid status values come from `config.yml`. The config file is the single source of truth. The `RawConfig` struct maps directly to the YAML structure; `ResolvedConfig` resolves relative paths to absolute and applies mode-specific defaults.

## Decision

Use fully config-driven entity definitions (Option 3). The `config.yml` file defines three key sections: `paths` (directory locations for each entity type), `id_prefixes` (the prefix used in entity IDs like `TASK-001`), and `statuses` (valid status values per entity type). The `init` command generates a complete config with sensible defaults. The `validate` command checks entities against their configured statuses.

## Consequences

- Adding a new status value or changing an ID prefix requires only a config edit — no recompilation
- The config file serves as documentation of the repository's structure
- Validation is config-aware: `mc validate` checks entity statuses against the configured valid values
- Config must be loaded before any entity operation, establishing a clear initialization order
- The `init` command must generate a complete, well-documented default config for each mode
- Misspelled config keys fail silently unless explicitly validated — a tradeoff of flexible YAML parsing
