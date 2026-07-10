# Phase 3 — rag-ingest — Proposal

> **Status**: contract. Downstream phases (spec, design, tasks, apply, verify, archive) treat the 8 locked decisions below as non-negotiable unless an explicit re-litigation note is filed against THIS proposal.
>
> **Inputs read while drafting**: `docs/RAG-PHASE-PLAN.md` (Phase 3 section, lines 163–204), `docs/6-improvements-RAG.md` (the canonical RAG doc), `docs/sdd/rag-embedder/archive-report.md` (Phase 2 carry-forwards), `docs/sdd/rag-embedder/design.md` (Embedder trait surface), `src-tauri/src/commands/chat.rs` (push_and_persist + tool result loop), `src-tauri/src/models/chat.rs` (ChatMessage, Role, StreamEvent), `src-tauri/src/state.rs` (AppState shape), `src-tauri/src/memory/identity.rs` (`derive_project_id`), `src-tauri/src/memory/db.rs` + `mod.rs` + `embedder.rs` (Phase 1/2 surface).
>
> **`docs/sdd/rag-ingest/explore.md`** was referenced in the launch prompt but does NOT exist on disk at the time of writing. The exploration findings are inlined into this proposal via the locked decisions and the "Inconsistencies surfaced" appendix.

---

## 1. Goal

`rag-ingest` wires the **producer side** of the RAG pipeline. Every persisted chat turn (`push_and_persist` in `commands/chat.rs`) emits a typed `MemoryEvent` onto a `tokio::mpsc::UnboundedSender<MemoryEvent>` owned by `AppState`. A single long-lived `memory_worker` async task — spawned once at app startup from the Tauri setup hook — drains that channel, selects a chunker by content type (chat | code | tool_error), embeds each chunk through `state.embedder` (skipping cleanly when the embedder cell is still `None`), and inserts both the `memory` row and its matching `memory_vectors` row through a typed `MemoryStore` insert API. Failures isolate per-event: a panic or error in one event MUST NOT kill the worker. This phase delivers the **producer-consumer pattern + chunker selector** end-to-end; retrieval (Phase 4) and the sycophancy guard (Phase 6) hook into the rows we land here.

---

## 2. Locked decisions (do NOT re-debate)

Each decision is anchor-linkable from spec/design/tasks. A change to any of these requires updating this proposal first, then propagating downstream.

### <a id="D1"></a>D1 — `ChatMessage.is_error: Option<bool>` (default `None`)

Add an `is_error: Option<bool>` field to `ChatMessage` in `src-tauri/src/models/chat.rs`. Default `None` via `#[serde(default)]` + `#[serde(skip_serializing_if = "Option::is_none")]`. The field is set ONLY for `Role::Tool` messages at `commands/chat.rs:1049` (the `push_and_persist` call in the tool result loop, where `is_error` is in scope from line 811). Rationale: cleaner than an inline send at the call site, future-proofs the model for any other consumer (logs, UI), and gives `push_and_persist` the single integration point it needs to derive `MemoryEventKind::ToolError` vs `MemoryEventKind::ChatTurn`.

### <a id="D2"></a>D2 — `AppState.memory_tx: tokio::mpsc::UnboundedSender<MemoryEvent>`

Add a new field `memory_tx` on `AppState`, mirroring the `embedder` precedent from Phase 2 (one new field per major substrate). The bounded-vs-unbounded choice is **unbounded** — backpressure on the chat hot path is unacceptable, and the consumer (single `memory_worker`) preserves FIFO. `AppState::new_for_test()` and `AppState::new_for_test_async()` MUST pre-seed `memory_tx` so tests never deadlock — pair the sender with either a recording receiver (for assertions) or a sink receiver (`tokio::spawn` a drain loop) when the test does not care.

### <a id="D3"></a>D3 — Agents without `project_path` → ingest at `scope='agent'`, `project_id=NULL`

The migration `CHECK` constraint in `memory` (doc 6 lines 191–195) allows `scope='agent' AND agent_id IS NOT NULL` with `project_id` unconstrained. Agents created without a project path therefore ingest with `scope = 'agent'`, `project_id = None`, `agent_id = Some(agent.id)`. This preserves agent-only memory without forcing a synthetic "default" project. When the agent DOES have `Some(project_path)`, the worker derives `project_id = Some(derive_project_id(Path::new(&project_path)))` and still uses `scope = 'agent'` (the agent-scoped memory is project-aware, not project-scoped). Cross-project promotion to `scope='project'` is Phase 12 UX.

### <a id="D4"></a>D4 — Embedder `NotReady` → worker SKIPS the entire insert

If `state.embedder.get()` returns `None` (cold start, model still downloading), or the embed call returns `EmbedError::NotReady`, the worker LOGS at `warn!` and **drops the entire event** — NO `memory` row, NO `memory_vectors` row. Rationale: a half-inserted row (memory without vector, or vector without memory) is worse than a lost event because the next retrieval would silently miss it OR return un-rankable rows. Phase 4 retrieval assumes 1-to-1 `memory ↔ memory_vectors` invariant. Lost cold-start events are acceptable because the chat is fully functional and the user gets no error — only the indexing of those first turns is skipped. (Phase 2 lesson: `AppState.embedder` IS empty at boot — REQ-STATE-001.)

### <a id="D5"></a>D5 — Rust MSRV stays at 1.77.2

No MSRV bump. This blocks `text-splitter` 0.30 (which requires 1.80+). It does not matter because we are NOT using `text-splitter` (see [D6](#D6)). Pinning MSRV keeps Phase 3 within the same toolchain Phase 1 + Phase 2 shipped on, eliminates a class of Windows-MSVC build risk, and removes one variable from the Cargo.toml diff.

### <a id="D6"></a>D6 — Token counting via `tokenizers` 0.22.2 (hand-wired chunker, no `text-splitter`)

`tokenizers 0.22.2` is already in `Cargo.lock` (line 5490) as a fastembed transitive — zero net dependency cost. Hand-wire `memory/chunker/chat.rs` (~100 lines) using the same XLMRoberta tokenizer files the embedder downloaded (`tokenizer.json` + `tokenizer_config.json` cached under `<base_dir>/models/multilingual-e5-small/`). Cap chunks at 480 tokens (under the 512 model max with safety margin). NO `text-splitter` crate. Trade-off: we own a small chunking helper instead of pulling in a third-party crate — acceptable because the chunker chat algorithm is intentionally simple (speaker boundary hard rule + token cap + no overlap) and downstream code (Phase 4 retrieval, Phase 6 sycophancy) does not need richer splitting semantics.

### <a id="D7"></a>D7 — Code-block AST detection in chat messages DEFERRED to Phase 4

Phase 3 chunker chat does NOT parse markdown fences (` ``` `) for AST enrichment. It is a **pure turn-based chunker**: speaker boundary as the hard rule, 480-token cap, no overlap, no fenced-code special-casing. Phase 4 (`rag-retrieval`) may augment with code-block extraction once it has the search side to validate the value. Rationale: scope discipline + the existing chunker scope is already meaningful work + we do not yet have evidence that mid-chat code blocks need AST routing (most code in chat is short and contextual; long code lives in actual files, which `chunker/code.rs` covers).

### <a id="D8"></a>D8 — `Role::System` messages NOT ingested

System messages never reach `push_and_persist`. System prompts live in `ChatRequest.system_prompt` (and `custom_instructions`), which is composed at request time from `compose_system_prompt(...)` — they are not stored in `conversations: HashMap<String, Vec<ChatMessage>>`. So the worker never sees a `Role::System` event and the dispatch enum (`MemoryEventKind`) does not need a variant for it. If a future change starts persisting system messages, it MUST update this proposal and the chunker selector.

---

## 3. Scope

### In scope (Phase 3)

| Item | File / location | Action |
|------|-----------------|--------|
| Async worker with mpsc consumer loop | `src-tauri/src/memory/worker.rs` | NEW |
| `MemoryEvent` struct + `MemoryEventKind` enum | `src-tauri/src/memory/events.rs` | NEW |
| Chunker selector by content type | `src-tauri/src/memory/chunker/mod.rs` | NEW |
| Chat chunker (turn-based, 480-token cap, speaker boundary hard rule) | `src-tauri/src/memory/chunker/chat.rs` | NEW |
| Code chunker (AST-based via tree-sitter for Rust + TS + Python) | `src-tauri/src/memory/chunker/code.rs` | NEW |
| Typed INSERT API for `memory` + `memory_vectors` (one transaction) | `MemoryStore::insert_memory(&MemoryRecordInsert, &[f32]) -> Result<i64>` on `db.rs` OR sibling `store_insert.rs` | NEW method |
| `ChatMessage.is_error: Option<bool>` | `src-tauri/src/models/chat.rs` | 3-line model change + 1 setter at `commands/chat.rs:1049` |
| `AppState.memory_tx` + worker spawn at setup hook | `src-tauri/src/state.rs` + `src-tauri/src/lib.rs::run()` | MODIFIED |
| Tree-sitter deps in `Cargo.toml` | `src-tauri/Cargo.toml` | MODIFIED |

### Out of scope (deferred, NOT regressions)

| Item | Where it lands |
|------|----------------|
| Code-block AST extraction in chat messages | Phase 4 |
| FTS5 fallback for cold-start | Phase 4 |
| `importance` heuristic beyond marker words | Phase 6 (sycophancy + Mem0 resolver) |
| Concurrent chat sessions per agent | Phase 4+ (current chat is single-session) |
| Reindex / migration tooling on embedder swap | Phase X |
| Promote / demote memories between scopes | Phase 12 UX |
| `feature = "test-helpers"` for `FakeEmbedder` cross-crate use | Defer until first integration test needs it |

---

## 4. Architecture (one-page sketch — design.md will expand)

```
┌─────────────────────────────────────────────────────────────────────┐
│ Tauri chat command (producer side)                                  │
│   stream_chat → push_and_persist(state, agent_id, msg)              │
│        │                                                            │
│        ▼   after the existing session.append_message succeeds       │
│   memory_tx.send(MemoryEvent { agent_id, project_id, role,          │
│                                content, is_error, source,           │
│                                timestamp_ms })                      │
└─────────────────────────────────────────────────────────────────────┘
                              │  tokio::mpsc::UnboundedSender
                              ▼
┌─────────────────────────────────────────────────────────────────────┐
│ memory_worker (single long-lived async task, spawned in setup hook) │
│                                                                     │
│   loop {                                                            │
│       Some(event) = rx.recv().await else break;                     │
│       let chunks = chunker::select(&event).chunk(&event)?;          │
│       for chunk in chunks {                                         │
│           let Some(embedder) = state.embedder.get() else {          │
│               log::warn!("embedder not ready, skipping");           │
│               continue;  // D4 — no half-inserts                    │
│           };                                                        │
│           let vec = match embedder.embed(&chunk.text).await {       │
│               Ok(v) => v,                                           │
│               Err(EmbedError::NotReady) => continue,  // D4         │
│               Err(e) => { log::warn!(...); continue; }              │
│           };                                                        │
│           state.memory_store                                        │
│               .insert_memory(&chunk.to_record(&event), &vec)        │
│               .await                                                │
│               .unwrap_or_else(|e| log::warn!(...));                 │
│       }                                                             │
│   }                                                                 │
└─────────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────────┐
│ MemoryStore.insert_memory(record, vector)                           │
│   one tokio_rusqlite call, inside a transaction:                    │
│     INSERT INTO memory (...) RETURNING id;                          │
│     INSERT INTO memory_vectors (rowid, embedding) VALUES (id, ?)    │
└─────────────────────────────────────────────────────────────────────┘
```

### Failure isolation contract

Per-event `try_*` / pattern-match boundaries. The worker NEVER panics:

- Chunker returns `Err` → log + skip event
- Embedder returns any `Err` → log + continue with next chunk (which then also bails on the same reason if it persists)
- `MemoryStore.insert_memory` returns `Err` → log + continue with next chunk
- `rx.recv()` returns `None` (sender dropped, app shutting down) → graceful exit

If a panic still occurs (e.g. tree-sitter parser bug), the spawned task dies. We accept this for v1 — recovery (respawn supervisor) is unscheduled. The chat path is unaffected; the only consequence is no further ingestion until the next app launch.

---

## 5. Data flow

The producer side runs inside the existing `push_and_persist` function. It MUST run AFTER the existing in-memory push + disk persist (so an ingest event is only emitted for messages that actually persisted) and MUST NOT block on the send (`UnboundedSender::send` is non-blocking by definition).

The event payload is:

```rust
pub struct MemoryEvent {
    pub agent_id: String,                 // Agent.id, e.g. UUID-format
    pub project_id: Option<String>,       // derived from agent.project_path, or None
    pub role: Role,                       // ChatMessage.role
    pub content: String,                  // ChatMessage.content
    pub is_error: Option<bool>,           // ChatMessage.is_error (set only for Role::Tool)
    pub source: MemorySource,             // 'chat' | 'tool_result' | ...
    pub timestamp_ms: u64,                // ChatMessage.timestamp_ms
}
```

The consumer derives:

- `scope = 'agent'` (always — for [D3](#D3); cross-scope promotion is Phase 12).
- `memory_type`:
  - `Role::Tool` + `is_error == Some(true)` → `"tool_error"`
  - `Role::Tool` + `is_error == Some(false)` OR `None` → `"chat_message"` (tool output without error is just a chat turn from the model's perspective)
  - `Role::User` / `Role::Assistant` → `"chat_message"`
  - `Role::System` is unreachable per [D8](#D8); defensive `unreachable!()` in the match.
- `importance`: heuristic from marker words (cascada nivel 1 per doc 6 lines 526–536). `tool_error` defaults to 6 (slightly above the 5.0 default, below the marker-word 7). Sycophancy guard interaction with the heuristic is Phase 6.
- `created_by_type = "agent"` for `Role::Assistant`/`Role::Tool`; `"user"` for `Role::User`.
- `created_by_ref = agent_id` (the agent the message belongs to — accurate for both branches).

---

## 6. Module layout

```
src-tauri/src/memory/
├── chunker/                       NEW (subdirectory)
│   ├── mod.rs                     NEW (pub mod chat, pub mod code, pub fn select)
│   ├── chat.rs                    NEW (turn-based, 480-token cap, speaker boundary)
│   └── code.rs                    NEW (AST-based — tree-sitter Rust / TS / Python)
├── events.rs                      NEW (MemoryEvent + MemoryEventKind + MemorySource)
├── worker.rs                      NEW (pub fn spawn_memory_worker(state) → JoinHandle)
├── db.rs                          MODIFIED (add MemoryStore::insert_memory or sibling store_insert.rs)
├── embedder.rs                    UNCHANGED (Phase 2 surface; FakeEmbedder used cross-file via #[cfg(test)] use)
├── error.rs                       MODIFIED (add IngestError variants if MemoryError needs separation)
├── identity.rs                    UNCHANGED (derive_project_id reused — but note: takes &Path, not &str — see Risks R3)
├── migrations.rs                  UNCHANGED
├── model_download.rs              UNCHANGED
├── mod.rs                         MODIFIED (pub mod chunker; pub mod events; pub mod worker;)
└── record.rs                      UNCHANGED (or MINOR — may need a MemoryRecordInsert builder; design decides)
```

Visibility:

- `events.rs`: `MemoryEvent`, `MemoryEventKind`, `MemorySource` are `pub` (used from `commands/chat.rs`).
- `worker.rs`: `spawn_memory_worker` is `pub` (called from `lib.rs::run()` setup hook). Internal helpers `pub(crate)` or private.
- `chunker/mod.rs`: `pub fn select(...)`, the `Chunk` struct, and the `Chunker` trait are `pub(crate)` — the worker is the only caller.
- `chunker/chat.rs`, `chunker/code.rs`: impls live `pub(crate)`; the only entry point external code uses is `select`.

`#[cfg(test)] use crate::memory::embedder::FakeEmbedder` inside `worker.rs::tests` is sufficient for the in-crate worker test (per the launch prompt's confirmation that `pub(crate)` test visibility holds). No refactor to a `feature = "test-helpers"` flag in Phase 3.

---

## 7. Test strategy

Strict TDD applies — every impl preceded by its failing test (`*-test` → `*-impl` task pairing, same convention as Phase 1 + Phase 2). All assertions documented in `spec.md`.

### Unit tests

| Module | Test surface |
|--------|--------------|
| `chunker/chat.rs` | Speaker boundary hard rule (turn N ≠ turn N+1 → separate chunks). 480-token cap (turn > 480 tokens → split, both chunks ≤ 480). No overlap (chunk[i].end == chunk[i+1].start). Empty content → empty chunks vec. |
| `chunker/code.rs` | AST detection by file extension (`.rs` → Rust grammar, `.ts`/`.tsx` → TS grammar, `.py` → Python grammar). Unknown extension → degraded turn-based fallback. Single function body → single chunk with header. Multi-function file → N chunks, each with its own header. Parse failure → degraded fallback (NOT panic — surface as `ChunkerError::ParseFailed`). |
| `chunker/mod.rs::select` | Routing matrix: `MemoryEventKind::ChatTurn` → chat chunker; `ToolError` → chat chunker (treated as one turn); `CodeFile` → code chunker. (`CodeFile` is reserved for the future file-watcher producer — Phase 3 only fires `ChatTurn` / `ToolError` from `push_and_persist`.) |
| `events.rs` | `MemoryEvent` Debug + serde round-trip. `MemorySource` enum exhaustively matched in the worker. |
| `worker.rs` | Integration: spawn worker with `FakeEmbedder` + `MemoryStore::new_for_test()` → send 5 `MemoryEvent`s → assert 5 rows in `memory`, 5 rows in `memory_vectors`, temporal order preserved (FIFO of unbounded mpsc on single consumer is guaranteed). |
| `store_insert.rs` (or `db.rs`) | `insert_memory` writes both rows in one transaction (drop the connection mid-write via fault injection → both rows absent or both present, never one without the other). |

### Failure-mode tests

| Scenario | Expected |
|----------|----------|
| `state.embedder` cell empty (`OnceCell::get() == None`) | Worker logs `warn!`, drops event, NO rows inserted. |
| `embedder.embed` returns `EmbedError::NotReady` | Worker logs `warn!`, drops chunk, continues loop. |
| `embedder.embed` returns `EmbedError::WorkerGone` | Worker logs `warn!`, drops chunk, continues loop. |
| Tree-sitter parser returns parse error on malformed code | `ChunkerError::ParseFailed`, fall back to degraded turn-based chunking. |
| `MemoryStore.insert_memory` returns `Err` | Worker logs `warn!`, continues loop. Worker stays alive. |
| `memory_rx.recv()` returns `None` (sender dropped, shutdown) | Worker exits gracefully (`return Ok(())`). |

### End-to-end test

A 5-turn chat (user → assistant → tool → user → assistant) routed through a stubbed `push_and_persist`-equivalent that sends `MemoryEvent` directly to the worker's channel. Assertions: 5 rows in `memory` with correct ordering by `created_at`, each with a matching `memory_vectors` row, `agent_id` correct, `project_id` correct (Some for project-bound agent, None for project-less agent — exercise both in two passes).

### NO sleep-in-tests

Carry forward Phase 2 lesson. Channel-based assertions only — `tokio::time::pause()` + `tokio::time::advance()` for any retry timing if added; `tokio::time::timeout(Duration::from_secs(1), ...)` as the test hard stop (real seconds, but only when a real bug would deadlock — never as a delay).

---

## 8. Risks

| ID | Risk | Mitigation |
|----|------|------------|
| <a id="R1"></a>R1 | Adding `is_error` to `ChatMessage` is a serialization-breaking change for any persisted conversation JSON written by Phase 1/2. | Add `#[serde(default)]` on the field so old JSON deserializes as `is_error: None`. Add `#[serde(skip_serializing_if = "Option::is_none")]` so re-serialized JSON does not gain a noisy field. Forward-compatible, no migration needed. |
| <a id="R2"></a>R2 | Tree-sitter grammars couple tightly to parent minor version. A mismatch causes runtime parse errors that look like content bugs. | Pin exact versions in `Cargo.toml`: `tree-sitter = "0.26"` (latest stable as of 2026-05 per the locked context), `tree-sitter-rust = "0.24"`, `tree-sitter-typescript = "0.23"`, `tree-sitter-python = "0.25"`. Use `~` constraints (e.g. `~0.26.9`) to forbid even minor bumps without a deliberate edit. Verify resolution with `cargo update --dry-run` on first commit. |
| <a id="R3"></a>R3 | The launch prompt described `derive_project_id(path)` as accepting `&str` and the prompt referred to `agent_project_path: String`. The ACTUAL signature is `derive_project_id(project_path: &Path) -> String` (verified in `memory/identity.rs:11`). | Worker constructs `derive_project_id(Path::new(agent.project_path.as_deref().unwrap_or("")))` and short-circuits on empty string to `None`. Document in design. **No code change to `identity.rs`** — adapt at the call site. Surface to the orchestrator (see "Inconsistencies surfaced"). |
| <a id="R4"></a>R4 | `MemoryStore` from Phase 1 has NO typed INSERT method (`Grep "pub (fn\|async fn) (insert\|store\|persist\|save)" memory/` returns zero matches). Phase 3 MUST add one. | Phase 3 deliverable, not a regression. Spec REQ-STORE-001 will pin the signature; design will pin the transaction shape. The Phase 1 archive does NOT promise an INSERT API — only `open`, `new_for_test`, and the raw `conn` are exposed. |
| <a id="R5"></a>R5 | Worker spawn at app startup: if `AppState::new()` panics or the channel receiver is dropped, ingest silently dies. | Graceful shutdown: drop the receiver only when the app exits (it is owned by the worker future, which is held by the spawned task's `JoinHandle`). If `AppState::new()` panics, the entire app dies — chat is unusable anyway, so silent ingest failure is moot. Document in design §lifecycle. |
| <a id="R6"></a>R6 | The chunker `code.rs` for unknown extensions falls back to turn-based, but `chunker/chat.rs` is reused — risk of subtle coupling where chat-specific assumptions (speaker boundary) leak into code fallback. | Factor a shared `TokenSplitter` helper that ONLY does "split by 480 tokens, no overlap". Both `chat.rs` (after speaker boundary) and `code.rs` fallback consume it. The shared piece does NOT know about speakers OR AST. |
| <a id="R7"></a>R7 | Tree-sitter `cc`-crate compile adds ~5-10 s per grammar on cold build (Windows MSVC). Cumulative impact for 3 grammars on first `cargo check`. | Acceptable for v1. Same precedent as `sqlite-vec` (Phase 1, also `cc`-built). Document the one-time cost in design + tasks (`apply` checkpoint 1 will hit it). |
| <a id="R8"></a>R8 | `tokenizers` 0.22.2 surface is technically a stable API but the model's `tokenizer.json` path is owned by Phase 2 (under `<base_dir>/models/multilingual-e5-small/`). If Phase 2's path layout shifts, the chunker breaks. | Loading the tokenizer in `chunker/chat.rs` MUST go through a path helper that lives in `memory/model_download.rs` (e.g. `model_download::tokenizer_path(base_dir) -> PathBuf`) — co-locating the path knowledge with the downloader. Defensive: if the file is absent (embedder still downloading), the chunker falls back to a **byte-length heuristic** (cap at ~2000 chars ≈ 480 tokens on average for the languages we support) and logs a `warn!`. NO panic. |

---

## 9. Test gates (must pass before archive)

- All Phase 1 tests still pass.
- All Phase 2 tests still pass.
- Both above are counted in the 249/249 Rust baseline established by Phase 2 archive — Phase 3 starts from 249/249.
- Net new Phase 3 tests pass.
- No new clippy warnings introduced by Phase 3 modules (`cargo clippy --lib -- -D warnings` on `memory/`, `commands/chat.rs`, `state.rs`, `lib.rs`).
- NO `cargo build` runs during development (user rule). `cargo check` + `cargo test --lib` are the only allowed CI-style commands.

---

## 10. Estimated task count

~30–35 test/impl pairs, per the Phase 3 line in `RAG-PHASE-PLAN.md`. Tree-sitter AST chunking is the densest section.

### Apply checkpoints

| # | Checkpoint | Gate |
|---|------------|------|
| 1 | `MemoryEvent` + `worker.rs` skeleton + channel wired through `AppState` + worker spawn in `lib.rs::run()` setup hook | Worker compiles. Empty consumer loop. `cargo check` clean. Worker drains 1 event in a fake test. |
| 2 | `chunker/chat.rs` complete + tested (speaker boundary, 480-token cap, no overlap) + `MemoryStore::insert_memory` complete + tested | Chat-only end-to-end: send `Role::User` event → 1 row in `memory` + 1 row in `memory_vectors`. |
| 3 | `chunker/code.rs` complete + tested (Rust, TS, Python grammars, degraded fallback) | Code chunker exercised against fixtures for the 3 grammars + a `.txt` fallback. |
| 4 | Full integration: `is_error` field on `ChatMessage`, setter at `chat.rs:1049`, `push_and_persist` emits `MemoryEvent`, tool_error routing verified | 5-turn E2E test passes. Failure-mode tests (NotReady, WorkerGone, parse-fail) pass. All 249 pre-existing + N new tests green. |

---

## 11. Carry-forward patterns from Phase 2 (apply unmodified)

| Pattern | Source | Where it applies in Phase 3 |
|---------|--------|------------------------------|
| `Arc<...>` for shared state, `OnceCell` for lazy init | Phase 2 `AppState.embedder` | The worker reads `state.embedder.get()` and tolerates `None`. The mpsc sender is `Arc`-wrapped implicitly (it Clones to a `Sender`). |
| Bespoke error enums per module + `thiserror` | Phase 1 `MemoryError`, Phase 2 `EmbedError` / `DownloadError` | New: `ChunkerError`, `IngestError` (or extend `MemoryError`). Design picks the split. |
| Compile-time `Send + Sync` assertions via `const _: fn()` blocks | Phase 2 `embedder.rs` | Apply to `MemoryEvent`, the worker's spawned future, and any new public type that crosses a task boundary. |
| `pub(crate)` over `pub` unless cross-crate use is needed | Project convention | Worker spawn entry point is `pub`; everything inside `chunker/` is `pub(crate)`. |
| Trait abstraction for testability (Phase 2 `EmbedderEventSink`) | Phase 2 design §5.6 | Phase 3 does NOT emit lifecycle events (no `app.emit`), so no sink trait needed. If a future change makes the worker emit progress, copy the pattern. |
| Strict TDD with mocks dedicated | Phase 2 lesson | `FakeEmbedder` for embed, `MemoryStore::new_for_test()` for store, hand-rolled `MockMemoryStore` only if the store needs fault-injection that `new_for_test()` does not cover. |
| NEVER `git stash` to verify regressions on untracked dirs | Phase 1 anti-pattern | `src-tauri/` is still `?? src-tauri/` in `git status`. Use explicit before/after `cargo test --lib` output comparison instead. |
| Sole-source terminal events | Phase 2 fix | Phase 3 has no terminal events (the worker loop is unbounded). N/A here, but the pattern is in place for Phase 4+. |

---

## 12. Inconsistencies surfaced (read by orchestrator before spec)

1. **`docs/sdd/rag-ingest/explore.md` does NOT exist on disk.** The launch prompt referenced it as "READ IT ENTIRELY" with library landscape, code touchpoints, risk analysis, and 5 open questions. The findings were inlined into the launch prompt as locked decisions, so the proposal is grounded. If a separate `explore.md` is expected by the SDD pipeline, the orchestrator should either (a) create a minimal stub pointing at this proposal as the source of record, or (b) accept that Phase 3 skipped a formal explore artifact. **Recommendation**: accept (b); the locked-context section of the launch prompt is comprehensive and the inconsistencies surfaced below cover the remaining gaps.
2. **`derive_project_id` signature inconsistency.** Launch prompt described it as taking `&str` and returning `String`. ACTUAL signature in `memory/identity.rs:11` is `pub fn derive_project_id(project_path: &Path) -> String`. The call site in the worker must do `derive_project_id(Path::new(s))`. No source change required — the adaptation is one line at the call site. ([R3](#R3))
3. **`MemoryStore` has NO `insert` API.** Phase 1 did not ship one. Phase 3 adds it as a deliverable. ([R4](#R4))
4. **`FakeEmbedder` visibility.** Confirmed adequate (`#[cfg(test)] pub(crate)` is enough for in-crate access from `worker.rs` under `#[cfg(test)]`). No refactor needed.
5. **Tree-sitter exact versions** documented in the launch prompt (0.26.9 / 0.24.2 / 0.23.2 / 0.25.0) are stated as "confirm in your proposal". This proposal pins the loose family (`tree-sitter = "0.26"` etc.) and defers exact-version pinning to design `Cargo.toml` diff after a `cargo update --dry-run` check at apply CHECKPOINT 1 — same pattern Phase 2 used for fastembed features. ([R2](#R2))

---

## 13. Done definition

Phase 3 is done when:

- All test gates in §9 pass.
- A real chat turn in `cargo test --lib memory::worker -- --ignored` (manual smoke, gated like the Phase 2 real-model test) lands a row in `memory` + `memory_vectors` with correct fields. Optional — only the team SHOULD run this in a real Tauri dev environment before Phase 4 starts.
- `docs/sdd/rag-ingest/archive-report.md` documents the locked decisions that held and any that drifted (with rationale).
- Phase 4 (`rag-retrieval`) is unblocked: it can read `memory` and `memory_vectors` directly via `MemoryStore.conn`, and the rows it finds have the shape this proposal contracts.
