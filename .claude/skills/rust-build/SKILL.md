---
name: rust-build
description: "Resolution. Use for Rust CLI Cargo configuration, toolchains, dependency features, compiler or linker failures, and requested build-profile changes."
---

# Rust Build

**Resolution.** Explain what Cargo actually selects before changing its declarations. Inspect the workspace, toolchain, edition, rust-version, target, features, profiles, configuration, and environment. Honor supplied requirements and settled technical choices; resolve only what the task leaves open.

Separate the compiler running the build, language edition, minimum supported Rust version, and executable target. Declaring rust-version does not prove dependencies or source compile on that version; exercise the supported baseline. A host build cannot establish target linker or system-library compatibility.

Trace dependencies through the resolved graph and feature activation. Declare directly used crates in the consuming package. Preserve the application's Cargo.lock and use locked resolution where reproducibility is required. Inspect transitive defaults: disabling a feature on one edge does not prevent another dependency from enabling it.

Choose only dependencies and features the present CLI needs. Review their maintenance, licensing, and vulnerability evidence when changing them, with scope proportional to the change. Keep build scripts and procedural macros in the build-time trust analysis. Do not fabricate lockfile checksums or bypass integrity checks to fix downloads.

Understand workspace-root profile settings and supported feature combinations. Optimize for the requested metric; strip, LTO, codegen units, and panic strategy have different effects. Panic abort changes cleanup behavior and cannot be treated as a free size setting.

Validate the affected layer with formatting, compiler checks, relevant Clippy diagnostics, tests, or artifact execution. Check intended feature configurations; an all-features build can hide a missing default-path dependency. Explain the resolved mismatch and actual correction, distinguishing compilation from tested runtime behavior.
