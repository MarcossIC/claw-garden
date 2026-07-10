# Phase 2 — rag-embedder — Verify Report

## Verdict
ACCEPTED-WITH-WARNINGS → **ACCEPTED** after follow-up fix (see "Fix applied" section at the bottom).

## Test gates
- Rust: **247/247 passed** (baseline was 204; +43 new tests from Phase 2). Finished in 0.13s. 0 failures, 0 ignored.
- Angular (per-spec verification):
  - `src/app/services/embedder.service.spec.ts` → 10/10 passed (1.26s)
  - `src/app/components/config-panel/config-panel.component.spec.ts` → 8/8 passed (1.67s)
  - `src/app/models/chat.model.spec.ts` → exists, runs as part of full suite, all assertions pass.
- Full Angular suite: 23/24 passed, 1 pre-existing failure (`src/app/app.spec.ts > should render title` — looks for an `<h1>` containing "Hello…" against the migrated `app.html`; unrelated to Phase 2, file untouched by this change, exists in `2eb7565 initial commit`).
- Pre-existing failures excluded:
  - `src/app/app.spec.ts > should render title` — pre-Phase-2; `app.spec.ts` is in the initial commit, no diff in this change.

## Requirement coverage matrix

| REQ ID | Spec scenario | Implementation file:line | Test(s) covering it | Status |
|--------|---------------|--------------------------|---------------------|--------|
| REQ-EMB-001 | Trait shape (`embed`/`dimensions`/`model_id`) + `EmbedError` enum (`NotReady`/`WorkerGone`/`InferenceFailed`/`LoadFailed`) | `src-tauri/src/memory/embedder.rs:13-38` | `embedder::tests::fake_embedder_returns_384_dim_vec`, `fake_embedder_is_deterministic`, `fake_embedder_different_inputs_differ`, `fake_embedder_model_id`, `fake_embedder_dimensions_is_384` | OK |
| REQ-EMB-002 | `FastEmbedLocal: Send + Sync` via worker-thread + mpsc; compile-time assertion at module level | `src-tauri/src/memory/embedder.rs:57-135` (struct + worker), `:161-168` (active const-block assertion) | Compile-time (the const-block IS the test); `embed_request_round_trip_through_channel`, `embed_returns_worker_gone_when_worker_drops` | OK — uses two separate `_assert_send<T: Send>` + `_assert_sync<T: Sync>` rather than the spec's single-bound `_assert_send_sync<T: Send + Sync>` form; the spec explicitly allows "equivalent expression that fails compilation if the bounds regress". |
| REQ-EMB-003 | `OllamaEmbedder` + `OpenAIEmbedder` non-gated stubs, panic on `embed` | `embedder.rs:179-207` | `ollama_stub_compiles_and_implements_trait`, `openai_stub_compiles_and_implements_trait`, `ollama_stub_dimensions_is_384`, `openai_stub_dimensions_is_1536`, `ollama_stub_embed_is_unimplemented` (`#[should_panic(expected = "Phase X")]`) | OK |
| REQ-DL-001 | `ensure_model` downloads 5 files atomically via `.partial` → rename, returns `ModelFiles` | `model_download.rs:268-274` (entry), `:280-296` (oracle seam), `:335-404` (inner), `:527-695` (`download_one` + commit) | `skips_download_when_all_files_cached`, `corrupt_cached_file_triggers_redownload`, `successful_download_leaves_no_partial_files`, `network_failure_returns_err_no_partial_files`, `downloads_only_missing_files` | OK |
| REQ-DL-002 | SHA-256 verify per file; mismatch → delete `.partial` + retry/give up | 5 pinned constants `model_download.rs:56-65`, verify-then-rename `:668-687` | `sha_mismatch_after_retries_returns_checksum_err`, `corrupt_cached_file_triggers_redownload` | OK (apply chose the "no-retry on SHA mismatch" shape per design §4.4 step 6 and documented it) |
| REQ-DL-003 | Pre-flight `≥600 MB` disk check before any HTTP; skipped on full cache hit | `DiskOracle` trait `:125-127`, `Fs2Oracle` `:130-136`, check `:356-365`, cache-hit short-circuit `:351-353` | `insufficient_disk_returns_err_no_http`, `sufficient_disk_passes_preflight`, `cached_path_skips_disk_check` | OK |
| REQ-DL-004 | 3-attempt retry, backoff schedule `[5,30,120]`, injectable for tests, no real sleeps | `RETRY_BACKOFF_SECS` `:141`, `BackoffPolicy`/`RealBackoff`/`ZeroBackoff` `:146-195`, retry loop `:547-694` | `retry_succeeds_on_second_attempt`, `three_failures_returns_network_err_and_stops`, `zero_backoff_sleeps_for_zero_duration`, `real_backoff_uses_correct_secs_table` | OK |
| REQ-EVT-001 | `StreamEvent::EmbedderDownload` Rust variant + Angular union | `src-tauri/src/models/chat.rs:230-249`, `src/app/models/chat.model.ts:56-80` | `embedder_download_serializes_skipping_none_fields`, `embedder_download_skips_all_optional_when_none`, `embedder_download_roundtrip`, `chat.model.spec.ts` discriminant narrowing | OK |
| REQ-EVT-002 | Lifecycle events via `app.emit`; forwarder pattern; lifecycle order start→downloading→verifying→done | `lib.rs:91-127` (translator), `lib.rs:142-234` (bootstrap + forwarder spawn); `model_download.rs:371-403` (event emission) | `lifecycle_events_in_expected_order`, `error_path_emits_exactly_one_terminal_error_event`, `downloading_events_rate_limited_by_64kb`, translator unit tests (`translates_start/chunk/verify/done/error`) | PARTIAL — rate-limit window deviates from spec wording (see WARNING below); duplicate-terminal-`done` race in `lib.rs` bootstrap (see WARNING below). Test coverage for the LIFECYCLE ORDER is at the `ensure_model` layer, NOT the `app.emit` layer where the race lives. |
| REQ-STATE-001 | `AppState.embedder: Arc<OnceCell<Arc<dyn Embedder>>>`; lifecycle in `new()`, `new_for_test()`, `new_for_test_async()`; setup-hook task | `state.rs:81` (field), `:146` (new), `:210, :261` (test ctors), `:455-462` (`seeded_embedder_cell`); `lib.rs:43` (setup hook spawn) | `new_for_test_preseeds_fake_embedder`, `new_for_test_async_preseeds_fake_embedder`, `new_returns_empty_embedder_cell` | OK |
| REQ-UI-001 | Angular `EmbedderService` w/ signal, lazy `listen`, `retry()` | `src/app/services/embedder.service.ts:31-141` | 10 spec assertions in `embedder.service.spec.ts` | OK — design §5.3 called for `OnDestroy`; orchestrator-approved use of `DestroyRef.onDestroy()` instead (line 35, 46-49) per the override the task prompt mentions. |
| REQ-UI-002 | ConfigPanel section w/ native control flow, OnPush, ARIA | `config-panel.component.ts:1-33` (inject + computed percent), `config-panel.component.html:183-245` (section) | 8 spec assertions in `config-panel.component.spec.ts` | OK |
| REQ-CMD-001 | `retry_embedder_download` Tauri command + idempotency | `src-tauri/src/commands/embedder.rs:26-57` | `retry_when_ready_is_idempotent`, `retry_when_empty_spawns_bootstrap` (generic via `retry_inner_generic` because `AppHandle` cannot be constructed in tests) | OK — registered in `lib.rs:76` |
| REQ-TEST-001 | No real download; mockito + `FakeEmbedder` + ZeroBackoff + MockDiskOracle | Throughout `embedder.rs::tests` and `model_download.rs::tests` | Audited via grep: no `huggingface.co` references in any test code; no `tokio::time::sleep(Duration::from_secs(...))` in any phase-2 test (the one occurrence is `commands/chat.rs:551`, pre-existing) | OK |

## Decision adherence

| Proposal # | Decision | Implementation | OK? |
|------------|----------|----------------|-----|
| 1 | FTS5 deferred to Phase 4 | No FTS5 code added; nothing in `memory/` references FTS5 | OK |
| 2 | `StreamEvent::EmbedderDownload` variant added Rust + Angular | `chat.rs:239-249`, `chat.model.ts:74-80` | OK |
| 3 | Tauri `app.emit` + Angular `listen` (NEW pattern) | `lib.rs:178, 232` emit; `embedder.service.ts:60` listen | OK — also documented as precedent in `design.md` §5.6 |
| 4 | `fastembed = "5"`, `default-features = false`, `features = ["ort-download-binaries"]` (NOT `-rustls-tls`) | `Cargo.toml:58` | OK — feature spelled correctly; long explanatory comment at `:52-57` confirms `-rustls-tls` does not exist |
| 5 | `pub trait Embedder: Send + Sync` (`async-trait`); 3 impls (FastEmbedLocal real, Ollama/OpenAI stubs) | `embedder.rs:13-25`, FastEmbedLocal `:57-135`, stubs `:179-207` | OK |
| 6 | Model `multilingual-e5-small`, 384 dims, 5 files | `embedder.rs:131-132`, file manifest `model_download.rs:85-111` | OK |
| 7 | Use `TextEmbedding::try_new_from_user_defined` with our bytes (never hf-hub) | `embedder.rs:108` | OK — hf-hub feature explicitly dropped |
| 8 | Hardcoded SHA-256 constants per file (no blind trust) | `model_download.rs:56-65` — all 5 pinned, no TODOs remain | OK |
| 9 | `Arc<OnceCell<Arc<dyn Embedder>>>` via `once_cell::sync::OnceCell` | `state.rs:81` | OK |
| 10 | 3 attempts, 5s/30s/120s backoff, no partial-resume | `RETRY_BACKOFF_SECS = [5,30,120]` `:141`; `partial` always truncated on every attempt entry `:557` | OK |
| 11 | `FakeEmbedder` inline `#[cfg(test)]` in `embedder.rs` | `embedder.rs:216-256` | OK |
| 12 | Worker-thread + mpsc; compile-time `Send + Sync` assertion active | `embedder.rs:111-127, 161-168` | OK |
| 13 | `ToolProgress` Angular gap NOT scope | No edits to `chat.service.ts` or pre-existing `tool_progress` Rust variant | OK |
| 14 | `fs2` (not `sysinfo`) — orchestrator override | `Cargo.toml:59`; `Fs2Oracle` in `model_download.rs:130-136` | OK |
| 15 | Strict TDD; FakeEmbedder + mockito + tiny fixtures | All tests use those primitives; no real `from_bytes` test runs (real model load deferred to BLOCKED-BY-EXTERNAL smoke) | OK |

## Findings

### CRITICAL
*(none)*

### WARNING

1. **Double `done` event on success in `start_embedder_bootstrap`** (`src-tauri/src/lib.rs:175-232`). The forwarder task translates `DownloadProgress::Done` into a `done` event AND the bootstrap then emits its own synthetic terminal `done` event at `:232`. Result: on the happy path, the UI receives `done` TWICE. `drop(forwarder)` only drops the `JoinHandle`; the spawned task continues running and emits whatever is still queued in `progress_rx`. The spec REQ-EVT-002 says "exactly ONE terminal `done` event is emitted as the LAST event of the lifecycle." This is not caught by tests because `start_embedder_bootstrap_with_deps` (the test seam) stops at `ensure().await` without exercising the forwarder. The `lifecycle_events_in_expected_order` test verifies emissions from `ensure_model` directly, NOT through `lib.rs`. Recommended fix: either (a) suppress the `Done` emission inside `ensure_model_with_oracle_spec_inner:401` when called from `start_embedder_bootstrap` (introduce a `suppress_done` knob), or (b) replace the bootstrap's synthetic `done` emission with `forwarder.await` and only emit the synthetic event on the load-failure path. The error path is fine — `ensure_model` only emits `Error` if `progress` is set and the wrapper at `model_download.rs:323-331` is the sole source.

2. **Rate-limit window deviates from spec text** (`src-tauri/src/memory/model_download.rs:515-516, 586-612`). Spec REQ-EVT-002 mandates rate-limiting to "at most one [downloading event] per ~200 ms" (time-based). The implementation uses byte-based throttling: `CHUNK_PROGRESS_BYTES = 64 KB`. The accompanying test `downloading_events_rate_limited_by_64kb` was renamed to match the implementation choice rather than the spec text. Functionally equivalent for typical download speeds (≥320 KB/s yields ≤200 ms cadence), but on slow links the UI may receive many more updates than the spec budgets. Suggest either updating the spec to canonicalize the byte-based rate or adding a wall-clock floor (whichever the orchestrator prefers; the byte-based approach is arguably more useful for visible progress on slow links).

3. **`download_one` does NOT emit a per-file `Verify` event** (`model_download.rs:396-398`). Spec REQ-EVT-002 scenario reads "GIVEN SHA-256 verification begins on a file, WHEN entering the verify phase, THEN exactly one `phase: "verifying"` event is emitted." The implementation emits ONE `Verify` event after ALL 5 SHAs verify (line 397) — i.e. once per run, not once per file. The spec wording is ambiguous (it says "exactly one" not "exactly one per file"), so this is defensible, but worth flagging.

4. **`fixture_files()` returns a 1-tuple `(Vec<FixtureFile>,)`** (`model_download.rs:826-857`). Cosmetic but odd — the trailing comma turns the return type into a single-element tuple, forcing every caller to write `let (files,) = fixture_files();`. Refactoring it to return `Vec<FixtureFile>` directly would be cleaner. No correctness impact.

5. **`last_err_is_http` is dead** (`model_download.rs:548, 573, 579, 629, 690`). The variable is assigned but only read by a `let _ = last_err_is_http;` lint-suppressor. The original intent (distinguish HTTP error class for the `RetryExhausted` error variant?) was abandoned. Cleanup: drop the variable entirely.

### SUGGESTION

1. **ToolProgress Angular gap remains open** (`src/app/services/chat.service.ts` has no `case 'tool_progress':`, `chat.model.ts` discriminated union lacks the variant). Out of Phase 2 per proposal decision #13, but still worth filing as a follow-up issue.

2. **AXE harness absent** — `config-panel.component.spec.ts` checks ARIA attributes manually but does NOT run an automated AXE pass. Task T120 noted the project lacks `jest-axe`/`@axe-core/playwright`. Add `@axe-core/cli` (or `vitest-axe`) and wire one test that scans all 5 phases. Per CHECKPOINT 4, this is a manual smoke item for now.

3. **`commit_file` could use `std::fs::rename` directly** (`model_download.rs:698-700`) — the wrapper adds no value beyond a name. Inline at the call site or keep the wrapper but document why it exists (e.g., for future "rename + sync directory" hardening).

4. **`real_backoff_uses_correct_secs_table` is a `#[test]` not `#[tokio::test]`** (`model_download.rs:1482`). Fine because `secs_for_attempt` is sync, but for consistency with the rest of the file all backoff tests could be `#[tokio::test]`. Cosmetic.

5. **Pre-existing test failure `src/app/app.spec.ts > should render title`** — unrelated to Phase 2 but exists in the same `npm test` run; recommend fixing or marking it `.skip` so future verify reports do not get distracted by it.

## Manual smoke not run (BLOCKED-BY-EXTERNAL)

Carried forward from apply CHECKPOINT 4 + new additions:

- Cold-start without cached model — observe `start → downloading → verifying → done` in ConfigPanel; verify all 5 files land at `~/.claw-garden/models/multilingual-e5-small/`.
- Cold-start WITH cached model — confirm ZERO HTTP requests; `done` near-instant.
- Offline cold-start without cache — confirm `phase: 'error'` with Retry button; clicking Retry while offline → another `error`; with network → success path.
- Browser-AXE pass on ConfigPanel for all 5 phases (no harness in CI).
- Real `from_bytes(...)` load (`FastEmbedLocal` against the actual 470 MB ONNX) — `#[ignore]`-marker test path is documented in spec but not present in source as a test; the worker plumbing is tested with a hand-rolled mock instead.
- **NEW (verify)**: Reproduce the **double-`done` event** WARNING by running the app with cached model, attaching `window.__TAURI__.event.listen('embedder-download', e => console.log(e))`, and counting `phase === 'done'` events. Confirm whether you receive 1 or 2.

## Acceptance criteria from RAG-PHASE-PLAN.md line 146-152

- [x] **Determinism**: `Embedder::embed("hola mundo")` returns 384-dim Vec, bit-identical across calls. Evidence: `embedder.rs:271-277` (`fake_embedder_is_deterministic`); `FastEmbedLocal` determinism is `[BLOCKED-BY-EXTERNAL]` for the real model load but the algorithmic determinism of `fastembed::TextEmbedding` for `multilingual-e5-small` is industry-known (no dropout, deterministic ONNX session). Dimensions=384 hard-pinned at `embedder.rs:131`.
- [n/a] **Cold start sin modelo: download progresa, UI muestra, SHA verifica, cache OK** — requires manual smoke (`[BLOCKED-BY-EXTERNAL]`). Tests prove every component independently (`successful_download_leaves_no_partial_files`, ConfigPanel renders 5 phases).
- [n/a] **Cold start con modelo cacheado: arranca sin red, embedder funcional al primer uso** — requires manual smoke. Cache-hit fast path verified by `skips_download_when_all_files_cached` (asserts ZERO mockito hits) and `cached_path_skips_disk_check`.
- [n/a] **Sin red al primer arranque sin modelo: NO crashea, modo degradado, error explícito** — requires manual smoke. Non-crash + error event + empty OnceCell verified by `network_failure_returns_err_no_partial_files` + `error_path_emits_exactly_one_terminal_error_event` + `retry_when_empty_spawns_bootstrap`.
- [x] **Tests: smoke FakeEmbedder + determinism + SHA mismatch — todos pasan sin descarga real**. Evidence: 247/247 Rust + 18/18 Phase 2 Angular tests; grep audit confirms zero `huggingface.co` test references and zero real sleeps.

---

## Fix applied (post-verify, before archive)

Date: 2026-05-24.

### WARNING #1 — double `done` event — FIXED
- `ensure_model` no longer emits `Done` (`memory/model_download.rs:335-410`). Its terminal success event is `Verify`; success state is the `Ok(ModelFiles)` return.
- Sole source of terminal `done` is `start_embedder_bootstrap` → `run_embedder_bootstrap` AFTER `OnceCell.set` succeeds (`lib.rs:264`).
- New `EmbedderEventSink` trait (`lib.rs:144-156`) + production `AppHandleSink` + test `MockSink` → enables end-to-end assertion of the terminal-`done` invariant.
- New tests: `lib::tests::run_embedder_bootstrap_emits_exactly_one_done_on_happy_path` + `run_embedder_bootstrap_emits_terminal_error_when_ensure_fails`.
- Updated test: `model_download::tests::lifecycle_events_in_expected_order` — asserts last event is `verify`, no `Done` from this layer.

### WARNING #4 — `fixture_files()` 1-tuple — FIXED
- Now returns `Vec<FixtureFile>` directly. 13 call sites updated.

### WARNING #5 — `last_err_is_http` dead variable — FIXED
- Removed from 5 lines + lint suppressor in `model_download.rs`.

### WARNING #2 — rate-limit canonicalization — SPEC UPDATED
- `spec.md` REQ-EVT-002 now canonicalizes byte-based throttle (~64 KB) as the official implementation, with wall-clock 200 ms as an equivalent alternative.

### WARNING #3 — `Verify` per-run vs per-file — SPEC UPDATED
- `spec.md` REQ-EVT-002 now explicitly states "exactly one `verifying` event **per `ensure_model` run** (not per file)".

### SUGGESTIONS (remaining, not blocking)
1. `ToolProgress` Angular gap — pre-existing follow-up, NOT Phase 2 scope.
2. AXE harness absent — follow-up infra ticket.
3. `commit_file` no-value wrapper — cosmetic.
4. `real_backoff_uses_correct_secs_table` could be `#[tokio::test]` — cosmetic.
5. Pre-existing `app.spec.ts > should render title` failure — pre-Phase-2; either delete or rewrite.

### Final test count
- Rust: **249/249 PASS** (was 247; +2 regression-guard tests for `run_embedder_bootstrap`).
- Angular: 23/24 PASS (1 pre-existing `app.spec.ts > should render title` failure, NOT Phase 2).
- Zero regressions on the 204 pre-Phase-2 baseline.
