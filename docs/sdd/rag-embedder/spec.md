# Phase 2 — rag-embedder — Spec

> Contract derived from `docs/sdd/rag-embedder/proposal.md`. Every requirement below is normative. RFC 2119 keywords (MUST / SHOULD / MAY) apply. Each requirement maps to ≥1 scenario in GIVEN/WHEN/THEN form and to ≥1 failing test (red) that apply phase makes green.

## Scope summary

Defines the observable behavior of the Phase 2 embedder slice: a `Send + Sync` async `Embedder` trait, its `FastEmbedLocal` concrete impl, the first-run model download with SHA-256 verification and disk-space pre-flight, the Tauri-emitted progress event flow, the `AppState.embedder` lifecycle, and the Angular `EmbedderService` + ConfigPanel UI. Strict TDD: each requirement names the test that proves it.

---

## Requirements

### REQ-EMB-001 — `Embedder` trait shape

**The system MUST** expose, from `src-tauri/src/memory/embedder.rs`, a public `trait Embedder: Send + Sync` (async via `async-trait`) with exactly these methods:

- `async fn embed(&self, text: &str) -> Result<Vec<f32>, EmbedError>` — returns a vector whose length equals `self.dimensions()`.
- `fn dimensions(&self) -> usize` — returns the embedding dimensionality.
- `fn model_id(&self) -> &str` — returns a stable identifier for the active model (e.g., `"multilingual-e5-small"`).

**The system MUST** expose a public `enum EmbedError` covering at minimum: `NotReady`, `Inference(String)`, `WorkerGone`.

#### Scenarios

- GIVEN a healthy `FastEmbedLocal` instance, WHEN `embed("hola mundo")` is awaited twice consecutively, THEN both calls return `Ok(Vec<f32>)` of length 384 that are bit-for-bit identical (determinism).
- GIVEN a `FakeEmbedder` test double, WHEN `embed("anything")` is awaited, THEN it returns `Ok(Vec<f32>)` of length 384 derived deterministically from a hash of the input.
- GIVEN any `Embedder` impl in this phase, WHEN `dimensions()` is called, THEN it returns `384`.
- GIVEN a `FastEmbedLocal` instance, WHEN `model_id()` is called, THEN it returns `"multilingual-e5-small"`.

#### Red tests
- `embedder::tests::fake_embedder_returns_384_dim_vec`
- `embedder::tests::fake_embedder_is_deterministic`
- `embedder::tests::fastembed_local_dimensions_is_384` (uses tiny ONNX fixture OR `#[ignore]` marker)

---

### REQ-EMB-002 — `Send + Sync` guarantee for `FastEmbedLocal`

**The publicly exposed `FastEmbedLocal` struct MUST be `Send + Sync`** even though the inner `fastembed::TextEmbedding` is not guaranteed to be. The struct MUST own a single worker thread to which embed requests are dispatched via a `tokio::sync::mpsc` channel, with responses returned via `tokio::sync::oneshot`.

**A compile-time assertion MUST exist** at the bottom of `embedder.rs`, as straight-line code (NOT inside a `#[test]` fn), of the form:

```rust
fn _assert_send_sync<T: Send + Sync>() {}
const _: () = { let _ = _assert_send_sync::<FastEmbedLocal>; };
```

(or an equivalent expression that fails compilation if the bounds regress).

#### Scenarios

- GIVEN the source tree at the end of apply, WHEN `cargo check --lib --manifest-path src-tauri/Cargo.toml` runs, THEN compilation succeeds (the `_assert_send_sync::<FastEmbedLocal>` call type-checks).
- GIVEN a `FastEmbedLocal` instance moved into a `tokio::spawn(async move { ... })` task, WHEN that task awaits `embed(...)`, THEN it compiles and runs without `!Send` borrow errors.

#### Red tests
- Compile-time only: `embedder.rs` MUST fail to compile if `FastEmbedLocal: !Send` or `!Sync`. No runtime test needed (the assertion IS the test).

---

### REQ-EMB-003 — Stub embedders compile, panic on use

**The system MUST** expose public structs `OllamaEmbedder` and `OpenAIEmbedder` in `memory/embedder.rs` that implement `Embedder` but whose `embed`, `dimensions`, and `model_id` method bodies call `unimplemented!()` (or return `Err(EmbedError::NotReady)` for `embed`, with `unimplemented!()` for the others — apply phase picks one shape consistently).

These stubs MUST NOT be feature-gated. Future phases pattern-match on `Box<dyn Embedder>` without `#[cfg]` branches.

#### Scenarios

- GIVEN the codebase compiles, WHEN you reference `OllamaEmbedder` or `OpenAIEmbedder` from anywhere in `src-tauri/`, THEN the types resolve without enabling any feature flag.
- GIVEN an `OllamaEmbedder` instance, WHEN `embed("x").await` is called in a test wrapped with `std::panic::catch_unwind` (or `should_panic`), THEN the call panics (or returns `Err(EmbedError::NotReady)` if apply picks that shape).

#### Red tests
- `embedder::tests::ollama_stub_compiles_and_implements_trait`
- `embedder::tests::openai_stub_compiles_and_implements_trait`
- `embedder::tests::ollama_stub_embed_is_unimplemented` (`#[should_panic]` or `assert!(matches!(err, EmbedError::NotReady))`)

---

### REQ-DL-001 — Download lifecycle (5 files, atomic)

**The system MUST** provide `pub async fn ensure_model(base_dir: &Path, progress: Option<UnboundedSender<DownloadProgress>>) -> Result<ModelFiles, DownloadError>` in `src-tauri/src/memory/model_download.rs` that:

1. Computes the target directory `<base_dir>/models/multilingual-e5-small/`.
2. Downloads, on first run, exactly these 5 files: `onnx/model.onnx`, `tokenizer.json`, `config.json`, `special_tokens_map.json`, `tokenizer_config.json`.
3. Writes each file FIRST to `<name>.partial`, verifies SHA-256, THEN atomically renames to its final name. A partial file MUST NOT be left in the final location.
4. Returns `ModelFiles { onnx: Vec<u8>, tokenizer: TokenizerFiles }` on success.

#### Scenarios

- GIVEN a clean cache directory (no files), an unsigned mock HF server serving valid bytes with matching SHA-256 constants, WHEN `ensure_model` runs, THEN all 5 files land in `<base_dir>/models/multilingual-e5-small/` (no `.partial` files remain) and `Ok(ModelFiles)` is returned.
- GIVEN all 5 files already present in the cache with matching SHA-256, WHEN `ensure_model` runs against a `mockito` server, THEN ZERO HTTP requests are issued to the mock and `Ok(ModelFiles)` is returned.
- GIVEN 4 of 5 files cached with matching SHAs and 1 missing, WHEN `ensure_model` runs, THEN only the missing file is downloaded; the other 4 are not re-requested.
- GIVEN no network (mock server returns connection errors on every call), no cached files, WHEN `ensure_model` runs, THEN after the retry budget exhausts (REQ-DL-004), the function returns `Err(DownloadError::Network(_))`, NO `.partial` files remain, and the function does NOT panic.

#### Red tests
- `model_download::tests::skips_download_when_all_files_cached`
- `model_download::tests::downloads_only_missing_files`
- `model_download::tests::network_failure_returns_err_no_partial_files`
- `model_download::tests::successful_download_leaves_no_partial_files`

---

### REQ-DL-002 — SHA-256 verification per file

**Every downloaded file** MUST be SHA-256-verified against a hardcoded constant in `memory/model_download.rs` BEFORE it is atomically renamed into the cache. Verification failures MUST cause the `.partial` file to be deleted and the download to be retried within the retry budget (REQ-DL-004). The constants are:

| Constant | File | Source |
|---|---|---|
| `MODEL_SHA256 = "ca456c06b3a9505ddfd9131408916dd79290368331e7d76bb621f1cba6bc8665"` | `onnx/model.onnx` | LFS oid + X-Linked-ETag (proposal §SHA-256-source) |
| `TOKENIZER_JSON_SHA256 = "0b44a9d7b51c3c62626640cda0e2c2f70fdacdc25bbbd68038369d14ebdf4c39"` | `tokenizer.json` | LFS oid (proposal §SHA-256-source) |
| `CONFIG_JSON_SHA256` | `config.json` | populated by apply (one-shot ~1 KB download, computed locally) |
| `SPECIAL_TOKENS_MAP_SHA256` | `special_tokens_map.json` | populated by apply |
| `TOKENIZER_CONFIG_SHA256` | `tokenizer_config.json` | populated by apply |

#### Scenarios

- GIVEN a file downloaded fully whose computed SHA-256 mismatches the constant, WHEN verification runs, THEN the `.partial` file is deleted AND the function retries the download (subject to REQ-DL-004 budget).
- GIVEN 3 consecutive SHA-256 mismatches on the same file (because the mock server keeps serving corrupt bytes), WHEN the retry budget exhausts, THEN `ensure_model` returns `Err(DownloadError::ChecksumMismatch { file, expected, actual })`, NO file is committed, and the embedder `OnceCell` stays unset.
- GIVEN a cached file whose on-disk SHA-256 mismatches the constant (e.g., user tampered with cache), WHEN `ensure_model` runs, THEN that file is treated as missing and re-downloaded.

#### Red tests
- `model_download::tests::sha_mismatch_deletes_partial_and_retries`
- `model_download::tests::sha_mismatch_after_retries_returns_checksum_err`
- `model_download::tests::corrupt_cached_file_triggers_redownload`

---

### REQ-DL-003 — Disk-space pre-flight (≥600 MB)

**Before** the first HTTP request is issued, `ensure_model` MUST verify the disk holding `<base_dir>` has ≥600 MB free via the `sysinfo` crate. If insufficient, `ensure_model` MUST:

1. Emit exactly one `DownloadProgress` event that the forwarder translates to `StreamEvent::EmbedderDownload { phase: "error", error: "Insufficient disk space (<XXX> MB free, 600 MB required)" }` (where `<XXX>` is the actual free-MB count).
2. Return `Err(DownloadError::InsufficientDisk { free_mb, required_mb: 600 })`.
3. Issue ZERO HTTP requests.

This check MUST be skipped (or trivially succeed) when ALL 5 files are already cached with valid SHAs (REQ-DL-001 case 2): the check exists to gate downloads, not cached startup.

#### Scenarios

- GIVEN a `DiskSpaceProbe` injected to report 599 MB free, WHEN `ensure_model` runs against a `mockito` server (no cached files), THEN the function returns `Err(DownloadError::InsufficientDisk { free_mb: 599, required_mb: 600 })` AND the mock server records zero hits.
- GIVEN a `DiskSpaceProbe` reporting 600 MB free, WHEN `ensure_model` runs, THEN the pre-flight passes and the download proceeds.
- GIVEN all 5 files cached and valid, AND a probe reporting 10 MB free, WHEN `ensure_model` runs, THEN it returns `Ok(ModelFiles)` without invoking the disk-space check (or invoking it and ignoring the result).

#### Red tests
- `model_download::tests::insufficient_disk_returns_err_no_http`
- `model_download::tests::sufficient_disk_proceeds_to_download`
- `model_download::tests::cached_path_skips_disk_check`

---

### REQ-DL-004 — Retry policy (3 attempts, 5s/30s/120s backoff)

**`ensure_model` MUST** retry transient failures (HTTP 5xx, connection refused, timeout, SHA-256 mismatch) up to 3 attempts total per file, with exponential backoff between attempts: 5s after attempt 1, 30s after attempt 2, 120s would precede a non-existent attempt 4 (so after 3 failures the function gives up).

Backoff SHOULD be wall-clock-accurate in production but MUST be injectable for tests (e.g., a `BackoffPolicy` trait whose test impl returns `Duration::ZERO`). Tests MUST NOT sleep for real seconds.

NO partial-resume support: every retry re-downloads the full file from scratch.

#### Scenarios

- GIVEN a `mockito` server returning HTTP 500 on attempt 1 then valid bytes on attempt 2, WHEN `ensure_model` runs with a zero-backoff injected policy, THEN attempt 2 succeeds, the file is committed, and `Ok(ModelFiles)` is returned.
- GIVEN a `mockito` server returning HTTP 500 on all 3 attempts, WHEN `ensure_model` runs, THEN it returns `Err(DownloadError::Network(_))` and the mock recorded exactly 3 hits for that file.
- GIVEN a `mockito` server serving bytes that always fail SHA-256, WHEN `ensure_model` runs, THEN exactly 3 download attempts occur and the function returns `Err(DownloadError::ChecksumMismatch { .. })`.

#### Red tests
- `model_download::tests::retry_succeeds_on_second_attempt`
- `model_download::tests::three_failures_returns_network_err_and_stops`
- `model_download::tests::sha_mismatch_consumes_retry_budget`

---

### REQ-EVT-001 — `StreamEvent::EmbedderDownload` variant + serde shape

**The system MUST** add to the `enum StreamEvent` in `src-tauri/src/models/chat.rs` a new variant:

```rust
#[serde(rename = "embedder_download")]
EmbedderDownload {
    phase: String, // one of: "start" | "downloading" | "verifying" | "done" | "error"
    #[serde(skip_serializing_if = "Option::is_none")]
    bytes_downloaded: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bytes_total: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}
```

The Angular union in `src/app/models/chat.model.ts` MUST add the mirroring TypeScript discriminant:

```ts
| {
    kind: 'embedder_download';
    phase: 'start' | 'downloading' | 'verifying' | 'done' | 'error';
    bytes_downloaded?: number;
    bytes_total?: number;
    error?: string;
  }
```

#### Scenarios

- GIVEN a `StreamEvent::EmbedderDownload { phase: "downloading", bytes_downloaded: Some(1024), bytes_total: Some(2048), error: None }`, WHEN it is serialized via `serde_json::to_string`, THEN the JSON contains `"kind":"embedder_download"`, `"phase":"downloading"`, `"bytes_downloaded":1024`, `"bytes_total":2048`, AND does NOT contain the `"error"` key.
- GIVEN the same value with `phase: "verifying"` and all numeric fields `None`, WHEN serialized, THEN the JSON contains only `"kind":"embedder_download"` and `"phase":"verifying"`.
- GIVEN a JSON `{ "kind": "embedder_download", "phase": "error", "error": "boom" }`, WHEN it is deserialized into `StreamEvent`, THEN it matches the `EmbedderDownload` variant with `error == Some("boom")`.

#### Red tests
- `models::chat::tests::embedder_download_serializes_skipping_none_fields`
- `models::chat::tests::embedder_download_roundtrip`

---

### REQ-EVT-002 — Event delivery via `app.emit`

**Progress events MUST flow** from the setup-hook background task via Tauri `app.emit("embedder-download", payload)`, where `payload` is a `StreamEvent::EmbedderDownload` value. The Angular `EmbedderService` MUST subscribe via `@tauri-apps/api/event::listen("embedder-download", ...)`.

A forwarder task MUST translate the internal `UnboundedReceiver<DownloadProgress>` into `app.emit` calls following the same translator pattern as the existing `ToolProgress` forwarder in `src-tauri/src/commands/chat.rs:855-897`.

`downloading` events MUST be rate-limited to bound IPC chatter (avoid Angular CD thrash). **Canonical implementation: byte-based throttle — emit one event per ~64 KB of payload progressed.** On typical broadband (≥320 KB/s) this yields ≤200 ms cadence; on slower links the cadence proportionally widens, which is acceptable (the UI shows live progress whenever bytes flow). A wall-clock 200 ms throttle is an equivalent alternative design; `apply` chose byte-based and tests assert on it.

The lifecycle event sequence on a successful cold start MUST be exactly:
`start → downloading (1..N, rate-limited) → verifying → done`.

On error: any sequence MAY occur, but exactly ONE terminal `error` event MUST be emitted and no further events MUST follow.

#### Scenarios

- GIVEN the download begins successfully (pre-flight OK), WHEN the forwarder runs, THEN exactly one `phase: "start"` event with `bytes_total` populated is emitted to `app.emit("embedder-download", ...)`.
- GIVEN a chunked download in progress, WHEN N chunks are processed, THEN `phase: "downloading"` events are emitted at most once per ~64 KB of payload (the rest are coalesced) — see canonical implementation note above.
- GIVEN SHA-256 verification phase begins after all files are downloaded (or cached files pass pre-check), WHEN entering the verify phase, THEN exactly one `phase: "verifying"` event is emitted **per `ensure_model` run** (not per file).
- GIVEN the model is loaded and `OnceCell::set` succeeds, WHEN the bootstrap completes, THEN exactly one `phase: "done"` event is emitted as the LAST event of the lifecycle. **`ensure_model` MUST NOT emit `Done` itself** — its terminal success state is the `Verify` event followed by `Ok(ModelFiles)`. The sole source of the terminal `done` is `start_embedder_bootstrap` AFTER `OnceCell.set` succeeds. This separation makes `done` mean "embedder ready to use", not "files on disk".
- GIVEN any error path (network exhaustion, SHA mismatch exhaustion, disk pre-flight fail, embedder load fail), WHEN it triggers, THEN exactly one terminal `phase: "error"` event with `error: Some(<msg>)` is emitted and no further events follow.

#### Red tests
- `model_download::tests::lifecycle_events_in_expected_order` (asserts the event vec ends in `verify`, no `Done` from this layer)
- `model_download::tests::downloading_events_rate_limited_by_64kb`
- `model_download::tests::error_path_emits_exactly_one_terminal_error_event`
- `lib::tests::run_embedder_bootstrap_emits_exactly_one_done_on_happy_path` (end-to-end through `EmbedderEventSink` test seam, asserts terminal `done` count == 1)
- `lib::tests::run_embedder_bootstrap_emits_terminal_error_when_ensure_fails`

---

### REQ-STATE-001 — `AppState.embedder` lifecycle

**The field** `pub embedder: Arc<OnceCell<Arc<dyn Embedder>>>` (using `once_cell::sync::OnceCell`) MUST be added to `AppState` in `src-tauri/src/state.rs`. Its lifecycle:

- `AppState::new()` MUST initialize the field as `Arc::new(OnceCell::new())` (empty).
- The setup hook in `src-tauri/src/lib.rs::run()` MUST spawn a `tauri::async_runtime::spawn` task that, on download success, calls `embedder.set(Arc::new(FastEmbedLocal::from_bytes(...)))` exactly once.
- `AppState::new_for_test()` (sync) MUST pre-seed the cell with a `FakeEmbedder` synchronously.
- `AppState::new_for_test_async()` MUST pre-seed the cell with a `FakeEmbedder` before returning.

The setter task MUST handle `OnceCell::set` failure (cell already filled) gracefully — log a warning and continue; this case is only reachable via test misuse or a bug.

#### Scenarios

- GIVEN a fresh `AppState::new()`, WHEN `state.embedder.get()` is called immediately after construction, THEN it returns `None` (caller MUST tolerate this).
- GIVEN an `AppState::new_for_test()` returned from a non-tokio context, WHEN `state.embedder.get()` is called, THEN it returns `Some(&Arc<dyn Embedder>)` whose `model_id()` indicates a fake.
- GIVEN an `AppState::new_for_test_async().await`, WHEN `state.embedder.get()` is called, THEN it returns `Some(...)` (same shape as the sync helper).
- GIVEN a production `AppState` after the setup-hook task succeeded, WHEN `state.embedder.get()` is called, THEN it returns `Some(&Arc<dyn Embedder>)` whose `embed(...)` produces a 384-dim vector.

#### Red tests
- `state::tests::new_returns_empty_embedder_cell`
- `state::tests::new_for_test_preseeds_fake_embedder`
- `state::tests::new_for_test_async_preseeds_fake_embedder`

---

### REQ-UI-001 — Angular `EmbedderService`

**The Angular service** `src/app/services/embedder.service.ts` MUST:

- Be `@Injectable({ providedIn: 'root' })`.
- Use `inject()` for any dependencies (no constructor injection).
- Expose `readonly status = signal<EmbedderStatus>(...)` where `EmbedderStatus` is `{ phase: 'idle' | 'start' | 'downloading' | 'verifying' | 'done' | 'error'; bytesDownloaded?: number; bytesTotal?: number; error?: string }`. Initial value: `{ phase: 'idle' }`.
- On construction, lazy-import `@tauri-apps/api/event` and subscribe via `listen<StreamEvent>('embedder-download', (e) => { ... })`, updating the `status` signal from each event payload.
- Expose `async retry(): Promise<void>` that invokes the Tauri command `retry_embedder_download` and resets `status` to `{ phase: 'start' }` optimistically.

#### Scenarios

- GIVEN the service is constructed, WHEN a `start` event arrives via `listen`, THEN `status()` returns `{ phase: 'start', ... }`.
- GIVEN a `downloading` event with `bytes_downloaded: 100, bytes_total: 200`, WHEN it arrives, THEN `status()` returns `{ phase: 'downloading', bytesDownloaded: 100, bytesTotal: 200 }`.
- GIVEN a `done` event, WHEN it arrives, THEN `status()` returns `{ phase: 'done' }`.
- GIVEN an `error` event with `error: "boom"`, WHEN it arrives, THEN `status()` returns `{ phase: 'error', error: 'boom' }`.
- GIVEN the user clicks Retry, WHEN `retry()` is called, THEN `invoke('retry_embedder_download', ...)` is called exactly once AND `status()` transitions to `{ phase: 'start' }`.

#### Red tests
- `embedder.service.spec.ts`: `updates_signal_on_each_phase_event`, `retry_invokes_tauri_command_and_resets_status`.

---

### REQ-UI-002 — ConfigPanel section + accessibility

**`ConfigPanelComponent` MUST** display a new section reflecting `embedderService.status()` using native control flow (`@if` / `@switch` / `@for`), `class` bindings (NOT `ngClass`), `style` bindings (NOT `ngStyle`), and `ChangeDetectionStrategy.OnPush`.

Per-phase rendering:

| `status().phase` | UI |
|---|---|
| `'idle'` | "Embedder not started" placeholder (or hidden, apply picks). |
| `'start'` | Indeterminate progress indicator labeled "Starting download…". |
| `'downloading'` | Determinate progress bar fed from `bytesDownloaded / bytesTotal`, with byte counts shown via `DecimalPipe`. |
| `'verifying'` | Indeterminate spinner labeled "Verifying model integrity". |
| `'done'` | Success indicator: "Embedder ready: multilingual-e5-small". |
| `'error'` | Error message (uses `status().error`) + a button "Retry download" wired to `embedderService.retry()`. |

**Accessibility requirements (project CLAUDE.md mandates AXE pass + WCAG AA):**

- The downloading progress bar MUST have `role="progressbar"`, `aria-valuemin="0"`, `aria-valuemax="100"`, `aria-valuenow="<computed integer 0..100>"`, and `aria-label="Embedder download progress"`.
- The `verifying` spinner MUST have `role="status"` and `aria-live="polite"`.
- The `error` block MUST have `role="alert"`.
- The Retry button MUST have an accessible name (visible text "Retry download" is sufficient; otherwise `aria-label`).
- Color contrast on text + status indicators MUST meet WCAG AA (≥4.5:1 for body text).
- AXE checks MUST pass with zero violations on the new section.

#### Scenarios

- GIVEN `status() === { phase: 'downloading', bytesDownloaded: 100, bytesTotal: 200 }`, WHEN the component renders, THEN the DOM contains exactly one element with `role="progressbar"` whose `aria-valuenow="50"`.
- GIVEN `status().phase === 'verifying'`, WHEN the component renders, THEN the DOM contains an element with `role="status"` and `aria-live="polite"`.
- GIVEN `status().phase === 'error'`, WHEN the component renders, THEN the DOM contains an element with `role="alert"` containing the error message AND a button whose accessible name matches `/retry/i`.
- GIVEN AXE is run against the rendered section in each of the 5 phases, WHEN scanning, THEN zero accessibility violations are reported.

#### Red tests
- `config-panel.component.spec.ts`:
  - `renders_progressbar_with_aria_valuenow_when_downloading`
  - `renders_status_role_with_aria_live_when_verifying`
  - `renders_alert_role_and_retry_button_when_error`
  - `axe_clean_across_all_phases`

---

### REQ-CMD-001 — `retry_embedder_download` Tauri command

**The system MUST** expose a Tauri command `#[tauri::command] async fn retry_embedder_download(app: AppHandle, state: State<'_, AppState>) -> Result<(), String>` that re-triggers the embedder bootstrap (download + load + `OnceCell::set`). The command MUST be registered in `src-tauri/src/lib.rs::run()`'s `.invoke_handler(...)`.

If the cell is already filled, the command MUST return `Ok(())` immediately and emit a single `phase: "done"` event (idempotent).

#### Scenarios

- GIVEN the embedder is in `phase: "error"` state, WHEN the UI invokes `retry_embedder_download`, THEN a fresh bootstrap is spawned and emits the normal `start → downloading → verifying → done` (or `error`) sequence.
- GIVEN the embedder is already ready (`OnceCell` filled), WHEN `retry_embedder_download` is invoked, THEN exactly one `phase: "done"` event is emitted and the command returns `Ok(())` without touching the network.

#### Red tests
- `commands::embedder::tests::retry_when_empty_spawns_bootstrap` (using injected `MockBootstrap`)
- `commands::embedder::tests::retry_when_ready_is_idempotent`

---

### REQ-TEST-001 — Test independence from real download

**No test in this phase** may download the real ~470 MB model from `huggingface.co`. Tests MUST use:

- `FakeEmbedder` for all embedder behavior tests.
- `mockito` (NEW dev-dep, `[dev-dependencies] mockito = "1"`) to serve fixture bytes with controllable SHA, content-length, chunking, and failure modes for `model_download` tests.
- `tokio::test` runtime for async paths.
- An injectable `BackoffPolicy` whose test impl returns `Duration::ZERO` (no real sleeps).
- An injectable `DiskSpaceProbe` whose test impl returns a configurable free-MB value (no real `sysinfo` call).
- Compile-time `_assert_send_sync::<FastEmbedLocal>()` (REQ-EMB-002).

CI MUST complete the Phase 2 test suite in seconds, not minutes.

#### Scenarios

- GIVEN the full Phase 2 test run, WHEN it executes, THEN ZERO outbound connections target `*.huggingface.co` (verifiable by mock-only setup — `mockito::Server` is the only HTTP endpoint).
- GIVEN any individual test, WHEN it runs, THEN it completes in < 1 s wall-clock (no real backoff sleeps).

#### Red tests
- (Meta-requirement satisfied by construction: every test in this spec uses fakes/mocks/injected probes. Verify phase will audit grep for `"huggingface"` and `tokio::time::sleep` in test code.)

---

## Acceptance criteria (gate for verify phase)

Drawn from `RAG-PHASE-PLAN.md` lines 146-152 and `proposal.md` §Acceptance Criteria. Verify phase MUST tick every box:

- [ ] `Embedder::embed("hola mundo").await` returns `Ok(Vec<f32>)` of length exactly 384, bit-identical across consecutive calls.
- [ ] Cold start without cached model (manual/e2e): UI displays the `start → downloading → verifying → done` sequence; model lands at `<base_dir>/models/multilingual-e5-small/` with all 5 files.
- [ ] Cold start with cached model: app boots without network; embedder is available at first use; ZERO HTTP requests during boot.
- [ ] Cold start without cached model AND no network: NO panic, NO crash; `OnceCell` stays empty; UI shows `phase: "error"` with a Retry button.
- [ ] `mockito` test "skip when all files cached + SHA OK" PASSES.
- [ ] `mockito` test "SHA-256 mismatch detected → delete partial → retry → fail after 3" PASSES.
- [ ] `mockito` test "attempt 1 = 500, attempt 2 = success" PASSES.
- [ ] `sysinfo` test "free disk < 600 MB → `InsufficientDisk`, no HTTP" PASSES.
- [ ] Compile-time `_assert_send_sync::<FastEmbedLocal>()` succeeds.
- [ ] `cargo test --lib --manifest-path src-tauri/Cargo.toml` shows 204 baseline tests + new Phase 2 tests (target 15-25) ALL PASS, ZERO regressions.
- [ ] No test touches `huggingface.co` over the network. No test sleeps for real seconds (injected zero-backoff).
- [ ] Angular ConfigPanel Embedder section renders correctly across all 5 phases (manual + spec-test).
- [ ] AXE checks pass on the new section in all 5 phases.
- [ ] The `app.emit` + `listen` pattern is documented in `design.md` as a codebase precedent with the explicit rule "per-invoke Channel for request-scoped events; `app.emit` for app-wide lifecycle events".

---

## Out of scope (deferred to other phases)

- **Phase 3 (`rag-ingest`)**: chunker + producer-consumer worker that consumes `Embedder`. Phase 3 MUST tolerate `embedder.get() == None` gracefully (carry as a Phase 3 spec requirement).
- **Phase 4 (`rag-retrieval`)**: FTS5 fallback when embedder is unavailable.
- **Phase X**: hot-swap of the active embedder (would require replacing `OnceCell` with `arc_swap::ArcSwap`) and re-indexing on embedder change.
- **Real implementations** of `OllamaEmbedder` / `OpenAIEmbedder` (HTTP/Ollama clients).
- **Partial-resume of failed downloads** (v1 re-downloads from scratch on any failure).
- **`ToolProgress` Angular gap** (`StreamEvent::ToolProgress` exists in Rust but is missing from the Angular discriminated union and the `chat.service.ts` switch) — pre-existing bug, filed as Phase 2 follow-up, NOT Phase 2 scope.
- **`cargo build`** invocation — explicitly forbidden by user CLAUDE.md; verification stays at `cargo check` / `cargo test`.
- **Refactoring `FakeEmbedder` for cross-file reuse** — inline `#[cfg(test)]` is sufficient for Phase 2; Phase 3 will move it behind `#[cfg(any(test, feature = "test-helpers"))]`.
