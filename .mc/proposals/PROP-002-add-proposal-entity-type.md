---
id: PROP-002
title: Add Proposal Entity Type
status: accepted
type: feature
author: Stefan Heussner
supersedes: ''
superseded_by: ''
tags:
- entity
- feature
- meta
created: 2026-02-09
updated: 2026-02-09
---

# Add Proposal Entity Type

## Context

MissionControl tracks customers, projects, meetings, research, tasks, and sprints. However, there is no way to record architectural decisions, feature proposals, or process changes in a structured manner. Teams need a BIP/ADR-style decision record that captures the context, options considered, decision made, and consequences — and that integrates with the existing entity system (listing, filtering, validation, MCP, web dashboard).

## Options Considered

### Option 1: Use research entities for proposals
Repurpose the existing research entity type with a tag like `type: proposal`. Simple but loses semantic distinction — proposals have different lifecycle statuses (draft/proposed/accepted/rejected/superseded/withdrawn) and fields (author, type, supersedes) that don't map well to research.

### Option 2: External ADR tool
Use a standalone tool like `adr-tools`. Keeps proposals outside of MissionControl, losing integration with the entity system, MCP server, and web dashboard.

### Option 3: New first-class Proposal entity
Add `Proposal` as a new variant of `EntityKind` with its own directory, ID prefix (`PROP-`), statuses, and frontmatter fields. Integrate across all commands, MCP tools, and the HTML dashboard.

## Decision

Add Proposal as a first-class entity type (Option 3). The proposal entity includes: unique statuses (draft, proposed, accepted, rejected, superseded, withdrawn), a `type` field (architecture, feature, process), `author` field, `supersedes`/`superseded_by` for proposal chaining, and the standard ADR body template (Context, Options Considered, Decision, Consequences). This proposal is itself PROP-002 — using the tool to develop the tool.

## Consequences

- Proposals are fully integrated with existing CLI commands, validation, MCP tools, and the web dashboard
- The `supersedes`/`superseded_by` fields enable tracking proposal evolution over time
- Adding a new entity type requires changes across ~13 source files (entity.rs, config.rs, cli.rs, all commands, mcp.rs, html.rs, init.rs)
- The proposal type field (architecture/feature/process) provides useful categorization for filtering
- Available in both standalone and embedded repository modes
