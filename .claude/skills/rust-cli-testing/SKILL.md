---
name: rust-cli-testing
description: "Mechanism. Use when Rust CLI tests must establish real argument parsing, exit status, streams, filesystem effects, pipes, signals, or packaged-executable behavior."
---

# Rust CLI Testing

**Prove the mechanism.** Choose the smallest boundary that includes what makes the command's promise true. Calling a function cannot establish executable parsing, process status, or terminal behavior. Honor supplied requirements and settled technical choices; resolve only what the task leaves open.

Run the actual built binary through Cargo's integration-test support or the established command-test helper. Reuse assert_cmd or equivalent when already suitable. Set arguments, stdin, environment, and working directory explicitly; isolate fixtures in owned temporary directories and keep their guards alive until children finish.

Assert stdout, stderr, status, and file effects where they matter. Preserve relevant byte and value distinctions; normalizing paths, line endings, colors, or JSON must not conceal the behavior under test. Snapshot intended interfaces without accepting every new snapshot as correct.

Choose scenarios that expose a plausible failure: a conflicting option, explicit configuration override, non-UTF-8 path on a supporting platform, or error after partial work. Keep destructive and external effects inside controlled fixtures.

Piped capture does not exercise a terminal. Use real pipes for backpressure and downstream closure, and a PTY when terminal detection or interaction is the claim. Coordinate at observable boundaries rather than relying on sleeps. A timeout must also stop and reap the child; an assertion failure must not leak processes.

Bound captured output and fixture size so the harness does not exhaust memory while testing memory behavior. Test platform-specific semantics on the relevant platform; cross-compilation alone is insufficient.

Confirm selected tests actually ran and cleanup completed. Report the command and boundary exercised, including any unavailable platform, terminal, or release-artifact evidence.
