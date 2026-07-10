# Phase 4 — rag-retrieval — Exploration

**Date**: 2026-05-24
**Status**: complete
**Test baseline at start**: 315/315 Rust PASS (249 Phase 1+2 + 66 Phase 3) + 22 Angular Phase 2

---

## Goal Recap

Phase 4 closes the RAG loop: embeddings that have been persisted by Phase 3's ingest
pipeline finally reach the LLM. The phase adds KNN search, a score-combination formula
over four UDFs, a `workspace.md` file that pins Tier 1 memories to every prompt, and
a `prompt_builder` that assembles the four-block system prompt
(`WORKSPACE / RECENT / LONG-TERM / ACTIVE CONFLICTS`). The chat hot path
(`stream_chat`) is modified to prepend these blocks to the `system_prompt` on every
agentic loop iteration (decision to confirm — see D2). Regression risk is the highest
of any phase so far because `stream_chat` is the most-tested path in the codebase.

---

## Library Landscape

No new crates are required for the core path. All dependencies are either already in
the dep graph or are pure Rust stdlib/std-library equivalents.

| Need | Solution | Notes |
|------|----------|-------|
| KNN search | `sqlite-vec` (already linked via Phase 1 `vec0`) | UDFs registered via `conn.call()` + `create_scalar_function` |
| Scalar UDFs | `rusqlite::functions::FunctionFlags` (already a dep) | `SQLITE_DETERMINISTIC` flag available |
| File I/O for workspace.md | `tokio::fs` (already in dep graph via Tauri) | Async reads for the hot path; sync writes OK for rare regeneration |
| Markdown section parsing | None — hand-parse `<!-- AUTO-START -->` / `<!-- AUTO-END -->` markers | ~30 lines, zero dep cost |
| Score formula | Pure Rust closures passed to `create_scalar_function` | exp, f64 arithmetic |
| `json_each` in CONFLICTS query | Built-in SQLite JSON function (bundled rusqlite has it) | Verify: `rusqlite` bundled includes `-DSQLITE_ENABLE_JSON1` by default |

**UDF registration mechanism**: `tokio_rusqlite::Connection::call(|c| { c.create_scalar_function(...) })`. This is the
correct async wrapper around the sync rusqlite call. The UDFs must be registered
**after** the connection is opened and migrations are run — they do not persist across
connection opens (they are connection-scoped, not DB-file-scoped). For production, the
right place is a new `register_score_udfs(conn: &Connection)` called from
`MemoryStore::open()` and `MemoryStore::new_for_test()` after `run_migrations()`.

**`SQLITE_DETERMINISTIC` flag and `exp_decay`**: `exp_decay(age_secs, lambda)` takes
`strftime('%s','now')` from the query as the first argument — the non-deterministic
part (`now()`) is computed OUTSIDE the UDF, passed in as a bound parameter at query
build time. The UDF itself receives a concrete integer and is therefore pure. Marking it
`SQLITE_DETERMINISTIC` is correct and allows the SQLite query planner to cache it.

**FTS5**: doc 6 mentions FTS5 fallback when embedder is not ready. FTS5 was listed as a
deferred item in Phase 2 archive. Phase 3 archive explicitly states: embedder `NotReady`
→ worker drops the event (zero rows). The FTS5 fallback was NOT built in Phase 2 or 3.
Phase 4 must decide whether to build it now or accept the degraded path (return empty
results when embedder cell is empty). This is an Open Question (see Q3).

---

## Code Touchpoints (Current State)

### `src-tauri/src/commands/chat.rs` — `stream_chat` (lines 411–1133)

**`ChatRequest` construction**: happens at the top of the agentic loop (line 548), inside
`for iteration in 0..MAX_TOOL_ITERATIONS`. The exact block (lines 548–563):

```rust
let chat_request = {
    let conversations = state.conversations.lock()...;
    let history = conversations.get(&agent_id).cloned().unwrap_or_default();
    ChatRequest {
        model: String::new(),
        messages: history,
        system_prompt: Some(agent_system_prompt.clone()),   // ← this is the injection point
        custom_instructions: agent_custom_instructions.clone(),
        tools: tools.clone(),
        extensions: Default::default(),
    }
};
```

`agent_system_prompt` is a `String` captured from `agent.system_prompt` before the loop
(line 428). It is the output of `compose_system_prompt(name, role, efficient_mode, project_path)`.
It does NOT change between iterations — it is cloned from the pre-loop capture on every
iteration.

**Injection point**: Phase 4 must prepend the memory blocks to `agent_system_prompt`
(or to the `ChatRequest.system_prompt` value) at or before line 558. Two options:
- (a) Pre-compute memory blocks ONCE before the loop, prepend to `agent_system_prompt` clone.
- (b) Re-compute memory blocks on each iteration, build a fresh combined string per iteration.

Both options fit in the existing ChatRequest construction block.

**Agentic loop structure**: `for iteration in 0..MAX_TOOL_ITERATIONS` (max = 5, line 28).
Each iteration: OAuth check → build ChatRequest → circuit breaker check → retry loop →
call provider → handle result (Text: done; ToolCalls: execute tools, push to history,
loop back; Error: return). The `system_prompt` is rebuilt every iteration because
`ChatRequest` is constructed inside the loop. If Phase 4 pre-computes once before the
loop, it avoids N retrieval calls but uses the same blocks for all iterations of a single
user message.

**No existing benches** in `src-tauri/`. The `benches/` directory does not exist. Latency
budget validation will need to be done via `cargo test --lib` timing or manual inspection
rather than `cargo bench`.

### `src-tauri/src/models/prompt.rs` — `compose_system_prompt`

**Current shape**: pure function, signature:
```rust
pub fn compose_system_prompt(name: &str, role: &AgentRole, efficient_mode: bool, project_path: Option<&str>) -> String
```

Returns a `String` assembled from four `const` blocks:
`BASE_RULES + SELF_REPORT + "\n\n" + ANTI_SYCOPHANCY + "\n\n" + role.template(name)` + optional
project path + optional EFFICIENT_MODE. No async, no side effects.

**Phase 4 change**: `compose_system_prompt` itself stays untouched (it returns the "base
prompt"). Memory block assembly happens in a NEW `memory/prompt_builder.rs`. The
`stream_chat` call site prepends the builder output to the `compose_system_prompt` result.
This avoids touching `compose_system_prompt`'s signature and keeps its 10 tests green with
zero modification.

**Tests at risk**: 10 tests in `models/prompt.rs` all call `compose_system_prompt` and
assert on its output content. These are SAFE because Phase 4 does not modify the function.
Risk only emerges if someone changes the prompt format in the builder and tests start
asserting the full combined string.

### `src-tauri/src/state.rs` — `AppState`

Fields relevant to Phase 4 (all confirmed present):

| Field | Type | Notes |
|-------|------|-------|
| `base_dir` | `PathBuf` | `~/.claw-garden`; workspace path = `base_dir.join("workspaces/{agent_id}/{project_id}/workspace.md")` |
| `memory_store` | `Arc<MemoryStore>` | `MemoryStore` is `Clone` (channel handle); no Mutex needed |
| `embedder` | `Arc<OnceCell<Arc<dyn Embedder>>>` | May be `None` at boot; Phase 4 must tolerate `get() == None` |
| `memory_tx` | `UnboundedSender<MemoryEvent>` | Ingest only; Phase 4 reads from `memory_store`, not from this channel |

Missing for Phase 4: no `access_tracker` yet (Phase 5). Phase 4 does NOT need one — it
just returns the top-K hits; the access tracking fire-and-forget is a Phase 5 concern.
Phase 4 can stub a call site comment: `// Phase 5: AccessTracker.record(hit_ids)`.

### `src-tauri/src/memory/mod.rs`

Current public re-exports:
- `pub use db::MemoryStore`
- `pub use error::MemoryError`
- `pub use identity::derive_project_id`
- `pub use record::{EpisodicTrajectoryRow, MemoryRecord, MemoryRow, ProceduralSkillRow, Scope, TableKind}`

Phase 4 adds: `pub mod search`, `pub mod score`, `pub mod workspace`, `pub mod prompt_builder`.
No existing re-exports need to change.

### `src-tauri/src/memory/db.rs` — `MemoryStore`

**UDF registration**: currently there are NO UDFs registered. `MemoryStore::open()` calls
`run_migrations(conn)` and returns. The UDF registration needs to be added here (after
migrations) so every connection instance — production and test — has the score functions
available.

**`create_scalar_function` call pattern** (from doc 6):
```rust
conn.call(|c| {
    let det = FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC;
    c.create_scalar_function("exp_decay", 2, det, |ctx| { ... })?;
    ...
    Ok(())
}).await
```
This is the `tokio_rusqlite` pattern: wrap sync rusqlite calls in `.call()`. Already
used extensively in the codebase for CHECK constraint tests in `db.rs`.

### `src-tauri/src/memory/record.rs`

**`MemoryRow` fields** (confirmed present): all Phase 4-relevant fields are there:
`id`, `content`, `memory_type`, `tier`, `status`, `last_accessed_at`, `importance`,
`disputed_against: Option<String>`, `deleted_at`, `agent_id`.

**`MemoryRecord::disputed_against()`**: returns `Option<&str>` — raw JSON string. Confirmed
by `record.rs` line 42 and `mod.rs` doc comment: "Phase 1 — typed Vec<i64> accessor ships
Phase 5/6." Phase 4 does NOT need to decode it; `dispute_factor` UDF takes `status: &str`,
not the JSON array.

### `src-tauri/src/memory/migrations/` (V1–V3 SQL)

All Phase 4-required columns confirmed present in `V1__init.sql`:

| Column | Table | Present? | Default |
|--------|-------|----------|---------|
| `tier` | `memory` | YES | `3` |
| `importance` | `memory` | YES | `5.0` |
| `access_count` | `memory` | YES | `0` |
| `last_accessed_at` | `memory` | YES | (NOT NULL, set at insert) |
| `status` | `memory` | YES | `'active'` |
| `disputed_against` | `memory` | YES | `NULL` (TEXT, nullable) |
| `deleted_at` | `memory` | YES | `NULL` |
| `tier` | `episodic_trajectory` | YES | `3` |
| `tier` | `procedural_skill` | YES | `1` |
| `memory_vectors` | V2 | YES | `vec0(embedding float[384])` |
| `episodic_vectors` | V2 | YES | `vec0(embedding float[384])` |
| `procedural_vectors` | V2 | YES | `vec0(embedding float[384])` |

**Index gap (minor)**: doc 6 shows `idx_memory_agent_active ON memory(agent_id, status)`,
but the actual V1 SQL creates `idx_memory_agent_active ON memory(agent_id, last_accessed_at)`.
Both use `WHERE deleted_at IS NULL`. The actual index is fine for retrieval (`agent_id`
prefix is the filter; `last_accessed_at` is the sort key). No schema migration needed.

**`memory_vectors` is a vec0 virtual table**: confirmed in `V2__vectors.sql`. All three
virtual tables use `float[384]`, matching `multilingual-e5-small` dimensions.

### Memory module file inventory

| File | Role |
|------|------|
| `memory/mod.rs` | Public re-exports, module declarations |
| `memory/db.rs` | `MemoryStore` (open, in-memory for tests), `ensure_vec0_registered` |
| `memory/record.rs` | `MemoryRecord` trait + `MemoryRow`, `EpisodicTrajectoryRow`, `ProceduralSkillRow` |
| `memory/error.rs` | `MemoryError` enum (`Sqlite`, `Migration`, `Internal`) |
| `memory/migrations.rs` | `embed_migrations!` + `runner()` re-export |
| `memory/identity.rs` | `derive_project_id(path)` — `basename + "-" + hash4(path)` |
| `memory/embedder.rs` | `Embedder` trait + `FastEmbedLocal` + `FakeEmbedder` (cfg test) |
| `memory/model_download.rs` | Download + SHA-256 + `tokenizer_path(base_dir)` |
| `memory/events.rs` | `MemoryEvent`, `MemorySource` |
| `memory/worker.rs` | `memory_worker` async loop + `start_memory_worker` |
| `memory/store_insert.rs` | `insert_memory_with_vector`, `NewMemoryRecord`, `MemoryType`, `CreatedByType` |
| `memory/chunker/mod.rs` | `ChunkerKind`, `Chunk`, `select_chunker`, `ChunkerError` |
| `memory/chunker/chat.rs` | Turn-based chunker (480-token cap, speaker boundary) |
| `memory/chunker/code.rs` | AST chunker via tree-sitter (Rust/TS/Python + fallback) |
| `memory/chunker/token_splitter.rs` | Shared `TokenSplitter` + byte-length fallback |

**Phase 4 adds**: `memory/search.rs`, `memory/score.rs`, `memory/workspace.rs`,
`memory/prompt_builder.rs`.

### Workspace.md — current state

There is NO `workspace.md` (RAG workspace file) in the codebase yet. There IS a
`commands/workspace.rs` that handles `WorkspaceConfig` — a per-project JSON file at
`<project>/.claw-garden/config.json`. This is a DIFFERENT concept (provider/model config
overrides) and must NOT be confused with the RAG `agent_workspace.md`.

The RAG workspace file path (per doc 6):
`app_data_dir/workspaces/{agent_id}/{project_id_or_default}/workspace.md`

`state.base_dir` resolves to `~/.claw-garden`. So the full path:
`~/.claw-garden/workspaces/{agent_id}/{project_id_or_default}/workspace.md`

No marker parsing, no auto-section generation, no `WorkspaceManager` struct exists today.
Phase 4 builds it from scratch in `memory/workspace.rs`.

---

## Existing Schema Verification

| Column | Table | Present | Default | Notes |
|--------|-------|---------|---------|-------|
| `tier` | `memory` | YES | `3` | INTEGER NOT NULL |
| `importance` | `memory` | YES | `5.0` | REAL NOT NULL |
| `access_count` | `memory` | YES | `0` | INTEGER NOT NULL |
| `last_accessed_at` | `memory` | YES | — | INTEGER NOT NULL (set at insert, = created_at initially) |
| `status` | `memory` | YES | `'active'` | TEXT NOT NULL |
| `disputed_against` | `memory` | YES | NULL | TEXT (raw JSON) |
| `deleted_at` | `memory` | YES | NULL | INTEGER (soft delete) |
| `tier` | `episodic_trajectory` | YES | `3` | INTEGER NOT NULL |
| `tier` | `procedural_skill` | YES | `1` | INTEGER NOT NULL |
| `memory_vectors` (vec0) | V2 | YES | — | `float[384]` |
| `episodic_vectors` (vec0) | V2 | YES | — | `float[384]` |
| `procedural_vectors` (vec0) | V2 | YES | — | `float[384]` |
| UDFs registered | `db.rs` | NO | — | Must add in Phase 4 |
| `all_memories` view | V3 | YES | — | UNION ALL, `kind` discriminator |

**No schema migrations needed for Phase 4.** All required columns are present. The only
gap is UDF registration, which is a connection-init concern, not a schema concern.

---

## Resolved Decisions (with Rationale)

### D1 — Hot Path Latency Budget

**Evidence**: No existing bench suite (`benches/` dir absent). No timing instrumentation
in `stream_chat`. `MAX_TOOL_ITERATIONS = 5`. From doc 6: target is <50ms for
KNN + decay + workspace read at 100k vectors.

**What the code tells us**: `stream_chat` already has a circuit breaker + retry loop that
can add 60s+. The KNN query is a single `.call()` on `tokio_rusqlite::Connection` which
dispatches to a dedicated background thread — contention comes from the ingest worker
(Phase 3) which also uses `.call()` on the same connection. At 100k rows, sqlite-vec
KNN on a single table with `float[384]` takes ~5-20ms on a typical dev SSD (based on
sqlite-vec published benchmarks). Three parallel `try_join!` KNN queries (memory +
episodic + procedural) with independent connections on the same file can overlap.
Workspace file read is a single `tokio::fs::read_to_string` — <2ms on SSD.

**Recommendation**: Accept the <50ms budget without a bench for v1. Add a
`log::debug!("retrieval took {}ms", ...)` timing call in `search_all_memories` so latency
is observable in logs without a formal bench. If P99 exceeds 50ms in practice, the first
optimization lever is the `tier > 1` filter (reduces KNN index size) and reducing
`k_pool` from 30 to 20. Do NOT add a bench step to the Phase 4 apply flow — it would
require `cargo build` (prohibited).

### D2 — Per-Iteration vs First-Iteration Retrieval in Agentic Loop

**Evidence from code**: `ChatRequest` is constructed INSIDE the `for iteration in
0..MAX_TOOL_ITERATIONS` loop (line 548). The `system_prompt` value is
`agent_system_prompt.clone()` — it is re-cloned from the pre-loop captured String on
every iteration, so it is the SAME content on all iterations today.

**Evidence from doc 6 (line ~1050)**: doc 6 does not explicitly say "re-run retrieval per
iteration." The injection point described is "al construir el ChatRequest" — which is
inside the loop. However, the doc's criteria (`KNN + decay + workspace read debe ser <50ms`)
implies a per-request cost, not a per-session cost. The phrase "fresh each iteration to
reflect new memories" in the master plan is a goal, not an absolute requirement for v1.

**Tradeoff**:
- Per-iteration: retrieval runs 1-5× per user message. New tool results ingested during
  the agentic loop (by Phase 3's worker) COULD be reflected in subsequent iterations.
  BUT: the ingest worker is async and fire-and-forget — there is no guarantee a chunk
  is committed to SQLite before the next iteration's retrieval query runs. The race window
  is small (embedding + INSERT vs loop iteration time). Net benefit is minimal for v1.
- First-iteration only: retrieval runs once per user message. Simpler implementation,
  lower cost, same quality for 95% of interactions (most sessions have 1-2 tool calls).

**Recommendation**: **First-iteration only for v1.** Pre-compute memory blocks BEFORE
the agentic loop, clone the combined string into `agent_system_prompt` for the loop.
This avoids a 1-5× retrieval cost per user message and eliminates the ingest-race
concern. Phase 4 can add a `// TODO Phase X: re-run retrieval per iteration if agentic
context accumulates high-importance memories` comment at the call site.

### D3 — `disputed_against` Typed Decode

**Evidence**: `MemoryRecord::disputed_against()` returns `Option<&str>` (raw JSON).
`dispute_factor` UDF (Phase 4) takes `status: TEXT` — it does NOT read `disputed_against`.
The CONFLICTS query (doc 6 lines 655-663) uses `json_each(m_new.disputed_against)` which
is a SQLite built-in, not the Rust-side type. The Rust struct's `Option<String>` field is
sufficient to pass to the SQL query as a parameter.

**Recommendation**: **Defer typed decode entirely to Phase 6.** Phase 4's CONFLICTS block
query only needs `disputed_against` as a TEXT column in SQL — `json_each()` handles the
JSON parsing inside SQLite. No Rust-side `Vec<i64>` decode needed. Keep
`MemoryRecord::disputed_against()` as `Option<&str>`, keep `MemoryRow.disputed_against`
as `Option<String>`. Phase 6 adds the typed accessor when the contradiction detector needs
to build `disputed_against = JSON([ids])` on the Rust side.

### D4 — Workspace.md Cache Invalidation

**Evidence**: Phase 5's `AccessTracker` (which would emit tier-change events) does NOT
exist yet. The ingest worker (Phase 3) does NOT emit tier-change events — it inserts with
`tier = compute_tier(3, importance, 0, type)` at insert time (initial tier only).

**Current state**: No invalidation channel exists. No tier-promotion path has been built.

**Problem**: doc 6 says workspace.md regenerates "at tier changes (eager)" — but Phase 4
cannot trigger on tier changes that don't exist yet (Phase 5 is what changes tiers at
access time).

**Recommendation**: Phase 4 delivers a **simpler, Phase 5-compatible shape**:
1. `WorkspaceManager::regenerate_auto_section()` is a standalone async function that
   queries the DB and rewrites the AUTO section.
2. Phase 4 calls it **once per `stream_chat` invocation**, before the agentic loop,
   only if the file is stale (use `last_modified` timestamp of the file vs. a
   simple `dirty` flag on `AppState` OR just always regenerate — at <2ms per call it's
   cheap enough).
3. Phase 5 adds `access_tracker: Arc<AccessTracker>` to AppState and calls
   `regenerate_auto_section()` from within the access tracker flush when any tier changes.
   Phase 4's `WorkspaceManager` is designed to be called from multiple sites — no
   interior state coupling.

This avoids painting Phase 5 into a corner: Phase 5 just calls the same function.
The "always regenerate at chat start" strategy is slightly wasteful (one DB query per
chat) but safe and correct for v1. When Phase 5 ships, the per-chat regeneration can
become conditional on a `dirty` flag.

### D5 — Filter `m.tier > 1` in Retrieval

**Evidence**: doc 6 (lines ~1075-1076): "El retrieval YA NO trae memorias Tier 1. Se
filtran del KNN con `WHERE m.tier > 1`." Doc 6 workspace section: Tier 1 memories are
listed in the AUTO section of workspace.md (all `user_preference` with importance >= 8,
all `architectural_decision` with importance >= 8, all procedural skills).

**Gap analysis**: Phase 3 inserts `chat_message` rows with `tier = 3` and `tool_error`
rows with `tier = 3`. No Phase 3 rows are ever Tier 1. The insert logic in
`store_insert.rs` hardcodes `tier = 3` for chat/tool types (per doc 6 tier assignment
rules — only `user_preference` and `architectural_decision` start at tier 1 or 2). So
for Phase 4's test dataset (all Phase 3 rows), the `WHERE tier > 1` filter would exclude
zero rows.

**But**: procedural_skill rows are ALWAYS tier 1 (per `V1__init.sql` default `tier = 1`)
and are included in workspace auto-section. So the filter is correct conceptually and
safe to implement now even if it hits nothing in Phase 4's test data.

**Recommendation**: Implement `WHERE m.tier > 1` filter in the KNN query as specified.
It is architecturally necessary (prevents Tier 1 from consuming retrieval budget),
correct for future data, and costs nothing for Phase 4's test data.

---

## Open Questions for Propose

**Q1 — UDF registration timing**: Should UDFs be registered in `MemoryStore::open()` /
`MemoryStore::new_for_test()` (always available, no caller coordination needed), or should
they be registered lazily in `search.rs` before the first search query? The always-register
approach is simpler and matches the vec0 registration pattern. Proposed answer: register in
`MemoryStore::open()` — but needs user confirmation because it adds a `conn.call()` await
to the constructor.

**Q2 — `dispute_factor` stub or full impl?** Phase 4 is supposed to stub `dispute_factor`
(always returns 1.0) and Phase 6 implements it with real values. But doc 6 already shows
the full implementation (1.0 / 0.6 / 0.0). Since the logic is 3 lines and has no
dependency on Phase 6's contradiction detector, it can be fully implemented in Phase 4
at zero additional cost. The "stub" only makes sense if the values were TBD, but they are
not. Proposed answer: implement fully in Phase 4, saving Phase 6 a task.

**Q3 — FTS5 fallback**: When `embedder.get() == None` at retrieval time (cold-start or
bootstrap not yet complete), what does `search_all_memories` return? Options:
- (a) Return empty `Vec<RankedHit>` immediately — safe, graceful, no memory blocks in
  system prompt. Simple.
- (b) FTS5 fallback — keyword search over `content` column. Better UX but was deferred
  in Phase 2 and Phase 3. Adds non-trivial implementation scope.
Recommended: (a) for v1. FTS5 fallback deferred. Phase 4 logs a `warn!` when embedder
is None and returns empty results.

**Q4 — CONFLICTS block in Phase 4 vs Phase 6**: doc 6 says CONFLICTS block is Phase 6's
responsibility (the sycophancy guard). But `prompt_builder.rs` is built in Phase 4.
Should Phase 4 build a stub CONFLICTS block (always empty) or skip it entirely and let
Phase 6 add it? Proposed: Phase 4 builds the block conditionally but only queries
`WHERE status = 'disputed'` — if the table has zero disputed rows (guaranteed for Phase
4's data), the block is simply omitted. Phase 6 adds the full CONFLICTS pair query.
This avoids leaving a half-built `prompt_builder` for Phase 6 to touch.

**Q5 — Workspace path when `project_id` is None**: for agents without a `project_path`
(Phase 3 D3 — `scope='agent'`, `project_id=NULL`), the workspace path becomes
`~/.claw-garden/workspaces/{agent_id}/default/workspace.md`. The `default` literal needs
to be defined somewhere. Needs confirmation in propose.

---

## Risks (R-NEW-1 ... R-NEW-8)

**R-NEW-1 — Hot path KNN latency spike on cold SSD / HDD**: sqlite-vec KNN on 100k
float[384] takes 5-20ms on SSD. On an HDD (some Windows dev machines), this may exceed
50ms. Since there is no bench suite to validate, the latency budget is assumed, not
measured. Mitigation: log retrieval time at `debug` level; if production reports lag,
first lever is `k_pool` reduction from 30→10 and adding `AND tier > 1` to the KNN query
to reduce the index scan.

**R-NEW-2 — Race between ingest worker and retrieval**: Phase 3's `memory_worker` is
async and fire-and-forget. A message sent via `push_and_persist` may not be committed to
SQLite before `stream_chat` kicks off the retrieval. This is expected and acceptable for
v1 — the just-ingested message for the current turn will simply not appear in the
retrieval for the same turn. The race is non-blocking (retrieval is read-only, ingest is
write-only; tokio_rusqlite serializes them on its background thread). No data corruption
risk.

**R-NEW-3 — Embedder cold-start during chat**: if the embedder cell is empty (download
not yet complete), Phase 4 must gracefully return empty results rather than panicking.
The `OnceCell::get()` pattern already exists in the worker. Phase 4 must replicate the
same `if let Some(embedder) = state.embedder.get() { ... } else { return Ok(vec![]) }`
pattern in `search_all_memories`. Failure mode: no memory blocks in system prompt until
embedder is ready. Acceptable for v1.

**R-NEW-4 — Regression: `prompt.rs` tests assert content of `compose_system_prompt`**:
10 tests in `models/prompt.rs` assert on the exact content of `compose_system_prompt`'s
output (contains "AgentOffice", "TOKEN EFFICIENCY MODE", "Response Integrity", etc.).
Phase 4 does NOT modify `compose_system_prompt`. Risk is zero IF the memory blocks are
prepended in `stream_chat` without touching `prompt.rs`. The call site must be
`memory_prefix + base_prompt`, not `compose_system_prompt(... + memory_prefix ...)`.

**R-NEW-5 — `tokio_rusqlite` write contention**: the ingest worker and the retrieval
query both use the same `MemoryStore.conn` (same tokio_rusqlite background thread).
sqlite-vec KNN on `memory_vectors` is a read, but `tokio_rusqlite` serializes ALL calls
on the background thread. A long-running INSERT (with embedding serialization) from the
ingest worker could block a retrieval call. At Phase 4 scales (hundreds of rows) this
is sub-ms. At Phase 5+ scales with batch flushes, the `AccessTracker` UPDATE could
block retrieval for 10-30ms. Mitigation: Phase 5's AccessTracker async batching already
accounts for this (fire-and-forget, not on the retrieval hot path).

**R-NEW-6 — Workspace.md file corruption on concurrent writes**: if two agents share
the same `(agent_id, project_id)` path (unlikely but possible in the current single-user
model), concurrent `regenerate_auto_section()` calls could corrupt the file. Mitigation:
use `tokio::fs::write()` which is atomic on most OS (rename-on-write pattern). For Phase
4 with a single-agent test setup, this is not a real risk.

**R-NEW-7 — `json_each()` on NULL `disputed_against`**: if a row has
`disputed_against = NULL`, `json_each(NULL)` returns zero rows in SQLite (no error).
The CONFLICTS query using `json_each(m_new.disputed_against)` is safe for NULL. Verified
by SQLite docs. No guard needed.

**R-NEW-8 — `importance` UDF input clamping**: `importance_factor(importance)` applies
`0.5 + (importance / 20.0)`. If `importance` is out of the `[1.0, 10.0]` range (possible
if a caller sets it incorrectly), the UDF silently returns a value outside `[0.55, 1.0]`.
Phase 4 should either clamp in the UDF or rely on the DB CHECK constraint — but there is
NO CHECK constraint on `importance` range in the current schema. The UDF should clamp:
`let imp = imp.clamp(1.0_f64, 10.0);` before the formula. Adds one line.

---

## Test Baseline

**Total Rust tests at Phase 4 start**: 315/315 PASS.

**Files where regression risk is highest**:

| File | Why at Risk | Test count (approx.) |
|------|-------------|---------------------|
| `src-tauri/src/commands/chat.rs` | Phase 4 modifies `stream_chat` hot path; 4 tests in `#[cfg(test)]` module | 4 |
| `src-tauri/src/models/prompt.rs` | 10 tests assert on `compose_system_prompt` output; safe IF function is not modified | 10 |
| `src-tauri/src/memory/db.rs` | UDF registration added to `MemoryStore::open()` — must not break existing 15+ schema tests | 15+ |
| `src-tauri/src/state.rs` | 8 tests on AppState construction, embedder cell, memory_tx | 8 |
| `src-tauri/src/memory/worker.rs` | Worker tests depend on `MemoryStore` being functional; if UDF registration breaks the store, worker tests fail | ~10 |

**New test coverage required** (per TDD mandate):
- `memory/score.rs`: unit tests for each UDF (exp_decay, lambda_for_type, importance_factor, dispute_factor)
- `memory/search.rs`: search with `FakeEmbedder` + in-memory store; tier filter; empty result graceful
- `memory/workspace.rs`: marker parsing; AUTO section regeneration; USER section preserved byte-for-byte
- `memory/prompt_builder.rs`: four-block assembly; correct separators (`===`); CONFLICTS block absent when no disputes
- `commands/chat.rs` integration: system_prompt prepended with memory blocks; graceful empty when embedder not ready

**Separator format** (from doc 6 lines ~1057-1073):
```
=== WORKSPACE ===
[content]

=== RECENT CONTEXT (Last 7 Days) ===
[content]

=== LONG-TERM KNOWLEDGE ===
[content]

=== ACTIVE CONFLICTS (review needed) ===  ← only when disputed rows exist
[content]
```

**`importance_factor` and `lambda_for_type`** are pure functions on column values only —
no external state needed. They can be tested as standalone Rust functions (called via UDF
registration + a test query) or as plain `fn` equivalents in a test module.

**`SearchScope` variants** (from doc 6 lines ~848-860): struct with three optional `usize`
fields: `memory: Option<usize>`, `episodic: Option<usize>`, `procedural: Option<usize>`.
Named constructors: `all_default()`, `procedural_only()`, `factual_pivots()`. Not an enum.

---

## Summary of Phase 4 New Files

| File | What it delivers |
|------|-----------------|
| `memory/score.rs` | `register_score_udfs(conn)` — four UDFs + tests |
| `memory/search.rs` | `SearchScope`, `RankedHit`, `search_all_memories()`, `knn_memory()`, `knn_episodic()`, `knn_procedural()` |
| `memory/workspace.rs` | `WorkspaceManager`, `regenerate_auto_section()`, marker-aware read/write |
| `memory/prompt_builder.rs` | `build_memory_prompt(agent_id, project_id, store, workspace_manager)` → `String` |

Modified files: `memory/db.rs` (add UDF registration call), `memory/mod.rs` (add 4 new `pub mod`),
`commands/chat.rs` (prepend memory blocks to `system_prompt` before the agentic loop).
