---
id: PROP-003
title: MCP Server for AI Agent Integration
status: accepted
type: architecture
author: Stefan Heussner
supersedes: ''
superseded_by: ''
tags:
- mcp
- ai
- integration
created: 2026-02-09
updated: 2026-02-09
---

# MCP Server for AI Agent Integration

## Context

AI coding assistants (Claude, Cursor, etc.) can interact with external tools through the Model Context Protocol (MCP). MissionControl stores structured project knowledge that would be valuable for AI agents to read and create — but without an MCP server, agents would need to parse raw files or shell out to the CLI, losing structured data and discoverability.

## Options Considered

### Option 1: CLI-only integration
AI agents invoke `mc` CLI commands via shell execution. Works but loses structured output (agents must parse terminal formatting), has no tool discoverability, and requires the agent to know the CLI interface upfront.

### Option 2: REST API server
Add an HTTP REST API. Provides structured JSON responses but requires a running server process, adds network dependencies, and is a heavier integration than most AI tools expect.

### Option 3: MCP server over stdio
Implement the Model Context Protocol with JSON-RPC over stdin/stdout. Provides structured tool definitions with JSON Schema parameter descriptions, resource URIs for data access, and runs as a subprocess — no network required. The `rmcp` crate provides a Rust implementation.

## Decision

Implement an MCP server (Option 3) exposed via `mc mcp` that mirrors the full CLI surface. Each CLI command has a corresponding MCP tool with `schemars`-annotated parameter structs for automatic JSON Schema generation. Entity data is also exposed as MCP resources (`mc://entities/{kind}`). The server runs over stdio using the `rmcp` crate.

## Consequences

- AI agents can discover and use all MissionControl capabilities through standard MCP tool definitions
- Structured JSON responses eliminate parsing ambiguity
- No running server or network dependencies — the MCP server is a subprocess
- The MCP surface must be kept in sync with CLI changes (dual maintenance)
- Resource URIs provide a clean read-only data access pattern alongside tool-based mutations
- Parameter schemas are auto-generated from Rust structs via `schemars`, reducing boilerplate
