# Archive Report — rag-compaction-integration (Phase 7, RAG v1)

**Status**: PASS WITH WARNINGS  
**Execution**: Verify passed (0 CRITICAL, 1 WARNING, 2 SUGGESTION); all warnings reconciled as non-blocking  
**Test evidence**: 468/468 tests PASS, stable across 3 consecutive runs  
**Plan position**: Phase 7 — FINAL PHASE of RAG v1 (plan complete)

---

## Change Summary

**Change**: `rag-compaction-integration`  
**Domain**: Compaction (`src-tauri/src/compaction/`) + Memory Ingestion (`src-tauri/src/memory/`) + Chat Wiring (`src-tauri/src/commands/chat.rs`)  
**Scope**: 5 REQs, 10 scenarios, 8 tasks (16 sub-items incl. tests), 4 implementation batches (A–D)  
**Outcome**: Disjoint delta compaction summary now flows into the RAG memory pipeline as `SessionSummary` type records w/ fixed importance 6.0, gated by persistence success, and routed before the D8 System-role skip guard.

---

## Specs Synced

| Domain | Action | Details |
|--------|--------|---------|
| Compaction | New requirement | REQ-1: `CompactionResult` exposes `new_summary` field (disjoint delta vs. merged) |
| Memory | New requirement | REQ-2: `MemoryType::SessionSummary` variant + `as_str()` mapping |
| Memory | New requirement | REQ-3: `MemoryEvent{source:Compaction}` maps to SessionSummary/System/6.0 (source-first override) |
| Memory | New requirement | REQ-4: Worker routes Compaction events via ChatChunker before D8 System-skip guard |
| Chat Wiring | New requirement | REQ-5: Only Full compaction emits event, gated by snapshot persist success, via `build_compaction_event` pure helper |

**Result**: Delta spec fully merged into RAG v1 design. No schema changes; no new crates. All inheritance (D4 embedder-not-ready drop, existing D2/D3 MemoryEvent stability, existing score.rs/tier.rs downstream) honored.

---

## Verification Compliance (10/10 scenarios PASS)

| REQ | Scenario | Status |
|---|---|---|
| REQ-1 | First compaction: `new_summary == summary` | ✅ |
| REQ-1 | Re-compaction: `new_summary` excludes prior, `summary` merges both | ✅ |
| REQ-2 | `MemoryType::SessionSummary.as_str() == "session_summary"` | ✅ |
| REQ-3 | Compaction source → SessionSummary/System/6.0 (role overridden) | ✅ |
| REQ-4 | Compaction event → 1 session_summary row + 1 vector (role::System, not skipped) | ✅ |
| REQ-4 | Chat + System (non-Compaction) → 0 rows (D8 guard intact) | ✅ |
| REQ-5 | Full compaction sends exactly 1 Compaction event w/ matching project_id | ✅ |
| REQ-5 | Micro compaction sends 0 events | ✅ |
| REQ-5 | No-action compaction sends 0 events | ✅ |
| REQ-5 | Snapshot persist failure suppresses event | ✅ |

**Evidence**: `cargo test --lib` executed 3 times consecutively; all runs returned 468/468 PASS (456 baseline + 12 net-new). `cargo check` returned 0 errors (104 pre-existing warnings, unchanged).

---

## Warnings Reconciliation

**W1 — apply-progress format gap: no formal "TDD Cycle Evidence" table**

- **Finding**: apply-progress artifact documents the Strict TDD approach (test-first, checkpoint pattern, cargo checks) but lacks a formal table proof-of-execution.
- **Substance**: The evidence IS there: all 8 tasks list `T*-test` before `T*-impl`, with checkpoint commands (`cargo check`, `cargo test --lib`) explicitly noted after each batch, and independent real-run verification confirms 468/468 PASS across 3 consecutive runs.
- **Reconciliation**: ACCEPTED. Report format nit; the substance of Strict TDD and its execution proof both present. Non-blocking. Recommend improving the template for Phase 8+ to include a tabular Execution Summary, but Phase 7 delivery is complete.

---

## Suggestions Reconciliation

**S1 — Parameter naming: `agent_project_path` vs spec's `project_path`**

- **Finding**: Design/spec document names the new param to `try_auto_compact` as `project_path`; implementation uses `agent_project_path`.
- **Reconciliation**: ACCEPTED. Semantically identical; `agent_project_path` is the established local convention throughout chat.rs (150+ uses in the file). Param position minimized diff noise by appending as 5th/last param, not inserting mid-sequence. Cosmetic only.

**S2 — Tests are unit/integration-in-process, not Tauri-E2E**

- **Finding**: All 12 net-new tests use `AppState::new_for_test_async()` + `spawn_test_worker` in-process harness. No dedicated Tauri command-layer E2E test exists.
- **Reconciliation**: ACCEPTED. The in-process harness is the correct seam for Rust unit/integration testing in claw-garden (no E2E runner bootstraps Tauri runtime for tests). REQ-5's Full-arm path (which includes the `memory_tx.send`) is exercised in-process, not mocked. Coverage is adequate for architectural verification.

---

## Locked Design Decisions — All Honored

| Decision | Proof | Status |
|----------|-------|--------|
| **Importance fixed at 6.0** | store_insert.rs:71 hard-codes `(SessionSummary, System, 6.0)`. Test asserts exact `== 6.0`. Spec closure: not derived, not configurable. | ✅ |
| **No new importance field in MemoryEvent** | Struct unchanged; `build_compaction_event` (chat.rs:281–296) constructs only {agent_id, project_id, role, content, is_error, source, timestamp_ms}. Importance attached downstream in `from_event_and_chunk`. | ✅ |
| **Pure helper `build_compaction_event`** | Tauri-runtime-free, no `state.memory_tx` call inside. Separates event construction from sending. Unit-tested in isolation (2 tests). | ✅ |
| **Source-first branch** | store_insert.rs:70 checks `source==Compaction` BEFORE `(role,is_error)` match. worker.rs:75 checks Compaction BEFORE `select_chunker` (bypasses D8 System-skip guard). Minimal blast radius. | ✅ |
| **Delta cloned before merge** | mod.rs:86 `let delta = new_summary.clone();` BEFORE the `has_prior_summary` if/else. Else-branch moves `new_summary` into final; both delta and full summary are preserved for their separate paths. | ✅ |
| **Send gated by `persisted`** | chat.rs:190 `if persisted { ... build_compaction_event + send }`. Mirrors the existing `StreamEvent::Compaction` gate at :186. No event on snapshot failure. | ✅ |

---

## Archive Contents

| Artifact | Type | Content | Status |
|----------|------|---------|--------|
| proposal | openspec | Intent, scope, rollback plan (Phase 7 entry document) | ✅ |
| spec | openspec | 5 REQs, 10 scenarios, delta-only focused | ✅ |
| design | openspec | Technical approach, architecture decisions, data flow, file changes table, testing strategy | ✅ |
| tasks | openspec | 8 tasks (16 sub-items), Strict TDD interleaving, traceability matrix, final test count | ✅ |
| apply-progress | openspec | 4 batches (A–D), all tasks complete, inline decisions, discoveries, final status | ✅ |
| verify-report | openspec | Spec compliance matrix (10/10 scenarios PASS), coherence check, warnings/suggestions, verdict PASS WITH WARNINGS | ✅ |

**Observation IDs** (Engram topic keys, for traceability):
- Proposal: `sdd/rag-compaction-integration/proposal`
- Spec: `sdd/rag-compaction-integration/spec`
- Design: `sdd/rag-compaction-integration/design`
- Tasks: `sdd/rag-compaction-integration/tasks`
- Apply-progress: `sdd/rag-compaction-integration/apply-progress`
- Verify-report: `sdd/rag-compaction-integration/verify-report`
- **This archive-report**: `sdd/rag-compaction-integration/archive-report`

---

## File Changes Summary

| File | Lines | Action | Key Change |
|------|-------|--------|------------|
| `compaction/result.rs` | :5–14 | Modify | Add `pub new_summary: String` field |
| `compaction/mod.rs` | :82, :111 | Modify | Clone delta pre-merge, populate `new_summary` in ctor |
| `memory/store_insert.rs` | :12–16, :19–25, :65–92 | Modify | Add `SessionSummary` variant, `as_str` mapping, source-first branch in `from_event_and_chunk` |
| `memory/worker.rs` | :15, :75–77 | Modify | Add `MemorySource` to use; branch Compaction before `select_chunker` |
| `commands/chat.rs` | :54, :157, :190, :200, :281–296, :754, :1186 | Modify | Add `agent_project_path` param, `build_compaction_event` helper, capture delta, send on Full arm |

**Total**: 5 files, 12 net-new tests, 0 schema changes, 0 new crates.

---

## Key Implementation Discoveries

### Discovery: AppState::new_for_test_async() global session root
The `SessionManager` in `AppState::new_for_test_async()` ties to a PID-keyed shared temp directory, not per-test. All tests in a `cargo test` binary share one sessions root. When simulating snapshot persist failures, any attempt to delete or mutate the root would cause cross-test flakiness. **Solution**: Use `SessionManager::current_session_dir()` + a per-agent-unique blocking path (e.g., creating a DIRECTORY at the .jsonl location for agent X only) to fail `fs::rename` deterministically without touching shared state. Applied in T7 scenario; verified stable across 5 consecutive parallel runs.

### Decision: Session Summary event insertion is PROVISIONAL
The insertion of a `MemoryEvent{source:Compaction}` that flows into `SessionSummary` type records was a design decision made during Phase 7 based on the stated RAG requirement (Phase 6 / autoDream document 9: "Compaction should contribute a disjoint summary to the memory pipeline"). This is implemented and verified, but remains **PROVISIONAL** — it is intended that autoDream (the orchestrator for the next phase, if any, or external doc 9) will review the session_summary insertion rationale and confirm or adjust the importance weighting, scoring tiers, or inclusion logic. Current implementation (importance=6.0, tier-2 by type, scored normally via existing lambda_for_type and tier rules) is architecturally sound and tested. If autoDream signals a different approach (e.g., direct-to-tier-1, different importance, or alternative channel), this is the seam to modify: `from_event_and_chunk` source-first branch (store_insert.rs:70–78).

---

## Next Steps

**Phase 7 is COMPLETE** — all 5 REQs delivered, 10/10 scenarios passing, 468/468 tests stable.

**RAG v1 Plan is COMPLETE** — this was the final phase. No phases follow.

**Recommended next framing**:
1. Observe the session_summary type behavior in production (if deployed) or integration testing.
2. If autoDream or external review signals adjustment needed (importance weighting, inclusion gate, scoring tier), it is localized to `from_event_and_chunk` source-first branch.
3. If the next project initiative requires memory pipeline enhancements beyond RAG v1, frame it as a new change proposal.

---

## Sign-Off

**Archive Status**: READY  
**Verdict**: PASS WITH WARNINGS — all blockers resolved, non-blocking findings documented, ready for deployment/handoff.  
**Archived by**: sdd-archive (Phase 7 sub-agent)  
**Date**: 2026-07-07  
**Plan completion**: RAG v1 (7 phases) — COMPLETE

---

## Appendix: Warning & Suggestion Details

**W1 Details**: The apply-progress artifact uses an inline list (`[x]` checkboxes) for the Strict TDD evidence, rather than a formal proof table. This is effective for reading but doesn't match the formal template recommended for archives. The substance — all 8 tasks, test-first, checkpoints, real run verification (468/468 stable × 3) — is present and solid. Recommend Template improvement for Phase 8+: add an "Execution Evidence" section with a proof table.

**S1 Details**: In the apply-progress artifact, the param is named `agent_project_path` to match local conventions; the spec document (design phase) proposed `project_path`. Both refer to the same thing: an optional path string used to derive the project_id for the Compaction event. No semantic difference. The implementation chose the longer name to reduce confusion with other "project" references; acceptable.

**S2 Details**: All new tests use Rust #[test] in-process harness (no external E2E or Tauri runtime). This is the correct approach for claw-garden's testing architecture (Rust-first, Tauri as a deployment layer). The in-process harness fully exercises the Full-arm path (including the `memory_tx.send` call) via `AppState::new_for_test_async()` and `spawn_test_worker`. Coverage is adequate. Noted only for completeness.
