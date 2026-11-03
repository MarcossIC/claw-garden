---
name: rust-errors
description: "Failure semantics. Use when Rust CLI errors, diagnostics, exit statuses, partial success, or cleanup can misrepresent the command's outcome."
---

# Rust Errors

**Failure semantics.** Follow failure from the operation that knows what happened to the process boundary that reports it. Honor supplied requirements and settled technical choices; resolve only what the task leaves open.

Use Result for expected failures and preserve the underlying cause when adding useful operation or path context. Retain typed distinctions when callers choose different recovery behavior; an application-level error wrapper can simplify reporting. Reuse the project's error approach before adding another crate. Classify structured errors rather than matching display strings.

Decide whether the command stops immediately, continues with individual failures, or reports partial success. Keep that decision consistent with output and side effects. A printed diagnostic followed by a successful exit can mislead automation; no matches or empty input may be a valid domain outcome.

Keep panic and unwrap out of ordinary invalid-input and I/O paths. A panic strategy is not an error policy. Render errors once at the CLI boundary, preserve causal detail where useful, and avoid leaking secrets through diagnostic context or debug output.

Complete fallible output and cleanup before reporting success. Returning ExitCode from main allows normal scope cleanup; process::exit skips Rust destructors. Resource release through Drop does not make flush or durable-write errors observable.

Treat a downstream stdout BrokenPipe according to the command's pipeline contract, without suppressing unrelated read, file-write, or stderr failures. Interruption does not reverse effects already performed.

Verify the relevant failure through exit status, diagnostics, output, and resulting state. Choose an assertion that would expose false success or an unintended effect.
