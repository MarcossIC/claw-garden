---
name: rust-debugging
description: "Causality. Use for an uncertain Rust CLI defect, panic, hang, incorrect output, resource leak, or platform-dependent failure."
---

# Rust Debugging

**Causality.** Find the first observable divergence between the intended command and the real execution path. Honor supplied requirements and settled technical choices; resolve only what the task leaves open.

Reproduce the smallest useful signal with the actual arguments, input bytes, environment, working directory, target, features, and build profile. Separate compiler diagnostics, process startup failures, domain errors, and crashes. Preserve raw stdout, stderr, and status when their differences explain the symptom.

Read the full error chain or backtrace and trace the affected callers. For ownership errors, identify the actual owner and required lifetime before adding clone, Arc, or unsafe. For optimized-only failures, compare cfg, assertions, overflow behavior, and timing rather than assuming an optimizer bug.

Choose the next observation to distinguish plausible causes. Follow file identity, buffer contents, short I/O operations, child streams, locks, and task completion across their real boundaries. A retry, panic catch, or discarded error can hide the mechanism without repairing it.

Use the appropriate debugger, profile, or syscall trace when runtime evidence is needed. Miri can detect some undefined behavior in supported executions, but it does not certify arbitrary OS interaction or prove overall soundness. Keep sensitive input and credentials out of diagnostic captures.

Control scheduling, fixtures, and dependencies enough to expose intermittent behavior; change one causal variable at a time. Fix the cause at its owner, replay the original failure, and check relevant neighboring paths.

Finish with a supported explanation and actual verification. Retain a regression check that detects the mechanism, remove temporary instrumentation, and name the next discriminating observation if uncertainty remains.
