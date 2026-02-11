# .mc/ — MissionControl's Own Knowledge Base

This folder is an **embedded MissionControl repository** — mc uses itself to track its own development. It's the canonical example of embedded mode (`.mc/` inside an existing project).

## What's Here

- **tasks/** — Development tasks, bugs, and feature work
- **sprints/** — Sprint planning and tracking
- **research/** — Technical research and investigations
- **proposals/** — Architectural decisions and feature proposals (ADR-style)
- **templates/** — Templates for creating new entities
- **config.yml** — Embedded-mode configuration

## Usage

From the repository root:

```bash
mc list tasks          # List all development tasks
mc list proposals      # List architectural decisions
mc new task            # Create a new task
mc validate            # Check all entities for errors
mc status              # Overview of all entity counts
```

## Why Embedded Mode?

Standalone mc repos manage the entire directory tree. Embedded mode (`.mc/`) lives inside an existing project — in this case, mc's own Rust codebase. Embedded mode excludes customer and project entities since those are organizational concepts that don't apply within a single project.
