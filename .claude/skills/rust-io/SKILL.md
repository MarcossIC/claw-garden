---
name: rust-io
description: "Streaming. Use when Rust CLI stdin, stdout, files, parsers, or pipelines need bounded buffering, byte fidelity, throughput, or correct I/O failure handling."
---

# Rust I/O

**Streaming.** Trace bytes from their source to their consumer, including what must remain buffered between steps. Honor supplied requirements and settled technical choices; resolve only what the task leaves open.

Use Read, BufRead, Write, and existing parsers before building an I/O abstraction. Stream growing input when the operation permits it; whole-input reads can be appropriate for demonstrably bounded data. Identify when sorting, aggregation, or lookahead genuinely requires retention and bound or externalize that state.

Choose bytes or UTF-8 text from the actual contract. Preserve delimiters, final unterminated records, and invalid-byte behavior where meaningful. A read may return fewer bytes than requested. A short read is not EOF, and a buffer boundary is not a record boundary.

Reuse buffers where repeated allocation matters. Reading by lines still permits one unbounded line; enforce required record limits while reading, before allocating the complete record. Bound decompressed data and pending output as well as source bytes. Treat a configured limit as an explicit error rather than silent truncation.

Lock standard streams for sustained access and buffer repeated small writes when useful. Use complete-write operations where required and observe the final flush result; BufWriter drop can hide errors. Balance batching with interactive latency and let slow consumers constrain production.

Handle Interrupted and BrokenPipe at the correct boundary using the command's policy. Propagate other failures without converting them to clean EOF or success.

Verify chunk splits, the relevant oversized or malformed record, partial writes, and late output failure through controlled readers or writers. Use real pipes when backpressure or downstream closure is the claim.
