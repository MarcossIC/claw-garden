# Exploration: rag-access-tracker

## Current State

Phase 4 delivered `search.rs` (`search_all_memories`), `score.rs` (4 UDFs), `prompt_builder.rs` (`build_memory_prompt`), and wired `memory_store: Arc<MemoryStore>` + `memory_tx: UnboundedSender<MemoryEvent>` + `embedder: Arc<OnceCell<...>>` into `state.rs`. The hot-path today does KNN + score but does NOT update `last_accessed_at`, `access_count`, or `tier` on any retrieved row. Phase 5 must add that without touching retrieval latency.

Test baseline: 390/390 Rust PASS (all in `src-tauri/src/`).

---

## Affected Areas

- `src-tauri/src/memory/db.rs` — `MemoryStore::open()` and `new_for_test()` must set `SQLITE_LIMIT_VARIABLE_NUMBER`; `register_score_udfs` called there is the anchor point for any new UDF registration
- `src-tauri/src/memory/score.rs` — pattern source for UDF registration (`create_scalar_function`); `compute_tier` UDF follows this exactly
- `src-tauri/src/memory/search.rs` — `search_all_memories` ends at line 328 (`Ok(all)`); `all: Vec<RankedHit>` carries `.id: i64` for every hit
- `src-tauri/src/memory/prompt_builder.rs` — `build_memory_prompt` calls `search_all_memories` at line 78; `hits: Vec<RankedHit>` is fully assembled at line 95 (end of match block); this is the fire-and-forget integration point
- `src-tauri/src/state.rs` — `AppState` struct (line 36); `new()` (line 103), `new_for_test()` (line 182), `new_for_test_async()` (line 252); `access_tracker: Arc<AccessTracker>` must mirror the `memory_store` field pattern
- `Cargo.toml` — `rusqlite = { version = "0.37", features = ["bundled", "load_extension", "functions"] }` (line 46); `tokio` dev-dep includes `test-util` (line 77); NO new crate needed

**New files** (to create in `src-tauri/src/memory/`):
- `access.rs` — `AccessTracker` struct + async worker
- `tier.rs` — `compute_tier` + `demote_inactive` pure Rust functions

---

## Open Decisions — Resolved with Evidence

### Decision 1: Flush trigger values

**Confirmed**: `doc 6:697` locks the values as **30 seconds OR ≥ 500 entries**. No ambiguity, no need to re-propose.

Implementation shape: the access worker loop uses `tokio::select!` between `tokio::time::interval(Duration::from_secs(30)).tick()` and `rx.recv()`. A size check after each `rx.recv()` triggers an immediate flush when `buffer.len() >= 500`.

The existing `worker.rs` uses a pure `while let Some(evt) = rx.recv().await` loop (no timer). The AccessTracker worker needs a DIFFERENT shape because it has two flush triggers. The `tokio::select!` pattern is the correct choice:

```
loop {
    select! {
        _ = interval.tick() => flush(),
        Some(ids) = rx.recv() => {
            accumulate(ids);
            if buffer.len() >= 500 { flush() }
        }
        else => break,  // channel closed
    }
}
```

**Test strategy for timer**: `Cargo.toml` dev-dependencies already include `tokio = { ..., features = ["test-util"] }` (line 77). Use `#[tokio::test(start_paused = true)]` + `tokio::time::advance(Duration::from_secs(31))` — no real sleeps, no ZeroBackoff injection needed for the timer trigger. The 500-entry threshold test uses a FakeTracker / direct channel send without the timer at all.

### Decision 2: SQLITE_MAX_VARIABLE_NUMBER

**Problem confirmed**: 500 entries × 3 placeholders (id, last_ts, hits) = 1500 variables. SQLite default `SQLITE_MAX_VARIABLE_NUMBER` is 999 (compile-time default for bundled SQLite). A flush batch of 334+ rows would hit the limit.

**Solution path**: `rusqlite 0.37` exposes `rusqlite::limits::Limit::SQLITE_LIMIT_VARIABLE_NUMBER` (stable since rusqlite 0.25). Method: `Connection::set_limit(limit: Limit, new_val: i32) -> i32` (returns old value, no Result). The `functions` feature already enabled in `Cargo.toml:46` is unrelated but confirms the dependency is fully set up.

**Integration point**: `db.rs::MemoryStore::open()` (line 60-61, right after `register_score_udfs`) and `db.rs::MemoryStore::new_for_test()` (line 76-77, same position). Pattern:

```rust
conn.call(|c| {
    let _ = c.set_limit(rusqlite::limits::Limit::SQLITE_LIMIT_VARIABLE_NUMBER, 32766);
    Ok(())
}).await.map_err(|_| MemoryError::Backend("set_limit failed".into()))?;
```

This is a `db.rs` modification (not a new file). It affects the baseline test suite only by making `new_for_test()` slightly wider — no existing test breaks.

**Note**: the CTE `VALUES` + `UPDATE ... FROM` shape requires SQLite 3.33+. Bundled rusqlite always compiles a recent SQLite version — no risk.

### Decision 3: Tier promotion mechanism — UDF vs fetch+compute+update

**Recommendation: UDF approach** (register `compute_tier` as a 5th scalar function alongside the existing 4 in `score.rs`/`tier.rs`).

**Evidence from `score.rs`**: Phase 4 registered 4 UDFs via `c.create_scalar_function(name, arity, FunctionFlags::SQLITE_UTF8 | SQLITE_DETERMINISTIC, closure)` (score.rs:16-62). All are invoked in the retrieval SQL. `compute_tier` fits the same mold: 4 args → 1 INTEGER result, output is a pure function of inputs, `SQLITE_DETERMINISTIC` is correct.

**Why UDF wins over fetch+compute+update**:

| | UDF in single CTE | Fetch → Rust compute → UPDATE |
|---|---|---|
| Atomicity | One statement, one transaction | Requires 2 `.call()` blocks or complex single-closure |
| Round-trips | Zero extra | 1 SELECT + 1 UPDATE per flush |
| Pattern entropy | Mirrors existing score UDFs | New ad-hoc pattern |
| Testability | SQL unit test (same as score tests) + Rust fn unit test | Rust fn only |
| Latency | Flush is background worker — any cost is off hot path | Same |

The pure Rust function `compute_tier(current: u8, importance: f64, new_access_count: u32, memory_type: &str) -> u8` lives in `tier.rs` and is separately unit-testable without SQL. The UDF wrapper delegates to it. Registration happens in a new `register_tier_udf(conn: &Connection)` function called from `MemoryStore::open()` and `new_for_test()` (same pattern as `register_score_udfs`).

**Flush UPDATE shape** (doc 6:701-715, confirmed feasible):

```sql
WITH updates(id, last_ts, hits) AS (VALUES (?,?,?),(?,?,?),...)
UPDATE memory
SET last_accessed_at = MAX(memory.last_accessed_at, updates.last_ts),
    access_count     = memory.access_count + updates.hits,
    tier             = compute_tier(
                          memory.tier,
                          memory.importance,
                          memory.access_count + updates.hits,
                          memory.memory_type
                       )
FROM updates
WHERE memory.id = updates.id;
```

**Tradeoff disclosed**: if `compute_tier` logic changes, the UDF must be re-registered (happens at every `MemoryStore::open()`). This is the same maintenance cost as the 4 existing UDFs — acceptable.

### Decision 4: `record(top_k_ids)` fire-and-forget wiring

**Exact integration point**: `prompt_builder.rs:95` — after the closing `}` of the `match embedder { ... }` block where `hits: Vec<RankedHit>` is fully assembled, before `let now_secs = ...` at line 98.

The plan says "modify `search.rs`", but the actual hit list is available SOONER at the call site in `prompt_builder.rs`. Wiring tracking into `search_all_memories` would couple a retrieval primitive to an access side-effect — wrong layer. The cleaner cut: `build_memory_prompt` receives an `Option<&AccessTracker>` and fires after collecting hits:

```rust
// prompt_builder.rs, after line 95
if let Some(tracker) = access_tracker {
    let ids: Vec<i64> = hits.iter().map(|h| h.id).collect();
    if !ids.is_empty() {
        tracker.record(ids);  // sends to UnboundedSender — non-blocking
    }
}
```

`search.rs` signature stays unchanged. `search.rs` tests stay unchanged (all 390 PASS preserved).

`build_memory_prompt` gains one parameter: `access_tracker: Option<&AccessTracker>`. The one call site in `commands/chat.rs` (via `compute_memory_prefix`) passes `Some(state.access_tracker.as_ref())`.

### Decision 5: AppState wiring pattern

**Pattern from `state.rs`**:

- `memory_store: Arc<MemoryStore>` — `block_on(MemoryStore::open(...))` in `new()` (line 137-140); `.await` in `new_for_test_async()` (line 271-274)
- `memory_tx: UnboundedSender<MemoryEvent>` — `unbounded_channel()` returns `(tx, rx)`, tx into state, rx returned (line 145, 170)
- `embedder: Arc<OnceCell<...>>` — empty in `new()` (line 168), pre-seeded via `seeded_embedder_cell()` in tests (lines 240, 297)

`access_tracker: Arc<AccessTracker>` follows the `memory_store` pattern:
- `AccessTracker::new(conn: tokio_rusqlite::Connection) -> Self` — sync constructor (no await), holds the `Connection` clone
- In `AppState::new()`: `let access_tracker = Arc::new(AccessTracker::new(memory_store.conn.clone()));` (sync, after memory_store is built)
- In `new_for_test_async()` and `new_for_test()`: same line — `AccessTracker::new(memory_store.conn.clone())`
- Worker spawned in `lib.rs::run()` setup hook via `start_access_worker(&app_handle)` (mirrors `start_memory_worker`)

**The `Arc<MemoryStore>` Clone anomaly** (documented in Phase 1 lessons): `tokio_rusqlite::Connection` is Clone — it's a channel handle to a background thread. `Arc<MemoryStore>` is correct (no Mutex). `AccessTracker` holds a bare `Connection` (from `memory_store.conn.clone()`) — same convention, no Mutex.

**Compile-time Send+Sync assertion** in `access.rs` (carry-forward from worker.rs:149-156 and search.rs:57-62):

```rust
const _: fn() = || {
    fn _ss<T: Send + Sync>() {}
    _ss::<AccessTracker>();
};
```

### Decision 6: `demote_inactive` — signature and "inactive" semantics

**Signature** (from doc 6:728):

```rust
/// Only called by autoDream (Phase 8+). Phase 5 delivers but does NOT invoke.
pub(crate) fn demote_inactive(current_tier: u8, days_since_last_access: f64, importance: f64) -> u8 {
    match current_tier {
        1 if days_since_last_access > 90.0 && importance < 9.0 => 2,
        2 if days_since_last_access > 180.0 => 3,
        _ => current_tier,  // tier 1 with importance >= 9 is absolutely pivotal — never demoted
    }
}
```

**"Inactive" definition** (doc 6:728):
- Tier 1 → 2: `>90 days since last_accessed_at AND importance < 9.0`
- Tier 2 → 3: `>180 days since last_accessed_at` (no importance floor — always demoted)
- Tier 1 with `importance >= 9`: NEVER demoted (absolutely pivotal)

`compute_tier` (promotion-only, invoked at INSERT and flush) is the asymmetric counterpart. `demote_inactive` (demotion-only, invoked by autoDream) is the lazy path. Both live in `tier.rs`. Neither calls the other.

---

## Approaches

### Approach 1 (RECOMMENDED): AccessTracker as standalone worker + UDF compute_tier

- `access.rs`: `AccessTracker` struct with `UnboundedSender<Vec<i64>>` and a spawnable async `run()` method
- `tier.rs`: pure Rust `compute_tier` and `demote_inactive` + `register_tier_udf(conn)` wrapper
- `db.rs` modification: add `set_limit(SQLITE_LIMIT_VARIABLE_NUMBER, 32766)` + call `register_tier_udf` after existing setup calls
- `prompt_builder.rs` modification: add `access_tracker: Option<&AccessTracker>` parameter, fire-and-forget after hits
- `state.rs` modification: add `access_tracker: Arc<AccessTracker>` field, init in all three constructors

**Pros**: mirrors existing worker.rs/score.rs patterns exactly; single CTE UPDATE is atomic; no new crate; `test-util` already in dev-deps for timer tests
**Cons**: `db.rs` gains another setup call; `build_memory_prompt` signature grows by one param
**Effort**: Medium (~20 tasks as estimated)

### Approach 2: Fetch + compute in Rust (no UDF)

- Access worker fetches rows to update, calls `compute_tier` in Rust, issues individual/batch UPDATEs
- No new UDF registration

**Pros**: `compute_tier` tested purely in Rust without SQL wrapper
**Cons**: 2 round-trips per flush (SELECT + UPDATE) inside the same `.call()` block; more code; breaks the one-statement atomic CTE pattern from doc 6
**Effort**: Medium but messier

---

## Recommendation

Approach 1. The doc 6 CTE UPDATE pattern with `compute_tier` as UDF is already fully specified (doc 6:701-715). The UDF pattern has 4 precedents in score.rs. The tokio `test-util` feature covers timer tests without real sleeps. No new crate, no new Cargo.toml lines.

---

## Risks

1. **`SQLITE_LIMIT_VARIABLE_NUMBER` default is 999** — forgetting to call `set_limit` would cause silent flush truncation or a runtime error for batches > 333 rows. Must add to `new_for_test()` as well, or integration tests with large flush batches will fail under test conditions matching production.

2. **Timer test flakiness** — `tokio::time::pause()` only works if `start_paused = true` is set on the `#[tokio::test]` attribute; a missing annotation will cause tests to race real time. Use `#[tokio::test(start_paused = true)]` consistently.

3. **`build_memory_prompt` signature change** — one call site (in `commands/chat.rs` via `compute_memory_prefix`) must be updated to pass `Some(&state.access_tracker)`. If there are test call sites for `build_memory_prompt`, they must be updated to pass `None`. Check test file count before `apply`.

4. **`compute_tier` only promotes, never demotes** — this is intentional (doc 6:730-732). If someone calls `compute_tier` expecting demotion to happen during flush, they will get silent no-ops. The docstring MUST state "only promotes; use demote_inactive for demotion".

5. **`demote_inactive` is delivered but not wired** — it must be `pub(crate)` with a clear doc comment that autoDream is the caller. Without that comment, a future apply agent could be tempted to call it from the flush path.

6. **`tier.rs` registration call order** — `register_tier_udf` must be called AFTER `run_migrations` (the table schema is needed only for runtime queries, not for UDF registration, so order doesn't matter technically — but keeping it after migrations is consistent with `register_score_udfs`).

---

## Decisions NOT Resolved (flag for propose)

All 6 open questions from the Phase 5 plan are now resolved with code evidence. No unresolved decisions remain for propose.

The only item that needs a propose judgment call is: **should `register_tier_udf` live inside `register_score_udfs` in `score.rs`, or in a separate function in `tier.rs`?** Architecturally the separation is cleaner (`tier.rs` owns its UDF). This is a low-stakes cosmetic call — propose should lock it.

---

## Ready for Proposal

Yes. All 6 open decisions are grounded. Propose can write the contract (files, function signatures, SQL shape, test strategy) directly from this exploration.
