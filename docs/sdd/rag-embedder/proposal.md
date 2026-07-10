# Phase 2 — rag-embedder — Proposal

> **Reading order**: this file is the contract between `explore` and `spec`/`design`. `spec` and `design` run in parallel against it. Every section below is normative.
>
> **Inputs**: `docs/RAG-PHASE-PLAN.md` lines 119-162, `docs/6-improvements-RAG.md` lines 30-143, `docs/sdd/rag-embedder/explore.md` (full).

---

## Intent

Ship a local, deterministic embedder (`multilingual-e5-small`, 384 dims, ONNX-on-CPU) decoupled from the chat LLM, with first-run model download, SHA-256 integrity verification, and live UI progress. Without this, Phase 3 (`rag-ingest`) cannot generate vectors, so the entire RAG pipeline is blocked. Phase 2 is the dependency-critical unblocker — narrow scope, hard contract.

---

## Scope

### In

- New module `src-tauri/src/memory/embedder.rs` exposing `trait Embedder: Send + Sync` (async via `async_trait`).
- `FastEmbedLocal` concrete impl backed by `fastembed::TextEmbedding`, constructed from bytes we own (no `hf-hub` network code path).
- `OllamaEmbedder` and `OpenAIEmbedder` non-feature-gated stubs returning `unimplemented!()` for trait method bodies — they satisfy the trait so future phases can pattern-match without conditional compilation.
- New module `src-tauri/src/memory/model_download.rs` with on-first-run download of all 5 required files for `intfloat/multilingual-e5-small`, SHA-256 verification per-file, atomic on-disk layout under `<base_dir>/models/multilingual-e5-small/`, and `tokio::sync::mpsc::UnboundedSender<DownloadProgress>` instrumentation.
- Pre-flight disk-space check (≥600 MB free) before any HTTP traffic.
- 3-attempt download retry with exponential backoff (5s → 30s → 120s), no partial-resume.
- New `StreamEvent::EmbedderDownload { phase, bytes_downloaded, bytes_total, error }` variant added to `src-tauri/src/models/chat.rs` and to Angular `src/app/models/chat.model.ts`.
- App-wide event emission via `app.emit("embedder-download", payload)` + Angular `@tauri-apps/api/event::listen()` — a NEW pattern in this codebase, deliberately introduced and documented in `design.md`.
- `AppState.embedder: Arc<OnceCell<Arc<dyn Embedder>>>` (using `once_cell::sync::OnceCell` already in deps), filled once from a background task spawned in the setup hook.
- New Angular `EmbedderService` (`providedIn: 'root'`, signals) and a new "Embedder" section in the `ConfigPanelComponent` template (download progress UI, verifying spinner, ready state, retry button on error).
- `FakeEmbedder` deterministic test double (inline `#[cfg(test)]` in `embedder.rs`) for unit tests.
- `mockito`-based tests for `model_download.rs` covering: SHA-256 mismatch detected, all 5 files present skips download, retry succeeds on attempt 2.
- Compile-time `Send + Sync` assertion for `FastEmbedLocal`.

### Out

- **Fallback FTS5 search** when embedder is unavailable — Phase 4 (`rag-retrieval`) owns it. Phase 2 must NOT crash on missing embedder, but does NOT provide an alternative.
- **Real implementations** of `OllamaEmbedder` / `OpenAIEmbedder` — Phase X.
- **Hot-swap of the active embedder** — `OnceCell` is set-once by design. Changing the embedder requires app restart in v1. Re-indexing on embedder change is documented in doc 6 lines 125-143 as a future Phase X concern.
- **Partial-resume of failed downloads** — v1 re-downloads from scratch (~470 MB cost is acceptable for the rare cold-start error path).
- **Pre-existing `ToolProgress` Angular gap** — `StreamEvent::ToolProgress` exists in Rust but is missing from the Angular discriminated union (`chat.model.ts` line 56-66). Carry as follow-up; NOT Phase 2 scope.
- **`cargo build`** — explicitly forbidden by user CLAUDE.md. All verification via `cargo check`, `cargo test`, `cargo add --dry-run`, `cargo info`.
- **Refactoring `FakeEmbedder` for cross-file reuse** — inline `#[cfg(test)]` is sufficient for Phase 2; Phase 3 will move it behind `#[cfg(any(test, feature = "test-helpers"))]` when it needs cross-file access.

---

## Decisions (locked)

> The orchestrator resolved every open question from `explore.md`. The table below IS the contract. Spec and design integrate these — do not re-debate.

| # | Topic | Decision |
|---|-------|----------|
| 1 | Fallback FTS5 | Deferred to Phase 4 — out of scope here. |
| 2 | UI progress event variant | New `StreamEvent::EmbedderDownload { phase, bytes_downloaded, bytes_total, error }` in `src-tauri/src/models/chat.rs`. Angular union `chat.model.ts` mirrors it. |
| 3 | Event delivery mechanism | Tauri `app.emit("embedder-download", payload)` + Angular `listen()`. Payload IS the `StreamEvent::EmbedderDownload` variant (consistent serialization). NEW pattern in this codebase — document it in `design.md` as a precedent. Per-invoke `Channel<StreamEvent>` is NOT used here (the download happens at boot, before any chat). |
| 4 | fastembed-rs | Version `5`. `default-features = false`. Features: `["ort-download-binaries"]`. No `image-models`, no `hf-hub`, no `native-tls`. **Corrected during apply CHECKPOINT 1**: explore documented `ort-download-binaries-rustls-tls` but `cargo add --dry-run` proved that feature does not exist on fastembed v5 (only `ort-download-binaries` and `ort-load-dynamic`; `-rustls-tls` suffix belongs to hf-hub features which we drop). Our own download path uses `reqwest` which is rustls-default already. |
| 5 | Embedder trait | `pub trait Embedder: Send + Sync` (async via `async-trait`). `FastEmbedLocal` concrete impl. `OllamaEmbedder`, `OpenAIEmbedder` are NON-feature-gated stubs returning `unimplemented!()`. |
| 6 | Model | `multilingual-e5-small` (384 dims, ~470 MB), 5 files: `onnx/model.onnx` + `tokenizer.json` + `config.json` + `special_tokens_map.json` + `tokenizer_config.json`. |
| 7 | Download path | Use `fastembed::TextEmbedding::try_new_from_user_defined` with bytes WE downloaded — never `try_new`'s built-in `hf-hub` path. |
| 8 | SHA-256 source | Hardcoded constant `MODEL_SHA256` in `memory/model_download.rs`. Value extraction documented below (HF API + `X-Linked-ETag` header). Tokenizer files also SHA-checked with their own constants (NOT trusted blindly). |
| 9 | `AppState.embedder` shape | `pub embedder: Arc<OnceCell<Arc<dyn Embedder>>>` using `once_cell::sync::OnceCell` (`once_cell` already in deps). Lock-free reads. Single set from setup-hook background task. Cannot hot-swap — out of scope for Phase 2. |
| 10 | Download retry policy | 3 attempts max. Exponential backoff: 5s → 30s → 120s between attempts. NO partial-resume support in v1 — re-download from scratch on any failure. |
| 11 | `FakeEmbedder` location | Inline in `memory/embedder.rs` under `#[cfg(test)]`. Refactor to `#[cfg(any(test, feature = "test-helpers"))]` when Phase 3 needs cross-file access — out of scope here. |
| 12 | `Send` for `FastEmbedLocal` | Risk #6 from explore: `fastembed::TextEmbedding` may not be `Send`. Mitigation: wrap inner `TextEmbedding` in a single owned `tokio::task::JoinHandle` worker thread accessed via `tokio::sync::mpsc` channel (request-response pattern). The struct exposed publicly IS `Send + Sync`. Compile-time assertion `fn _assert_send_sync<T: Send + Sync>(){} _assert_send_sync::<FastEmbedLocal>();`. |
| 13 | Pre-existing `ToolProgress` Angular gap | NOT Phase 2 scope. Documented in "Follow-ups" below. |
| 14 | Disk space pre-flight | YES — before initiating download, check `<base_dir>` parent has ≥600 MB free. If not, emit `StreamEvent::EmbedderDownload { phase: "error", error: "Insufficient disk space (need 600 MB)" }` and abort. Use the `fs2 = "0.4"` crate (NEW dep; ~50 KB; single function `fs2::available_space(path) -> Result<u64>`; cross-platform; chosen over `sysinfo` which would pull ~1 MB of unused OS introspection). **Overrides explore §sysinfo recommendation per design.md §10.** |
| 15 | Test strategy | Strict TDD. Tests MUST NOT download the real 470 MB. Use: (a) `FakeEmbedder` for behavior tests, (b) tiny ONNX fixture (or skip-with-marker) for `from_bytes` smoke, (c) `mockito` (NEW dev-dep) to mock HF server for download tests covering SHA mismatch, "all 5 files present skips download", and "retry succeeds on attempt 2". |

### SHA-256 source extraction — authoritative protocol

The `MODEL_SHA256` constant for `onnx/model.onnx` is derived from HuggingFace LFS metadata. Verified during exploration (HEAD request, ~1 KB traffic; no binary downloaded):

**Command** (PowerShell or bash equivalent):

```bash
# Authoritative source #1 — HEAD on the resolve URL exposes the LFS SHA-256 in `X-Linked-ETag`
curl -sI "https://huggingface.co/intfloat/multilingual-e5-small/resolve/main/onnx/model.onnx" \
  | grep -i '^x-linked-etag:'
# → x-linked-etag: "ca456c06b3a9505ddfd9131408916dd79290368331e7d76bb621f1cba6bc8665"

# Authoritative source #2 — the tree API exposes the same value as the LFS object oid
curl -s "https://huggingface.co/api/models/intfloat/multilingual-e5-small/tree/main/onnx" \
  | jq '.[] | select(.path == "onnx/model.onnx") | .lfs.oid'
# → "ca456c06b3a9505ddfd9131408916dd79290368331e7d76bb621f1cba6bc8665"
```

Both sources MUST agree (cross-check is the integrity guarantee — if HF was compromised, both would need to be tampered consistently, raising the attack cost).

**Recorded values (verified 2026-05-23 against commit `614241f622f53c4eeff9890bdc4f31cfecc418b3`)**:

| File | Size (bytes) | SHA-256 / source |
|------|-------------:|------------------|
| `onnx/model.onnx` | 470 268 510 | `ca456c06b3a9505ddfd9131408916dd79290368331e7d76bb621f1cba6bc8665` (LFS oid + X-Linked-ETag) |
| `onnx/tokenizer.json` | 17 082 730 | `0b44a9d7b51c3c62626640cda0e2c2f70fdacdc25bbbd68038369d14ebdf4c39` (LFS oid) |
| `onnx/config.json` | 653 | git SHA-1 `698e598c24523f3857295b11abec0703d19ea180` — file is NOT LFS; SHA-256 must be computed once and recorded in apply phase |
| `onnx/special_tokens_map.json` | 167 | git SHA-1 `e0b1d18ecd0ae4ff1d47bd297d910c0cf83e504b` — file is NOT LFS; SHA-256 computed in apply |
| `onnx/tokenizer_config.json` | 443 | git SHA-1 `059214673d9d6d2ee319411e2ffec8c024b816d5` — file is NOT LFS; SHA-256 computed in apply |

The 3 small JSON files are non-LFS, so the API only exposes their git SHA-1 (blob hash) and not a SHA-256. The apply phase will download them once (~1.3 KB total), compute SHA-256 locally, and pin the constants. The risk window is "first download by the team member writing the constants" — acceptable for the small JSON files, and the model.onnx + tokenizer.json (which are 99.99% of total bytes) are pinned from authoritative LFS metadata up front.

---

## Approach

**Boot sequence** (`src-tauri/src/lib.rs` setup hook):

1. `AppState::new()` runs synchronously as today; it initializes `embedder: Arc::new(OnceCell::new())` (empty).
2. After `app.manage(state)`, the setup hook spawns a `tauri::async_runtime::spawn(async move { ... })` task that owns the embedder bootstrap.
3. The task: (a) reads `base_dir` from `AppState`, (b) emits `EmbedderDownload { phase: "start", ... }` via `app.emit`, (c) calls `memory::model_download::ensure_model(&base_dir, Some(progress_tx)).await`, (d) on success constructs `FastEmbedLocal::from_bytes(...)` (CPU-bound, runs inside `tokio::task::spawn_blocking`), (e) sets the `OnceCell`, (f) emits `EmbedderDownload { phase: "done", ... }`.
4. A forwarder task translates `UnboundedReceiver<DownloadProgress>` events into `app.emit("embedder-download", StreamEvent::EmbedderDownload { ... })` calls — same translator pattern as `ToolProgress` (chat.rs:855-897).

**Download flow** (`memory/model_download.rs::ensure_model`):

1. Compute target dir `<base_dir>/models/multilingual-e5-small/`.
2. If ALL 5 files exist and ALL 5 SHA-256 verifications pass → return cached bytes. Skip download. (Test: "all files present skips download".)
3. Otherwise, pre-flight: check ≥600 MB free on the disk holding `<base_dir>`. If not, emit `error` event and return `DownloadError::InsufficientDisk`.
4. For each file in `[model.onnx, tokenizer.json, config.json, special_tokens_map.json, tokenizer_config.json]`:
   1. Download to `<file>.partial` via `reqwest` `stream()` — chunked.
   2. After every chunk, push `DownloadProgress::Chunk { file, bytes_downloaded, bytes_total }` to the channel.
   3. On HTTP error or network error: backoff (5s/30s/120s schedule), retry up to 3 attempts total. After attempt 3 fails: delete `.partial`, return `DownloadError::Network`.
   4. Once complete, push `DownloadProgress::Verify { file }`, compute SHA-256, compare against the per-file constant. If mismatch: delete `.partial`, return `DownloadError::ChecksumMismatch { file, expected, actual }`.
   5. Atomic `rename(<file>.partial, <file>)`.
5. After all 5: push `DownloadProgress::Done`, return `ModelFiles { onnx, tokenizer }`.

**Embedder lifecycle**:

- `FastEmbedLocal::from_bytes(onnx, tokenizer)`: spawns a single worker thread via `std::thread::spawn` that owns the inner `fastembed::TextEmbedding`. Holds a `tokio::sync::mpsc::UnboundedSender<EmbedRequest>` where `EmbedRequest = (String, tokio::sync::oneshot::Sender<Result<Vec<f32>, EmbedError>>)`.
- `Embedder::embed` sends a request and awaits the oneshot. This makes the public struct `Send + Sync` regardless of `TextEmbedding`'s own bounds (Risk #6 mitigation, decision #12).
- Determinism: `multilingual-e5-small` is deterministic by construction — no dropout in inference, fixed weights. Test asserts bit-identical output for same input.

**Event flow** (Rust → Angular):

```
[setup hook spawn] ───┐
                       ├─→ ensure_model(progress_tx) ──> DownloadProgress
                       │                                    │
                       │                                    ▼
                       │                          [forwarder task]
                       │                                    │
                       │     app.emit("embedder-download", StreamEvent::EmbedderDownload {…})
                       │                                    │
                       ▼                                    ▼
                  OnceCell.set(…)               ┌─────────────────────────┐
                                                 │ Angular EmbedderService │
                                                 │  listen("embedder-     │
                                                 │   download", e => …)   │
                                                 │  status = signal({…})  │
                                                 └─────────────────────────┘
                                                              │
                                                              ▼
                                                   ConfigPanelComponent
                                                   @switch (status().phase) {…}
```

**Error paths**:

- Network failure after 3 retries → emit `{phase: "error", error: "..."}`, OnceCell stays empty. Phase 4 fallback (FTS5) will activate when implemented. Phase 3 (`rag-ingest`) MUST be specced to tolerate `embedder = None`.
- SHA-256 mismatch → emit `error`, do NOT retry (likely MITM or corruption — retry would just re-fail), OnceCell stays empty.
- Insufficient disk → emit `error` before any HTTP traffic, OnceCell stays empty.
- Embedder load failure (`from_bytes` returns `Err`) → emit `error`, OnceCell stays empty.

All error paths are non-panicking. The app keeps running. Chat continues to work (RAG is additive — without an embedder, the chat hot path runs as it does today).

---

## Files to be created/modified

### New files

| Path | Purpose |
|------|---------|
| `src-tauri/src/memory/embedder.rs` | `trait Embedder`, `EmbedError`, `FastEmbedLocal`, `OllamaEmbedder` stub, `OpenAIEmbedder` stub, `FakeEmbedder` (`#[cfg(test)]`), compile-time `Send + Sync` assertion. |
| `src-tauri/src/memory/model_download.rs` | `pub async fn ensure_model`, `pub struct ModelFiles`, `pub enum DownloadProgress`, `pub enum DownloadError`, `MODEL_SHA256` + 4 tokenizer SHA constants, retry/backoff logic, atomic file commit. |
| `src-tauri/src/commands/embedder.rs` *(optional — propose phase recommends; spec/design confirm)* | `#[tauri::command] async fn get_embedder_status(...) -> EmbedderStatus` + `#[tauri::command] async fn retry_embedder_download(...)` for the UI retry button. NO subscribe command — events flow via `app.emit`. |
| `src/app/services/embedder.service.ts` | `@Injectable({ providedIn: 'root' })`, `readonly status = signal<EmbedderStatus>(...)`, `listen("embedder-download", e => …)` subscription, `retry()` method. |

### Modified files

| Path | Current (line refs) | Planned change |
|------|---------------------|----------------|
| `src-tauri/Cargo.toml` | Lines 44-50 ("RAG foundations (Phase 1)" deps section) | Add "Phase 2 (embedder)" subsection with `fastembed`, `sysinfo`, `mockito` (dev-dep). |
| `src-tauri/src/state.rs` | Lines 32-70 (AppState struct), 73-132 (`new()`), 138-195 (`new_for_test()`), 197-245 (`new_for_test_async()`) | Add `pub embedder: Arc<OnceCell<Arc<dyn Embedder>>>` field. `new()` initializes empty cell. `new_for_test()` / `new_for_test_async()` pre-seed cell with `FakeEmbedder`. |
| `src-tauri/src/lib.rs` | Lines 17-66 (`run()` setup hook) | Spawn embedder bootstrap task inside `.setup()` closure (see "Approach" above). |
| `src-tauri/src/models/chat.rs` | Lines 96-229 (`enum StreamEvent`) | Add `EmbedderDownload { phase, bytes_downloaded, bytes_total, error }` variant with `#[serde(rename = "embedder_download")]`. |
| `src-tauri/src/memory/mod.rs` | Current: `pub mod db, error, identity, migrations, record;` | Add `pub mod embedder;` and `pub mod model_download;`. |
| `src/app/models/chat.model.ts` | Lines 56-66 (`StreamEvent` union) | Add discriminant `{ kind: 'embedder_download'; phase: 'start' \| 'downloading' \| 'verifying' \| 'done' \| 'error'; bytes_downloaded?: number; bytes_total?: number; error?: string }`. |
| `src/app/components/config-panel/config-panel.component.html` | Lines 1-183 | Add new `<section class="config-section">` for "Embedder" with `@switch (embedderService.status().phase)` blocks and a progress bar. Use native control flow (`@if`/`@switch`/`@for`) per project conventions. |
| `src/app/components/config-panel/config-panel.component.ts` | Lines 1-120 | Inject `EmbedderService` via `inject()`. No new state — read-through to the service signals. Keep `ChangeDetectionStrategy.OnPush`. |

### Not modified (out of Phase 2)

- `src-tauri/src/commands/chat.rs` — Phase 4 will prepend memory blocks to system prompt.
- `src-tauri/src/memory/db.rs` — no schema changes; `float[384]` already in place.
- Any migration file — no SQL change.

---

## Deps to add

| Crate | Where | Why |
|-------|-------|-----|
| `fastembed` | `[dependencies]`, version `"5"`, `default-features = false`, `features = ["ort-download-binaries"]` | Embedder runtime. Drops `default-features` to avoid `image-models`, `hf-hub`, `native-tls`. Pulls `ort 2.0.0-rc.12` (precompiled ONNX Runtime) + `tokenizers 0.22.2` transitively. Validated via `cargo add --dry-run` in apply CHECKPOINT 1. NOTE: explore documented `-rustls-tls` suffix; that feature does NOT exist on fastembed v5 (verified). |
| `sysinfo` | `[dependencies]`, version `"0.31"` (or whatever latest stable resolves) | Cross-platform disk-space pre-flight (decision #14). Pure Rust API. Alternative considered: `fs2` crate. Pick `sysinfo` because it's more actively maintained and used elsewhere in the Tauri/Rust ecosystem. Spec phase to confirm exact minor version via `cargo info`. |
| `mockito` | `[dev-dependencies]`, version `"1"` (latest stable) | Mock HTTP server for `model_download.rs` tests (decision #15). Lets us cover SHA mismatch, retry success-on-attempt-2, and skip-when-cached without touching the network or downloading the real model. |

**Already in deps (verified in explore)**: `sha2`, `reqwest` (with `stream`), `tokio`, `async-trait`, `dirs`, `once_cell`, `serde`, `serde_json`. No new versions of these.

**Transitive deps pulled by `fastembed`** (informational, no action): `ort 2.0.0-rc.12` (pinned exact by fastembed), `tokenizers 0.22.2` (with `onig` feature — first build will compile `onig_sys` native C on Windows, ~5-10 minutes), `ndarray 0.17.2`, `safetensors 0.7.0`.

---

## Risks

| # | Description | Likelihood | Impact | Mitigation | Owner phase |
|---|-------------|------------|--------|------------|-------------|
| 1 | `ort 2.0.0-rc.12` is a release candidate, not stable. Future `ort 2.0.0` release may break `fastembed`. | Low (fastembed pins `=2.0.0-rc.12` exactly) | Medium (would require fastembed update) | Document in design.md. Pin `fastembed = "=5.13.x"` if drift becomes evident. No Phase 2 action. | design (documentation), Phase X (if it breaks) |
| 2 | `tokenizers` crate `onig` feature builds native C on Windows. First `cargo build` after Phase 2 lands takes 5-10 minutes. | High (first build only) | Low (one-time per dev) | Warn team in design.md README section. Not a blocker. | design (documentation) |
| 3 | Permanent embedder unavailability (no network at first run + no cached model). | Medium (cold-start on offline machine) | High (RAG fully degraded until Phase 4 ships) | Phase 2 ensures no crash, no panic, OnceCell stays empty. UI surfaces error with Retry. Phase 4 owns the FTS5 fallback. Phase 3 MUST tolerate missing embedder. | Phase 2 (no-crash guarantee), Phase 3 (tolerate `None`), Phase 4 (fallback) |
| 4 | Disk space exhaustion at first run (~475 MB needed). | Medium (constrained dev machines, CI) | High (download silently fails halfway) | Pre-flight `sysinfo` check ≥600 MB before any HTTP. Emit `error` event with explicit message. (Decision #14.) | apply |
| 5 | `AppState::new_for_test()` (sync) panics if called from a tokio runtime — pattern already exists for `MemoryStore`. Tests must pre-seed `OnceCell` with `FakeEmbedder` synchronously OR migrate to `new_for_test_async()`. | Low | Medium (test-only) | Pre-seed the cell in BOTH test helpers. Document the constraint in design.md. | apply (must add seed call), design (must spec) |
| 6 | `fastembed::TextEmbedding` may not be `Send`. Required for `Arc<dyn Embedder>` with `Send + Sync` bounds. | Medium (verified `ort 2.x` Session is Send, but combined struct unverified) | High (would block public type) | Channel-based worker thread architecture (decision #12). Public struct exposes a `Sender`, NOT the inner `TextEmbedding`. Compile-time `_assert_send_sync::<FastEmbedLocal>()` makes regressions a compile error. | spec (define the worker), apply (implement) |
| 7 | NEW pattern `app.emit` + `listen` introduces precedent (no prior Rust→Angular global events in this codebase). Risk of inconsistent future usage (per-invoke Channel vs app.emit). | Low | Low | Document the pattern + rationale in design.md as a codebase precedent. Rule: per-invoke Channel for request-scoped events; `app.emit` for app-wide lifecycle events (embedder, future model-swap, future updates). | design (documentation) |
| 8 | SHA-256 of 3 small JSON tokenizer files is NOT exposed by HF API (non-LFS files). Initial value must be computed by whoever runs apply. | Low | Low (1.3 KB total, manual compute is trivial) | Apply phase task: download once, compute, pin constants, document the computed values in the source file as a comment. | apply |

---

## Acceptance Criteria

Drawn from `RAG-PHASE-PLAN.md` lines 146-152 plus the additional contract items locked here:

1. **Determinism**: `Embedder::embed("hola mundo").await` returns a `Vec<f32>` of length exactly 384, identical bit-for-bit across runs (same input → same output).
2. **Cold start without cached model**: download proceeds, UI shows progress (start → downloading → verifying → done), SHA-256 verifies, model is cached on disk, embedder becomes available within reasonable time.
3. **Cold start with cached model**: app starts without network, embedder becomes available at first use (no re-download).
4. **Offline first-run without model**: app does NOT crash. Embedder stays unavailable. UI surfaces clear error with Retry button. RAG operates in degraded mode (Phase 4 fallback when implemented; explicit error otherwise).
5. **SHA-256 mismatch test PASSES**: mockito-served corrupt bytes are detected, file is deleted, error is surfaced.
6. **Retry test PASSES**: mockito-driven attempt 1 fails (500), attempt 2 succeeds → final state is success.
7. **Skip test PASSES**: if all 5 files exist and SHA-256 matches, `ensure_model` does NOT make any HTTP request.
8. **Pre-flight test PASSES**: if available disk < 600 MB, `ensure_model` returns `DownloadError::InsufficientDisk` without HTTP.
9. **Compile-time assertion**: `fn _assert_send_sync<T: Send + Sync>(){} _assert_send_sync::<FastEmbedLocal>();` compiles (it is a regression guard, not a runtime test).
10. **Tests do NOT download the real 470 MB model.** No test touches `huggingface.co` over the network. CI completes in seconds, not minutes.
11. **`cargo build` is NOT required to run tests** — `cargo test --lib memory` should pass with `cargo check`-level verification (matches Phase 1 convention).
12. **Existing test suite**: 204/204 still PASS plus new Phase 2 tests (target ~15-25 new tests). No regression.
13. **`app.emit` pattern is documented** in `design.md` as a codebase precedent with explicit rule for when to use it vs per-invoke Channel.
14. **Angular UI**: ConfigPanel Embedder section renders all 5 phases correctly (start, downloading with progress bar, verifying spinner, done with checkmark, error with retry button). Native `@switch` control flow. `OnPush`.
15. **Accessibility (per project CLAUDE.md)**: Embedder UI section passes AXE checks, WCAG AA color contrast, ARIA attributes for the progress bar (`role="progressbar"`, `aria-valuenow`, `aria-valuemin`, `aria-valuemax`).

### Strict TDD task ordering (for `tasks.md`)

Every implementation task is preceded by its test task. Suggested ordering:

1. `embedder-trait-test` — fake impl satisfies trait, dims = 384, determinism, Send + Sync assertion.
2. `embedder-trait-impl` — define `trait Embedder`, `EmbedError`, `FakeEmbedder`, compile-time assertion.
3. `fastembed-local-test` — `from_bytes` with tiny ONNX fixture (or skip-marker), worker-thread send/recv smoke.
4. `fastembed-local-impl` — `FastEmbedLocal` struct + worker-thread architecture.
5. `model-download-skip-test` — mockito server NOT hit if all 5 files cached + SHA-OK.
6. `model-download-sha-mismatch-test` — mockito serves corrupt bytes → `DownloadError::ChecksumMismatch`.
7. `model-download-retry-test` — mockito attempt 1 = 500, attempt 2 = success.
8. `model-download-diskspace-test` — `sysinfo` mock returns <600 MB → `DownloadError::InsufficientDisk`, no HTTP.
9. `model-download-impl` — `ensure_model`, retry loop, atomic rename, SHA constants.
10. `streamevent-variant-test` — serde round-trip for `EmbedderDownload` variant matches `chat.model.ts` shape.
11. `streamevent-variant-impl` — add variant to `chat.rs`.
12. `appstate-embedder-test` — `OnceCell` empty by default, `new_for_test*` pre-seed with `FakeEmbedder`.
13. `appstate-embedder-impl` — add field to `AppState`, seed in test helpers.
14. `setup-hook-test` — verify setup spawns the bootstrap task (use a `FakeEmbedderProvider` injected via cfg).
15. `setup-hook-impl` — spawn in `lib.rs::run()`.
16. `angular-embedder-service-test` — service receives `embedder_download` events, signal updates.
17. `angular-embedder-service-impl` — `EmbedderService` with `listen()` subscription.
18. `angular-config-panel-test` — component renders 5 phases correctly.
19. `angular-config-panel-impl` — template + minimal TS wiring.

(Spec phase finalizes the exact ordering; tasks phase produces the checklist.)

---

## Open questions

**Empty.** The orchestrator locked every question from `explore.md`. If `spec` or `design` discovers a new ambiguity, they MUST raise it back to the orchestrator before deciding — do not invent answers silently.

---

## Follow-ups (out of Phase 2)

- **ToolProgress Angular gap**: `StreamEvent::ToolProgress` exists in `src-tauri/src/models/chat.rs` but is NOT in the Angular union `src/app/models/chat.model.ts:56-66`, and `chat.service.ts:118-157` has no `case 'tool_progress':` arm. The Rust side emits these events; Angular silently drops them. This is a pre-existing bug surfaced by exploration. File a separate issue.
- **Hot-swap embedder UI**: when the user changes the embedder in advanced settings, the system needs to swap implementations and trigger re-indexing. `OnceCell` (Phase 2 decision #9) cannot swap — Phase X must replace it with `arc_swap::ArcSwap` (or equivalent) when the feature is built. Documented in doc 6 lines 125-143.
- **Re-indexing on embedder change**: per doc 6 lines 125-143, changing embedder invalidates the vector DB. Build the migration command (`reindex(...)`) when hot-swap is wired up. Phase X.
- **`FakeEmbedder` cross-file reuse**: when Phase 3 (`rag-ingest`) needs `FakeEmbedder` from the worker tests, refactor from `#[cfg(test)]` inline to `#[cfg(any(test, feature = "test-helpers"))]` at module level. Trivial change — no migration cost.
- **`mockito` vs `wiremock`**: if other future phases need HTTP mocking, evaluate `wiremock` (async-native, more featureful). Phase 2 uses `mockito` because it's simpler and the test surface is small.
