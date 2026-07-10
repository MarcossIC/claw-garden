# Phase 4 — rag-retrieval — Tasks

> Strict TDD. Each `*-impl` is preceded by its `*-test`. The apply sub-agent works through this checklist top-to-bottom, mechanically. Every test goes RED FIRST (compile-error or assertion fail). Its paired impl makes it GREEN. Do NOT collapse pairs.

> **Baseline before starting**: `cargo test --lib --manifest-path src-tauri/Cargo.toml` → **315/315 PASS**. Record the exact count; verify is asked to detect zero regressions.

> **No `cargo build`** at any step (user CLAUDE.md forbids it). Verification gates: `cargo check --manifest-path src-tauri/Cargo.toml` for type-level, `cargo test --lib --manifest-path src-tauri/Cargo.toml` for behavior.

> **No `tokio::time::sleep(Duration::from_secs(...))`** in any test. All assertions are direct SQLite queries, string equality checks, or channel-based signaling.

> **All new code**: `log::` macros for logging (NOT `tracing::`), `thiserror` for errors, `pub(crate)` default, conventional commits only.

> **Alignment answers** (from spec.md §alignment table — treat as additive constraints):
> - Error strategy: extend `MemoryError` with `WorkspaceIo { path, source }` + `Backend(String)` variants.
> - `WorkspaceManager` constructed locally in `stream_chat` — NOT a field on `AppState`.
> - `user_query` = `&message` from `stream_chat` parameter.
> - `fetch_auto_rows` private helper runs 5 SELECTs in a single `Connection::call`.
> - Token cap = byte-length approximation: `AUTO_BYTE_CAP = AUTO_TOKEN_CAP * 4 = 6000 bytes`.
> - Date formatting = zero-dep `unix_secs_to_ymd` integer arithmetic (no `chrono`/`time`/`jiff`).
> - `K_FINAL = 10` constant in `prompt_builder.rs`.
> - `SOURCE: {TableKind:?} | TYPE: {memory_type}` format.
> - `current_focus` section = emit empty header only.
> - `memory/mod.rs` = `pub(crate) mod` declarations only, zero re-exports.

---

## CHECKPOINT 1 — Foundation (error model + module declarations)

**Pause point: run `cargo test --lib --manifest-path src-tauri/Cargo.toml`; verify = 315 baseline + N new (zero regressions).**

### T01-test — MemoryError new variants compile

**Trace**: REQ-INT-002 (alignment #1), D6
**File**: `src-tauri/src/memory/error.rs` (or wherever `MemoryError` is defined — confirm via `grep "enum MemoryError"`)
**Test name(s)**: `memory::tests::workspace_io_error_displays_path_and_source`, `memory::tests::backend_error_displays_message`, `memory::tests::workspace_io_error_is_send_sync`
**Description**: Write three tests that reference the two new `MemoryError` variants (`WorkspaceIo` and `Backend`) by constructing instances and asserting on Display output and `Send + Sync` bounds. All three must fail (compile error: variants do not exist yet) before the impl task.
**Acceptance**:
- `cargo test --lib memory::tests::workspace_io_error_displays_path_and_source` fails with compile error referencing unknown variant.
- `cargo test --lib memory::tests::backend_error_displays_message` fails with compile error.
- `cargo test --lib memory::tests::workspace_io_error_is_send_sync` fails with compile error.

### T01-impl — MemoryError new variants

**Trace**: REQ-INT-002, design §4.1
**File(s)**: `src-tauri/src/memory/error.rs` (the file containing `pub enum MemoryError`)
**Description**: Append two new variants to the existing `MemoryError` enum. `WorkspaceIo { path: std::path::PathBuf, #[source] source: std::io::Error }` with `#[error("workspace I/O failed at {path}: {source}")]`. `Backend(String)` with `#[error("memory backend channel error: {0}")]`. No other changes; existing `From` impls and existing variants are untouched.
**Acceptance**:
- `MemoryError::WorkspaceIo { path: PathBuf::from("/x"), source: io_err }` constructs without error.
- `MemoryError::Backend("closed".into())` constructs without error.
- `format!("{}", MemoryError::Backend("closed".into()))` contains `"closed"`.
- `format!("{}", MemoryError::WorkspaceIo { path: PathBuf::from("/x"), source: ... })` contains `"/x"`.
- All three T01 tests pass; all 315 baseline tests still pass.

---

### T02-test — memory/mod.rs module declarations (compile-time)

**Trace**: REQ-INT-002
**File**: `src-tauri/src/memory/mod.rs`
**Test name(s)**: compile-time only — `cargo check` will fail if the `mod` lines reference non-existent files.
**Description**: Add the four `pub(crate) mod` declarations to `memory/mod.rs` for `score`, `search`, `workspace`, and `prompt_builder`. Each referenced file does not exist yet, so `cargo check` fails with "file not found for module". Create stub files (empty or with a `// placeholder` comment) to unblock compilation, then confirm `cargo check` passes.
**Acceptance**:
- After adding the four `pub(crate) mod` lines, `cargo check` returns an error about missing files.
- This is the expected RED state before T02-impl.

### T02-impl — Create stub module files + mod declarations

**Trace**: REQ-INT-002, design §6.6
**File(s)**: `src-tauri/src/memory/mod.rs`, `src-tauri/src/memory/score.rs` (new), `src-tauri/src/memory/search.rs` (new), `src-tauri/src/memory/workspace.rs` (new), `src-tauri/src/memory/prompt_builder.rs` (new)
**Description**: Append the four module declarations to `memory/mod.rs` (after existing `pub mod` lines, NO new re-exports). Create four new empty stub files, each containing only a `// Phase 4 stub — filled in by subsequent tasks` comment. Confirm `cargo check` and `cargo test --lib` pass with zero regressions against the 315 baseline (the stubs introduce no symbols, so nothing breaks).
**Acceptance**:
- `memory/mod.rs` contains `pub(crate) mod score;`, `pub(crate) mod search;`, `pub(crate) mod workspace;`, `pub(crate) mod prompt_builder;`.
- All four stub files exist and are valid empty Rust modules.
- `cargo check --manifest-path src-tauri/Cargo.toml` succeeds.
- `cargo test --lib --manifest-path src-tauri/Cargo.toml` shows 315 pass, 0 failures.

---

### Verify — CHECKPOINT 1

Run: `cargo test --lib --manifest-path src-tauri/Cargo.toml`
Expected: ≥ 318 tests pass (315 baseline + T01 3 new tests). Zero failures.
If failure: STOP. Diagnose root cause before CHECKPOINT 2.

---

## CHECKPOINT 2 — Score UDFs (score.rs)

**Pause point: run `cargo test --lib --manifest-path src-tauri/Cargo.toml`; verify zero regressions before starting.**

### T03-test — `register_score_udfs` skeleton + `exp_decay` UDF

**Trace**: REQ-SCORE-001, REQ-STATE-001, D6
**File**: `src-tauri/src/memory/score.rs`
**Test name(s)**: `memory::score::tests::exp_decay_zero_age_returns_one`, `memory::score::tests::exp_decay_one_day_chat_lambda`, `memory::score::tests::exp_decay_seven_days_tool_error_lambda_approx_half`, `memory::score::tests::register_score_udfs_makes_exp_decay_callable`
**Description**: Write four tests. The first three call `SELECT exp_decay(?, ?)` against a `MemoryStore::new_for_test().await.unwrap()` and assert numeric results. The fourth calls `SELECT exp_decay(0, 0.023)` and asserts it returns `1.0` without a "no such function" error. All four must fail with `"no such function: exp_decay"` before the impl task. Use the validation pattern from design §7.2: `store.conn.call(|c| c.query_row("SELECT exp_decay(0, 0.023)", [], |r| r.get::<_, f64>(0))).await.unwrap()`.
**Acceptance**:
- All four tests fail with `SqliteFailure` or compile error before impl.
- Tests assert: `exp_decay(0, 0.023) == 1.0`, `(exp_decay(86400, 0.023) - 0.977).abs() < 0.001`, `(exp_decay(604800, 0.099) - 0.5).abs() < 0.01`.

### T03-impl — `register_score_udfs` skeleton + `exp_decay` UDF

**Trace**: REQ-SCORE-001, REQ-STATE-001, D6
**File(s)**: `src-tauri/src/memory/score.rs`, `src-tauri/src/memory/db.rs`
**Description**: Replace the stub in `score.rs` with the full module. Define `pub(crate) async fn register_score_udfs(conn: &tokio_rusqlite::Connection) -> Result<(), MemoryError>`. Inside its single `conn.call(|c| { ... Ok(()) }).await` closure, register only `exp_decay` for now (the other three come in T04–T06). `exp_decay` body: `let age_days = age_secs as f64 / 86_400.0; Ok((-lambda * age_days).exp())`. Also add the `register_score_udfs(&conn).await?` call to both `MemoryStore::open()` and `MemoryStore::new_for_test()` in `db.rs` (after `run_migrations`, before returning the store). Add the `use crate::memory::MemoryError;` import to `score.rs`.
**Acceptance**:
- All four T03 tests pass.
- All 315+ prior tests still pass (UDF registration is additive).
- `cargo check` is clean.

---

### T04-test — `lambda_for_type` UDF

**Trace**: REQ-SCORE-002, D6
**File**: `src-tauri/src/memory/score.rs`
**Test name(s)**: `memory::score::tests::lambda_user_preference`, `memory::score::tests::lambda_chat_message`, `memory::score::tests::lambda_tool_error`, `memory::score::tests::lambda_unknown_type_returns_default`
**Description**: Write four tests querying `SELECT lambda_for_type(?)` against `MemoryStore::new_for_test()`. Assert: `lambda_for_type('user_preference') == 0.0019`, `lambda_for_type('chat_message') == 0.023`, `lambda_for_type('tool_error') == 0.099`, `lambda_for_type('unknown_future_type') == 0.023`. All four fail with "no such function: lambda_for_type" before impl.
**Acceptance**:
- All four tests fail with SQLite function-not-found error before impl.
- Tests use absolute equality comparisons with `(result - expected).abs() < 1e-9`.

### T04-impl — `lambda_for_type` UDF

**Trace**: REQ-SCORE-002, D6
**File(s)**: `src-tauri/src/memory/score.rs`
**Description**: Add the `lambda_for_type` UDF registration inside the existing `conn.call()` closure in `register_score_udfs`. The UDF has arity 1, flags `SQLITE_UTF8 | SQLITE_DETERMINISTIC`. Body: a `match memory_type.as_str()` with six named arms and a default `_ => 0.023`. The six named types and their lambda values are: `user_preference → 0.0019`, `architectural_decision → 0.0039`, `session_summary → 0.0077`, `semantic_fact → 0.0039`, `chat_message → 0.023`, `tool_error → 0.099`.
**Acceptance**:
- All four T04 tests pass.
- All prior tests still pass.
- The `match` arm for `semantic_fact` explicitly listed (NOT covered by the default arm).

---

### T05-test — `importance_factor` UDF

**Trace**: REQ-SCORE-003, D6, R-NEW-8
**File**: `src-tauri/src/memory/score.rs`
**Test name(s)**: `memory::score::tests::importance_factor_min_value`, `memory::score::tests::importance_factor_max_value`, `memory::score::tests::importance_factor_mid_value`, `memory::score::tests::importance_factor_clamp_below_floor`, `memory::score::tests::importance_factor_clamp_above_ceiling`
**Description**: Write five tests querying `SELECT importance_factor(?)`. Assert: `importance_factor(1.0) == 0.55`, `importance_factor(10.0) == 1.0`, `importance_factor(5.0) == 0.75`, `importance_factor(0.0) == 0.55` (clamped to floor 1.0), `importance_factor(15.0) == 1.0` (clamped to ceiling 10.0). All five fail before impl.
**Acceptance**:
- Tests fail with "no such function: importance_factor" before impl.
- `importance_factor(5.0)` asserted with `(result - 0.75).abs() < 1e-9`.

### T05-impl — `importance_factor` UDF

**Trace**: REQ-SCORE-003, D6, R-NEW-8
**File(s)**: `src-tauri/src/memory/score.rs`
**Description**: Add `importance_factor` UDF registration inside the `conn.call()` closure. Arity 1, `SQLITE_UTF8 | SQLITE_DETERMINISTIC`. Body: `let imp = imp.clamp(1.0_f64, 10.0); Ok(0.5 + (imp / 20.0))`. The clamp is applied BEFORE the formula — this is the R-NEW-8 mitigation.
**Acceptance**:
- All five T05 tests pass.
- All prior tests still pass.
- The clamp is present and before the arithmetic (reviewers must verify order).

---

### T06-test — `dispute_factor` UDF

**Trace**: REQ-SCORE-004, D7
**File**: `src-tauri/src/memory/score.rs`
**Test name(s)**: `memory::score::tests::dispute_factor_active`, `memory::score::tests::dispute_factor_disputed`, `memory::score::tests::dispute_factor_superseded`, `memory::score::tests::dispute_factor_invalid`, `memory::score::tests::dispute_factor_unknown_status`
**Description**: Write five tests querying `SELECT dispute_factor(?)`. Assert: `dispute_factor('active') == 1.0`, `dispute_factor('disputed') == 0.6`, `dispute_factor('superseded') == 0.0`, `dispute_factor('invalid') == 0.0`, `dispute_factor('bogus_future_status') == 0.0`. All five fail before impl.
**Acceptance**:
- Tests fail with "no such function: dispute_factor" before impl.
- All five assertions use `(result - expected).abs() < 1e-9`.

### T06-impl — `dispute_factor` UDF (FULL, not a stub — per D7)

**Trace**: REQ-SCORE-004, D7
**File(s)**: `src-tauri/src/memory/score.rs`
**Description**: Add `dispute_factor` UDF registration inside the `conn.call()` closure. Arity 1, `SQLITE_UTF8 | SQLITE_DETERMINISTIC`. Body: `match status.as_str() { "active" => Ok(1.0_f64), "disputed" => Ok(0.6_f64), _ => Ok(0.0_f64), }`. Per D7, this is the FINAL implementation — Phase 6 inserts rows with `status='disputed'`/`'superseded'` but DOES NOT modify `score.rs`.
**Acceptance**:
- All five T06 tests pass.
- The `"disputed"` arm explicitly returns `0.6` (not computed).
- The wildcard arm covers `'superseded'`, `'invalid'`, and any future unknown status.
- All prior tests still pass.

---

### T07-test — UDF registration integration + all four callable

**Trace**: REQ-STATE-001, D6
**File**: `src-tauri/src/memory/score.rs`
**Test name(s)**: `memory::score::tests::register_score_udfs_makes_all_four_callable`, `memory::score::tests::full_score_expression_in_sql_integration`
**Description**: Two integration tests. First: call all four UDFs in a single `SELECT exp_decay(86400, lambda_for_type('chat_message')) * importance_factor(5.0) * dispute_factor('active')` query; assert result is in `(0.0, 1.0]`. Second: confirms all four UDFs exist on the same `new_for_test()` connection. These tests should ALREADY pass after T03-T06 impl tasks; if they do not, T07-impl corrects the wiring.
**Acceptance**:
- First test fails if any UDF registration is missing or the combined expression errors.
- Second test fails if the combined SQL returns an error.

### T07-impl — Verify wiring of all four UDFs in constructors

**Trace**: REQ-STATE-001, D6
**File(s)**: `src-tauri/src/memory/score.rs`, `src-tauri/src/memory/db.rs`
**Description**: If T07 tests are already green after T03–T06 (all four UDFs registered in the same `conn.call()` closure), no code change is needed — this is a verification task. If any test fails, ensure: (a) all four `create_scalar_function` calls are inside ONE `conn.call()` closure, (b) `register_score_udfs(&conn).await?` appears in BOTH `MemoryStore::open()` and `MemoryStore::new_for_test()` in `db.rs`, after `run_migrations`. Confirm the `?` propagates any registration error as `MemoryError`.
**Acceptance**:
- Both T07 tests pass.
- All 315+ prior tests pass (UDFs add no conflict with existing connection behavior).
- `cargo check` is clean.

---

### T08-test — Full score expression integration test

**Trace**: REQ-TEST-001, D6, D7
**File**: `src-tauri/src/memory/score.rs`
**Test name(s)**: `memory::score::tests::full_score_expression_in_sql_integration`
**Description**: One end-to-end test that evaluates the full score formula in a single SQL query against `MemoryStore::new_for_test()`. The query: `SELECT (1.0 - 0.1) * MAX(0.2, exp_decay(86400, lambda_for_type('chat_message'))) * importance_factor(5.0) * dispute_factor('active')`. Assert the result is approximately `0.66` (within `0.05` tolerance) and strictly in `(0.0, 1.0]`. This test may already pass after T07; if so, this is a documentation/verification task that confirms REQ-TEST-001 scenario.
**Acceptance**:
- Test computes `≈ 0.9 × MAX(0.2, 0.977) × 0.75 × 1.0 ≈ 0.66`.
- Result is in range `(0.5, 0.75)`.
- All prior tests still pass.

### T08-impl — Wire `register_score_udfs` into `MemoryStore::open()` + `new_for_test()`

**Trace**: REQ-STATE-001, D6
**File(s)**: `src-tauri/src/memory/db.rs`
**Description**: If this is not already done in T03-impl (it should be), add the `score::register_score_udfs(&conn).await?` call to both `MemoryStore::open()` and `MemoryStore::new_for_test()` in `db.rs`. This is purely additive: placed after `run_migrations(&conn).await?`, before the `Ok(Self { conn })` return. The `?` propagates any registration failure. Verify that all 15+ existing db.rs/worker.rs tests still pass (the UDFs are pure additions with no naming conflicts).
**Acceptance**:
- Both `open()` and `new_for_test()` call `score::register_score_udfs(&conn).await?`.
- All T03–T08 score tests pass.
- All 315+ prior tests pass.
- `cargo check` is clean.

---

### Verify — CHECKPOINT 2

Run: `cargo test --lib --manifest-path src-tauri/Cargo.toml`
Expected: ≥ 333 tests pass (315 baseline + ~18 new score tests). Zero failures.
If failure: STOP. Diagnose root cause before CHECKPOINT 3.

---

## CHECKPOINT 3 — Search (search.rs)

**Pause point: run `cargo test --lib --manifest-path src-tauri/Cargo.toml`; verify zero regressions before starting.**

### T09-test — `SearchScope` struct + 3 named constructors + Send/Sync assertion

**Trace**: REQ-SEARCH-001, REQ-STATE-002, D5
**File**: `src-tauri/src/memory/search.rs`
**Test name(s)**: `memory::search::tests::scope_all_default_values`, `memory::search::tests::scope_procedural_only_values`, `memory::search::tests::scope_factual_pivots_values`
**Description**: Write three unit tests. Each constructs a `SearchScope` via a named constructor and asserts all three field values. `all_default()` asserts `memory == Some(30)`, `episodic == Some(5)`, `procedural == Some(5)`. `procedural_only()` asserts `memory == None` and `episodic == None` and `procedural == Some(10)`. `factual_pivots()` asserts `episodic == None`, `memory == Some(5)`, `procedural == Some(5)`. All three fail with compile errors before impl.
**Acceptance**:
- Tests fail with "unresolved import" or "not found in this scope" compile errors before impl.
- Tests use direct equality on `Option<usize>` fields.

### T09-impl — `SearchScope` struct + 3 named constructors + Send/Sync assertions

**Trace**: REQ-SEARCH-001, REQ-STATE-002, D5
**File(s)**: `src-tauri/src/memory/search.rs`
**Description**: Replace the stub in `search.rs` with the `SearchScope` struct (`#[derive(Debug, Clone, Copy)]`, fields `memory: Option<usize>`, `episodic: Option<usize>`, `procedural: Option<usize>`) and its three `impl SearchScope` constructors. Add the compile-time `const _: fn()` block that calls `_ss::<SearchScope>()` and `_ss::<RankedHit>()` (even though `RankedHit` doesn't exist yet, stub it with `struct RankedHit;` temporarily — or add both structs together). All fields are `pub`. Add `use crate::memory::{MemoryError, MemoryStore};` and `use crate::memory::record::TableKind;` imports.
**Acceptance**:
- All three T09 tests pass.
- `cargo check` is clean (const block compiles without error for `SearchScope`).
- All prior tests still pass.

---

### T10-test — `RankedHit` struct + Send + sort test

**Trace**: REQ-SEARCH-002, D5, D3
**File**: `src-tauri/src/memory/search.rs`
**Test name(s)**: `memory::search::tests::ranked_hit_is_send`, `memory::search::tests::ranked_hit_sort_descending_by_score`
**Description**: Write two unit tests. `ranked_hit_is_send`: compile-time assertion `fn _s<T: Send>() {} _s::<RankedHit>();` inside a `#[test]`. `ranked_hit_sort_descending_by_score`: construct two `RankedHit` values with `score = 0.9` and `score = 0.3`; push both into a `Vec`, sort by `b.score.partial_cmp(&a.score)`; assert first element has `score == 0.9`. Both fail before impl.
**Acceptance**:
- `ranked_hit_is_send` fails with compile error referencing unknown struct `RankedHit`.
- `ranked_hit_sort_descending_by_score` fails with compile error.

### T10-impl — `RankedHit` struct

**Trace**: REQ-SEARCH-002, D5, D3
**File(s)**: `src-tauri/src/memory/search.rs`
**Description**: Add `#[derive(Debug, Clone)] pub(crate) struct RankedHit` with all nine fields: `id: i64`, `kind: TableKind`, `content: String`, `memory_type: String`, `tier: i64`, `status: String`, `score: f64`, `distance: f64` (exposed for Phase 6 — NOT hidden), `created_at: i64`. All fields `pub`. Update the compile-time `const _` block to include `_ss::<RankedHit>()` (the `_send_sync` call that proves `RankedHit: Send + Sync`).
**Acceptance**:
- Both T10 tests pass.
- `distance` field is `pub` (Phase 6 reads `hit.distance < 0.20` directly).
- All prior tests still pass.

---

### T11-test — `serialize_f32_query` helper (LE bytes)

**Trace**: REQ-SEARCH-003, D5
**File**: `src-tauri/src/memory/search.rs`
**Test name(s)**: `memory::search::tests::serialize_f32_query_produces_le_bytes`, `memory::search::tests::serialize_f32_query_length_matches`
**Description**: Write two unit tests for the private (not `pub`) `serialize_f32_query` function. First test: serialize `[1.0_f32, 0.0_f32]`; assert the first 4 bytes equal `1.0_f32.to_le_bytes()`. Second test: serialize a 384-element `vec![0.0_f32; 384]`; assert the output length is `384 * 4 = 1536` bytes. Both fail before impl because the function does not exist.
**Acceptance**:
- Tests fail with "not found in this scope" compile error before impl.
- Tests use direct byte comparison.

### T11-impl — `serialize_f32_query` helper

**Trace**: REQ-SEARCH-003, D5
**File(s)**: `src-tauri/src/memory/search.rs`
**Description**: Add private function `fn serialize_f32_query(v: &[f32]) -> Vec<u8>` to `search.rs`. Implementation: `v.iter().flat_map(|f| f.to_le_bytes()).collect()`. This encodes the query vector in little-endian format for the vec0 `MATCH` operator. It is a private helper used by the three `knn_*` functions. No `pub` or `pub(crate)` modifier.
**Acceptance**:
- Both T11 tests pass.
- Function is private (no pub modifier).
- All prior tests still pass.

---

### T12-test — `knn_memory` private helper (SEARCH_SQL_MEMORY)

**Trace**: REQ-SEARCH-003, REQ-SEARCH-004, REQ-SEARCH-005, D5
**File**: `src-tauri/src/memory/search.rs`
**Test name(s)**: `memory::search::tests::tier1_rows_excluded_from_results`, `memory::search::tests::superseded_rows_excluded_from_results`, `memory::search::tests::disputed_rows_included_with_degraded_score`
**Description**: Write three integration tests that use `MemoryStore::new_for_test()`. Each test hand-INSERTs rows directly via `store.conn.call(|c| c.execute("INSERT INTO memory ...", params![...]).map(|_| ()))` and then calls `search_all_memories` (or directly `knn_memory` if exposed via `#[cfg(test)]`). Tier-1 exclusion test: insert one row with `tier=1` and three with `tier=3`; assert only tier-3 rows appear. Superseded exclusion: insert one `status='superseded'` row; assert it does NOT appear. Disputed inclusion: insert one `status='disputed'` row; assert it DOES appear in results. All three fail before `knn_memory` exists.
**Acceptance**:
- All three tests fail with compile errors or empty results before impl.
- Tests insert rows with explicit `embedding` bytes (can use `vec![0.0_f32; 384]` serialized) and verify filter behavior.

### T12-impl — `knn_memory` private helper + SEARCH_SQL_MEMORY constant

**Trace**: REQ-SEARCH-003, REQ-SEARCH-004, REQ-SEARCH-005, D5
**File(s)**: `src-tauri/src/memory/search.rs`
**Description**: Define the `SEARCH_SQL_MEMORY` string constant and the private `async fn knn_memory(store: &MemoryStore, q_bytes: &[u8], agent_id: &str, k: usize) -> Result<Vec<RankedHit>, MemoryError>` function. The SQL must JOIN `memory m` with `memory_vectors knn` (using the vec0 MATCH syntax: `FROM memory_vectors knn JOIN memory m ON m.id = knn.rowid WHERE knn.embedding MATCH ? AND k = ?`). WHERE clause MUST include: `m.status IN ('active', 'disputed')`, `m.deleted_at IS NULL`, `m.agent_id = ?`, `m.tier > 1`. SELECT must include the score expression: `(1.0 - knn.distance) * MAX(CASE m.tier WHEN 1 THEN 0.5 WHEN 2 THEN 0.2 ELSE 0.05 END, exp_decay(strftime('%s','now') - m.last_accessed_at, lambda_for_type(m.memory_type))) * importance_factor(m.importance) * dispute_factor(m.status) AS score`. SELECT columns: `m.id, m.content, m.memory_type, m.tier, m.status, knn.distance, m.created_at, score`. Row mapper hardcodes `kind: TableKind::Memory`. Bind parameters: `?1 = q_bytes`, `?2 = k`, `?3 = agent_id`. Map `tokio_rusqlite::Error` to `MemoryError::Backend`.
**Acceptance**:
- All three T12 tests pass.
- SQL WHERE clause contains all four required predicates including `m.tier > 1`.
- Score formula matches design §2.2 exactly.
- All prior tests still pass.

---

### T13-test — `knn_episodic` + `knn_procedural` (hardcoded lambdas)

**Trace**: REQ-SEARCH-005, D5
**File**: `src-tauri/src/memory/search.rs`
**Test name(s)**: `memory::search::tests::score_formula_integration_spot_check`, `memory::search::tests::disputed_status_reduces_score_by_factor_0_6`
**Description**: Write two tests that verify the score formula behavior. `score_formula_integration_spot_check`: hand-INSERT a memory row with known values (`tier=2`, `last_accessed_at=(now-86400)`, `memory_type='chat_message'`, `importance=5.0`, `status='active'`); call `search_all_memories` with a `[0.0; 384]` query vector; assert the result score is approximately `≈ 0.66` (within `0.1` tolerance). `disputed_status_reduces_score_by_factor_0_6`: insert same row with `status='disputed'`; assert its score is `≤ 0.6 × (score if 'active')`. Both fail before `knn_episodic` and `knn_procedural` exist (needed by `search_all_memories`).
**Acceptance**:
- Both tests fail with compile errors or incorrect results before impl.
- Tests use `MemoryStore::new_for_test()` and hand-INSERTs with explicit embedding bytes.

### T13-impl — `knn_episodic` + `knn_procedural` (hardcoded lambdas)

**Trace**: REQ-SEARCH-005, D5
**File(s)**: `src-tauri/src/memory/search.rs`
**Description**: Define `SEARCH_SQL_EPISODIC` and `SEARCH_SQL_PROCEDURAL` constants and their corresponding private `async fn knn_episodic(...)` and `async fn knn_procedural(...)` helpers. `knn_episodic` JOINs `episodic_trajectory epi` with `episodic_vectors knn`; uses hardcoded lambda `0.0077` in the `exp_decay(...)` call (NOT `lambda_for_type`). `knn_procedural` JOINs `procedural_skill pro` with `procedural_vectors knn`; uses hardcoded lambda `0.0039`. Both share the same WHERE predicate structure as `knn_memory`: `status IN ('active', 'disputed')`, `deleted_at IS NULL`, `agent_id = ?`, `tier > 1`. Row mapper for episodic hardcodes `kind: TableKind::Episodic`; procedural hardcodes `kind: TableKind::Procedural`. If these `TableKind` variants don't exist yet, check `record.rs` and add them if missing.
**Acceptance**:
- Both T13 tests pass.
- Episodic and procedural helpers use hardcoded lambdas (not `lambda_for_type`).
- All prior tests still pass.

---

### T14-test — `search_all_memories` (try_join! + merge + sort + truncate + log::debug! timing)

**Trace**: REQ-SEARCH-003, REQ-SEARCH-006, D1, D5
**File**: `src-tauri/src/memory/search.rs`
**Test name(s)**: `memory::search::tests::search_returns_results_sorted_by_score_desc`, `memory::search::tests::search_respects_scope_none_skips_table`, `memory::search::tests::search_truncates_to_k_final`, `memory::search::tests::search_empty_db_returns_empty_vec`
**Description**: Write four tests for `search_all_memories`. Sorted results: insert 3 memory rows with different importances; call with `all_default()` scope and `k_final=3`; assert results are sorted `score DESC`. Scope none: call with `SearchScope { memory: None, episodic: None, procedural: None }`; assert result is `Ok(vec![])` (all tables skipped). Truncation: insert 5 rows and call with `k_final=2`; assert exactly 2 hits returned. Empty DB: call on empty store; assert `Ok(vec![])`. All four fail before `search_all_memories` exists.
**Acceptance**:
- All four tests fail with "not found in this scope" before impl.
- Sorted test verifies `results[0].score >= results[1].score >= results[2].score`.
- No `tokio::time::sleep` in any test.

### T14-impl — `search_all_memories` (public function)

**Trace**: REQ-SEARCH-003, D1, D5
**File(s)**: `src-tauri/src/memory/search.rs`
**Description**: Define `pub(crate) async fn search_all_memories(store: &MemoryStore, embedding: &[f32], agent_id: &str, scope: SearchScope, k_final: usize) -> Result<Vec<RankedHit>, MemoryError>`. Body: serialize `embedding` via `serialize_f32_query`; run `tokio::try_join!` with three async branches (each checking `scope.memory/episodic/procedural` and calling the appropriate `knn_*` helper or returning `Ok(Vec::new())`); chain all three result vecs; sort by `b.score.partial_cmp(&a.score).unwrap_or(Equal)`; truncate to `k_final`; emit `log::debug!("retrieval={}ms hits={} pool=({}+{}+{}) k_final={}", t0.elapsed().as_millis(), all.len(), scope.memory.unwrap_or(0), scope.episodic.unwrap_or(0), scope.procedural.unwrap_or(0), k_final)`. Use `std::time::Instant::now()` before the join for timing.
**Acceptance**:
- All four T14 tests pass.
- `log::debug!` call is present after the join block.
- `tokio::try_join!` is used (not sequential awaits).
- All prior tests still pass.

---

### T15-test — Integration: 5-row corpus, k_final=3, tier=1 excluded

**Trace**: REQ-TEST-002, D5
**File**: `src-tauri/src/memory/search.rs`
**Test name(s)**: `memory::search::tests::integration_5_rows_fake_embedder_top3_sorted`
**Description**: One end-to-end integration test. Use `MemoryStore::new_for_test()` and `FakeEmbedder` (from `crate::memory::embedder::FakeEmbedder`). Hand-INSERT 5 rows: 2 with `tier=3`, 1 with `tier=1`, 2 with `tier=2`; span at least 2 `memory_type` values. Call `embedder.embed("test").await.unwrap()` to get a 384-dim vector, then call `search_all_memories(store, &q_vec, "agent-test", SearchScope::all_default(), 3)`. Assert: exactly 3 hits returned, none has `tier==1`, all scores in `(0.0, 1.0]`, results are sorted `score DESC`.
**Acceptance**:
- Test fails with compile error or assertion error before this CHECKPOINT is complete.
- Test verifies that the tier-1 row (which was INSERTed) does NOT appear in the 3 results.
- No `tokio::time::sleep` in the test.
- Test must import `FakeEmbedder` via `use crate::memory::embedder::FakeEmbedder;` within `#[cfg(test)]` block.

### T15-impl — Fix any issues surfaced by T15 integration test

**Trace**: REQ-TEST-002, D5
**File(s)**: `src-tauri/src/memory/search.rs`
**Description**: If T15-test passes after T14-impl, this is a verification-only task (no new code). If the test fails, diagnose: (a) check that `memory_vectors` INSERT uses the same serialization as `serialize_f32_query` (LE bytes, 4 bytes per f32); (b) verify the `MATCH` + `k =` syntax matches the vec0 extension API used in Phase 3's `store_insert.rs`; (c) verify the WHERE clause in `SEARCH_SQL_MEMORY` is correct. Fix the root cause. Do NOT add `tokio::time::sleep` as a workaround.
**Acceptance**:
- `integration_5_rows_fake_embedder_top3_sorted` passes.
- All prior tests still pass.
- `cargo check` is clean.

---

### Verify — CHECKPOINT 3

Run: `cargo test --lib --manifest-path src-tauri/Cargo.toml`
Expected: ≥ 348 tests pass (315 baseline + ~18 score tests + ~15 new search tests). Zero failures.
If failure: STOP. Diagnose root cause before CHECKPOINT 4.

---

## CHECKPOINT 4 — Workspace (workspace.rs)

**Pause point: run `cargo test --lib --manifest-path src-tauri/Cargo.toml`; verify zero regressions before starting.**

### T16-test — `WorkspaceManager` + constants + Send/Sync + `workspace_path` tests

**Trace**: REQ-WORKSPACE-001, REQ-WORKSPACE-002, D4, D9
**File**: `src-tauri/src/memory/workspace.rs`
**Test name(s)**: `memory::workspace::tests::path_with_project_id`, `memory::workspace::tests::path_with_no_project_uses_default_segment`, `memory::workspace::tests::workspace_manager_is_send_sync`, `memory::workspace::tests::markers_locate_auto_section_in_existing_file`, `memory::workspace::tests::missing_markers_creates_file_with_both_sections`
**Description**: Write five tests. Path tests: construct `WorkspaceManager::new(PathBuf::from("/base/workspaces"))` and call `workspace_path`. With `Some("proj-abc")` assert result ends with `"proj-abc/workspace.md"` and contains `"agent-1"`. With `None` assert result ends with `"default/workspace.md"`. Send/Sync: compile-time `fn _ss<T: Send + Sync>() {} _ss::<WorkspaceManager>();` inside a `#[test]`. Marker tests: construct a string containing `AUTO_START` and `AUTO_END`; verify a simple substring search finds both (testing that the constants have the exact values). All five fail before impl.
**Acceptance**:
- All five tests fail with compile errors before impl.
- Path tests use `assert!(path.to_string_lossy().contains(...))` assertions.
- Marker test verifies exact constant string values.

### T16-impl — `WorkspaceManager` + constants

**Trace**: REQ-WORKSPACE-001, REQ-WORKSPACE-002, D4, D9
**File(s)**: `src-tauri/src/memory/workspace.rs`
**Description**: Replace the stub in `workspace.rs` with: (1) The five `pub(crate) const` declarations (`DEFAULT_PROJECT_SEGMENT`, `AUTO_START`, `AUTO_END`, `USER_START`, `USER_END`, `AUTO_TOKEN_CAP: usize = 1500`, `AUTO_BYTE_CAP: usize = AUTO_TOKEN_CAP * 4`). (2) `#[derive(Debug, Clone)] pub(crate) struct WorkspaceManager { workspaces_root: PathBuf }`. (3) `impl WorkspaceManager { pub(crate) fn new(workspaces_root: PathBuf) -> Self`, `pub(crate) fn workspace_path(&self, agent_id: &str, project_id: Option<&str>) -> PathBuf }`. `workspace_path` body: `self.workspaces_root.join(agent_id).join(project_id.unwrap_or(DEFAULT_PROJECT_SEGMENT)).join("workspace.md")`. Add the compile-time `const _: fn()` block for `WorkspaceManager: Send + Sync + Clone`. Add required imports.
**Acceptance**:
- All five T16 tests pass.
- Compile-time Send/Sync/Clone const block is active.
- `workspace_path` uses `DEFAULT_PROJECT_SEGMENT` constant (not the literal string `"default"` inline).
- All prior tests still pass.

---

### T17-test — `read()` with NotFound → Ok("")

**Trace**: REQ-WORKSPACE-004, D4, R-NEW-3
**File**: `src-tauri/src/memory/workspace.rs`
**Test name(s)**: `memory::workspace::tests::read_missing_file_returns_ok_empty`, `memory::workspace::tests::read_existing_file_returns_content`
**Description**: Write two async tests. First: create a `WorkspaceManager` with a `tempfile::tempdir()` root; call `read("agent-1", None)` on a path that does not exist; assert the result is `Ok("")`. Second: pre-write a file with `tokio::fs::write` containing `"hello world"`; call `read("agent-1", None)`; assert `Ok("hello world")`. Both fail before `read()` exists.
**Acceptance**:
- Both tests are `#[tokio::test]`.
- First test fails with compile error (method `read` does not exist) before impl.
- Second test verifies that existing file content is returned exactly.

### T17-impl — `read()` with NotFound → Ok("")

**Trace**: REQ-WORKSPACE-004, D4, R-NEW-3
**File(s)**: `src-tauri/src/memory/workspace.rs`
**Description**: Add `pub(crate) async fn read(&self, agent_id: &str, project_id: Option<&str>) -> Result<String, MemoryError>` to `WorkspaceManager`. Body: `let path = self.workspace_path(agent_id, project_id); match tokio::fs::read_to_string(&path).await { Ok(s) => Ok(s), Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()), Err(e) => Err(MemoryError::WorkspaceIo { path, source: e }) }`.
**Acceptance**:
- Both T17 tests pass.
- `NotFound` is mapped to `Ok(String::new())` (NOT propagated as error).
- Any other IO error is mapped to `MemoryError::WorkspaceIo`.
- All prior tests still pass.

---

### T18-test — `split_existing` + `render_file` (USER section byte-for-byte preservation)

**Trace**: REQ-WORKSPACE-002, REQ-WORKSPACE-003, D4
**File**: `src-tauri/src/memory/workspace.rs`
**Test name(s)**: `memory::workspace::tests::user_section_preserved_byte_for_byte`, `memory::workspace::tests::zero_tier1_rows_produces_empty_auto_section_with_headers`
**Description**: Write two async tests. `user_section_preserved_byte_for_byte`: create a TempDir; pre-write a workspace file containing both AUTO and USER sections with the USER section having content `"  my custom notes\n  with trailing space  \n"` (including exact whitespace). Call `regenerate_auto_section` with an empty `MemoryStore::new_for_test()`. Read the resulting file back; assert the USER section content is byte-for-byte `"  my custom notes\n  with trailing space  \n"`. `zero_tier1_rows_produces_empty_auto_section_with_headers`: call `regenerate_auto_section` with zero Tier 1 rows; read the file; assert it contains the AUTO markers, contains the `## User Preferences (always active)` sub-header, and contains zero bullet lines. Both fail before `regenerate_auto_section` exists.
**Acceptance**:
- Both tests fail with compile errors before impl.
- First test asserts exact byte equality for the USER section (no whitespace normalization, no Unicode change).
- Both tests are `#[tokio::test]`.

### T18-impl — `split_existing` + `render_file` + `render_auto_section` (private helpers)

**Trace**: REQ-WORKSPACE-002, REQ-WORKSPACE-003, D4
**File(s)**: `src-tauri/src/memory/workspace.rs`
**Description**: Implement the private parsing and rendering helpers needed by `regenerate_auto_section`. `fn split_existing(content: &str)` does substring search for `AUTO_START`, `AUTO_END`, `USER_START`, `USER_END` and returns byte ranges/slices for each section. `fn render_auto_section(rows: &AutoRows) -> String` emits the markdown shape: `AUTO_START\n`, the five sub-section headers (`## User Preferences (always active)`, `## Architectural Decisions`, `## Procedural Skills`, `## Current Session Focus`, `## Active Workspace Disputes`), any row bullets under each header, `AUTO_END\n`. `fn render_file(before_auto: &str, auto_section: &str, between: &str, user_section: &str, after: &str) -> String` reassembles. The `AutoRows` struct (private): fields `user_prefs: Vec<(String, f64)>` (content + importance), `arch_decisions: Vec<(String, f64)>`, `top_skills: Vec<(String, i64)>` (content + access_count), `current_focus: Option<String>`, `workspace_disputes: Vec<(i64, i64)>`. In Phase 4, `current_focus` is always `None` and `workspace_disputes` is always empty. NOTE: `regenerate_auto_section` itself is NOT yet implemented here (that comes in T20-impl); this task only implements the private helpers needed by it.
**Acceptance**:
- Both T18 tests pass (requires partial `regenerate_auto_section` stub to call `render_auto_section`).
- USER section content is preserved byte-for-byte in the re-assembled file.
- Sub-section headers appear even when there are zero row bullets.
- All prior tests still pass.

---

### T19-test — `fetch_auto_rows` + `render_auto_section` (5 SELECTs, byte-cap recovery)

**Trace**: REQ-WORKSPACE-003, D4, alignment #4, #5
**File**: `src-tauri/src/memory/workspace.rs`
**Test name(s)**: `memory::workspace::tests::auto_section_respects_1500_token_cap`, `memory::workspace::tests::creates_directory_and_file_on_first_run`
**Description**: Write two async tests. `auto_section_respects_1500_token_cap`: insert 200 `user_preference` memory rows with `importance=10.0` and `tier=1` into `MemoryStore::new_for_test()`; call `regenerate_auto_section`; read the resulting AUTO section bytes; assert `AUTO section content bytes ≤ AUTO_BYTE_CAP (6000)`. `creates_directory_and_file_on_first_run`: create a TempDir but do NOT pre-create the workspace subdirectory; call `regenerate_auto_section` on a path inside a non-existent subdirectory; assert the call returns `Ok(())` and the file now exists. Both fail before `fetch_auto_rows` and the full `regenerate_auto_section` implementation exist.
**Acceptance**:
- Both tests fail with compile errors or panics before impl.
- Byte-cap test inserts rows via direct SQL into the `memory` table (no embedding needed — workspace query filters by `tier=1`, not by embedding).
- Directory creation test uses `tokio::fs::metadata` or `std::fs::exists` to verify file was created.

### T19-impl — `fetch_auto_rows` private helper

**Trace**: REQ-WORKSPACE-003, D4, alignment #4
**File(s)**: `src-tauri/src/memory/workspace.rs`
**Description**: Implement the private `async fn fetch_auto_rows(store: &MemoryStore, agent_id: &str, project_id: Option<&str>) -> Result<AutoRows, MemoryError>` helper. This runs FIVE queries inside a SINGLE `store.conn.call(|c| { ... })` closure: (1) `SELECT content, importance FROM memory WHERE memory_type='user_preference' AND importance >= 8 AND agent_id=? AND tier=1 AND deleted_at IS NULL ORDER BY importance DESC`; (2) `SELECT content, importance FROM memory WHERE memory_type='architectural_decision' AND importance >= 8 AND agent_id=? AND project_id=? AND tier=1 AND deleted_at IS NULL ORDER BY importance DESC`; (3) `SELECT content, access_count FROM procedural_skill WHERE agent_id=? AND tier=1 AND deleted_at IS NULL ORDER BY access_count DESC LIMIT 5`; (4) current_focus stub — returns `None`; (5) workspace disputes stub — returns empty `Vec`. Assemble into `AutoRows`. Also implement the byte-cap recovery logic in `render_auto_section`: if assembled content exceeds `AUTO_BYTE_CAP`, trim sections in reverse priority order (disputes → focus → skills → arch_decisions → user_prefs are NEVER trimmed as a last resort).
**Acceptance**:
- Both T19 tests pass.
- All five SELECT categories are queried (even if 4 and 5 are stubs returning empty in Phase 4).
- Byte cap enforcement is in `render_auto_section` using `content.len() <= AUTO_BYTE_CAP` check.
- All prior tests still pass.

---

### T20-test — `regenerate_auto_section` e2e (create_dir_all + write)

**Trace**: REQ-WORKSPACE-003, D4
**File**: `src-tauri/src/memory/workspace.rs`
**Test name(s)**: `memory::workspace::tests::write_failure_returns_workspace_io_error`
**Description**: Write one test for the error path: mock a workspace path that is not writable (on Windows, use a file-as-parent trick: create a FILE at the directory path that should be the parent, so `create_dir_all` will fail). Call `regenerate_auto_section`; assert the result is `Err(MemoryError::WorkspaceIo { .. })`. This test is platform-dependent; if the approach is fragile on Windows, use a path under a read-only location (e.g., `C:\Windows\workspace.md` — should fail with permission denied). This test fails before the full `regenerate_auto_section` implementation.
**Acceptance**:
- Test fails with compile error (method does not exist) before impl.
- Test asserts `matches!(result, Err(MemoryError::WorkspaceIo { .. }))`.

### T20-impl — `regenerate_auto_section` e2e (create_dir_all + write)

**Trace**: REQ-WORKSPACE-003, D4
**File(s)**: `src-tauri/src/memory/workspace.rs`
**Description**: Implement the full `pub(crate) async fn regenerate_auto_section(&self, agent_id: &str, project_id: Option<&str>, store: &MemoryStore) -> Result<(), MemoryError>`. Body: (1) `let path = self.workspace_path(agent_id, project_id)`. (2) Read existing file (may be empty on first run). (3) Call `fetch_auto_rows(store, agent_id, project_id).await?`. (4) Generate new AUTO section via `render_auto_section(&rows)`. (5) Use `split_existing` to locate AUTO/USER sections in the existing content; if the file was empty/missing, build fresh content with AUTO section + empty USER section. (6) `tokio::fs::create_dir_all(path.parent().unwrap()).await.map_err(|e| MemoryError::WorkspaceIo { path: path.clone(), source: e })?`. (7) `tokio::fs::write(&path, assembled_content).await.map_err(|e| MemoryError::WorkspaceIo { path: path.clone(), source: e })?`. Return `Ok(())`.
**Acceptance**:
- T20 write-failure test passes.
- All T18/T19 tests also pass (they rely on the full `regenerate_auto_section`).
- `create_dir_all` is called before `write` (directory may not exist on first run).
- All prior tests still pass.
- `cargo check` is clean.

---

### Verify — CHECKPOINT 4

Run: `cargo test --lib --manifest-path src-tauri/Cargo.toml`
Expected: ≥ 363 tests pass (315 baseline + ~18 score + ~15 search + ~10 workspace). Zero failures.
If failure: STOP. Diagnose root cause before CHECKPOINT 5.

---

## CHECKPOINT 5 — Prompt builder + stream_chat integration

**Pause point: run `cargo test --lib --manifest-path src-tauri/Cargo.toml`; verify zero regressions before starting.**

### T21-test — Constants + `unix_secs_to_ymd` + `format_hit_line` + `format_conflict_pair` + `bucket_by_recency`

**Trace**: REQ-PROMPT-002, REQ-PROMPT-003, D8, alignment #6, #8
**File**: `src-tauri/src/memory/prompt_builder.rs`
**Test name(s)**: `memory::prompt_builder::tests::unix_secs_zero_is_1970_01_01`, `memory::prompt_builder::tests::unix_secs_known_epoch_to_date`, `memory::prompt_builder::tests::format_hit_line_emits_canonical_format`, `memory::prompt_builder::tests::bucket_by_recency_splits_at_7_days`
**Description**: Write four unit tests. `unix_secs_zero_is_1970_01_01`: call `unix_secs_to_ymd(0)`; assert result is `(1970, 1, 1)`. `unix_secs_known_epoch_to_date`: call `unix_secs_to_ymd(1_700_000_000)`; assert result is `(2023, 11, 14)` (verified known epoch). `format_hit_line_emits_canonical_format`: construct a `RankedHit` with `created_at=1_700_000_000`, `kind=TableKind::Memory`, `memory_type="chat_message"`, `content="test content"`; call `format_hit_line`; assert the result starts with `"[2023-11-14] SOURCE: Memory | TYPE: chat_message"` and contains `"CONTENT: test content"`. `bucket_by_recency_splits_at_7_days`: construct 3 hits where 2 have `created_at = now - 86400` (1 day ago) and 1 has `created_at = now - 30 * 86400` (30 days ago); call `bucket_by_recency(hits, now_secs)`; assert RECENT has 2 elements and LONGTERM has 1 element. All four fail before impl.
**Acceptance**:
- All four tests fail with compile errors before impl.
- `format_hit_line` test asserts exact format string prefix (case-sensitive).
- `bucket_by_recency` test uses `std::time::SystemTime` or a fixed `now_secs` value.

### T21-impl — Constants + `unix_secs_to_ymd` + pure helpers

**Trace**: REQ-PROMPT-002, REQ-PROMPT-003, D8, alignment #6, #7, #8
**File(s)**: `src-tauri/src/memory/prompt_builder.rs`
**Description**: Replace the `prompt_builder.rs` stub with: (1) Module-level constants: `const SEP_WORKSPACE: &str = "=== WORKSPACE ==="`, `const SEP_RECENT: &str = "=== RECENT CONTEXT (Last 7 Days) ==="`, `const SEP_LONGTERM: &str = "=== LONG-TERM KNOWLEDGE ==="`, `const SEP_CONFLICTS: &str = "=== ACTIVE CONFLICTS (review needed) ==="`, `const RECENT_WINDOW_SECS: i64 = 7 * 86_400`, `const K_FINAL: usize = 10`. (2) Private `fn unix_secs_to_ymd(ts: i64) -> (i32, u32, u32)` using Howard Hinnant's integer-arithmetic algorithm (~25 lines, no `chrono`/`time` dep). (3) Private `fn format_hit_line(hit: &RankedHit) -> String` that produces `"[YYYY-MM-DD] SOURCE: {kind:?} | TYPE: {memory_type}\nCONTENT: {content}"`. (4) Private `struct ConflictPair` with nine fields (matching design §2.4). (5) Private `fn format_conflict_pair(p: &ConflictPair) -> String` producing the exact four-line format from design §3.5. (6) Private `fn bucket_by_recency(hits: Vec<RankedHit>, now_secs: i64) -> (Vec<RankedHit>, Vec<RankedHit>)` partitioning by `hit.created_at >= now_secs - RECENT_WINDOW_SECS`. Add required imports: `use crate::memory::{MemoryError, MemoryStore, search::{search_all_memories, RankedHit, SearchScope}, workspace::WorkspaceManager, embedder::Embedder}; use std::sync::Arc;`.
**Acceptance**:
- All four T21 tests pass.
- `unix_secs_to_ymd(0)` returns `(1970, 1, 1)` exactly.
- `unix_secs_to_ymd(1_700_000_000)` returns `(2023, 11, 14)` exactly.
- No new Cargo dependencies (zero-dep date arithmetic).
- All prior tests still pass.

---

### T22-test — `fetch_active_conflicts` + CONFLICTS SQL (json_each on NULL = 0 rows)

**Trace**: REQ-PROMPT-004, D3, D8, R-NEW-7
**File**: `src-tauri/src/memory/prompt_builder.rs`
**Test name(s)**: `memory::prompt_builder::tests::conflicts_query_zero_disputed_rows`, `memory::prompt_builder::tests::conflicts_query_one_disputed_pair_renders_correctly`, `memory::prompt_builder::tests::json_each_null_returns_zero_rows_no_error`
**Description**: Write three async tests using `MemoryStore::new_for_test()`. `conflicts_query_zero_disputed_rows`: call `fetch_active_conflicts` with an empty store; assert `Ok(vec![])`. `conflicts_query_one_disputed_pair_renders_correctly`: hand-INSERT two rows via `store.conn.call(...)` — first row is the "old" memory (status='active'), second row is the "new" memory (status='disputed', `disputed_against=json('[' || old_id || ']')`); call `fetch_active_conflicts`; assert result has 1 element with matching `old_id` and `new_id`. `json_each_null_returns_zero_rows_no_error`: insert a row with `status='disputed'` and `disputed_against IS NULL`; call `fetch_active_conflicts`; assert `Ok(vec![])` (no panic, no error). All three fail before `fetch_active_conflicts` exists.
**Acceptance**:
- All three tests fail with compile errors before impl.
- Tests hand-INSERT rows with explicit column values (`id`, `content`, `status`, `disputed_against`, `agent_id`, etc.).
- `json_each_null_returns_zero_rows_no_error` specifically exercises the R-NEW-7 safety property.

### T22-impl — `fetch_active_conflicts` private function

**Trace**: REQ-PROMPT-004, D3, D8, R-NEW-7
**File(s)**: `src-tauri/src/memory/prompt_builder.rs`
**Description**: Implement private `async fn fetch_active_conflicts(store: &MemoryStore, agent_id: &str) -> Result<Vec<ConflictPair>, MemoryError>`. The SQL is EXACTLY from design §2.4: `SELECT m_old.id, m_old.created_at, m_old.content, m_old.importance, m_old.tier, m_new.id, m_new.created_at, m_new.content, m_new.importance FROM memory m_new JOIN memory m_old ON m_old.id IN (SELECT value FROM json_each(m_new.disputed_against)) WHERE m_new.status = 'disputed' AND m_new.agent_id = ?1 AND m_new.deleted_at IS NULL AND m_old.deleted_at IS NULL`. On SQL error, return `Err(MemoryError::Sqlite(_))` — the caller (`build_memory_prompt`) wraps this in `unwrap_or_else(|e| { log::warn!(...); Vec::new() })`. Row mapper constructs `ConflictPair` from the 9 selected columns.
**Acceptance**:
- All three T22 tests pass.
- SQL includes `m_new.deleted_at IS NULL` AND `m_old.deleted_at IS NULL` in the WHERE clause.
- No NULL guard is needed for `json_each(NULL)` — SQLite handles it correctly.
- All prior tests still pass.

---

### T23-test — `build_memory_prompt` assembly + 4 failure-branch degradations + golden e2e

**Trace**: REQ-PROMPT-001, REQ-PROMPT-002, REQ-PROMPT-003, REQ-PROMPT-004, REQ-TEST-003, D2, D4, D8, R-NEW-3
**File**: `src-tauri/src/memory/prompt_builder.rs`
**Test name(s)**: `memory::prompt_builder::tests::all_empty_returns_empty_string`, `memory::prompt_builder::tests::workspace_only_returns_workspace_block`, `memory::prompt_builder::tests::embedder_none_skips_retrieval_no_panic`, `memory::prompt_builder::tests::embedder_none_emits_workspace_only_if_present`, `memory::prompt_builder::tests::three_hits_split_into_recent_and_longterm_blocks`, `memory::prompt_builder::tests::disputed_pair_renders_conflicts_block`, `memory::prompt_builder::tests::no_disputed_rows_omits_conflicts_block`, `memory::prompt_builder::tests::separator_strings_match_exactly`, `memory::prompt_builder::tests::recent_longterm_split_by_7day_window`, `memory::prompt_builder::tests::all_old_hits_go_to_longterm_only`, `memory::prompt_builder::tests::all_recent_hits_go_to_recent_only`, `memory::prompt_builder::tests::e2e_full_4_block_output`
**Description**: Write twelve tests for `build_memory_prompt`. Key tests: `all_empty_returns_empty_string` — empty store + missing workspace + `embedder=None` → `Ok("")`. `workspace_only_returns_workspace_block` — workspace file present + `embedder=None` → output contains `=== WORKSPACE ===`. `embedder_none_skips_retrieval_no_panic` — `embedder=None` → no panic, no `=== RECENT CONTEXT` separator in output. `three_hits_split_into_recent_and_longterm_blocks` — inject 3 hits via a `FakeEmbedder` + hand-INSERTed rows where 2 are within 7 days and 1 is 30 days old → output contains both `SEP_RECENT` and `SEP_LONGTERM`. `disputed_pair_renders_conflicts_block` — hand-INSERT a disputed pair → output contains `=== ACTIVE CONFLICTS (review needed) ===`. `no_disputed_rows_omits_conflicts_block` — no disputed rows → `"ACTIVE CONFLICTS"` not in output. `separator_strings_match_exactly` — output separator is verified character-by-character against `"=== RECENT CONTEXT (Last 7 Days) ==="`. `e2e_full_4_block_output` (REQ-TEST-003): use `MemoryStore::new_for_test()` + `tempfile::TempDir`; insert 5 rows (3 chat_message with known `created_at` timestamps, 1 `user_preference` with `importance=10` and `tier=1`, 1 `status='disputed'` with `disputed_against`); call `regenerate_auto_section`; call `build_memory_prompt` with `FakeEmbedder`; assert all four separators appear in order; assert `user_preference` content is in the WORKSPACE block (not RECENT/LONGTERM); assert the disputed pair is in CONFLICTS. All twelve fail before `build_memory_prompt` exists.
**Acceptance**:
- All twelve tests fail with compile errors before impl.
- `e2e_full_4_block_output` uses fixed `created_at` timestamps (e.g., recent = `now - 86400`, old = `now - 30 * 86400`) for deterministic date assertions.
- No `tokio::time::sleep` in any test.

### T23-impl — `build_memory_prompt` public function

**Trace**: REQ-PROMPT-001, REQ-PROMPT-002, REQ-PROMPT-003, REQ-PROMPT-004, D2, D4, D8, R-NEW-3
**File(s)**: `src-tauri/src/memory/prompt_builder.rs`
**Description**: Implement `pub(crate) async fn build_memory_prompt(store: &MemoryStore, workspace: &WorkspaceManager, embedder: Option<Arc<dyn Embedder>>, agent_id: &str, project_id: Option<&str>, user_query: &str) -> Result<String, MemoryError>`. Body follows design §2.4 annotated code block exactly: (1) read workspace via `workspace.read(...).await.unwrap_or_else(|e| { log::warn!(...); String::new() })`; (2) compute hits via embedder match — `None` → `log::warn!("embedder not ready; retrieval skipped this turn (agent_id={agent_id})")` + empty vec, `Some(emb)` → embed + `search_all_memories(..., SearchScope::all_default(), K_FINAL).await` (each failure branch logs warn and returns empty hits); (3) compute `now_secs` via `std::time::SystemTime::now().duration_since(UNIX_EPOCH)`; (4) `bucket_by_recency(hits, now_secs)` → `(recent, longterm)`; (5) `fetch_active_conflicts(store, agent_id).await.unwrap_or_else(|e| { log::warn!(...); Vec::new() })`; (6) assemble output string per design §2.4 with conditional block inclusion (empty string → block omitted). Return `Ok(out)`.
**Acceptance**:
- All twelve T23 tests pass.
- Blocks are assembled in order: WORKSPACE → RECENT → LONG-TERM → CONFLICTS.
- CONFLICTS block is omitted when `conflicts.is_empty()` (D8).
- Each block separator is emitted on its own line followed by `\n`.
- `log::warn!` is called when `embedder` is `None` (R-NEW-3).
- All prior tests still pass.

---

### T24-test — `stream_chat` integration: pre-loop memory pre-compute + line 558 swap + 3 integration tests

**Trace**: REQ-INT-001, D2, R-NEW-3, R-NEW-4
**File**: `src-tauri/src/commands/chat.rs`
**Test name(s)**: `commands::chat::tests::stream_chat_with_memories_prepends_blocks_to_system_prompt`, `commands::chat::tests::stream_chat_with_no_memories_uses_unmodified_system_prompt`, `commands::chat::tests::stream_chat_retrieval_error_does_not_fail_chat`, `commands::chat::tests::stream_chat_with_embedder_not_ready_does_not_panic`
**Description**: Write four integration tests in `commands/chat.rs::tests`. `stream_chat_with_memories_prepends_blocks_to_system_prompt`: seed `MemoryStore::new_for_test()` in a test `AppState` with pre-INSERTed rows + a workspace file; drive `stream_chat` via the existing test provider that captures `ChatRequest`; assert the captured `system_prompt` contains at least one of the four memory block separators (WORKSPACE or RECENT). `stream_chat_with_no_memories_uses_unmodified_system_prompt`: empty store + no workspace file; assert captured `system_prompt == Some(agent_system_prompt)` byte-equal (no separator strings). `stream_chat_retrieval_error_does_not_fail_chat`: force a retrieval error by dropping the DB connection before `stream_chat`; assert chat returns `Ok(...)` (no error propagated). `stream_chat_with_embedder_not_ready_does_not_panic`: use a test `AppState` with an empty embedder `OnceCell` (need a test helper or accessor to reset/replace the embedder); assert `stream_chat` completes without panic. All four fail before the `stream_chat` modification.
**Acceptance**:
- All four tests fail before the impl task modifies `stream_chat`.
- Tests reuse the existing `FakeEmbedder` and test provider infrastructure from Phase 3.
- `stream_chat_with_no_memories_uses_unmodified_system_prompt` asserts byte equality against the pre-Phase-4 behavior.

### T24-impl — `stream_chat` modification: pre-loop memory pre-compute + line 558 swap

**Trace**: REQ-INT-001, D2, D4, R-NEW-4
**File(s)**: `src-tauri/src/commands/chat.rs`
**Description**: Modify `commands/chat.rs::stream_chat` with two targeted changes. CHANGE 1 — insert the pre-loop block (AFTER the agent context capture and BEFORE the `for iteration in 0..MAX_TOOL_ITERATIONS` loop, i.e., between lines 532 and 538 per design §6.1): derive `project_id_opt` from `agent_project_path`; construct `workspace_mgr = crate::memory::workspace::WorkspaceManager::new(state.base_dir.join("workspaces"))`; call `workspace_mgr.regenerate_auto_section(...).await` with `log::warn!` on error; call `crate::memory::prompt_builder::build_memory_prompt(..., &message).await.unwrap_or_default()` for `memory_prefix`; compute `combined_system_prompt = if memory_prefix.is_empty() { agent_system_prompt.clone() } else { format!("{}\n\n{}", memory_prefix, agent_system_prompt) }`; add the `// TODO Phase 5+: re-run retrieval per iteration when AccessTracker emits synchronous tier-change events mid-loop (D2 forward-marker)` comment. CHANGE 2 — inside the loop body at the `ChatRequest` construction (the single line that sets `system_prompt`): swap `Some(agent_system_prompt.clone())` to `Some(combined_system_prompt.clone())`. That is the ONLY change inside the loop body.
**Acceptance**:
- All four T24 tests pass.
- All four pre-existing `chat.rs` tests (from Phase 3) still pass (they seed no memories → `combined_system_prompt == agent_system_prompt`).
- The TODO comment is present at the call site.
- `build_memory_prompt` is called exactly ONCE per `stream_chat` invocation (before the loop).
- `compose_system_prompt` in `models/prompt.rs` is NOT modified (all 10 of its tests remain green).
- `cargo check` is clean.

---

### Verify — CHECKPOINT 5 (FINAL)

Run: `cargo test --lib --manifest-path src-tauri/Cargo.toml`
Expected: ≥ 345 tests pass (315 baseline + ≥30 new Phase 4 tests). Zero failures. Zero regressions on the 315 baseline.

If the count is < 345, identify which Phase 4 tests are missing and trace back to the task that should have introduced them. Fix before marking Phase 4 complete.

---

## Out of scope (do NOT implement in Phase 4)

| Item | Phase |
|------|-------|
| `AccessTracker` — `last_accessed_at` updates on retrieval | Phase 5 |
| `dirty: AtomicBool` flag on `AppState` for lazy workspace invalidation | Phase 5 |
| Tier promotion (`compute_tier`) and `access_count` update | Phase 5 |
| `disputed_against_ids() -> Option<Vec<i64>>` typed accessor | Phase 6 |
| Contradiction detector and sycophancy resolver | Phase 6 |
| `status='disputed'` row insertion (Phase 6 produces these) | Phase 6 |
| Any modification to `models/prompt.rs::compose_system_prompt` | Out of scope permanently |
| Criterion bench harness or `benches/` directory | Out of scope (D1) |
| FTS5 fallback for cold-start | Deferred (D1 / R-NEW-3) |
| New Tauri commands | None in Phase 4 |
| Angular / frontend changes | None in Phase 4 |
| Any new Cargo dependencies | Forbidden (proposal §5) |
