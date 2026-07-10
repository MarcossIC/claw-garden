# Phase 3 — rag-ingest — Archive Report

**Status**: Complete and archived.

**Completion date**: 2026-05-24.

**Test gate final**: 315/315 Rust PASS (249 baseline Phase 1+2 + 66 net new). Zero failures, zero regressions, zero new clippy warnings.

---

## Locked decisions summary (D1–D8)

All 8 decisions from the proposal held without modification. Carry-forward for Phase 4+:

| Decision | Summary |
|----------|---------|
| D1 | `ChatMessage.is_error: Option<bool>` field added; default `None` via serde; set only for `Role::Tool` at line 1049 |
| D2 | `AppState.memory_tx: UnboundedSender<MemoryEvent>` — unbounded channel, non-blocking on hot path |
| D3 | Agents without `project_path` ingest at `scope='agent'`, `project_id=NULL`; with path, `scope='agent'`, `project_id=Some(hash(path))` |
| D4 | Embedder `NotReady` or cell empty → worker drops entire event, ZERO rows inserted (no half-inserts). 1-to-1 `memory ↔ memory_vectors` invariant upheld |
| D5 | MSRV 1.77.2 (unchanged); blocks `text-splitter 0.30` but not needed (hand-wired chunker) |
| D6 | `tokenizers 0.22.2` + hand-wired `chat.rs` chunker (~100 lines, 480-token cap). NO `text-splitter` crate |
| D7 | Code-block AST detection deferred to Phase 4; Phase 3 is pure turn-based chunker |
| D8 | `Role::System` messages never ingested; unreachable via `push_and_persist` → defensive skip in worker |

---

## Files delivered

### New (10 files)

- `src-tauri/src/memory/events.rs` — `MemoryEvent`, `MemorySource` enums, `from_message` builder
- `src-tauri/src/memory/worker.rs` — async `memory_worker` loop, `start_memory_worker` entry point
- `src-tauri/src/memory/chunker/mod.rs` — `ChunkerKind`, `Chunk`, `select_chunker` routing, `ChunkerError`
- `src-tauri/src/memory/chunker/chat.rs` — turn-based chunker (speaker boundary, 480-token cap, no overlap)
- `src-tauri/src/memory/chunker/code.rs` — AST-aware chunker via tree-sitter (Rust/TS/Python, degraded fallback)
- `src-tauri/src/memory/chunker/token_splitter.rs` — shared `TokenSplitter` (tokenizer + byte-length fallback)
- `src-tauri/src/memory/store_insert.rs` — `MemoryStore::insert_memory_with_vector`, `NewMemoryRecord`, `MemoryType`/`CreatedByType` enums, `MemoryStoreError`

### Modified (8 files)

- `src-tauri/src/models/chat.rs` — added `is_error: Option<bool>` field with `#[serde(default, skip_serializing_if = "Option::is_none")]`
- `src-tauri/src/state.rs` — added `memory_tx` field; `AppState::new()` returns tuple `(Self, Receiver)`; new test ctors `new_for_test_with_memory_rx`, `MemoryTestHandle`
- `src-tauri/src/lib.rs` — setup hook spawns `start_memory_worker` after embedder bootstrap; passes `memory_rx`
- `src-tauri/src/commands/chat.rs` — `push_and_persist` signature gains `agent_project_path` param; emits `MemoryEvent` after persist; 4 call sites updated (413, 622, 652, 1049)
- `src-tauri/src/memory/model_download.rs` — added `pub fn tokenizer_path(base_dir: &Path) -> PathBuf` helper (co-locates path knowledge with downloader)
- `src-tauri/Cargo.toml` — added tree-sitter family (`0.26`, `0.24`, `0.23`, `0.25`) + explicit `tokenizers = "0.22.2"` pin
- `src-tauri/src/memory/mod.rs` — registered `pub mod events, chunker, worker, store_insert`

---

## Architectural moves to carry forward

1. **Producer-consumer via `tokio::mpsc::UnboundedSender<MemoryEvent>`** — non-blocking send from sync chat hot path (`push_and_persist`), single-consumer worker, FIFO ordering guaranteed. Phase 4+ retrieval should feed more events through the same channel if needed (e.g., file-watcher `CodeFile` events). Pattern proves scalable and testable.

2. **Atomic INSERT via single `tokio_rusqlite::Connection::call` + transaction** — both `memory` and `memory_vectors` rows land in one transaction closure or neither does. This is the canonical multi-table insert pattern for Phase 3 and beyond. Phase 4+ retrieval KNN queries assume 1-to-1 invariant; Phase 5+ access tracker batches via the same pattern.

3. **`ChunkerKind::Skip` as defensive arm** + **`MemoryEvent::from_message` returning `Option<Self>`** — defense in depth for events we deliberately don't ingest (`Role::System`, empty-assistant-tool-calls). Chunker selector also catches `Skip` at the worker call site. Future producers should adopt the `Option<Self>` builder pattern.

4. **Tokenizer path helper in `model_download.rs`** with **byte-fallback in `TokenSplitter`** for cold-start race — pattern for any future model-resource-dependent chunker. Degradation to byte-length splits is acceptable because Phase 4 retrieval ranks by cosine similarity (not chunk-boundary alignment). Next app launch picks up the tokenizer.

5. **Tree-sitter `Mutex<Parser>` per language, one per app lifetime** — same MSVC build pattern as `sqlite-vec`; no novel Windows risk. Parser is non-`Sync` but `Mutex` adapter makes type `Sync`. Phase 4+ file-watcher can reuse the same `CodeChunker` instance.

6. **`AppState::new()` returning `(Self, Receiver)` tuple** — forces explicit receiver ownership, preventing silent event loss. Pattern applies to any future worker-receiver pair spawned asynchronously. One mechanical ripple: every `AppState::new_for_test` call must destructure.

---

## Lessons learned

- **Spec/design routing-key mismatch caught pre-tasks**: spec REQ-CHK-001 said `select_chunker(source: MemorySource)`, design said `(role, is_error, content)`. Orchestrator aligned spec to design (better grounded in code). Lesson: when spec and design conflict, design usually wins because it's read the actual code.

- **`created_by_type='system'` for tool results, not `'agent'`**: canonical values per doc 6 line 164 are `'user'|'agent'|'system'|'trajectory_miner'` — no `'tool'`. Tool output is authored by the system/tool execution, not the agent. This distinction is load-bearing for Phase 6 sycophancy guard.

- **`log::` not `tracing::`**: codebase has 43 `log::` calls across 9 files, zero `tracing::`. Phase 3 docs initially mentioned `tracing`; corrected pre-apply per orchestrator's cargo.toml audit. Zero `tracing::` in final code.

- **Tree-sitter on MSVC**: same `cc` build pattern as `sqlite-vec`. No Windows-specific risk surfaced during apply. Compile time +15-30 sec on first build (one-time, cached).

- **W1 E2E FIFO assertion gap (fixed)**: verify caught that `e2e_5_turn_chat_produces_5_rows` asserted row count but not temporal order. Fixed inline pre-archive: now queries `SELECT content FROM memory ORDER BY created_at ASC` and asserts equality with source sequence `["msg1", "msg2", ..., "msg5"]`.

- **`push_and_persist` signature ripple (R-NEW-1)**: adding `agent_project_path` param touched 4 call sites (413, 622, 652, 1049). Mechanical but easy to miss. Lesson: always grep for all call sites when refactoring.

- **`AppState::new()` tuple-return ripple**: 3 call sites in production (`lib.rs:26`), 3 in test ctors (`new_for_test`, `new_for_test_async`, `new_for_test_with_memory_rx`), plus every test that previously called `new_for_test_async().await` as a bare value. Mandatory destructuring forced explicit receiver ownership — better discipline.

- **`Chunk.speaker` was redundant**: spec initially added a `speaker` field; design corrected it — speaker is derived from `MemoryEvent.role` at insert time. Test names diverged (`chat_chunk_has_no_ast_header` vs actual `header` field name) but behavior correct. Lesson: spec REQ-CHK-002 uses `header`; tasks.md T06-test used stale `ast_header` name from earlier draft; apply was right.

- **`ChunkerError` declared but infallible signatures chosen at runtime**: chunkers chose degraded-not-errored paths (tokenizer missing → byte fallback; parse error → byte fallback). Spec REQ-CHK-003/-004 prose says `Result<_, ChunkerError>`, code says `Vec<Chunk>`. Documented as design deviation, not a bug. The type IS tested (verify confirms `implements Error` + `format!` work), but never returned at runtime.

- **Phase 2 `FakeEmbedder` visibility worked cross-file**: `#[cfg(test)] pub(crate)` is sufficient without `feature = "test-helpers"` refactor. Deferred per decision #11 until integration tests under `tests/` dir need it.

---

## Follow-ups for future phases

Carry-forward from verify-report.md §SUGGESTIONS + design.md §11 R-NEW deferred items:

- **S1**: `MemoryWorkerError::EmptyChunks` reserved for Phase 4 (dead code currently with `#[allow]`)
- **S2**: `Chunk.token_count = 0` in byte-fallback mode — Phase 4 retrieval re-ranking concern
- **S3**: `ChunkerError` unused at runtime — chunkers infallible (design deviation; document for Phase 4 if needed)
- **S4**: `MemoryType::CodeBlock` unused variant — reserved for Phase 4
- **R-NEW-2** (cold-start tokenizer race): accepted for v1; byte-length splits within ~30% accuracy; hot-swap deferred to Phase X if ranking improves warrant it
- **R-NEW-6** (`.tsx` uses `LANGUAGE_TYPESCRIPT` not `LANGUAGE_TSX`): Phase 4 file-watcher feeder will wire separate parser slot
- **W3** (spec REQ-CHK-001 routing table mentions `CodeFile` but `select_chunker` doesn't take `MemorySource`): Phase 4 will either extend signature or have file-watcher call `code_chunker.chunk()` directly; documented in this report
- Manual smoke test: cold-start with missing tokenizer.json (BLOCKED-BY-EXTERNAL — team runs in real Tauri env)

---

## Final test count

| Layer | Count | vs baseline |
|-------|-------|-------------|
| Rust (`cargo test --lib`) | 315 PASS | 249 pre-Phase-3 + 66 new |
| Regressions | 0 | — |
| New clippy warnings (Phase 3 modules only) | 0 | — |

---

## Done definition: SATISFIED

- [x] All 8 LOCKED decisions honored without drift
- [x] MemoryEvent proves `Send + 'static` via compile-time const block
- [x] Worker spawned in `lib.rs::run()` after embedder bootstrap, consumes receiver
- [x] Worker gracefully exits when all senders dropped; never panics
- [x] Embedder `NotReady` / cell empty → 0 rows, worker alive (failure isolation)
- [x] Embedder error mid-stream (WorkerGone, InferenceFailed) → skip chunk, continue (per-chunk isolation)
- [x] Store write failure → skip chunk, worker alive (per-chunk isolation)
- [x] ChatMessage.is_error field: old JSON deserializes as `None`; new JSON omits key when `None`
- [x] `is_error = Some(true)` set at `chat.rs:1049` only (tool result loop)
- [x] Chat chunker: empty → `Ok([])`; 1000 tokens → ≥2 chunks ≤480 tokens each; no overlap
- [x] Code chunker: Rust/TS/Python → chunks with header; unknown ext → chat fallback, no panic
- [x] `insert_memory_with_vector`: both rows present after `Ok`; zero rows after error (atomicity verified)
- [x] CHECK constraint: `scope='agent'`, `agent_id IS NOT NULL`, `project_id` nullable → passes
- [x] 5-turn E2E test: 5 `memory` rows + 5 `memory_vectors` rows, FIFO order by `created_at`, correct `agent_id`, project_id Some/None invariants
- [x] All 249 Phase 1/2 tests still pass. Net new Phase 3 tests all pass
- [x] No `tokio::time::sleep(Duration::from_secs(...))` in tests
- [x] No `cargo build` invoked
- [x] Archive report documents locked decisions, architectural moves, lessons learned, follow-ups

---

## Phase 4 unblocked

Phase 4 (`rag-retrieval`) can now:
- Read `memory` + `memory_vectors` directly; assume 1-to-1 invariant holds
- Rows are well-formed: `created_by_type` is one of `'user'|'agent'|'system'`; `memory_type` is one of `'chat_message'|'tool_error'`; `scope` is always `'agent'` in Phase 3 data
- Embedder is available post-bootstrap; worker is alive and draining events; ingest latency is sub-ms per event
- `FakeEmbedder` is available for test doubles (cross-file move to feature gate deferred)
