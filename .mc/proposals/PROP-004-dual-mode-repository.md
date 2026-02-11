---
id: PROP-004
title: Dual-Mode Repository (Standalone vs Embedded)
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

# Dual-Mode Repository (Standalone vs Embedded)

## Context

MissionControl was originally designed as a standalone tool that owns an entire repository. However, many teams want to track tasks, research, and decisions alongside their existing codebase without restructuring their project. The tool needs to support both use cases: dedicated knowledge-base repos and lightweight embedded usage within existing projects.

## Options Considered

### Option 1: Standalone only
Require a dedicated repository for all MissionControl data. Simple but forces users to maintain a separate repo, losing the benefit of co-locating project knowledge with code.

### Option 2: Configurable root directory
Allow pointing mc at any directory via a flag or environment variable. Flexible but requires users to remember the path on every invocation, and doesn't clearly separate mc data from project files.

### Option 3: Dual-mode with automatic detection
Support two modes — standalone (`config/config.yml` at repo root) and embedded (`.mc/config.yml` inside any project). The `find_repo_root()` function walks up the directory tree, checking for `.mc/config.yml` first (embedded), then `config/config.yml` (standalone). A `RepoMode` enum threads through config loading and entity availability.

## Decision

Implement dual-mode with automatic detection (Option 3). The `find_repo_root()` function in `config.rs` walks up from the current directory, preferring embedded mode when both configs exist. The `RepoMode` enum (Standalone/Embedded) determines which entity kinds are available — embedded mode excludes Customer and Project entities since those are organizational concepts that don't apply within a single project. The `init` command supports both `mc init` (standalone) and `mc init --embedded` to set up either mode.

## Consequences

- Users can adopt mc incrementally by adding `.mc/` to an existing project
- Standalone repos support the full entity set including customers and projects
- Embedded repos focus on tasks, sprints, research, and proposals — the entities relevant within a single project
- `find_repo_root()` prefers embedded mode when both configs exist, which is a deliberate choice to support nested workflows
- The `RepoMode` enum must be checked in commands, MCP tools, and the web dashboard to prevent accessing unavailable entity types
- Config paths are relative to the mode-specific root (`.mc/` or repo root), handled by `ResolvedConfig`
