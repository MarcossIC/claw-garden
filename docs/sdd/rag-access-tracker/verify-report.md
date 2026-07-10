# Verification Report: rag-access-tracker (Phase 5)

**Change**: rag-access-tracker
**Version**: spec.md (REQ-TRACK, REQ-FLUSH, REQ-TIER, REQ-DEMOTE, REQ-LIMIT, REQ-STATE)
**Mode**: Strict TDD
**Date**: 2026-06-27
**Verdict**: **PASS WITH WARNINGS**

---

## Completeness

| Metric | Value |
|--------|-------|
| Tasks total | 23 |
| Tasks complete | 23 |
| Tasks incomplete | 0 |

All 11 test/impl pairs + final verification (`T-12-verify`) checked `[x]`.

---

## Build & Tests Execution

**`cargo test --lib`**: `test result: ok. 423 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out` (exit 0)

Baseline was 390 (per STATE-4); +33 net-new tracker tests → 423, zero regressions.

**`cargo check --lib`**: exit 0. Many PRE-EXISTING baseline warnings in unrelated modules (config, hooks, mcp, tools, recovery — all dead-code / unused-import noise predating this change). Exactly ONE warning originates in a Phase-5 touched file: `function demote_inactive is never used` (`tier.rs:57`) — this is the EXPECTED and DESIRED consequence of REQ-DEMOTE (delivered but not wired in Phase 5). No new errors. `cargo build` was NOT run (per constraints).

**Coverage**: Not configured — not available.

---

## Adversarial Scrutiny (high-risk points the apply surfaced)

### 1. `block_on` + `tokio::spawn` lifetime — the #1 concern → **SAFE (real issue ruled out)**

`AccessTracker::new()` (`access.rs:47-87`) calls `tokio::spawn` for the flush worker. In production (`state.rs:161-165`) and `new_for_test` (`state.rs:247-251`) this runs inside `tauri::async_runtime::block_on`.

Verified against the actual dependency source (`tauri-2.10.3/src/async_runtime.rs`):
- `default_runtime()` (line 214) builds `Runtime::Tokio(TokioRuntime::new().unwrap())`. `tokio::runtime::Runtime::new()` is a **multi-threaded** runtime (`new_multi_thread().enable_all()`).
- It is stored in a process-lifetime `OnceCell` (`RUNTIME.get_or_init`) — never dropped.
- `block_on` → `r.block_on(task)` on the multi-thread runtime. `tokio::spawn` inside the future schedules onto the runtime's **worker threads**, which keep polling the task **after `block_on` returns**, for the process lifetime.

Conclusion: the spawned flush worker genuinely SURVIVES `block_on` and keeps running in production. This would only be a silent failure if tauri used a current-thread runtime — it does not. **No defect.**

### 2. `start_access_worker` no-op → **HARMLESS (confirmed)**

The spawn genuinely happens inside `AccessTracker::new()` (`access.rs:50`), invoked during `AppState::new()`. `lib.rs:106-108` `start_access_worker` is an intentional no-op for lifecycle parity with `start_memory_worker`, with an accurate docstring. It is NOT a silently-missing spawn. **No defect.**

### 3. Fire-and-forget never blocks/panics the hot path → **CONFIRMED**

`record()` (`access.rs:93-98`) is synchronous (no `await`), early-returns on empty `Vec` (TRACK-2), and does `self.tx.send(ids).ok()`. `UnboundedSender::send` never blocks (unbounded → never "full") and returns a `Result` swallowed by `.ok()` — a closed channel (dead worker) yields `Err` that is discarded, never a panic. `build_memory_prompt` (`prompt_builder.rs:106-107`) cannot panic from this path. **No defect.**

### 4. `set_limit` via raw FFI on the flushing connection → **CONFIRMED**

`sqlite3_limit(c.handle(), SQLITE_LIMIT_VARIABLE_NUMBER, 32766)` is applied inside `conn.call(...)` in BOTH `MemoryStore::open()` (`db.rs:65-77`) and `new_for_test()` (`db.rs:96-108`), after `register_tier_udf`. `tokio_rusqlite::Connection` is a channel handle to ONE background-thread rusqlite connection; `memory_store.conn.clone()` (passed to `AccessTracker::new`) shares the SAME underlying connection. The limit therefore governs the exact connection the flush uses. Proven by `limit_variable_number_set_in_open_path` + `..new_for_test_path` (both assert 32766) and `flush_500_distinct_ids_respects_variable_limit` (1500 bind vars succeed). **No defect.**

### 5. Atomic CTE flush → **CONFIRMED**

`flush()` (`access.rs:115-184`): HashMap dedup (`access.rs:127-130`) runs BEFORE param binding; a single `conn.call(move |c| { let tx = c.transaction()?; …execute…; tx.commit() })` (`access.rs:170-178`) — one `.call()`, one transaction. `compute_tier` UDF is registered on the same connection (`db.rs:61` / `:94`). `memory_vectors` is untouched (tier/access_count live only on `memory`), so the 1:1 `memory ↔ memory_vectors` invariant holds trivially. SET clause matches spec: `last_accessed_at = MAX(...)`, `access_count = stored + hits`, `tier = compute_tier(...)`. Proven by `flush_promotes_tier_atomically_in_same_cte_statement`. **No defect.**

### 6. `compute_tier` fidelity & promote-only invariant → **CONFIRMED (matches spec; no undocumented 3→2 rule)**

`compute_tier` (`tier.rs:19-49`) matches the spec.md truth table EXACTLY:
- `user_preference` / `procedural_skill` → 1 (TIER-4)
- `architectural_decision` / `session_summary` → 2 at INSERT, → 1 when `access_count >= 3` (TIER-5, TIER-1)
- `chat_message` / `episodic_trajectory` / unknown → `current` unchanged (TIER-6, TIER-7)
- `importance >= 9.0` → `current.min(1)` fast-track

Promote-only invariant holds: every branch uses `current.min(target)` or returns `current`, so the result is NEVER numerically greater than `current`. **No undocumented tier 3→2 rule was invented** — `chat_message`/`episodic_trajectory` at tier 3 stay at 3. (Note: `design.md` lines 59-66 contained a SPECULATIVE non-anchor table — `3 → 2 @ access_count>=5`, `semantic_fact` in the 2→1 set — explicitly flagged as an unresolved Open Question at `design.md:132`. The implementation correctly resolved that question toward the authoritative spec and did NOT implement those speculative rules. See Coherence + WARNING below.) **No defect.**

### 7. `demote_inactive` delivered but NOT wired → **CONFIRMED**

Codebase-wide grep: `demote_inactive` is referenced ONLY in `tier.rs` (definition at `:57`, two doc-comment mentions, and unit tests at `:215-251`). NO flush/worker/production path references it. The compiler independently confirms this — `cargo check` reports `function demote_inactive is never used`. Docstring (`tier.rs:53`) reads verbatim: *"Only called by autoDream (Phase 8+). Phase 5 delivers but does NOT invoke."* DEMOTE-1 (the function is unwired during flush) holds. **No defect.**

### 8. `log::warn!` vs spec's `log::error!` → **WARNING (real, non-blocking)**

REQ-FLUSH (`spec.md:46`) and FLUSH-3 (`spec.md:64`) require flush errors be logged via `log::error!`. The implementation uses `log::warn!` (`access.rs:182`). The FLUSH-3 test (`flush_db_error_does_not_panic_worker`) only asserts no-panic / worker survival — it does NOT assert the log level, so the mismatch passes undetected. `tasks.md` T-08-impl itself instructed `log::warn!`, i.e. tasks contradicted the spec and the impl followed tasks. Severity-only deviation; no behavioral impact. See WARNING #1.

### 9. No real-time sleeps in tests → **CONFIRMED**

`timer_triggers_flush_after_30_seconds` uses `#[tokio::test(start_paused = true)]` + `tokio::time::advance(31s)`. `size_trigger_at_500_entries_flushes_immediately` uses `start_paused = true` (no `advance`, so only the size trigger can fire — deterministic). Grep for `thread::sleep` / `time::sleep` in `access.rs`: **no matches**. Tests drive the worker via `tokio::task::yield_now()`. **No defect.**

---

## Spec Compliance Matrix

| Requirement | Scenario | Test | Result |
|-------------|----------|------|--------|
| REQ-TRACK | TRACK-1 fire-and-forget | `access.rs > record_nonempty_vec_sends_to_channel`, `state.rs > t10_..accepts_record` | ✅ COMPLIANT |
| REQ-TRACK | TRACK-2 empty no-op | `access.rs > record_empty_vec_sends_nothing_to_channel` | ✅ COMPLIANT |
| REQ-TRACK | TRACK-3 1000 concurrent | `access.rs > concurrent_1000_record_calls_no_stall` | ✅ COMPLIANT |
| REQ-FLUSH | FLUSH-1 timer trigger | `access.rs > timer_triggers_flush_after_30_seconds` | ✅ COMPLIANT |
| REQ-FLUSH | FLUSH-2 size@500 | `access.rs > size_trigger_at_500_entries_flushes_immediately` | ✅ COMPLIANT |
| REQ-FLUSH | FLUSH-3 DB error recovery | `access.rs > flush_db_error_does_not_panic_worker` | ⚠️ PARTIAL (no-panic proven; `log::error!` level NOT enforced + impl uses `warn`) |
| REQ-FLUSH | FLUSH-4 tier in same statement | `access.rs > flush_promotes_tier_atomically_in_same_cte_statement` | ✅ COMPLIANT |
| REQ-TIER | TIER-1 arch_decision 2→1 @ flush | `tier.rs > compute_tier_architectural_decision_access_count_3_promotes_to_tier_1` + flush test | ✅ COMPLIANT |
| REQ-TIER | TIER-2 promote invariant | `tier.rs > compute_tier_promote_only_chat_message_at_tier_1_stays_1` | ✅ COMPLIANT |
| REQ-TIER | TIER-3 pure-Rust / UDF callable | `tier.rs > register_tier_udf_makes_compute_tier_callable_via_sql` | ✅ COMPLIANT |
| REQ-TIER | TIER-4 INSERT tier-1 types | `tier.rs > compute_tier_user_preference_/_procedural_skill_inserts_at_tier_1` | ✅ COMPLIANT |
| REQ-TIER | TIER-5 INSERT tier-2 types | `tier.rs > ..architectural_decision_/_session_summary_inserts_at_tier_2` | ✅ COMPLIANT |
| REQ-TIER | TIER-6 INSERT tier-3 types | `tier.rs > ..chat_message_/_episodic_trajectory_inserts_at_tier_3` | ✅ COMPLIANT |
| REQ-TIER | TIER-7 no rule → current | `tier.rs > compute_tier_no_rule_fires_returns_current_unchanged` | ✅ COMPLIANT |
| REQ-DEMOTE | DEMOTE-1 unwired in Phase 5 | grep + `cargo check` "never used" (structural — negative behavior) | ✅ COMPLIANT |
| REQ-DEMOTE | DEMOTE-2 pivotal never demotes | `tier.rs > demote_inactive_tier1_high_importance_never_demotes` | ✅ COMPLIANT |
| REQ-DEMOTE | DEMOTE-3 tier2 →3 @ 180d | `tier.rs > demote_inactive_tier2_over_180_days_demotes_to_3` | ✅ COMPLIANT |
| REQ-DEMOTE | DEMOTE-4 tier1 →2 @ 90d low-imp | `tier.rs > demote_inactive_tier1_low_importance_over_90_days_demotes_to_2` | ✅ COMPLIANT |
| REQ-DEMOTE | DEMOTE-5 no condition → current | `tier.rs > demote_inactive_tier2_not_inactive_enough_returns_current` | ✅ COMPLIANT |
| REQ-LIMIT | LIMIT-1 open() path | `db.rs > limit_variable_number_set_in_open_path` | ✅ COMPLIANT |
| REQ-LIMIT | LIMIT-2 new_for_test() path | `db.rs > limit_variable_number_set_in_new_for_test_path` | ✅ COMPLIANT |
| REQ-LIMIT | LIMIT-3 500-entry flush (1500 vars) | `access.rs > flush_500_distinct_ids_respects_variable_limit` | ✅ COMPLIANT |
| REQ-STATE | STATE-1 new() wires tracker | structural (`state.rs:161-165`); identical wiring exercised behaviorally via async ctor | ✅ COMPLIANT |
| REQ-STATE | STATE-2 test ctors wire tracker | `state.rs > t10_new_for_test_async_access_tracker_accepts_record`, `t10_access_tracker_arc_is_clonable_and_usable` | ✅ COMPLIANT |
| REQ-STATE | STATE-3 prod call site passes Some | `prompt_builder.rs > t11_build_memory_prompt_accepts_none_access_tracker` + `chat.rs:458` structural | ✅ COMPLIANT |
| REQ-STATE | STATE-4 baseline holds | full suite 423/423 (390 baseline + 33 new) | ✅ COMPLIANT |

**Compliance summary**: 26/26 scenarios compliant (FLUSH-3 marked PARTIAL on the log-level sub-requirement — behavior compliant, severity level deviates).

---

## Correctness (Static — Structural Evidence)

| Requirement | Status | Notes |
|------------|--------|-------|
| REQ-TRACK | ✅ Implemented | `UnboundedSender<Vec<i64>>`, infallible non-awaiting `record`, empty-vec discard, `Send+Sync` const guard |
| REQ-FLUSH | ✅ Implemented | `tokio::select!` dual trigger (30s `interval_at` + ≥500), single atomic CTE UPDATE, error→log+continue |
| REQ-TIER | ✅ Implemented | pure `compute_tier` exactly matches spec table; `register_tier_udf` arity-4 `SQLITE_DETERMINISTIC` |
| REQ-DEMOTE | ✅ Implemented | `pub(crate)`, correct truth table, doc states "delivers but does NOT invoke", unwired (grep + compiler-confirmed) |
| REQ-LIMIT | ✅ Implemented | `sqlite3_limit(...,32766)` on the flushing connection in both open paths |
| REQ-STATE | ✅ Implemented | `access_tracker: Arc<AccessTracker>` field in all 3 ctors; `build_memory_prompt` gains `Option<&AccessTracker>`; prod passes `Some` |

---

## Coherence (Design)

| Decision | Followed? | Notes |
|----------|-----------|-------|
| AD-1 `UnboundedSender<Vec<i64>>`, sender-only | ✅ Yes | `access.rs:27-29` |
| AD-2 `tokio::select!` dual-trigger worker | ✅ Yes | `access.rs:62-83` (uses `interval_at(now+30s,30s)` to avoid t=0 tick racing the size trigger — a sound refinement) |
| AD-3 `compute_tier` UDF delegating to pure fn | ✅ Yes | `tier.rs:97-122` |
| AD-4 `register_tier_udf` in `tier.rs` | ✅ Yes | own module boundary |
| AD-5 `AccessTracker::new(conn: Connection)` bare clone | ✅ Yes | `state.rs` passes `memory_store.conn.clone()` |
| AD-6 fire at `prompt_builder` after hits, `search.rs` untouched | ✅ Yes | fires at `prompt_builder.rs:106-107` (design said `:95` — line drift only) |
| `compute_tier` non-anchor table (design `:59-66`) | ⚠️ Deviated (CORRECTLY) | Design's speculative `3→2 @ access_count>=5` + `semantic_fact` 2→1 rules were NOT implemented; the impl follows the authoritative spec.md. Design Open Question (`:132`) remains unchecked — stale doc, not a code defect. |

---

## Issues Found

### CRITICAL (must fix before archive)
None.

### WARNING (should fix)
1. **Log level mismatch — `log::warn!` vs spec `log::error!`.** `access.rs:182` logs flush failures with `log::warn!`; REQ-FLUSH (`spec.md:46`) and FLUSH-3 (`spec.md:64`) mandate `log::error!`. The FLUSH-3 test does not assert the level, so it slips through. Root cause: `tasks.md` T-08-impl contradicted the spec. Fix is a one-token change OR a spec amendment; behavior (no-panic, worker continues) is correct either way.
2. **Design doc drift — stale `compute_tier` non-anchor table + open Open Question.** `design.md:59-66` lists speculative rules the implementation correctly did NOT build, and `design.md:132` still shows the reconciliation Open Question as unchecked. The implementation is right (it follows spec); the DESIGN should be reconciled to spec during archive so the artifact trail isn't misleading.

### SUGGESTION (nice to have)
1. **`demote_inactive` triggers a `never used` compiler warning** in non-test builds (`tier.rs:57`). This is the expected by-product of the "delivered-but-not-wired" contract and actually corroborates DEMOTE-1, but T-12's "no warnings" claim is therefore slightly inaccurate. Consider `#[allow(dead_code)]` with a "wired in Phase 8+" comment to make the intent explicit and keep `cargo check` quiet.
2. **Stale line-number references in `design.md`/`tasks.md`** (record fires at `:106-107` not `:95`; prod call site is `chat.rs:458` not `:451`). Cosmetic; update for accuracy.

---

## Verdict

**PASS WITH WARNINGS** — All 23 tasks complete; 423/423 tests green; `cargo check` exit 0. All 6 requirements and 26 scenarios behaviorally compliant. The single most important risk (the `block_on` + `tokio::spawn` worker survival) was adversarially verified against tauri 2.10.3 source and is SAFE — the worker runs on a process-lifetime multi-threaded runtime and outlives `block_on`. Two non-blocking WARNINGs (log-level severity mismatch; design-doc drift) and two SUGGESTIONs remain. None block archive.

**Next recommended**: `sdd-archive` (reconcile the two WARNINGs into the archive: fix `log::warn!`→`log::error!` or amend spec, and sync `design.md` to the as-built `compute_tier` table).
