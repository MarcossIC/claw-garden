# Tasks: rag-access-tracker (Phase 5)

> **Strict TDD**: every `*-impl` is immediately preceded by its `*-test`. No impl without a failing test first.
> **Constraints**: `log::` only — never `tracing::`. Never `cargo build` (use `cargo check`/`cargo test --lib`). Never real sleep in tests.

---

## Phase 1: tier.rs — Pure-Rust Functions (No Dependencies)

- [x] T-01-test Write failing unit tests for `compute_tier`: all 6 initial-tier truth-table rows (TIER-4/5/6), the promote-only invariant — result never numerically > `current` (TIER-2), and the no-rule-fires no-op (TIER-7). REQ: TIER-2,4,5,6,7. File: `memory/tier.rs`.
- [x] T-01-impl Implement `pub(crate) fn compute_tier(current: i64, importance: f64, access_count: i64, memory_type: &str) -> i64` in new `memory/tier.rs` — promotion-only table (`importance >= 9.0` → 1; `access_count >= 3` + architectural_decision/session_summary → 1; else `current`). Doc: "promotes only; use `demote_inactive` for decay." REQ: REQ-TIER, TIER-1..7. File: `memory/tier.rs`.

- [x] T-02-test Write failing unit tests for `demote_inactive`: tier-1→2 when `days > 90 AND importance < 9.0` (DEMOTE-4); `importance >= 9.0` never-demote no-op returns 1 (DEMOTE-2); tier-2→3 when `days > 180` (DEMOTE-3); no-condition-met returns `current` unchanged (DEMOTE-5). REQ: DEMOTE-2,3,4,5. File: `memory/tier.rs`.
- [x] T-02-impl Implement `pub(crate) fn demote_inactive(current_tier: i64, days_since_last_access: f64, importance: f64) -> i64` in `memory/tier.rs`. Doc comment MUST read: *"Only called by autoDream (Phase 8+). Phase 5 delivers but does NOT invoke."* REQ: REQ-DEMOTE, DEMOTE-1..5. File: `memory/tier.rs`.

---

## Phase 2: UDF Registration + SQLite Variable Limit

- [x] T-03-test Write failing test: call `register_tier_udf(&conn).await`, then assert `SELECT compute_tier(2, 5.0, 3, 'architectural_decision')` returns `1` (mirrors `score.rs` test shape using `new_for_test()` connection). REQ: REQ-TIER, TIER-3. File: `memory/tier.rs`.
- [x] T-03-impl Implement `pub(crate) async fn register_tier_udf(conn: &Connection) -> Result<(), MemoryError>` in `memory/tier.rs` — arity 4, `SQLITE_UTF8 | SQLITE_DETERMINISTIC`, `ctx.get(0..3)`, delegates to pure `compute_tier`. Add `mod tier;` to `memory/mod.rs`. REQ: REQ-TIER. Files: `memory/tier.rs`, `memory/mod.rs`.

- [x] T-04-test Write two separate failing tests: (a) after `MemoryStore::open()` assert `SQLITE_LIMIT_VARIABLE_NUMBER == 32766` (LIMIT-1); (b) after `MemoryStore::new_for_test()` assert the same value (LIMIT-2). REQ: REQ-LIMIT, LIMIT-1, LIMIT-2. File: `memory/db.rs`.
- [x] T-04-impl In `memory/db.rs`, add `conn.call(|c| { c.set_limit(SQLITE_LIMIT_VARIABLE_NUMBER, 32766); Ok(()) }).await?` + `register_tier_udf(&conn).await?` in both `open()` (after `:60` `register_score_udfs`) AND `new_for_test()` (after `:76`) — two sites, explicit. REQ: REQ-LIMIT, LIMIT-1, LIMIT-2. File: `memory/db.rs`.

---

## Phase 3: AccessTracker Struct + record

- [x] T-05-test Write failing tests: compile-time `Send + Sync` const guard (`const _: fn() = || { fn _ss<T: Send + Sync>() {} _ss::<AccessTracker>(); };`); TRACK-2 empty-vec no-op (assert channel receives nothing after `record(vec![])`). REQ: REQ-TRACK, TRACK-2. File: `memory/access.rs`.
- [x] T-05-impl Create `memory/access.rs`: `pub(crate) struct AccessTracker { tx: UnboundedSender<Vec<i64>> }`, `new(conn: Connection) -> Self` stub (no worker yet — spawns placeholder task), `record(&self, ids: Vec<i64>)` with empty-vec early-return + `self.tx.send(ids).ok()`, and the compile-time `Send + Sync` const guard. Add `mod access;` to `memory/mod.rs`. REQ: REQ-TRACK, TRACK-1, TRACK-2. Files: `memory/access.rs`, `memory/mod.rs`.

---

## Phase 4: Flush Worker (tokio::select!) + CTE SQL

- [x] T-06-test Write failing `#[tokio::test(start_paused = true)]` test: buffer <500 entries, call `tokio::time::advance(Duration::from_secs(31))`, assert flush executed and buffer cleared — no `std::thread::sleep` or real wall-clock delay. REQ: REQ-FLUSH, FLUSH-1. File: `memory/access.rs`.
- [x] T-06-impl Wire the `tokio::select!` worker into `AccessTracker::new`: spawn a task with `interval(Duration::from_secs(30)).tick()` vs `rx.recv()` → accumulate to buffer `Vec<i64>`, call `flush()` on timer tick. REQ: REQ-FLUSH, FLUSH-1. Files: `memory/access.rs`.

- [x] T-07-test Write failing test (no `time::advance`): send exactly 500 IDs via successive `record` calls, assert flush fires immediately — verified by checking DB row or an atomic flush counter — without advancing the clock. REQ: REQ-FLUSH, FLUSH-2. File: `memory/access.rs`.
- [x] T-07-impl In the `rx.recv()` arm: after accumulating, check `buffer.len() >= 500`; if so call `flush()` immediately before the next `select!` iteration. REQ: REQ-FLUSH, FLUSH-2. File: `memory/access.rs`.

- [x] T-08-test Write failing integration tests: (a) inject a failing DB connection, assert `log::error!` is called and worker continues without panicking (FLUSH-3); (b) seed `memory` row with `tier=2, access_count=2, memory_type='architectural_decision'`, record 1 more access, flush, assert single CTE UPDATE sets `access_count=3` AND `tier=1` atomically (FLUSH-4, TIER-1). REQ: FLUSH-3, FLUSH-4, TIER-1. File: `memory/access.rs`.
- [x] T-08-impl Implement `async fn flush(conn: &Connection, buffer: Vec<i64>)`: coalesce duplicate ids into `HashMap<i64, u32>` (hits per id), build CTE UPDATE SQL with `compute_tier(memory.tier, memory.importance, memory.access_count + updates.hits, memory.memory_type)` in SET clause, execute inside `conn.call(|c| tx.transaction(...))`, on error `log::warn!` + continue — never panic. REQ: REQ-FLUSH, FLUSH-3, FLUSH-4. File: `memory/access.rs`.

---

## Phase 5: Concurrency + Limit Stress

- [x] T-09-test Write failing tests: (a) TRACK-3 — spawn 1000 concurrent `tokio::task` calls each invoking `record(vec![id])`, join all handles, assert tracker drains without stalling and hot path returns; (b) LIMIT-3 — flush exactly 500 distinct rows (1500 bind variables) via the real `MemoryStore::new_for_test()` connection, assert execution completes with no variable-number SQLite error. REQ: REQ-TRACK (TRACK-3), REQ-LIMIT (LIMIT-3). File: `memory/access.rs`.
- [x] T-09-impl Confirm `UnboundedSender::clone()` enables concurrent callers; verify `new_for_test()` path (T-04) already provides the `set_limit` needed for the 500-row flush. No new production code expected — this task is green only if T-04 + T-07 are correct. REQ: REQ-TRACK, REQ-LIMIT. Files: `memory/access.rs`, `memory/db.rs`.

---

## Phase 6: AppState + lib.rs Wiring

- [x] T-10-test Write failing tests for STATE-1/STATE-2: assert `AppState::new()`, `AppState::new_for_test()`, and `AppState::new_for_test_async()` each produce a non-panicking `Arc<AccessTracker>` that accepts a `record` call without blocking. REQ: REQ-STATE, STATE-1, STATE-2. File: `state.rs`.
- [x] T-10-impl Add `access_tracker: Arc<AccessTracker>` field to `AppState`; initialize with `Arc::new(AccessTracker::new(memory_store.conn.clone()))` in all 3 constructors (`new`, `new_for_test`, `new_for_test_async`); in `lib.rs::run()` add `start_access_worker(app.handle().clone())` after `start_memory_worker`. REQ: REQ-STATE, STATE-1, STATE-2. Files: `state.rs`, `lib.rs`.

---

## Phase 7: Prompt Builder + Call-Site Ripple (9 Edits)

- [x] T-11-test Write failing test confirming `build_memory_prompt` compiles with the new `access_tracker: Option<&AccessTracker>` parameter; assert production call path at `chat.rs:451` passes `Some(&state.access_tracker)` and test paths pass `None` without compile errors. REQ: REQ-STATE, STATE-3. Files: `memory/prompt_builder.rs`, `commands/chat.rs`.
- [x] T-11-impl Add `access_tracker: Option<&AccessTracker>` to `build_memory_prompt`; after line `:95` add `if let Some(t) = access_tracker { t.record(ids.clone()); }`; update `commands/chat.rs:451` to pass `Some(state.access_tracker.as_ref())`; update all 8 test call sites in `prompt_builder.rs` tests to pass `None` — 9 total edits, single mechanical task. REQ: REQ-STATE, STATE-3, STATE-4. Files: `memory/prompt_builder.rs`, `commands/chat.rs`.

---

## Phase 8: Final Verification

- [x] T-12-verify Run `cargo test --lib`: confirm all 390 baseline tests PASS with zero regressions AND all net-new tracker tests PASS. Run `cargo check` across all touched modules with no warnings. REQ: REQ-STATE (STATE-4), all. Files: all touched modules.

---

**Total: 23 tasks** (11 test/impl pairs + 1 final verification)
