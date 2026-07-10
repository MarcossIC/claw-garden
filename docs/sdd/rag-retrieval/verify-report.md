# Phase 4 — rag-retrieval — Verify Report

**Date**: 2026-05-24
**Verdict**: ACCEPTED
**Final test count**: 390/390 PASS (verified via `cargo test --lib --manifest-path src-tauri/Cargo.toml`)
**Regressions**: 0 (baseline 315 → 390, +75 net new — matches apply claim exactly)

Per-module breakdown:

| Module | Tests | Notes |
|---|---|---|
| `memory::error` (new variants) | 3 | T01 — WorkspaceIo + Backend + Send/Sync |
| `memory::score` | 20 | T03–T08 — exp_decay, lambda_for_type, importance_factor (with clamp), dispute_factor, register + full expression |
| `memory::search` | 17 | T09–T15 — SearchScope, RankedHit, serialize_f32_query, knn_memory + filters, score formula, search_all_memories, integration |
| `memory::workspace` | 12 | T16–T20 — paths, markers, read, regenerate_auto_section, byte cap, write-failure |
| `memory::prompt_builder` | 19 | T21–T23 — date helper, format_hit_line, bucket_by_recency, fetch_active_conflicts (incl. json_each NULL), build_memory_prompt (12 branches), e2e |
| `commands::chat` (new) | 4 | T24 — compute_memory_prefix with memories / no memories / err / no embedder |
| **Phase 4 net new** | **75** | Matches apply phase report |

---

## REQ → test trace

| REQ ID | Documented test name | Actual test name | Status |
|---|---|---|---|
| REQ-SCORE-001 | `exp_decay_zero_age_returns_one` | `memory::score::tests::exp_decay_zero_age_returns_one` | PASS |
| REQ-SCORE-001 | `exp_decay_one_day_chat_lambda` | same | PASS |
| REQ-SCORE-001 | `exp_decay_seven_days_tool_error_lambda_approx_half` | same | PASS |
| REQ-SCORE-002 | `lambda_user_preference` | `memory::score::tests::lambda_user_preference` | PASS |
| REQ-SCORE-002 | `lambda_chat_message` | same | PASS |
| REQ-SCORE-002 | `lambda_tool_error` | same | PASS |
| REQ-SCORE-002 | `lambda_unknown_type_returns_default` | same | PASS |
| REQ-SCORE-003 | `importance_factor_min_value` / `max_value` / `mid_value` / `clamp_below_floor` / `clamp_above_ceiling` | all 5 present | PASS |
| REQ-SCORE-004 | `dispute_factor_active` / `disputed` / `superseded` / `invalid` / `unknown_status` | all 5 present | PASS |
| REQ-STATE-001 | `register_score_udfs_makes_exp_decay_callable` + `register_score_udfs_makes_all_four_callable` | both present | PASS |
| REQ-SEARCH-001 | `scope_all_default_values` / `scope_procedural_only_values` / `scope_factual_pivots_values` | all 3 present | PASS |
| REQ-SEARCH-002 | `ranked_hit_is_send` + `ranked_hit_sort_descending_by_score` | both present | PASS |
| REQ-SEARCH-003 | `search_returns_results_sorted_by_score_desc` / `search_respects_scope_none_skips_table` / `search_truncates_to_k_final` / `search_empty_db_returns_empty_vec` | all 4 present | PASS |
| REQ-SEARCH-004 | `tier1_rows_excluded_from_results` / `superseded_rows_excluded_from_results` / `disputed_rows_included_with_degraded_score` | all 3 present | PASS |
| REQ-SEARCH-005 | `score_formula_integration_spot_check` / `disputed_status_reduces_score_by_factor_0_6` | both present | PASS |
| REQ-SEARCH-006 | `embedder_none_skips_retrieval_no_panic` / `embedder_none_emits_workspace_only_if_present` | both present in prompt_builder | PASS |
| REQ-WORKSPACE-001 | `path_with_project_id` / `path_with_no_project_uses_default_segment` / `workspace_manager_is_send_sync` | all 3 present | PASS |
| REQ-WORKSPACE-002 | `markers_locate_auto_section_in_existing_file` / `missing_markers_creates_file_with_both_sections` | both present | PASS |
| REQ-WORKSPACE-003 | `user_section_preserved_byte_for_byte` / `creates_directory_and_file_on_first_run` / `zero_tier1_rows_produces_empty_auto_section_with_headers` / `auto_section_respects_1500_token_cap` / `write_failure_returns_workspace_io_error` | all 5 present | PASS |
| REQ-WORKSPACE-004 | `read_missing_file_returns_ok_empty` / `read_existing_file_returns_content` | both present | PASS |
| REQ-PROMPT-001 | `all_empty_returns_empty_string` / `workspace_only_returns_workspace_block` | both present | PASS |
| REQ-PROMPT-002 | `three_hits_split_into_recent_and_longterm_blocks` / `disputed_pair_renders_conflicts_block` / `no_disputed_rows_omits_conflicts_block` / `separator_strings_match_exactly` | all 4 present | PASS |
| REQ-PROMPT-003 | `recent_longterm_split_by_7day_window` / `all_old_hits_go_to_longterm_only` / `all_recent_hits_go_to_recent_only` | all 3 present | PASS |
| REQ-PROMPT-004 | `conflicts_query_zero_disputed_rows` / `conflicts_query_one_disputed_pair_renders_correctly` / `json_each_null_returns_zero_rows_no_error` | all 3 present | PASS |
| REQ-INT-001 | `stream_chat_with_memories_prepends_blocks_to_system_prompt` / `stream_chat_with_no_memories_uses_unmodified_system_prompt` / `stream_chat_retrieval_error_does_not_fail_chat` / `stream_chat_with_embedder_not_ready_does_not_panic` | all 4 present | PASS |
| REQ-INT-002 | compile-time only (mod.rs additions) | verified by `cargo check` clean; 4 `pub(crate) mod` declarations present in `memory/mod.rs` lines 46-49 | PASS |
| REQ-STATE-002 | compile-time `const _` Send+Sync block | present in `search.rs:58-62` and `workspace.rs:26-31` (NOT inside `#[test]`) | PASS |
| REQ-TEST-001 | `full_score_expression_in_sql_integration` | `memory::score::tests::full_score_expression_in_sql_integration` | PASS |
| REQ-TEST-002 | `integration_5_rows_fake_embedder_top3_sorted` | `memory::search::tests::integration_5_rows_fake_embedder_top3_sorted` | PASS |
| REQ-TEST-003 | `e2e_full_4_block_output` | `memory::prompt_builder::tests::e2e_full_4_block_output` | PASS |
| REQ-TEST-004 | meta — no sleep + 315 baseline preserved | verified via grep + final count 390/390 | PASS |

---

## D1–D9 compliance

| Decision | Verdict | Evidence |
|---|---|---|
| **D1** — `log::debug!` for retrieval latency, no bench harness | OK | `search.rs:318-326` emits `log::debug!("retrieval={}ms hits={} pool=...")`. No `benches/` dir. |
| **D2** — Pre-loop memory pre-compute, one call per `stream_chat` | OK | `chat.rs:605-613` calls `compute_memory_prefix` BEFORE the `for iteration in 0..MAX_TOOL_ITERATIONS` (line 618). Inside the loop only `system_prompt: Some(combined_system_prompt.clone())` (line 637). TODO comment present at line 614-615. |
| **D3** — `disputed_against` stays `Option<&str>`; CONFLICTS uses `json_each` SQL | OK | `memory/record.rs:42,90,138,186` still return `Option<&str>`. `prompt_builder.rs:271` uses `json_each(m_new.disputed_against)` in SQL. No typed `Vec<i64>` decode added. |
| **D4** — Always regen at `stream_chat` start; no `dirty` flag | OK | `chat.rs:443-448` calls `regenerate_auto_section` unconditionally inside `compute_memory_prefix`. `WorkspaceManager` holds only `PathBuf`, no interior state. |
| **D5** — `WHERE m.tier > 1` filter on all 3 KNN queries | OK | `search.rs:95` (`SEARCH_SQL_MEMORY`), line 117 (`SEARCH_SQL_EPISODIC`), line 139 (`SEARCH_SQL_PROCEDURAL`) each include `AND <prefix>.tier > 1`. Test `tier1_rows_excluded_from_results` confirms behavior. |
| **D6** — `register_score_udfs` in BOTH `open()` + `new_for_test()`, in one `.call()` | OK | `db.rs:60` (open) and `db.rs:76` (new_for_test) both invoke after `run_migrations`. `score.rs:11-56` uses ONE `conn.call(...)` with all four `create_scalar_function` calls inside. |
| **D7** — `dispute_factor` returns 1.0 / 0.6 / 0.0 — final, not stub | OK | `score.rs:46-53` has exact match `active=>1.0, disputed=>0.6, _=>0.0`. Tests for `active`, `disputed`, `superseded`, `invalid`, `bogus_future_status` all pass. |
| **D8** — CONFLICTS block omitted when 0 rows | OK | `prompt_builder.rs:142` gates `if !conflicts.is_empty()`. Test `no_disputed_rows_omits_conflicts_block` confirms `"ACTIVE CONFLICTS"` absent when no rows. |
| **D9** — `DEFAULT_PROJECT_SEGMENT = "default"` | OK | `workspace.rs:8` defines the const; `workspace_path` (line 40-45) uses `project_id.unwrap_or(DEFAULT_PROJECT_SEGMENT)`. |

All 9 LOCKED decisions honored without drift.

---

## Findings

### CRITICAL (0)

None. All test gates pass; all D1–D9 decisions honored; hot path is safe; no banned dependencies / patterns.

### WARNING (2)

**W-01 — `WORKSPACE` block sometimes lacks markers in the rendered prompt**
Location: `prompt_builder.rs::build_memory_prompt` + `strip_empty_workspace_markers` (lines 229-254).
Detail: When the workspace file contains the `<!-- AUTO-START -->` / `<!-- AUTO-END -->` framework but has no payload (no `\n- ` bullets and no USER content), `strip_empty_workspace_markers` returns `""`, which suppresses the entire WORKSPACE block. This is the documented behavior per design §1 ("workspace_content.is_empty() → block omitted") and is exactly what the test `workspace_only_returns_workspace_block` exercises (it injects bullets to make the block non-empty). However, the design.md §1 lifecycle diagram and spec REQ-PROMPT-002 do not explicitly describe stripping the markers themselves — they describe omitting the block when "workspace file is empty/missing". The behavior is **stricter than spec**: it also omits when the file exists but contains only structural markers with no payload. This is a defensible interpretation (an empty AUTO + empty USER carries zero signal to the LLM and burns tokens), but it is not a literal reading of the spec.
Recommendation: document this behavior explicitly in spec.md (or `docs/sdd/rag-retrieval/archive-report.md`) so Phase 5 doesn't accidentally regress when the AUTO section starts having real payload. Code is correct; spec wording could be clearer. **No code fix required.**

**W-02 — `arch_decisions` queries all decisions when `project_id` is None**
Location: `workspace.rs::fetch_auto_rows` lines 149-184.
Detail: When `project_id` is `None` (agent has no project bound — the "default" workspace), the architectural_decision SELECT omits the `AND project_id = ?` predicate and returns ALL `architectural_decision` rows for that agent_id (matching `tier=1 AND importance >= 8 AND deleted_at IS NULL`). The apply phase flagged this as an interpretation decision. **Verification**: this is a sound interpretation of D9: when the agent has no project, the workspace.md is the project-agnostic "default" workspace, so it makes sense to surface ALL architectural decisions the agent has made regardless of project. The alternative ("project_id IS NULL only") would silently hide arch decisions stored against specific projects from the default workspace, which is the opposite of useful for an agent that doesn't have a current project pinned. The design.md §2.3 does not specify the no-project branch behavior, so the apply phase made the right call. Phase 4 has zero `architectural_decision` rows in any test, so this branch has no test coverage at runtime — Phase 5+ might want to add one. **No code fix required**; flag for future test coverage.

### SUGGESTION (3)

**S-01 — `serialize_f32_query_pub` test-only re-export is well-scoped**
Location: `search.rs:70-73`. The `#[cfg(test)] pub(crate) fn serialize_f32_query_pub` is correctly gated by `#[cfg(test)]` and only re-exposes the private serializer for cross-module test use (consumed by `prompt_builder.rs::tests::three_hits_split_into_recent_and_longterm_blocks` and `e2e_full_4_block_output`). Marked here as a tracked pattern — future helpers that need cross-module test access should follow the same `#[cfg(test)] pub(crate)` shape.

**S-02 — `SEP_WORKSPACE_TEST` re-export idiom**
Location: `prompt_builder.rs:23-24`. Same pattern as S-01 but for a `const`. Used by `chat.rs::tests::stream_chat_with_memories_prepends_blocks_to_system_prompt` to assert the WORKSPACE separator appears in the combined prompt. Idiomatic; no concern.

**S-03 — `enforce_byte_cap` re-renders on every trim iteration (O(N²) on payload size)**
Location: `workspace.rs:268-309`. The recovery loop trims items one at a time and calls `render_auto_section_raw` on the entire row set each iteration. For the 200-row test fixture this is fine (sub-ms), but for very large fixtures (10k+ rows) this would be slow. Phase 4 has no production scenario producing 10k tier=1 rows for a single agent, so this is purely a Phase 5+ pre-work suggestion. No code change for Phase 4.

---

## Hot path / regression / banned-pattern checks

| Check | Status | Evidence |
|---|---|---|
| `compose_system_prompt` untouched | PASS | `prompt.rs` not in `git diff` (and not in Phase 4's file list); apply phase confirms no edit. Grep for `compose_system_prompt` finds only callers in `chat.rs` + the original definition in `prompt.rs` + a test in `agent_crud.rs`. |
| `stream_chat` does not propagate retrieval errors | PASS | `compute_memory_prefix` returns `String` (never `Result`); `build_memory_prompt(...).await.unwrap_or_default()` at `chat.rs:459-460`. Test `stream_chat_retrieval_error_does_not_fail_chat` passes. |
| Memory prefix concat is empty-or-prepended (no corruption) | PASS | `chat.rs:462-466`: empty → return `agent_system_prompt.to_string()`; non-empty → `format!("{}\n\n{}", memory_prefix, agent_system_prompt)`. No mid-string injection. |
| No `tracing::` calls added | PASS | `grep tracing::` in `src-tauri` returns zero hits. `log::` used exclusively. |
| No new runtime crates in `Cargo.toml [dependencies]` | PASS | Only `rusqlite` feature `"functions"` activated (line 46). `tempfile = "3"` correctly placed in `[dev-dependencies]` line 79 (W-Cargo check below). No new crate names in `[dependencies]` block. |
| No `tokio::time::sleep` in Phase 4 tests | PASS | Grep finds 2 hits: `chat.rs:667` (production cooldown — pre-Phase-4, NOT a test) and `hooks/runner.rs:802` (pre-Phase-4). Zero in any Phase 4 test file. |
| No `cargo build` references in tests/docs | PASS | One hit in `hooks/matcher.rs:107` is `assert!(!rule.matches("bash", "cargo build"))` — it's testing that the hooks ALLOWLIST does not match `cargo build` (i.e., the hooks system would deny it). Not a build invocation. No `cargo build` references in Phase 4 docs/tests. |
| `Send + Sync` const block NOT inside `#[test]` | PASS | `search.rs:58-62` and `workspace.rs:26-31` are top-level `const _: fn() = ...;` blocks (not in `#[cfg(test)]`). `error.rs:67-71` (`workspace_io_error_is_send_sync`) is inside `#[test]` but tests `MemoryError` directly via a runtime call — acceptable as a test, but the structural Send+Sync proofs for SearchScope/RankedHit/WorkspaceManager are correctly compile-time. |
| `register_score_udfs` single `.call()` pattern | PASS | `score.rs:11-56` uses ONE `conn.call(|c| { ...; ...; ...; ...; Ok(()) }).await` closure with all four `create_scalar_function` calls inside. |
| vec0 query shape parity with Phase 3 INSERT | PASS | Phase 3 INSERT (per `store_insert.rs`, also referenced in `db.rs::tests::vec0_insert_and_knn`) uses `WHERE embedding MATCH ?`. Phase 4 SEARCH_SQL_* uses `WHERE knn.embedding MATCH ?1 AND knn.k = ?2 AND ...`. The `k = ?` predicate is the vec0 KNN limit operator (replaces a sub-query LIMIT) — this is the correct shape for vec0 with combined WHERE filters. Tests `tier1_rows_excluded_from_results`, `search_truncates_to_k_final`, `integration_5_rows_fake_embedder_top3_sorted` confirm the queries execute and return expected results. |
| `json_each(NULL)` returns 0 rows (R-NEW-7) | PASS | Test `json_each_null_returns_zero_rows_no_error` explicitly inserts a row with `status='disputed' AND disputed_against IS NULL` and asserts `fetch_active_conflicts` returns `Ok(vec![])` without panic. |
| `message` move + `message_text_for_query` clone ordering in `stream_chat` | PASS | `chat.rs:507`: `let message_text_for_query = message.clone();` happens BEFORE the conditional `push_and_persist(... content: message, ...)` at line 511-525 (which moves `message`). At line 611 inside `compute_memory_prefix(... &message_text_for_query, ...)` we pass a reference to the clone — never the original `message`. The query embedding works correctly. |
| `tempfile` in `[dev-dependencies]` not `[dependencies]` | PASS | `Cargo.toml:79` — `tempfile = "3"` is under `[dev-dependencies]` (line 74). |
| `log::debug!` placement matches design §8 | PASS | `search.rs:318-326` is after the `try_join!` block, before `Ok(all)`. Format string matches design template: `"retrieval={}ms hits={} pool=({}+{}+{}) k_final={}"`. |
| TODO Phase 5 comment at `stream_chat` call site | PASS | `chat.rs:614-615` contains `// TODO Phase 5+: re-run retrieval per iteration when AccessTracker emits` per design §6.1. |
| Workspace tier=1 filter consistency | PASS | `workspace.rs:131-139,150-184,187-193` — all three AUTO queries filter `tier = 1` (workspace owns tier=1; retrieval excludes it per D5). Counterpart to D5 — both halves of the contract honored. |

---

## Inline decisions surfaced by apply — verified

| Decision | Orchestrator's call |
|---|---|
| `arch_decisions` all-agent fallback when `project_id=None` | **Acceptable** — see W-02. Sound interpretation of D9; spec didn't pin the no-project branch, design didn't either. Worth covering in archive-report.md. |
| `strip_empty_workspace_markers` helper | **Acceptable** — see W-01. Stricter-than-spec but defensible (saves tokens on empty workspace). Document in archive. |
| `serialize_f32_query_pub` test-exposed helper | **Acceptable** — see S-01. Correctly `#[cfg(test)] pub(crate)`; idiomatic. |
| `SEP_WORKSPACE_TEST` test-exposed const | **Acceptable** — see S-02. Same pattern as S-01; correctly scoped. |
| Extraction of `compute_memory_prefix` as a `pub(crate)` helper from `stream_chat` | **Acceptable, in fact superior** — design §6.1 inlined the block but extracting it makes it directly testable from `chat.rs::tests` without spinning up a full `stream_chat` flow (which is heavy and requires a mock provider). The 4 T24 tests are clean and fast as a result. No drift from D2: the helper is called exactly once before the loop. |

---

## Recommendation

**ARCHIVE.**

Phase 4 is functionally complete:
- All 31 REQ items have at least one passing test.
- All 9 LOCKED decisions (D1–D9) honored without drift.
- 0 CRITICAL findings; 2 WARNINGs (both documentation clarifications, no code changes required); 3 SUGGESTIONs (future-phase pre-work, no Phase 4 impact).
- Final count 390/390 PASS matches apply phase claim exactly.
- Zero regressions on the 315 baseline.
- Zero banned dependencies / patterns.
- Hot path (`stream_chat`) is safe: retrieval failures never break chat; `compose_system_prompt` untouched; memory prefix is empty-or-prepended.
- `tempfile` correctly in `[dev-dependencies]`; only `rusqlite "functions"` feature added (no new runtime crates).

The two WARNINGs surface design ambiguities (workspace marker stripping, default-project arch_decisions behavior) that the apply phase resolved sensibly. They should be captured in `docs/sdd/rag-retrieval/archive-report.md` as "decisions that landed during apply, not in proposal/design", so Phase 5+ knows what behavior is locked in.

Phase 5 (`rag-access-tracker`) is unblocked.
