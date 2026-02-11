---
id: PROP-007
title: Multi-Scope Task Discovery
status: accepted
type: architecture
author: Stefan Heussner
supersedes: ''
superseded_by: ''
tags:
- tasks
- scoping
- architecture
created: 2026-02-09
updated: 2026-02-09
---

# Multi-Scope Task Discovery

## Context

Tasks in MissionControl can exist at three different scopes: global (the top-level `tasks/` directory), project-scoped (`projects/*/tasks/`), and customer-scoped (`customers/*/tasks/`). A single task query—listing, filtering, or ID allocation—must search across all scopes transparently. Within each scope, tasks are further divided into `todo/` and `done/` subdirectories based on active vs. completed status.

This multi-scope design is distinct from the general polymorphic dispatch described in PROP-006. While other entity kinds each have a single base directory, tasks require a discovery mechanism that walks multiple directory trees and deduplicates results.

## Options Considered

### Option 1: Single global task directory

All tasks live in one `tasks/` directory regardless of their project or customer association. Associations are tracked only in frontmatter fields. Simple directory structure, but loses the organizational benefit of co-locating tasks with their parent entity.

### Option 2: Tasks only in parent entity directories

Tasks exist only under their parent project or customer directory. Global tasks are not supported. This forces every task to belong to a parent, which doesn't fit standalone work items.

### Option 3: Multi-scope discovery with deduplication

Tasks can live in `tasks/` (global), `projects/*/tasks/`, or `customers/*/tasks/`. A `collect_all_task_dirs()` function discovers all task directories dynamically. Collection functions scan all discovered directories and deduplicate by ID. ID allocation (`next_id`) scans all scopes to prevent collisions.

## Decision

Use multi-scope discovery (Option 3). The `collect_all_task_dirs()` function in `entity.rs` walks the global tasks directory, then iterates over all project and customer subdirectories looking for `tasks/` folders. This list of `TaskLocation` values feeds into:

- **`collect_tasks()`** in `data.rs` — gathers all tasks across scopes, deduplicating by ID via a `HashSet`
- **`collect_tasks_filtered()`** in `data.rs` — applies multi-dimensional filters (status, priority, sprint, owner, project, customer, tag) on top of the full collection
- **`next_id()`** in `entity.rs` — scans both `todo/` and `done/` in every task location to find the maximum ID, ensuring globally unique IDs

## Consequences

- Tasks can be organized close to their parent entity on disk while remaining globally queryable
- ID uniqueness is guaranteed across all scopes — no two tasks share an ID regardless of location
- Every task query requires scanning multiple directory trees, which is acceptable for file-based repos but would not scale to thousands of tasks
- The `todo/` and `done/` subdirectory split means moving a task to "done" status involves a file move, handled by `move_task_programmatic()` in `task.rs`
- Adding a new scope (e.g., team-scoped tasks) requires updating `collect_all_task_dirs()` only
