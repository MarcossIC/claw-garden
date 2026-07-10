# Phase 2 — rag-embedder — Tasks

> Strict TDD. Each `*-impl` is preceded by its `*-test`. The apply sub-agent works through this checklist top-to-bottom, mechanically.

> **Baseline before starting**: `cargo test --lib --manifest-path src-tauri/Cargo.toml` → **204/204 PASS**. Record the exact count; verify is asked to detect zero regressions.

> **No `cargo build`** at any step (user CLAUDE.md forbids it). Verification gates: `cargo check --manifest-path src-tauri/Cargo.toml` for type-level, `cargo test --lib --manifest-path src-tauri/Cargo.toml` for behavior, `cargo add --dry-run …` for dep resolution.

> **No real model download** in any test. Mocks only: `mockito` server, `FakeEmbedder`, fixture bytes, injected `DiskOracle` + `BackoffPolicy`.

> Each test goes RED FIRST (compile-error or assertion fail). Its paired impl makes it GREEN. Do NOT collapse pairs.

> Decision lock: `fs2 = "0.4"` is the disk-space dep (proposal §Decisions row 14, design §10). NOT `sysinfo`. Apply MUST add `fs2`, NOT `sysinfo`.

---

## Phase 2.0 — Deps + module skeletons

- [ ] **T001** — Add Phase 2 dependencies to `src-tauri/Cargo.toml`.
  - File: `src-tauri/Cargo.toml`.
  - Under `[dependencies]`, after the Phase 1 RAG block, add:
    ```toml
    # RAG Phase 2 (embedder)
    fastembed = { version = "5", default-features = false, features = ["ort-download-binaries"] }
    fs2 = "0.4"
    ```
  - Under `[dev-dependencies]`, add: `mockito = "1"`.
  - `async-trait` is required at runtime — verify it is already present; if not, add `async-trait = "0.1"` to `[dependencies]`.
  - Verify: `cargo add --dry-run fastembed --no-default-features --features ort-download-binaries` resolves to `5.13.x` clean. (Earlier docs mentioned `-rustls-tls` suffix — that feature does NOT exist on fastembed v5; corrected during apply CHECKPOINT 1.)
  - Verify: `cargo add --dry-run fs2` resolves to `0.4.x` clean.
  - Verify: `cargo add --dry-run --dev mockito` resolves to `1.x` clean.
  - Verify: `cargo check --manifest-path src-tauri/Cargo.toml` succeeds. NO `cargo build`.

- [ ] **T002** — Create `src-tauri/src/memory/embedder.rs` skeleton + register module.
  - Files: `src-tauri/src/memory/embedder.rs` (NEW), `src-tauri/src/memory/mod.rs` (MODIFIED).
  - In `embedder.rs`, write the bare minimum: `use async_trait::async_trait;`, an empty `pub trait Embedder: Send + Sync {}`, and `#[derive(Debug, thiserror::Error)] pub enum EmbedError { #[error("not ready")] NotReady, #[error("worker gone")] WorkerGone, #[error("inference failed: {0}")] InferenceFailed(String), #[error("load failed: {0}")] LoadFailed(String) }`.
  - In `mod.rs`, append `pub mod embedder;`.
  - Verify: `cargo check --manifest-path src-tauri/Cargo.toml` succeeds.

- [ ] **T003** — Create `src-tauri/src/memory/model_download.rs` skeleton + register module.
  - Files: `src-tauri/src/memory/model_download.rs` (NEW), `src-tauri/src/memory/mod.rs` (MODIFIED).
  - Write empty error enum stub `pub enum DownloadError { InsufficientDisk { free_mb: u64, required_mb: u64 } }` + a stub `pub async fn ensure_model(_base_dir: &std::path::Path, _progress: Option<tokio::sync::mpsc::UnboundedSender<DownloadProgress>>) -> Result<ModelFiles, DownloadError> { unimplemented!() }` and the placeholder structs `pub struct ModelFiles {}` and `pub enum DownloadProgress {}`.
  - In `mod.rs`, append `pub mod model_download;`.
  - Verify: `cargo check --manifest-path src-tauri/Cargo.toml` succeeds.

---

## Phase 2.1 — Embedder trait + FakeEmbedder (REQ-EMB-001, REQ-EMB-002, REQ-TEST-001)

- [ ] **T010-test** — Write failing tests in `src-tauri/src/memory/embedder.rs::tests` (REQ-EMB-001):
  - `fake_embedder_returns_384_dim_vec` — `FakeEmbedder::default().embed("hola mundo").await.unwrap().len() == 384`.
  - `fake_embedder_is_deterministic` — two consecutive calls with the same input return bit-identical `Vec<f32>`.
  - `fake_embedder_different_inputs_differ` — `embed("a")` and `embed("b")` differ at ≥1 index (anti-collision smoke).
  - `fake_embedder_model_id` — `model_id() == "fake-embedder"`.
  - `fake_embedder_dimensions_is_384` — `dimensions() == 384`.
  - Verify (must FAIL): `cargo test --lib memory::embedder` — compile errors (no `FakeEmbedder` yet) or assertion fails.

- [ ] **T010-impl** — Implement `Embedder` trait + `FakeEmbedder` test double in `embedder.rs` (REQ-EMB-001).
  - Add the full `pub trait Embedder: Send + Sync { async fn embed(&self, text: &str) -> Result<Vec<f32>, EmbedError>; fn dimensions(&self) -> usize; fn model_id(&self) -> &str; }` with `#[async_trait]`.
  - Add `#[cfg(test)] pub(crate) struct FakeEmbedder { dim: usize }` + `Default` + `#[async_trait] impl Embedder for FakeEmbedder { ... }` using the siphasher-based deterministic algorithm from design §3.4.
  - Verify: `cargo test --lib memory::embedder` — 5 new tests PASS, 204 baseline PASS, 0 regressions.

- [ ] **T011-test** — Write compile-time `Send + Sync` assertion test (REQ-EMB-002).
  - In `embedder.rs`, at module level (NOT inside a fn or `#[test]`), add (commented-OUT for now to keep T010 green):
    ```rust
    // const _: fn() = || {
    //     fn _assert_send<T: Send>() {}
    //     fn _assert_sync<T: Sync>() {}
    //     _assert_send::<FastEmbedLocal>();
    //     _assert_sync::<FastEmbedLocal>();
    //     _assert_send::<std::sync::Arc<dyn Embedder>>();
    //     _assert_sync::<std::sync::Arc<dyn Embedder>>();
    // };
    ```
  - Uncomment ONLY the `Arc<dyn Embedder>` lines now (the `FastEmbedLocal` ones come alive in T021).
  - Verify (must FAIL on `FastEmbedLocal` reference if any line uncommented): `cargo check --manifest-path src-tauri/Cargo.toml` — compile error if reference exists without the type.
  - Verify (must PASS with only the `Arc<dyn Embedder>` lines uncommented): `cargo check` succeeds.

- [ ] **T011-impl** — Cement the `Arc<dyn Embedder>: Send + Sync` portion of the const-block assertion.
  - Keep the two `_assert_send::<Arc<dyn Embedder>>()` / `_assert_sync::<Arc<dyn Embedder>>()` lines active.
  - Leave the `FastEmbedLocal` lines commented with a TODO referencing T021.
  - Verify: `cargo check` succeeds. (The assertion IS the test — no `#[test]` fn needed.)

---

## Phase 2.2 — Stub embedders (REQ-EMB-003)

- [ ] **T015-test** — Write failing tests for stub embedders in `embedder.rs::tests`:
  - `ollama_stub_compiles_and_implements_trait` — `let _: Box<dyn Embedder> = Box::new(OllamaEmbedder);` (compile check).
  - `openai_stub_compiles_and_implements_trait` — `let _: Box<dyn Embedder> = Box::new(OpenAIEmbedder);` (compile check).
  - `ollama_stub_dimensions_is_384` — `OllamaEmbedder.dimensions() == 384`.
  - `openai_stub_dimensions_is_1536` — `OpenAIEmbedder.dimensions() == 1536`.
  - `ollama_stub_embed_is_unimplemented` — annotated `#[should_panic(expected = "Phase X")]`, body calls `OllamaEmbedder.embed("x").await.unwrap();`.
  - Verify (must FAIL): `cargo test --lib memory::embedder` — compile errors (no stubs yet).

- [ ] **T015-impl** — Implement `OllamaEmbedder` + `OpenAIEmbedder` non-feature-gated stubs (REQ-EMB-003).
  - In `embedder.rs`, add `pub struct OllamaEmbedder;` + `pub struct OpenAIEmbedder;` and `#[async_trait] impl Embedder for ...` for both, with `embed` calling `unimplemented!("OllamaEmbedder is a Phase X feature")` / `unimplemented!("OpenAIEmbedder is a Phase X feature")`, `dimensions()` returning `384` / `1536`, `model_id()` returning `"ollama-placeholder"` / `"openai-placeholder"`.
  - Verify: `cargo test --lib memory::embedder` — 5 new tests PASS + 5 from T010 + 204 baseline. 0 regressions.

---

## CHECKPOINT 1 — Embedder trait + FakeEmbedder + stubs

- [ ] Run `cargo test --lib --manifest-path src-tauri/Cargo.toml` → 204 + 10 new PASS, 0 regressions.
- [ ] Run `cargo check --manifest-path src-tauri/Cargo.toml` → success (the `Arc<dyn Embedder>: Send + Sync` const-block compiles).
- [ ] Confirm `FastEmbedLocal` assertion lines in `embedder.rs` are still commented with a TODO referencing T021 (they go live in §2.3).
- [ ] No `cargo build` was invoked.

---

## Phase 2.3 — FastEmbedLocal (worker-thread architecture) (REQ-EMB-002)

> The real `TextEmbedding` integration test requires a ~470 MB model and is `[BLOCKED-BY-EXTERNAL]` for CI. We test plumbing with a mock worker; the real load is covered by manual smoke at CHECKPOINT 2.

- [ ] **T020-test** — Write failing tests for the request/oneshot channel plumbing in `embedder.rs::tests` (REQ-EMB-002):
  - `embed_request_round_trip_through_channel` — using a HAND-ROLLED mock worker thread that owns a `tokio::sync::mpsc::UnboundedReceiver<EmbedRequest>` and replies with a canned `Vec<f32>` of length 384, assert the public `embed(...).await` returns that vec.
  - `embed_returns_worker_gone_when_worker_drops` — drop the worker's receiver, call `embed(...)`, assert `Err(EmbedError::WorkerGone)`.
  - Both tests use a `#[cfg(test)] impl FastEmbedLocal { pub(crate) fn from_mock_worker(...) -> Self }` test constructor — NOT `from_bytes`.
  - Verify (must FAIL): `cargo test --lib memory::embedder` — compile errors (no `FastEmbedLocal`, no `EmbedRequest`).

- [ ] **T020-impl** — Implement the `FastEmbedLocal` shell (struct + `Embedder` impl + test-only `from_mock_worker`) WITHOUT real `fastembed` loading yet.
  - Define `struct EmbedRequest { text: String, resp: oneshot::Sender<Result<Vec<f32>, EmbedError>> }` (private).
  - `pub struct FastEmbedLocal { req_tx: mpsc::UnboundedSender<EmbedRequest>, dimensions: usize, model_id: &'static str }`.
  - `#[async_trait] impl Embedder for FastEmbedLocal { ... }` per design §3.2 (`embed` sends a request and awaits the oneshot, maps `mpsc` and `oneshot` errors to `EmbedError::WorkerGone`).
  - Add `#[cfg(test)] impl FastEmbedLocal { pub(crate) fn from_mock_worker(req_tx: mpsc::UnboundedSender<EmbedRequest>) -> Self { Self { req_tx, dimensions: 384, model_id: "mock-embedder" } } }`.
  - Verify: `cargo test --lib memory::embedder::tests::embed_request_round_trip_through_channel` PASSES + `embed_returns_worker_gone_when_worker_drops` PASSES.

- [ ] **T021-test** — Activate the `FastEmbedLocal` compile-time Send + Sync assertion (REQ-EMB-002).
  - Uncomment the `_assert_send::<FastEmbedLocal>(); _assert_sync::<FastEmbedLocal>();` lines in the const-block from T011.
  - Verify (must PASS at this point because `FastEmbedLocal` holds only `Sender` + primitives): `cargo check --manifest-path src-tauri/Cargo.toml` succeeds. The assertion IS the test; no runtime test is added.

- [ ] **T021-impl** — Implement `FastEmbedLocal::from_bytes(onnx: Vec<u8>, tokenizer: TokenizerFiles) -> Result<Self, EmbedError>` per design §3.2 — spawns the real `fastembed::TextEmbedding` on a dedicated `std::thread`, owns the worker loop with `futures::executor::block_on(req_rx.recv())`.
  - Use `fastembed::{TextEmbedding, TokenizerFiles, UserDefinedEmbeddingModel, InitOptionsUserDefined}`.
  - Load synchronously on the calling thread first to surface load errors as `EmbedError::LoadFailed`.
  - Then spawn the worker thread (named `"fastembed-worker"`) that loops over `req_rx.recv()` and calls `model.embed(...)`.
  - NO test added in this step (real model load is `[BLOCKED-BY-EXTERNAL]`; the worker plumbing was already covered by T020).
  - Verify: `cargo check --manifest-path src-tauri/Cargo.toml` succeeds.
  - Verify: `cargo test --lib memory::embedder` still 10 PASS, 0 regressions.

---

## Phase 2.4 — Disk pre-flight (`fs2`) (REQ-DL-003)

- [ ] **T030-test** — Write failing tests in `model_download.rs::tests` for the `DiskOracle` injection seam (REQ-DL-003).
  - `insufficient_disk_returns_err_no_http` — inject `MockDiskOracle::returning(599 * 1024 * 1024)`, call `ensure_model_with_oracle(...)` against a `mockito::Server` that asserts `expect(0)` on ALL routes, expect `Err(DownloadError::InsufficientDisk { free_mb: 599, required_mb: 600 })`.
  - `sufficient_disk_passes_preflight` — inject `MockDiskOracle::returning(600 * 1024 * 1024)`, assert the function proceeds past the disk check (does NOT return `InsufficientDisk`).
  - `cached_path_skips_disk_check` — pre-write all 5 files with correct bytes + SHAs in a tempdir, inject `MockDiskOracle::returning(10 * 1024 * 1024)` (way under 600 MB), expect `Ok(ModelFiles)` (disk check skipped because cache hit).
  - Verify (must FAIL): `cargo test --lib memory::model_download` — compile errors (no `DiskOracle`, no `ensure_model_with_oracle`).

- [ ] **T030-impl** — Implement the `DiskOracle` trait + `Fs2Oracle` production impl + `ensure_model_with_oracle` (REQ-DL-003).
  - Add: `pub trait DiskOracle: Send + Sync { fn available_bytes(&self, path: &std::path::Path) -> std::io::Result<u64>; }`.
  - Add: `struct Fs2Oracle; impl DiskOracle for Fs2Oracle { fn available_bytes(&self, p: &Path) -> io::Result<u64> { fs2::available_space(p) } }`.
  - Add: `#[cfg(test)] pub(crate) struct MockDiskOracle { free_bytes: u64 }` + `impl MockDiskOracle { pub(crate) fn returning(b: u64) -> Self { Self { free_bytes: b } } }` + `impl DiskOracle for MockDiskOracle`.
  - Add `pub(crate) async fn ensure_model_with_oracle(base_dir: &Path, oracle: &dyn DiskOracle, base_url: &str, backoff: &dyn BackoffPolicy, progress: Option<UnboundedSender<DownloadProgress>>) -> Result<ModelFiles, DownloadError>` — body: implement the cache-hit fast path + disk pre-flight using `oracle.available_bytes(...)`, returning `InsufficientDisk` if `< 600 * 1024 * 1024`.
  - `BackoffPolicy` does not exist yet — for this task, make the parameter optional or use a stub trait that compiles; T040 wires it properly.
  - Keep `pub async fn ensure_model(base_dir: &Path, progress: Option<UnboundedSender<DownloadProgress>>) -> Result<ModelFiles, DownloadError>` as a thin wrapper that hard-codes `Fs2Oracle` + `RealBackoff` + `MODEL_BASE_URL`.
  - Verify: `cargo test --lib memory::model_download::tests::insufficient_disk_returns_err_no_http` PASS; `sufficient_disk_passes_preflight` PASS; `cached_path_skips_disk_check` PASS (the last requires SHA constants — if they aren't pinned yet, use the `MODEL_SHA256` / `TOKENIZER_JSON_SHA256` from spec REQ-DL-002 and leave the 3 small JSON constants as `&str` placeholders that the test writes matching-bytes for).

---

## Phase 2.5 — Backoff policy injection (REQ-DL-004, REQ-TEST-001)

- [ ] **T035-test** — Write failing tests for `BackoffPolicy` injection in `model_download.rs::tests`:
  - `zero_backoff_sleeps_for_zero_duration` — construct `ZeroBackoff`, assert `backoff.sleep_for_attempt(1).await` completes in `< 50 ms` wall-clock.
  - `real_backoff_uses_correct_secs_table` — `RealBackoff::default()` exposes `secs_for_attempt(i)` returning `[5, 30, 120][i]` for `i in 0..3` (introspection without sleeping).
  - Verify (must FAIL): compile errors (no `BackoffPolicy`).

- [ ] **T035-impl** — Implement `BackoffPolicy` trait + `RealBackoff` + `ZeroBackoff` (REQ-DL-004).
  - `pub trait BackoffPolicy: Send + Sync { fn sleep_for_attempt(&self, attempt: u32) -> futures::future::BoxFuture<'static, ()>; fn secs_for_attempt(&self, attempt: u32) -> u64; }`.
  - `pub struct RealBackoff;` impl: `secs_for_attempt` returns `[5, 30, 120][attempt as usize]` clamped; `sleep_for_attempt` calls `tokio::time::sleep(Duration::from_secs(...))`.
  - `#[cfg(test)] pub(crate) struct ZeroBackoff;` impl: both methods return zero / immediately-ready future.
  - Verify: `cargo test --lib memory::model_download::tests::zero_backoff_sleeps_for_zero_duration` PASS + sibling PASS.

---

## Phase 2.6 — Download module — SHA constants + cached fast path (REQ-DL-001, REQ-DL-002)

- [ ] **T040-test** — Write failing tests for the cached-fast-path + SHA constants in `model_download.rs::tests`:
  - `skips_download_when_all_files_cached` — pre-write the 5 expected files with the EXACT bytes whose SHAs match the constants (the test fixtures live inline as `b"..."` byte literals or computed at test time), point a `mockito::Server` at `base_url`, assert `mock.expect(0)` on every endpoint, expect `Ok(ModelFiles { onnx, tokenizer })`.
  - `corrupt_cached_file_triggers_redownload` — pre-write 5 files but tamper byte 0 of `onnx/model.onnx`, set up `mockito` with a valid response for `onnx/model.onnx`, assert exactly 1 hit for that file and 0 for the others, expect `Ok`.
  - For the 3 small JSON files whose SHAs are `TODO_APPLY_PHASE_COMPUTE_SHA256_*` (design §4.2), pick stable fixture bytes (e.g. `br#"{"model_type":"bert"}"#`), compute their SHAs once in the apply step below, and pin them in this commit.
  - Verify (must FAIL): compile errors or wrong cached-detection logic.

- [ ] **T040-impl** — Implement SHA-256 constants + cached fast path + atomic file commit in `model_download.rs` (REQ-DL-001, REQ-DL-002).
  - Add module-level constants per design §4.2: `MODEL_SHA256`, `TOKENIZER_JSON_SHA256`, `CONFIG_JSON_SHA256`, `SPECIAL_TOKENS_MAP_SHA256`, `TOKENIZER_CONFIG_SHA256`, `MIN_FREE_BYTES`, `MODEL_BASE_URL`, `ALL_FILES: [FileSpec; 5]`.
  - For the 3 TODO constants, use the fixture bytes from T040-test and pin the SHA-256 computed via `sha2::Sha256::digest`. Add a comment: `// SHA computed locally from fixture bytes (REQ-DL-002); update when production constants are pinned.`
  - Implement the cache-hit fast path in `ensure_model_with_oracle`: for each `FileSpec`, if `cache_dir.join(spec.path).exists()` AND `sha256(read(path)) == spec.sha256`, collect bytes; if all 5 succeed → return `Ok(ModelFiles { onnx, tokenizer })` WITHOUT emitting any event and WITHOUT calling `oracle.available_bytes`.
  - Implement the atomic `.partial` → final rename helper `fn commit_file(partial: &Path, final_path: &Path) -> io::Result<()>` (used by §2.7).
  - Verify: `cargo test --lib memory::model_download::tests::skips_download_when_all_files_cached` PASS + `corrupt_cached_file_triggers_redownload` PASS + `cached_path_skips_disk_check` (from T030) still PASS.

---

## Phase 2.7 — Download module — HTTP + retry + SHA verify (REQ-DL-001, REQ-DL-002, REQ-DL-004)

- [ ] **T045-test** — Write failing tests for the per-file download + retry path in `model_download.rs::tests`:
  - `successful_download_leaves_no_partial_files` — `mockito` serves valid bytes for all 5 files; after `ensure_model_with_oracle(...)`, walk the cache dir and assert ZERO `.partial` files remain.
  - `network_failure_returns_err_no_partial_files` — `mockito` returns connection-refused (use `.with_status(503)`) for all attempts; assert `Err(DownloadError::RetryExhausted { file, attempts: 3 })` and ZERO `.partial` files remain.
  - `retry_succeeds_on_second_attempt` — `mockito` returns HTTP 500 once, then valid bytes; with `ZeroBackoff` injected, expect `Ok(ModelFiles)` and exactly 2 hits on the failing file's mock.
  - `three_failures_returns_network_err_and_stops` — `mockito.expect(3)` returns 500; assert `Err(DownloadError::RetryExhausted { attempts: 3 })`, mock recorded EXACTLY 3 hits.
  - `sha_mismatch_after_retries_returns_checksum_err` — `mockito` serves bytes that produce a wrong SHA on every attempt; expect `Err(DownloadError::ChecksumMismatch { file, expected, actual })` AND `.partial` is deleted. (Per design §4.4 step 6, SHA mismatch returns IMMEDIATELY without consuming the retry budget — confirm exactly 1 hit.)
  - `sha_mismatch_deletes_partial_and_retries` — slight variant: if applying §4.4 step 6 literally (no retry on mismatch), this test should assert SHA mismatch returns ChecksumMismatch after 1 attempt with `.partial` deleted. Otherwise (if apply chooses to retry per REQ-DL-002 scenario 2), assert 3 attempts and ChecksumMismatch. PICK ONE shape consistently; document the choice in a code comment with a REQ link.
  - `downloads_only_missing_files` — pre-write 4 of 5 files with correct SHAs, mockito serves the 5th; assert exactly 1 hit (on the missing file) and 0 hits on the other 4.
  - Verify (must FAIL): compile errors or wrong retry counts.

- [ ] **T045-impl** — Implement the per-file `download_one(spec, cache_dir, base_url, backoff, progress) -> Result<(), DownloadError>` helper per design §4.4 + integrate into `ensure_model_with_oracle`.
  - Use `reqwest::Client::new().get(format!("{base_url}/{}", spec.path)).send().await` + `resp.bytes_stream()` for chunked progress.
  - Write each chunk to `<spec.path>.partial`, throttle `DownloadProgress::Chunk` emissions to one per 64 KB.
  - After full stream: `flush + sync_all`, compute SHA-256, on mismatch → delete `.partial` and return immediately with `ChecksumMismatch` (per design §4.4 step 6), OR if T045-test chose the "consume budget" shape, continue the retry loop.
  - Atomic rename `.partial` → final on success.
  - Retry loop iterates `0..3`; calls `backoff.sleep_for_attempt(i)` BEFORE attempt `i` only when `i > 0`.
  - After loop exhausted → `Err(DownloadError::RetryExhausted { file: spec.path.into(), attempts: 3 })`.
  - Verify: all T045-test tests PASS, 0 regressions.

---

## Phase 2.8 — Download module — Progress events (REQ-EVT-002)

- [ ] **T050-test** — Write failing tests for the `DownloadProgress` event sequence in `model_download.rs::tests` (REQ-EVT-002):
  - `lifecycle_events_in_expected_order` — successful path: collect emitted `DownloadProgress` events into a `Vec`, assert sequence matches: `Start { bytes_total: <sum> }`, ≥1 `Chunk { ... }`, `Verify`, `Done` (in that order).
  - `error_path_emits_exactly_one_terminal_error_event` — force `RetryExhausted`, assert the last event is `DownloadProgress::Error { message: <non-empty> }` AND no events follow.
  - `downloading_events_rate_limited_by_64kb` — serve a known-size response (say 256 KB), assert at most `ceil(256/64) + 1 = 5` `Chunk` events emitted (lenient ±1 to absorb implementation detail).
  - Verify (must FAIL): no event-emission code yet.

- [ ] **T050-impl** — Wire `DownloadProgress::{Start, Chunk, Verify, Done, Error}` emission throughout `ensure_model_with_oracle` + `download_one` per design §4.3 / §4.4.
  - Emit `Start { bytes_total: sum_of_HEAD_lengths_or_sum_of_known_sizes }` once after the disk pre-flight succeeds.
  - In `download_one`: after every 64 KB written, emit `Chunk { bytes_downloaded: <running_total_across_all_files>, bytes_total }`.
  - After all 5 files committed: emit `Verify` once, then `Done` once.
  - On terminal error: emit `Error { message: <e.to_string()> }` once and return.
  - Verify: T050-test PASS, 0 regressions.

---

## CHECKPOINT 2 — Backend download + embedder working end-to-end (no Angular yet)

> Natural pause per proposal lines 161-162: validate download + embedder before touching Angular.

- [ ] `cargo test --lib --manifest-path src-tauri/Cargo.toml` → 204 + ~20 new PASS, 0 regressions.
- [ ] `cargo check --manifest-path src-tauri/Cargo.toml` → success.
- [ ] Grep test files: `tests/` and `src-tauri/src/memory/{embedder,model_download}.rs::tests` MUST contain ZERO references to `"huggingface.co"` and ZERO uses of `tokio::time::sleep(Duration::from_secs` (only `ZeroBackoff`).
- [ ] **`[BLOCKED-BY-EXTERNAL]`** Real model end-to-end smoke (manual, deferred to CHECKPOINT 3): the actual ~470 MB download + `TextEmbedding` load cannot run in CI. Skip here; manually verified at CHECKPOINT 3 after the setup hook is wired.

---

## Phase 2.9 — `StreamEvent::EmbedderDownload` variant (REQ-EVT-001)

- [x] **T060-test** — Write failing tests in `src-tauri/src/models/chat.rs::tests` (extend existing test module) for the new variant:
  - `embedder_download_serializes_skipping_none_fields` — construct `StreamEvent::EmbedderDownload { phase: "downloading".into(), bytes_downloaded: Some(1024), bytes_total: Some(2048), error: None }`, serialize via `serde_json::to_string`, assert JSON contains `"kind":"embedder_download"`, `"phase":"downloading"`, `"bytes_downloaded":1024`, `"bytes_total":2048`, AND does NOT contain `"error"`.
  - `embedder_download_skips_all_optional_when_none` — `phase: "verifying"`, all numerics `None`; assert JSON has only `kind` + `phase`.
  - `embedder_download_roundtrip` — serialize then deserialize the `phase: "error", error: Some("boom")` variant, assert equality.
  - Verify (must FAIL): compile error — no such variant.

- [x] **T060-impl** — Add the `EmbedderDownload` variant to `enum StreamEvent` in `src-tauri/src/models/chat.rs` (REQ-EVT-001).
  - Per design §5.4 / spec REQ-EVT-001, exactly: `#[serde(rename = "embedder_download")] EmbedderDownload { phase: String, #[serde(skip_serializing_if = "Option::is_none")] bytes_downloaded: Option<u64>, #[serde(skip_serializing_if = "Option::is_none")] bytes_total: Option<u64>, #[serde(skip_serializing_if = "Option::is_none")] error: Option<String> }`.
  - Verify: T060-test PASS, 0 regressions across the chat module tests.

---

## Phase 2.10 — `AppState.embedder` field + test pre-seeding (REQ-STATE-001)

- [x] **T070-test** — Write failing tests in `src-tauri/src/state.rs::tests` (extend existing) for the new field (REQ-STATE-001):
  - `new_returns_empty_embedder_cell` — `AppState::new().embedder.get().is_none()`.
  - `new_for_test_preseeds_fake_embedder` — `AppState::new_for_test().embedder.get().unwrap().model_id() == "fake-embedder"`.
  - `new_for_test_async_preseeds_fake_embedder` — `AppState::new_for_test_async().await.embedder.get().unwrap().embed("x").await.unwrap().len() == 384`.
  - Verify (must FAIL): compile error — no `embedder` field.

- [x] **T070-impl** — Add `pub embedder: Arc<once_cell::sync::OnceCell<Arc<dyn Embedder>>>` field to `AppState` + initialize in all 3 constructors (REQ-STATE-001).
  - Imports: `use once_cell::sync::OnceCell;` + `use crate::memory::embedder::Embedder;`.
  - `new()`: `embedder: Arc::new(OnceCell::new())`.
  - `new_for_test()`: build the cell, `cell.set(Arc::new(FakeEmbedder::default()) as Arc<dyn Embedder>).ok();`, then `embedder: Arc::new(cell)`.
  - `new_for_test_async()`: same as `new_for_test()`.
  - Verify: T070-test PASS, 0 regressions on the 204 baseline.

---

## Phase 2.11 — `progress_to_stream_event` translator (REQ-EVT-002)

- [x] **T075-test** — Write failing tests in `src-tauri/src/lib.rs::tests` (or a co-located test module) for the translator:
  - `translates_start` — `progress_to_stream_event(DownloadProgress::Start { bytes_total: 100 })` returns `StreamEvent::EmbedderDownload { phase: "start", bytes_downloaded: Some(0), bytes_total: Some(100), error: None }`.
  - `translates_chunk_downloading` — `DownloadProgress::Chunk { bytes_downloaded: 50, bytes_total: 100 }` → `phase: "downloading", bytes_downloaded: Some(50), bytes_total: Some(100), error: None`.
  - `translates_verify` — `phase: "verifying"`, all optionals `None`.
  - `translates_done` — `phase: "done"`, all optionals `None`.
  - `translates_error` — `DownloadProgress::Error { message: "boom" }` → `phase: "error", error: Some("boom")`.
  - Verify (must FAIL): no such fn.

- [x] **T075-impl** — Implement `pub(crate) fn progress_to_stream_event(p: DownloadProgress) -> StreamEvent` in `src-tauri/src/lib.rs` per design §5.1.
  - Verify: T075-test PASS, 0 regressions.

---

## Phase 2.12 — Tauri command `retry_embedder_download` (REQ-CMD-001)

- [x] **T080-test** — Write failing tests in `src-tauri/src/commands/embedder.rs::tests` (new file's test module) (REQ-CMD-001):
  - `retry_when_ready_is_idempotent` — pre-seed `AppState.embedder` cell with `FakeEmbedder`, call a `pub(crate) async fn retry_inner(state: Arc<AppState>) -> Result<(), String>` (test seam — NOT the `#[tauri::command]` wrapper, which is hard to invoke without a Tauri runtime), expect `Ok(())` AND no bootstrap spawned (assert via a `BootstrapSpawned` `AtomicBool` flag injected into the call).
  - `retry_when_empty_spawns_bootstrap` — empty cell, call `retry_inner(...)`, assert the spawn flag flips to `true`.
  - Verify (must FAIL): no such file.

- [x] **T080-impl** — Create `src-tauri/src/commands/embedder.rs` with the command + register it (REQ-CMD-001).
  - File: `src-tauri/src/commands/embedder.rs` (NEW), `src-tauri/src/commands/mod.rs` (MODIFIED — append `pub mod embedder;`).
  - Add `#[tauri::command] pub async fn retry_embedder_download(app: AppHandle, state: State<'_, AppState>) -> Result<(), String>` per design §5.2.
  - Extract a `pub(crate) async fn retry_inner(...)` that the test can call without `AppHandle` (use a trait-bound `BootstrapSpawner` or an `Arc<dyn Fn() + Send + Sync>` injectable).
  - Verify: T080-test PASS, 0 regressions.
  - Verify: `cargo check` succeeds.

---

## Phase 2.13 — `start_embedder_bootstrap` + setup-hook registration (REQ-STATE-001, REQ-EVT-002)

- [x] **T085-test** — Write a failing test in `src-tauri/src/lib.rs::tests` for the bootstrap factoring (REQ-STATE-001):
  - `start_embedder_bootstrap_is_noop_when_cell_filled` — pre-seed `AppState.embedder` with `FakeEmbedder`, call `start_embedder_bootstrap_with_deps(...)` (a test seam taking an injected `Fn(&Path, Option<UnboundedSender<DownloadProgress>>) -> BoxFuture<Result<ModelFiles, DownloadError>>` instead of `ensure_model`), assert the injected fn was NOT called.
  - `start_embedder_bootstrap_calls_ensure_model_when_cell_empty` — empty cell, inject a fake `ensure_model` that returns `Ok(test_ModelFiles)`, assert the injected fn WAS called exactly once.
  - Verify (must FAIL): no such pub fn.

- [x] **T085-impl** — Implement `pub(crate) fn start_embedder_bootstrap(app_handle: tauri::AppHandle)` + the testable seam `start_embedder_bootstrap_with_deps(...)` per design §5.1.
  - Spawn via `tauri::async_runtime::spawn(async move { ... })`. Inside:
    - If `state.embedder.get().is_some()` → return early.
    - Build `(progress_tx, progress_rx)` via `tokio::sync::mpsc::unbounded_channel`.
    - Spawn a forwarder task that drains `progress_rx`, calls `progress_to_stream_event`, calls `app_handle.emit("embedder-download", &event)`.
    - Await `ensure_model(&base_dir, Some(progress_tx)).await`.
    - On `Ok(files)` → `tokio::task::spawn_blocking(move || FastEmbedLocal::from_bytes(files.onnx, files.tokenizer)).await`, then `embedder_cell.set(Arc::new(local) as Arc<dyn Embedder>).ok()`, emit final `done` event.
    - On `Err(e)` → emit final `error` event with `e.to_string()`.
  - Register in `run()`'s `.setup(|app| { ... start_embedder_bootstrap(app.handle().clone()); Ok(()) })` AND register `commands::embedder::retry_embedder_download` in the `.invoke_handler(tauri::generate_handler![..])` macro.
  - Verify: T085-test PASS, 0 regressions.
  - Verify: `cargo check --manifest-path src-tauri/Cargo.toml` succeeds.

---

## CHECKPOINT 3 — Backend complete

- [ ] `cargo test --lib --manifest-path src-tauri/Cargo.toml` → 204 + ~28 new PASS, 0 regressions.
- [ ] `cargo check --manifest-path src-tauri/Cargo.toml` → success.
- [ ] **`[BLOCKED-BY-EXTERNAL]`** Manual smoke (optional in CI; required for human verify): run `cargo tauri dev` (or your dev script), open browser devtools, register a temporary `window.__TAURI__.event.listen('embedder-download', console.log)` listener. Confirm at least the `start` event fires within ~5 s of app boot. Full download takes minutes — observing the first 1–2 `downloading` events is sufficient; do not wait for `done`.
- [ ] If a model is already cached at `~/.claw-garden/models/multilingual-e5-small/` from a prior run, confirm boot emits ZERO HTTP requests (use a network sniffer or `netstat`) and reaches `done` near-instantly.

---

## Phase 2.14 — Angular StreamEvent union extension (REQ-EVT-001)

- [x] **T100-test** — Write failing tests in `src/app/models/chat.model.spec.ts` (or co-located if the project uses a single chat-model test file — create one if absent):
  - `embedder_download_discriminant_narrows_correctly` — given `const e: StreamEvent = { kind: 'embedder_download', phase: 'downloading', bytes_downloaded: 50, bytes_total: 100 };`, after `if (e.kind === 'embedder_download') { ... }`, TypeScript must allow access to `e.phase`, `e.bytes_downloaded`, `e.bytes_total`, `e.error`.
  - This is a compile-time test; assert via a `// @ts-expect-error` block that accessing those fields on OTHER discriminants is rejected.
  - Verify (must FAIL): compile error — no such discriminant.

- [x] **T100-impl** — Extend the `StreamEvent` discriminated union in `src/app/models/chat.model.ts` per spec REQ-EVT-001.
  - Add: `| { kind: 'embedder_download'; phase: 'start' | 'downloading' | 'verifying' | 'done' | 'error'; bytes_downloaded?: number; bytes_total?: number; error?: string; }`.
  - Verify: T100-test PASS; `tsc --noEmit` (or `ng build --configuration=development` if `tsc` is not standalone-runnable in this workspace — but PREFER `tsc --noEmit` to avoid bundling) succeeds.
  - Do NOT touch the pre-existing `tool_progress` gap (out of Phase 2 scope per proposal).

---

## Phase 2.15 — Angular `EmbedderService` (REQ-UI-001)

- [x] **T110-test** — Write failing tests in `src/app/services/embedder.service.spec.ts` (NEW) using `TestBed`:
  - `initial_status_is_idle` — after `TestBed.inject(EmbedderService)`, `service.status().phase === 'idle'`.
  - `updates_signal_on_start_event` — manually call a test-only seam `service.__test_handleEvent({ kind: 'embedder_download', phase: 'start' })`, assert `status() === { phase: 'start' }`.
  - `updates_signal_on_downloading_event` — handle `{ kind: 'embedder_download', phase: 'downloading', bytes_downloaded: 100, bytes_total: 200 }`, assert `status() === { phase: 'downloading', bytesDownloaded: 100, bytesTotal: 200 }`.
  - `updates_signal_on_done_event` — assert `status() === { phase: 'done' }`.
  - `updates_signal_on_error_event` — assert `status() === { phase: 'error', error: 'boom' }`.
  - `retry_invokes_tauri_command_and_resets_status` — mock the Tauri `invoke` via a swappable function in the service; after `await service.retry()`, assert mock called with `'retry_embedder_download'` and `status() === { phase: 'start' }`.
  - `ignores_non_embedder_events` — handle `{ kind: 'delta', text: '...' }`, assert `status()` unchanged.
  - Verify (must FAIL): no such service.

- [x] **T110-impl** — Implement `src/app/services/embedder.service.ts` per design §5.3 (REQ-UI-001).
  - `@Injectable({ providedIn: 'root' })`, `readonly status = signal<EmbedderStatus>({ phase: 'idle' })`.
  - Use `inject()` for any injected deps (none currently — but follow the pattern).
  - Lazy-import `@tauri-apps/api/event::listen` in `bootstrap()` (called from constructor); only in Tauri env (`isTauriEnv()` helper).
  - Expose a `pub(crate)` (TS: `/** @internal */`) test seam `__test_handleEvent(ev: StreamEvent): void` that runs the same signal-update logic as the listen callback.
  - Expose `async retry(): Promise<void>`: set `status({ phase: 'start' })`, lazy-import `invoke`, `await invoke('retry_embedder_download')`.
  - Implement `ngOnDestroy` to call the saved `unlisten()` returned by `listen(...)`.
  - Verify: T110-test PASS.

---

## Phase 2.16 — Angular ConfigPanel section + accessibility (REQ-UI-002)

- [x] **T120-test** — Write failing tests in `src/app/components/config-panel/config-panel.component.spec.ts` (extend if exists, else NEW):
  - `renders_idle_placeholder_when_phase_idle` — DOM contains the idle copy.
  - `renders_progressbar_with_aria_valuenow_when_downloading` — inject a stub `EmbedderService` whose `status()` returns `{ phase: 'downloading', bytesDownloaded: 100, bytesTotal: 200 }`; assert exactly one element with `role="progressbar"` AND `aria-valuenow="50"` (computed integer 0..100).
  - `renders_status_role_with_aria_live_when_verifying` — DOM contains `role="status"` AND `aria-live="polite"`.
  - `renders_alert_role_and_retry_button_when_error` — DOM contains `role="alert"` containing the error message AND a button whose accessible name matches `/retry/i`.
  - `renders_done_message_when_phase_done` — DOM contains the "Embedder ready: multilingual-e5-small" copy.
  - `clicking_retry_calls_service_retry` — install spy on `embedderService.retry`, click the retry button, assert called once.
  - `axe_clean_across_all_phases` — for each of the 5 phases, render the component, run `axe-core` (via `@axe-core/playwright` or `jest-axe` — apply picks per project test runner), assert ZERO violations. If the project lacks an AXE harness, file an inline TODO and run `axe-core` once manually via browser devtools at CHECKPOINT 4.
  - Verify (must FAIL): no embedder section in the template.

- [x] **T120-impl** — Add the Embedder section to `ConfigPanelComponent` per design §5.5 (REQ-UI-002).
  - File `config-panel.component.ts` (MODIFIED): add `readonly embedderService = inject(EmbedderService);` (no new state). Keep `changeDetection: ChangeDetectionStrategy.OnPush`.
  - File `config-panel.component.html` (MODIFIED): append the `<section class="config-section">` block from design §5.5 verbatim, using `@switch` / `@case` native control flow (NOT `*ngSwitch`), `class` bindings (NOT `ngClass`), `style` bindings (NOT `ngStyle`).
  - Wire `aria-valuenow="<computed 0..100 integer>"` via a `[attr.aria-valuenow]` binding. Since arrow fns are not allowed in templates (project CLAUDE.md), expose a `protected progressPercent = computed(() => { const s = this.embedderService.status(); if (s.phase !== 'downloading' || !s.bytesTotal) return 0; return Math.round((s.bytesDownloaded ?? 0) * 100 / s.bytesTotal); });` in the TS and bind `[attr.aria-valuenow]="progressPercent()"`.
  - Verify: T120-test PASS (AXE step may be SKIPped with a TODO if no harness — record under CHECKPOINT 4).
  - Verify: no Angular template arrow functions added.

---

## CHECKPOINT 4 — Full Phase 2 verification

- [ ] `cargo test --lib --manifest-path src-tauri/Cargo.toml` → 204 + ~28 new PASS, 0 regressions.
- [ ] `cargo check --manifest-path src-tauri/Cargo.toml` → success.
- [ ] Angular tests (project test runner) → ConfigPanel + EmbedderService specs PASS, 0 regressions on existing specs.
- [ ] **`[BLOCKED-BY-EXTERNAL]`** Cold-start manual smoke without cached model: rename `~/.claw-garden/models/multilingual-e5-small/` to `…-backup`, launch app, observe `start → downloading → verifying → done` in the ConfigPanel Embedder section. Confirm the model lands on disk with all 5 files.
- [ ] Cold-start manual smoke WITH cached model: launch app, confirm ZERO HTTP requests (verify via `netstat` or devtools network tab) and `done` reached near-instantly.
- [ ] Offline manual smoke without cached model: disable network, launch app, confirm UI reaches `phase: 'error'` (after the retry budget) with a Retry button. Click Retry while still offline → another `error`. Re-enable network → click Retry → success path completes.
- [ ] AXE checks on ConfigPanel pass for all 5 phases (use browser devtools → Lighthouse → Accessibility, or `axe-core` browser extension if no Jest harness landed).
- [ ] Hand off to `sdd-verify`.

---

## Out of scope (do NOT do here)

- Real `OllamaEmbedder` / `OpenAIEmbedder` HTTP impls (Phase X).
- FTS5 fallback when embedder unavailable (Phase 4).
- Hot-swap of the active embedder (Phase X — would require `arc_swap::ArcSwap` in place of `OnceCell`).
- Pre-existing `ToolProgress` Angular gap (separate follow-up; documented in proposal §Follow-ups).
- Any edit to `commands/chat.rs::push_and_persist` or the chat hot path — that's Phase 3 (`rag-ingest`).
- Real ~470 MB model download in any test (`[BLOCKED-BY-EXTERNAL]` items only).
- Any `cargo build` invocation (user CLAUDE.md forbids it).
- Refactoring `FakeEmbedder` to be reused cross-file (Phase 3 will move it behind `#[cfg(any(test, feature = "test-helpers"))]`).
