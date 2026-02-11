---
id: PROP-001
title: Use Markdown with YAML Frontmatter for Entity Storage
status: accepted
type: architecture
author: Stefan Heussner
supersedes: ''
superseded_by: ''
tags:
- core
- storage
- architecture
created: 2026-02-09
updated: 2026-02-09
---

# Use Markdown with YAML Frontmatter for Entity Storage

## Context

MissionControl needs a file format for storing structured entities (customers, projects, meetings, research, tasks, sprints, proposals). The format must support both structured metadata (for querying and filtering) and rich free-form content (notes, descriptions, decisions). Files must be human-readable, version-controllable with git, and editable in any text editor.

## Options Considered

### Option 1: JSON files
Fully structured, easy to parse programmatically. Poor readability for long-form content. Merge conflicts are common and hard to resolve. No native support for rich text.

### Option 2: YAML-only files
Good for structured data but awkward for multi-paragraph prose. Multiline strings in YAML are fragile and confusing.

### Option 3: SQLite database
Powerful querying but opaque to git, not human-editable, and requires tooling for every interaction.

### Option 4: Markdown with YAML frontmatter
Combines structured metadata in a YAML header (`---` delimited) with free-form Markdown body. Widely understood format (Jekyll, Hugo, Obsidian). Human-readable, git-friendly, editable in any text editor or IDE. Structured fields are easily parseable; body supports full Markdown.

## Decision

Use Markdown files with YAML frontmatter (Option 4). Each entity is a single `.md` file with a `---`-delimited YAML header containing typed fields (id, title, status, dates, etc.) followed by a Markdown body for free-form content. The `frontmatter.rs` module handles parsing and serialization.

## Consequences

- Entities are readable and editable without any tooling
- Git diffs are clean and meaningful
- YAML frontmatter provides structured querying while Markdown body allows rich documentation
- Parsing requires splitting on `---` delimiters which is straightforward but must handle edge cases
- No relational querying — cross-entity references use ID strings (e.g. `project: PROJ-001`)
- Schema validation must be done at the application level rather than by the storage layer
