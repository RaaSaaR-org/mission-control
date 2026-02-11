---
id: PROP-009
title: Error Handling with User-Facing Hints
status: accepted
type: architecture
author: Stefan Heussner
supersedes: ''
superseded_by: ''
tags:
- errors
- ux
- architecture
created: 2026-02-09
updated: 2026-02-09
---

# Error Handling with User-Facing Hints

## Context

CLI tools often present cryptic error messages that leave users unsure what to do next. MissionControl needs an error handling strategy that provides both a clear description of what went wrong and actionable guidance on how to fix it. Errors should be typed for programmatic handling while remaining human-friendly in terminal output.

## Options Considered

### Option 1: String-based errors with anyhow

Use `anyhow::Error` for all errors. Simple to implement, good backtraces, but no structured error types and no natural place to attach hints.

### Option 2: Typed errors without hints

Define an `McError` enum via `thiserror` with descriptive `#[error()]` messages. Provides typed matching but the error message alone may not tell users how to resolve the issue.

### Option 3: Typed errors with a hint method

Define `McError` via `thiserror` and add a `hint()` method that returns an `Option<String>` with actionable suggestions. The main entry point renders errors in red and hints in yellow, giving users both the problem and a path forward.

## Decision

Use typed errors with hints (Option 3). The `McError` enum in `error.rs` defines variants for each failure category (config not found, entity not found, invalid ID, validation failures, mode restrictions, etc.) using `thiserror` derive macros. The `hint()` method pattern-matches on error variants to return contextual suggestions:

- `InvalidId` → shows valid ID format examples
- `EntityNotFound` → suggests the `mc list` command
- `RepoRootNotFound` → suggests `mc init` or `--root`
- `AlreadyInitialized` → suggests `--force`
- `NotAvailableInMode` → explains embedded mode limitations

In `main.rs`, error rendering displays the error message in red followed by the hint (if any) in yellow, clearly separating "what happened" from "what to do about it."

## Consequences

- Users get immediate, actionable guidance when something goes wrong
- Error variants are typed, enabling programmatic matching in tests and MCP responses
- The `McResult<T>` type alias keeps function signatures clean
- Adding a new error variant requires considering whether it needs a hint — a good forcing function for UX thinking
- Hints are defined centrally in one `match` block, making them easy to audit and update
- The two-color rendering (red error + yellow hint) creates a consistent visual pattern in terminal output
