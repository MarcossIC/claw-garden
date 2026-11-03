---
name: rust-performance
description: "Evidence. Use for a reported Rust CLI startup, elapsed-time, throughput, CPU, or I/O problem, or a requested benchmark or optimization."
---

# Rust Performance

**Evidence.** Turn the performance goal into a workload, observable metric, and comparable baseline. Separate startup, time to first result, total runtime, throughput, memory, and binary size. Honor supplied requirements and settled technical choices; resolve only what the task leaves open.

Measure an optimized executable directly; cargo run adds orchestration and possible build work. Keep toolchain, target, features, profile, input, and environment comparable. Define the relevant warm or cold cache condition and control output destinations; terminal rendering or a slow pipe can dominate computation.

Choose evidence that separates plausible causes: CPU samples for computation, allocation profiles for churn, syscall or I/O evidence for repeated operations and waiting. Account for profiler overhead. Inspect complexity, repeated parsing, formatting, copies, and unnecessary initialization before changing low-level mechanics.

Compare repeated whole-command samples with a tool such as hyperfine. Use a microbenchmark for a specific operation, with realistic inputs and observable results; it cannot establish whole-command improvement. Select the comparison and sample plan before measuring, and retain variance rather than rerunning until a favorable sample appears.

Treat LTO, codegen units, optimization level, allocator changes, SIMD, and PGO as measured alternatives. Smaller code and maximum optimization settings do not guarantee faster execution. Preserve the target CPU baseline; native CPU tuning is unsuitable for a generally distributed binary unless that compatibility limit is deliberate.

Verify output, errors, ordering, and resource bounds alongside speed. Finish with the measured cause, intervention, before-and-after evidence, and limits of the conclusion. When evidence is inconclusive, retain the simpler correct implementation and state the uncertainty.
