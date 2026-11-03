---
name: rust-cli-interface
description: "Composition. Use when Rust command arguments, configuration precedence, help, terminal interaction, or human and machine output need implementation or review."
---

# Rust CLI Interface

**Composition.** Treat the command as an interface to both people and other programs. Identify accepted inputs, output streams, exit behavior, and effects. Honor supplied requirements and settled technical choices; resolve only what the task leaves open.

Preserve the established argument parser. For a new command, match parser complexity to its grammar; clap is an established option for structured arguments, validation, and help. Reuse its supported mechanisms instead of duplicating parsing rules. Keep help and version paths inexpensive and free of unrelated initialization or effects.

Model conflicts, required combinations, defaults, and subcommands intentionally. Preserve path and argument values through OsString or PathBuf where UTF-8 is not guaranteed. Account for end-of-options markers and values beginning with a dash. Merge flags, environment, and configuration in a documented precedence order; distinguish absent values from explicit false, zero, or empty overrides.

Keep result data on stdout and diagnostics or progress on stderr. Preserve any established machine-readable schema, record framing, and ordering. Use terminal detection for presentation, with explicit user choices taking precedence; never make scripts parse decoration. Escape untrusted terminal control characters in human diagnostics while preserving the intended data format.

Prompt only when interaction is part of the command's contract and the appropriate terminal is available. Do not consume piped data as a confirmation answer. Keep noninteractive failure actionable.

Exercise the actual argument parser and command output, including the relevant invalid combination, precedence conflict, or redirected stream. Confirm help, diagnostics, and effects agree with the command's documented behavior.
