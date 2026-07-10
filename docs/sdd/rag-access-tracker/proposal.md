# Proposal: rag-access-tracker (Phase 5)

## Intent

Phase 4 retrieval reads memories but never records that they were read. Phase 5 adds a batched `AccessTracker` that updates `last_accessed_at` + `access_count` and promotes `tier` **off the hot path** — fire-and-forget, zero added retrieval latency. Without it, decay/tier scoring drifts and pivotal memories never climb tiers.

## Scope

### In Scope
- `access.rs` — `AccessTracker` struct + batched async worker (`tokio::select!`: 30s OR ≥500 entries).
- `tier.rs` — pure-Rust `compute_tier` (promotion-only) + `demote_inactive` + `register_tier_udf`.
- `prompt_builder.rs` — `build_memory_prompt` gains `access_tracker: Option<&AccessTracker>`; fires after hits assembled (line 95).
- `state.rs` — `access_tracker: Arc<AccessTracker>` in all 3 constructors; worker spawned in `lib.rs::run()`.
- `db.rs` — `set_limit(SQLITE_LIMIT_VARIABLE_NUMBER, 32766)` + `register_tier_udf` call in `open()` AND `new_for_test()`.

### Out of Scope
- **Calling** `demote_inactive` at runtime — delivered as `pub(crate)`, invoked by autoDream (doc 9) later.
- Any UI. `search.rs` signature change (stays untouched — cleaner cut is at the call site).

## Locked Decisions

| # | Decision | Rationale |
|---|----------|-----------|
| a | `register_tier_udf` is a **separate fn in `tier.rs`**, called alongside `register_score_udfs` | `tier.rs` owns its UDF; clean module boundary; low-stakes cosmetic |
| b | `record(&self, ids: Vec<i64>)` — **owned `Vec`** | Caller builds owned Vec; sender takes ownership; avoids internal `.to_vec()` |
| c | Flush errors → **log + continue, never panic** | Mirrors `worker.rs`; background worker must survive transient DB errors |
| d | `compute_tier` as **UDF delegating to pure-Rust fn** | Atomic single-CTE UPDATE (doc 6:701-715), 4 UDF precedents in `score.rs`, Rust fn separately unit-testable. Overrides plan line 256 parenthetical; aligns with plan:266 + doc 6 |

## Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| `set_limit` missing in `new_for_test()` → runtime explosion past 333 rows (1500 vars > 999 default) | High | Add to BOTH `new()` and `new_for_test()`; explicit task |
| Timer test races real time | Med | `#[tokio::test(start_paused = true)]` + `tokio::time::advance` on EVERY timer test; never sleep real seconds |
| Future agent calls `demote_inactive` from flush | Med | `pub(crate)` + docstring: "NOT called in Phase 5; autoDream owns runtime invocation" |
| `compute_tier` only promotes → silent no-op if used for demotion | Low | Docstring: "promotes only; use `demote_inactive`" |
| `build_memory_prompt` call sites break | Low | Update `commands/chat.rs` to `Some(&state.access_tracker)`; test sites pass `None` |

## Rollback Plan

New files (`access.rs`, `tier.rs`) deleted; revert `db.rs`/`state.rs`/`prompt_builder.rs`/`lib.rs` diffs. No schema migration, no Cargo.toml change → clean revert, baseline 390/390 restored.

## Dependencies

None new. `rusqlite 0.37 (functions)`, `tokio (test-util)` already present. Stack locked: tokio-rusqlite 0.7.0, sqlite-vec, NO sqlx.

## Success Criteria

- [ ] 1000 concurrent retrievals → tracker never stalls; hot-path latency unchanged.
- [ ] 500 buffered entries → automatic flush.
- [ ] Memory with `access_count >= 3` + `architectural_decision` promotes tier 2 → 1 in same flush.
- [ ] Baseline 390/390 Rust PASS holds; every `*-impl` preceded by `*-test`.
