---
name: rust-design
description: "Cohesion. Use when Rust CLI responsibilities, modules, domain models, or abstractions make a command difficult to extend or reason about."
---

# Rust Design

Design for **cohesion**: each rule has a natural home, and callers need little knowledge of its implementation. Honor supplied requirements and settled technical choices; resolve only what the task leaves open.

Trace arguments and configuration through the operation to output and effects. Keep parsing, useful computation, and process-level decisions distinguishable where their contracts differ. A small command can remain a small program; introduce modules or a library target when they provide useful ownership, reuse, or test access.

Prefer concrete types, functions, and explicit dependencies. Use traits for real capabilities or variation, not a counterpart for every struct. Standard Read and Write boundaries often provide sufficient flexibility. Choose generics or dynamic dispatch for the actual caller relationship; account for monomorphized code size and dispatch costs when relevant, without blanket bans on either.

Let types express meaningful invariants and ownership. Keep lifetime parameters local where possible. Avoid self-referential structures, global mutable state, or Arc and Mutex merely to simplify wiring. A builder or configuration layer needs enough real choices to justify its cost.

Evaluate a boundary by the knowledge it removes from callers. Centralize shared policy while preserving independent reasons to change. Keep allocation and effect ownership explainable across the retained boundaries.

For refactoring, compare output, failures, ordering, mutation, and resource lifetime. Finish when the changed responsibility has a clear owner, each abstraction earns its cost, and focused checks preserve the behavior that matters.
