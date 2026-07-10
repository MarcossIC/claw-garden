# Design: rag-access-tracker (Phase 5)

## Technical Approach

A standalone `AccessTracker` (new `access.rs`) buffers retrieved memory ids off the
hot path and flushes them in one atomic CTE `UPDATE` that bumps `last_accessed_at`,
`access_count`, and recomputes `tier` via a new `compute_tier` UDF (`tier.rs`).
The worker mirrors Phase 3's `worker.rs` spawn shape but uses `tokio::select!`
because it has TWO flush triggers (30s timer OR ≥500 buffered entries) instead of
one. `prompt_builder.rs` fires `record(ids)` fire-and-forget after hits are
assembled (line 95). All locked decisions in `proposal.md` (a–d) are honored.

## Architecture Decisions

| # | Choice | Alternative rejected | Rationale |
|---|--------|----------------------|-----------|
| 1 | `UnboundedSender<Vec<i64>>` channel; tracker exposes only the `Sender` | bounded channel | Mirrors `memory_tx` (state.rs:78). Hot path MUST NOT block; sender-only keeps the struct `Send + Sync` with no interior `Connection` in the public type. `record` can never await/error. |
| 2 | `tokio::select!` loop on `interval(30s).tick()` vs `rx.recv()` | `worker.rs`'s plain `while let Some` | Two triggers; `worker.rs` has one (explore.md:34). |
| 3 | `compute_tier` as UDF delegating to pure Rust fn (locked-d) | fetch→compute→update | One atomic statement, one `tx.transaction()`; 4 UDF precedents in `score.rs`; Rust fn unit-testable without SQL. |
| 4 | `register_tier_udf` separate fn in `tier.rs` (locked-a) | fold into `register_score_udfs` | `tier.rs` owns its UDF; clean module boundary. |
| 5 | `AccessTracker::new(conn: Connection)` holds a bare `Connection` clone | `Arc<MemoryStore>` / added `Mutex` | `tokio_rusqlite::Connection` is `Clone` (db.rs:38-44) — a channel handle, no `Mutex` (explore.md:141). |
| 6 | Fire at `prompt_builder.rs:95`, `search.rs` untouched | wire into `search_all_memories` | Avoids coupling a retrieval primitive to an access side-effect; hits already owned at the call site. |

## Data Flow

```
build_memory_prompt (hits assembled @:95)
      │ record(Vec<i64>)  ── non-blocking send ──┐
      ▼                                          ▼
  [hot path returns]                    UnboundedSender<Vec<i64>>
                                                 │
                                   AccessTracker worker (select!)
                                   ┌─────────────┴─────────────┐
                          interval(30s).tick()         rx.recv() → buffer
                                   └──────► flush() ◄── buffer.len() ≥ 500
                                                 │
                          single conn.call → tx.transaction() → CTE UPDATE
                          (compute_tier UDF recomputes tier in-statement)
```

## Interfaces / Contracts

```rust
// access.rs
pub(crate) struct AccessTracker { tx: UnboundedSender<Vec<i64>> }
impl AccessTracker {
    pub(crate) fn new(conn: Connection) -> Self;     // spawns worker, returns Sender holder
    pub(crate) fn record(&self, ids: Vec<i64>);      // owned Vec (locked-b); send().ok() — never panics
}
const _: fn() = || { fn _ss<T: Send + Sync>() {} _ss::<AccessTracker>(); };  // compile-time guard

// tier.rs
pub(crate) fn compute_tier(current: i64, importance: f64, access_count: i64, memory_type: &str) -> i64;
pub(crate) async fn register_tier_udf(conn: &Connection) -> Result<(), MemoryError>;
/// NOT called in Phase 5; autoDream (doc 9) owns runtime invocation. Promotion-only counterpart.
pub(crate) fn demote_inactive(current_tier: i64, days_since_last_access: f64, importance: f64) -> i64;
```

`compute_tier` — promotion-only (result is never numerically larger than `current`):

| current | condition | → tier |
|---------|-----------|--------|
| any | `importance >= 9.0` | 1 (pivotal fast-track) |
| 2 | `access_count >= 3` AND type ∈ {architectural_decision, session_summary} | 1 |
| _ | otherwise | `current` (no-op) |

> **Reconciled post-implementation (archive)**: doc 6:720-732 specifies NO tier 3→2 promotion rule; implementation is promote-only with only the documented initial tiers + the 2→1 anchor. No 3→2 rule exists by design. `user_preference` and `semantic_fact` are NOT in the 2→1 anchor set — only `architectural_decision` and `session_summary` promote 2→1 at `access_count >= 3`. The speculative rows (`3→2 @ access_count>=5`, `semantic_fact` in the 2→1 set) that appeared in earlier drafts were NEVER implemented and are removed from this table.

`register_tier_udf` wraps it: arity 4, `SQLITE_UTF8 | SQLITE_DETERMINISTIC`,
`ctx.get(0..3)`, delegates to the pure fn (score.rs:16 pattern).

### Flush SQL (single `.call()` + `tx.transaction()` — preserves 1:1 memory↔memory_vectors)

```sql
WITH updates(id, last_ts, hits) AS (VALUES (?,?,?),(?,?,?), ...)
UPDATE memory
SET last_accessed_at = MAX(memory.last_accessed_at, updates.last_ts),
    access_count     = memory.access_count + updates.hits,
    tier = compute_tier(memory.tier, memory.importance,
                        memory.access_count + updates.hits, memory.memory_type)
FROM updates WHERE memory.id = updates.id;
```

Coalesce duplicate ids in the buffer (`HashMap<i64, hits>`) before building VALUES.
`memory_vectors` is untouched (tier/access live only on `memory`) so the invariant
holds trivially. Flush errors → `log::warn!` + continue, never panic (locked-c, worker.rs:55).

**Placeholder math**: ≤500 rows × 3 vars/row = **1500** > SQLite default **999** →
runtime failure past row 333. `set_limit(SQLITE_LIMIT_VARIABLE_NUMBER, 32766)` (the
SQLite hard ceiling) gives headroom. Must land in BOTH `open()` and `new_for_test()`.

## File Changes

| File | Action | Description |
|------|--------|-------------|
| `memory/access.rs` | Create | `AccessTracker` + `select!` worker + `start_access_worker(app_handle)` + Send+Sync guard |
| `memory/tier.rs` | Create | `compute_tier`, `register_tier_udf`, `demote_inactive` |
| `memory/mod.rs` | Modify | `mod access; mod tier;` declarations |
| `memory/db.rs` | Modify | `set_limit(…,32766)` + `register_tier_udf(&conn).await?` in `open()` (after :60) AND `new_for_test()` (after :76) — 2 sites |
| `memory/prompt_builder.rs` | Modify | `build_memory_prompt` gains `access_tracker: Option<&AccessTracker>`; fire-and-forget after :95 |
| `state.rs` | Modify | `access_tracker: Arc<AccessTracker>` field + init in `new()`, `new_for_test()`, `new_for_test_async()` (3 ctors) via `AccessTracker::new(memory_store.conn.clone())` |
| `commands/chat.rs` | Modify | `compute_memory_prefix` passes `Some(state.access_tracker.as_ref())` at :451 |
| `lib.rs` | Modify | `start_access_worker(app.handle().clone())` after :50 |

**Call sites touched for the new param**: 1 production (chat.rs:451) + 8 test calls
in `prompt_builder.rs` (all pass `None`) = 9 edits.

## Testing Strategy (Strict TDD — every `*-impl` preceded by `*-test`; never `cargo build`)

| Layer | What | Approach |
|-------|------|----------|
| Unit | `compute_tier` truth table (each row + pivotal + no-op) | pure Rust, no SQL |
| Unit | `demote_inactive` (90d/180d boundaries, pivotal-never) | pure Rust |
| Unit | `register_tier_udf` callable | `SELECT compute_tier(2,5.0,3,'architectural_decision')` → 1 (score.rs test shape) |
| Integration | 30s timer flush | `#[tokio::test(start_paused = true)]` + `tokio::time::advance(31s)` — NO real sleep |
| Integration | ≥500-entry trigger | send 500 ids, assert flush WITHOUT advancing the clock |
| Integration | tier promotion in flush | seed tier-2 architectural_decision, record 3×, flush, assert tier=1 |
| Integration | 1000 concurrent retrievals no-stall | spawn 1000 `record` calls, assert tracker drains + hot path returns |
| Compile | `Send + Sync` | `const _: fn()` guard in `access.rs` |

`set_limit` correctness is exercised by a ≥334-row flush integration test (would fail
at default 999). No `ZeroBackoff`/injected clock needed — paused-time covers the timer.

## Migration / Rollout

No migration. New files deletable; `db.rs`/`state.rs`/`prompt_builder.rs`/`lib.rs`/
`chat.rs` diffs revertible. No schema change, no Cargo.toml change. Baseline 390/390 restored on rollback.

## Open Questions

- [x] ~~Non-anchor `compute_tier` thresholds (tier 3→2 count, eligible types for 2→1) must be reconciled against doc 6:715-730 before `compute_tier-impl`. Anchor rule is locked.~~ **RESOLVED (archive 2026-06-27)**: doc 6:720-732 specifies NO tier 3→2 promotion rule; implementation is promote-only with only the documented initial tiers + the 2→1 anchor. No 3→2 rule exists by design.
