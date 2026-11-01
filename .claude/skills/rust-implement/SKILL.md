---
name: rust-implement
description: "Execution. Use to turn clear requirements, a specification, or an agreed design into working Rust command-line utility code."
---

# Rust Implement

**Execution.** When the intended behavior is clear, implement it directly. Treat supplied requirements and settled technical decisions as constraints, not invitations to redesign.

Read the affected code and callers, then extend the existing path. Preserve the project's Rust edition, minimum supported Rust version, target platforms, CLI contract, and selected dependencies.

**Reuse.** Check existing code, standard-library operations, and declared crates before writing technical helpers. Use matching APIs directly. Custom mechanics need a concrete semantic or operational gap; wrappers should add domain meaning or adaptation. A parser, runtime, or utility crate should answer a current requirement.

**Clarity.** Write for the next reader: intention-revealing names, cohesive responsibilities, explicit ownership, and visible effects and failure paths. Keep changes local and idiomatic to Rust. Apply SOLID, DRY, and YAGNI as heuristics: centralize shared knowledge, preserve distinct rules, and add only structure justified by current requirements. Prefer the simplest implementation that remains easy to read and change.

Keep data moving without unnecessary copies or whole-input materialization. Account for buffers, queued work, and retained results when input can grow. Start synchronously when that satisfies the task; concurrency and optimization need a relevant workload or resource requirement. Preserve correctness while reducing cost.

Resolve routine details autonomously. If a concrete contradiction prevents implementation, identify it and continue independent work; ask only for information that changes the required outcome.

Verify the requested behavior with focused checks that would fail for a plausible contract violation. Match testing effort to the change and respect required checks. Finish with working code, actual verification, and specific unresolved requirements.
