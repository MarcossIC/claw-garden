---
name: rust-concurrency
description: "Ownership. Use when Rust CLI threads, parallel iterators, async tasks, or shared state need capacity bounds, cancellation, ordering, or completion guarantees."
---

# Rust Concurrency

**Ownership.** Identify who admits work, observes failure, requests cancellation, and waits for completion. Honor supplied requirements and settled technical choices; resolve only what the task leaves open.

Keep sequential work sequential when it meets the requirement. Use scoped threads or existing parallel facilities for useful overlap; Rayon can fit CPU data parallelism, while an async runtime needs concurrent waiting or an existing integration that warrants it. Async does not make CPU computation faster.

Bound active work, queues, and retained results together. Worker count alone does not limit buffered input or ordered output waiting behind a slow item. Choose granularity against scheduling and copying costs, available resources, and nested parallelism. Preserve ordering only where the contract requires it.

Follow shared data and lock ownership. Arc shares ownership, not synchronized mutation; Send and Sync do not establish atomic business operations or freedom from deadlock. Keep lock scope small and inspect guards held across blocking calls or await points.

Cancellation is cooperative. Make blocked sends, receives, I/O, and workers able to finish or unblock. Dropping a thread JoinHandle detaches it; await or join owned work and surface failures. Started spawn_blocking work cannot be aborted merely by aborting its task handle.

Install signal handling only when the command needs graceful interruption. Use supported facilities to notify normal execution rather than performing arbitrary cleanup in a raw signal handler. Release terminal state and owned resources before completion.

Verify the relevant interruption, saturation, failure, or slow-consumer case with controlled coordination. A timer or clean compilation cannot prove termination; observe completion and leave no running test work.
