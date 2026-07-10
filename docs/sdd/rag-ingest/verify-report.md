# Phase 3 — rag-ingest — Verify Report

## Verdict

ACCEPTED (post-fix)

Originally ACCEPTED-WITH-WARNINGS. All 3 WARNINGs were resolved inline by the orchestrator before archive:

- **W1 (E2E FIFO assertion)**: FIXED. `memory/worker.rs::e2e_5_turn_chat_produces_5_rows` now queries `SELECT created_at, content FROM memory ORDER BY created_at ASC, id ASC` and asserts the resulting Vec equals `[(1, "msg1"), (2, "msg2"), ..., (5, "msg5")]`. A regression that scrambled order would now fail with a diff-style message.
- **W2 (test naming `ast_header` vs `header`)**: FIXED. `memory/chunker/mod.rs` tests renamed `chat_chunk_has_no_ast_header → chat_chunk_has_no_header` and `code_chunk_has_ast_header → code_chunk_has_header`. Test bodies were already correct.
- **W3 (spec routing table mentions `MemorySource::CodeFile`)**: FIXED. `docs/sdd/rag-ingest/spec.md` REQ-CHK-001 routing table updated — Phase 3 producers only emit `Role::{User,Assistant,Tool}`; `ChunkerKind::Code` is unreachable from Phase 3 alone. The forward-only hook for Phase 5+ file watcher is documented to live at the worker call site (`if source == MemorySource::CodeFile { Code } else { select_chunker(...) }`), NOT inside `select_chunker`.

Post-fix gate: **315/315 PASS** (unchanged count — the FIFO strengthening tightens an existing test without adding a new one). The implementation satisfies all 27 REQs at the behavior level, atomicity is provably upheld via a single `tokio_rusqlite::Connection::call` transaction, the embedder-gate (LOCKED D4) is wired correctly, no `tracing::` calls slipped in, no `cargo build` was invoked. The bug-hunt checklist surfaced **NO CRITICAL findings**. Four SUGGESTIONs remain documented for Phase 4+ follow-ups.

## Executive summary

- 315/315 tests pass (249 Phase 1+2 baseline + 66 new Phase 3), zero failures, zero regressions, zero new clippy warnings in Phase 3 modules.
- Atomicity contract REQ-INS-003 is correctly implemented as a single `tx.transaction()` → `INSERT memory` → `INSERT memory_vectors` → `tx.commit()` inside ONE `.call(...)` closure. Both INSERTs land or neither does.
- The Phase-2-style "sneaky bug" hunt found NO repeat of the double-terminal-event class (Phase 3 has no terminal event by design). Two WARNINGs concern the E2E test depth (no explicit ORDER BY assertion) and a divergence between the spec's `ChunkerError` name and the actual `Chunk.header` field name vs the spec's `header`.

## Test gate

- Baseline (Phase 1+2): 249/249 PASS (already counted into the 315).
- New (Phase 3): 66/66 PASS.
- Total: **315/315 PASS, 0 failed, 0 ignored**. Finished in 0.38s.
- Clippy: clean for all Phase 3 modules (`memory/{events,worker,store_insert,chunker/*,model_download}.rs`, `state.rs`, `lib.rs`, `commands/chat.rs`, `models/chat.rs`). The 118 warnings reported by `cargo clippy --lib` are ALL pre-existing in unrelated paths (`services/credentials.rs`, `services/model_meta.rs`, `memory/record.rs`, `memory/error.rs`, `memory/embedder.rs`, `memory/model_download.rs`, `memory/mod.rs`, `memory/db.rs` — none of the Phase 3 NEW files contributed a warning).
- `cargo build` was NEVER invoked (verified via the orchestrator's gate + my own commands: only `cargo test --lib` and `cargo clippy --lib`).

## REQ coverage matrix

| REQ ID | Impl site | Test name(s) | Verdict |
|---|---|---|---|
| REQ-EVT-001 (MemoryEvent struct) | `memory/events.rs:13-22` | `memory_event_is_send`, `memory_event_fields_round_trip` | SATISFIED |
| REQ-EVT-002 (MemorySource enum) | `memory/events.rs:30-37` | `memory_source_is_copy_and_clone`, `memory_source_exhaustive_match` | SATISFIED |
| REQ-EVT-003 (FIFO + dropped-receiver) | `memory/events.rs` (channel type) + `commands/chat.rs:254-256` (log::debug on Err) | `sender_dropped_does_not_panic`, `memory_tx_clone_sends_independently`, `push_and_persist_dropped_receiver_no_panic` | SATISFIED |
| REQ-WRK-001 (worker spawn) | `lib.rs:50` (setup hook), `memory/worker.rs:130-145` (spawn fn) | `spawn_memory_worker_returns_join_handle` | SATISFIED |
| REQ-WRK-002 (consumer loop + per-event failure isolation) | `memory/worker.rs:38-63` | `worker_exits_gracefully_when_all_senders_dropped`, `worker_continues_after_per_event_error` | SATISFIED |
| REQ-WRK-003 (NotReady → drop event) | `memory/worker.rs:85-92` + `:106-113` | `embedder_cell_empty_no_rows_inserted`, `embedder_not_ready_no_rows_inserted`, `embedder_worker_gone_skips_chunk_continues` | SATISFIED |
| REQ-WRK-004 (store write failure isolation) | `memory/worker.rs:98-104` | `store_write_failure_worker_stays_alive` | SATISFIED |
| REQ-WRK-005 (memory_type/created_by_type derivation) | `memory/store_insert.rs:60-87` | `user_role_produces_chat_message_type_user_creator`, `assistant_role_produces_agent_creator`, `tool_error_produces_tool_error_type_system_creator_importance_6`, `tool_success_produces_chat_message_type_system_creator` | SATISFIED |
| REQ-CHK-001 (select_chunker) | `memory/chunker/mod.rs:53-69` | 5 `select_chunker_*` tests | SATISFIED |
| REQ-CHK-002 (Chunk struct) | `memory/chunker/mod.rs:27-37` | `chat_chunk_has_no_ast_header`, `code_chunk_has_ast_header` | SATISFIED — NB the field is named `header` (matches design.md §3.2) not `ast_header` as named in some spec tables. The internal field name diverges from one of the spec headings but matches design.md. Test names use `ast_header` but assertions inspect `header`. See WARNING #2. |
| REQ-CHK-003 (chat chunker turn-based, 480 cap, no overlap) | `memory/chunker/chat.rs:28-68`, `memory/chunker/token_splitter.rs:38-86` | 6 chat-chunker tests | SATISFIED |
| REQ-CHK-004 (code chunker AST + fallback) | `memory/chunker/code.rs:34-103` | 6 code-chunker tests | SATISFIED |
| REQ-CHK-005 (shared TokenSplitter) | `memory/chunker/token_splitter.rs:38-86` | `token_split_respects_cap`, `token_split_short_input_single_segment` | SATISFIED |
| REQ-CHK-006 (ChunkerError) | `memory/chunker/mod.rs:43-50` | `chunker_error_implements_error_trait`, `chunker_error_parse_failed_displays_message` | SATISFIED |
| REQ-INS-001 (insert_memory_with_vector signature) | `memory/store_insert.rs:101-153` | `insert_memory_with_vector_returns_rowid`, `insert_memory_with_vector_returns_err_on_wrong_dimension` | SATISFIED — connection-close test was substituted with DimensionMismatch (acknowledged in tasks T12). |
| REQ-INS-002 (NewMemoryRecord) | `memory/store_insert.rs:46-88` | `insert_agent_scope_no_project_passes_check`, `insert_agent_scope_with_project_passes_check` | SATISFIED |
| REQ-INS-003 (atomicity) | `memory/store_insert.rs:115-152` (single `.call(...)` with tx) | `both_rows_present_after_successful_insert`, `vector_insert_failure_leaves_no_memory_row` | SATISFIED. Manual code-read confirms: ONE `.call()` closure, `tx = c.transaction()`, two `tx.execute()`s, `tx.commit()`. Atomic. |
| REQ-INS-004 (MemoryStoreError Send+Sync) | `memory/store_insert.rs:90-98` | `memory_store_error_is_send_sync`, `memory_store_error_displays_message` | SATISFIED |
| REQ-INT-001 (ChatMessage.is_error) | `models/chat.rs:24-28` + 4 call sites updated (`commands/chat.rs:458, 669, 701, 1100`) | 4 `is_error_field_*` tests | SATISFIED |
| REQ-INT-002 (push_and_persist emit) | `commands/chat.rs:204-258` (push_and_persist + push_and_persist_inner) | 4 `push_and_persist_*` tests | SATISFIED |
| REQ-INT-003 (derive_project_id call-site adaptation) | `commands/chat.rs:247-249` | `project_id_derived_from_path`, `project_id_none_when_no_path` | SATISFIED |
| REQ-STATE-001 (AppState.memory_tx) | `state.rs:78`, `state.rs:103-171` (`new()` returns tuple) | `memory_tx_send_before_worker_buffers_event`, `memory_tx_clone_sends_independently` | SATISFIED |
| REQ-STATE-002 (test ctors seed memory_tx) | `state.rs:181-308` (`new_for_test`, `new_for_test_async`, `new_for_test_with_memory_rx`, `MemoryTestHandle`) | `new_for_test_async_memory_tx_does_not_deadlock`, `new_for_test_with_memory_rx_round_trip` | SATISFIED |
| REQ-STATE-003 (compile-time Send+Sync assertions) | `memory/events.rs:69-75` (module-level const block), `memory/worker.rs:149-156`, `memory/chunker/chat.rs:21-26`, `memory/chunker/code.rs:27-32` | Compile-time (the const-blocks ARE the test) + `memory_event_and_worker_types_are_send_sync` runtime mirror | SATISFIED |
| REQ-TEST-001 (5-turn E2E) | `memory/worker.rs:523-635` | `e2e_5_turn_chat_produces_5_rows` | PARTIAL — see WARNING #1. The test asserts count, agent_id, JOIN integrity, and project_id Some/None invariants in 3 sub-passes. It does NOT explicitly assert FIFO temporal order via `ORDER BY created_at ASC` + per-row content match (spec REQ-TEST-001 step 5). |
| REQ-TEST-002 (mandatory failure-mode coverage) | All tests listed in the spec table | All listed tests present and PASS | SATISFIED |
| REQ-TEST-003 (baseline still passes) | 249 baseline carried forward | `cargo test --lib` → 315/315 PASS (includes 249 baseline) | SATISFIED |

## Bug-hunt checklist results (Phase 3 specific)

1. **Atomicity (single `.call` closure with tx)**: ✅ Verified at `memory/store_insert.rs:115-152`. Single `.call()` invocation, `tx = c.transaction()?`, two `tx.execute()`s, `tx.commit()?`. Either both rows land or neither does.
2. **Orphan rows**: ✅ Impossible with the implementation above. If `memory_vectors` insert fails (e.g. dimension/schema issue), the transaction rolls back the `memory` insert too. The `vector_insert_failure_leaves_no_memory_row` test exercises the dimension-mismatch fast-path which returns early BEFORE opening the connection (`store_insert.rs:106-111`), so it tests the fast-path. There is no end-to-end test that injects a mid-transaction failure (e.g. dropping `memory_vectors` table BETWEEN the two INSERTs inside the same transaction). The single-connection-call architecture makes the BUG class impossible by construction.
3. **Double terminal events**: N/A — Phase 3 has no terminal events (no `done` analog). The worker emits NO `app.emit` calls. Clean.
4. **`created_by_type` semantic for Role::Tool = `'system'`**: ✅ Verified at `memory/store_insert.rs:71-72`. Test `tool_error_produces_tool_error_type_system_creator_importance_6` and `tool_success_produces_chat_message_type_system_creator` both assert `cbt == "system"`.
5. **Skip filter bypass**: ✅ Defense in depth confirmed. `MemoryEvent::from_message` filters `Role::System` AND `Role::Assistant + empty + Some(tool_calls)`. The worker's `select_chunker` ALSO returns `ChunkerKind::Skip` for `content.is_empty()` (any role) AND `Role::System` (defensive). Tests confirm both layers.
6. **Embedder NotReady → no orphan**: ✅ Verified at `memory/worker.rs:85-92`. The `embedder_cell.get()` check happens AFTER chunking but BEFORE the first `embed()` call → entire event dropped if cell empty. Test `embedder_cell_empty_no_rows_inserted` asserts 0 rows. The per-chunk `NotReady|WorkerGone` arm at `:106-113` does `return Ok(())` (abandons rest of event), preserving 1-to-1 invariant.
7. **5-turn temporal order**: ⚠️ See WARNING #1 — the E2E test asserts row count, agent_id, JOIN integrity but does NOT explicitly assert temporal order via `ORDER BY created_at ASC` against the source-sequence content.
8. **`log::` not `tracing::`**: ✅ Grep across `src-tauri/src` for `tracing::` returns ZERO matches. All Phase 3 logging uses `log::warn!`, `log::debug!`, `log::info!`. The design.md mentions `tracing::` in code samples; the apply phase correctly translated to `log::` per the orchestrator's `cargo.toml` note (Cargo.toml has no `tracing` dep).
9. **Worker survives panics**: ✅ Verified by reading `memory/worker.rs:38-124`. The outer loop body contains: `rx.recv().await` (returns Option), `process_event(...).await` (returns Result, only logged on Err), `log::warn!`. ZERO `.unwrap()` / `.expect()` / `panic!()` in the worker loop body. The few `.unwrap()` in `worker.rs` are inside `#[cfg(test)]` blocks only.
10. **CHECK constraint compliance (scope='agent', agent_id NOT NULL, project_id nullable)**: ✅ Verified. `NewMemoryRecord` has `scope: &'static str = "agent"`, `agent_id: Some(evt.agent_id.clone())`. Test `insert_agent_scope_no_project_passes_check` exercises `project_id = None` and asserts `Ok(_)` — CHECK constraint passes.
11. **`AppState::new()` tuple-return ripple**: ✅ Verified via grep — the ONLY production call to `AppState::new(` is at `lib.rs:26` which destructures: `let (state, memory_rx) = AppState::new();`. Test ctors (`new_for_test`, `new_for_test_async`, `new_for_test_with_memory_rx`) all return tuples and all test call sites destructure correctly.
12. **Cold-start tokenizer race**: ✅ Verified. `ChatChunker::new_from_base_dir` at `memory/chunker/chat.rs:31-46` uses `match Tokenizer::from_file(&path)` — on Err, falls back to `TokenSplitter::fallback()` and `log::warn!`s. NO panic. Test `tokenizer_absent_falls_back_to_byte_heuristic` constructs with `/nonexistent/path` and asserts the chunker still produces chunks.
13. **Channel send-on-dropped-receiver**: ✅ Verified at `commands/chat.rs:254-256`: `if let Err(e) = state.memory_tx.send(evt) { log::debug!(...) }` — no unwrap, no panic, no error propagation to the chat return value. Test `push_and_persist_dropped_receiver_no_panic` exercises this.
14. **Phase 2 tests still pass**: ✅ 315/315 = 249 baseline + 66 new. Phase 1+2 modules (`db.rs`, `embedder.rs`, `model_download.rs`, `lib.rs` Phase 2 tests, `state.rs` Phase 2 tests) are unchanged in behavior. Verified by reading `model_download.rs` (the only modification is the `tokenizer_path` helper added at lines 113-119 — additive only, no existing logic touched) and `state.rs` (the existing Phase 1+2 tests `new_for_test_async_opens_memory_store`, `new_for_test_panics_in_tokio_context`, `new_for_test_preseeds_fake_embedder`, `new_for_test_async_preseeds_fake_embedder`, `new_returns_empty_embedder_cell` all still appear in the file and pass per the test run).

## Findings

### CRITICAL (0)

*None.* The implementation satisfies every spec REQ at the behavior level. Atomicity holds. Failure isolation holds. No orphans possible. Worker liveness invariant upheld.

### WARNING (3)

#### W1 — REQ-TEST-001 5-turn E2E does not explicitly assert temporal ORDER BY

- **Location**: `src-tauri/src/memory/worker.rs:523-635` (`e2e_5_turn_chat_produces_5_rows`).
- **Spec**: REQ-TEST-001 step 5 — "Queries `SELECT * FROM memory ORDER BY created_at ASC` and asserts exactly 5 rows in the correct temporal order."
- **Evidence**: The test asserts `mc == 5` (count), `vc == 5` (vector count), `agent_count == 5` (all rows have agent_id='agent-test'), `mismatch == 0` (JOIN integrity), `proj_count == 5` and `null_count == 5` in the two project_id sub-passes. It does NOT query rows ORDER BY created_at and assert content equals the source sequence `("msg1", "msg2", "msg3", "msg4", "msg5")`. Practically, FIFO is guaranteed by the channel + single-consumer worker + monotonically increasing `timestamp_ms` (events sent with `(i as u64) + 1`), but the test doesn't PROVE it. A regression that, say, processed events in reverse or scrambled order would still pass this test as long as 5 rows landed.
- **Recommended fix**: add an assertion such as

```rust
let contents: Vec<String> = store.conn
    .call(|c| {
        let mut stmt = c.prepare("SELECT content FROM memory ORDER BY created_at ASC")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    })
    .await.unwrap();
assert_eq!(contents, vec!["msg1", "msg2", "msg3", "msg4", "msg5"]);
```

Non-blocking for archive — the test still confirms the FIVE-row aggregate invariant. But a future regression to ordering would slip past.

#### W2 — `Chunk.header` field name vs spec REQ-CHK-002 — tests reference `ast_header`

- **Location**: spec REQ-CHK-002 names the field `header`. Implementation also uses `header`. But the test names `chat_chunk_has_no_ast_header` and `code_chunk_has_ast_header` (in `memory/chunker/mod.rs:101-111`) refer to "ast_header" while the asserted field is `header`. Tasks.md T06-test mandated the field name `ast_header`, and an earlier-draft `Chunk` struct in tasks.md included `speaker: SpeakerKind, ast_header: Option<String>` — neither field appears in the actual `Chunk` struct.
- **Spec divergence**: The IMPLEMENTATION matches spec REQ-CHK-002 (which uses `header`, NOT `ast_header`, NOT `speaker`). The TASKS document (T06-test) used the older `ast_header` + `speaker: SpeakerKind` names that were later refined in the spec. Tests are correctly checking the right field (just with a misleading test name).
- **Recommended fix**: rename test `chat_chunk_has_no_ast_header` → `chat_chunk_has_no_header`, rename `code_chunk_has_ast_header` → `code_chunk_has_header`. Mechanical — no behavior change. Also note that `SpeakerKind` does NOT exist in the codebase; the spec's "speaker derivation" comment correctly states the speaker is `MemoryEvent.role`. The tasks.md T07-impl mentions `speaker: SpeakerKind::Agent` as a chunk default — this was correctly dropped during apply.
- **Non-blocking** — cosmetic test-name divergence. The contracts hold.

#### W3 — Spec REQ-CHK-001 routing for `MemorySource::CodeFile` not exercisable in Phase 3

- **Location**: spec REQ-CHK-001 routing table includes a row: "Future: `content` with `MemorySource::CodeFile` → `Code`". The implementation's `select_chunker` (in `memory/chunker/mod.rs:53-69`) does NOT take `MemorySource` as a parameter — it only takes `(Role, Option<bool>, &str)`. So a `MemorySource::CodeFile` event would still route to `Chat` based on its `Role`.
- **Spec divergence**: This is a known limitation acknowledged in design.md §3.2 which says "CodeFile events (Phase 4 feeder) will set Role::User + is_error=None with a marker source; that path is unreachable in Phase 3 because push_and_persist is the sole producer." But the spec REQ-CHK-001 table is misleading — it lists a routing rule that the current `select_chunker` signature cannot honor.
- **Impact**: ZERO in Phase 3 (no producer fires `MemorySource::CodeFile`). Phase 4 will need to either (a) extend `select_chunker` to take `MemorySource`, or (b) have the file-watcher producer call `code_chunker.chunk(...)` directly instead of going through the selector.
- **Recommended fix (defer to Phase 4)**: update REQ-CHK-001 spec table to clarify "CodeFile routing deferred — Phase 3 selector does not consume MemorySource"; or add a planning note in the archive report.
- **Non-blocking** — Phase 3 has no CodeFile producer, so this is forward-only debt.

### SUGGESTION (4)

#### S1 — `MemoryWorkerError::EmptyChunks` is dead code (acknowledged with `#[allow(dead_code)]`)

- **Location**: `memory/worker.rs:29-31`.
- **Observation**: The variant is documented as "Reserved for Phase 4". Leaving dead-code variants behind allow-attributes is acceptable but a fresh reader will wonder why it exists. Either wire it (e.g. `if chunks.is_empty() && !evt.content.is_empty() { return Err(EmptyChunks); }` instead of `return Ok(())` at `worker.rs:79-82`) or drop the variant and re-add when Phase 4 needs it.
- **Defer** — orchestrator judgment call.

#### S2 — `Chunk.token_count` carries `0` in fallback mode (acknowledged with `#[allow(dead_code)]`)

- **Location**: `memory/chunker/mod.rs:30-33`, `memory/chunker/token_splitter.rs:75, 82`.
- **Observation**: In byte-fallback mode every Chunk has `token_count = 0`, which is a lie (the field is supposed to "approximate" token count). If Phase 4 retrieval starts re-ranking by `token_count`, fallback-mode events will mis-rank uniformly to 0. Either remove the field or set a heuristic value (e.g. `text.len() / 4` so the byte estimate matches the cap policy).
- **Non-blocking** — Phase 3 has no consumer of `token_count`.

#### S3 — `ChunkerError` type is currently unused at runtime (acknowledged with `#[allow(dead_code)]`)

- **Location**: `memory/chunker/mod.rs:42-50`.
- **Observation**: Neither `ChatChunker` nor `CodeChunker` returns `Result<_, ChunkerError>`. Both signatures return plain `Vec<Chunk>`. Spec REQ-CHK-003 and REQ-CHK-004 prose mention `Result<Vec<Chunk>, ChunkerError>`, but the implementation chose infallible signatures (degraded fallback never returns Err). This is a deliberate, defensible deviation (no failure mode can propagate up: tokenizer error → byte fallback; parse error → byte fallback) but it makes the ChunkerError type vestigial.
- **Recommended fix (defer)**: either drop `ChunkerError` until Phase 4 needs it (and adjust spec REQ-CHK-003/-004 to match), or change the chunker signatures to actually return `Result<_, ChunkerError>` so the type isn't dead. Currently neither — spec says one thing, code says another.
- **Non-blocking** — REQ-CHK-006 tests still pass because they exercise `ChunkerError` directly via `format!` and `Box<dyn Error>` coercion.

#### S4 — `MemoryType::CodeBlock` variant unused in Phase 3

- **Location**: `memory/store_insert.rs:15`.
- **Observation**: Reserved for Phase 4+. Same pattern as S1 — defer dead-code cleanup vs leave with comment.

## Sign-off conditions

- [x] All CRITICAL fixed — there are no CRITICAL findings.
- [x] Test gate green — 315/315 PASS, 0 failures, 0 regressions, 0 new clippy warnings in Phase 3 modules.
- [x] Spec/proposal/design/tasks consistent with final impl — modulo the cosmetic divergences in W2, W3, S3 (none affect runtime behavior).
- [ ] (Optional, recommended) Decide whether W1 (E2E temporal order assertion) is worth a follow-up patch before archive or filed as a follow-up issue.

## Next steps for the orchestrator

1. **Accept-with-warnings and archive** — the WARNINGs are documentation/test-strength concerns, NOT functional defects. The atomicity, failure isolation, and Send+Sync contracts are all upheld with both compile-time and runtime evidence.
2. (Alternative) Send the 3 WARNINGs back to apply for a quick patch:
   - W1: ~10 lines added to `e2e_5_turn_chat_produces_5_rows` to assert ORDER BY content equality.
   - W2: 2 test renames in `memory/chunker/mod.rs`.
   - W3: 1-line spec table footnote update in spec.md.
3. Archive report should capture: (a) ChunkerError is currently dead-but-tested — Phase 4 to wire it; (b) MemorySource routing for CodeFile is Phase 4 work; (c) Phase 3 took the "infallible chunker" path (degraded-not-errored) which deviates from the design.md `Result<Vec<Chunk>, ChunkerError>` signatures.
