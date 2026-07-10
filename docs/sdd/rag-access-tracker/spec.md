# rag-access-tracker Specification

## Purpose

Batched, fire-and-forget access tracking that updates `last_accessed_at`, `access_count`, and `tier` off the hot retrieval path. Zero retrieval latency added; tier scoring stays accurate over time.

## Requirements Summary

| ID | Name | Strength |
|----|------|----------|
| REQ-TRACK | AccessTracker record buffer | MUST |
| REQ-FLUSH | Batched flush trigger + atomic UPDATE | MUST |
| REQ-TIER | compute_tier promotion UDF | MUST |
| REQ-DEMOTE | demote_inactive contract (not wired Phase 5) | MUST NOT invoke |
| REQ-LIMIT | SQLite variable number limit | MUST |
| REQ-STATE | AppState wiring (all 3 constructors) | MUST |

---

## REQ-TRACK — AccessTracker Record Buffer

`AccessTracker::record(ids: Vec<i64>)` MUST send IDs to an internal `UnboundedSender<Vec<i64>>` without blocking. The call MUST NOT await, return a `Result`, or panic. `AccessTracker` MUST implement `Send + Sync` (compile-time assertion required). Empty `Vec` MUST be discarded without sending.

#### Scenario: TRACK-1 — fire-and-forget on hot path

- GIVEN an `AccessTracker` with a live async worker
- WHEN `record(vec![1, 2, 3])` is called from the retrieval hot path
- THEN the call returns immediately without awaiting or blocking

#### Scenario: TRACK-2 — empty vec is a no-op

- GIVEN an `AccessTracker` with a live async worker
- WHEN `record(vec![])` is called
- THEN nothing is sent to the internal channel

#### Scenario: TRACK-3 — 1000 concurrent callers

- GIVEN 1000 concurrent calls to `build_memory_prompt` each invoking `record`
- WHEN all calls complete
- THEN the tracker does not stall and hot-path latency does not degrade

---

## REQ-FLUSH — Batched Flush Trigger + Atomic UPDATE

The access worker MUST flush when EITHER: (a) 30 seconds elapsed since last flush, OR (b) buffer contains ≥500 entries. Flush MUST execute one CTE `UPDATE` setting `last_accessed_at = MAX(stored, new)`, `access_count = stored + delta`, and `tier = compute_tier(...)` atomically. Flush errors MUST be logged via `log::warn!` and the worker MUST continue — never panic.

#### Scenario: FLUSH-1 — timer trigger

- GIVEN a worker with `start_paused = true` and <500 buffered entries
- WHEN `tokio::time::advance(Duration::from_secs(31))` is called
- THEN a flush executes and the buffer is cleared

#### Scenario: FLUSH-2 — size trigger at 500 entries

- GIVEN a worker with exactly 499 buffered IDs
- WHEN one more ID is recorded (total = 500)
- THEN a flush executes immediately without waiting for the 30-second timer

#### Scenario: FLUSH-3 — DB error recovery

- GIVEN a flush that encounters a transient DB error
- WHEN the error occurs
- THEN `log::warn!` is called and the worker continues without panicking

#### Scenario: FLUSH-4 — tier updated in same statement

- GIVEN a memory row with access_count=2, tier=2, memory_type='architectural_decision'
- WHEN a flush batch adds 1 access for that row
- THEN the single CTE UPDATE sets access_count=3 AND tier=1 in one statement

---

## REQ-TIER — compute_tier Promotion UDF

`compute_tier(current: u8, importance: f64, access_count: u32, memory_type: &str) -> u8` MUST be a pure-Rust function in `tier.rs`, separately unit-testable without SQL. It MUST be registered as a SQLite scalar UDF (`SQLITE_DETERMINISTIC`, 4 args → INTEGER) via `register_tier_udf`. It MUST be invoked at INSERT (`compute_tier(3, importance, 0, type)`) AND at every flush (`compute_tier(current, importance, access_count + hits, type)`).

**Promotion invariant**: `compute_tier` MUST NEVER return a tier numerically GREATER than `current` (promotion only moves toward tier 1). If no rule fires, it MUST return `current` unchanged.

**INSERT initial-tier truth table** (called with `current = 3`, `access_count = 0`):

| memory_type | Initial tier |
|-------------|--------------|
| `user_preference` | 1 |
| `procedural_skill` | 1 (ALL rows) |
| `architectural_decision` | 2 |
| `session_summary` | 2 |
| `chat_message` | 3 |
| `episodic_trajectory` | 3 |

**Flush re-eval rule**: crossing `access_count >= 3` MUST promote `architectural_decision`/`session_summary` from tier 2 → 1.

#### Scenario: TIER-1 — architectural_decision promotes tier 2 → 1 at flush

- GIVEN a memory with tier=2, access_count=2, memory_type='architectural_decision', importance=7.0
- WHEN a flush records 1 additional access (new access_count=3)
- THEN `compute_tier` returns 1 and the CTE UPDATE sets tier=1

#### Scenario: TIER-2 — promotion invariant (never demotes)

- GIVEN a memory with tier=1, access_count=0, memory_type='chat_message', importance=1.0
- WHEN `compute_tier(1, 1.0, 0, 'chat_message')` is called
- THEN the returned value is 1 (result MUST NOT exceed current tier)

#### Scenario: TIER-3 — pure Rust fn is unit-testable

- GIVEN the pure-Rust `compute_tier` function imported directly (no SQL)
- WHEN called with valid arguments
- THEN it returns the expected promotion result without any DB connection

#### Scenario: TIER-4 — INSERT tier 1 types

- GIVEN a new memory of type `user_preference` (and separately `procedural_skill`)
- WHEN `compute_tier(3, importance, 0, type)` is called at INSERT
- THEN it returns 1 for both types

#### Scenario: TIER-5 — INSERT tier 2 types

- GIVEN a new memory of type `architectural_decision` (and separately `session_summary`)
- WHEN `compute_tier(3, importance, 0, type)` is called at INSERT
- THEN it returns 2 for both types

#### Scenario: TIER-6 — INSERT tier 3 types

- GIVEN a new memory of type `chat_message` (and separately `episodic_trajectory`)
- WHEN `compute_tier(3, importance, 0, type)` is called at INSERT
- THEN it returns 3 for both types

#### Scenario: TIER-7 — no rule fires returns current unchanged

- GIVEN a memory with tier=2, access_count=1, memory_type='chat_message', importance=2.0
- WHEN `compute_tier` is called (below the access_count >= 3 threshold)
- THEN it returns 2 (current tier unchanged, no promotion)

---

## REQ-DEMOTE — demote_inactive Contract (Phase 5: NOT wired)

`demote_inactive(current_tier: u8, days_since_last_access: f64, importance: f64) -> u8` MUST be `pub(crate)` in `tier.rs`. It is the ONLY demoting function. Phase 5 MUST NOT invoke it at runtime. Its doc comment MUST state: *"Only called by autoDream (Phase 8+). Phase 5 delivers but does NOT invoke."*

**Demotion truth table** (demotes only — never promotes):

| current_tier | Condition | Result |
|--------------|-----------|--------|
| 1 | `days_since_last_access > 90` AND `importance < 9.0` | 2 |
| 1 | `importance >= 9.0` (any days) | 1 (NEVER demoted) |
| 2 | `days_since_last_access > 180` | 3 |
| any | no condition met | `current_tier` unchanged |

#### Scenario: DEMOTE-1 — function is unwired in Phase 5

- GIVEN Phase 5 is running and a flush executes
- WHEN the access worker processes 500 buffered entries
- THEN `demote_inactive` is NOT called during the flush

#### Scenario: DEMOTE-2 — absolutely pivotal memories never demote

- GIVEN `demote_inactive(1, 91.0, 9.0)` is called
- WHEN the function executes
- THEN it returns 1 (importance >= 9.0 protects tier 1 from demotion)

#### Scenario: DEMOTE-3 — tier 2 demotes after 180 days

- GIVEN `demote_inactive(2, 181.0, 5.0)` is called
- WHEN the function executes
- THEN it returns 3

#### Scenario: DEMOTE-4 — tier 1 demotes to 2 when inactive and low importance

- GIVEN `demote_inactive(1, 91.0, 5.0)` is called
- WHEN the function executes
- THEN it returns 2 (days > 90 AND importance < 9.0)

#### Scenario: DEMOTE-5 — no condition met returns current unchanged

- GIVEN `demote_inactive(2, 100.0, 5.0)` is called (days <= 180 for tier 2)
- WHEN the function executes
- THEN it returns 2 (no demotion threshold crossed)

---

## REQ-LIMIT — SQLite Variable Number Limit

`MemoryStore::open()` AND `MemoryStore::new_for_test()` MUST call `set_limit(SQLITE_LIMIT_VARIABLE_NUMBER, 32766)` after `register_score_udfs`. Flush batches of up to 500 entries (1500 bind variables) MUST NOT fail due to the default 999-variable SQLite limit.

#### Scenario: LIMIT-1 — set_limit in production path

- GIVEN `MemoryStore::open()` completes
- WHEN a query is executed on that connection
- THEN `SQLITE_LIMIT_VARIABLE_NUMBER` is 32766

#### Scenario: LIMIT-2 — set_limit in test path

- GIVEN `MemoryStore::new_for_test()` completes
- WHEN a query is executed on that connection
- THEN `SQLITE_LIMIT_VARIABLE_NUMBER` is 32766

#### Scenario: LIMIT-3 — 500-entry flush executes without error

- GIVEN `set_limit(SQLITE_LIMIT_VARIABLE_NUMBER, 32766)` is applied
- WHEN a CTE UPDATE flush with exactly 500 rows (1500 bind variables) executes
- THEN the statement completes without a variable-number error

---

## REQ-STATE — AppState Wiring

`AppState` MUST contain `access_tracker: Arc<AccessTracker>`. It MUST be initialized in `new()`, `new_for_test()`, and `new_for_test_async()` via `AccessTracker::new(memory_store.conn.clone())`. The access worker MUST be spawned in `lib.rs::run()` after `start_memory_worker`. `build_memory_prompt` MUST accept `access_tracker: Option<&AccessTracker>`; production call site MUST pass `Some(&state.access_tracker)`; test call sites MAY pass `None`.

#### Scenario: STATE-1 — AppState::new wires tracker

- GIVEN `AppState::new()` is called
- WHEN initialization completes
- THEN `state.access_tracker` is an `Arc<AccessTracker>` ready to accept `record` calls

#### Scenario: STATE-2 — test constructors wire tracker

- GIVEN `AppState::new_for_test()` or `new_for_test_async()` is called
- WHEN initialization completes
- THEN `state.access_tracker` is an `Arc<AccessTracker>` backed by the test connection

#### Scenario: STATE-3 — production call site passes tracker

- GIVEN `build_memory_prompt` is invoked from `commands/chat.rs`
- WHEN hits are assembled at line 95
- THEN `Some(&state.access_tracker)` is passed and IDs are recorded fire-and-forget

#### Scenario: STATE-4 — baseline holds after wiring

- GIVEN all AppState constructors are updated
- WHEN the full test suite runs
- THEN all 390 existing tests PASS with no regressions
