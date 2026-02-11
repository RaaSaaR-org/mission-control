---
id: PROP-008
title: Self-Contained Web Dashboard
status: accepted
type: architecture
author: Stefan Heussner
supersedes: ''
superseded_by: ''
tags:
- web
- dashboard
- architecture
created: 2026-02-09
updated: 2026-02-09
---

# Self-Contained Web Dashboard

## Context

MissionControl needs a web interface for browsing entities, viewing status dashboards, and reading rendered Markdown content. The dashboard must work without any external dependencies—no CDN links, no npm build step, no separate frontend project. It should be a single `mc serve` command that starts a fully functional web UI.

## Options Considered

### Option 1: Separate frontend application

Build a React/Vue/Svelte SPA served alongside an API. Rich interactivity but adds a build toolchain, node_modules, and deployment complexity. Contradicts the single-binary philosophy.

### Option 2: Server-rendered HTML with external CSS/JS

Generate HTML server-side but link to external stylesheets and scripts via CDN. Simpler than a SPA but requires internet access and introduces external dependencies.

### Option 3: Fully embedded HTML generation

Generate complete HTML pages in Rust with all CSS embedded at compile time via `include_str!()`. No external dependencies at all. The binary contains everything needed to serve the dashboard.

## Decision

Use fully embedded HTML generation (Option 3). CSS is compiled into the binary using two `include_str!()` calls in `html.rs`:

- **SimpleCSS** (`simple.min.css`) — a classless CSS framework providing sensible defaults for all HTML elements
- **Custom design system** (`custom.css`) — status badges, tag styles, navigation, dark mode support, and layout overrides

HTML is generated in-process by the `html.rs` module with helper functions for layouts, status badges, tag badges, entity links, and Markdown rendering. The `render_markdown()` function uses `pulldown_cmark` with an auto-linking pass that converts entity ID patterns (e.g., `CUST-001`, `PROJ-002`) into clickable links, preserving existing `<a>` tags.

## Consequences

- `mc serve` works offline with zero external dependencies — the binary is fully self-contained
- Two-layer CSS (classless base + custom overrides) provides a clean design with minimal CSS code
- Dark mode is supported via CSS `prefers-color-scheme` media queries
- Entity IDs in Markdown bodies become navigable links automatically
- CSS changes require recompilation since assets are embedded at build time
- No client-side JavaScript — the dashboard is purely server-rendered HTML, which limits interactivity but maximizes simplicity
