# SDD Explore — `rag-ingest` (Phase 3)

> Read-only investigation. Output of the `sdd-explore` phase, reconstructed from the
> sub-agent's summary because the Explore agent type lacks Write tooling. Faithful
> to what the sub-agent reported; if you re-run the exploration, regenerate this
> document and diff it against this one.

---

## Executive Summary

- **Current state**: Phase 1 (schema, `MemoryStore`) and Phase 2 (embedder, `FastEmbedLocal`) are fully shipped and archived. The `memory` table, `vec0` virtual tables, `MemoryRecord` trait, `MemoryStore`, `Embedder` trait, and `FakeEmbedder` are all in place. Phase 3 can write directly against these APIs without new schema work.
- **Biggest risk**: `is_error` is a **local variable scoped inside `stream_chat`** that is **not persisted** in `ChatMessage`. The plan assumes `push_and_persist` can attach it to the event, but it never reaches the `ChatMessage` struct. Phase 3 must either (a) change `push_and_persist` to accept an `is_error` flag and add it to the struct/event, or (b) derive `is_error` in the worker from content-pattern heuristics. This contradicts the plan's assumption and must be resolved before design is locked.
- **Recommended path**: Option (a) — add `is_error: bool` field to `ChatMessage` and pass it from `stream_chat` at the `push_and_persist` call site (line 1049). Then `MemoryEvent` carries it cleanly. Separately, tree-sitter is safe for Windows MSVC (ships pre-generated C, same build pattern as sqlite-vec) and `tokenizers` is already in `Cargo.lock` at `0.22.2` via fastembed — zero net cost for token counting.

---

## Library Landscape

| Crate | Latest stable | MSVC Windows OK? | Transitive deps (approx.) | License | Recommendation |
|---|---|---|---|---|---|
| `tree-sitter` | 0.26.9 | Yes — ships pre-generated C bindings via `cc` crate, no bindgen needed at build time unless the `bindgen` feature is enabled (off by default). MSVC + `cc` is the same pattern as `sqlite-vec`. | ~3 (cc, regex, regex-syntax) | MIT | Use. No novel Windows risk. |
| `tree-sitter-rust` | 0.24.2 | Yes | ~2 (tree-sitter, cc) | MIT | Use. Pinned to tree-sitter ≤ 0.26 in its Cargo.toml; couples tightly to parent minor version. |
| `tree-sitter-typescript` | 0.23.2 | Yes | ~2 | MIT | Use. Same coupling model. |
| `tree-sitter-python` | 0.25.0 | Yes | ~2 | MIT | Use. |
| `tokenizers` (token counting) | 0.22.2 (already in Cargo.lock via fastembed) | Yes — already compiled in Phase 2 | already resolved | Apache-2.0 | **Zero net cost** — already a transitive dep. Reuse it for the 480-token cap. No `tiktoken-rs` needed. |
| `text-splitter` | 0.30.1 (requires rustc ≥ 1.86) or 0.20.1 (for rust-version = 1.77.2) | Yes | ~5-8 | MIT | See tradeoff below |

**`text-splitter` vs hand-rolled tradeoff:** The project's declared `rust-version = "1.77.2"` blocks `text-splitter` 0.30.1. Version 0.20.1 supports turn-based chat splitting only via character count, not token count. The speaker-boundary hard rule requires custom logic in any case. **Hand-roll `chunker/chat.rs`** — it's ~100 lines, avoids the dep.

---

## Code Touchpoints

### `commands/chat.rs` — `push_and_persist` (line 200)

`/src-tauri/src/commands/chat.rs:200` — `fn push_and_persist(state: &AppState, agent_id: &str, message: ChatMessage)`. Sync, non-async, called in 4 places:

1. **Line 413**: user message — `Role::User`, content is raw user string.
2. **Line 622**: assistant text response — `Role::Assistant`, content is `response_text`.
3. **Line 652**: assistant tool-call message — `Role::Assistant`, `tool_calls` set, `content = ""`.
4. **Line 1049**: tool result — `Role::Tool`, content = `final_content`, `tool_calls = Some(vec![tc.clone()])`.

**Critical finding**: `is_error` is bound at line 811 (`let (result_content, is_error) = ...`) and is in scope at line 1049. NOT passed to `push_and_persist` and NOT stored in `ChatMessage`. To send `is_error` to the worker, either:
- (a) Add `pub is_error: Option<bool>` to `ChatMessage` and set it only for `Role::Tool` messages (RECOMMENDED).
- (b) Move the channel send to a new inline call at line 1049 before `push_and_persist`.

**Data available at line 1049**: `agent_id` (String), `final_content` (String), `tc.function.name` (tool name), `is_error` (bool), `timestamp_ms`. `project_id` is NOT directly accessible — `agent_project_path` is a `String` path. `derive_project_id` in `identity.rs` handles the conversion.

### `models/chat.rs` — `Role` and `ChatMessage`

`/src-tauri/src/models/chat.rs:7-11` — `Role` enum: `User`, `Assistant`, `System`, `Tool`.

`ChatMessage` (line 15-24): `id: String`, `role: Role`, `content: String`, `tool_calls: Option<Vec<ToolCall>>`, `timestamp_ms: u64`. **No `is_error` field.**

`ToolCall` (line 37-43): `id: String`, `function: ToolFunction`. `ToolFunction` (line 49-53): `name: String`, `arguments: serde_json::Value`.

System prompts: constructed entirely in `models/prompt.rs::compose_system_prompt()`, passed as `ChatRequest.system_prompt`. **NOT persisted via `push_and_persist`.**

### `state.rs` — `AppState`

`/src-tauri/src/state.rs:34-82` — relevant fields:
- `memory_store: Arc<MemoryStore>` (line 71)
- `embedder: Arc<OnceCell<Arc<dyn Embedder>>>` (line 81)
- **Field to ADD**: `memory_tx: tokio::mpsc::UnboundedSender<MemoryEvent>`.
- `new_for_test()` and `new_for_test_async()` both pre-seed embedder via `seeded_embedder_cell()` — Phase 3 ctors need to pre-seed `memory_tx`.

### `lib.rs::run()` — setup hook

`/src-tauri/src/lib.rs:30-45` — setup hook spawns `start_embedder_bootstrap`. Phase 3: create `(memory_tx, memory_rx)` unbounded channel, store `memory_tx` in `AppState`, spawn `memory_worker(memory_rx, state.memory_store.clone(), state.embedder.clone())` via `tauri::async_runtime::spawn`.

### `memory/embedder.rs` — `FakeEmbedder` visibility

`/src-tauri/src/memory/embedder.rs:216-256` — `#[cfg(test)] pub(crate) struct FakeEmbedder`. `lib.rs` (sibling module) already uses it via `use crate::memory::embedder::FakeEmbedder` at line 431. **`#[cfg(test)] pub(crate)` IS cross-file within the same crate.** No refactor needed for Phase 3 worker tests under `src-tauri/src/memory/worker.rs`.

### `memory/db.rs` and `record.rs` — INSERT API

`MemoryStore` wraps `tokio_rusqlite::Connection`. Phase 1 exposes `conn: Connection` (public field) but **NO typed INSERT method**. Phase 3 must write `insert_memory_row` itself. Raw path: `memory_store.conn.call(|c| c.execute("INSERT INTO memory ...", params![...]))`. Required columns: `scope`, `agent_id`, `content`, `memory_type`, `source`, `created_at`, `created_by_type`, `created_by_ref`, `last_modified_at`, `last_accessed_at`. `memory_vectors` INSERT follows (same rowid).

---

## Five Pending Decisions (resolved by user — see proposal.md)

### a) Granularity per `Role`

**Recommendation (LOCKED in proposal D8)**:
- `Role::User` → chunker chat, `memory_type = 'chat_message'`, `created_by_type = 'user'`
- `Role::Assistant` (text) → chunker chat, `memory_type = 'chat_message'`, `created_by_type = 'agent'`
- `Role::Assistant` (tool-calls, empty content) → SKIP
- `Role::Tool` with `is_error = true` → `memory_type = 'tool_error'`, `created_by_type = 'system'`
- `Role::Tool` with `is_error = false` → chunker chat, `memory_type = 'chat_message'`, `created_by_type = 'system'`
- `Role::System` → SKIP (not present in stored history)

### b) Code-block AST detection in chat messages

**LOCKED D7**: defer to Phase 4. Phase 3 already has 30-35 tasks; AST enrichment is incremental retrieval improvement, can be added without schema change.

### c) Backpressure

**LOCKED**: `mpsc::unbounded_channel`. Chat hot path is synchronous in `push_and_persist`; UnboundedSender is non-blocking. Doc 6 stance: "El canal mpsc es unbounded intencionalmente — un retrieval no debería bloquearse esperando capacidad de canal."

### d) Tree-sitter on Windows MSVC

**LOCKED**: SAFE. Uses `cc` crate (same as `sqlite-vec`). No `bindgen` feature. Build adds ~5-10s per grammar cold. Fallback for unknown languages: fall through to turn-based chunker (no crash, less-rich chunks).

### e) `FakeEmbedder` cross-file visibility

**LOCKED**: keep `#[cfg(test)] pub(crate)`. Sufficient for in-crate tests. Promote to `#[cfg(any(test, feature = "test-helpers"))]` only when integration tests under `tests/` dir land.

---

## Architecture Risks (Phase 3 specific)

**R1 — Worker tolerates embedder `None`**: `state.embedder.get()` returns `Option<&Arc<dyn Embedder>>`. On cold start, worker receives events before embedder is ready. Per LOCKED D4: skip insert entirely (no orphan `memory` rows without vectors).

**R2 — Ordering guarantees**: `tokio::mpsc::unbounded` preserves FIFO on single consumer. 5-turn test passes. Breaks only if concurrent `stream_chat` invocations for same `agent_id` are permitted — flag for Phase 4.

**R3 — `importance` heuristic for `tool_error`**: marker words for level 7: "siempre", "nunca", "decidimos", "convención". Tool-specific high-importance: "Tool blocked", "permission denied", "Max tool iterations", auth failures. Low-importance (default 5): transient network errors, user-denied tool calls.

**R4 — `project_id` derivation**: `agent_project_path: Option<String>` (filesystem path). `memory::identity::derive_project_id(&Path)` returns `String`. When `None`: `MemoryEvent.project_id = None`, `scope = 'agent'` per LOCKED D3.

**R5 — Plan contradiction on `is_error`**: documented above. Resolved by LOCKED D1.

---

## Resolved Open Questions

All 5 open questions answered by user before proposal:

| Q | Resolution |
|---|------------|
| Q1 `is_error` field on `ChatMessage` | Add `is_error: Option<bool>` (D1) |
| Q2 Worker placement (AppState vs managed) | `AppState` field (D2) |
| Q3 No-project agents | `scope='agent'`, `project_id=NULL` (D3) |
| Q4 Rust version + text-splitter | Stay 1.77.2, hand-wire tokenizers (D5, D6) |
| Q5 Embedder NotReady behavior | Skip insert entire (D4) |

---

## `skill_resolution`

`injected` — project standards (Rust conventions, no `cargo build`, strict TDD, Phase 1/2 patterns) were embedded in the exploration prompt.
