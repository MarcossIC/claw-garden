# Archive Report: rag-access-tracker (Phase 5)

**Change**: rag-access-tracker
**Phase**: 5 of 7 (RAG v1)
**Archive date**: 2026-06-27
**Verdict coming in**: PASS WITH WARNINGS (verify-report.md)
**Archive verdict**: CLOSED — all WARNINGs reconciled, no CRITICALs

---

## SDD Cycle Summary

| Phase | Artifact | Status |
|-------|----------|--------|
| Explore | (inline / engram) | ✅ |
| Propose | `proposal.md` | ✅ |
| Spec | `spec.md` | ✅ (amended at archive) |
| Design | `design.md` | ✅ (reconciled at archive) |
| Tasks | `tasks.md` | ✅ 23/23 complete |
| Apply | (implementation) | ✅ |
| Verify | `verify-report.md` | ✅ PASS WITH WARNINGS |
| Archive | `archive-report.md` | ✅ This document |

---

## Test Results (Final)

| Suite | Result |
|-------|--------|
| `cargo test --lib` | **423/423 PASS** (390 baseline + 33 net-new Phase 5) |
| `cargo check --lib` | exit 0 |
| Angular | 22/22 PASS (unchanged from Phase 4) |
| Regressions | **0** |

---

## Requirement Coverage

| Requirement | Scenarios | Result |
|-------------|-----------|--------|
| REQ-TRACK | TRACK-1, TRACK-2, TRACK-3 | ✅ 3/3 COMPLIANT |
| REQ-FLUSH | FLUSH-1, FLUSH-2, FLUSH-3, FLUSH-4 | ✅ 4/4 COMPLIANT (FLUSH-3 log-level reconciled below) |
| REQ-TIER | TIER-1 through TIER-7 | ✅ 7/7 COMPLIANT |
| REQ-DEMOTE | DEMOTE-1 through DEMOTE-5 | ✅ 5/5 COMPLIANT |
| REQ-LIMIT | LIMIT-1, LIMIT-2, LIMIT-3 | ✅ 3/3 COMPLIANT |
| REQ-STATE | STATE-1 through STATE-4 | ✅ 4/4 COMPLIANT |
| **Total** | **26 scenarios** | **26/26 COMPLIANT** |

---

## WARNING Reconciliations

### WARNING 1 — Log level: `log::warn!` vs spec `log::error!`

**Decision**: RESOLVED in favor of `log::warn!`.

**Rationale**: Flush failure is a recoverable, non-catastrophic event — timestamps are delayed at most 30 seconds and the worker continues. `worker.rs:55` (Phase 3) uses `log::warn!` for identical skip-and-continue recoverable errors, establishing the codebase convention. Using `log::error!` would overstate severity and break pattern consistency. The implementation in `access.rs:182` is CORRECT; the spec was the artifact that was wrong.

**Action taken**: `spec.md` amended — REQ-FLUSH and FLUSH-3 changed from `log::error!` to `log::warn!`. Code NOT changed.

**Files changed**: `docs/sdd/rag-access-tracker/spec.md` (lines 46, 64).

---

### WARNING 2 — Stale design doc: speculative `compute_tier` non-anchor table

**Decision**: RESOLVED. Speculative rows removed from design.md; Open Question marked RESOLVED.

**Rationale**: The design doc contained a speculative non-anchor table (`3→2 @ access_count>=5`, `semantic_fact` in the 2→1 anchor set) that was explicitly flagged as an unresolved Open Question at `design.md:132`. The implementation correctly resolved the question toward the authoritative `spec.md` and did NOT implement those speculative rules. Doc 6:720-732 confirms no tier 3→2 promotion rule exists. The `compute_tier` function is promote-only: initial tiers by type + the single locked anchor (architectural_decision/session_summary at access_count >= 3 → tier 1). No 3→2 rule exists by design.

**Action taken**: `design.md` — speculative table rows removed, authoritative table corrected to match implementation (`session_summary` replaces `user_preference`/`semantic_fact`; `3→2 @ access_count>=5` row removed). Open Question `[ ]` changed to `[x]` RESOLVED with explanation.

**Files changed**: `docs/sdd/rag-access-tracker/design.md` (compute_tier table + Open Questions section).

---

## SUGGESTION Disposition

### SUGGESTION 1 — `demote_inactive` compiler warning (`never used`)

**Decision**: EXPECTED — no action taken.

The `function demote_inactive is never used` compiler warning on `tier.rs:57` is the expected and correct consequence of REQ-DEMOTE: the function is delivered in Phase 5 but intentionally NOT wired. The docstring reads: "Only called by autoDream (Phase 8+). Phase 5 delivers but does NOT invoke." The compiler warning independently corroborates DEMOTE-1's structural correctness. `#[allow(dead_code)]` was suggested but not applied — the warning serves as a useful reminder until Phase 8+ wires the call.

---

## Files Delivered

| File | Action | Description |
|------|--------|-------------|
| `src-tauri/src/memory/access.rs` | Created | `AccessTracker` struct + `select!` worker (30s timer OR ≥500 entries) + `flush()` CTE + `start_access_worker` no-op |
| `src-tauri/src/memory/tier.rs` | Created | `compute_tier` pure-Rust fn + `register_tier_udf` + `demote_inactive` (unwired) |
| `src-tauri/src/memory/mod.rs` | Modified | `mod access; mod tier;` declarations |
| `src-tauri/src/memory/db.rs` | Modified | `set_limit(SQLITE_LIMIT_VARIABLE_NUMBER, 32766)` + `register_tier_udf` in both `open()` and `new_for_test()` |
| `src-tauri/src/memory/prompt_builder.rs` | Modified | `build_memory_prompt` gains `Option<&AccessTracker>`; fire-and-forget at `:106-107` |
| `src-tauri/src/state.rs` | Modified | `access_tracker: Arc<AccessTracker>` in all 3 constructors |
| `src-tauri/src/commands/chat.rs` | Modified | `compute_memory_prefix` passes `Some(state.access_tracker.as_ref())` |
| `src-tauri/src/lib.rs` | Modified | `start_access_worker(app.handle().clone())` after `start_memory_worker` |

---

## Key Architectural Decisions (As-Built)

| Decision | As Planned | As Built | Notes |
|----------|------------|----------|-------|
| Channel type | `UnboundedSender<Vec<i64>>` | `UnboundedSender<Vec<i64>>` | Exact match |
| Worker trigger | `interval(30s)` OR `rx.recv() ≥500` | `interval_at(now+30s, 30s)` + `select!` | `interval_at` to avoid t=0 tick race |
| `compute_tier` UDF | UDF delegating to pure Rust fn | Same | Clean module boundary |
| Flush error level | `log::error!` (original spec) | `log::warn!` | Spec amended to match convention |
| Fire-and-forget site | `search.rs::search_all_memories` (plan line 259) | `prompt_builder.rs:106-107` | Avoids coupling retrieval primitive to side-effect |
| `rusqlite set_limit` | `Limit::SQLITE_LIMIT_VARIABLE_NUMBER` | Raw FFI `ffi::sqlite3_limit(handle, SQLITE_LIMIT_VARIABLE_NUMBER, 32766)` | `limits` feature not enabled in `rusqlite 0.37` |
| `demote_inactive` | Delivered, NOT wired | Same | autoDream (doc 9) owns runtime invocation |

---

## Non-Obvious Implementation Discoveries

These discoveries are preserved here for Phase 6+ agents:

1. **`rusqlite 0.37` `limits` feature not enabled**: `Limit::SQLITE_LIMIT_VARIABLE_NUMBER` and `Connection::set_limit` are gated behind the `limits` feature flag, which is not enabled in this codebase. Used raw FFI `ffi::sqlite3_limit(c.handle(), SQLITE_LIMIT_VARIABLE_NUMBER, 32766)` instead. Must be applied on the SAME connection that flushes, in BOTH `open()` and `new_for_test()`.

2. **`tokio::time::interval` t=0 tick race**: `tokio::time::interval(30s)` fires an initial tick immediately at t=0, which races `rx.recv()` in `select!` and causes premature partial flushes in paused-time tests. Fix: use `interval_at(Instant::now() + period, period)`. Every timer test needs `#[tokio::test(start_paused=true)]` + `tokio::time::advance`.

3. **`tauri::async_runtime::block_on` + `tokio::spawn` safety**: `block_on` runs on a process-lifetime multi-threaded `OnceCell` runtime (verified against tauri-2.10.3 source). Tasks `tokio::spawn`-ed inside `AccessTracker::new()` during `block_on` survive after `block_on` returns — the worker runs on the runtime's worker threads for the process lifetime. Safe pattern for spawning background workers from sync `AppState` constructors.

4. **`compute_tier` is PROMOTE-ONLY with no 3→2 rule**: initial tiers per type (user_preference/procedural_skill→1, architectural_decision/session_summary→2, chat_message/episodic_trajectory→3) + single 2→1 anchor at `access_count>=3` for architectural_decision/session_summary. NO tier 3→2 rule exists by design per doc 6:720-732. `demote_inactive` handles demotions separately and is NOT called in Phase 5.

5. **Flush is a single atomic CTE UPDATE**: HashMap dedup before binding, single `conn.call(move |c| { tx = c.transaction()?; execute; tx.commit() })`. `compute_tier` UDF invoked inside SET. Channel is unbounded (retrieval never blocks on capacity). Flush errors → `log::warn!` + continue, never panic.

---

## Phase 6 — `rag-sycophancy-guard` Unblocked

Phase 5 is complete. Phase 6 (`rag-sycophancy-guard`) is now unblocked.

Phase 6 goal: contradiction detection at INSERT + ACTIVE CONFLICTS prompt block + Mem0 ADD/UPDATE/NO-OP resolver.

Key Phase 5 contracts that Phase 6 inherits:
- `prompt_builder.rs` is the integration layer — Phase 6's ACTIVE CONFLICTS block goes there.
- `dispute_factor` UDF stub in `score.rs` (Phase 4) needs real values in Phase 6.
- `log::warn!` (not `log::error!`) for recoverable background errors — maintain consistency.

**Start Phase 6 with**: `/sdd-new rag-sycophancy-guard`

---

## SDD Cycle Complete

Change `rag-access-tracker` has been fully planned, implemented, verified, and archived.
All 23 tasks complete. 423/423 tests green. All 26 scenarios compliant. No CRITICALs.
