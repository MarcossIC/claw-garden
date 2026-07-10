# Phase 3 — rag-ingest — Tasks

> Strict TDD. Each `*-impl` is preceded by its `*-test`. The apply sub-agent works through this checklist top-to-bottom, mechanically. Every test goes RED FIRST (compile-error or assertion fail). Its paired impl makes it GREEN. Do NOT collapse pairs.

> **Baseline before starting**: `cargo test --lib --manifest-path src-tauri/Cargo.toml` → **249/249 PASS**. Record the exact count; verify is asked to detect zero regressions.

> **No `cargo build`** at any step (user CLAUDE.md forbids it). Verification gates: `cargo check --manifest-path src-tauri/Cargo.toml` for type-level, `cargo test --lib --manifest-path src-tauri/Cargo.toml` for behavior.

> **No `tokio::time::sleep(Duration::from_secs(...))`** in any test. Use `tokio::time::timeout(Duration::from_millis(500), ...)` as a hard-stop deadlock breaker only.

> **All new code**: `log::` macros for logging (project uses `log::warn!` / `log::debug!` — NOT `tracing`; design §10 confirms this), `thiserror` for errors, `pub(crate)` default except where spec mandates `pub`.

---

## Phase 3.0 — Dep pinning

### T01-cargo-add

Dry-run dependency resolution before any code change to surface transitive conflicts early.

- File: `src-tauri/Cargo.toml`
- Run: `cargo add --dry-run tree-sitter --version "0.26"` — confirm resolves to `0.26.x` clean.
- Run: `cargo add --dry-run tree-sitter-rust --version "0.24"` — confirm resolves.
- Run: `cargo add --dry-run tree-sitter-typescript --version "0.23"` — confirm resolves.
- Run: `cargo add --dry-run tree-sitter-python --version "0.25"` — confirm resolves.
- Run: `cargo add --dry-run tokenizers --version "0.22.2"` — confirm resolves (already transitive; explicit pin prevents silent bump).
- Add to `[dependencies]` under a `# RAG Phase 3 (ingest)` comment:
  ```toml
  tree-sitter            = "0.26"
  tree-sitter-rust       = "0.24"
  tree-sitter-typescript = "0.23"
  tree-sitter-python     = "0.25"
  tokenizers             = "0.22.2"
  ```
- Verify: `cargo check --manifest-path src-tauri/Cargo.toml` succeeds. Expect ~15-30 s added compile time on first `cc` grammar build (proposal R7 — known, one-time).
- NO `cargo build` invoked.

---

## CHECKPOINT 1 — Foundations: events, channel, ChatMessage, AppState

**Pause point: run `cargo test --lib --manifest-path src-tauri/Cargo.toml`; verify count = 249 baseline + N new (zero regressions).**

### T02-test

Write failing tests for `MemoryEvent` and `MemorySource` in `src-tauri/src/memory/events.rs::tests` (REQ-EVT-001, REQ-EVT-002).

Test function names (MUST match spec):
- `memory_event_is_send` — `fn _assert_send<T: Send>() {} _assert_send::<MemoryEvent>();` inside a `#[test]`. Will fail with compile error until `MemoryEvent` exists and is `Send`.
- `memory_event_fields_round_trip` — construct a `MemoryEvent { role: Role::Tool, is_error: Some(true), content: "failed".into(), agent_id: "a".into(), project_id: None, source: MemorySource::Chat, timestamp_ms: 123 }`, assert `role == Role::Tool` and `is_error == Some(true)`.
- `memory_source_is_copy_and_clone` — `let s = MemorySource::Chat; let _a = s; let _b = s;` — uses both copy and clone semantics; compiles only if `Copy + Clone`.
- `memory_source_exhaustive_match` — match on `MemorySource::Chat` with three arms (Chat, Compaction, CodeFile); assert no compile warning about non-exhaustive match.
- `sender_dropped_does_not_panic` — create an unbounded channel, drop the receiver immediately, call `tx.send(MemoryEvent { ... })`, assert no panic (the `Err` is swallowed).

Verify (must FAIL): `cargo test --lib memory::events` — compile errors (no module yet).

### T02-impl

Create `src-tauri/src/memory/events.rs` and register in `mod.rs`. Satisfies REQ-EVT-001, REQ-EVT-002, REQ-EVT-003.

- Define `#[derive(Debug, Clone, Serialize, Deserialize)] pub(crate) struct MemoryEvent` with exact fields from spec REQ-EVT-001.
- Define `#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)] #[serde(rename_all = "snake_case")] pub(crate) enum MemorySource { Chat, Compaction, CodeFile }`.
- Add `impl MemoryEvent { pub fn from_message(...) -> Option<Self> }` per design §3.1 — filters `Role::System` and empty-content assistant tool-call turns.
- Add the compile-time `const _: fn()` Send + 'static block per REQ-STATE-003 and design §3.1.
- Register: `pub mod events;` in `src-tauri/src/memory/mod.rs`.

Verify: `cargo test --lib memory::events` — 5 new tests PASS, 249 baseline PASS.

---

### T03-test

Write failing tests for `ChatMessage.is_error` field in `src-tauri/src/models/chat.rs::tests` (REQ-INT-001).

Test function names:
- `is_error_field_defaults_to_none_on_missing_key` — deserialize `{"role":"user","content":"hi","timestamp_ms":0}` into `ChatMessage`; assert `is_error == None`.
- `is_error_field_serializes_false_when_some_false` — construct `ChatMessage { ..., is_error: Some(false) }`, serialize to JSON; assert output contains `"is_error":false`.
- `is_error_field_omitted_when_none` — construct with `is_error: None`, serialize; assert output does NOT contain `"is_error"`.
- `is_error_field_some_true_serializes_correctly` — `is_error: Some(true)`, serialize; assert contains `"is_error":true`.

Verify (must FAIL): compile error — `ChatMessage` struct literal misses `is_error` field (once `is_error` is added, existing construction sites without it will cause compile errors too — that's expected).

### T03-impl

Add `is_error: Option<bool>` to `ChatMessage` in `src-tauri/src/models/chat.rs`. Satisfies REQ-INT-001, proposal D1.

- Add field with `#[serde(default, skip_serializing_if = "Option::is_none")]` as specified in design §4.c.
- Update ALL existing `ChatMessage { ... }` construction sites in `commands/chat.rs` (lines 413, 622, 652) to include `is_error: None`.
- At line 1049 ONLY: set `is_error: Some(is_error)` (the `is_error: bool` local is already in scope at that line per explore.md §critical-finding).

Verify: `cargo test --lib models::chat` — 4 new tests PASS + 249 baseline PASS, ZERO compile errors across the codebase.

---

### T04-test

Write failing tests for `AppState.memory_tx` in `src-tauri/src/state.rs::tests` (REQ-STATE-001, REQ-STATE-002).

Test function names:
- `memory_tx_send_before_worker_buffers_event` — call `AppState::new()` (now returns tuple), send one `MemoryEvent` via `state.memory_tx`, assert send returns `Ok(())`.
- `memory_tx_clone_sends_independently` — clone `state.memory_tx`, send via both clones, consume both via the returned `memory_rx`; assert 2 events received.
- `new_for_test_async_memory_tx_does_not_deadlock` — call `AppState::new_for_test_async().await` (tuple return), send 3 events via `state.memory_tx`, drain rx; assert all 3 sends return `Ok(())` without deadlock.
- `new_for_test_with_memory_rx_round_trip` — call the new `AppState::new_for_test_with_memory_rx().await` helper; send 1 event, `memory_rx.recv().await`; assert event received correctly.

Verify (must FAIL): compile errors — `AppState::new()` still returns `Self`, no `memory_tx` field.

### T04-impl

Add `memory_tx` field to `AppState` and change all constructors to return tuple `(Self, UnboundedReceiver<MemoryEvent>)`. Satisfies REQ-STATE-001, REQ-STATE-002, design §4.a.

- In `state.rs`: add `pub memory_tx: UnboundedSender<MemoryEvent>` field.
- Change `AppState::new()` to return `(Self, UnboundedReceiver<MemoryEvent>)` per design §4.a. This is risk R-NEW-3 — EVERY call site of `AppState::new()` must be updated.
- Change `new_for_test()` and `new_for_test_async()` to also return tuples. Tests that don't assert on events get a `tokio::spawn(async move { while rx.recv().await.is_some() {} });` sink drain.
- Add `#[cfg(test)] pub async fn new_for_test_with_memory_rx() -> MemoryTestHandle` per spec REQ-STATE-002 (or equivalent struct with `state` + `memory_rx` fields).
- Update `lib.rs::run()` to destructure the tuple: `let (state, memory_rx) = AppState::new();`.
- Update ALL test files that previously called `AppState::new_for_test_async().await` as a plain value (not destructured tuple) — mechanical per R-NEW-3.

Verify: `cargo test --lib state` — 4 new tests PASS + 249 baseline PASS. `cargo check` clean.

---

### T05-test

Write failing test for worker spawn in `src-tauri/src/memory/worker.rs::tests` (REQ-WRK-001).

Test function name:
- `spawn_memory_worker_returns_join_handle` — call `spawn_test_worker(store, embedder)` (the internal test harness helper per design §8.2); assert the returned `JoinHandle` is not yet complete (immediately); drop the sender; await handle with `tokio::time::timeout`; assert it resolves `Ok`.

Verify (must FAIL): compile errors — no `worker.rs` module yet.

### T05-impl

Create `src-tauri/src/memory/worker.rs` skeleton with channel plumbing. Satisfies REQ-WRK-001, design §3.6.

- Define `pub(crate) async fn memory_worker(mut rx, store, embedder_cell, chat_chunker, code_chunker)` — empty loop that just drains `rx` for now.
- Define `pub(crate) fn start_memory_worker(app_handle: tauri::AppHandle, memory_rx: UnboundedReceiver<MemoryEvent>)` per design §3.6.
- Add the compile-time `const _: fn()` Send guard block per design §9 (worker.rs location).
- Add `#[cfg(test)] async fn spawn_test_worker(store, embedder) -> (UnboundedSender<MemoryEvent>, JoinHandle<()>)` per design §8.2.
- Define `#[derive(Debug, thiserror::Error)] pub(crate) enum MemoryWorkerError` per design §5.1.
- Register: `pub mod worker;` in `memory/mod.rs`.
- Wire `start_memory_worker` call into `lib.rs::run()` setup hook after `start_embedder_bootstrap`, passing `memory_rx` (per design §4.b).

Verify: `cargo test --lib memory::worker::tests::spawn_memory_worker_returns_join_handle` — PASS. `cargo check` clean.

---

**Pause point: run `cargo test --lib --manifest-path src-tauri/Cargo.toml`; verify count = 249 baseline + N new (zero regressions).**

---

## CHECKPOINT 2 — Chat chunker complete

### T06-test

Write failing tests for `ChunkerError`, `ChunkerKind`, `Chunk`, `SpeakerKind`, and `select_chunker` in `src-tauri/src/memory/chunker/mod.rs::tests` (REQ-CHK-001, REQ-CHK-002, REQ-CHK-006).

Test function names:
- `select_chunker_user_routes_to_chat` — `select_chunker(Role::User, None, "hello")` returns `ChunkerKind::Chat`.
- `select_chunker_assistant_with_content_routes_to_chat` — `select_chunker(Role::Assistant, None, "some text")` returns `ChunkerKind::Chat`.
- `select_chunker_assistant_empty_content_routes_to_skip` — `select_chunker(Role::Assistant, None, "")` returns `ChunkerKind::Skip`.
- `select_chunker_tool_routes_to_chat` — `select_chunker(Role::Tool, Some(true), "error msg")` returns `ChunkerKind::Chat`.
- `select_chunker_system_routes_to_skip` — `select_chunker(Role::System, None, "system prompt")` returns `ChunkerKind::Skip`.
- `chat_chunk_has_no_ast_header` — a `Chunk { text: "hi".into(), token_count: 1, speaker: SpeakerKind::User, ast_header: None }` has `ast_header == None`.
- `code_chunk_has_ast_header` — `Chunk { ..., ast_header: Some("fn foo".into()) }` has `ast_header == Some(...)`.
- `chunker_error_implements_error_trait` — `let _: Box<dyn std::error::Error> = Box::new(ChunkerError::ParseFailed("x".into()));` compiles.
- `chunker_error_parse_failed_displays_message` — `format!("{}", ChunkerError::ParseFailed("syntax error".into()))` contains `"syntax error"`.

Verify (must FAIL): compile errors — no `chunker` module.

### T06-impl

Create `src-tauri/src/memory/chunker/mod.rs` with all shared types. Satisfies REQ-CHK-001, REQ-CHK-002, REQ-CHK-006.

- Define `pub(crate) enum ChunkerKind { Chat, Code, Skip }`.
- Define `pub(crate) enum SpeakerKind { User, Agent, Tool }`.
- Define `pub(crate) struct Chunk { pub text: String, pub token_count: usize, pub speaker: SpeakerKind, pub ast_header: Option<String> }` with `#[derive(Debug, Clone)]`.
- Define `#[derive(Debug, thiserror::Error)] pub(crate) enum ChunkerError { #[error("{0}")] ParseFailed(String), #[error("tokenizer init failed: {0}")] TokenizerInit(String) }`.
- Implement `pub(crate) fn select_chunker(role: Role, is_error: Option<bool>, content: &str) -> ChunkerKind` per spec REQ-CHK-001 and design §3.2 (note: design routes ALL non-empty roles except System to Chat in Phase 3; `is_error` is consumed with `let _ = is_error`).
- Add `pub(crate) mod chat; pub(crate) mod code; pub(crate) mod token_splitter;` declarations.
- Register: `pub mod chunker;` in `memory/mod.rs`.

Verify: `cargo test --lib memory::chunker::tests` — 9 new tests PASS, 249 baseline PASS.

---

### T07-test

Write failing tests for `TokenSplitter` in `src-tauri/src/memory/chunker/mod.rs::tests` (REQ-CHK-005).

Test function names:
- `token_split_respects_cap` — create a `TokenSplitter::fallback()`, call `split("a".repeat(10_000).as_str(), 480)` (using byte heuristic: max_bytes = 480 * 4 = 1920); assert output has ≥ 2 segments AND each segment's byte length ≤ 1920.
- `token_split_short_input_single_segment` — `TokenSplitter::fallback().split("hello world", 480)` returns exactly 1 segment.

Verify (must FAIL): compile error — `TokenSplitter` not defined yet.

### T07-impl

Create `src-tauri/src/memory/chunker/token_splitter.rs`. Satisfies REQ-CHK-005, design §3.3.

- Define `pub(crate) struct TokenSplitter { tokenizer: Option<Arc<Tokenizer>> }`.
- Implement `pub fn with_tokenizer(tokenizer: Arc<Tokenizer>) -> Self` and `pub fn fallback() -> Self`.
- Implement `pub fn split(&self, text: &str, max_tokens: usize) -> Vec<Chunk>` per design §3.3: tokenizer path uses `encode` + offset slicing; byte path uses `max_tokens.saturating_mul(4)` bytes cap walking char boundaries.
- Returned `Chunk` values from this helper have `speaker: SpeakerKind::Agent` as a neutral default; callers overwrite `speaker` if needed.

Verify: `cargo test --lib memory::chunker::tests::token_split_respects_cap` PASS + `token_split_short_input_single_segment` PASS.

---

### T08-test

Write failing tests for `ChatChunker` in `src-tauri/src/memory/chunker/chat.rs::tests` (REQ-CHK-003).

Test function names (MUST match spec):
- `speaker_boundary_respected_single_turn` — `ChatChunker::with_fallback().chunk("hello world", 480)` returns 1 chunk with `token_count ≤ 100` (byte fallback: any short string fits).
- `token_cap_480_produces_multiple_chunks` — build a content string of ~6000 bytes (≈ 1500 tokens in byte mode at 4 bytes/token); `ChatChunker::with_fallback().chunk(content, 480)` returns ≥ 2 chunks each with byte length ≤ 1920.
- `no_overlap_adjacent_chunks` — for the above multi-chunk result, concatenate all chunk texts; assert the concatenation equals the original input (no duplication, no gaps).
- `empty_content_returns_empty_vec` — `ChatChunker::with_fallback().chunk("", 480)` returns `Ok(vec![])` (or just `vec![]` since the design returns `Vec<Chunk>` not `Result`).
- `tokenizer_absent_falls_back_to_byte_heuristic` — `ChatChunker::new_from_base_dir(Path::new("/nonexistent/path"))` constructs without panic; `chunk("hello", 480)` returns a non-empty vec.
- `exactly_480_tokens_is_single_chunk` — a string whose byte length is exactly 480 * 4 = 1920 in fallback mode produces exactly 1 chunk.

Verify (must FAIL): compile error — `ChatChunker` not yet defined.

### T08-impl

Create `src-tauri/src/memory/chunker/chat.rs`. Satisfies REQ-CHK-003, design §3.4.

- Define `pub(crate) struct ChatChunker { splitter: TokenSplitter }`.
- Implement `pub fn new_from_base_dir(base_dir: &Path) -> Self` — attempts to load tokenizer via `crate::memory::model_download::tokenizer_path(base_dir)`, falls back to `TokenSplitter::fallback()` on any error, logs `log::warn!` on fallback.
- Add `#[cfg(test)] pub fn with_tokenizer(tok: Arc<Tokenizer>) -> Self` and `#[cfg(test)] pub fn with_fallback() -> Self` per design §3.4.
- Implement `pub fn chunk(&self, content: &str, max_tokens: usize) -> Vec<Chunk>` — empty guard, then `self.splitter.split(content, max_tokens)`.
- Returned chunks inherit `speaker: SpeakerKind::Agent` from `TokenSplitter`; caller sets speaker when building `NewMemoryRecord`.
- Add compile-time `ChatChunker: Send + Sync` guard per design §9.

Verify: `cargo test --lib memory::chunker::chat::tests` — 6 new tests PASS + all previous PASS.

---

### T09-test

Write failing test for `tokenizer_path` helper in `src-tauri/src/memory/model_download.rs::tests` (proposal R8).

Test function name:
- `tokenizer_path_returns_correct_subpath` — `model_download::tokenizer_path(Path::new("/base"))` equals `PathBuf::from("/base/models/multilingual-e5-small/tokenizer.json")`.

Verify (must FAIL): compile error — `tokenizer_path` fn does not exist yet.

### T09-impl

Add `pub fn tokenizer_path(base_dir: &Path) -> PathBuf` to `src-tauri/src/memory/model_download.rs`. Satisfies proposal R8 and design §6.1.

- One-liner: `base_dir.join("models").join("multilingual-e5-small").join("tokenizer.json")`.
- No other changes to `model_download.rs`.

Verify: `cargo test --lib memory::model_download::tests::tokenizer_path_returns_correct_subpath` PASS.

---

**Pause point: run `cargo test --lib --manifest-path src-tauri/Cargo.toml`; verify count = 249 baseline + N new (zero regressions).**

---

## CHECKPOINT 3 — Code chunker complete

### T10-test

Write failing tests for `CodeChunker` in `src-tauri/src/memory/chunker/code.rs::tests` (REQ-CHK-004).

Test function names (MUST match spec):
- `rust_two_functions_produces_two_chunks_with_headers` — input is a valid Rust snippet with two `fn` definitions; `CodeChunker::new().chunk(content, Some("rs"))` returns exactly 2 chunks, both with `ast_header` of the form `Some("function_item (rs)")` or equivalent node-kind prefix.
- `typescript_class_produces_chunk_with_header` — TypeScript class source; `chunk(content, Some("ts"))` returns ≥ 1 chunk with `ast_header` containing `"class_declaration"` or similar tree-sitter node kind string.
- `python_function_produces_chunk_with_header` — Python `def` source; `chunk(content, Some("py"))` returns ≥ 1 chunk with `ast_header` containing `"function_definition"` or similar.
- `unknown_extension_falls_back_to_chat_chunker` — `CodeChunker::new().chunk("some text", None)` returns `Ok(vec)` without error or panic; chunks have `ast_header == None`.
- `malformed_rust_falls_back_no_panic` — input is `"fn {{{{{ broken"` with `language = Some("rs")`; call does NOT panic and returns at least one chunk (fallback path).
- `large_function_split_respects_token_cap` — a Rust `fn` body of ~8000 bytes; all output chunks have byte length ≤ 1920 (byte fallback mode) AND each carries the same `ast_header`.

Verify (must FAIL): compile error — `CodeChunker` not defined, tree-sitter grammars not imported.

### T10-impl

Create `src-tauri/src/memory/chunker/code.rs`. Satisfies REQ-CHK-004, design §3.5.

- Define `pub(crate) struct CodeChunker { rust: Mutex<Parser>, typescript: Mutex<Parser>, python: Mutex<Parser>, fallback: TokenSplitter }`.
- Implement `pub fn new() -> Self` per design §3.5 — `make_parser` helper for each grammar; panics on grammar-version mismatch (acceptable: detected at startup, not per-request).
- Implement `pub fn chunk(&self, content: &str, language: Option<&str>) -> Vec<Chunk>` per design §3.5: unknown/None language → `self.fallback.split(content, 480)`; known language → parse, walk top-level named children, emit one `Chunk` per node with `ast_header = Some(format!("{} ({})", node.kind(), language))`, sub-split oversized nodes via `self.fallback.split(snippet, 480)`.
- Parse failure (parse returns None) or empty tree root → fallback.
- Add compile-time `CodeChunker: Send + Sync` guard per design §9 / §7.3.

Verify: `cargo test --lib memory::chunker::code::tests` — 6 new tests PASS + all previous PASS.

---

**Pause point: run `cargo test --lib --manifest-path src-tauri/Cargo.toml`; verify count = 249 baseline + N new (zero regressions).**

---

## CHECKPOINT 4 — MemoryStore insert API complete

### T11-test

Write failing tests for `MemoryStoreError` in `src-tauri/src/memory/store_insert.rs::tests` (REQ-INS-004).

Test function names:
- `memory_store_error_is_send_sync` — compile-time: `fn _s<T: Send + Sync>() {} _s::<MemoryStoreError>();` inside a `#[test]`.
- `memory_store_error_displays_message` — `format!("{}", MemoryStoreError::DimensionMismatch { expected: 384, actual: 100 })` returns a non-empty string containing `"384"` and `"100"`.

Verify (must FAIL): compile error — no `store_insert.rs` module.

### T11-impl

Create `src-tauri/src/memory/store_insert.rs` with error types and supporting structs. Satisfies REQ-INS-001 (partial), REQ-INS-002, REQ-INS-004, design §3.7.

- Define `pub enum MemoryType { ChatMessage, ToolError, CodeBlock }` with `as_str()`.
- Define `pub enum CreatedByType { Agent, User, System }` with `as_str()`.
- Define `pub struct NewMemoryRecord` with all fields from spec REQ-INS-002 (use `pub scope: &'static str`, `pub agent_id: Option<String>`, etc. per design §3.7).
- Implement `NewMemoryRecord::from_event_and_chunk(evt: &MemoryEvent, chunk: &Chunk) -> Self` per design §3.7 — derives `memory_type`, `created_by_type`, `importance` per REQ-WRK-005.
- Define `#[derive(Debug, thiserror::Error)] pub enum MemoryStoreError { DimensionMismatch { expected: usize, actual: usize }, Sqlite(rusqlite::Error), Connection(tokio_rusqlite::Error) }` per design §5.2.
- Register: `pub mod store_insert;` in `memory/mod.rs`.

Verify: `cargo test --lib memory::store_insert::tests::memory_store_error_is_send_sync` PASS + `memory_store_error_displays_message` PASS.

---

### T12-test

Write failing tests for `insert_memory_with_vector` in `src-tauri/src/memory/store_insert.rs::tests` (or `memory::db::tests`) (REQ-INS-001, REQ-INS-003).

Test function names (MUST match spec):
- `insert_memory_with_vector_returns_rowid` — construct a `MemoryStore::new_for_test().await` and a valid `NewMemoryRecord` with `scope = "agent"`, `agent_id = Some("a")`, a 384-dim `Vec<f32>` embedding; call `store.insert_memory_with_vector(record, embedding).await`; assert `Ok(id)` where `id > 0`.
- `insert_memory_with_vector_returns_err_on_closed_conn` — this test may be skipped if tokio_rusqlite does not expose a clean connection-close API; add an inline `// TODO: R-NEW-7 — connection-close testing deferred` comment if skipped and use `MemoryStoreError::DimensionMismatch` as a proxy: pass a 10-dim embedding and assert `Err(MemoryStoreError::DimensionMismatch { expected: 384, actual: 10 })`.
- `both_rows_present_after_successful_insert` — after `Ok(id)`: `SELECT COUNT(*) FROM memory WHERE id=?` returns 1 AND `SELECT COUNT(*) FROM memory_vectors WHERE rowid=?` returns 1.
- `vector_insert_failure_leaves_no_memory_row` — inject wrong-dimension embedding (not 384); assert `Err(_)` AND `SELECT COUNT(*) FROM memory` returns 0.
- `insert_agent_scope_no_project_passes_check` — `project_id = None`, `scope = "agent"`, `agent_id = Some("a")`; assert `Ok(id)` (CHECK constraint satisfied per proposal D3).
- `insert_agent_scope_with_project_passes_check` — `project_id = Some("proj-1")`, `scope = "agent"`; assert `Ok(id)`.

Verify (must FAIL): compile error — `insert_memory_with_vector` method not yet on `MemoryStore`.

### T12-impl

Add `insert_memory_with_vector` as an `impl MemoryStore` block in `store_insert.rs`. Satisfies REQ-INS-001, REQ-INS-003, design §3.7.

- Implement the atomic transaction per design §3.7 and §1 lifecycle: open tx → `INSERT INTO memory (...) VALUES (...)` → `tx.last_insert_rowid()` → serialize `embedding` to little-endian bytes → `INSERT INTO memory_vectors(rowid, embedding) VALUES (?, ?)` → `tx.commit()`.
- Dimension guard: check `embedding.len() != 384` before hitting SQLite; return `MemoryStoreError::DimensionMismatch` immediately.
- Signature: `pub async fn insert_memory_with_vector(&self, record: NewMemoryRecord, embedding: Vec<f32>) -> Result<i64, MemoryStoreError>`.
- Map `tokio_rusqlite::Error` to `MemoryStoreError::Connection` via `From`.

Verify: all T12 tests PASS + all prior PASS. `cargo check` clean.

---

**Pause point: run `cargo test --lib --manifest-path src-tauri/Cargo.toml`; verify count = 249 baseline + N new (zero regressions).**

---

## CHECKPOINT 5 — Worker integration + push_and_persist wiring + E2E

### T13-test

Write failing tests for `push_and_persist` event emission in `src-tauri/src/commands/chat.rs::tests` (REQ-INT-002).

Test function names (MUST match spec):
- `push_and_persist_emits_event_for_user_role` — build `(state, mut rx)` via `AppState::new_for_test_async().await`; call a `push_and_persist_inner` test seam (extracted from the full `push_and_persist`) with a `Role::User` message; assert `rx.try_recv()` yields one `MemoryEvent` with `role == Role::User`.
- `push_and_persist_skips_event_for_empty_assistant_tool_call` — `Role::Assistant` with `content = ""` and `tool_calls = Some(...)` → `rx.try_recv()` returns `Err(TryRecvError::Empty)`.
- `push_and_persist_emits_event_for_tool_with_is_error` — `Role::Tool` with `is_error = Some(true)` → event received with `is_error == Some(true)`.
- `push_and_persist_dropped_receiver_no_panic` — drop `rx` immediately after `new_for_test_async`; call push_and_persist; assert no panic.

Verify (must FAIL): compile error — `push_and_persist` does not accept `agent_project_path` param yet, no test seam.

### T13-impl

Update `push_and_persist` in `src-tauri/src/commands/chat.rs` to emit `MemoryEvent`. Satisfies REQ-INT-002, REQ-INT-003, design §4.d, risk R-NEW-1.

- Add `agent_project_path: Option<&str>` parameter to `push_and_persist` — this is risk R-NEW-1. Update ALL 4 call sites (lines 413, 622, 652, 1049) to pass the appropriate value. Lines 413/622/652 pass `agent.project_path.as_deref()` (or the equivalent available binding); line 1049 passes the same.
- After the existing `session.append_message(...)` success, add the ingest emit block per design §4.d:
  - Derive `project_id` per REQ-INT-003: `agent_project_path.filter(|s| !s.is_empty()).map(|s| derive_project_id(Path::new(s)))`.
  - Build `MemoryEvent::from_message(agent_id.to_string(), project_id, entry.last().unwrap(), MemorySource::Chat)`.
  - On `Some(evt)`: `let _ = state.memory_tx.send(evt)` — fire-and-forget; on `Err(e)`: `log::debug!(...)`.
- Add a `#[cfg(test)]` extracted inner function to expose a testable seam that bypasses the full Tauri state machinery if needed.

Verify: T13 tests PASS + all prior PASS.

---

### T14-test

Write failing tests for worker graceful shutdown and per-event error isolation in `src-tauri/src/memory/worker.rs::tests` (REQ-WRK-002).

Test function names (MUST match spec):
- `worker_exits_gracefully_when_all_senders_dropped` — `spawn_test_worker(store, embedder).await`; drop sender; await handle via `tokio::time::timeout(Duration::from_millis(500), handle)`.expect no timeout and no panic.
- `worker_continues_after_per_event_error` — send one event where the `ChunkerKind::Skip` path is taken (empty content, assistant role), then send a valid event; after draining, assert the worker is still alive (timeout not expired) and at least 1 row in `memory`.

Verify (must FAIL): worker loop is still a no-op skeleton — no processing occurs, so rows assertion fails.

### T14-impl

Fill in the worker event-processing loop in `worker.rs`. Satisfies REQ-WRK-002, design §3.6.

- Implement the full `process_event` inner function per design §3.6: `select_chunker` → `match ChunkerKind` → embedder gate (D4) → per-chunk embed+insert loop.
- The outer `memory_worker` loop: `while let Some(evt) = rx.recv().await { if let Err(e) = process_event(...).await { log::warn!("ingest event dropped: {e}"); } }` followed by `log::info!("memory_worker shutting down")`.
- NEVER panics per REQ-WRK-002. All errors propagate through `process_event → MemoryWorkerError` and get logged.

Verify: T14 tests PASS + all prior PASS.

---

### T15-test

Write failing tests for embedder failure isolation in `src-tauri/src/memory/worker.rs::tests` (REQ-WRK-003, REQ-WRK-004).

Test function names (MUST match spec):
- `embedder_cell_empty_no_rows_inserted` — call `spawn_test_worker` but pass an empty `OnceCell` (NOT seeded with an embedder); send 1 event; drain channel; assert `SELECT COUNT(*) FROM memory` = 0.
- `embedder_not_ready_no_rows_inserted` — use a `FakeEmbedder` variant that always returns `Err(EmbedError::NotReady)`; send 1 event; assert 0 rows.
- `embedder_worker_gone_skips_chunk_continues` — first event: embedder returns `EmbedError::WorkerGone`; second event: embedder returns `Ok(vec)`; assert first event = 0 rows, second event = 1 row (the worker must have continued).
- `store_write_failure_worker_stays_alive` — after injecting a fault (drop `memory_vectors` table via `store.conn.call(|c| c.execute("DROP TABLE memory_vectors", []))` between events), send 2 events; assert the worker is still alive after both (handle not resolved) and logged warnings.

Verify (must FAIL): embedder-cell-empty path not implemented in process_event yet.

### T15-impl

Implement the embedder gate (LOCKED D4) and store-write failure isolation in `process_event`. Satisfies REQ-WRK-003, REQ-WRK-004.

- The `let Some(embedder) = embedder_cell.get().cloned() else { log::warn!(...); return Ok(()); }` gate per design §3.6 and §1 lifecycle.
- Per-chunk embed failure handling per design §1 failure branches: `NotReady | WorkerGone` → break inner loop (whole event lost); `InferenceFailed | LoadFailed` → warn + continue (single chunk skipped).
- `insert_memory_with_vector` `Err` → `log::warn!` + continue inner loop; worker NEVER stops.
- The `spawn_test_worker` helper must allow injecting an empty cell (expose a second test constructor that accepts `Option<Arc<dyn Embedder>>` where `None` = empty cell).

Verify: T15 tests PASS + all prior PASS.

---

### T16-test

Write failing tests for `memory_type` / `created_by_type` derivation in `src-tauri/src/memory/worker.rs::tests` (REQ-WRK-005).

Test function names (MUST match spec):
- `user_role_produces_chat_message_type_user_creator` — send `Role::User` event; after processing, query `SELECT memory_type, created_by_type FROM memory LIMIT 1`; assert `memory_type = "chat_message"` and `created_by_type = "user"`.
- `assistant_role_produces_agent_creator` — `Role::Assistant` with non-empty content; assert `created_by_type = "agent"`.
- `tool_error_produces_tool_error_type_system_creator_importance_6` — `Role::Tool` + `is_error = Some(true)`; assert `memory_type = "tool_error"`, `created_by_type = "system"`, `importance >= 6.0`.
- `tool_success_produces_chat_message_type_system_creator` — `Role::Tool` + `is_error = Some(false)`; assert `memory_type = "chat_message"`, `created_by_type = "system"`.

Verify (must FAIL): `NewMemoryRecord::from_event_and_chunk` is implemented but the worker loop was not yet fully wired to actually insert rows (T14-impl only scaffolded the loop; by T15-impl it should be doing inserts).

### T16-impl

This impl task is verification-only: the derivation logic lives in `NewMemoryRecord::from_event_and_chunk` (T11-impl). No new code is added here — run the tests and confirm the derivation produces correct SQL values. If a field mapping is wrong, fix it in `store_insert.rs::NewMemoryRecord::from_event_and_chunk`.

Verify: T16 tests PASS + all prior PASS.

---

### T17-test

Write failing tests for `project_id` derivation in the worker in `src-tauri/src/memory/worker.rs::tests` (REQ-INT-003).

Test function names (MUST match spec):
- `project_id_derived_from_path` — send a `MemoryEvent { project_id: Some("derived-hash".into()), ... }` (pre-derived at call site); after insert, query `SELECT project_id FROM memory LIMIT 1`; assert equals `"derived-hash"`.
- `project_id_none_when_no_path` — send event with `project_id: None`; after insert, query; assert `project_id IS NULL`.

Note: `project_id` is derived at the `push_and_persist` call site (R-NEW-1 design decision) and carried in `MemoryEvent.project_id`. The worker does NOT re-derive it — these tests confirm the value flows through correctly to the SQL row.

Verify (must FAIL): if the SQL INSERT is missing the `project_id` column mapping.

### T17-impl

If `project_id` is not being written to the SQL row correctly, fix the INSERT statement in `store_insert.rs`. Confirm `NewMemoryRecord.project_id` maps to the `project_id` column. No new code if already correct — this is a verification task.

Verify: T17 tests PASS + all prior PASS.

---

### T18-test

Write failing test for `send + sync` compile-time assertions in `src-tauri/src/memory/events.rs` (REQ-STATE-003).

This is a compile-time-only task. The `const _: fn()` block must be present and active (NOT commented out).

Verify (must FAIL): if the const block references types not yet defined or has a bounds violation — force this by temporarily removing a `Send` impl and confirming the block catches it, then restore.

### T18-impl

Ensure the full compile-time Send + Sync assertion block from spec REQ-STATE-003 is active in `events.rs` (or `worker.rs`) per design §3.1 and §9. Covers `MemoryEvent: Send + Sync` and `UnboundedSender<MemoryEvent>: Send`.

Verify: `cargo check --manifest-path src-tauri/Cargo.toml` succeeds with the const block active.

---

### T19-test

Write the 5-turn end-to-end integration test in `src-tauri/src/memory/worker.rs::tests` (REQ-TEST-001).

Test function name (MUST match spec):
- `e2e_5_turn_chat_produces_5_rows` — full sequence per spec REQ-TEST-001:
  1. `MemoryStore::new_for_test().await` + `FakeEmbedder`.
  2. `spawn_test_worker(store.clone(), embedder).await`.
  3. Send 5 events: `Role::User`, `Role::Assistant` (content non-empty), `Role::Tool` (is_error=false), `Role::User`, `Role::Assistant`. Each with `timestamp_ms` incrementing by 1 to enforce temporal order.
  4. Drop sender; await handle via `tokio::time::timeout(Duration::from_secs(2), handle)`.
  5. Assert: `SELECT COUNT(*) FROM memory` = 5.
  6. Assert: `SELECT COUNT(*) FROM memory_vectors` = 5.
  7. Assert: `SELECT agent_id FROM memory ORDER BY created_at ASC` — all 5 equal `"agent-test"`.
  8. Assert: `SELECT rowid FROM memory_vectors ORDER BY rowid ASC` matches the 5 memory row IDs.
  9. Run the SAME sequence with `project_id: Some("proj-hash")` in all events; assert `project_id IS NOT NULL` for all rows.
  10. Run again with `project_id: None`; assert `project_id IS NULL` for all rows.

NO `tokio::time::sleep` in this test.

Verify (must FAIL): if any field mapping is wrong or row count is off.

### T19-impl

No new production code unless T19-test reveals a bug. Fix any field-mapping or transaction bug surfaced by the E2E test.

Verify: `e2e_5_turn_chat_produces_5_rows` PASS + all prior PASS.

---

### T20-test

Write failing tests for the REQ-TEST-002 mandatory failure-mode coverage not yet covered by prior test tasks.

New test function names from spec REQ-TEST-002 table (not already covered above):
- `worker::tests::system_role_never_reaches_worker` — call `MemoryEvent::from_message` with a `Role::System` message; assert it returns `None` (the filter in `from_message` prevents the event from reaching the channel).
- `worker::tests::empty_assistant_tool_call_not_ingested` — `MemoryEvent::from_message` with `Role::Assistant`, `content=""`, `tool_calls=Some(...)` → returns `None`.

(All other REQ-TEST-002 entries are covered by T15, T08, T10, T12 tests above.)

Verify (must FAIL): compile error if `MemoryEvent::from_message` is not importable in this context, or assertion fails if the filter logic is wrong.

### T20-impl

No new production code expected. If `from_message` filter is missing a case, fix it in `events.rs`. This is primarily a verification that the filter contract from T02-impl is correct.

Verify: T20 tests PASS + all prior PASS.

---

### T21-gate-regression

Final regression gate. Satisfies REQ-TEST-003.

- Run `cargo test --lib --manifest-path src-tauri/Cargo.toml`.
- Assert: count = 249 baseline + N Phase 3 new, ZERO failures, ZERO regressions.
- Run `cargo clippy --lib --manifest-path src-tauri/Cargo.toml -- -D warnings`.
- Assert: ZERO new clippy warnings introduced by Phase 3 modules (`memory/`, `commands/chat.rs`, `state.rs`, `lib.rs`).
- Run `cargo check --manifest-path src-tauri/Cargo.toml`.
- Assert: clean compile.
- NO `cargo build` invoked at any point.

If any test fails or any clippy warning fires, fix before marking Phase 3 done. Gate is BLOCKING for Phase 4 (`rag-retrieval`) start.

---

## Out of scope (do NOT do here)

- Code-block AST extraction in chat messages (Phase 4).
- FTS5 fallback for cold-start (Phase 4).
- `importance` marker-word heuristic beyond the numeric 5.0 / 6.0 defaults (Phase 6).
- `MemoryStore::search_similar` retrieval API (Phase 4).
- `LANGUAGE_TSX` separate parser slot for `.tsx` files (Phase 4 file-watcher feeder — R-NEW-6).
- Any Angular / TypeScript changes (Phase 3 is Rust/Tauri only).
- Any `cargo build` invocation (user CLAUDE.md forbids it).
- `feature = "test-helpers"` for `FakeEmbedder` cross-crate use (deferred).
- Reindex / migration tooling (Phase X).
