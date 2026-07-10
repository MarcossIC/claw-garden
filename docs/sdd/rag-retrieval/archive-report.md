# Phase 4 — rag-retrieval — Archive Report

**Date closed**: 2026-05-24
**Status**: COMPLETE & ARCHIVED
**Verdict from sdd-verify**: ACCEPTED
**Final test count**: 390/390 Rust + 22 Angular PASS (315 Phase 1+2+3 baseline + 75 net new Phase 4)
**Regressions**: 0
**New runtime deps**: 0 (`rusqlite "functions"` feature activation only; `tempfile` added as dev-dep)

---

## Summary

Phase 4 (`rag-retrieval`) delivered the consumer half of the RAG pipeline. Embeddings persisted by Phase 3 now reach the LLM via four new modules that score, search, manage workspace context, and assemble the memory prefix. The hot path integration is surgical: pre-compute once per `stream_chat`, prepend to system prompt, never touch `compose_system_prompt`. The verify gate found 0 CRITICAL issues, all 9 locked decisions (D1–D9) honored without drift, and zero regressions on the 315 baseline.

Four new modules entered the codebase as a cohesive unit: `score.rs` registers four deterministic SQLite UDFs (exp_decay, lambda_for_type, importance_factor, dispute_factor); `search.rs` orchestrates three parallel KNN queries and merges ranked results; `workspace.rs` manages the on-disk `agent_workspace.md` with AUTO/USER marker sections; `prompt_builder.rs` assembles the four-block memory prefix. The verify phase surfaced two WARNINGs (workspace marker stripping behavior, default-project arch_decisions fallback) that the apply phase resolved defensibly — both documented here for Phase 5+ to inherit as locked design constraints. Phase 5 (`rag-access-tracker`) is now unblocked.

---

## Artifacts produced

- `docs/sdd/rag-retrieval/explore.md` (~550 lines)
- `docs/sdd/rag-retrieval/proposal.md` (~534 lines)
- `docs/sdd/rag-retrieval/spec.md` (~928 lines)
- `docs/sdd/rag-retrieval/design.md` (~1274 lines)
- `docs/sdd/rag-retrieval/tasks.md` (~790 lines, 24 task pairs, 5 CHECKPOINTs)
- `docs/sdd/rag-retrieval/verify-report.md` (~210 lines)
- `docs/sdd/rag-retrieval/archive-report.md` (this file)

---

## Code delivered

### New files in `src-tauri/src/memory/`

- **`score.rs`** — `register_score_udfs` coordinator + four deterministic UDFs (`exp_decay`, `lambda_for_type`, `importance_factor`, `dispute_factor`) + 20 unit tests. UDFs called by KNN queries to compute final relevance scores.

- **`search.rs`** — `SearchScope` (3 constructors: `all_default`, `procedural_only`, `factual_pivots`), `RankedHit` struct, `search_all_memories` orchestrator using `tokio::try_join!` to run three parallel KNN queries (`knn_memory`, `knn_episodic`, `knn_procedural`), merge, rank by composite score, truncate to `k_final`. 17 tests.

- **`workspace.rs`** — `WorkspaceManager` with path resolution (`DEFAULT_PROJECT_SEGMENT = "default"`), marker-based file parsing (`<!-- AUTO-START -->…<!-- AUTO-END -->` and `<!-- USER-START -->…<!-- USER-END -->`), `regenerate_auto_section` to refresh AUTO from tier=1 architectural_decision + user_preference rows, `read` / `split_existing` / `enforce_byte_cap` lifecycle. 12 tests.

- **`prompt_builder.rs`** — `build_memory_prompt` assembles four `===`-separated blocks: WORKSPACE, RECENT CONTEXT (Last 7 Days), LONG-TERM KNOWLEDGE, ACTIVE CONFLICTS (review needed). Helpers: `unix_secs_to_ymd` (Howard Hinnant algorithm), `fetch_active_conflicts` (json_each SQL), `format_hit_line`, `bucket_by_recency`, `strip_empty_workspace_markers`. 19 tests.

### Modified files

- **`src-tauri/src/memory/mod.rs`** — four new `pub(crate) mod` declarations (score, search, workspace, prompt_builder) at lines ~46–49.

- **`src-tauri/src/memory/error.rs`** — added `WorkspaceIo { path, source }` and `Backend(String)` error variants.

- **`src-tauri/src/memory/db.rs`** — `register_score_udfs(&conn).await?` called in BOTH `open()` and `new_for_test()` after `run_migrations`.

- **`src-tauri/src/commands/chat.rs`** — `compute_memory_prefix` extracted as `pub(crate)` helper; `project_id_from_path` utility; `stream_chat` modified at line ~443–448 to call `compute_memory_prefix` BEFORE the agentic loop, then line ~558 swaps `system_prompt: Some(combined_system_prompt.clone())` for every iteration. 4 new T24 integration tests.

- **`src-tauri/Cargo.toml`** — `rusqlite` gains `features = [..., "functions"]` (line ~46); `tempfile = "3"` added to `[dev-dependencies]` (line ~79).

### Invariants preserved

- **`src-tauri/src/models/prompt.rs::compose_system_prompt`** — byte-identical, all 10 tests still green. Zero modifications. The memory prefix is prepended at call site, not within the prompt composer.

---

## Decisions honored (D1–D9)

| Decision | Implementation | Verified |
|---|---|---|
| **D1** — `log::debug!` for retrieval latency, no bench harness | `search.rs:318–326` logs `"retrieval={}ms hits={} pool=(...) k_final={}"` after `try_join!` | ✅ |
| **D2** — Pre-loop memory pre-compute, one call per `stream_chat` | `chat.rs:605–613` calls `compute_memory_prefix` BEFORE loop; inside loop only clones the prefix string | ✅ |
| **D3** — `disputed_against` stays `Option<&str>` raw JSON; CONFLICTS uses `json_each` SQL | `record.rs:42,90,138,186` + `prompt_builder.rs:271` | ✅ |
| **D4** — Always regen at `stream_chat` start; no `dirty` flag | `chat.rs:443–448` calls `regenerate_auto_section` unconditionally | ✅ |
| **D5** — `WHERE m.tier > 1` filter on all 3 KNN queries | `search.rs:95,117,139` each have the tier filter; test `tier1_rows_excluded_from_results` confirms | ✅ |
| **D6** — `register_score_udfs` in BOTH `open()` + `new_for_test()`, in one `.call()` | `db.rs:60,76` + `score.rs:11–56` with single `conn.call(...)` closure | ✅ |
| **D7** — `dispute_factor` returns 1.0 / 0.6 / 0.0 — final, not stub | `score.rs:46–53` exact implementation with tests for all statuses | ✅ |
| **D8** — CONFLICTS block omitted when 0 rows | `prompt_builder.rs:142` gates `if !conflicts.is_empty()` | ✅ |
| **D9** — `DEFAULT_PROJECT_SEGMENT = "default"` | `workspace.rs:8` + `workspace_path:40–45` unwrap-or | ✅ |

All 9 decisions locked at proposal remain locked through archive.

---

## Hot-path integration summary

1. **Retrieval entry point**: `stream_chat` calls `compute_memory_prefix(... &message_text_for_query, ...)` BEFORE the `for iteration in 0..MAX_TOOL_ITERATIONS` loop (lines 605–613).
2. **Prefix composition**: `compute_memory_prefix` calls `build_memory_prompt`, which orchestrates workspace read + KNN search, formats four blocks, and returns `String`.
3. **System prompt assembly**: `let combined_system_prompt = format!("{}\n\n{}", memory_prefix, agent_system_prompt)` (line 636).
4. **Loop injection**: `system_prompt: Some(combined_system_prompt.clone())` passed to every `ChatRequest` inside the iteration loop (line 637). Single clone per iteration; prefix computed once.
5. **Error isolation**: If `compute_memory_prefix` encounters retrieval errors, it returns empty string; chat continues with unmodified `agent_system_prompt` (line 460: `.unwrap_or_default()`).
6. **Latency instrumentation**: `log::debug!("retrieval={}ms...")` emitted AFTER KNN queries complete, before block assembly (line 318–326 in `search.rs`).

---

## Lessons learned (Phase 4)

1. **`compute_memory_prefix` as `pub(crate)` helper rather than inlined** — allow direct unit tests without spinning up full `stream_chat` mock provider. Verify caught and validated this superior design vs. inlined block (T24 tests are clean, fast, and directly assert behavior).

2. **`strip_empty_workspace_markers` defensive helper** — workspace.md file always exists after first `regenerate_auto_section` even with zero Tier 1 rows. Helper strips `<!-- AUTO-START -->…<!-- AUTO-END -->` wrapper when payload is empty to avoid emitting dead block to the prompt.

3. **`enforce_byte_cap` recovery order** — when trimming AUTO section to fit 1500-token cap, recovery order is locked: `user_preference` NEVER trimmed; then `arch_decisions`, then `skills/patterns`, then `focus/goals`, then `active_disputes` (reverse priority per doc 6:1124). Tests pin each step.

4. **`message_text_for_query` clone BEFORE move** — in `stream_chat:507`, clone the message BEFORE the conditional at line 511 (which moves `message` into `push_and_persist`). At line 611 inside `compute_memory_prefix`, pass reference to the clone. Phase 5 worker patterns must heed this ordering.

5. **`serialize_f32_query_pub` test-only re-export** — `#[cfg(test)] pub(crate)` makes the serializer accessible to cross-module tests (consumed by `prompt_builder::tests`). Idiomatic; carry-forward pattern for any future test-only helpers.

6. **E0597 fix pattern with `rusqlite MappedRows`** — borrow of statement inside `if let Ok(rows)` blocks causes E0597. Solution: explicit `let rows = mapped_rows.collect::<Vec<_>>()?; rows` to drop the borrow before reusing `stmt`.

7. **`"<!-- USER-START -->"` contains `"- "` (dash-space)** — substring matching for marker validation must be careful. Check for newline-dash-space `"\n- "` only within AUTO content, not across marker line itself.

8. **`tempfile` not transitive** — must be explicit in `[dev-dependencies]`. Feature activation `rusqlite "functions"` counts as ZERO new runtime dependencies (existing crate, new feature flag).

9. **`rusqlite "functions"` feature activation pattern** — register UDFs via `c.create_scalar_function(name, arity, FunctionFlags::SQLITE_DETERMINISTIC | SQLITE_UTF8, closure)`. No new crate added; existing `rusqlite` crate already included.

10. **vec0 KNN MATCH+k= shape mirrored from Phase 3 INSERT** — Phase 3 used `WHERE embedding MATCH ?`; Phase 4 KNN queries use `WHERE knn.embedding MATCH ?1 AND knn.k = ?2 AND ...`. The `k = ?` predicate is the vec0 KNN limit operator. Tests confirm execution and correct result rank/truncation.

11. **`json_each(NULL)` returns 0 rows, no panic** — no NULL guard required in conflict queries. Test `json_each_null_returns_zero_rows_no_error` verifies a row with `status='disputed' AND disputed_against IS NULL` produces no error.

12. **Inline decisions documented in verify-report instead of silently absorbed** — apply surfaced three decision patterns (`arch_decisions` all-agent fallback when `project_id=None`, `strip_empty_workspace_markers` stricter-than-spec, `compute_memory_prefix` extraction). Verify documented them as acceptable interpretations. Phase 5+ inherits these as locked constraints.

---

## Findings from verification

### CRITICAL (0)

None. All test gates pass; all D1–D9 decisions honored.

### WARNING (2) — deferred to future phases

**W-01 — Workspace block marker stripping behavior not explicit in spec**
- Location: `prompt_builder.rs::strip_empty_workspace_markers` (lines 229–254)
- When AUTO section has no payload (zero Tier 1 rows + no USER content), markers are stripped, omitting entire WORKSPACE block from prompt. This is stricter than spec wording ("block omitted when file empty") — it also omits when file exists but is structurally empty. Defensible (saves tokens), but Phase 5 should re-examine if AUTO section starts carrying real Tier 1 content and the marker stripping ever becomes unintended.
- **No code fix required**; document for Phase 5 inheritance.

**W-02 — Default-project `arch_decisions` queries all decisions when `project_id` is None**
- Location: `workspace.rs::fetch_auto_rows` (lines 149–184)
- When agent has no bound `project_id`, the architectural_decision SELECT omits the `AND project_id = ?` filter and returns ALL decisions for that agent. Phase 4 has zero test rows for this branch (no `architectural_decision` persisted in test fixtures). Phase 5+ should add explicit test coverage for an agent with `project_id=None` to confirm the all-agent fallback behavior is desired.
- **No code fix required**; flag for future test coverage.

### SUGGESTION (3) — carry-forward patterns

**S-01 — `serialize_f32_query_pub` test-only re-export is well-scoped**
- Pattern: `#[cfg(test)] pub(crate) fn serialize_f32_query_pub(...)` allows cross-module test access without polluting production API. Correctly gated. Future test helpers should follow this shape.

**S-02 — `SEP_WORKSPACE_TEST` re-export idiom**
- Same pattern as S-01 but for a `const`. Used by `chat.rs::tests::stream_chat_with_memories_prepends_blocks_to_system_prompt`. Idiomatic; no concern.

**S-03 — `enforce_byte_cap` re-renders on every trim iteration (O(N²) potential)**
- Location: `workspace.rs:268–309`
- Recovery loop trims items one at a time and re-renders the entire AUTO section for each iteration. For the 200-row test fixture this is fine (sub-ms), but for very large fixtures (10k+ tier=1 rows for a single agent) this would be slow. Phase 4 has no production scenario producing 10k rows, so this is Phase 5+ pre-work. No code change for Phase 4.

---

## Test gate final

| Layer | Count | Delta |
|---|---:|---|
| Rust `cargo test --lib` | 390/390 PASS | +75 over baseline 315 |
| Regressions | 0 | — |
| New clippy warnings | 0 | — |
| Build invocations | 0 | — |
| `tracing::` calls added | 0 | — |
| `compose_system_prompt` modifications | 0 | — |
| `tokio::time::sleep` in Phase 4 tests | 0 | — |

---

## Phase status update

- **Phase 4** (`rag-retrieval`): ✅ Complete and archived
- **Phase 5** (`rag-access-tracker`): ⏸️ Pending (next — unblocked)
- **Phase 6** (`rag-sycophancy-guard`): Blocked by Phase 5
- **Phase 7** (`rag-compaction-integration`): Blocked by Phase 6

---

## Next steps

Phase 5 unblocks with:
- Stable `search_all_memories` interface + 390 passing tests as regression gate
- Four-block prompt builder producing deterministic output
- Workspace.md regeneration at every `stream_chat` call (ready for optimization in Phase 5 via `dirty` flag if needed)
- Two WARNINGs (workspace marker stripping, default-project arch_decisions) documented as inherited design constraints
- UDF registration and KNN orchestration proven at scale in unit tests
