# Phase 4 — rag-retrieval — Proposal

> **Status**: contract. Downstream phases (spec, design, tasks, apply, verify, archive) treat the 9 locked decisions below as non-negotiable unless an explicit re-litigation note is filed against THIS proposal.
>
> **Inputs read while drafting**: `docs/sdd/rag-retrieval/explore.md` (550 lines, 5 resolved decisions, 5 open questions, 8 R-NEW risks), `docs/RAG-PHASE-PLAN.md` Phase 4 (lines 208–244), `docs/6-improvements-RAG.md` (canonical RAG: score formula §463–525, sycophancy guard §547–650, query final §743–835, search orchestrator §837–912, prompt injection §1047–1077, workspace §1079–1166), `docs/sdd/rag-ingest/proposal.md` (Phase 3 style + carry-forwards), `docs/sdd/rag-ingest/archive-report.md` (Phase 3 lessons + 315/315 baseline + invariants).
>
> **Test baseline at start**: 315/315 Rust PASS (249 Phase 1+2 + 66 Phase 3) + 22 Angular tests. Phase 4 must finish at `315 + N` PASS where `N` is the net new test count (~30–50 per Phase 4's ~40-task estimate). Zero regressions, zero new clippy warnings.

---

## 1. Goal

`rag-retrieval` is the **consumer side** of the RAG pipeline. The embeddings persisted by Phase 3 (`memory` + `memory_vectors`, plus the still-empty-in-Phase-4 `episodic_trajectory` + `procedural_skill` mirror tables) finally reach the LLM. The phase adds four modules:

1. **`memory/score.rs`** — registers four `SQLITE_DETERMINISTIC` scalar UDFs (`exp_decay`, `lambda_for_type`, `importance_factor`, `dispute_factor`) on every connection opened by `MemoryStore`.
2. **`memory/search.rs`** — exposes `SearchScope` + `RankedHit` + `search_all_memories(...)` that runs three parallel KNN queries (`knn_memory`, `knn_episodic`, `knn_procedural`) via `tokio::try_join!`, merges into a single ranked list, and truncates to `k_final`.
3. **`memory/workspace.rs`** — `WorkspaceManager` owns the on-disk `agent_workspace.md` at `~/.claw-garden/workspaces/{agent_id}/{project_id_or_default}/workspace.md`, regenerates the `<!-- AUTO-START -->…<!-- AUTO-END -->` section from DB queries, and preserves the `<!-- USER-START -->…<!-- USER-END -->` section byte-for-byte.
4. **`memory/prompt_builder.rs`** — assembles the four `===`-separated blocks (`WORKSPACE`, `RECENT CONTEXT (Last 7 Days)`, `LONG-TERM KNOWLEDGE`, `ACTIVE CONFLICTS (review needed)`) and returns a `String` prefix.

The hot path (`commands/chat.rs::stream_chat`) is modified to **pre-compute** the memory prefix ONCE before the agentic loop (D2) and concatenate it with the existing `agent_system_prompt` for every iteration of `for iteration in 0..MAX_TOOL_ITERATIONS`. `models/prompt.rs::compose_system_prompt` is **not touched** — keeps all 10 of its tests green (R-NEW-4 mitigation).

Regression risk is the highest of any phase so far because `stream_chat` is the most-tested path in the codebase (4 tests) and the modification ripples through `state.rs` (8 tests), `memory/db.rs` (15+ tests via UDF registration in `open()`/`new_for_test()`), and the worker (~10 tests that depend on the store being functional).

---

## 2. Locked decisions (do NOT re-debate)

Each decision is anchor-linkable from spec/design/tasks. A change to any of these requires updating this proposal first, then propagating downstream.

### <a id="D1"></a>D1 — Hot path latency budget: accept <50ms target, NO bench harness in Phase 4

**Decision**: Accept the documented <50ms budget (KNN + decay + workspace read at 100k vectors) without a Criterion bench harness. Add a `log::debug!("retrieval took {}ms", elapsed.as_millis())` call inside `search_all_memories` (after the `tokio::try_join!` block, before returning `Ok(all)`). NO `benches/` directory, NO `cargo bench` step.

**Why**: `benches/` does not exist in `src-tauri/` (verified by explore). Building a bench harness would require either pulling in `criterion` (new dep, blocked by user rule against `cargo build`) or hand-rolling timing tests under `cargo test --lib` — neither produces stable P99 numbers without a controlled-load runner that this project does not have. The 50ms figure in doc 6 is an aspirational ceiling at 100k vectors, not a regression gate for a project whose largest known fixture is the 5-row Phase 3 E2E test. Logging the actual elapsed at `debug` level makes the cost observable when a user reports lag, which is the only realistic feedback channel today.

**Alternatives considered**:
- **Criterion bench at retrieval cold/warm with 1k / 10k / 100k vector fixtures**: rejected. Requires `cargo build` (user rule violation), requires generating 100k synthetic vectors at test setup (slow, not free), and the result would be a number tuned for the dev machine — not predictive of user hardware variance.
- **Bench inline inside `cargo test --lib` using `Instant::now()` + `assert!(elapsed < Duration::from_millis(50))`**: rejected. Flaky on shared CI / slow SSD / Windows defender scanning. Would either be ignored (`#[ignore]`) and forgotten or fail spuriously and erode trust in the test suite.

**Consequences**:
- **Enables**: Phase 4 ships in proposed scope without bench infrastructure debt.
- **Precludes**: automatic regression detection if Phase 5 (`AccessTracker`) or Phase 6 (sycophancy resolver) inadvertently regress retrieval latency. Mitigated by `log::debug!` instrumentation.
- **Ripples**: if production reports lag, the first lever is reducing `k_pool` from 30 → 10 (in `SearchScope::all_default()`) AND the already-implemented `WHERE m.tier > 1` filter (D5). The second lever is moving workspace.md regeneration to lazy (Phase 5's `dirty` flag — D4 carry-forward).

**Locked at**: sdd-propose (you), 2026-05-24, ref `explore.md` §D1 + `RAG-PHASE-PLAN.md:229`.

### <a id="D2"></a>D2 — Retrieval frequency in agentic loop: first-iteration only

**Decision**: Pre-compute the memory prefix **once per `stream_chat` invocation**, BEFORE the `for iteration in 0..MAX_TOOL_ITERATIONS` loop. The same `format!("{}\n\n{}", memory_prefix, agent_system_prompt)` string seeds `ChatRequest.system_prompt` on every iteration. Retrieval runs `1×` per user message, not `1..=5×`.

**Why**: `ChatRequest` is currently constructed INSIDE the agentic loop (line 548 of `commands/chat.rs`), and `agent_system_prompt` is re-cloned from the pre-loop capture on every iteration — so the system_prompt content is already identical across all 5 possible iterations today. Per-iteration retrieval would offer a theoretical benefit (newly-ingested chunks from intermediate tool results visible in iteration N+1) but Phase 3's `memory_worker` is async fire-and-forget with no commit-before-next-iter guarantee — the race window is small but real, so the practical hit-rate of "new chunk visible mid-loop" is near zero. Cost is concrete: `1..=5× ~10ms` extra latency per user message that produces zero measurable quality gain.

**Alternatives considered**:
- **Per-iteration retrieval** (build memory prefix inside the agentic loop): rejected. 1-5× retrieval cost per user message, ingest-vs-retrieval race that is fundamentally unfixable without a synchronous commit barrier (which we don't want — it would block the chat hot path on disk I/O).
- **Per-iteration retrieval but only for iterations where the previous iteration produced `ToolCalls`**: rejected. Adds branching logic at the `stream_chat` call site that doesn't pull weight for v1 — the race still exists, just at half frequency.

**Consequences**:
- **Enables**: simple call site (one extra block before the loop), zero ingest-vs-retrieval race surface in Phase 4.
- **Precludes**: dynamic memory injection during a multi-turn tool sequence. Acceptable for v1 — most agentic sequences are 1-2 iterations.
- **Ripples**: a `// TODO Phase X: re-run retrieval per iteration when AccessTracker emits tier-change events mid-loop` comment lands at the call site as a forward-marker. Phase 5+ may revisit when `AccessTracker` provides a synchronous "tier changed" signal that justifies re-querying.

**Locked at**: sdd-propose (you), 2026-05-24, ref `explore.md` §D2 + user Q4.

### <a id="D3"></a>D3 — `disputed_against` typed decode: defer entirely to Phase 6

**Decision**: `MemoryRecord::disputed_against()` keeps its `Option<&str>` return type. `MemoryRow.disputed_against` keeps its `Option<String>` shape. Phase 4's `ACTIVE CONFLICTS` query (D8) uses SQLite's built-in `json_each(m_new.disputed_against)` to expand the JSON array INSIDE the SQL query, NOT on the Rust side.

**Why**: `dispute_factor` UDF takes `status: TEXT`, NOT the JSON array — the Rust struct never needs `Vec<i64>` in Phase 4. The CONFLICTS query joins `memory m_new` to `memory m_old` via `m_old.id IN (SELECT value FROM json_each(m_new.disputed_against))` — pure SQL, zero Rust-side decode. `json_each(NULL)` returns zero rows in SQLite (verified by docs, see R-NEW-7 in explore), so no NULL guard is needed. The bundled `rusqlite` ships with `-DSQLITE_ENABLE_JSON1` by default, so `json_each` is always available.

**Alternatives considered**:
- **Add typed `disputed_against_ids() -> Option<Vec<i64>>` accessor in Phase 4**: rejected. Phase 4 has no caller — `dispute_factor` operates on `status`, the CONFLICTS query uses SQL `json_each`. Adding a Rust-side decoder now would be dead code until Phase 6's contradiction detector needs to BUILD `disputed_against` (the inverse direction).
- **Migrate `MemoryRow.disputed_against` to `Vec<i64>` at the SQL row → struct boundary**: rejected. Requires touching every row-mapping site for zero Phase 4 value, and Phase 6 still needs the raw `String` to write back via `INSERT` / `UPDATE`.

**Consequences**:
- **Enables**: Phase 4 ships with zero changes to `memory/record.rs`. The 10 row-mapping tests stay green untouched.
- **Precludes**: any Phase 4-internal Rust code that needs to read the JSON array. None exists; CONFLICTS is the only consumer and it uses SQL.
- **Ripples**: Phase 6 MUST add a typed accessor (`disputed_against_ids() -> Option<Vec<i64>>`) alongside the existing `Option<&str>` raw accessor — additive, no break for Phase 4 callers (none exist).

**Locked at**: sdd-propose (you), 2026-05-24, ref `explore.md` §D3.

### <a id="D4"></a>D4 — Workspace.md cache invalidation: always regenerate at `stream_chat` start (v1)

**Decision**: `WorkspaceManager::regenerate_auto_section(agent_id, project_id, db)` is a standalone async function. Phase 4's `stream_chat` call site invokes it UNCONDITIONALLY once per chat invocation, BEFORE building the memory prefix. NO `dirty` flag on `AppState`, NO interior cache state on `WorkspaceManager`.

**Why**: doc 6 says workspace.md regenerates "at tier changes (eager) and at session close (lazy)" — but Phase 5's `AccessTracker` (which is what changes tiers at access time) does NOT exist yet. Phase 3's ingest worker inserts with `tier = 3` and never mutates tier afterward. So there is no concrete "tier change event" for Phase 4 to listen on. The two surviving options for Phase 4 are: (a) always regenerate at chat start (cheap: one DB query, one disk write, ~2-5ms total), or (b) check file `last_modified` vs a `dirty` flag (more code, same outcome in practice because the file mtime barely changes between sessions without Phase 5).

Option (a) is correct for v1: cheap, simple, Phase 5-compatible. When Phase 5 ships `AccessTracker`, the same `regenerate_auto_section()` function will be invoked from the access tracker flush — the WorkspaceManager has no interior coupling that would block Phase 5 from adding multiple call sites.

**Alternatives considered**:
- **Lazy regeneration with `dirty: AtomicBool` on `WorkspaceManager`**: rejected for v1. Adds state that Phase 4 cannot meaningfully invalidate (no tier-change source). Would either always be `true` (same as eager) or always be `false` (workspace never regenerates) — both pathological without Phase 5.
- **Skip workspace regeneration in Phase 4, only inject existing file content**: rejected. The first ever `stream_chat` call would find no file, return no WORKSPACE block, and the user never gets Tier 1 memories injected. Phase 4's deliverable explicitly includes "workspace.md is part of the system prompt" — must be regenerated at least once.

**Consequences**:
- **Enables**: WorkspaceManager has zero interior state, trivially `Clone` / `Send + Sync`, no mutex contention.
- **Precludes**: short-circuiting the regenerate when nothing changed. Wasted DB query per chat — measured cost is ~2-5ms, well within the 50ms budget (D1).
- **Ripples**: Phase 5 adds a `dirty: AtomicBool` field on `AppState` (or per-workspace), the access tracker flush sets `dirty = true` on any tier change, and `stream_chat` reads `dirty` to decide whether to call `regenerate_auto_section()`. Same function, different call gate — zero Phase 4 surface change required.

**Locked at**: sdd-propose (you), 2026-05-24, ref `explore.md` §D4 + `6-improvements-RAG.md:1149-1158`.

### <a id="D5"></a>D5 — Filter `WHERE m.tier > 1` in retrieval KNN queries

**Decision**: All three KNN queries (`knn_memory`, `knn_episodic`, `knn_procedural`) add `AND m.tier > 1` (and the equivalent for the `episodic_trajectory` / `procedural_skill` joins) to the `WHERE` clause. Tier 1 memories are excluded from retrieval and live exclusively in the WORKSPACE block.

**Why**: doc 6 line 1075 is unambiguous — *"El retrieval YA NO trae memorias Tier 1. Se filtran del KNN con `WHERE m.tier > 1`."* The rationale is to prevent Tier 1 from consuming top-K budget when those memories are already permanently in the WORKSPACE block (double inclusion + wasted slots). Phase 3's test data has zero Tier 1 rows (chat/tool messages all start at `tier=3`), so this filter excludes zero rows in Phase 4's tests — but the filter is architecturally correct and ready for any data that lands in Tier 1 via Phase 5 promotion or future direct insertion.

**Alternatives considered**:
- **No filter — let Tier 1 compete in retrieval with the `tier_floor=0.5` boost**: rejected. Per doc 6, this is exactly the failure mode the WORKSPACE block was designed to prevent — Tier 1 should NEVER fall out of the prompt due to KNN distance, and competing in retrieval risks that.
- **Filter only `knn_memory` (the polymorphic table), leave `knn_procedural` untouched** since `procedural_skill` rows default to `tier=1`: rejected. That would mean `knn_procedural` always returns Tier 1 procedural skills, which is the SAME double-inclusion problem (procedural skills live in WORKSPACE per doc 6 §1100-1102). The filter applies uniformly across all three.

**Consequences**:
- **Enables**: clean separation of "always-present context (Tier 1, WORKSPACE block)" from "ranked-by-similarity context (Tier 2/3, RECENT/LONG-TERM blocks)". Top-K is fully spent on the ranking-sensitive tier.
- **Precludes**: any retrieval-time visibility into Tier 1 memories. The WORKSPACE block must always exist when there are Tier 1 rows (D4 guarantees this).
- **Ripples**: Phase 5's `compute_tier()` promotion logic MUST regenerate workspace.md immediately on Tier-1 promotion (eager) — otherwise the promoted memory disappears from retrieval AND is absent from WORKSPACE for the gap window. Phase 5 will already handle this per doc 6 §1151.

**Locked at**: sdd-propose (you), 2026-05-24, ref `explore.md` §D5 + `6-improvements-RAG.md:1075`.

### <a id="D6"></a>D6 — UDF registration in `MemoryStore::open()` and `MemoryStore::new_for_test()`

**Decision**: Add `register_score_udfs(conn: &tokio_rusqlite::Connection) -> Result<()>` to `memory/score.rs`. Call it from `MemoryStore::open()` and `MemoryStore::new_for_test()` AFTER `run_migrations()`. Always-register pattern — no lazy-init, no caller-side opt-in.

**Why**: rusqlite UDFs are connection-scoped, not DB-file-scoped — they do NOT persist across `Connection::open()` calls. Registering at `MemoryStore::open()` time guarantees every code path that uses the store (production and test) has the four UDFs available, eliminating an entire class of "forgot to register" bugs. The pattern mirrors `ensure_vec0_registered` which Phase 1 already established for the vec0 extension. The cost is one extra `conn.call()` await at store construction (~sub-ms) and four 3-5 line UDF closures.

**Alternatives considered**:
- **Lazy-register in `search.rs::search_all_memories()` first call, gated by a `OnceCell<()>`**: rejected. Adds a synchronization primitive, masks the UDF-registration step at a non-obvious call site, and breaks the pattern set by `ensure_vec0_registered`.
- **Register UDFs in a new `MemoryStore::init_retrieval()` method called manually after `open()`**: rejected. Two-step construction is an anti-pattern — callers will forget, tests will silently use the store without UDFs and fail at query time with cryptic "no such function: exp_decay" errors.

**Consequences**:
- **Enables**: every `MemoryStore` instance has UDFs available before any query runs. Tests that don't even use retrieval get them for free (sub-ms cost). Production setup is identical to test setup.
- **Precludes**: opt-out for embedded scenarios that don't want UDFs registered. None exist; all consumers either use retrieval or are agnostic to extra UDFs.
- **Ripples**: any future UDF (Phase 6 `dispute_factor` enhancements, hypothetical Phase 7 `freshness_score`) lands inside `register_score_udfs` — single source of truth for connection-scoped scalar functions.

**Locked at**: user (you), 2026-05-24, ref user Q1.

### <a id="D7"></a>D7 — `dispute_factor` UDF fully implemented in Phase 4 (not stub)

**Decision**: `dispute_factor(status TEXT) -> REAL` UDF returns the full match per doc 6:
```rust
match status.as_str() {
    "active"   => 1.0,
    "disputed" => 0.6,
    _          => 0.0,  // 'superseded' | 'invalid' | anything else → 0.0
}
```
NO Phase 6 follow-up modification to `score.rs` is required.

**Why**: the canonical RAG doc (line 626-633, 778-785) already specifies the full implementation — three lines of pattern match, zero dependency on Phase 6's contradiction detector. A "stub" only makes sense when the target values are unknown; here they are fully specified. Implementing fully in Phase 4 saves Phase 6 a task and eliminates a "must update score.rs" reminder in the Phase 6 backlog.

Phase 6 ADDS data (`status='disputed'` and `status='superseded'` rows produced by the contradiction detector) — `dispute_factor` was always designed to handle those values; Phase 4 just inserts no rows that trigger them yet.

**Alternatives considered**:
- **Phase 4 ships `dispute_factor` always returning 1.0, Phase 6 swaps in the real implementation**: rejected. Two-step rollout for a three-line function — net cost is higher than just shipping the final code now. The "stub" buys nothing because no Phase 4 row has `status != 'active'`.

**Consequences**:
- **Enables**: Phase 6 only adds the contradiction detector + the row-status writes; the score formula is final.
- **Precludes**: nothing.
- **Ripples**: Phase 4's `score.rs` unit tests cover all four match arms ('active' → 1.0, 'disputed' → 0.6, 'superseded' → 0.0, fallback → 0.0). Phase 6 needs no `score.rs` test additions.

**Locked at**: user (you), 2026-05-24, ref user Q2 + `6-improvements-RAG.md:626-633`.

### <a id="D8"></a>D8 — `ACTIVE CONFLICTS` block in `prompt_builder.rs`: conditional, omit if zero rows

**Decision**: Phase 4's `prompt_builder.rs::build_memory_prompt` runs the CONFLICTS query (per doc 6 §652-663) joining `memory m_new` to `memory m_old` via `json_each(m_new.disputed_against)`, filtered to `WHERE m_new.status='disputed' AND m_new.agent_id = ?1 AND m_new.deleted_at IS NULL`. If the query returns ZERO rows, the `=== ACTIVE CONFLICTS (review needed) ===` separator and block are OMITTED from the output. If it returns rows, the block is included with one `[ts] OLD: ... NEW: ...` line per (old, new) pair.

**Why**: doc 6 §1054 + §1077 explicitly state the block is "conditional — only when disputed rows exist for the agent_id". Including an empty `=== ACTIVE CONFLICTS ===` header with no rows below it would pollute the system prompt and confuse the model. The query is cheap (single JOIN, single index lookup) so running it unconditionally is fine — only the OUTPUT is conditional. Phase 4's test data has zero disputed rows, so the block is always omitted in Phase 4 — but the query path is exercised, the conditional logic is exercised, and Phase 6 only needs to ADD rows for the block to start appearing. Zero Phase 6 changes to `prompt_builder.rs`.

**Alternatives considered**:
- **Skip the CONFLICTS query entirely in Phase 4, add it in Phase 6**: rejected. Leaves Phase 4 with a half-built `prompt_builder` that Phase 6 has to revisit, breaking the "Phase N ships its module complete" discipline. Also leaves a hardcoded "3-block prompt" that Phase 6 has to refactor to 4-block — risky.
- **Include the CONFLICTS header always, with "(none)" body when zero rows**: rejected. Wastes prompt tokens, confuses the model, contradicts doc 6's explicit "only present when disputes exist".

**Consequences**:
- **Enables**: `prompt_builder.rs` ships complete and Phase 6-compatible in Phase 4. Phase 6 only inserts disputed rows; the prompt assembly is final.
- **Precludes**: nothing.
- **Ripples**: Phase 4 tests must exercise both branches — `prompt_builder_with_zero_disputes_omits_block` and `prompt_builder_with_one_dispute_renders_block` (test data hand-INSERTed with `status='disputed'` + `disputed_against=JSON([id])`).

**Locked at**: user (you), 2026-05-24, ref user Q2 + `6-improvements-RAG.md:1054, 1077`.

### <a id="D9"></a>D9 — Workspace path for agents with no `project_path`: literal segment `default`

**Decision**: Define `const DEFAULT_PROJECT_SEGMENT: &str = "default";` in `memory/workspace.rs`. When the active agent has no `project_path` (Phase 3 D3 — `scope='agent'`, `project_id=None`), the workspace file lives at `~/.claw-garden/workspaces/{agent_id}/default/workspace.md`. When the agent has a project path, the path uses `derive_project_id(project_path)` as the segment.

**Why**: doc 6 §1083 references the path as `app_data_dir/workspaces/{agent_id}/{project_id_or_default}/workspace.md` without specifying the literal. A constant in `memory/workspace.rs` (the owner module) centralizes the literal so Phase 5/6 don't accidentally use a different string (`"none"`, `"unscoped"`, `"agent-only"`). The literal `default` matches the pattern used by Phase 3's `derive_project_id` (which would never produce the string `default` for any real path — `default` is reserved for the "no project" sentinel).

**Alternatives considered**:
- **Use the agent_id again as the path segment** (`{agent_id}/{agent_id}/workspace.md`): rejected. Visually confusing in the file tree, and ties the no-project case to a specific (agent_id) shape — breaks if agent_id format changes.
- **Skip the project segment entirely** (`{agent_id}/workspace.md` for no-project agents, `{agent_id}/{project_id}/workspace.md` for project agents): rejected. Inconsistent path shapes complicate the lookup logic and any future "list all workspaces" tooling.

**Consequences**:
- **Enables**: deterministic file path resolution: `workspace_path = base_dir.join("workspaces").join(agent_id).join(project_id.unwrap_or(DEFAULT_PROJECT_SEGMENT)).join("workspace.md")` — three lines, fully testable.
- **Precludes**: a real project named literally "default" from coexisting with the no-project agent workspace. Mitigation: `derive_project_id(path)` always produces `basename + "-" + hash4(path)` — the literal `default` cannot collide with any real `derive_project_id` output.
- **Ripples**: Phase 5/6 must use `DEFAULT_PROJECT_SEGMENT` (re-exported from `memory/workspace.rs`) instead of redefining the literal. Spec REQ-WS-001 will pin the constant name and visibility (`pub(crate)`).

**Locked at**: user (you), 2026-05-24, ref user Q3 + `6-improvements-RAG.md:1083`.

---

## 3. New files & modules

Each `pub(crate)` surface listed below is the **contract** for Phase 5+ consumers. Signatures may be refined by the design phase (parameter ordering, error type granularity) but the public types and function names are locked here.

### `src-tauri/src/memory/score.rs` (NEW)

```rust
//! UDFs registered on every MemoryStore connection.
//! Phase 4: exp_decay, lambda_for_type, importance_factor, dispute_factor.

pub(crate) async fn register_score_udfs(
    conn: &tokio_rusqlite::Connection,
) -> Result<(), MemoryError>;

// Tested via in-memory MemoryStore + SELECT exp_decay(86400, 0.023) -- etc.
// No public Rust closures; UDFs are SQL-callable only.
```

### `src-tauri/src/memory/search.rs` (NEW)

```rust
//! KNN orchestrator over 3 vec0 tables with combined score ranking.

#[derive(Debug, Clone, Copy)]
pub(crate) struct SearchScope {
    pub memory:     Option<usize>,   // k_pool per table; None = skip
    pub episodic:   Option<usize>,
    pub procedural: Option<usize>,
}

impl SearchScope {
    pub(crate) fn all_default() -> Self;         // (Some(30), Some(5), Some(5))
    pub(crate) fn procedural_only() -> Self;     // (None,    None,    Some(10))
    pub(crate) fn factual_pivots() -> Self;      // (Some(5), None,    Some(5))   — Phase 6 caller
}

#[derive(Debug, Clone)]
pub(crate) struct RankedHit {
    pub id: i64,
    pub kind: TableKind,            // existing enum from record.rs
    pub content: String,
    pub memory_type: String,
    pub tier: i64,
    pub status: String,
    pub score: f64,
    pub distance: f64,              // raw KNN distance (needed by Phase 6 contradiction detector)
    pub created_at: i64,            // for RECENT vs LONG-TERM bucketing in prompt_builder
}

pub(crate) async fn search_all_memories(
    store: &MemoryStore,
    embedding: &[f32],
    agent_id: &str,
    scope: SearchScope,
    k_final: usize,
) -> Result<Vec<RankedHit>, MemoryError>;

// Internal helpers (not part of public surface):
//   async fn knn_memory(...)     — runs SEARCH_SQL with lambda_for_type(memory_type)
//   async fn knn_episodic(...)   — runs same shape with hardcoded λ = 0.0077
//   async fn knn_procedural(...) — runs same shape with hardcoded λ = 0.0039
```

### `src-tauri/src/memory/workspace.rs` (NEW)

```rust
//! Per-(agent_id, project_id) workspace.md file owner.
//! Phase 4: AUTO section regeneration + USER section byte-preservation.

pub(crate) const DEFAULT_PROJECT_SEGMENT: &str = "default";
pub(crate) const AUTO_START: &str = "<!-- AUTO-START -->";
pub(crate) const AUTO_END:   &str = "<!-- AUTO-END -->";
pub(crate) const USER_START: &str = "<!-- USER-START -->";
pub(crate) const USER_END:   &str = "<!-- USER-END -->";
pub(crate) const AUTO_TOKEN_CAP: usize = 1500;

#[derive(Debug, Clone)]
pub(crate) struct WorkspaceManager {
    base_dir: PathBuf,
    // No interior mutability — stateless façade. Clone is cheap.
}

impl WorkspaceManager {
    pub(crate) fn new(base_dir: PathBuf) -> Self;

    pub(crate) fn workspace_path(&self, agent_id: &str, project_id: Option<&str>) -> PathBuf;

    /// Regenerates ONLY the section between AUTO_START / AUTO_END.
    /// USER_START / USER_END section preserved byte-for-byte.
    /// File created with both sections if it does not exist.
    pub(crate) async fn regenerate_auto_section(
        &self,
        agent_id: &str,
        project_id: Option<&str>,
        store: &MemoryStore,
    ) -> Result<(), MemoryError>;

    /// Reads the entire file (auto + user sections). Used by prompt_builder.
    /// Returns empty string if file does not exist (defensive — first run).
    pub(crate) async fn read(
        &self,
        agent_id: &str,
        project_id: Option<&str>,
    ) -> Result<String, MemoryError>;
}
```

### `src-tauri/src/memory/prompt_builder.rs` (NEW)

```rust
//! Assembles the 4-block memory prefix for the system prompt.

/// Builds the WORKSPACE / RECENT / LONG-TERM / [ACTIVE CONFLICTS] string.
/// Returns "" if there are no hits AND no workspace AND no conflicts (the prefix
/// would be useless — let the caller decide to concat or not).
pub(crate) async fn build_memory_prompt(
    store: &MemoryStore,
    workspace: &WorkspaceManager,
    embedder: Option<Arc<dyn Embedder>>,        // None → no retrieval, workspace + conflicts only
    agent_id: &str,
    project_id: Option<&str>,
    user_query: &str,                            // text used to compute the query embedding
) -> Result<String, MemoryError>;

// Internal:
//   const SEPARATOR_WORKSPACE: &str   = "=== WORKSPACE ===";
//   const SEPARATOR_RECENT:    &str   = "=== RECENT CONTEXT (Last 7 Days) ===";
//   const SEPARATOR_LONGTERM:  &str   = "=== LONG-TERM KNOWLEDGE ===";
//   const SEPARATOR_CONFLICTS: &str   = "=== ACTIVE CONFLICTS (review needed) ===";
//   const RECENT_WINDOW_SECS:  i64    = 7 * 86_400;
//   async fn fetch_active_conflicts(store, agent_id) -> Result<Vec<ConflictPair>>;
```

---

## 4. Modified files

| File | Change | Risk |
|------|--------|------|
| `src-tauri/src/memory/mod.rs` | Add `pub(crate) mod score;`, `pub(crate) mod search;`, `pub(crate) mod workspace;`, `pub(crate) mod prompt_builder;`. NO new re-exports — internal modules. | None — additive. |
| `src-tauri/src/memory/db.rs` | `MemoryStore::open()` and `MemoryStore::new_for_test()` call `score::register_score_udfs(&conn).await?` AFTER `run_migrations(&conn).await?`. Two lines per ctor, propagated `?` error path. | LOW — 15+ existing tests on store construction must still pass. The UDFs are pure adds (no override of existing names) — verified by `grep "create_scalar_function" memory/` returning zero matches (only `ensure_vec0_registered` exists, which registers vtab modules, not scalar fns). |
| `src-tauri/src/commands/chat.rs::stream_chat` | BEFORE the `for iteration in 0..MAX_TOOL_ITERATIONS` loop: (1) compute `let memory_prefix = build_memory_prompt(...).await.unwrap_or_default();` — `unwrap_or_default` so retrieval failure does not break chat (logs warn inside the function); (2) `let combined_system_prompt = if memory_prefix.is_empty() { agent_system_prompt.clone() } else { format!("{}\n\n{}", memory_prefix, agent_system_prompt) };`. INSIDE the loop, `ChatRequest.system_prompt` becomes `Some(combined_system_prompt.clone())`. | MEDIUM — 4 existing tests in chat.rs's `#[cfg(test)]` module. The `ChatRequest.system_prompt` content changes from `agent_system_prompt` to `combined_system_prompt`, which is `agent_system_prompt` when no memories exist. Tests that don't seed memories will see identical content. |

**Files NOT touched** (locked by D2/D3/R-NEW-4):
- `src-tauri/src/models/prompt.rs` — `compose_system_prompt` signature and body unchanged. All 10 tests pass without modification.
- `src-tauri/src/memory/record.rs` — `MemoryRecord` trait and row structs unchanged.
- `src-tauri/src/memory/migrations/*.sql` — no schema changes (all required columns present per explore §"Existing Schema Verification").
- `src-tauri/src/state.rs` — `AppState` shape unchanged. No new field needed (`memory_store` + `embedder` + `base_dir` are sufficient; `WorkspaceManager` is constructed at the `stream_chat` call site from `state.base_dir`, or stored as a derived `AppState` field — design decides).

---

## 5. Cargo.toml

**Zero new dependencies.** Phase 4 uses only:
- `rusqlite::functions::FunctionFlags` + `create_scalar_function` (already a dep via Phase 1).
- `tokio_rusqlite::Connection::call` (already a dep).
- `tokio::fs::{read_to_string, write}` (already a dep via Tauri).
- `tokio::try_join!` (already a dep).
- `sqlite-vec` vec0 KNN query (already linked via Phase 1).
- `std::path::{Path, PathBuf}`, `std::time::Instant`, `std::sync::Arc` (stdlib).
- `log::{debug, warn}` (already 43 call sites — codebase logging crate, NOT `tracing`).
- `thiserror` for `MemoryError` extension if needed (already a dep).

The bundled `rusqlite` ships with `-DSQLITE_ENABLE_JSON1` by default, providing `json_each` for the CONFLICTS query at zero dep cost. This was verified during explore via the existing JSON column presence in `MemoryRow.disputed_against`.

Phase 4 is a **pure-Rust, dep-graph-stable** change.

---

## 6. Test strategy

Strict TDD applies — every impl preceded by its failing test (`*-test` → `*-impl` task pairing, same convention as Phase 1/2/3). All assertions are documented in `spec.md`.

### Unit tests

| Module | Test surface |
|--------|--------------|
| `memory/score.rs` | `exp_decay(0, 0.023) == 1.0`; `exp_decay(86_400, 0.023) ≈ 0.977` (1d); `exp_decay(7 × 86_400, 0.099) ≈ 0.5` (verifies 7d half-life for tool_error). `lambda_for_type("user_preference") == 0.0019`; `..._("chat_message") == 0.023`; `..._("unknown_type") == 0.023` (default arm). `importance_factor(1.0) == 0.55`; `importance_factor(10.0) == 1.0`; `importance_factor(5.0) == 0.75`; clamping behavior (R-NEW-8): `importance_factor(0.0) == 0.55` (clamp to 1.0 floor) AND `importance_factor(15.0) == 1.0` (clamp to 10.0 ceiling). `dispute_factor("active") == 1.0`; `dispute_factor("disputed") == 0.6`; `dispute_factor("superseded") == 0.0`; `dispute_factor("invalid") == 0.0`; `dispute_factor("bogus") == 0.0`. |
| `memory/search.rs` | `SearchScope::all_default()` returns `(Some(30), Some(5), Some(5))`. `SearchScope::procedural_only()` returns `(None, None, Some(10))`. `SearchScope::factual_pivots()` returns `(Some(5), None, Some(5))`. `search_all_memories` with `embedder` returning fixed vector + 3 hand-INSERTed `memory` rows: returns 3 ranked hits in DESC score order. `WHERE m.tier > 1` filter: insert one row with `tier=1` and three with `tier=3`, assert only the tier-3 rows appear. `WHERE m.status IN ('active', 'disputed')` filter: insert one `status='superseded'`, assert it is excluded. Empty embedder (`OnceCell::get() == None` in caller): returns `Ok(vec![])` (caller-side branch; tested at `prompt_builder` layer). |
| `memory/workspace.rs` | `workspace_path` resolves with `Some("foo-1234")` → `…/workspaces/{agent_id}/foo-1234/workspace.md`. `workspace_path` with `None` → `…/workspaces/{agent_id}/default/workspace.md`. `regenerate_auto_section` on a non-existent file creates the file with both AUTO and USER sections. `regenerate_auto_section` on an existing file with USER content: AUTO regenerates, USER preserved BYTE-FOR-BYTE (including any trailing whitespace, Unicode, code fences). `regenerate_auto_section` with zero matching rows in DB: AUTO section contains the headers (`## User Preferences`, etc.) with no bullet lines under them. Hard cap at 1500 tokens: insert 200 user_preference rows with importance=10, assert AUTO section ≤1500 tokens after recovery order (per doc 6 §1124: disputes → focus → skills → arch decisions → user_preference). `read` on a missing file returns `Ok("")`. |
| `memory/prompt_builder.rs` | `build_memory_prompt` with empty store + missing workspace file: returns `Ok("")`. With workspace file present + zero KNN hits: returns `=== WORKSPACE ===\n<content>` only, no RECENT/LONG-TERM/CONFLICTS separators. With workspace + 3 KNN hits (2 within 7 days, 1 older): RECENT has 2 entries, LONG-TERM has 1 entry. With one disputed memory pair: CONFLICTS block is included with the expected `OLD: ... NEW: ...` format. With NO disputed pairs: CONFLICTS block is OMITTED (no separator). Separator format exact match: `=== WORKSPACE ===\n`, `=== RECENT CONTEXT (Last 7 Days) ===\n`, `=== LONG-TERM KNOWLEDGE ===\n`, `=== ACTIVE CONFLICTS (review needed) ===\n`. Line format per hit: `[YYYY-MM-DD] SOURCE: ... | TYPE: ...\nCONTENT: ...`. |

### Integration tests

| Location | Surface |
|----------|---------|
| `memory/search.rs::tests` | Full `search_all_memories` integration with `MemoryStore::new_for_test()` + `FakeEmbedder` + 5 hand-INSERTed rows spanning 3 memory_types. Assert: top-3 returned, sorted by score DESC, scores computed correctly per the formula (`(1-distance) × MAX(floor, decay) × imp_factor × dispute_factor`). |
| `memory/prompt_builder.rs::tests` | End-to-end: in-memory store + temp dir for workspace + 5 ingested rows + 1 user_preference (importance=10) → assert full 4-block prompt output matches a golden string fixture (with `[date]` substituted to a stable test timestamp). |
| `commands/chat.rs::tests` (NEW) | `stream_chat_with_memories_prepends_blocks_to_system_prompt`: instrument a test provider that captures `ChatRequest.system_prompt`, assert it starts with `=== WORKSPACE ===` when memories exist. `stream_chat_with_no_memories_uses_unmodified_system_prompt`: assert the system_prompt is byte-equal to `agent_system_prompt` (no separators added). `stream_chat_with_embedder_not_ready_does_not_panic`: clear the embedder OnceCell, assert chat proceeds with no prefix (warn logged, no error returned). |

### Failure-mode tests

| Scenario | Expected |
|----------|----------|
| `state.embedder.get() == None` at retrieval | `search_all_memories` returns `Ok(vec![])`; `prompt_builder` skips RECENT/LONG-TERM/CONFLICTS, still emits WORKSPACE if file present; chat proceeds. `log::warn!` recorded. |
| `tokio::fs::read_to_string(workspace_path)` returns `ErrorKind::NotFound` | `WorkspaceManager::read` returns `Ok("")` (defensive — first run on a fresh install). |
| `tokio::fs::write(workspace_path)` fails (e.g. permission denied) | `regenerate_auto_section` returns `Err(MemoryError::WorkspaceIo(...))`; caller (chat) logs warn and proceeds without WORKSPACE block. |
| `json_each(NULL)` on a row with `disputed_against IS NULL` | Returns zero rows (verified by SQLite docs, R-NEW-7); no NULL guard needed. |
| KNN returns zero hits (empty database) | `search_all_memories` returns `Ok(vec![])`; `prompt_builder` returns `""` if workspace also empty. |
| Two simultaneous `regenerate_auto_section` calls for the same path | Last writer wins (`tokio::fs::write` is atomic on rename-on-write platforms; for Phase 4 single-agent setup, this is not exercised in tests but documented as accepted risk R-NEW-6). |

### NO sleep-in-tests

Carry forward Phase 2 + Phase 3 lesson. All test assertions are direct DB queries or string equality. `tokio::time::pause()` + `tokio::time::advance()` for any retry timing if added. No `tokio::time::sleep(Duration::from_secs(...))` in tests.

### NO bench harness

Per D1, no `benches/` directory added. Latency is observed via `log::debug!` in production code.

### Test count target

- **Baseline at start**: 315/315 Rust PASS.
- **Phase 4 net new**: 30-50 tests across the 4 new modules + `chat.rs` integration. Est. distribution: ~12 score.rs (4 UDFs × 3 cases), ~10 search.rs (constructors + filters + scoping), ~10 workspace.rs (path + read + write + cap + preservation), ~8 prompt_builder.rs (4 separators + 4 branches), ~3 chat.rs integration.
- **Phase 4 end target**: 345-365 / 345-365 PASS. Zero regressions on the 315 baseline.

---

## 7. Risks carried forward from explore

All 8 R-NEW risks from `explore.md` carry into design with the following dispositions:

| ID | Risk | Phase 4 disposition |
|----|------|---------------------|
| **R-NEW-1** | Hot path KNN latency spike on cold SSD / HDD | Mitigated by `log::debug!` instrumentation (D1). If reported, levers are `k_pool` reduction + tier filter (already in D5). |
| **R-NEW-2** | Race between ingest worker and retrieval | Accepted. Just-ingested chunk for current turn may not appear in current-turn retrieval — that's correct behavior (retrieval reflects committed state). Documented in design lifecycle section. |
| **R-NEW-3** | Embedder cold-start during chat | Mitigated by `if let Some(embedder) = state.embedder.get()` guard at `build_memory_prompt` entry. Returns empty hits, `log::warn!`, chat proceeds. |
| **R-NEW-4** | `prompt.rs` tests assert content of `compose_system_prompt` | Mitigated by D2 + §4 of this proposal: `compose_system_prompt` is NOT modified. Memory prefix is prepended at the `stream_chat` call site. 10 tests stay green untouched. |
| **R-NEW-5** | `tokio_rusqlite` write contention between ingest worker and retrieval | Accepted for Phase 4 (sub-ms). Phase 5's `AccessTracker` batches via the same connection — its async batching design already accounts for this. |
| **R-NEW-6** | Workspace.md file corruption on concurrent writes | Accepted for Phase 4 single-agent setup. `tokio::fs::write` provides atomic write-then-rename on most platforms. Phase 5+ may add `Mutex<()>` per `(agent_id, project_id)` if multi-session lands. |
| **R-NEW-7** | `json_each()` on NULL `disputed_against` | Verified safe by SQLite docs. No guard needed; query returns zero rows for NULL input. |
| **R-NEW-8** | `importance` UDF input clamping | Mitigated: `importance_factor` UDF body clamps `imp = imp.clamp(1.0, 10.0)` before the formula. One extra line. Tested via the score.rs unit test surface above. |

Two additional Phase 4-specific risks surface during proposal drafting:

| ID | Risk | Mitigation |
|----|------|------------|
| **R-NEW-9** | UDF registration in `MemoryStore::open()` adds an `.await` to construction — may break tests that construct stores synchronously or expect `?Sized` futures. | Verify by `grep "MemoryStore::open\|new_for_test" src-tauri/src/` during apply. Existing ctors are already async (`pub async fn open(...)`, `pub async fn new_for_test() -> Self`) per Phase 1 archive. Risk: minimal. |
| **R-NEW-10** | `build_memory_prompt` is async and called from `stream_chat` which is already deeply async. Adding it before the agentic loop could subtly increase the time-to-first-token when no memories exist. | The "no memories" path returns `Ok("")` after one DB query (zero rows) + one `tokio::fs::read_to_string` (file may not exist, returns `Ok("")` per R-NEW-3-style guard). Total: sub-2ms overhead. Negligible vs. the ~500ms LLM TTFT typical of Anthropic/OpenAI providers. |

---

## 8. Constraints summary (Phase 4 → 5/6/7 contracts)

Compact reference of every Phase 4 commitment that ripples forward. Phase 5/6/7 MUST honor these without breaking Phase 4 callers.

| Phase 4 commitment | Phase 5 implication | Phase 6 implication | Phase 7+ implication |
|--------------------|---------------------|---------------------|----------------------|
| `MemoryRecord::disputed_against()` returns `Option<&str>` (D3) | — | MUST add `disputed_against_ids() -> Option<Vec<i64>>` as ADDITIVE accessor (no break). | — |
| `WorkspaceManager` is stateless (D4) | MAY add `dirty: AtomicBool` per (agent_id, project_id) on `AppState`; sets `true` on tier change; `stream_chat` reads to gate `regenerate_auto_section` call. | — | — |
| `register_score_udfs` is called in both `open()` and `new_for_test()` (D6) | Any new UDF (e.g. `freshness_score`) MUST be added inside `register_score_udfs`, not as a sibling function. | Same — sycophancy guard UDFs go inside `register_score_udfs`. | — |
| `dispute_factor` UDF is final (D7) | — | NO `score.rs` modification needed. Just INSERT rows with `status='disputed'` / `'superseded'`. | — |
| `ACTIVE CONFLICTS` block scaffolding is final (D8) | — | NO `prompt_builder.rs` modification needed. Just INSERT pairs (`disputed_against = JSON([old_id])` on new row). | — |
| `DEFAULT_PROJECT_SEGMENT = "default"` (D9) | MUST use the constant (re-exported as needed) when constructing workspace paths in the access tracker flush. | Same. | Same. |
| Retrieval excludes `tier=1` rows (D5) | `compute_tier` promotion to Tier 1 MUST eagerly trigger `regenerate_auto_section` — otherwise the promoted memory is invisible to the prompt for the gap window. | — | — |
| `SearchScope::factual_pivots()` exists (search.rs surface) | — | `detect_contradiction` consumes this exact constructor — name is locked. | — |
| `RankedHit.distance` is exposed (search.rs surface) | — | Phase 6's contradiction detector reads `distance < 0.20` from this field directly. | — |
| `MemoryStore` ctor remains `pub async fn` with same signature (D6, R-NEW-9) | — | — | Tauri setup hook continues to call `MemoryStore::open(...).await?`. |
| Memory prefix pre-computed once per `stream_chat` (D2) | MAY revisit when AccessTracker emits synchronous tier-change events mid-loop. | — | — |
| `compose_system_prompt` signature unchanged (R-NEW-4) | — | — | Any future system_prompt template addition lands inside `compose_system_prompt`, not in `prompt_builder.rs`. |

---

## 9. Test gates (must pass before archive)

- All Phase 1 tests still pass.
- All Phase 2 tests still pass.
- All Phase 3 tests still pass.
- All three are counted in the 315/315 Rust baseline established by Phase 3 archive — Phase 4 starts from 315/315.
- Net new Phase 4 tests pass (target: 30–50).
- No new clippy warnings introduced by Phase 4 modules (`cargo clippy --lib -- -D warnings` on `memory/score.rs`, `memory/search.rs`, `memory/workspace.rs`, `memory/prompt_builder.rs`, `memory/db.rs`, `memory/mod.rs`, `commands/chat.rs`).
- NO `cargo build` runs during development (user rule). `cargo check` + `cargo test --lib` are the only allowed CI-style commands.
- NO `tokio::time::sleep(Duration::from_secs(...))` in any new test.
- `log::` (not `tracing::`) for all instrumentation — match the 43-call-site codebase convention.

---

## 10. Estimated task count

~40 test/impl pairs, per the Phase 4 line in `RAG-PHASE-PLAN.md:244` ("Complejidad alta. ~40 tasks"). UDFs in SQLite + workspace markdown parser + integration with the chat hot path is the densest section.

### Apply checkpoints

| # | Checkpoint | Gate |
|---|------------|------|
| 1 | `memory/score.rs` + 4 UDFs registered in `MemoryStore::open()` and `MemoryStore::new_for_test()` | All 4 UDFs callable via `SELECT exp_decay(...)` in-memory test. All 315 existing tests still pass. ~12 new score.rs unit tests pass. |
| 2 | `memory/search.rs` with `SearchScope` + `RankedHit` + `search_all_memories` + 3 KNN helpers | Integration test: 5 hand-INSERTed rows + FakeEmbedder + `all_default()` scope → 5 ranked hits in correct DESC score order. `WHERE tier > 1` and `status IN ('active', 'disputed')` filters verified. ~10 new search.rs tests pass. |
| 3 | `memory/workspace.rs` with `WorkspaceManager` + AUTO/USER section discipline | USER section byte-preservation test on a hand-crafted fixture file with Unicode + trailing whitespace. 1500-token cap recovery order test. ~10 new workspace.rs tests pass. |
| 4 | `memory/prompt_builder.rs` assembling 4 blocks + CONFLICTS query | Golden string test for the full 4-block output (with stable test timestamp). CONFLICTS-omitted-when-empty test. CONFLICTS-included-when-disputed test (hand-INSERT a `status='disputed'` row with `disputed_against = JSON([old_id])`). ~8 new prompt_builder.rs tests pass. |
| 5 | `commands/chat.rs::stream_chat` integration: pre-compute prefix before loop, concat to `agent_system_prompt`, pass into `ChatRequest.system_prompt` | 3 new integration tests pass (prefix prepended, no-memory bypass, embedder-not-ready resilience). All 4 existing chat.rs tests still pass. Total target: 345–365 / 345–365 PASS. |

---

## 11. Carry-forward patterns from Phase 3 (apply unmodified)

| Pattern | Source | Where it applies in Phase 4 |
|---------|--------|------------------------------|
| Atomic INSERT via single `tokio_rusqlite::Connection::call` + transaction | Phase 3 `insert_memory_with_vector` | Phase 4's CONFLICTS query is read-only, so no transaction needed. UDF registration uses `.call()` with multiple `create_scalar_function` in one closure for connection-scoped atomicity. |
| Bespoke error enums per module + `thiserror` | Phase 1 `MemoryError`, Phase 2 `EmbedError`, Phase 3 `MemoryStoreError` / `ChunkerError` | Phase 4 EITHER extends `MemoryError` with `WorkspaceIo`, `RetrievalKnnFailed` variants OR introduces `RetrievalError` / `WorkspaceError`. Design decides the split. |
| `Arc<...>` for shared state, `OnceCell` for lazy init | Phase 2 `AppState.embedder` | Phase 4 reads `state.embedder.get()` and tolerates `None` (R-NEW-3). `WorkspaceManager` is `Clone + Send + Sync` because it holds only `PathBuf` (no Arc needed). |
| `pub(crate)` over `pub` unless cross-crate use is needed | Project convention | All four new modules are `pub(crate)`. Only the entry point invoked from `commands/chat.rs::stream_chat` (`build_memory_prompt`) is `pub(crate)` — no `pub` re-exports from `memory/mod.rs`. |
| Compile-time `Send + Sync` assertions via `const _: fn()` blocks | Phase 2 `embedder.rs` | Apply to `SearchScope`, `RankedHit`, `WorkspaceManager` — anything that crosses a task boundary via `tokio::try_join!` or `tokio::spawn`. |
| `log::` not `tracing::` | Phase 3 archive lesson | All instrumentation in Phase 4 uses `log::{debug, warn, info}`. Zero `tracing::` calls. |
| Strict TDD with dedicated mocks | Phase 2/3 lesson | `FakeEmbedder` (Phase 2 surface) for retrieval tests. `MemoryStore::new_for_test()` for in-memory store. `tempfile::TempDir` for workspace.md tests. |
| `tokio_rusqlite` multi-table writes in single `.call()` with `tx.transaction()` | Phase 1 `ensure_vec0_registered` + Phase 3 `insert_memory_with_vector` | Phase 4 UDF registration follows the same pattern: one `.call()` closure, four `create_scalar_function` calls, return `Ok(())`. |
| NEVER `cargo build` during development | User rule | Phase 4 uses `cargo check` + `cargo test --lib` exclusively. |
| No `Co-Authored-By` in commits, conventional commits only | User rule | All Phase 4 commits use `feat(rag): ...`, `test(rag): ...`, `refactor(rag): ...` format. |

---

## 12. Inconsistencies surfaced (read by orchestrator before spec)

1. **`docs/sdd/rag-retrieval/explore.md` exists and is comprehensive.** This proposal is grounded in it (550 lines, 5 D-decisions resolved with rationale, 8 R-NEW risks enumerated, 5 open questions all answered by user input). No drift from explore to propose.

2. **`MAX_TOOL_ITERATIONS = 5` is hardcoded in `commands/chat.rs:28`.** Phase 4 assumes this constant for sizing the "first-iteration only" decision (D2). If a future phase parameterizes or raises this, D2 should be revisited. Documented for orchestrator awareness.

3. **`compose_system_prompt` is called at `stream_chat:428` (pre-loop) per explore — NOT inside the agentic loop.** Explore confirms `agent_system_prompt` is captured once before the loop and cloned per iteration. This makes the "pre-compute memory prefix before the loop" decision (D2) trivially compatible with existing structure — no refactoring of the loop entry needed. The combined `format!("{}\n\n{}", memory_prefix, agent_system_prompt)` lives next to `agent_system_prompt`'s capture.

4. **`MemoryStore` ctor signatures already async per Phase 1 archive.** R-NEW-9 confirms no ripple from adding `.await` for UDF registration.

5. **No Open Questions surface beyond D1–D9.** Phase 4 scope is fully resolved by the locked decisions + the 8 carry-forward R-NEW risks. If apply discovers a gap, it MUST file against this proposal — orchestrator's gate.

6. **`pub(crate) const DEFAULT_PROJECT_SEGMENT` location.** D9 places this in `memory/workspace.rs`. Phase 5+ that need the constant should re-export from `memory/mod.rs` only if cross-module consumption emerges. Phase 4 has only one consumer (the WorkspaceManager itself) — no re-export needed today.

---

## 13. Done definition

Phase 4 is done when:

- All test gates in §9 pass.
- All 9 LOCKED decisions (D1–D9) honored without drift.
- 4 new modules (`score.rs`, `search.rs`, `workspace.rs`, `prompt_builder.rs`) ship complete and Phase 5/6-compatible per §8 constraints.
- 3 modified files (`memory/mod.rs`, `memory/db.rs`, `commands/chat.rs`) ship with the changes described in §4. Zero other files modified.
- Zero new dependencies in `src-tauri/Cargo.toml`.
- 30–50 net new tests pass on top of the 315 baseline.
- A real chat turn with seeded test memories produces a `ChatRequest.system_prompt` that starts with `=== WORKSPACE ===` (verified via integration test capturing the request).
- A real chat turn with an empty memory store + no workspace file produces a `ChatRequest.system_prompt` byte-equal to `agent_system_prompt` (no separators, no breakage).
- `docs/sdd/rag-retrieval/archive-report.md` documents the locked decisions that held and any that drifted (with rationale + lessons learned).
- Phase 5 (`rag-access-tracker`) is unblocked: it can call `WorkspaceManager::regenerate_auto_section` directly, can add UDFs via `register_score_udfs`, and the search/score path is stable.
