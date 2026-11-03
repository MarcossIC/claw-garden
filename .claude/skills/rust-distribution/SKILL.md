---
name: rust-distribution
description: "Compatibility. Use when packaging or releasing a Rust CLI through Cargo, downloadable binaries, archives, or platform installers."
---

# Rust Distribution

**Compatibility.** Define which users can install and run the artifact: operating system, architecture, CPU baseline, system libraries, and installation method. Honor supplied requirements and settled technical choices; resolve only what the task leaves open.

Preserve the project's release tooling and supported target set. A source package and a prebuilt binary have different prerequisites. Cargo installation depends on a suitable toolchain and build dependencies; a downloaded executable depends on its actual linkage and platform baseline.

Build artifacts from the intended source revision and dependency resolution. Inspect executable naming, permissions, linked libraries, included licenses, and required runtime files. Cross-compiling successfully does not demonstrate that the binary starts on the oldest supported system. CPU-native optimization can silently narrow compatibility.

Keep profile decisions measurable. Stripping symbols changes diagnostic options and file size; it does not establish lower runtime memory. Static linking, especially with native dependencies, is a target-specific property rather than a universal portability switch. Test the produced artifact with its selected features and runtime assumptions.

Include help, completions, or manuals when they are part of the distribution contract, deriving them from the same command definition where supported. Keep archives and installation paths predictable, and make removal or replacement respect user-owned configuration and data.

Reuse the existing release mechanism for checksums, signatures, and provenance when required; distinguish artifact integrity from trusted origin. Keep registry credentials, signing identity, and publication authority owner-controlled.

Before an authorized publication, verify installation and a meaningful command using the final artifact in the relevant environment. Report exactly which targets and installation paths were exercised, and keep untested compatibility claims explicit.
