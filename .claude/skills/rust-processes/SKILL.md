---
name: rust-processes
description: "Process ownership. Use when a Rust CLI launches commands or pipelines and must manage arguments, environment, captured output, termination, and child cleanup."
---

# Rust Processes

**Process ownership.** Account for each child from spawn through terminal status and resource release. Honor supplied requirements and settled technical choices; resolve only what the task leaves open.

Use Command with explicit argument values rather than assembling shell text. Preserve OsStr arguments and choose executable lookup, working directory, and inherited environment intentionally. Literal arguments still enter the child's own option parser; use its supported end-of-options mechanism where needed. Windows batch files and command interpreters require their actual quoting and trust rules.

Choose inherited, discarded, or captured streams according to the task. output and wait_with_output collect output in memory; use bounded streaming for potentially large results. Drain stdout and stderr without letting one fill while waiting on the other. Close every owned stdin writer when input is complete, including handles taken out of Child.

Distinguish failure to spawn from a nonzero exit or signal termination. Inspect status even when output looks valid, and preserve the command's documented pipeline outcome. An error after spawn still requires handling the running child.

Dropping Child neither stops nor reaps it. Define who requests termination and who waits afterward. A timeout ends waiting, not execution; killing one process may leave descendants. Use platform process groups or job facilities when the requirement covers a process tree.

Keep secrets out of command diagnostics and avoid expanding ambient authority without a requirement. Verify argument preservation, the relevant full-pipe or early-exit case, and actual child termination. Bound test waits and clean up children when an assertion fails.
