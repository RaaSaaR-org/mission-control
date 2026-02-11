---
id: PROP-010
title: Inlined Init Templates
status: accepted
type: architecture
author: Stefan Heussner
supersedes: ''
superseded_by: ''
tags:
- init
- templates
- architecture
created: 2026-02-09
updated: 2026-02-09
---

# Inlined Init Templates

## Context

The `mc init` command creates a complete MissionControl repository with config files, directory structure, and template Markdown files for each entity type. These templates serve as the starting point for new entities and must be available when `mc init` runs — before any repo structure exists on disk.

## Options Considered

### Option 1: Templates as external files loaded with include_str!()

Store templates as separate `.md` files in `src/templates/` and embed them at compile time with `include_str!()`. Clean separation of concerns but adds many small files to the source tree and makes template content less visible when reading the init code.

### Option 2: Templates downloaded at runtime

Fetch templates from a remote source during `mc init`. Allows updates without recompilation but introduces a network dependency and failure mode for a fundamental operation.

### Option 3: Templates as const strings in init.rs

Define each template as a `const &str` directly in `commands/init.rs`. All template content is visible in the same file that uses it. No external files, no runtime dependencies, no build complexity.

## Decision

Use inlined const strings (Option 3). Each entity template (`TEMPLATE_CUSTOMER`, `TEMPLATE_PROJECT`, `TEMPLATE_MEETING`, `TEMPLATE_RESEARCH`, `TEMPLATE_TASK`, `TEMPLATE_SPRINT`, `TEMPLATE_PROPOSAL`) is defined as a `const &str` in `commands/init.rs`. The `mc init` function writes these templates to the appropriate directory paths.

## Consequences

- `mc init` works with zero external dependencies — everything is in the binary
- Templates are immediately visible when reading the init command code — no indirection
- Template changes require recompilation, but this is consistent with the single-binary philosophy
- The init module is longer than it would be with external files, but the tradeoff favors discoverability
- Templates serve as implicit documentation of the expected frontmatter schema for each entity type
