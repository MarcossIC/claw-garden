---
name: rust-filesystem
description: "File identity. Use when Rust CLI traversal, paths, temporary files, replacement, or deletion must preserve data and behave correctly across filesystem boundaries."
---

# Rust Filesystem

**File identity.** Distinguish a path spelling from the object the operation actually opens or changes. Honor supplied requirements and settled technical choices; resolve only what the task leaves open.

Use Path and OsStr through filesystem operations; lossy display is not a round-trip representation. Preserve the supported platforms' path, case, permission, and link behavior. Directory iteration order is not a sorting guarantee; make deterministic output explicit only where required.

Define traversal policy for symlinks, hidden entries, ignored paths, cycles, and errors from individual entries. Bound open handles and retained traversal state. Reuse existing traversal facilities when their semantics match rather than rebuilding their policy accidentally.

Choose open options from the intended effect. Check-then-create can race; use atomic exclusive creation when replacing an existing object is forbidden. Lexical normalization or a prior canonicalization does not by itself enforce containment against concurrent path changes. Use handle-relative or platform facilities when the threat model requires them.

For replacement that must preserve the old file on failure, stage through a securely created temporary file on the destination filesystem and use the appropriate commit operation. Inspect replacement, metadata, and platform semantics. Atomic visibility and crash durability are separate promises; flush buffers and perform the required file and directory synchronization for the latter.

Do not truncate an input through an aliased output path before processing it. Keep cleanup ownership explicit, including errors during staging or commit; retain recoverable data when recovery is the contract.

Verify the relevant existing-target, link, alias, permission, or interrupted-write case and inspect resulting files independently. Report the platform and durability boundary actually exercised.
