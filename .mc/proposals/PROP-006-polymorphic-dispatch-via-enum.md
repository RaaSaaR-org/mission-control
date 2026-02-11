---
id: PROP-006
title: Polymorphic Dispatch via Enum
status: accepted
type: architecture
author: Stefan Heussner
supersedes: ''
superseded_by: ''
tags:
- core
- architecture
- rust
created: 2026-02-09
updated: 2026-02-09
---

# Polymorphic Dispatch via Enum

## Context

MissionControl has seven entity kinds (Customer, Project, Meeting, Research, Task, Sprint, Proposal), each with different labels, ID prefixes, directory paths, valid statuses, and mode availability. The system needs a way to dispatch behavior based on entity kind while keeping most command handlers generic. Rust offers several patterns for this: trait objects, generics, or enum-based dispatch.

## Options Considered

### Option 1: Trait objects (dyn EntityType)
Define an `EntityType` trait with methods like `label()`, `prefix()`, `statuses()`, and use `Box<dyn EntityType>`. Provides runtime polymorphism but adds heap allocation, makes pattern matching impossible, and complicates ownership. Overly abstract for a fixed, known set of variants.

### Option 2: Generic type parameters
Make command handlers generic over `T: EntityType`. Provides zero-cost abstraction but leads to monomorphization bloat for seven variants and makes dynamic dispatch (e.g., iterating over all kinds) awkward.

### Option 3: Enum with match arms
Define `EntityKind` as a simple enum and implement behavior via `match` expressions in methods like `label()`, `prefix()`, `directory()`, `statuses()`, and `available_in_mode()`. Exhaustive matching ensures every variant is handled when adding a new entity type.

## Decision

Use enum-based dispatch (Option 3). The `EntityKind` enum in `entity.rs` has a variant for each entity type. Behavior is implemented as methods on `EntityKind` using `match` expressions. Most command handlers accept `EntityKind` as a parameter and work generically — `list`, `show`, `validate`, and `new` all dispatch through the same enum methods. Only tasks have special-case handling due to their scoping rules.

## Consequences

- Adding a new entity kind requires adding a variant and updating each `match` block — the compiler enforces exhaustiveness
- No heap allocation or vtable indirection — all dispatch is static
- Pattern matching enables clean handling of special cases (e.g., task scoping) without breaking the generic flow
- All seven variants are known at compile time, which is appropriate since entity kinds are a closed set
- Methods like `available_in_mode()` naturally express mode-specific logic as match arms
- The approach is idiomatic Rust and familiar to contributors
