# Phase 2 — rag-embedder — Exploration

## Baseline

- **Test suite**: 204/204 PASS confirmed (`cargo test --lib --manifest-path src-tauri/Cargo.toml`).
- **Date**: 2026-05-23.
- **Branch**: `main`. `src-tauri/` is fully untracked (`?? src-tauri/`) — same situation as Phase 1, so do NOT use `git stash` to verify regressions (Phase 1 anti-pattern); compare outputs directly.
- **No cargo build** was executed (explicitly forbidden by user CLAUDE.md). All resolution checks used `cargo info`, `cargo search`, `cargo tree`, and `cargo add --dry-run`.

## Locked decisions (passed by orchestrator)

These are NOT up for debate in `propose`:

1. **Fallback FTS5** is deferred to Phase 4 (`rag-retrieval`). Phase 2 ships embedder + download only.
2. **UI progress event** is a NEW typed `StreamEvent::EmbedderDownload { phase, bytes_downloaded, bytes_total }` variant. We do NOT reuse `StreamEvent::ToolProgress`.
3. **Embedder trait** is `trait Embedder: Send + Sync` (async via `async-trait`). `FastEmbedLocal` is the only concrete impl in Phase 2. `OllamaEmbedder` and `OpenAIEmbedder` are stubs returning `unimplemented!()` — NOT feature-gated.
4. **SHA-256 source** is a hardcoded constant in `memory/model_download.rs`.
5. **Model** is `multilingual-e5-small` — 384 dims, ~470 MB. Matches `float[384]` in Phase 1 schema (`memory_vectors`, `episodic_vectors`, `procedural_vectors`).

## Library findings

### fastembed-rs

| Attribute | Value | Source |
|-----------|-------|--------|
| Latest stable version | **5.13.4** | `cargo search fastembed` |
| Pinned ONNX backend | **`ort = "=2.0.0-rc.12"`** (exact pin) | Cargo.toml extracted from `~/.cargo/registry/cache/.../fastembed-5.13.4.crate` |
| `multilingual-e5-small` in registry | **YES** — `EmbeddingModel::MultilingualE5Small`, `dim: 384`, `model_code: "intfloat/multilingual-e5-small"`, `model_file: "onnx/model.onnx"`, `additional_files: []` | `src/models/text_embedding.rs:303-311` |
| API supports user-controlled download path | **YES** via `TextEmbedding::try_new_from_user_defined(UserDefinedEmbeddingModel, InitOptionsUserDefined)` — accepts raw bytes for ONNX + 4 tokenizer files; NO network access | `src/text_embedding/impl.rs:115-167` |
| Built-in download path | `TextEmbedding::try_new(TextInitOptions::new(model).with_cache_dir(path).with_show_download_progress(bool))` — uses `hf-hub` crate internally, progress only via stderr print | `src/text_embedding/impl.rs:44-105`, `src/init.rs:14-50` |
| Required tokenizer files (for user-defined path) | `tokenizer.json`, `config.json`, `special_tokens_map.json`, `tokenizer_config.json` (all 4 as `Vec<u8>`) | `src/common.rs:27-46` (`TokenizerFiles` struct) |
| Other dependencies | `tokenizers 0.22.2` (`onig` feature), `ndarray 0.17.2`, `safetensors 0.7.0`, `anyhow 1` | `fastembed-5.13.4/Cargo.toml` |
| Sync API | `TextEmbedding::embed(Vec<&str>, Option<usize>) -> Result<Vec<Vec<f32>>>` — CPU-bound, **not async**. Must wrap in `tokio::task::spawn_blocking` from async context (matches design in doc 6 line 104) | `src/text_embedding/impl.rs` |
| Windows gotchas | None specific to fastembed-rs itself. `ort 2.0.0-rc.12` ships precompiled ONNX Runtime binaries on Windows via the `download-binaries` feature — no MSVC build step required for ONNX. `tokenizers` `onig` feature pulls `onig_sys` which DOES build native C with MSVC — but `onig` is well-tested on Windows for years (used by `regex`/`rg` ecosystem) and has no known Tauri-specific issues. |

**Decision implication for propose**: use the `try_new_from_user_defined` path. It is the ONLY path that lets us (a) own the download lifecycle, (b) emit typed progress events, (c) verify SHA-256 before commit. The built-in `try_new` path writes progress to stderr via `hf-hub`'s `ureq` client — we cannot intercept it without forking.

### Recommended feature set (avoid `default-features`)

`default = ["ort-download-binaries-native-tls", "hf-hub-native-tls", "image-models"]` — defaults pull `image` crate (~10 deps), `native-tls` (which conflicts with our `reqwest` rustls posture indirectly through `hf-hub`), and `hf-hub` (which we do NOT use in the user-defined path).

Proposed feature set:
```toml
fastembed = { version = "5", default-features = false, features = ["ort-download-binaries-rustls-tls"] }
```

Rationale:
- `ort-download-binaries-rustls-tls` → enables `ort/download-binaries` (precompiled ONNX Runtime) + `ort/tls-rustls` (consistent with `reqwest` already in project).
- No `image-models` → we don't embed images.
- No `hf-hub` → we do the download ourselves with `reqwest` (already a dep).

`cargo add fastembed --dry-run --no-default-features --features ort-download-binaries-rustls-tls,hf-hub-rustls-tls` resolved cleanly to 5.13.4 (verified). Note: dry-run added `hf-hub-rustls-tls` for cross-check, but the propose proposal should drop it.

### Compatibility matrix

| Crate | Our current pin | Latest published | Phase 2 plan | Conflict risk |
|-------|-----------------|------------------|--------------|---------------|
| `tokio-rusqlite` | 0.7.0 | 0.7.0 | unchanged | none — still the latest |
| `rusqlite` | 0.37 (bundled + load_extension) | 0.37.0 | unchanged | none |
| `refinery` | 0.9.1 | 0.9.1 | unchanged | none |
| `sqlite-vec` | 0.1 | 0.1 | unchanged | none |
| `libsqlite3-sys` (transitive) | 0.35.0 (via rusqlite, `bundled`) | — | unchanged | none — `bundled` means no system `libsqlite3` is involved; `ort` does not pull `libsqlite3-sys` |
| `ort` | not present | 2.0.0-rc.12 | **NEW** transitive via fastembed | LOW — fastembed pins `ort = "=2.0.0-rc.12"` exact, so no version drift. `ort 2.x` is the modern API; older 1.x is unrelated. Our project has no other `ort` consumer. |
| `tokenizers` | not present | 0.22.2 (transitive via fastembed) | NEW transitive | LOW — no other consumer in tree |
| `ndarray` | not present | 0.17.2 (transitive via fastembed) | NEW transitive | LOW — no other consumer |
| `safetensors` | not present | 0.7.0 (transitive) | NEW transitive | none |
| `sha2` | 0.10 (direct) | — | reused for SHA-256 verification of downloaded model | none |
| `reqwest` | 0.12 (features: json, stream) | — | reused for HTTP download (already has `stream` for chunked progress) | none |

Verified via `cargo tree --duplicates` on current tree: no duplicates of interest. The only duplicate is `bitflags` (v1 from `png` via Tauri, v2 from `rusqlite`/`tao`) — pre-existing, unrelated to Phase 2.

## Code touchpoints

### Files to create

| File | Purpose | Public surface (proposed) |
|------|---------|---------------------------|
| `src-tauri/src/memory/embedder.rs` | `trait Embedder` (async, `Send + Sync`); `FastEmbedLocal` concrete impl wrapping `Arc<fastembed::TextEmbedding>`; `OllamaEmbedder` and `OpenAIEmbedder` stubs returning `unimplemented!()`; `EmbedError` enum. | `pub trait Embedder { async fn embed(&self, text: &str) -> Result<Vec<f32>, EmbedError>; fn dimensions(&self) -> usize; fn model_id(&self) -> &str; }`, `pub struct FastEmbedLocal { inner: Arc<TextEmbedding> }`, `impl FastEmbedLocal { pub fn from_bytes(onnx: Vec<u8>, tokenizer: TokenizerFiles) -> Result<Self, EmbedError> }`. |
| `src-tauri/src/memory/model_download.rs` | Async download of `intfloat/multilingual-e5-small` from HuggingFace (5 files), SHA-256 verification, atomic cache layout under `<base_dir>/models/multilingual-e5-small/`, progress channel. | `pub async fn ensure_model(base_dir: &Path, progress: Option<UnboundedSender<DownloadProgress>>) -> Result<ModelFiles, DownloadError>`, `pub struct ModelFiles { pub onnx: Vec<u8>, pub tokenizer: TokenizerFiles }`, `pub enum DownloadProgress { Start { bytes_total: u64 }, Chunk { bytes_downloaded: u64 }, Verify, Done }`, `const MODEL_SHA256: &str = "..."` (hardcoded constant — TBD by propose, derived from upstream `intfloat/multilingual-e5-small@onnx/model.onnx`). |

### Files to modify (with current state + planned change)

**`src-tauri/Cargo.toml`** (lines 44-50, "RAG foundations (Phase 1)" section)
- Current: 5 RAG-related deps (tokio-rusqlite, rusqlite, refinery, sqlite-vec, siphasher).
- Planned: add a "Phase 2 (embedder)" subsection with:
  ```toml
  fastembed = { version = "5", default-features = false, features = ["ort-download-binaries-rustls-tls"] }
  ```
  No other deps added — `sha2`, `reqwest`, `tokio`, `async-trait`, `dirs` already present.

**`src-tauri/src/state.rs`** (lines 32-70, `AppState` struct; lines 73-132 `new()`; lines 138-195 `new_for_test()`; lines 197-245 `new_for_test_async()`)
- Current: `memory_store: Arc<MemoryStore>` is the only async-init'd field, opened via `tauri::async_runtime::block_on` in `new()`.
- Planned: add `pub embedder: Arc<dyn Embedder>` to `AppState`. In `new()`, it is initialized as a **placeholder / lazy holder** — see open question 1. In `new_for_test_async()`, inject a `FakeEmbedder` (deterministic hash-to-vector, returns `Vec<f32>` of length 384, NO model load). In `new_for_test()` (sync test helper), same fake.
- The `Arc<dyn Embedder>` matches the doc 6 pattern (line 56-60) and is `Send + Sync` so it crosses tokio task boundaries cheaply.

**`src-tauri/src/lib.rs`** (lines 17-66, `run()` setup hook)
- Current: minimal setup — registers plugins, manages `AppState::new()`, conditionally registers `tauri-plugin-log` in debug. No background tasks spawned.
- Planned: inside the `.setup(|app| { ... })` closure, spawn a `tauri::async_runtime::spawn` task that:
  1. Reads `state.base_dir` from `app.state::<AppState>()`.
  2. Creates a `tokio::sync::broadcast::Sender<EmbedderDownloadEvent>` stored in `AppState` (new field — see open question 2 for the broadcast vs per-window channel question).
  3. Calls `memory::model_download::ensure_model(&base_dir, Some(progress_tx)).await`.
  4. On success: constructs `FastEmbedLocal::from_bytes(...)` and swaps the `AppState.embedder` placeholder via `RwLock` or `OnceCell` (see open question 1).
  5. On failure: logs and leaves embedder unavailable; Phase 4 fallback handles this.

**`src-tauri/src/models/chat.rs`** (lines 96-229, `enum StreamEvent`)
- Current: 16 variants serialized via `#[serde(tag = "kind")]` with kebab-case rename. Existing variants include `Delta`, `State`, `Done`, `Error`, `ToolUse`, `Compaction`, `Usage`, `HookWarning`, `HookGardenEvent`, `RecoveryAttempt`, `RecoveryResult`, `ToolProgress`, `ConfirmationRequired`, `ConfirmationResolved`, `AnomalyReported`.
- Planned: add a new variant:
  ```rust
  #[serde(rename = "embedder_download")]
  EmbedderDownload {
      /// One of: "start" | "downloading" | "verifying" | "done" | "error"
      phase: String,
      #[serde(skip_serializing_if = "Option::is_none")]
      bytes_downloaded: Option<u64>,
      #[serde(skip_serializing_if = "Option::is_none")]
      bytes_total: Option<u64>,
      #[serde(skip_serializing_if = "Option::is_none")]
      error: Option<String>,
  },
  ```
- **CRITICAL — emission delivery channel**: ALL current `StreamEvent`s flow through `tauri::ipc::Channel<StreamEvent>` instantiated PER `stream_chat` invocation (`src-tauri/src/commands/chat.rs:336, 343, 362, 548, 668, 866-892`, etc.). The Angular side instantiates one `Channel<StreamEvent>` in `src/app/services/chat.service.ts:116` per chat send. There is NO global/persistent event channel from Rust to Angular today. The embedder download begins at app boot — BEFORE any chat. See open question 2 for the resolution.

**`src-tauri/src/commands/`** (likely new file `commands/embedder.rs` — to be confirmed by propose)
- Current: no embedder-related command.
- Planned: a Tauri command the Angular ConfigPanelComponent can `invoke` to (a) subscribe to download progress via a fresh `Channel<StreamEvent::EmbedderDownload>`, (b) get current embedder state (`not_started | downloading | ready | failed`). One option: command `subscribe_embedder_download(on_event: Channel<StreamEvent>)` mirrors the `stream_chat` pattern; on subscribe, fan in the active broadcast receiver. Decision for propose.

**Angular: `src/app/models/chat.model.ts`** (lines 56-66, `type StreamEvent`)
- Current: discriminated union with 11 `kind` values — `delta | state | done | error | tool_use | compaction | usage | confirmation_required | confirmation_resolved | anomaly_reported`. Notably **`tool_progress` is NOT listed** — the Rust variant exists but the Angular union does not include it; `chat.service.ts:118-157` likewise has no `case 'tool_progress':` arm. This is a pre-existing inconsistency in the codebase (Rust emits it, Angular ignores it). Phase 2 should NOT fix this — file a follow-up.
- Planned: add `| { kind: 'embedder_download'; phase: 'start' | 'downloading' | 'verifying' | 'done' | 'error'; bytes_downloaded?: number; bytes_total?: number; error?: string }` to the union.

**Angular: `src/app/services/` — new file `embedder.service.ts`**
- Current: no embedder service. Existing pattern (`config.service.ts:39-138`) is `@Injectable({ providedIn: 'root' })` + `signal<T>()` state + lazy `invoke()` calls.
- Planned: `EmbedderService` exposes `readonly status = signal<EmbedderStatus>(...)` with `{ phase, bytesDownloaded, bytesTotal, error }` and on construction `invoke('subscribe_embedder_download', { onEvent: channel })` with a `Channel<StreamEvent>`, switching on `kind === 'embedder_download'`.

**Angular: `src/app/components/config-panel/`** (lines 1-120 TS, 1-183 HTML)
- Current: provider settings UI sections, project path field. Uses `[ngModel]` + `(ngModelChange)` (note: project CLAUDE.md says "do NOT use `ngClass` / `ngStyle`" — `ngModel` is fine and already used everywhere).
- Planned: add a new `<section class="config-section">` "Embedder" that reads from `EmbedderService.status()`, showing:
  - `phase === 'downloading'`: progress bar `bytes_downloaded / bytes_total` (use `DecimalPipe` already imported).
  - `phase === 'verifying'`: indeterminate spinner with text "Verifying model integrity".
  - `phase === 'done'`: green checkmark "Embedder ready: multilingual-e5-small".
  - `phase === 'error'`: error message with `Retry` button (invokes a `retry_embedder_download` command — propose decides).
- Native control flow (`@if` / `@switch` per project conventions).
- `ChangeDetectionStrategy.OnPush` (already the convention).

### Existing patterns to follow

**`AsyncInit` pattern** — `MemoryStore` precedent (`src-tauri/src/state.rs:107-112` + `src-tauri/src/memory/db.rs:51-61`):
- Sync `new()` uses `tauri::async_runtime::block_on` because it runs at app boot before any tokio context exists.
- Async test variant `new_for_test_async()` (`state.rs:201-245`) awaits directly.
- Sync test variant `new_for_test()` (`state.rs:138-195`) detects tokio context via `tokio::runtime::Handle::try_current()` and panics with a guidance message if called from `#[tokio::test]`.
- **For the embedder**: model load (`FastEmbedLocal::from_bytes`) is CPU-bound and takes 100ms+. It also cannot happen at `AppState::new()` because the model may not be on disk yet (cold start). It MUST be lazy/async — initialize a placeholder (or `OnceCell<Arc<dyn Embedder>>`) and fill it from the setup-hook background task. See open question 1.

**StreamEvent emission** — `src-tauri/src/commands/chat.rs`:
- All callers use `let _ = on_event.send(StreamEvent::Variant { ... });` (fire-and-forget; channel may be closed if the frontend disconnected).
- The `Channel<StreamEvent>` is owned by the `stream_chat` command for its lifetime and passed by reference (`&Channel<StreamEvent>`) or cloned (`on_event.clone()`) when spawning forwarder tasks (chat.rs:858).
- Forwarder pattern at `chat.rs:855-897` translates a `tokio::sync::mpsc::UnboundedReceiver<ToolProgress>` into `StreamEvent::ToolProgress` emissions — this is the exact pattern we replicate for embedder download (an `UnboundedReceiver<DownloadProgress>` → `StreamEvent::EmbedderDownload`).

**Angular event listener** — `src/app/services/chat.service.ts:114-160`:
- Lazy-import `{ invoke, Channel }` from `@tauri-apps/api/core`.
- `const channel = new Channel<StreamEvent>();`
- `channel.onmessage = (event) => switch (event.kind) { ... }`.
- Pass as `onEvent: channel` to `invoke('stream_chat', { ..., onEvent: channel })`.
- Replicate verbatim in the new `embedder.service.ts`.

**Project ID path conventions** — `state.base_dir` is set in `state.rs:77-79`:
```rust
let base_dir = dirs::home_dir()
    .unwrap_or_else(|| PathBuf::from("."))
    .join(".claw-garden");
```
Runtime: `C:\Users\<user>\.claw-garden\` on Windows, `~/.claw-garden/` on Unix. Tests (`new_for_test`): `std::env::temp_dir().join(format!("claw-garden-test-{}", pid))`. Models cache: `base_dir.join("models").join("multilingual-e5-small")` — directory does NOT exist today.

## Open questions for propose

1. **How does `AppState.embedder` start out?** Three viable shapes, propose must pick:
   - **(a) `Arc<RwLock<Option<Arc<dyn Embedder>>>>`** — flexible, but every embed call locks. Read-heavy traffic in Phase 3+ makes lock contention plausible.
   - **(b) `Arc<OnceCell<Arc<dyn Embedder>>>`** (via `once_cell` — already in our deps at `once_cell = "1"`, see Cargo.toml:42) — set once, lock-free reads after. CANNOT swap (acceptable; embedder doesn't hot-swap until user explicitly changes it in settings — that's a Phase X concern).
   - **(c) `Arc<dyn Embedder>` initialized to a `PendingEmbedder` stub** that returns `EmbedError::NotReady`, then swapped via an `arc_swap::ArcSwap` (would need new dep `arc-swap`).
   - **Recommendation for propose**: **(b)** `OnceCell`. Already have `once_cell` in deps. Lock-free, simpler than (c), no contention. The fact that we can't swap is FINE for Phase 2 — re-indexing on embedder change is a future Phase X concern per doc 6 lines 125-143.

2. **How does Angular receive embedder download events?** The codebase has NO persistent Rust→Angular event channel — `StreamEvent` flows only through per-invoke `tauri::ipc::Channel` instances. Embedder download begins at app boot. Three options:
   - **(a)** Use Tauri's `app.emit("embedder-download", payload)` + Angular `@tauri-apps/api/event::listen()`. NEW pattern in this codebase but it is the standard Tauri way for app-wide events.
   - **(b)** Have ConfigPanelComponent invoke `subscribe_embedder_download(onEvent: Channel)` on init. The Rust command attaches the channel to a `tokio::sync::broadcast::Receiver`. Late subscribers get a "current state snapshot" plus future events. Matches the Channel pattern but requires broadcast plumbing.
   - **(c)** Polling: Angular `invoke('get_embedder_status')` every 500ms during download.
   - **Recommendation for propose**: **(a)** `app.emit` + `listen`. It's idiomatic Tauri, matches the "app-wide event" semantics, no extra runtime plumbing. The cost is introducing a NEW pattern in this codebase — that should be documented in `design.md`.

3. **`MODEL_SHA256` value** — we know the file is `intfloat/multilingual-e5-small/onnx/model.onnx` on HuggingFace, but we cannot run `cargo build` to download and hash it. Options:
   - Propose phase consults HuggingFace's API (`/api/models/intfloat/multilingual-e5-small`) or the LFS pointer in the repo to extract the published SHA-256 — that's the authoritative source.
   - **Or**: leave the constant as `"PLACEHOLDER_FILL_DURING_APPLY"` in propose/spec, mark a task in `tasks.md` for the apply phase to compute it from a one-shot download, then commit.
   - **Recommendation**: pick the first — HF exposes the SHA in the repo metadata; propose should document the exact URL + verification command (e.g. `curl -sI https://huggingface.co/intfloat/multilingual-e5-small/resolve/main/onnx/model.onnx | grep -i x-linked-etag`). Tokenizer files (~5 MB combined) can either be SHA-checked or trusted by repo integrity (tradeoff for propose).

4. **Download retry policy** — doc 6 line 121 says "Retry con backoff si falla la descarga". Specifics (max retries, backoff schedule, partial-resume support) are NOT locked. Propose must spec it. Recommendation: 3 retries, exponential backoff 5s/30s/120s, NO partial-resume in v1 (re-download from scratch; the ~470 MB cost is acceptable for a rare cold-start error).

5. **TDD test layout for `embedder.rs` and `model_download.rs`** — strict TDD is enabled. The tests should run WITHOUT downloading 470 MB. Concretely:
   - `embedder.rs` tests: build a `FakeEmbedder` (also useful for Phase 3 worker tests) + a `FastEmbedLocal::from_bytes` test that loads a TINY fixture ONNX file (or skips when fixture absent and asserts behavior on a stub). Avoid networking.
   - `model_download.rs` tests: mock the HTTP server with `mockito` (NOT in deps yet — propose decides) OR write integration-style tests against a local file:// URL. Cover: SHA mismatch detected, partial download resumes, 5/5 files present skips download, missing file triggers download.
   - SHA-mismatch test is CRITICAL per the success criteria (`RAG-PHASE-PLAN.md` line 152).

6. **Where does `FakeEmbedder` live?** It is needed by Phase 2 tests AND by Phase 3 worker tests. Three options: (a) inline in `embedder.rs` under `#[cfg(test)]`, (b) public-but-test-only in `memory/embedder.rs` behind `#[cfg(any(test, feature = "test-helpers"))]`, (c) in a separate `memory/embedder_test_helpers.rs`. Recommendation: (a) for Phase 2; refactor to (b) when Phase 3 needs it. Note in propose.

## Risks identified

1. **`ort 2.0.0-rc.12` is a release candidate, not stable** — `cargo info ort` shows `latest 2.0.0-rc.12`; semver-wise, 2.0.0 has never been released. fastembed pins it exactly (`=2.0.0-rc.12`). A future `ort` 2.0.0 release MAY break fastembed; we're at the mercy of fastembed's update cadence. Mitigation: document in design.md as a known dep risk and watch fastembed releases. No action needed in Phase 2.

2. **`tokenizers` crate `onig` feature builds native C on Windows** — `fastembed` depends on `tokenizers 0.22.2` with `features = ["onig"]`. `onig_sys` requires `cc` build-deps (already present in our tree). On clean Windows MSVC setups this works, but our project has not yet pulled it. First `cargo build` after Phase 2 lands may take 5-10 minutes (compiling `onig_sys`, `ort-sys` shims, `safetensors`, `tokenizers`). Mitigation: warn the team in design.md; not a blocker.

3. **Phase 4 fallback FTS5 is the only way to survive a permanently failed embedder download** — Phase 2 ends with the system entering a degraded state if download fails. The success criterion at `RAG-PHASE-PLAN.md` line 151 ("Sin red al primer arranque sin modelo: el sistema NO crashea") is satisfied as long as Phase 4 owns the fallback. Phase 2's responsibility: do NOT crash, log loudly, leave `embedder = None` (or whatever OnceCell pattern resolves). Phase 3 (`rag-ingest`) MUST tolerate a missing embedder — its tasks will need to spec that explicitly.

4. **Disk space at first run** — `~/.claw-garden/models/multilingual-e5-small/` will hold ~475 MB after first launch. On constrained dev machines this could surprise. UI should show the size BEFORE starting download. Decision for propose: pre-flight `<disk-space-check>` step before initiating the actual download.

5. **`AppState::new_for_test()` panic guard** — if propose decides on `Arc<OnceCell<Arc<dyn Embedder>>>`, `new_for_test_async()` must seed the cell BEFORE returning, so tests can `state.embedder.get().unwrap().embed(...)` synchronously. The construction path differs from `MemoryStore` (which initializes always). Be explicit in design.md.

6. **`fastembed::TextEmbedding` is NOT `Send`** — `TextEmbedding` holds an `ort::Session` and `tokenizers::Tokenizer`. Sessions in `ort 2.x` are `Send` (verified via type signatures in `text_embedding/init.rs`), but the COMBINED struct's bounds need verification. If `TextEmbedding: !Send`, our `Arc<dyn Embedder>` requiring `Send + Sync` breaks. Mitigation for propose/design: the canonical workaround is to wrap inside `tokio::sync::Mutex<TextEmbedding>` OR to never share — keep it pinned to a single `spawn_blocking` worker thread accessed via channel. Validate during apply via a 1-line compile assertion `fn assert_send<T: Send>(){} assert_send::<Arc<TextEmbedding>>();`.

## Summary of evidence pointers (for propose)

- Phase plan canonical: `docs/RAG-PHASE-PLAN.md` lines 119-162 (Phase 2 section).
- RAG architecture canonical: `docs/6-improvements-RAG.md` lines 30-143 (embedder + distribution + reindexing).
- fastembed registry entry for our model: `~/.cargo/registry/cache/index.crates.io-1949cf8c6b5b557f/fastembed-5.13.4/src/models/text_embedding.rs:303-311`.
- fastembed user-defined constructor: same archive, `src/text_embedding/impl.rs:115-167`.
- TokenizerFiles spec: same archive, `src/common.rs:27-46`.
- Current `StreamEvent` enum: `src-tauri/src/models/chat.rs:96-229`.
- Angular `StreamEvent` union: `src/app/models/chat.model.ts:56-66` (note: missing `tool_progress` variant — pre-existing inconsistency).
- Channel pattern: backend `src-tauri/src/commands/chat.rs:855-897`; frontend `src/app/services/chat.service.ts:114-160`.
- `AppState` shape: `src-tauri/src/state.rs:32-70`; constructor variants: `73-245`.
- Setup hook: `src-tauri/src/lib.rs:17-66` (currently empty of background tasks).
- Memory module entry: `src-tauri/src/memory/mod.rs` (`pub mod db, error, identity, migrations, record;` — confirmed `embedder.rs` and `model_download.rs` do NOT exist).
- `base_dir` computation: `src-tauri/src/state.rs:77-79`.
