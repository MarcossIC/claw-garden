# Phase 2 — rag-embedder — Archive Report

**Status**: Complete and archived.
**Completion date**: 2026-05-24.
**Test gates**: 249/249 Rust (was 204 baseline + 45 net new) PASS. 22 new Angular tests PASS. Zero regressions.

## Executive summary

Phase 2 shipped the local, deterministic embedder slice (`multilingual-e5-small`, 384 dims, ONNX-on-CPU) with first-run model download, per-file SHA-256 verification, disk-space pre-flight, retry/backoff, and live UI progress through the new `app.emit` + Angular `listen` pattern. The two architectural cornerstones are (1) a dedicated `std::thread` worker that confines the non-`Send` `fastembed::TextEmbedding` to a single owning thread accessed via `tokio::sync::mpsc`, making the public `FastEmbedLocal` provably `Send + Sync` at compile time, and (2) a sole-source terminal-`done` pattern enforced through a new `EmbedderEventSink` trait, introduced post-verify to fix a real double-emission bug that every prior gate had missed. All 15 locked decisions held — one feature-name correction (`ort-download-binaries`, not `-rustls-tls`) was caught at apply CHECKPOINT 1 via `cargo add --dry-run`. The verify phase escalated 5 warnings; 3 were fixed in code, 2 were resolved by canonicalizing the spec. Phase 3 (`rag-ingest`) is now unblocked.

## What shipped

- `trait Embedder: Send + Sync` (async via `async-trait`) with `embed`, `dimensions`, `model_id` methods.
- `FastEmbedLocal` concrete impl backed by a dedicated `std::thread` worker (named `fastembed-worker`) + `tokio::sync::mpsc` request/oneshot channel.
- Compile-time `Send + Sync` regression guard via const-block at module level.
- `OllamaEmbedder` + `OpenAIEmbedder` non-feature-gated stubs returning `unimplemented!()`.
- `FakeEmbedder` deterministic test double (siphasher-seeded PRNG, inline `#[cfg(test)]`).
- `model_download::ensure_model` with 5-file atomic download (`.partial` → SHA-256 verify → rename).
- `BackoffPolicy` trait + `RealBackoff` ([5,30,120] secs) + `ZeroBackoff` test double.
- `DiskOracle` trait + `Fs2Oracle` production impl + `MockDiskOracle` test double; ≥600 MB pre-flight.
- 5 pinned per-file SHA-256 constants (2 from HF LFS metadata, 3 computed locally from real one-shot download during apply CHECKPOINT 3).
- New `StreamEvent::EmbedderDownload { phase, bytes_downloaded, bytes_total, error }` variant — Rust + Angular union mirror.
- `EmbedderEventSink` trait with production `AppHandleSink` + test `MockSink` (introduced post-verify).
- `start_embedder_bootstrap` + `start_embedder_bootstrap_with_deps` test seam; spawned from the Tauri `setup` hook.
- `retry_embedder_download` Tauri command + `retry_inner_generic` test seam.
- `AppState.embedder: Arc<OnceCell<Arc<dyn Embedder>>>` pre-seeded in both test helpers.
- Angular `EmbedderService` with `signal<EmbedderStatus>` + lazy `listen()` + `retry()` + test seam `__test_handleEvent`.
- New "Embedder" section in `ConfigPanelComponent` with native `@switch` control flow, `role="progressbar"`/`aria-valuenow`, `role="status"`/`aria-live="polite"`, and `role="alert"` per phase.

## Decisions retrospective

| # | Locked decision | Held up in implementation? | Notes |
|---|-----------------|----------------------------|-------|
| 1 | FTS5 deferred to Phase 4 | yes | — |
| 2 | `StreamEvent::EmbedderDownload` variant added (not reuse `ToolProgress`) | yes | — |
| 3 | Tauri `app.emit` + Angular `listen` (NEW pattern) | yes | Precedent established and documented in design §5.6 |
| 4 | fastembed feature `ort-download-binaries` | **corrected during apply CHECKPOINT 1** | Explore documented `-rustls-tls` suffix — that feature does NOT exist on fastembed v5 (only `ort-download-binaries` and `ort-load-dynamic`); `-rustls-tls` belongs to `hf-hub` features which we dropped |
| 5 | `Embedder` trait + 3 impls (FastEmbedLocal real, Ollama/OpenAI stubs) | yes | — |
| 6 | Model `multilingual-e5-small` (384 dims, 5 files) | yes | — |
| 7 | `try_new_from_user_defined` with bytes we own (never hf-hub) | yes | — |
| 8 | Hardcoded SHA-256 constants per file | yes | 2 from HF LFS metadata; 3 computed locally during apply CHECKPOINT 3 |
| 9 | `Arc<OnceCell<Arc<dyn Embedder>>>` via `once_cell::sync::OnceCell` | yes | — |
| 10 | 3 attempts max, exponential backoff [5,30,120], no partial-resume | yes | — |
| 11 | `FakeEmbedder` inline `#[cfg(test)]` in `embedder.rs` | yes | Phase 3 will move it to `#[cfg(any(test, feature = "test-helpers"))]` when cross-file access is needed |
| 12 | Worker-thread architecture for `FastEmbedLocal`; compile-time Send+Sync assertion | yes | Const-block at module level, two separate `_assert_send<T: Send>` + `_assert_sync<T: Sync>` calls — spec explicitly allowed equivalents |
| 13 | Pre-existing `ToolProgress` Angular gap not Phase 2 scope | yes | Carried as follow-up |
| 14 | `fs2` (not `sysinfo`) | yes | Orchestrator override of explore §sysinfo recommendation; design §10 surfaced the contradiction |
| 15 | Strict TDD + `mockito` + `FakeEmbedder` | yes | Every test pair preceded its impl; CI never hits `huggingface.co`; no real sleeps |

## Architectural moves worth carrying forward

1. **`app.emit` + `listen` pattern for app-wide lifecycle events** — first usage in this codebase. Future app-wide signals (autoDream, re-indexing progress per doc 6, model-swap notifications) should follow the same pattern. Per-invoke Tauri Channels remain the right tool for request-scoped streaming (chat tokens, tool progress within a chat). The rule is documented in `design.md` §5.6.
2. **Dedicated `std::thread` worker for non-`Send` ONNX state** — `FastEmbedLocal` confines `fastembed::TextEmbedding` to a single owning thread accessed via `tokio::sync::mpsc` channel. The public type IS `Send + Sync` and the const-block compile assertion makes regression a compile error. Carry this pattern to ANY future native-runtime integration whose state is not natively `Send`.
3. **`EmbedderEventSink` trait abstraction** — introduced post-verify to fix the double-`done` bug and enable end-to-end testing of bootstrap → forwarder → emit. Future `app.emit` consumers should consider an equivalent trait abstraction if testability of the full emission chain matters.
4. **Test seam pattern for Tauri commands** — `retry_inner_generic<H>` and `start_embedder_bootstrap_with_deps` show how to factor real commands so the control flow can be unit-tested without a real `AppHandle`. Use this whenever a Tauri command's logic is non-trivial.
5. **Sole-source terminal events** — when an event family has "exactly one terminal" semantics (e.g., `done` here), exactly one layer of the call stack MUST own its emission. Any layer below that suppresses the terminal event and returns its success via the function return value, not via a side-channel event.

## Lessons learned (Phase 2)

### What surprised us

1. **`fastembed v5` feature `ort-download-binaries-rustls-tls` does NOT exist** — explore documented it via inference from feature naming patterns. Only `ort-download-binaries` and `ort-load-dynamic` exist; the `-rustls-tls` suffix belongs to `hf-hub` features (which we dropped). Caught at apply T001 via `cargo add --dry-run`. Lesson: ALWAYS verify feature names via `cargo info <crate>` or `--dry-run` before locking them into a proposal.
2. **First `cargo check` after dep addition compiled the full transitive chain** (fastembed + ort + tokenizers + onig_sys + ~50 deps) — but it was cached by the next CHECKPOINT 2 check (~6 s instead of the warned 5-10 min). The 5-10 min warning was over-cautious because the dep-resolution `cargo check` in CHECKPOINT 1 already triggered the full compile.
3. **The verify phase caught a real semantic bug that every prior gate missed**: double-`done` event because both `ensure_model` and `start_embedder_bootstrap` emitted a terminal `done`. Tests didn't catch it because the test seam (`start_embedder_bootstrap_with_deps`) stopped at `ensure().await` without exercising the forwarder. Fix: introduce `EmbedderEventSink` trait, make bootstrap the sole `done` source, and add a regression test with the test sink.
4. **Mockito's chunked delivery is finicky** — `downloading_events_rate_limited_by_64kb` initially over-asserted on the upper bound; widened the allowed range to absorb boundary splits + per-file final-flush events.
5. **3 small JSON files have NO LFS SHA exposed in HF API** — only LFS-large files get a SHA from the metadata endpoint. The 3 tokenizer config JSONs needed a one-shot real download in apply CHECKPOINT 3 to compute SHA-256 (~1.3 KB total — trivial).

### Anti-patterns we avoided (carry forward)

1. **NOT using `git stash` to verify regressions on untracked dirs** — Phase 1 lesson held; explicit output comparisons used throughout. `src-tauri/` is still `?? src-tauri/` in `git status`.
2. **NOT mocking the embedder for behavior tests of the embedder itself** — `FakeEmbedder` is for callers of the trait. The `FastEmbedLocal` worker plumbing is tested directly via channel round-trips and a `from_mock_worker` constructor that does not load a real model.
3. **NOT silently overriding the proposal** when design surfaced `sysinfo` vs `fs2` — design explicitly flagged the contradiction in §10, orchestrator locked `fs2`, proposal updated.

### Anti-pattern detected during apply (do NOT repeat in Phase 3+)

- **Apply silently emitting `Done` from two layers** (`ensure_model` + `start_embedder_bootstrap`) — a semantic violation hidden because each layer's tests passed in isolation. For Phase 3+, any lifecycle event with "exactly one terminal" semantics MUST have a regression test that exercises the FULL emission chain (not just the layer that produces it). Where the emission chain includes a Tauri `AppHandle`, introduce a sink trait (like `EmbedderEventSink`) so the chain becomes testable end-to-end.

## Open follow-ups (NOT Phase 2 scope, defer to future)

| Follow-up | Where | Owner phase |
|-----------|-------|-------------|
| `ToolProgress` Angular gap (`chat.model.ts` union + `chat.service.ts` switch) | `src/app/models/chat.model.ts`, `src/app/services/chat.service.ts` | unscheduled bug-fix ticket |
| AXE harness in CI (`vitest-axe` or `@axe-core/cli`) | project-wide test infra | unscheduled DX ticket |
| Pre-existing `app.spec.ts > should render title` failure | `src/app/app.spec.ts` | unscheduled — delete or rewrite for game shell |
| Real `FastEmbedLocal::from_bytes` `#[ignore]`'d test against the actual ONNX | `src-tauri/src/memory/embedder.rs::tests` | Phase 3 or manual smoke step |
| Hot-swap embedder + re-indexing per doc 6 lines 125-143 | TBD | Phase X |
| `commit_file` no-value wrapper around `std::fs::rename` | `model_download.rs:698-700` | cosmetic, Phase 3 if touched |
| `real_backoff_uses_correct_secs_table` consistency (`#[test]` vs `#[tokio::test]`) | `model_download.rs:1482` | cosmetic, low priority |

## Manual smoke (BLOCKED-BY-EXTERNAL — for the team)

Before Phase 3 starts, the team SHOULD run these in a real Tauri dev environment:

- Cold-start without cached model: observe full `start → downloading → verifying → done` sequence in ConfigPanel; verify 5 files at `~/.claw-garden/models/multilingual-e5-small/`.
- Cold-start with cached model: zero HTTP requests (devtools Network tab); `done` event near-instant.
- Offline cold-start (kill network + delete cache): error phase + Retry button visible; clicking Retry while still offline produces another `error`; re-enabling network + Retry succeeds.
- Browser AXE pass on ConfigPanel for all 5 phases.
- Verify the bootstrap emits EXACTLY ONE `done` event: `window.__TAURI__.event.listen('embedder-download', e => console.log(e))` → count `phase === 'done'`. Should be 1, not 2 (regression guard for the post-verify fix).

## Final test count

| Layer | Count | Delta from baseline |
|-------|-------|---------------------|
| Rust (`cargo test --lib`) | 249 PASS | +45 (baseline 204) |
| Angular (Phase 2 specs) | 22 PASS | +22 (new) |
| Pre-existing Angular failure | 1 (`app.spec.ts > should render title`) | pre-Phase-2, unrelated |
| Regressions | 0 | — |

## Files touched (summary)

### Rust (`src-tauri/`)

- NEW: `src/memory/embedder.rs`, `src/memory/model_download.rs`, `src/commands/embedder.rs`
- MODIFIED: `Cargo.toml`, `src/memory/mod.rs`, `src/commands/mod.rs`, `src/models/chat.rs`, `src/state.rs`, `src/lib.rs`

### Angular (`src/`)

- NEW: `app/services/embedder.service.ts` + `.spec.ts`, `app/models/chat.model.spec.ts`, `app/components/config-panel/config-panel.component.spec.ts`
- MODIFIED: `app/models/chat.model.ts`, `app/components/config-panel/config-panel.component.{ts,html,css}`

### Tooling

- NEW: `scripts/compute_sha.ps1` (one-shot helper used during CHECKPOINT 3 to pin the 3 small JSON SHA-256 constants)

### Docs

- All 6 artifacts in `docs/sdd/rag-embedder/` finalized (proposal, explore, spec, design, tasks, verify-report) plus this archive-report.

## Phase 3 (`rag-ingest`) — what's now unblocked

Phase 3 can begin. Its consumers will read:

- `AppState.embedder: Arc<OnceCell<Arc<dyn Embedder>>>` — Phase 3 worker MUST tolerate `embedder.get() == None` and skip embedding when not ready (per spec REQ-STATE-001 scenario "GIVEN a fresh `AppState::new()`, ... `state.embedder.get()` returns `None`").
- `FakeEmbedder` for tests — move it from inline `#[cfg(test)]` to `#[cfg(any(test, feature = "test-helpers"))]` at module scope when Phase 3 needs cross-file access (per proposal decision #11).
- The `EmbedderEventSink` trait pattern — consider an equivalent abstraction for any Phase 3 lifecycle events that need end-to-end testability.
