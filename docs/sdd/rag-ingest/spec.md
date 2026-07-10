# Phase 3 — rag-ingest — Spec

> **Status**: normative contract. Derived from `docs/sdd/rag-ingest/proposal.md` (locked decisions D1–D8) and corroborated by `docs/sdd/rag-ingest/explore.md`. Every requirement below is normative. RFC 2119 keywords (MUST / SHOULD / MAY) apply. Each requirement maps to ≥1 GIVEN/WHEN/THEN scenario and ≥1 failing test (red) that the apply phase makes green.
>
> **Baseline**: Phase 1 + Phase 2 = 249 passing tests. Phase 3 MUST NOT regress this count.

---

## Scope summary

Phase 3 wires the producer side of the RAG pipeline: a `MemoryEvent` struct emitted from `push_and_persist`, a `memory_worker` async task consuming those events, a chunker selector with chat and code backends, and a typed `MemoryStore::insert_memory_with_vector` API that atomically writes both `memory` and `memory_vectors` rows. No retrieval, no UI, no new Tauri commands.

---

## Requirements

### REQ-EVT-001 — `MemoryEvent` struct definition

**The system MUST** define, in `src-tauri/src/memory/events.rs`, a struct `MemoryEvent` (visibility `pub(crate)` — same-crate usage only) with these exact fields and types:

```rust
pub(crate) struct MemoryEvent {
    pub agent_id:     String,          // Agent.id (UUID-format string)
    pub project_id:   Option<String>,  // derived from agent.project_path via derive_project_id, or None
    pub role:         Role,            // from models/chat.rs — User, Assistant, Tool
    pub content:      String,          // ChatMessage.content (owned)
    pub is_error:     Option<bool>,    // ChatMessage.is_error (set only for Role::Tool)
    pub source:       MemorySource,    // enum variant — see REQ-EVT-002
    pub timestamp_ms: u64,             // ChatMessage.timestamp_ms
}
```

All fields MUST be owned (no borrows). The struct MUST be `Send + 'static` so it can cross the `tokio::mpsc::UnboundedSender` boundary without lifetime constraints. Per proposal §D1 and §D8: `Role::System` is never placed in a `MemoryEvent`; the worker MUST treat receiving one as `unreachable!()`.

#### Scenarios

- GIVEN any `MemoryEvent` value, WHEN it is moved into a `tokio::spawn(async move { ... })` block, THEN it compiles without `!Send` or lifetime errors.
- GIVEN a `MemoryEvent { role: Role::Tool, is_error: Some(true), content: "failed", .. }`, WHEN its fields are inspected, THEN `role` is `Role::Tool` and `is_error` is `Some(true)`.
- GIVEN a `MemoryEvent { role: Role::User, is_error: None, .. }`, WHEN its `is_error` field is accessed, THEN it is `None`.

#### Red tests

- `memory::events::tests::memory_event_is_send`
- `memory::events::tests::memory_event_fields_round_trip`

---

### REQ-EVT-002 — `MemorySource` enum

**The system MUST** define, in `src-tauri/src/memory/events.rs`, a public enum `MemorySource`:

```rust
pub(crate) enum MemorySource {
    Chat,        // Chat command — any Role::User, Role::Assistant, Role::Tool message
    Compaction,  // Reserved for Phase 7 — try_auto_compact emits session_summary
    CodeFile,    // Reserved for Phase 5+ — file watcher emits code chunks
}
```

`MemorySource` MUST be `Debug + Clone + Copy + PartialEq + Eq`.

**Layering rationale**: `source` describes WHERE the event originated (which producer), not what role the underlying message had. The `Role` distinction lives in `MemoryEvent.role` and propagates to `created_by_type`. Phase 3 producers only emit `MemorySource::Chat`; the other variants are reserved for downstream phases. The match on `MemorySource` in the worker MUST be exhaustive — `Compaction` and `CodeFile` MUST be handled (both route to code or chat chunker as appropriate) even though Phase 3 never emits them, so that future phases can fire events without touching worker.rs.

#### Scenarios

- GIVEN a `match` on `MemorySource` with three arms, WHEN all variants are listed, THEN the match compiles without warning.
- GIVEN a `MemorySource::Chat` value, WHEN `clone()` and `copy` semantics are used, THEN both compile.

#### Red tests

- `memory::events::tests::memory_source_is_copy_and_clone`
- `memory::events::tests::memory_source_exhaustive_match`

---

### REQ-EVT-003 — FIFO ordering guarantee (channel semantics)

**The producer MUST** use a `tokio::mpsc::UnboundedSender<MemoryEvent>` (per proposal §D2). The send MUST be non-blocking (`UnboundedSender::send` never blocks). FIFO order on a single consumer is guaranteed by `tokio::mpsc` semantics.

**If the receiver is dropped** (app shutdown or test teardown), the `send()` call returns `Err(_)`. The caller MUST log at `debug!` and drop the event. The caller MUST NOT panic and MUST NOT propagate the error to the chat command's return value.

#### Scenarios

- GIVEN a `MemoryEvent` sent via `UnboundedSender` then another sent immediately after, WHEN the receiver processes both, THEN they arrive in the order sent (FIFO).
- GIVEN a receiver that has been dropped, WHEN `memory_tx.send(event)` is called, THEN the function returns without panicking and logs at `debug!` level.

#### Red tests

- `memory::events::tests::sender_dropped_does_not_panic`

---

### REQ-WRK-001 — Worker spawn site

**The `memory_worker` MUST** be spawned exactly once, in `src-tauri/src/lib.rs::run()`, after the call to `start_embedder_bootstrap` (per proposal §module-layout). The spawn MUST use `tauri::async_runtime::spawn`. The worker receives the `tokio::mpsc::UnboundedReceiver<MemoryEvent>`, an `Arc<MemoryStore>`, and an `Arc<OnceCell<Arc<dyn Embedder>>>`.

The public entry point is `pub fn spawn_memory_worker(rx: UnboundedReceiver<MemoryEvent>, store: Arc<MemoryStore>, embedder: Arc<OnceCell<Arc<dyn Embedder>>>) -> JoinHandle<()>` in `src-tauri/src/memory/worker.rs`.

#### Scenarios

- GIVEN the app setup hook in `lib.rs`, WHEN `run()` executes, THEN a single `memory_worker` task is spawned after `start_embedder_bootstrap`.
- GIVEN `spawn_memory_worker` is called with valid arguments, WHEN it returns, THEN a `JoinHandle<()>` is returned (the task is running in the background).

#### Red tests

- `memory::worker::tests::spawn_memory_worker_returns_join_handle`

---

### REQ-WRK-002 — Consumer loop — event processing contract

**The worker's inner loop MUST** implement this semantics:

```
loop {
    let Some(event) = rx.recv().await else { break; };
    // process event (chunker → embed → insert); per-event errors logged, worker continues
}
// rx returned None → all senders dropped → graceful return
```

The loop MUST NOT panic on any per-event failure. It MUST continue to the next event after logging any error. Graceful exit occurs ONLY when `rx.recv()` returns `None` (i.e., all senders have been dropped — app shutdown).

#### Scenarios

- GIVEN a worker running with one sender, WHEN 5 events are sent and the sender is dropped, THEN the worker processes all 5 events and then exits gracefully (join handle resolves `Ok(())`).
- GIVEN a worker running, WHEN an event arrives that causes the chunker to return `Err(ChunkerError::ParseFailed)`, THEN the worker logs the error at `warn!` and continues processing subsequent events without panicking.
- GIVEN a worker running, WHEN the only sender is dropped with no pending events, THEN the worker exits gracefully within the timeout.

#### Red tests

- `memory::worker::tests::worker_exits_gracefully_when_all_senders_dropped`
- `memory::worker::tests::worker_continues_after_per_event_error`

---

### REQ-WRK-003 — Failure isolation — embedder not ready

**Per proposal §D4**: if `state.embedder.get()` returns `None` (embedder OnceCell not yet filled), the worker MUST:

1. Log at `debug!` (expected during cold start, not an error)
2. Drop the entire event (NO `memory` row, NO `memory_vectors` row inserted)
3. Continue the consumer loop

If `embedder.embed(...)` returns `EmbedError::NotReady` or `EmbedError::WorkerGone`, the worker MUST:

1. Log at `warn!`
2. Skip the current chunk (continue to the next chunk in the same event; if all chunks fail, no rows are inserted for this event)
3. Continue the consumer loop

The 1-to-1 `memory ↔ memory_vectors` invariant (per proposal §D4) MUST be preserved: if embedding fails for a chunk, that chunk's `memory` row MUST NOT be inserted.

#### Scenarios

- GIVEN a worker where `AppState.embedder` cell is `None`, WHEN a `MemoryEvent` is received, THEN zero rows exist in `memory` and zero in `memory_vectors` after processing.
- GIVEN a worker with an embedder that always returns `EmbedError::NotReady`, WHEN a `MemoryEvent` is received, THEN zero rows are inserted, and the worker loop continues to the next event.
- GIVEN a worker with an embedder that returns `EmbedError::WorkerGone` on one chunk and `Ok(vec)` on the next chunk of a different event, WHEN both events are processed, THEN the first event produces zero rows and the second produces one row.

#### Red tests

- `memory::worker::tests::embedder_cell_empty_no_rows_inserted`
- `memory::worker::tests::embedder_not_ready_no_rows_inserted`
- `memory::worker::tests::embedder_worker_gone_skips_chunk_continues`

---

### REQ-WRK-004 — Failure isolation — store write failure

**If `MemoryStore::insert_memory_with_vector` returns `Err(_)`**, the worker MUST:

1. Log at `warn!` with the error details
2. Continue the consumer loop (the worker MUST NOT stop or panic)

The worker's liveness invariant: a single SQLite write failure MUST NOT kill the worker.

#### Scenarios

- GIVEN a worker with a store that returns `Err(MemoryStoreError::Sqlite(_))` for every insert, WHEN 3 events are sent, THEN the worker logs 3 warnings and remains alive (join handle has not resolved).
- GIVEN a worker that experienced a failed insert, WHEN a subsequent event arrives with a functioning store, THEN the insert succeeds normally.

#### Red tests

- `memory::worker::tests::store_write_failure_worker_stays_alive`

---

### REQ-WRK-005 — `memory_type` and `created_by_type` derivation

**The worker MUST** derive the following fields from the incoming `MemoryEvent` before calling the insert API (per proposal §data-flow):

| Role + is_error | `memory_type` | `created_by_type` |
|---|---|---|
| `Role::User` | `"chat_message"` | `"user"` |
| `Role::Assistant` (content non-empty) | `"chat_message"` | `"agent"` |
| `Role::Tool` + `is_error == Some(true)` | `"tool_error"` | `"system"` |
| `Role::Tool` + `is_error == Some(false)` or `None` | `"chat_message"` | `"system"` |
| `Role::System` | `unreachable!()` | N/A |

`created_by_type` values are constrained by `docs/6-improvements-RAG.md` line 164 to one of `'user' | 'agent' | 'system' | 'trajectory_miner'`. Tool results use `'system'` because the tool execution (not the agent) authored the textual content; the agent only authored the tool-call arguments. This distinction is load-bearing for the Phase 6 sycophancy guard (it MUST NOT confuse tool outputs with the agent's own statements).

`created_by_ref` MUST be set to `event.agent_id.clone()` in all cases (memory is still OWNED by the agent for retrieval scoping).

`importance` defaults: `"tool_error"` → `6.0`; `"chat_message"` → `5.0`. Both are overrideable by the marker-word heuristic (Phase 6 concern; Phase 3 sets the numeric default and no more).

`scope` MUST always be `"agent"` (per proposal §D3; cross-scope promotion is Phase 12).

#### Scenarios

- GIVEN a `Role::User` event, WHEN the worker processes it, THEN the inserted `memory` row has `memory_type = 'chat_message'` and `created_by_type = 'user'`.
- GIVEN a `Role::Assistant` event with non-empty content, WHEN processed, THEN the row has `memory_type = 'chat_message'` and `created_by_type = 'agent'`.
- GIVEN a `Role::Tool` event with `is_error = Some(true)`, WHEN processed, THEN the `memory` row has `memory_type = 'tool_error'`, `created_by_type = 'system'`, and `importance ≥ 6.0`.
- GIVEN a `Role::Tool` event with `is_error = Some(false)`, WHEN processed, THEN the `memory` row has `memory_type = 'chat_message'` and `created_by_type = 'system'`.

#### Red tests

- `memory::worker::tests::user_role_produces_chat_message_type_user_creator`
- `memory::worker::tests::assistant_role_produces_agent_creator`
- `memory::worker::tests::tool_error_produces_tool_error_type_system_creator_importance_6`
- `memory::worker::tests::tool_success_produces_chat_message_type_system_creator`

---

### REQ-CHK-001 — Chunker selector — `select_chunker`

**The system MUST** provide, in `src-tauri/src/memory/chunker/mod.rs`, a public function:

```rust
pub(crate) fn select_chunker(
    role: Role,
    is_error: Option<bool>,
    content: &str,
) -> ChunkerKind
```

where `ChunkerKind` is an enum:

```rust
pub(crate) enum ChunkerKind {
    Chat,
    Code,
    /// Defensive arm — `MemoryEvent::from_message` should already have filtered
    /// these out, but the worker treats `Skip` as "drop without insert".
    Skip,
}
```

The routing rules (per proposal §D7 and explore.md §pending-decisions-a):

| Role + is_error + content | `ChunkerKind` | Notes |
|---|---|---|
| `Role::User` (non-empty content) | `Chat` | user turn |
| `Role::Assistant` (non-empty content) | `Chat` | assistant prose turn |
| `Role::Assistant` (empty content) | `Skip` | tool-call-only announcement — no prose to embed |
| `Role::Tool` + `is_error` any value | `Chat` | tool result treated as prose turn (per D7, no AST in Phase 3) |
| `Role::System` | `Skip` | LOCKED D8 — defensive arm; never reaches the worker |

**Phase 3 producers only emit `Role::User`, `Role::Assistant`, `Role::Tool` events**, so `ChunkerKind::Code` is unreachable from Phase 3 alone. The `Code` variant exists so Phase 5+ producers (file watcher with `MemorySource::CodeFile`) can dispatch to the code chunker without modifying `select_chunker`'s signature — that producer wraps the route around `select_chunker` (e.g., `if source == MemorySource::CodeFile { ChunkerKind::Code } else { select_chunker(...) }`). The hook lives at the worker call site, NOT inside `select_chunker` itself.

`select_chunker` MUST be **pure, total, and deterministic** — no side effects, no `Result`. Taking `content: &str` (even if currently only inspected for emptiness) reserves the hook for Phase 4 markdown-fence AST detection without a signature change.

Per design.md §3.2: the worker uses `match` on `ChunkerKind` with all three arms (`Skip` → `continue` in the consumer loop).

#### Scenarios

- GIVEN `Role::User` with non-empty content, WHEN `select_chunker` is called, THEN it returns `ChunkerKind::Chat`.
- GIVEN `Role::Assistant` with non-empty content, WHEN `select_chunker` is called, THEN it returns `ChunkerKind::Chat`.
- GIVEN `Role::Assistant` with empty content, WHEN `select_chunker` is called, THEN it returns `ChunkerKind::Skip`.
- GIVEN `Role::Tool` with any `is_error`, WHEN `select_chunker` is called, THEN it returns `ChunkerKind::Chat`.
- GIVEN `Role::System` with any content, WHEN `select_chunker` is called, THEN it returns `ChunkerKind::Skip` (defensive).

#### Red tests

- `memory::chunker::tests::select_chunker_user_routes_to_chat`
- `memory::chunker::tests::select_chunker_assistant_with_content_routes_to_chat`
- `memory::chunker::tests::select_chunker_assistant_empty_content_routes_to_skip`
- `memory::chunker::tests::select_chunker_tool_routes_to_chat`
- `memory::chunker::tests::select_chunker_system_routes_to_skip`

---

### REQ-CHK-002 — `Chunk` output struct

**The system MUST** define, in `src-tauri/src/memory/chunker/mod.rs`, a struct:

```rust
pub(crate) struct Chunk {
    pub text:        String,
    pub token_count: usize,
    /// Optional context header (e.g. `"fn my_function"` for code chunks).
    /// `None` for chat chunks. Reserved for Phase 4 retrieval re-ranking
    /// without a schema change.
    pub header:      Option<String>,
}
```

`Chunk` MUST implement `Debug + Clone`.

**Speaker derivation**: there is NO `speaker` field on `Chunk`. The "speaker" of a chunk is the `Role` of the originating `MemoryEvent`, available at insert time via `MemoryEvent.role`. This avoids redundant data on the chunk and matches design.md §3.2.

#### Scenarios

- GIVEN a chat chunker output, WHEN each chunk is inspected, THEN `header` is `None`.
- GIVEN a code chunker output for a Rust function, WHEN the chunk is inspected, THEN `header` is `Some("fn <name>")`.

#### Red tests

- `memory::chunker::tests::chat_chunk_has_no_header`
- `memory::chunker::tests::code_chunk_has_header`

---

### REQ-CHK-003 — Chat chunker — turn-based, speaker-boundary hard rule

**The system MUST** provide, in `src-tauri/src/memory/chunker/chat.rs`, a chat chunker with this contract (per proposal §D7):

- Input: `content: &str`, `speaker: SpeakerKind`
- Output: `Result<Vec<Chunk>, ChunkerError>`
- The speaker boundary is a **hard rule**: all chunks produced from a single call share the same `speaker` (the input `speaker`). A caller invoking the chat chunker once per turn guarantees no cross-speaker chunks.
- Token cap: **480 tokens** per chunk (per proposal §D6, safety margin under the 512 model max).
- **No overlap** between adjacent chunks: `chunk[i]` ends exactly where `chunk[i+1]` begins (no repeated tokens).
- If `content` is empty, the output MUST be an empty `Vec<Chunk>` (NOT an error).
- Token counting MUST use the `tokenizers` 0.22.2 tokenizer loaded from the model path (`memory/model_download::tokenizer_path(base_dir) -> PathBuf` helper, per proposal §R8). If the tokenizer file is absent, the chunker MUST fall back to a **byte-length heuristic** (cap at 2000 bytes ≈ 480 tokens) and log `warn!`. The tokenizer MUST be loaded once and cached (e.g., `std::sync::LazyLock` or `once_cell::sync::Lazy`).

#### Scenarios

- GIVEN a `content` of 100 tokens, WHEN chunked, THEN the output is exactly one `Chunk` with `token_count ≤ 100` and `speaker == input_speaker`.
- GIVEN a `content` of 1000 tokens (single speaker turn), WHEN chunked, THEN the output has ≥ 2 chunks, each with `token_count ≤ 480`, and no chunk contains tokens from a different turn.
- GIVEN two consecutive calls with different `speaker` values, WHEN their outputs are concatenated, THEN no chunk in the first output contains tokens from the second (speaker boundary respected per-call).
- GIVEN `content = ""`, WHEN chunked, THEN the output is `Ok(vec![])`.
- GIVEN the tokenizer file is absent, WHEN chunking a 3000-byte string, THEN the chunker returns chunks each ≤ 2000 bytes, logs `warn!`, and does NOT panic.
- GIVEN a content that is exactly 480 tokens, WHEN chunked, THEN the output is exactly one chunk with `token_count == 480`.

#### Red tests

- `memory::chunker::chat::tests::speaker_boundary_respected_single_turn`
- `memory::chunker::chat::tests::token_cap_480_produces_multiple_chunks`
- `memory::chunker::chat::tests::no_overlap_adjacent_chunks`
- `memory::chunker::chat::tests::empty_content_returns_empty_vec`
- `memory::chunker::chat::tests::tokenizer_absent_falls_back_to_byte_heuristic`
- `memory::chunker::chat::tests::exactly_480_tokens_is_single_chunk`

---

### REQ-CHK-004 — Code chunker — AST-based with degraded fallback

**The system MUST** provide, in `src-tauri/src/memory/chunker/code.rs`, a code chunker with this contract (per proposal §scope):

- Input: `content: &str`, `language: Option<&str>` (derived from file extension)
- Output: `Result<Vec<Chunk>, ChunkerError>`
- Supported languages (per proposal §scope): Rust (`.rs`), TypeScript (`.ts`, `.tsx`), Python (`.py`)
- For supported languages: parse via `tree-sitter`, emit one chunk per top-level function/method/class body. Each chunk MUST carry `header: Some("<kind> <name>")` (e.g., `"fn my_function"`, `"class MyClass"`).
- Same 480-token cap applies (per proposal §D6); a node whose text exceeds 480 tokens MUST be further split via the shared `TokenSplitter` (see REQ-CHK-005). Resulting sub-chunks inherit the same `header`.
- For **unknown or `None` languages**: fall back to the chat chunker (NOT a panic; NOT an error return). The fallback MUST use `SpeakerKind::Agent` as the default speaker.
- If `tree-sitter` parse produces a parse error node: fall back to the chat chunker for the ENTIRE input (per proposal §R6; log `warn!`). This is `ChunkerError::ParseFailed` treated as a degraded path, NOT a propagated error.

#### Scenarios

- GIVEN a Rust source file with two top-level functions, WHEN chunked, THEN the output has 2 chunks, each with `header` of the form `"fn <name>"`.
- GIVEN a TypeScript file with a class, WHEN chunked, THEN the output has ≥1 chunk with `header` containing `"class <name>"`.
- GIVEN a Python file, WHEN chunked, THEN chunks have `header` of the form `"def <name>"` or `"class <name>"`.
- GIVEN an unknown extension (e.g., `.txt`), WHEN the code chunker is called, THEN it falls back to the chat chunker and returns `Ok(Vec<Chunk>)` without an `header`.
- GIVEN malformed Rust source that tree-sitter cannot parse cleanly, WHEN chunked, THEN the code chunker falls back to the chat chunker and does NOT return `Err`.
- GIVEN a Rust function whose body exceeds 480 tokens, WHEN chunked, THEN all resulting sub-chunks have `token_count ≤ 480` and each carries the same `header`.

#### Red tests

- `memory::chunker::code::tests::rust_two_functions_produces_two_chunks_with_headers`
- `memory::chunker::code::tests::typescript_class_produces_chunk_with_header`
- `memory::chunker::code::tests::python_function_produces_chunk_with_header`
- `memory::chunker::code::tests::unknown_extension_falls_back_to_chat_chunker`
- `memory::chunker::code::tests::malformed_rust_falls_back_no_panic`
- `memory::chunker::code::tests::large_function_split_respects_token_cap`

---

### REQ-CHK-005 — Shared `TokenSplitter` helper

**The system MUST** factor a `pub(crate) fn token_split(text: &str, cap: usize) -> Vec<String>` helper (or equivalent struct) in `src-tauri/src/memory/chunker/mod.rs`. This helper:

- Splits `text` into segments each with `token_count ≤ cap`
- Has NO knowledge of speakers or AST headers
- Is consumed by both the chat chunker (after speaker boundary) and the code chunker fallback
- Uses the same tokenizer / byte-length heuristic as REQ-CHK-003

Per proposal §R6: the shared piece MUST NOT know about speakers or AST structure. Both chunker implementations delegate splitting to this helper.

#### Scenarios

- GIVEN a 1000-token string and `cap = 480`, WHEN `token_split` is called, THEN the output has ≥ 2 segments and each has `≤ 480` tokens.
- GIVEN a 100-token string and `cap = 480`, WHEN `token_split` is called, THEN the output has exactly 1 segment.

#### Red tests

- `memory::chunker::tests::token_split_respects_cap`
- `memory::chunker::tests::token_split_short_input_single_segment`

---

### REQ-CHK-006 — `ChunkerError` enum

**The system MUST** define, in `src-tauri/src/memory/chunker/mod.rs` (or a sibling `error.rs`), a `pub(crate) enum ChunkerError` covering at minimum:

```rust
pub(crate) enum ChunkerError {
    ParseFailed(String),   // tree-sitter returned an ERROR node
    TokenizerInit(String), // tokenizer::Tokenizer::from_file failed AND byte fallback also failed (should be unreachable in practice)
}
```

`ChunkerError` MUST implement `std::error::Error` via `thiserror`. It MUST NOT use `anyhow`.

#### Scenarios

- GIVEN `ChunkerError::ParseFailed("syntax error at line 3".into())`, WHEN `format!("{}", err)` is called, THEN the message contains `"ParseFailed"` or the inner string.
- GIVEN any `ChunkerError`, WHEN used with `?` in a function returning `Result<_, Box<dyn std::error::Error>>`, THEN it compiles (i.e., it implements `std::error::Error`).

#### Red tests

- `memory::chunker::tests::chunker_error_implements_error_trait`
- `memory::chunker::tests::chunker_error_parse_failed_displays_message`

---

### REQ-INS-001 — `MemoryStore::insert_memory_with_vector` — signature

**The system MUST** add to `MemoryStore` (in `src-tauri/src/memory/db.rs` or a sibling file `store_insert.rs` re-exported from `db.rs`) the method:

```rust
pub async fn insert_memory_with_vector(
    &self,
    record: &NewMemoryRecord,
    embedding: &[f32],
) -> Result<i64, MemoryStoreError>
```

where `i64` is the `AUTOINCREMENT` rowid of the newly inserted `memory` row, and `NewMemoryRecord` is a struct (see REQ-INS-002).

`MemoryStoreError` is a new `pub enum` in `src-tauri/src/memory/error.rs` (or a sibling) implementing `std::error::Error` via `thiserror`. It MUST have at minimum:

```rust
pub enum MemoryStoreError {
    Sqlite(rusqlite::Error),
    Connection(tokio_rusqlite::Error),
}
```

(Phase 3 does NOT reuse `MemoryError` directly to keep the INSERT surface clearly scoped; `MemoryStoreError` may implement `From<MemoryError>` if useful.)

#### Scenarios

- GIVEN a valid `NewMemoryRecord` and a 384-dim embedding, WHEN `insert_memory_with_vector` is awaited, THEN it returns `Ok(id)` where `id > 0`.
- GIVEN the connection is closed before the call, WHEN `insert_memory_with_vector` is awaited, THEN it returns `Err(MemoryStoreError::Connection(_))` or `Err(MemoryStoreError::Sqlite(_))`.

#### Red tests

- `memory::db::tests::insert_memory_with_vector_returns_rowid`
- `memory::db::tests::insert_memory_with_vector_returns_err_on_closed_conn`

---

### REQ-INS-002 — `NewMemoryRecord` — input struct

**The system MUST** define a `pub struct NewMemoryRecord` with these fields (mapping to `memory` table columns from `V1__init.sql`):

```rust
pub struct NewMemoryRecord {
    pub scope:             String,        // always "agent" for Phase 3
    pub agent_id:          String,        // NOT NULL when scope='agent' (CHECK constraint)
    pub project_id:        Option<String>,// NULL for no-project agents (D3)
    pub content:           String,        // chunk text
    pub memory_type:       String,        // "chat_message" | "tool_error"
    pub source:            String,        // "chat" | "tool_result" | "code_file"
    pub created_at:        i64,           // epoch milliseconds (ChatMessage.timestamp_ms cast)
    pub created_by_type:   String,        // "user" | "agent"
    pub created_by_ref:    String,        // agent_id
    pub source_session_id: Option<String>,// current session id if available; else None
    pub last_modified_at:  i64,           // same as created_at on insert
    pub last_accessed_at:  i64,           // same as created_at on insert
    pub importance:        f64,           // 5.0 default; 6.0 for tool_error
}
```

Columns not in `NewMemoryRecord` use their schema defaults (`tier = 3`, `status = 'active'`, `update_count = 0`, `access_count = 0`, `superseded_by = NULL`, `dispute_count = 0`, `disputed_against = NULL`, `last_disputed_at = NULL`, `deleted_at = NULL`, `last_modified_by = NULL`).

#### Scenarios

- GIVEN a `NewMemoryRecord` with `scope = "agent"` and `agent_id = "agent-123"`, WHEN inserted via `insert_memory_with_vector`, THEN the `memory` row satisfies the CHECK constraint (`scope='agent' AND agent_id IS NOT NULL`).
- GIVEN a `NewMemoryRecord` with `scope = "agent"`, `agent_id = "agent-123"`, `project_id = None`, WHEN inserted, THEN the row passes the CHECK constraint (per proposal §D3).

#### Red tests

- `memory::db::tests::insert_agent_scope_no_project_passes_check`
- `memory::db::tests::insert_agent_scope_with_project_passes_check`

---

### REQ-INS-003 — Atomicity: `memory` + `memory_vectors` in one transaction

**The insert MUST** be atomic: both the `memory` row and the corresponding `memory_vectors` row MUST be written in a single `tokio_rusqlite::Connection::call` closure (no second `.call(...)` for the vector insert). Inside the closure, the implementation MUST:

1. `INSERT INTO memory (...) RETURNING id` to get the rowid
2. Serialize `embedding: &[f32]` to little-endian IEEE 754 bytes
3. `INSERT INTO memory_vectors(rowid, embedding) VALUES (?, ?)` using the rowid from step 1

Either BOTH rows are present after the call, or NEITHER is (per `tokio_rusqlite` single-connection serialization semantics — explicit `BEGIN`/`COMMIT` MAY be used if needed, but the single `.call(...)` boundary already provides serialization).

Per proposal §D4: idempotency is NOT required. There is no dedup logic in Phase 3.

#### Scenarios

- GIVEN a valid insert call, WHEN `insert_memory_with_vector` returns `Ok(id)`, THEN both `SELECT * FROM memory WHERE id=?` and `SELECT rowid FROM memory_vectors WHERE rowid=?` return exactly one row with the same id.
- GIVEN a simulated fault that makes the `memory_vectors` insert fail (e.g., wrong dimension count for the vec0 table), WHEN the insert runs, THEN NEITHER the `memory` row NOR the `memory_vectors` row is present (both absent — atomicity upheld).

#### Red tests

- `memory::db::tests::both_rows_present_after_successful_insert`
- `memory::db::tests::vector_insert_failure_leaves_no_memory_row`

---

### REQ-INS-004 — `MemoryStoreError` implements `std::error::Error`

**`MemoryStoreError` MUST** implement `std::error::Error` via `thiserror` and MUST NOT use `anyhow`. It MUST be `Send + Sync` so it can be used with `?` in async functions that require `Send` futures.

#### Scenarios

- GIVEN `MemoryStoreError::Sqlite(rusqlite::Error::InvalidQuery)`, WHEN `format!("{}", err)` is called, THEN it produces a non-empty message.
- GIVEN `Box<dyn std::error::Error + Send + Sync>` as the return type, WHEN `MemoryStoreError` is converted via `?`, THEN it compiles.

#### Red tests

- `memory::error::tests::memory_store_error_is_send_sync`
- `memory::error::tests::memory_store_error_displays_message`

---

### REQ-INT-001 — `ChatMessage.is_error` field addition

**The system MUST** add to `ChatMessage` in `src-tauri/src/models/chat.rs` the field (per proposal §D1):

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub is_error: Option<bool>,
```

The `#[serde(default)]` attribute ensures old persisted JSON (without this key) deserializes as `is_error: None`. The `skip_serializing_if = "Option::is_none"` attribute ensures re-serialized JSON does not gain a `null` field on messages where `is_error` was not set.

The field MUST default to `None` in all existing `ChatMessage { ... }` construction sites in `commands/chat.rs` (lines 413, 622, 652). Only at **line 1049** (the `Role::Tool` push) MUST `is_error` be set to `Some(is_error)` (using the local `is_error: bool` variable that is already in scope at that point).

#### Scenarios

- GIVEN a JSON string `{"role":"user","content":"hi"}` (no `is_error` key), WHEN deserialized into `ChatMessage`, THEN `is_error == None`.
- GIVEN a `ChatMessage { role: Role::Tool, content: "ok", is_error: Some(false), .. }`, WHEN serialized to JSON, THEN the output contains `"is_error":false`.
- GIVEN a `ChatMessage { role: Role::User, content: "hi", is_error: None, .. }`, WHEN serialized to JSON, THEN the output does NOT contain the `"is_error"` key.
- GIVEN the tool result loop at `chat.rs:1049`, WHEN `is_error` is `true` (tool failed), THEN the constructed `ChatMessage` has `is_error: Some(true)`.

#### Red tests

- `models::chat::tests::is_error_field_defaults_to_none_on_missing_key`
- `models::chat::tests::is_error_field_serializes_false_when_some_false`
- `models::chat::tests::is_error_field_omitted_when_none`
- `models::chat::tests::is_error_field_some_true_serializes_correctly`

---

### REQ-INT-002 — `push_and_persist` emits `MemoryEvent` after persist

**The `push_and_persist` function in `src-tauri/src/commands/chat.rs` MUST** be updated to accept the `memory_tx` sender and, after the existing `session.append_message` succeeds, send a `MemoryEvent` to `state.memory_tx`.

The send MUST occur AFTER the successful in-memory push + disk persist (so a `MemoryEvent` is only emitted for messages that actually persisted). The send MUST be fire-and-forget via `UnboundedSender::send`. On send failure (receiver dropped), log at `debug!` and continue — do NOT propagate the error.

**Skip rules** (per proposal §D8): the worker MUST NOT receive events for:
- `Role::System` messages (not stored in conversations, so this case never arises in practice per D8; defensive check still REQUIRED in `push_and_persist`)
- `Role::Assistant` messages with empty `content` AND non-empty `tool_calls` (the tool-call-only turn has no prose to index)

All other roles (`Role::User`, `Role::Assistant` with content, `Role::Tool`) MUST emit a `MemoryEvent`.

#### Scenarios

- GIVEN a `Role::User` message pushed via `push_and_persist`, WHEN the function returns, THEN a `MemoryEvent` with `role = Role::User` is available on the receiver.
- GIVEN a `Role::Assistant` message with `content = ""` and `tool_calls = Some(...)`, WHEN pushed, THEN NO `MemoryEvent` is sent.
- GIVEN a `Role::Tool` message with `is_error = Some(true)`, WHEN pushed, THEN a `MemoryEvent` with `is_error = Some(true)` is sent.
- GIVEN the memory_tx receiver is already dropped, WHEN `push_and_persist` is called with any message, THEN the function returns normally without panicking.

#### Red tests

- `commands::chat::tests::push_and_persist_emits_event_for_user_role`
- `commands::chat::tests::push_and_persist_skips_event_for_empty_assistant_tool_call`
- `commands::chat::tests::push_and_persist_emits_event_for_tool_with_is_error`
- `commands::chat::tests::push_and_persist_dropped_receiver_no_panic`

---

### REQ-INT-003 — `derive_project_id` call-site adaptation

**The worker MUST** derive `project_id` as follows when building a `NewMemoryRecord` (per proposal §R3, §D3):

```rust
let project_id: Option<String> = agent.project_path
    .as_deref()
    .filter(|s| !s.is_empty())
    .map(|s| derive_project_id(Path::new(s)));
```

`derive_project_id` from `memory/identity.rs` takes `&Path` (NOT `&str`). The call site MUST adapt with `Path::new(s)`. `identity.rs` MUST NOT be modified.

When `project_path` is `None` or an empty string, `project_id` MUST be `None`, and `scope` stays `"agent"` (which satisfies the CHECK constraint with only `agent_id IS NOT NULL`).

#### Scenarios

- GIVEN an agent with `project_path = Some("/home/user/my-app")`, WHEN the worker derives `project_id`, THEN it equals `derive_project_id(Path::new("/home/user/my-app"))`.
- GIVEN an agent with `project_path = None`, WHEN the worker derives `project_id`, THEN `project_id` is `None`.
- GIVEN an agent with `project_path = Some("")`, WHEN the worker derives `project_id`, THEN `project_id` is `None`.

#### Red tests

- `memory::worker::tests::project_id_derived_from_path`
- `memory::worker::tests::project_id_none_when_no_path`

---

### REQ-STATE-001 — `AppState.memory_tx` field

**The system MUST** add to `AppState` in `src-tauri/src/state.rs` the field (per proposal §D2):

```rust
pub memory_tx: tokio::sync::mpsc::UnboundedSender<MemoryEvent>,
```

`AppState::new()` MUST create the unbounded channel, store the sender in `memory_tx`, and pass the receiver to `spawn_memory_worker` (called from `lib.rs::run()`). The `AppState::new()` constructor does NOT spawn the worker directly (it has no tokio context); it only creates the channel.

`memory_tx` MUST be `Clone`-able (all `UnboundedSender` instances are) so `push_and_persist`, which receives `&AppState`, can call `state.memory_tx.send(event)` directly.

#### Scenarios

- GIVEN a fresh `AppState::new()`, WHEN `state.memory_tx.send(event)` is called before the worker is spawned, THEN the send returns `Ok(())` (the channel buffers the event until the receiver is connected — unbounded).
- GIVEN two clones of `state.memory_tx`, WHEN each sends one event and the worker receives both, THEN both events arrive in the worker.

#### Red tests

- `state::tests::memory_tx_send_before_worker_buffers_event`
- `state::tests::memory_tx_clone_sends_independently`

---

### REQ-STATE-002 — Test constructors seed `memory_tx`

**`AppState::new_for_test()` and `AppState::new_for_test_async()` MUST** pre-seed `memory_tx` so that tests never deadlock on a missing receiver (per proposal §D2).

The recommended pattern is a `TestHandle` struct that bundles `AppState` with the `UnboundedReceiver<MemoryEvent>`, allowing tests to assert on emitted events:

```rust
#[cfg(test)]
pub struct MemoryTestHandle {
    pub state: AppState,
    pub memory_rx: tokio::sync::mpsc::UnboundedReceiver<MemoryEvent>,
}

#[cfg(test)]
impl AppState {
    pub async fn new_for_test_with_memory_rx() -> MemoryTestHandle { ... }
}
```

Alternatively, tests that do not assert on events MAY use `new_for_test_async()` with a no-op sink (the receiver is dropped or consumed by a drain task). Both patterns MUST NOT deadlock.

#### Scenarios

- GIVEN `AppState::new_for_test_async().await`, WHEN `state.memory_tx.send(event)` is called 3 times, THEN the sends all return `Ok(())` (unbounded channel does not block, even with no consumer).
- GIVEN `AppState::new_for_test_with_memory_rx().await`, WHEN 1 event is sent via `state.memory_tx` and `memory_rx.recv().await` is called, THEN the event is received correctly.

#### Red tests

- `state::tests::new_for_test_async_memory_tx_does_not_deadlock`
- `state::tests::new_for_test_with_memory_rx_round_trip`

---

### REQ-STATE-003 — Compile-time `Send + Sync` assertions

**The system MUST** add compile-time assertions in `src-tauri/src/memory/events.rs` (or in a new `src-tauri/src/memory/worker.rs` assertion block) for all new public types that cross task boundaries:

```rust
const _: fn() = || {
    fn _assert_send<T: Send>() {}
    fn _assert_sync<T: Sync>() {}
    _assert_send::<MemoryEvent>();
    _assert_sync::<MemoryEvent>();
    _assert_send::<tokio::sync::mpsc::UnboundedSender<MemoryEvent>>();
};
```

Per Phase 2 carry-forward pattern (Phase 2 spec REQ-EMB-002): compile-time assertions MUST exist as straight-line const-block code, NOT inside `#[test]` functions.

#### Scenarios

- GIVEN the source tree at the end of apply, WHEN `cargo check --lib` runs, THEN the const assertion block type-checks, proving `MemoryEvent: Send + Sync`.

#### Red tests

- Compile-time only: spec assertion block IS the test.

---

### REQ-TEST-001 — 5-turn end-to-end integration test

**The test suite MUST** include an end-to-end test in `memory::worker::tests` that:

1. Constructs a `MemoryStore::new_for_test().await` and a `FakeEmbedder`.
2. Creates the channel and spawns `spawn_memory_worker`.
3. Sends 5 `MemoryEvent`s (user → assistant → tool → user → assistant sequence) via the sender.
4. Drops the sender to trigger graceful shutdown, then awaits the join handle.
5. Queries `SELECT * FROM memory ORDER BY created_at ASC` and asserts exactly 5 rows in the correct temporal order.
6. Queries `SELECT rowid FROM memory_vectors` and asserts exactly 5 rowids matching the 5 memory row ids.
7. Asserts `agent_id` is correct for all rows.
8. Runs the test TWICE in the same test body: once with `project_path = Some(...)` (asserts `project_id IS NOT NULL`) and once with `project_path = None` (asserts `project_id IS NULL`).

NO `tokio::time::sleep` may appear in this test (per Phase 2 lesson — proposal §test-strategy). Channel-based signaling only.

#### Scenarios

- GIVEN 5 well-formed `MemoryEvent`s sent over the channel, WHEN the worker processes all 5 and the sender is dropped, THEN 5 `memory` rows + 5 `memory_vectors` rows exist, ordered by `created_at` ascending.

#### Red tests

- `memory::worker::tests::e2e_5_turn_chat_produces_5_rows`

---

### REQ-TEST-002 — Mandatory failure-mode coverage

**The test suite MUST** include all of the following tests as a gate for Phase 3 archive:

| Test name | Asserts |
|---|---|
| `worker::tests::embedder_cell_empty_no_rows_inserted` | `state.embedder` cell empty → 0 rows in `memory` and `memory_vectors` (REQ-WRK-003) |
| `worker::tests::tool_error_produces_tool_error_type_system_creator_importance_6` | `Role::Tool` + `is_error=true` → `memory_type='tool_error'`, `created_by_type='system'`, `importance ≥ 6.0` (REQ-WRK-005) |
| `worker::tests::tool_success_produces_chat_message_type_system_creator` | `Role::Tool` + `is_error=false` → `memory_type='chat_message'`, `created_by_type='system'` (REQ-WRK-005) |
| `worker::tests::system_role_never_reaches_worker` | `push_and_persist` skips `Role::System` — no event sent, no row (REQ-INT-002) |
| `worker::tests::empty_assistant_tool_call_not_ingested` | `Role::Assistant` with `content=""` and `tool_calls=Some(...)` → 0 rows (REQ-INT-002) |
| `chunker::chat::tests::token_cap_480_produces_multiple_chunks` | 1000-token turn → ≥2 chunks, each ≤480 tokens, no cross-speaker split (REQ-CHK-003) |
| `chunker::code::tests::rust_two_functions_produces_two_chunks_with_headers` | Rust source → 2 chunks with `header` set (REQ-CHK-004) |
| `chunker::code::tests::unknown_extension_falls_back_to_chat_chunker` | `.txt` input → chat fallback, `Ok(Vec<Chunk>)`, no panic (REQ-CHK-004) |
| `db::tests::both_rows_present_after_successful_insert` | Insert → both `memory` and `memory_vectors` rows present (REQ-INS-003) |

NO test in Phase 3 may use `tokio::time::sleep(Duration::from_secs(...))`. Use `tokio::time::timeout(Duration::from_millis(500), ...)` as a hard stop for tests that might deadlock.

#### Red tests

- All tests listed in the table above.

---

### REQ-TEST-003 — Phase 1 + Phase 2 baseline must pass

**All 249 Phase 1 + Phase 2 tests MUST** still pass after Phase 3 is applied. The `cargo test --lib` output for `src-tauri/` MUST show the Phase 1/2 baseline count (249) plus new Phase 3 tests, with ZERO failures and ZERO regressions.

`cargo build` is forbidden (user CLAUDE.md rule). Only `cargo check` and `cargo test --lib` are permitted as gate commands.

#### Scenarios

- GIVEN the final Phase 3 apply state, WHEN `cargo test --lib --manifest-path src-tauri/Cargo.toml` runs, THEN the output shows ≥249 + N_phase3 passing tests with 0 failures.

#### Red tests

- (Meta-requirement — no new red test here; Phase 1/2 tests are already green and MUST stay green.)

---

## Cross-cutting requirements

### Logging

All new code MUST use the `log` crate (already a dep at `Cargo.toml:31`, 43 call sites across 9 files; `tracing` is NOT in the dependency graph). Per-event errors in the worker: `log::warn!`. Embedder `NotReady` / cell empty: `log::debug!` (expected during cold start). Parser fallback: `log::warn!`. Send-failure (receiver dropped): `log::debug!`.

### Error types

- `ChunkerError`: `pub(crate)`, `thiserror`, no `anyhow`.
- `MemoryStoreError`: `pub`, `thiserror`, no `anyhow`.
- No `unwrap()` or `expect()` in non-test code paths. Use `?` + appropriate error variant propagation.

### Visibility

Per proposal §module-layout:
- `MemoryEvent`, `MemorySource`: `pub(crate)` (used from `commands/chat.rs` — same crate)
- `spawn_memory_worker`: `pub` (called from `lib.rs`)
- `Chunk`, `SpeakerKind`, `ChunkerKind`, `select_chunker`, `ChunkerError`: `pub(crate)`
- `NewMemoryRecord`, `MemoryStoreError`, `insert_memory_with_vector`: `pub`

### No `standalone: true` in Angular

This requirement is Rust/Tauri only — no Angular changes in Phase 3.

---

## Acceptance criteria (gate for verify phase)

- [ ] `MemoryEvent` is `Send + 'static` (compile-time assertion passes).
- [ ] `AppState.memory_tx` field present; `new_for_test_async()` and `new_for_test_with_memory_rx()` do not deadlock.
- [ ] Worker spawned in `lib.rs::run()` after `start_embedder_bootstrap`.
- [ ] Worker exits gracefully when all senders are dropped (join handle resolves).
- [ ] Embedder `None` / `NotReady` / `WorkerGone` → 0 rows inserted, worker alive (failure isolation).
- [ ] `ChatMessage.is_error` field: old JSON deserializes as `None`; new JSON omits key when `None`.
- [ ] `is_error = Some(true)` set at `chat.rs:1049` only.
- [ ] Chat chunker: empty content → `Ok([])`; 1000-token turn → ≥2 chunks, each ≤480 tokens.
- [ ] Code chunker: Rust/TS/Python → chunks with `header`; unknown ext → chat fallback.
- [ ] `insert_memory_with_vector`: both rows present after `Ok`; zero rows after error (atomicity).
- [ ] CHECK constraint: `scope='agent'`, `agent_id IS NOT NULL`, `project_id` nullable → passes.
- [ ] 5-turn E2E test: 5 `memory` rows + 5 `memory_vectors` rows, FIFO order, correct `agent_id`.
- [ ] All 249 Phase 1/2 tests still pass. Net new Phase 3 tests all pass.
- [ ] No `tokio::time::sleep(Duration::from_secs(...))` in any test.
- [ ] No `cargo build` invoked. All gating via `cargo check` + `cargo test --lib`.
- [ ] `docs/sdd/rag-ingest/archive-report.md` lists any drifted decisions before Phase 4 starts.

---

## Out of scope (deferred — NOT regressions)

| Item | Phase |
|---|---|
| Code-block AST extraction in chat messages (markdown fence parsing) | Phase 4 |
| FTS5 fallback for cold-start | Phase 4 |
| `importance` heuristic beyond marker-word defaults | Phase 6 |
| Retrieval API (`MemoryStore::search_similar`) | Phase 4 |
| Reindex / migration tooling on embedder swap | Phase X |
| Cross-scope promotion (agent → project → global) | Phase 12 |
| `feature = "test-helpers"` for `FakeEmbedder` cross-crate use | Deferred until integration tests under `tests/` dir land |
| New Tauri commands | None in Phase 3 |
