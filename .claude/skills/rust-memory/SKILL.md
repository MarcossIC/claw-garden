---
name: rust-memory
description: "Retention. Use when Rust CLI allocation rate, peak memory, buffer growth, or data ownership needs diagnosis or reduction."
---

# Rust Memory

**Retention.** Account for what stays live, how large it can become, and which owner releases it. Separate allocation count, live heap, peak resident memory, stack, and mapped pages. Honor supplied requirements and settled technical choices; resolve only what the task leaves open.

Trace memory as input size and concurrency grow. Include parser buffers, collections, queues, worker state, output reordering, and captured subprocess data. Streaming one stage does not bound a later collector. Name the term that grows before selecting a smaller representation.

Remove demonstrated redundant copies and intermediate collections. Borrow where lifetimes stay simple, move owned data when possible, and reuse buffers in repeated work. A tiny borrowed view or shared reference can retain a large owner; copying a small surviving value may reduce total retention.

Choose capacity from credible bounds or observed distributions. Vec::clear retains capacity; repeated shrinking can replace retention with allocation churn. Account for oversized-record recovery and long-lived buffers. Inline collections enlarge every containing value, and large stack arrays can exhaust worker stacks.

Measure before introducing interning, arenas, custom allocators, alternative hashers, or unsafe storage. Preserve collision resistance where input is adversarial. Memory mapping changes paging and lifetime behavior; file-backed maps require safety conditions against external modification and are not a universal zero-memory read.

Use allocation evidence to locate churn and resident-memory evidence for the process budget. Account for allocator retention, page cache conditions, child processes, and measurement overhead. A falling allocation count alone does not establish lower peak memory.

Compare representative and growing inputs under equivalent conditions, preserving output and failures. Finish with the changed owner or growth term, observed memory results, and any remaining bound or uncertainty.
