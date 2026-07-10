# Phase 4 — rag-retrieval — Spec

> **Status**: normative contract. Derived from `docs/sdd/rag-retrieval/proposal.md` (locked decisions D1–D9) and corroborated by `docs/sdd/rag-retrieval/explore.md` and `docs/6-improvements-RAG.md` (score formula §463–525, search orchestrator §837–912, prompt injection §1047–1077, workspace §1079–1166). Every requirement below is normative. RFC 2119 keywords (MUST / SHOULD / MAY) apply. Each requirement maps to ≥1 GIVEN/WHEN/THEN scenario and ≥1 failing test (red) that the apply phase makes green.
>
> **Baseline**: Phase 1 + Phase 2 + Phase 3 = 315 passing tests. Phase 4 MUST NOT regress this count.
>
> **Trace key**: each REQ cites the locked decision(s) it implements (D1–D9) or the risk it mitigates (R-NEW-N).

---

## Scope summary

Phase 4 wires the consumer side of the RAG pipeline. Embeddings persisted by Phase 3 finally reach the LLM via four new modules: `memory/score.rs` (four SQLite UDFs), `memory/search.rs` (KNN orchestrator + ranked hits), `memory/workspace.rs` (`WorkspaceManager` with marker-aware AUTO/USER section management), and `memory/prompt_builder.rs` (four-block system prompt assembler). The chat hot path (`commands/chat.rs::stream_chat`) is modified to pre-compute a memory prefix once per invocation and prepend it to the agent system prompt. No new Tauri commands; no schema migrations; zero new Cargo dependencies.

---

## Requirements

### REQ-SCORE-001 — `exp_decay` UDF — registration and formula

**Trace**: D6, D7

**The system MUST** register a scalar UDF named `exp_decay` with arity 2 and flag `SQLITE_DETERMINISTIC | SQLITE_UTF8` on every `MemoryStore` connection. The UDF MUST implement:

```
exp_decay(age_secs INTEGER, lambda REAL) -> REAL
    age_days = age_secs / 86_400.0
    result   = e^(-lambda × age_days)
```

The UDF is pure: `age_secs` is computed OUTSIDE the UDF (as `strftime('%s','now') - m.last_accessed_at` in the SQL query) and passed in as a bound integer. The `SQLITE_DETERMINISTIC` flag is therefore valid and allows the query planner to cache per-query.

#### Scenarios

- GIVEN a registered store, WHEN `SELECT exp_decay(0, 0.023)` is evaluated, THEN the result equals `1.0` (age=0 → no decay).
- GIVEN `SELECT exp_decay(86400, 0.023)`, THEN the result is approximately `0.977` (1 day at chat_message lambda).
- GIVEN `SELECT exp_decay(604800, 0.099)`, THEN the result is approximately `0.5` (7 days at tool_error lambda — the half-life for that type).
- GIVEN negative `age_secs` (clock skew), WHEN evaluated, THEN the result is > 1.0 (natural exponential behavior — no clamp required for the decay term itself).

#### Red tests

- `memory::score::tests::exp_decay_zero_age_returns_one`
- `memory::score::tests::exp_decay_one_day_chat_lambda`
- `memory::score::tests::exp_decay_seven_days_tool_error_lambda_approx_half`

---

### REQ-SCORE-002 — `lambda_for_type` UDF — registration and values

**Trace**: D6

**The system MUST** register a scalar UDF named `lambda_for_type` with arity 1 and `SQLITE_DETERMINISTIC | SQLITE_UTF8`. The UDF MUST implement the exact mapping from `docs/6-improvements-RAG.md` §486–499:

| `memory_type` | lambda |
|---|---|
| `"user_preference"` | `0.0019` |
| `"architectural_decision"` | `0.0039` |
| `"session_summary"` | `0.0077` |
| `"semantic_fact"` | `0.0039` |
| `"chat_message"` | `0.023` |
| `"tool_error"` | `0.099` |
| any other value | `0.023` (default arm) |

The default arm MUST cover unknown future `memory_type` values without error or panic.

#### Scenarios

- GIVEN `SELECT lambda_for_type('user_preference')`, THEN result is `0.0019`.
- GIVEN `SELECT lambda_for_type('chat_message')`, THEN result is `0.023`.
- GIVEN `SELECT lambda_for_type('tool_error')`, THEN result is `0.099`.
- GIVEN `SELECT lambda_for_type('unknown_future_type')`, THEN result is `0.023` (default).

#### Red tests

- `memory::score::tests::lambda_user_preference`
- `memory::score::tests::lambda_chat_message`
- `memory::score::tests::lambda_tool_error`
- `memory::score::tests::lambda_unknown_type_returns_default`

---

### REQ-SCORE-003 — `importance_factor` UDF — registration, formula, and clamping

**Trace**: D6, R-NEW-8

**The system MUST** register a scalar UDF named `importance_factor` with arity 1 and `SQLITE_DETERMINISTIC | SQLITE_UTF8`. The UDF MUST implement:

```
importance_factor(importance REAL) -> REAL
    imp    = importance.clamp(1.0, 10.0)
    result = 0.5 + (imp / 20.0)
```

The clamp MUST be applied BEFORE the formula. This ensures the output range is always `[0.55, 1.0]` regardless of out-of-range DB values (R-NEW-8: no CHECK constraint exists on the `importance` column, so clamping in the UDF is the only safeguard).

#### Scenarios

- GIVEN `SELECT importance_factor(1.0)`, THEN result equals `0.55` (minimum: `0.5 + 1/20`).
- GIVEN `SELECT importance_factor(10.0)`, THEN result equals `1.0` (maximum: `0.5 + 10/20`).
- GIVEN `SELECT importance_factor(5.0)`, THEN result equals `0.75`.
- GIVEN `SELECT importance_factor(0.0)` (below clamp floor), THEN result equals `0.55` (clamped to 1.0 before formula).
- GIVEN `SELECT importance_factor(15.0)` (above clamp ceiling), THEN result equals `1.0` (clamped to 10.0 before formula).

#### Red tests

- `memory::score::tests::importance_factor_min_value`
- `memory::score::tests::importance_factor_max_value`
- `memory::score::tests::importance_factor_mid_value`
- `memory::score::tests::importance_factor_clamp_below_floor`
- `memory::score::tests::importance_factor_clamp_above_ceiling`

---

### REQ-SCORE-004 — `dispute_factor` UDF — full implementation

**Trace**: D7

**The system MUST** register a scalar UDF named `dispute_factor` with arity 1 and `SQLITE_DETERMINISTIC | SQLITE_UTF8`. The UDF MUST implement the complete match per `docs/6-improvements-RAG.md` §517–524:

```
dispute_factor(status TEXT) -> REAL
    "active"    => 1.0
    "disputed"  => 0.6   // degraded but visible — Phase 6 adds rows triggering this arm
    _           => 0.0   // "superseded" | "invalid" | anything else → out of ranking
```

This is the FINAL implementation. Phase 6 MUST NOT modify `score.rs`. Phase 6 only inserts rows with `status='disputed'` / `'superseded'`; the UDF already handles those values correctly.

#### Scenarios

- GIVEN `SELECT dispute_factor('active')`, THEN result is `1.0`.
- GIVEN `SELECT dispute_factor('disputed')`, THEN result is `0.6`.
- GIVEN `SELECT dispute_factor('superseded')`, THEN result is `0.0`.
- GIVEN `SELECT dispute_factor('invalid')`, THEN result is `0.0`.
- GIVEN `SELECT dispute_factor('bogus_future_status')`, THEN result is `0.0` (wildcard arm).

#### Red tests

- `memory::score::tests::dispute_factor_active`
- `memory::score::tests::dispute_factor_disputed`
- `memory::score::tests::dispute_factor_superseded`
- `memory::score::tests::dispute_factor_invalid`
- `memory::score::tests::dispute_factor_unknown_status`

---

### REQ-STATE-001 — UDF registration in `MemoryStore` constructors

**Trace**: D6

**The system MUST** provide, in `src-tauri/src/memory/score.rs`, a function:

```rust
pub(crate) async fn register_score_udfs(
    conn: &tokio_rusqlite::Connection,
) -> Result<(), MemoryError>
```

This function MUST register all four UDFs (`exp_decay`, `lambda_for_type`, `importance_factor`, `dispute_factor`) in a single `conn.call(|c| { ... Ok(()) }).await` closure.

`MemoryStore::open()` MUST call `register_score_udfs(&conn).await?` immediately after `run_migrations(&conn).await?`. `MemoryStore::new_for_test()` MUST do the same. The registration MUST happen on EVERY new connection — UDFs are connection-scoped, not DB-file-scoped, and do NOT persist across `Connection::open()` calls.

All 15+ existing `MemoryStore` construction-site tests MUST still pass after this change (the UDF registration is additive: it introduces no new SQL names that conflict with existing names).

#### Scenarios

- GIVEN a freshly opened `MemoryStore`, WHEN `SELECT exp_decay(0, 0.023)` is issued on its connection, THEN it returns `1.0` without "no such function" error.
- GIVEN `MemoryStore::new_for_test()`, WHEN any of the four UDFs is called in a test query, THEN no error occurs.
- GIVEN the existing 15+ `db.rs` / `worker.rs` tests, WHEN they run after UDF registration is added to constructors, THEN all still pass (zero regressions).

#### Red tests

- `memory::score::tests::register_score_udfs_makes_exp_decay_callable`
- `memory::score::tests::register_score_udfs_makes_all_four_callable`

---

### REQ-SEARCH-001 — `SearchScope` struct and named constructors

**Trace**: D5 (per-table KNN scope)

**The system MUST** define, in `src-tauri/src/memory/search.rs`:

```rust
pub(crate) struct SearchScope {
    pub memory:     Option<usize>,   // k_pool for memory table; None = skip
    pub episodic:   Option<usize>,   // k_pool for episodic_trajectory table; None = skip
    pub procedural: Option<usize>,   // k_pool for procedural_skill table; None = skip
}
```

With three named constructors:

| Constructor | Returns |
|---|---|
| `SearchScope::all_default()` | `(Some(30), Some(5), Some(5))` |
| `SearchScope::procedural_only()` | `(None, None, Some(10))` |
| `SearchScope::factual_pivots()` | `(Some(5), None, Some(5))` — Phase 6 contradiction detector caller |

`SearchScope` MUST derive `Debug + Clone + Copy`.

#### Scenarios

- GIVEN `SearchScope::all_default()`, THEN `memory == Some(30)`, `episodic == Some(5)`, `procedural == Some(5)`.
- GIVEN `SearchScope::procedural_only()`, THEN `memory == None` and `episodic == None`.
- GIVEN `SearchScope::factual_pivots()`, THEN `episodic == None`, `memory == Some(5)`, `procedural == Some(5)`.

#### Red tests

- `memory::search::tests::scope_all_default_values`
- `memory::search::tests::scope_procedural_only_values`
- `memory::search::tests::scope_factual_pivots_values`

---

### REQ-SEARCH-002 — `RankedHit` struct

**Trace**: D5, D3

**The system MUST** define, in `src-tauri/src/memory/search.rs`:

```rust
pub(crate) struct RankedHit {
    pub id:          i64,
    pub kind:        TableKind,    // re-used enum from memory/record.rs
    pub content:     String,
    pub memory_type: String,
    pub tier:        i64,
    pub status:      String,
    pub score:       f64,          // combined score: (1-dist) × MAX(floor,decay) × imp × dispute
    pub distance:    f64,          // raw KNN distance — consumed by Phase 6 contradiction detector
    pub created_at:  i64,          // epoch seconds — used by prompt_builder for RECENT vs LONG-TERM split
}
```

`RankedHit` MUST derive `Debug + Clone`. `distance` MUST be exposed (not hidden) because Phase 6's `detect_contradiction` reads `hit.distance < 0.20` directly from this field.

#### Scenarios

- GIVEN a `RankedHit` value, WHEN moved into a `tokio::spawn` closure, THEN it compiles without `!Send` errors.
- GIVEN two `RankedHit` values with different `score` fields, WHEN sorted descending by `score`, THEN the higher-score hit comes first.

#### Red tests

- `memory::search::tests::ranked_hit_is_send`
- `memory::search::tests::ranked_hit_sort_descending_by_score`

---

### REQ-SEARCH-003 — `search_all_memories` — parallel KNN and merge

**Trace**: D5, D1, R-NEW-3

**The system MUST** provide:

```rust
pub(crate) async fn search_all_memories(
    store: &MemoryStore,
    embedding: &[f32],
    agent_id: &str,
    scope: SearchScope,
    k_final: usize,
) -> Result<Vec<RankedHit>, MemoryError>
```

The function MUST launch up to three KNN queries in parallel using `tokio::try_join!`. Each table query runs only if its corresponding `scope` field is `Some(k)`. All three results are merged into a single `Vec<RankedHit>`, sorted by `score DESC`, and truncated to `k_final`.

After the `tokio::try_join!` block (before returning), the function MUST record elapsed time via:

```rust
log::debug!("retrieval took {}ms", elapsed.as_millis());
```

This satisfies D1 (no bench harness; latency is observable via logs).

#### Scenarios

- GIVEN 5 hand-INSERTed `memory` rows and `scope = SearchScope::all_default()`, WHEN `search_all_memories` returns, THEN the results are sorted by `score` descending.
- GIVEN `scope.episodic = None`, WHEN `search_all_memories` runs, THEN the episodic table is NOT queried (confirmed by zero `episodic_trajectory` rows in results).
- GIVEN `k_final = 3` and 10 candidate hits, WHEN the function returns, THEN the result vec has exactly 3 elements (the top-3 by score).
- GIVEN an empty database, WHEN `search_all_memories` is called, THEN it returns `Ok(vec![])` without error.

#### Red tests

- `memory::search::tests::search_returns_results_sorted_by_score_desc`
- `memory::search::tests::search_respects_scope_none_skips_table`
- `memory::search::tests::search_truncates_to_k_final`
- `memory::search::tests::search_empty_db_returns_empty_vec`

---

### REQ-SEARCH-004 — KNN tier filter and status filter

**Trace**: D5

**All three KNN queries MUST** include the following predicates:

```sql
WHERE m.tier > 1          -- Tier 1 excluded (lives in WORKSPACE block, D5)
  AND m.status IN ('active', 'disputed')   -- 'disputed' enters ranking at 0.6; 'superseded'/'invalid' excluded
  AND m.deleted_at IS NULL
  AND m.agent_id = ?
```

Tier 1 rows MUST NOT appear in any `RankedHit` result set, regardless of their score. Rows with `status = 'superseded'` or `status = 'invalid'` MUST NOT appear in results (their `dispute_factor` would be 0.0 anyway, but the filter prevents them consuming KNN slots).

#### Scenarios

- GIVEN one `memory` row with `tier = 1` and three with `tier = 3`, WHEN `search_all_memories` is called, THEN only the tier-3 rows appear in results (the tier-1 row is absent).
- GIVEN a `memory` row with `status = 'superseded'`, WHEN `search_all_memories` is called, THEN it does NOT appear in the results.
- GIVEN a `memory` row with `status = 'disputed'`, WHEN `search_all_memories` is called, THEN it DOES appear in results (with `dispute_factor = 0.6` reducing its score).

#### Red tests

- `memory::search::tests::tier1_rows_excluded_from_results`
- `memory::search::tests::superseded_rows_excluded_from_results`
- `memory::search::tests::disputed_rows_included_with_degraded_score`

---

### REQ-SEARCH-005 — Combined score formula

**Trace**: D5, D7

**The combined score MUST** be computed in SQL (not Rust) and MUST exactly match the formula from `docs/6-improvements-RAG.md` §463–524:

```sql
(1.0 - knn.distance)
    * MAX(
        CASE m.tier WHEN 1 THEN 0.5 WHEN 2 THEN 0.2 ELSE 0.05 END,
        exp_decay(
            strftime('%s','now') - m.last_accessed_at,
            lambda_for_type(m.memory_type)
        )
      )
    * importance_factor(m.importance)
    * dispute_factor(m.status)
AS score
```

The `tier_floor` values are inline in a `CASE` expression (not a UDF) so they are visible and debuggable directly in the query. `strftime('%s','now')` is computed by SQLite as the non-deterministic component; the UDF receives the concrete integer delta.

The `knn_episodic` and `knn_procedural` helpers MUST hardcode their lambda values directly in the `exp_decay(...)` call (not via `lambda_for_type`) because the table determines the type: episodic uses `0.0077`, procedural uses `0.0039`.

#### Scenarios

- GIVEN a memory row with `distance=0.1`, `tier=2`, `last_accessed_at=(now-1day)`, `memory_type='chat_message'`, `importance=5.0`, `status='active'`, WHEN the score is computed, THEN `score ≈ (0.9) × MAX(0.2, exp_decay(86400, 0.023)) × 0.75 × 1.0 ≈ 0.9 × MAX(0.2, 0.977) × 0.75 × 1.0 ≈ 0.66`.
- GIVEN a row with `status='disputed'` and all other factors at maximum, THEN `score ≤ 0.6 × (score if status='active')`.

#### Red tests

- `memory::search::tests::score_formula_integration_spot_check`
- `memory::search::tests::disputed_status_reduces_score_by_factor_0_6`

---

### REQ-SEARCH-006 — Graceful degradation when embedder is not ready

**Trace**: R-NEW-3

**When `state.embedder.get()` returns `None`** (embedder OnceCell not yet filled at boot time), the caller-side code in `build_memory_prompt` (not inside `search_all_memories` itself) MUST short-circuit: `search_all_memories` is NOT called, and the RECENT/LONG-TERM/CONFLICTS blocks are omitted. A `log::warn!` MUST be emitted to signal that retrieval was skipped.

`search_all_memories` itself takes an `embedding: &[f32]` slice (already computed) — it does not access `AppState` directly. The `None`-embedder guard lives at the `build_memory_prompt` call site.

#### Scenarios

- GIVEN `build_memory_prompt` called with `embedder: None`, WHEN it returns, THEN the result does NOT contain `=== RECENT CONTEXT` or `=== LONG-TERM KNOWLEDGE` separators.
- GIVEN the same call, THEN a `warn!` log entry is emitted (observable in test via a logging spy or by checking function output shape).
- GIVEN the embedder is `None`, THEN no panic occurs and `build_memory_prompt` returns `Ok(...)`.

#### Red tests

- `memory::prompt_builder::tests::embedder_none_skips_retrieval_no_panic`
- `memory::prompt_builder::tests::embedder_none_emits_workspace_only_if_present`

---

### REQ-WORKSPACE-001 — `WorkspaceManager` struct and workspace path resolution

**Trace**: D4, D9

**The system MUST** define, in `src-tauri/src/memory/workspace.rs`:

```rust
pub(crate) struct WorkspaceManager {
    workspaces_root: PathBuf,   // no interior mutability — stateless façade
}

impl WorkspaceManager {
    /// Caller pre-joins the `workspaces` segment to `state.base_dir` before construction.
    /// Phase 4 call site: `WorkspaceManager::new(state.base_dir.join("workspaces"))`.
    pub(crate) fn new(workspaces_root: PathBuf) -> Self;
    pub(crate) fn workspace_path(&self, agent_id: &str, project_id: Option<&str>) -> PathBuf;
}
```

`workspace_path` MUST resolve to:
- `workspaces_root/{agent_id}/{project_id}/workspace.md` when `project_id` is `Some(id)`
- `workspaces_root/{agent_id}/default/workspace.md` when `project_id` is `None`

The literal `"default"` MUST be defined as:

```rust
pub(crate) const DEFAULT_PROJECT_SEGMENT: &str = "default";
```

in `workspace.rs`. This constant MUST be re-exported if Phase 5/6 need it; Phase 4 has a single consumer (the `WorkspaceManager` itself), so no re-export from `memory/mod.rs` is required today.

`WorkspaceManager` MUST be `Clone + Send + Sync` (it holds only a `PathBuf`).

#### Scenarios

- GIVEN `WorkspaceManager::new(base.join("workspaces"))` and `workspace_path("agent-1", Some("proj-abc"))`, THEN result is `base/workspaces/agent-1/proj-abc/workspace.md`.
- GIVEN the same manager and `workspace_path("agent-1", None)`, THEN result is `base/workspaces/agent-1/default/workspace.md`.
- GIVEN a `WorkspaceManager` instance, WHEN it is cloned and moved into a `tokio::spawn` closure, THEN it compiles without `!Send` errors.

#### Red tests

- `memory::workspace::tests::path_with_project_id`
- `memory::workspace::tests::path_with_no_project_uses_default_segment`
- `memory::workspace::tests::workspace_manager_is_send_sync`

---

### REQ-WORKSPACE-002 — AUTO/USER marker constants

**Trace**: D4

**The system MUST** define the following constants in `src-tauri/src/memory/workspace.rs`:

```rust
pub(crate) const AUTO_START: &str = "<!-- AUTO-START -->";
pub(crate) const AUTO_END:   &str = "<!-- AUTO-END -->";
pub(crate) const USER_START: &str = "<!-- USER-START -->";
pub(crate) const USER_END:   &str = "<!-- USER-END -->";
pub(crate) const AUTO_TOKEN_CAP: usize = 1500;
```

These are the ONLY markers recognized by the workspace parser. Any existing file content outside these markers MUST be treated as preamble/header and preserved byte-for-byte. The token cap of 1500 is a hard ceiling on the AUTO section content.

#### Scenarios

- GIVEN a file that contains `<!-- AUTO-START -->` and `<!-- AUTO-END -->`, WHEN the parser locates the AUTO section boundaries, THEN it identifies the exact byte range between (and including) the markers.
- GIVEN a file with none of the markers, WHEN `regenerate_auto_section` is called, THEN the file is created with both AUTO and USER sections using these exact marker strings.

#### Red tests

- `memory::workspace::tests::markers_locate_auto_section_in_existing_file`
- `memory::workspace::tests::missing_markers_creates_file_with_both_sections`

---

### REQ-WORKSPACE-003 — `regenerate_auto_section` — AUTO section rewrite

**Trace**: D4

**The system MUST** provide:

```rust
pub(crate) async fn regenerate_auto_section(
    &self,
    agent_id: &str,
    project_id: Option<&str>,
    store: &MemoryStore,
) -> Result<(), MemoryError>
```

The function MUST:

1. Query the DB for the AUTO section content (Tier 1 memories for the `(agent_id, project_id)` pair, ordered by the recovery priority from `docs/6-improvements-RAG.md` §1124: active disputes touching Tier 1 → current session focus → top-5 procedural skills by `access_count` → `architectural_decision` with `importance >= 8` → `user_preference` with `importance >= 8`).
2. Truncate the assembled content to `AUTO_TOKEN_CAP = 1500` tokens if needed (least-important categories removed first in reverse priority order).
3. Write the result into the AUTO section (`<!-- AUTO-START -->` … `<!-- AUTO-END -->`) of the workspace file. If the file does not exist, create it with both AUTO and USER sections (USER section is initially empty between its markers).
4. PRESERVE the USER section (`<!-- USER-START -->` … `<!-- USER-END -->`) byte-for-byte. No diff, no merge, no content modification of the USER section.

If the parent directory does not exist, `regenerate_auto_section` MUST create it (via `tokio::fs::create_dir_all`). If the file write fails (e.g., permission denied), the function MUST return `Err(MemoryError::WorkspaceIo(...))`.

#### Scenarios

- GIVEN a file with USER section content `"  my custom notes\n"`, WHEN `regenerate_auto_section` runs, THEN the USER section content is byte-for-byte `"  my custom notes\n"` — no trailing whitespace stripped, no Unicode normalized.
- GIVEN a non-existent workspace path, WHEN `regenerate_auto_section` runs, THEN the directory tree is created and the file is written with both sections.
- GIVEN zero Tier 1 rows in the DB, WHEN `regenerate_auto_section` runs, THEN the AUTO section contains the sub-headers (`## User Preferences (always active)`, etc.) with no memory bullet lines under them.
- GIVEN 200 `user_preference` rows with `importance = 10`, WHEN `regenerate_auto_section` runs, THEN the AUTO section content does NOT exceed `AUTO_TOKEN_CAP = 1500` tokens.
- GIVEN a file write that fails with `PermissionDenied`, WHEN `regenerate_auto_section` is called, THEN it returns `Err(MemoryError::WorkspaceIo(...))`.

#### Red tests

- `memory::workspace::tests::user_section_preserved_byte_for_byte`
- `memory::workspace::tests::creates_directory_and_file_on_first_run`
- `memory::workspace::tests::zero_tier1_rows_produces_empty_auto_section_with_headers`
- `memory::workspace::tests::auto_section_respects_1500_token_cap`
- `memory::workspace::tests::write_failure_returns_workspace_io_error`

---

### REQ-WORKSPACE-004 — `WorkspaceManager::read` — defensive read

**Trace**: D4, R-NEW-3

**The system MUST** provide:

```rust
pub(crate) async fn read(
    &self,
    agent_id: &str,
    project_id: Option<&str>,
) -> Result<String, MemoryError>
```

If the workspace file does NOT exist (`tokio::fs::read_to_string` returns `ErrorKind::NotFound`), the function MUST return `Ok(String::new())` — the caller receives an empty string without error. This handles the first-run case where no workspace file has been generated yet (R-NEW-3 pattern: graceful empty result for uninitialized state).

For any other I/O error, the function MUST return `Err(MemoryError::WorkspaceIo(...))`.

#### Scenarios

- GIVEN a path that does not exist on disk, WHEN `WorkspaceManager::read` is called, THEN it returns `Ok("")`.
- GIVEN a workspace file with content `"hello"`, WHEN `read` is called, THEN it returns `Ok("hello")`.

#### Red tests

- `memory::workspace::tests::read_missing_file_returns_ok_empty`
- `memory::workspace::tests::read_existing_file_returns_content`

---

### REQ-PROMPT-001 — `build_memory_prompt` signature

**Trace**: D2, D4, D8, R-NEW-3

**The system MUST** provide, in `src-tauri/src/memory/prompt_builder.rs`:

```rust
pub(crate) async fn build_memory_prompt(
    store: &MemoryStore,
    workspace: &WorkspaceManager,
    embedder: Option<Arc<dyn Embedder>>,   // None → no retrieval; workspace + conflicts only
    agent_id: &str,
    project_id: Option<&str>,
    user_query: &str,                       // text used to embed the query vector
) -> Result<String, MemoryError>
```

If both the workspace read and the retrieval produce empty/missing results AND there are no disputed rows, the function MUST return `Ok(String::new())`. The caller (`stream_chat`) checks for empty and conditionally omits the concat.

#### Scenarios

- GIVEN an empty in-memory store, a missing workspace file, and `embedder = None`, WHEN `build_memory_prompt` is called, THEN it returns `Ok("")`.
- GIVEN an in-memory store with `embedder = None` and a workspace file present, THEN it returns a non-empty string (at minimum the WORKSPACE block).

#### Red tests

- `memory::prompt_builder::tests::all_empty_returns_empty_string`
- `memory::prompt_builder::tests::workspace_only_returns_workspace_block`

---

### REQ-PROMPT-002 — Four-block output format and separators

**Trace**: D8

**The prompt output MUST** follow this exact block structure and separator strings (from `docs/6-improvements-RAG.md` §1056–1073):

```
=== WORKSPACE ===
{workspace_content}

=== RECENT CONTEXT (Last 7 Days) ===
{recent_hits}

=== LONG-TERM KNOWLEDGE ===
{longterm_hits}

=== ACTIVE CONFLICTS (review needed) ===
{conflict_pairs}
```

The separator format MUST be exactly `=== {name} ===` (no leading/trailing spaces on the `===` delimiters, single space between `===` and the name). Blocks MUST appear in the order: WORKSPACE → RECENT → LONG-TERM → CONFLICTS.

The CONFLICTS block MUST be omitted entirely (including its separator line) when there are zero disputed rows for the `agent_id` (D8). The WORKSPACE block separator and content MUST be omitted when the workspace file is empty/missing. The RECENT and LONG-TERM separators MUST be omitted when they have no content.

#### Scenarios

- GIVEN a populated workspace and hits with 2 within 7 days and 1 older, THEN the output contains the WORKSPACE, RECENT, and LONG-TERM separators in order, with no CONFLICTS separator.
- GIVEN a disputed memory pair (hand-INSERTed `status='disputed'` row with `disputed_against=JSON([old_id])`), THEN the output includes the `=== ACTIVE CONFLICTS (review needed) ===` separator.
- GIVEN zero disputed rows, THEN the string `"ACTIVE CONFLICTS"` does NOT appear anywhere in the output.
- GIVEN the separator `=== RECENT CONTEXT (Last 7 Days) ===` expected, WHEN the output is checked character-by-character, THEN the separator matches exactly (no extra spaces, correct case).

#### Red tests

- `memory::prompt_builder::tests::three_hits_split_into_recent_and_longterm_blocks`
- `memory::prompt_builder::tests::disputed_pair_renders_conflicts_block`
- `memory::prompt_builder::tests::no_disputed_rows_omits_conflicts_block`
- `memory::prompt_builder::tests::separator_strings_match_exactly`

---

### REQ-PROMPT-003 — RECENT vs LONG-TERM split by 7-day window

**Trace**: D2 (score formula context)

**Hits from `search_all_memories` MUST** be partitioned into RECENT and LONG-TERM sections based on `RankedHit.created_at`:

- **RECENT**: `created_at >= (now_epoch_secs - 7 * 86_400)` — last 7 days.
- **LONG-TERM**: `created_at < (now_epoch_secs - 7 * 86_400)` — older than 7 days.

The window constant MUST be defined as:

```rust
const RECENT_WINDOW_SECS: i64 = 7 * 86_400;
```

Within each section, hits MUST appear ordered by `score` descending (they already come sorted from `search_all_memories`). Each hit line MUST follow the format:

```
[YYYY-MM-DD] SOURCE: {source} | TYPE: {memory_type}
CONTENT: {content}
```

Where `YYYY-MM-DD` is derived from `RankedHit.created_at` (epoch seconds → UTC date).

#### Scenarios

- GIVEN 3 hits where 2 have `created_at` within 7 days and 1 has `created_at` 30 days ago, WHEN the prompt is built, THEN RECENT has 2 entries and LONG-TERM has 1 entry.
- GIVEN all hits older than 7 days, THEN the RECENT separator is omitted and LONG-TERM contains all hits.
- GIVEN all hits within 7 days, THEN the LONG-TERM separator is omitted and RECENT contains all hits.

#### Red tests

- `memory::prompt_builder::tests::recent_longterm_split_by_7day_window`
- `memory::prompt_builder::tests::all_old_hits_go_to_longterm_only`
- `memory::prompt_builder::tests::all_recent_hits_go_to_recent_only`

---

### REQ-PROMPT-004 — ACTIVE CONFLICTS block — query and conditional render

**Trace**: D3, D8

**The system MUST** run the following SQL query (from `docs/6-improvements-RAG.md` §654–663) to populate the CONFLICTS block:

```sql
SELECT m_old.id, m_old.created_at, m_old.content, m_old.importance, m_old.tier,
       m_new.id, m_new.created_at, m_new.content, m_new.importance
FROM memory m_new
JOIN memory m_old
  ON m_old.id IN (SELECT value FROM json_each(m_new.disputed_against))
WHERE m_new.status      = 'disputed'
  AND m_new.agent_id    = ?
  AND m_new.deleted_at IS NULL
  AND m_old.deleted_at IS NULL
```

`json_each(NULL)` returns zero rows in SQLite — no NULL guard is needed (R-NEW-7). The query MUST be run unconditionally on every `build_memory_prompt` call; only the OUTPUT is conditional (block omitted when query returns zero rows).

Each conflict pair MUST render as:

```
[{old_date}] OLD: "{old_content}" [memory:{old_id}, importance:{old_importance}, tier:{old_tier}]
[{new_date}] NEW: "{new_content}" [memory:{new_id}, importance:{new_importance}, status:disputed]
NOTE: estas memorias se contradicen y aún no resolviste cuál vale.
Si vas a tomar una decisión que dependa de esto, preguntale al usuario.
```

#### Scenarios

- GIVEN zero rows with `status='disputed'` for the agent, WHEN the prompt is built, THEN the CONFLICTS block and separator are absent from the output.
- GIVEN one hand-INSERTed row with `status='disputed'` and `disputed_against=JSON([old_id])`, WHEN the prompt is built, THEN the CONFLICTS block contains one OLD/NEW pair with the correct memory IDs.
- GIVEN a row with `disputed_against IS NULL`, WHEN the query runs via `json_each(NULL)`, THEN zero rows are returned and the block is omitted (no error, no panic).

#### Red tests

- `memory::prompt_builder::tests::conflicts_query_zero_disputed_rows`
- `memory::prompt_builder::tests::conflicts_query_one_disputed_pair_renders_correctly`
- `memory::prompt_builder::tests::json_each_null_returns_zero_rows_no_error`

---

### REQ-INT-001 — `stream_chat` pre-computes memory prefix once before the agentic loop

**Trace**: D2, R-NEW-4

**The system MUST** modify `src-tauri/src/commands/chat.rs::stream_chat` as follows. BEFORE the `for iteration in 0..MAX_TOOL_ITERATIONS` loop, add:

```rust
let memory_prefix = build_memory_prompt(
    &state.memory_store,
    &workspace_manager,
    state.embedder.get().cloned(),
    &agent_id,
    project_id.as_deref(),
    &user_message_content,
).await.unwrap_or_else(|e| {
    log::warn!("retrieval failed, proceeding without memory context: {e}");
    String::new()
});

let combined_system_prompt = if memory_prefix.is_empty() {
    agent_system_prompt.clone()
} else {
    format!("{}\n\n{}", memory_prefix, agent_system_prompt)
};
```

INSIDE the loop, `ChatRequest.system_prompt` MUST be set to `Some(combined_system_prompt.clone())`.

`compose_system_prompt` in `src-tauri/src/models/prompt.rs` MUST NOT be modified (R-NEW-4). Its 10 tests MUST remain green.

A `// TODO Phase X: re-run retrieval per iteration when AccessTracker emits tier-change events mid-loop` comment MUST be placed at the call site.

#### Scenarios

- GIVEN a `MemoryStore` with seeded memories and a functioning embedder, WHEN `stream_chat` is called, THEN the captured `ChatRequest.system_prompt` starts with `=== WORKSPACE ===` (or another memory block separator).
- GIVEN an empty store and no workspace file, WHEN `stream_chat` is called, THEN `ChatRequest.system_prompt` is byte-equal to `agent_system_prompt` (no separator strings added).
- GIVEN `build_memory_prompt` returning `Err(...)`, WHEN `stream_chat` runs, THEN it does NOT propagate the error — chat proceeds with the unmodified `agent_system_prompt`. A `warn!` log is emitted.
- GIVEN multiple iterations of the agentic loop, THEN `build_memory_prompt` is called exactly ONCE per `stream_chat` invocation (not once per iteration).

#### Red tests

- `commands::chat::tests::stream_chat_with_memories_prepends_blocks_to_system_prompt`
- `commands::chat::tests::stream_chat_with_no_memories_uses_unmodified_system_prompt`
- `commands::chat::tests::stream_chat_retrieval_error_does_not_fail_chat`
- `commands::chat::tests::stream_chat_with_embedder_not_ready_does_not_panic`

---

### REQ-INT-002 — `memory/mod.rs` module declarations

**Trace**: D6

**The system MUST** add the following module declarations to `src-tauri/src/memory/mod.rs`:

```rust
pub(crate) mod score;
pub(crate) mod search;
pub(crate) mod workspace;
pub(crate) mod prompt_builder;
```

No new re-exports from `memory/mod.rs` are required (all four modules are `pub(crate)` internal modules; only `commands/chat.rs` calls `build_memory_prompt` directly through the module path). Existing public re-exports MUST remain unchanged.

#### Scenarios

- GIVEN the final `memory/mod.rs`, WHEN `cargo check --lib` runs, THEN the four new `mod` declarations compile without `dead_code` or `unused_imports` warnings.

#### Red tests

- Compile-time only (verified by `cargo check --lib` pass).

---

### REQ-STATE-002 — Compile-time `Send + Sync` assertions for Phase 4 types

**Trace**: Phase 3 carry-forward pattern (REQ-STATE-003 in Phase 3 spec)

**The system MUST** add compile-time `Send + Sync` assertions in `src-tauri/src/memory/search.rs` (or a dedicated `_assert` block in that module) for all types that cross `tokio::try_join!` or `tokio::spawn` boundaries:

```rust
const _: fn() = || {
    fn _assert_send<T: Send>() {}
    fn _assert_sync<T: Sync>() {}
    _assert_send::<SearchScope>();
    _assert_sync::<SearchScope>();
    _assert_send::<RankedHit>();
    _assert_sync::<RankedHit>();
    _assert_send::<WorkspaceManager>();
    _assert_sync::<WorkspaceManager>();
};
```

These MUST be straight-line `const` block code, NOT inside `#[test]` functions (per Phase 3 REQ-STATE-003 carry-forward).

#### Scenarios

- GIVEN the source tree at the end of apply, WHEN `cargo check --lib` runs, THEN the const assertion block type-checks, proving the listed types are `Send + Sync`.

#### Red tests

- Compile-time only.

---

### REQ-TEST-001 — Score UDF integration via in-memory store

**Trace**: D6, D7

**The test suite MUST** include tests that exercise UDFs through actual SQL queries on an in-memory `MemoryStore` (not mocked closures). Specifically:

- All four UDFs callable via `SELECT` on a `MemoryStore::new_for_test()` connection.
- The full score expression `(1.0 - distance) * MAX(...) * importance_factor(...) * dispute_factor(...)` evaluated in a single query returning a numeric result within the expected range `[0.0, 1.0]`.

#### Scenarios

- GIVEN `MemoryStore::new_for_test()` and a `SELECT exp_decay(?, ?) * dispute_factor(?)` query, WHEN evaluated with valid inputs, THEN the result is in `[0.0, 1.0]`.

#### Red tests

- `memory::score::tests::full_score_expression_in_sql_integration`

---

### REQ-TEST-002 — `search_all_memories` integration with `FakeEmbedder`

**Trace**: D5, D1

**The test suite MUST** include an integration test in `memory::search::tests` that:

1. Constructs `MemoryStore::new_for_test()` and a `FakeEmbedder` (returning fixed 384-dim vector).
2. Hand-INSERTs 5 rows spanning at least 2 `memory_type` values and 2 `tier` values (including at least one `tier=1` row to verify the filter).
3. Calls `search_all_memories` with `SearchScope::all_default()` and `k_final = 3`.
4. Asserts exactly 3 hits returned, sorted by `score DESC`.
5. Asserts the `tier=1` row is NOT in the results.
6. Asserts scores are in the range `(0.0, 1.0]`.

NO `tokio::time::sleep` in this test. Channel/future semantics only.

#### Scenarios

- GIVEN 5 rows (2 tier=3, 1 tier=1, 2 tier=2), WHEN `search_all_memories(k_final=3)` is called, THEN exactly 3 results are returned, none with `tier=1`, sorted descending by score.

#### Red tests

- `memory::search::tests::integration_5_rows_fake_embedder_top3_sorted`

---

### REQ-TEST-003 — `prompt_builder.rs` golden-string integration test

**Trace**: D8, D4

**The test suite MUST** include an end-to-end test in `memory::prompt_builder::tests` that:

1. Uses `MemoryStore::new_for_test()` + `tempfile::TempDir` for the workspace base.
2. Inserts 5 memory rows: 3 `chat_message` (2 recent, 1 old), 1 `user_preference` with `importance=10` and `tier=1`, 1 row with `status='disputed'` + `disputed_against=JSON([old_id])`.
3. Calls `regenerate_auto_section` to populate the workspace file.
4. Calls `build_memory_prompt` with a `FakeEmbedder`.
5. Asserts the output contains all four separators (WORKSPACE, RECENT, LONG-TERM, CONFLICTS) in order.
6. Asserts the `user_preference` row appears in the WORKSPACE AUTO section (not in RECENT/LONG-TERM — it is tier=1).
7. Asserts the disputed pair appears in the CONFLICTS block.

Dates in the output MUST be validated against a known test `created_at` value (no reliance on `now()` for date assertions — use a fixed epoch timestamp).

#### Scenarios

- GIVEN the test setup above, WHEN `build_memory_prompt` is called, THEN the output is a multi-block string with correct separators, user_preference in WORKSPACE, chat hits in RECENT/LONG-TERM, and the disputed pair in CONFLICTS.

#### Red tests

- `memory::prompt_builder::tests::e2e_full_4_block_output`

---

### REQ-TEST-004 — No sleep in tests; baseline must not regress

**Trace**: Phase 2 / Phase 3 carry-forward

**The test suite MUST NOT** contain any `tokio::time::sleep(Duration::from_secs(...))` call in Phase 4 tests. All assertions MUST be direct DB queries, string equality checks, or channel-based signaling.

**All 315 Phase 1+2+3 tests MUST** still pass after Phase 4 is applied. The net new Phase 4 test count target is 30–50 tests. The final `cargo test --lib` output MUST show `315 + N` passing with `N ≥ 30` and zero failures.

`cargo build` is forbidden. All gating uses `cargo check` + `cargo test --lib` exclusively.

#### Scenarios

- GIVEN the final Phase 4 apply state, WHEN `cargo test --lib --manifest-path src-tauri/Cargo.toml` runs, THEN the output shows ≥ 345 passing tests with 0 failures and 0 regressions on the 315 baseline.

#### Red tests

- (Meta-requirement — no new red test; the 315 baseline tests are already green and MUST stay green.)

---

## Cross-cutting requirements

### Logging

All new code MUST use `log::{debug, warn, info}` (the codebase logging crate, 43 existing call sites). `tracing::` is NOT in the dependency graph and MUST NOT be added. Retrieval timing: `log::debug!`. Embedder cold-start skip: `log::warn!`. Workspace I/O failure: `log::warn!`. UDF registration failure: propagated as `MemoryError` (not silently logged).

### Error types

Phase 4 EITHER extends `MemoryError` with new variants (`WorkspaceIo`, `RetrievalFailed`) OR introduces separate `RetrievalError` / `WorkspaceError` enums — the design phase decides the split. All new error types MUST implement `std::error::Error` via `thiserror`. No `anyhow`. No `unwrap()` / `expect()` in non-test code. Use `?` + error variant propagation.

### Visibility

- All four new modules: `pub(crate) mod` in `memory/mod.rs`.
- `register_score_udfs`, `SearchScope`, `RankedHit`, `search_all_memories`, `WorkspaceManager`, `DEFAULT_PROJECT_SEGMENT`, `AUTO_START`, `AUTO_END`, `USER_START`, `USER_END`, `AUTO_TOKEN_CAP`, `build_memory_prompt`: `pub(crate)`.
- No `pub` re-exports from `memory/mod.rs` for Phase 4 types (all consumers are within the same crate).

### No new Cargo dependencies

Phase 4 is a pure-Rust, dep-graph-stable change. Zero new entries in `src-tauri/Cargo.toml`. All required functionality is available via existing deps (`rusqlite::functions`, `tokio::fs`, `tokio::try_join!`, `std::sync::Arc`, `log`).

### Strict TDD

Every implementation task MUST be preceded by its failing test. Task pairs follow the Phase 3 convention: `*-test` (red) → `*-impl` (green). No `cargo build` at any step. Only `cargo check` + `cargo test --lib`.

---

## Out of scope

The following items are explicitly NOT part of Phase 4. Their absence is intentional and MUST NOT be treated as a regression:

| Item | Phase |
|---|---|
| `AccessTracker` — `last_accessed_at` updates on retrieval | Phase 5 |
| `dirty: AtomicBool` flag on `AppState` for lazy workspace invalidation | Phase 5 |
| Tier promotion (`compute_tier`) and `access_count` update | Phase 5 |
| FTS5 fallback when embedder is not ready | Deferred (accepted degradation per D1 / R-NEW-3) |
| `disputed_against_ids() -> Option<Vec<i64>>` typed accessor on `MemoryRecord` | Phase 6 |
| Contradiction detector (`detect_contradiction`) and sycophancy resolver | Phase 6 |
| `status='disputed'` row insertion (rows produced by detector) | Phase 6 |
| Any modification to `models/prompt.rs::compose_system_prompt` | Out of scope permanently (R-NEW-4) |
| Criterion bench harness / `benches/` directory | Out of scope (D1) |
| `cargo bench` as a gate step | Out of scope (D1 + user CLAUDE.md rule against `cargo build`) |
| Phase 7 compaction integration (`MemorySource::Compaction` path) | Phase 7 |
| Cross-scope promotion (agent → project → global) | Phase 12 |
| New Tauri commands | None in Phase 4 |
| Angular / frontend changes | None in Phase 4 |

---

## Spec ↔ Design alignment (resolved before sdd-tasks)

All 5 original open issues + design's 7 follow-up questions are RESOLVED. Locked answers below; sdd-tasks treats these as additive constraints to D1–D9.

| # | Question | Locked answer | Source |
|---|----------|---------------|--------|
| 1 | Error type strategy | **Extend `MemoryError`** with `WorkspaceIo { path, source }` + `Backend(String)` variants. No new module-local enums. | design §4.1 |
| 2 | `WorkspaceManager` construction site | **Local in `stream_chat`** — `WorkspaceManager::new(state.base_dir.join("workspaces"))` before the agentic loop. NO field on `AppState`. | design §2.3 + §6.1 |
| 3 | `user_query` for `build_memory_prompt` | **`&message`** — the `stream_chat` parameter at line 411-417. Single-message query, no concat. | design §6.2 |
| 4 | AUTO section SQL queries (5 categories) | **`fetch_auto_rows` private helper** in `workspace.rs` runs the 5 SELECTs in a single `Connection::call`. Order: user_pref (importance≥8) → arch_decision (importance≥8, scoped to project) → top-5 procedural_skill by access_count DESC → current_focus stub (empty in Phase 4) → workspace_disputes (Tier-1 IDs, empty in Phase 4). | design §2.3 |
| 5 | Token counting in `regenerate_auto_section` | **Byte-length approximation** — `AUTO_BYTE_CAP = AUTO_TOKEN_CAP * 4 = 6000 bytes`. NO TokenSplitter call (Phase 3's tokenizer reserved for retrieval-side). Recovery order (reverse priority, prefs NEVER trimmed): disputes → focus → skills → arch_decision → user_prefs. | design §2.3 + §3.3 |
| 6 | Date formatting for `format_hit_line` | **Zero-dep integer arithmetic** — private `fn unix_secs_to_ymd(ts: i64) -> (i32, u32, u32)` (Howard Hinnant algorithm, ~30 lines, unit-tested). NO new `time` / `chrono` / `jiff` dep. | design §3.5 + §11.1 + R-NEW-12 |
| 7 | `K_FINAL` for `build_memory_prompt` | **`const K_FINAL: usize = 10;`** in `prompt_builder.rs`. Balances RECENT + LONG-TERM coverage against the prefix token budget. | design §2.4 + §11.3 |
| 8 | `SOURCE:` label format | **`SOURCE: {TableKind:?} | TYPE: {memory_type}`** — `TableKind` debug ("Memory"/"Episodic"/"Procedural") for source, schema column for type. | design §3.5 + §11.4 |
| 9 | `current_focus` section in workspace.md | **Emit empty header** (`## Current Session Focus` with no body). Phase 9 autoDream fills the body without changing the file shape. | design §2.3 + §11.5 |
| 10 | `memory/mod.rs` re-exports | **NONE in Phase 4** — only `pub(crate) mod` declarations. Phase 5+ adds `pub(crate) use` re-exports when a second consumer materializes. KISS. | design §6.6 + §11.7 |

### Tests that materially change with these answers

- REQ-WORKSPACE-001 ctor test renamed: `new` takes `workspaces_root` (caller pre-joins).
- REQ-WORKSPACE-003 byte-cap test asserts `≤ AUTO_BYTE_CAP = 6000 bytes` (NOT a tokenizer-counted 1500-token assert).
- REQ-PROMPT-003 hit line test asserts the explicit format `"[YYYY-MM-DD] SOURCE: Memory | TYPE: chat_message\nCONTENT: ..."`.
- REQ-PROMPT-004 CONFLICTS SQL test asserts the filter includes `m_new.deleted_at IS NULL` (corrected from earlier spec draft).

