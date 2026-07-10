# Phase 4 — rag-retrieval — Design

> **Inputs**: `proposal.md` (9 LOCKED decisions D1–D9) + `explore.md` (550 lines, 8 R-NEW + code touchpoints) + `docs/6-improvements-RAG.md` (canonical RAG, lines 463–1166) + `docs/sdd/rag-ingest/design.md` (Phase 3 patterns).
>
> **Status**: every signature, type, integration point and risk has been re-verified against the live source tree (`commands/chat.rs`, `state.rs`, `memory/db.rs`, `memory/record.rs`, `memory/embedder.rs`, `memory/mod.rs`, `models/prompt.rs`). Where Phase 3's design uses `tracing::`, Phase 4 honors the project rule and uses `log::` exclusively (43 call sites in the codebase, `tracing::` is NOT in the dep graph).
>
> **Carry-forward Phase 3 patterns**: tuple-return ctor for `(Self, Receiver)`, `Option<Arc<dyn Trait>>` cell guard, atomic transaction pattern, `#[cfg(test)] pub(crate)` for cross-file mock reuse, compile-time `Send + Sync` assertion via `const _: fn()`.
>
> **What this design DOES NOT introduce**: new dependencies, schema migrations, modifications to `compose_system_prompt`, modifications to `MemoryRecord` / `MemoryRow`, modifications to the agentic loop body, or any decision beyond D1–D9. Any ambiguity surfaced during code re-verification is listed in §11 "Design questions for orchestrator".

---

## 1. Lifecycle diagram (ASCII)

Single `stream_chat` invocation, from user message arrival to first `ChatRequest` construction. Memory prefix is computed ONCE before the agentic loop (D2); the same combined system prompt seeds all `MAX_TOOL_ITERATIONS = 5` iterations.

```text
┌──────────────────────────────────────────────────────────────────────────────┐
│ Frontend → stream_chat(agent_id, message, on_event, state)                   │
│                                                                              │
│ commands/chat.rs:411                                                         │
│   (1) capture (agent_system_prompt, agent_custom_instructions,               │
│                agent_provider_id, agent_project_path, agent_hooks) under     │
│                state.agents lock (line 421-436) — UNCHANGED                  │
│                                                                              │
│   (2) push_and_persist user message (line 448-460) — UNCHANGED               │
│       └── async: state.memory_tx.send(MemoryEvent {...})                     │
│                  Phase 3 worker picks it up; commit is NOT awaited.          │
│                  → "ingest-vs-retrieval race" is intentional (R-NEW-2).      │
│                                                                              │
│   (3) [Phase 5 will sit here, BEFORE retrieval — workspace regen on dirty]   │
│                                                                              │
│   (4) ── NEW Phase 4 — pre-loop memory pre-computation ──                    │
│       project_id_opt = agent_project_path.as_deref().and_then(|p|            │
│           if p.is_empty() { None } else                                      │
│           { Some(derive_project_id(Path::new(p))) });                        │
│                                                                              │
│       workspace_mgr = WorkspaceManager::new(state.base_dir.join("workspaces"))│
│                                                                              │
│       // D4 — unconditional regen of AUTO section (cheap, ~2-5ms)            │
│       let regen_t0 = Instant::now();                                         │
│       if let Err(e) = workspace_mgr                                          │
│           .regenerate_auto_section(&agent_id, project_id_opt.as_deref(),     │
│                                    &state.memory_store).await                │
│       { log::warn!("workspace regen failed: {e}"); }                         │
│       log::debug!("workspace regen took {}ms", regen_t0.elapsed().as_millis())│
│                                                                              │
│       // build full 4-block prefix (workspace + recent + longterm + conflicts)│
│       let build_t0 = Instant::now();                                         │
│       let embedder_opt = state.embedder.get().cloned();                      │
│       let memory_prefix = prompt_builder::build_memory_prompt(               │
│           &state.memory_store,                                               │
│           &workspace_mgr,                                                    │
│           embedder_opt,                                                      │
│           &agent_id,                                                         │
│           project_id_opt.as_deref(),                                         │
│           &message_text_for_query,  // see §6 "user_query source"            │
│       ).await.unwrap_or_default();                                           │
│       log::debug!(                                                           │
│           "memory prefix build took {}ms, len={}",                           │
│           build_t0.elapsed().as_millis(), memory_prefix.len()                │
│       );                                                                     │
│                                                                              │
│       let combined_system_prompt = if memory_prefix.is_empty() {             │
│           agent_system_prompt.clone()                                        │
│       } else {                                                               │
│           format!("{}\n\n{}", memory_prefix, agent_system_prompt)            │
│       };                                                                     │
└──────────────────────────────────────────────────────────────────────────────┘
                                       │
                                       ▼
┌──────────────────────────────────────────────────────────────────────────────┐
│ for iteration in 0..MAX_TOOL_ITERATIONS  (line 539 — UNCHANGED structure)    │
│                                                                              │
│   // line 548-563 — ChatRequest construction (MODIFIED on line 558)          │
│   let chat_request = {                                                       │
│       let conversations = state.conversations.lock()…;                       │
│       let history = conversations.get(&agent_id).cloned().unwrap_or_default()│
│       ChatRequest {                                                          │
│           model: String::new(),                                              │
│           messages: history,                                                 │
│           system_prompt: Some(combined_system_prompt.clone()),  // ← CHANGED │
│           custom_instructions: agent_custom_instructions.clone(),            │
│           tools: tools.clone(),                                              │
│           extensions: Default::default(),                                    │
│       }                                                                      │
│   };                                                                         │
│                                                                              │
│   // … circuit breaker, retry loop, provider.chat_stream — UNCHANGED …       │
└──────────────────────────────────────────────────────────────────────────────┘
                                       │
                                       ▼
                  (loop body unchanged — same memory_prefix
                   used for all 5 possible iterations per D2)
```

### Where memory blocks come from

```text
build_memory_prompt(store, workspace_mgr, embedder_opt, agent_id, project_id_opt, user_query)
        │
        ├─→ workspace_mgr.read(agent_id, project_id_opt).await
        │       └── tokio::fs::read_to_string("…/workspaces/{agent_id}/{project_id|default}/workspace.md")
        │       └── on NotFound → Ok(String::new()) (defensive)
        │
        ├─→ if let Some(embedder) = embedder_opt:
        │       ├── let q_vec = embedder.embed(user_query).await?
        │       └── search::search_all_memories(
        │               store, &q_vec, agent_id,
        │               SearchScope::all_default(),  // (Some(30), Some(5), Some(5))
        │               k_final = 10,                // see §3 "constants"
        │           ).await?
        │           ├── try_join!(knn_memory, knn_episodic, knn_procedural)
        │           │       ↳ each runs SEARCH_SQL variant with the table-specific lambda
        │           │       ↳ filters: status IN ('active','disputed'), tier > 1,
        │           │                  deleted_at IS NULL, agent_id = ?
        │           ├── merge + sort DESC by score (per doc 6:892-898)
        │           └── truncate to k_final
        │       (no embedder → log::warn!, hits = vec![])
        │
        ├─→ fetch_active_conflicts(store, agent_id).await
        │       └── SQL: JOIN m_new + m_old via json_each(m_new.disputed_against)
        │              WHERE m_new.status='disputed' (per doc 6:654-663)
        │       └── on empty → Vec::new()
        │
        └─→ ASSEMBLE in order (D8):
                if !workspace_content.is_empty():  "=== WORKSPACE ===\n{content}\n"
                if !recent_hits.is_empty():        "\n=== RECENT CONTEXT (Last 7 Days) ===\n…"
                if !longterm_hits.is_empty():      "\n=== LONG-TERM KNOWLEDGE ===\n…"
                if !conflicts.is_empty():          "\n=== ACTIVE CONFLICTS (review needed) ===\n…"
            return String (may be "")
```

### Workspace.md regen vs retrieval ordering

The lifecycle runs them **sequentially** (regen first, then build) — NOT in parallel via `tokio::join!`. Reasons:

1. `regenerate_auto_section` writes to disk; `build_memory_prompt` reads the same file. A parallel race would either re-read the old content or hit a half-written file. Sequencing eliminates the race trivially.
2. Both operations share the same `tokio_rusqlite::Connection` background thread — they would serialize at the SQLite layer anyway. The "parallel" version saves zero wall time but adds two `.await` join points.
3. Combined cost is dominated by KNN (5-20ms on SSD per doc 6 estimate). Workspace regen is a single SELECT + one file write (~2-5ms). Sequencing keeps the design auditable.

### Failure branches (annotated)

| Branch | Reached when | Handling |
|--------|--------------|----------|
| `state.embedder.get() == None` | Phase 2 bootstrap still downloading | `embedder_opt = None` → `search_all_memories` skipped → only WORKSPACE + CONFLICTS blocks emitted. `log::warn!("embedder not ready; retrieval skipped")` inside `build_memory_prompt`. |
| `embedder.embed(query).await → Err` | ORT inference failure, worker gone | `log::warn!` + `hits = vec![]`. WORKSPACE + CONFLICTS still emitted. |
| `search_all_memories → Err` | tokio_rusqlite background thread closed, SQLite IO error | `log::warn!` + `hits = vec![]`. WORKSPACE + CONFLICTS still emitted. NEVER propagates to caller. |
| `workspace_mgr.regenerate_auto_section → Err` | Disk full, permission denied | `log::warn!` at call site. Chat proceeds. `read()` may still return stale content (existing file). |
| `workspace_mgr.read → Err` other than NotFound | Permission denied, IO error | `log::warn!` + treat as empty string. Other blocks still emitted. |
| `fetch_active_conflicts → Err` | SQLite IO error | `log::warn!` + treat as empty Vec. CONFLICTS block omitted; other blocks unaffected. |
| `build_memory_prompt → Err` (any unrecovered) | Defensive — should never happen because every internal branch swallows | `.unwrap_or_default()` at the `stream_chat` call site → empty prefix → `agent_system_prompt` used unchanged. Chat ALWAYS proceeds. |

The core invariant: **retrieval failures never break chat**. Every recoverable error is downgraded to "empty contribution to the prefix" at the lowest practical layer, and the call site additionally wraps the final `Result` in `.unwrap_or_default()` (R-NEW-10 mitigation).

---

## 2. Module structure

Four new files under `src-tauri/src/memory/`. All `pub(crate)`. None re-exported from `memory/mod.rs` in Phase 4 (no cross-module consumer outside `commands/chat.rs::stream_chat`, which uses fully-qualified paths).

```
src-tauri/src/memory/
├── score.rs                NEW — register_score_udfs + 4 UDF closures
├── search.rs               NEW — SearchScope, RankedHit, search_all_memories + 3 knn_* helpers
├── workspace.rs            NEW — WorkspaceManager, marker constants, AUTO/USER section discipline
├── prompt_builder.rs       NEW — build_memory_prompt + fetch_active_conflicts + separator constants
├── mod.rs                  MODIFIED — add 4 `pub(crate) mod` declarations, NO re-exports
├── db.rs                   MODIFIED — open() + new_for_test() call score::register_score_udfs after migrations
└── (other files UNCHANGED)
```

### 2.1 `memory/score.rs` — UDF registration

**Public surface** (one function, deliberately small):

```rust
//! Registers the four SQLITE_DETERMINISTIC scalar UDFs used by the score formula.
//! Called from MemoryStore::open() and MemoryStore::new_for_test() (D6).

use crate::memory::MemoryError;

/// Registers `exp_decay`, `lambda_for_type`, `importance_factor`, `dispute_factor`
/// on the connection. ALL four UDFs share `SQLITE_UTF8 | SQLITE_DETERMINISTIC` flags.
///
/// Idempotent at the call-site level: only called from `MemoryStore::open()` and
/// `MemoryStore::new_for_test()`, both of which open a fresh connection. UDFs are
/// connection-scoped (NOT process-global like `sqlite3_auto_extension` / vec0).
pub(crate) async fn register_score_udfs(
    conn: &tokio_rusqlite::Connection,
) -> Result<(), MemoryError>;
```

**Internal structure**:

- One `.call()` closure on the connection. Inside, sequentially calls `c.create_scalar_function(...)` four times. All four close-over `det = FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC`. Returns `Ok(())` on success; any `?` propagates as `tokio_rusqlite::Error` → mapped to `MemoryError::Sqlite` via the existing `From` impl (Phase 1).
- UDF #1 — `exp_decay(age_secs INTEGER, lambda REAL) -> REAL`. Body: `let age_days = age_secs as f64 / 86_400.0; Ok((-lambda * age_days).exp())`. Reference: doc 6:754-758.
- UDF #2 — `lambda_for_type(memory_type TEXT) -> REAL`. Match-arms verbatim from doc 6:760-771: `user_preference → 0.0019`, `architectural_decision → 0.0039`, `session_summary → 0.0077`, `semantic_fact → 0.0039`, `chat_message → 0.023`, `tool_error → 0.099`, fallback → `0.023`.
- UDF #3 — `importance_factor(importance REAL) -> REAL`. Body: `let imp = imp.clamp(1.0_f64, 10.0); Ok(0.5 + (imp / 20.0))`. The `clamp` is the R-NEW-8 mitigation (per proposal §11).
- UDF #4 — `dispute_factor(status TEXT) -> REAL`. Match arms verbatim from doc 6:778-785: `active → 1.0`, `disputed → 0.6`, fallback → `0.0`. Per D7 this is the FULL implementation, NOT a stub.

**Dependencies on other memory modules**: only `crate::memory::MemoryError`.

**Concurrency story**: The closure handed to `conn.call()` runs on the tokio_rusqlite background thread. `create_scalar_function` takes a `Fn` closure that becomes part of the connection's function table — that closure is invoked by SQLite's query engine on the same background thread (since `tokio_rusqlite::Connection` serializes all access through one channel). No extra `Send + Sync` proof needed for the closures themselves; the surrounding `register_score_udfs` future is `Send` because it captures only `&Connection`.

**Construction / destruction**:
- **Construction**: called inside `MemoryStore::open()` and `MemoryStore::new_for_test()` AFTER `run_migrations(&conn).await?`, BEFORE returning the store. New code lives between lines 59 and 60 (`open`) and between lines 74 and 75 (`new_for_test`) of `memory/db.rs`.
- **Destruction**: UDFs are tied to the connection lifetime. When `MemoryStore` is dropped (i.e. the underlying `tokio_rusqlite::Connection` and its background thread shut down), the UDF table is freed along with it. No explicit teardown.

### 2.2 `memory/search.rs` — KNN orchestrator

**Public surface**:

```rust
//! KNN search across memory + episodic + procedural via tokio::try_join!,
//! merged + ranked by the score formula computed in SQL (doc 6:463-525).

use std::time::Instant;

use crate::memory::{MemoryError, MemoryStore, TableKind};

/// Per-table k_pool selection. `None` means "skip this table entirely".
/// Constructors are named after canonical call sites (doc 6:849-861).
#[derive(Debug, Clone, Copy)]
pub(crate) struct SearchScope {
    pub memory:     Option<usize>,
    pub episodic:   Option<usize>,
    pub procedural: Option<usize>,
}

impl SearchScope {
    /// Default Phase 4 scope — (Some(30), Some(5), Some(5)) per doc 6:850.
    pub(crate) fn all_default() -> Self { Self { memory: Some(30), episodic: Some(5), procedural: Some(5) } }
    /// "how do I X?" queries — (None, None, Some(10)) per doc 6:853.
    pub(crate) fn procedural_only() -> Self { Self { memory: None, episodic: None, procedural: Some(10) } }
    /// Phase 6 contradiction detector — (Some(5), None, Some(5)) per doc 6:858.
    pub(crate) fn factual_pivots() -> Self { Self { memory: Some(5), episodic: None, procedural: Some(5) } }
}

/// One result row after the score formula has been applied SQL-side.
#[derive(Debug, Clone)]
pub(crate) struct RankedHit {
    pub id: i64,
    pub kind: TableKind,        // existing enum from memory/record.rs
    pub content: String,
    pub memory_type: String,
    pub tier: i64,
    pub status: String,
    pub score: f64,
    pub distance: f64,          // raw KNN distance — exposed for Phase 6 contradiction detector
    pub created_at: i64,        // for RECENT vs LONG-TERM bucketing in prompt_builder
}

/// Top-level KNN orchestrator. Returns up to `k_final` hits sorted DESC by score.
///
/// Logs `log::debug!("retrieval={ms}ms hits={n} embed_dim={d}")` after the join
/// (D1 instrumentation; no benches in Phase 4).
pub(crate) async fn search_all_memories(
    store: &MemoryStore,
    embedding: &[f32],
    agent_id: &str,
    scope: SearchScope,
    k_final: usize,
) -> Result<Vec<RankedHit>, MemoryError>;
```

**Internal structure** (private helpers):

- `async fn knn_memory(store, embedding, agent_id, k) -> Result<Vec<RankedHit>, MemoryError>`. Runs `SEARCH_SQL_MEMORY` (constant defined below). Score formula uses polymorphic `lambda_for_type(memory_type)` per doc 6:489-499.
- `async fn knn_episodic(store, embedding, agent_id, k) -> Result<Vec<RankedHit>, MemoryError>`. Runs `SEARCH_SQL_EPISODIC`. Hardcodes `exp_decay(strftime('%s','now') - last_accessed_at, 0.0077)` per doc 6:502 + 907.
- `async fn knn_procedural(store, embedding, agent_id, k) -> Result<Vec<RankedHit>, MemoryError>`. Runs `SEARCH_SQL_PROCEDURAL`. Hardcodes `exp_decay(..., 0.0039)` per doc 6:502 + 908.
- `fn serialize_f32_query(v: &[f32]) -> Vec<u8>`. LE-encodes the query vector for the vec0 `MATCH` operator. Mirrors `serialize_f32_vec` already present in `memory/db.rs::tests` and `memory/store_insert.rs` — Phase 4 may inline it locally or pull a shared helper into `memory/mod.rs`. Design decision: **inline a private copy in `search.rs`** to avoid touching `mod.rs`'s public surface in this phase.

**SQL constants** (private to the module):

- `SEARCH_SQL_MEMORY` — adapts doc 6:793-819. Verbatim WHERE clauses: `WHERE m.status IN ('active', 'disputed') AND m.deleted_at IS NULL AND m.agent_id = ?3 AND m.tier > 1` (the last AND is the D5 addition not in doc 6). Selects `m.id, m.content, m.memory_type, m.tier, m.status, knn.distance, m.created_at, <score-expr> AS score`. ORDER BY score DESC LIMIT ?4. Bind params: `?1 = query bytes`, `?2 = k_pool`, `?3 = agent_id`, `?4 = k_pool` (LIMIT same as `k`).
- `SEARCH_SQL_EPISODIC` — same shape but JOINs `episodic_trajectory` ON `episodic_vectors.rowid = epi.id`. Score formula uses hardcoded `0.0077` lambda. Filters: `WHERE epi.status IN ('active','disputed') AND epi.deleted_at IS NULL AND epi.agent_id = ?3 AND epi.tier > 1`.
- `SEARCH_SQL_PROCEDURAL` — same shape but JOINs `procedural_skill` ON `procedural_vectors.rowid = pro.id`. Score formula uses hardcoded `0.0039` lambda. Filters: `WHERE pro.status IN ('active','disputed') AND pro.deleted_at IS NULL AND pro.agent_id = ?3 AND pro.tier > 1`.

> All three queries `SELECT` an additional literal column to populate `RankedHit.kind`. Either use a SQL `'memory'`/`'episodic'`/`'procedural'` literal and parse Rust-side, or hardcode the `TableKind` after the row map (cleaner). Design: **hardcode `TableKind` in the row mapper of each helper** — saves one column over the wire.

**Score formula reference** (from doc 6:467-472, applied inline in each `SEARCH_SQL_*`):

```sql
(1.0 - knn.distance)
  * MAX(
        CASE m.tier WHEN 1 THEN 0.5 WHEN 2 THEN 0.2 ELSE 0.05 END,
        exp_decay(
            strftime('%s','now') - m.last_accessed_at,
            lambda_for_type(m.memory_type)   -- or hardcoded 0.0077 / 0.0039
        )
    )
  * importance_factor(m.importance)
  * dispute_factor(m.status)
AS score
```

`tier_floor` is INLINE in `CASE`, not hidden in a UDF (per doc 6:833 — "visible y debuggeable directamente desde la query"). The `tier > 1` filter (D5) means the `CASE m.tier WHEN 1 THEN 0.5` arm is unreachable in Phase 4, but kept for parity with doc 6 and for the day Phase 5 lazy-mode allows Tier 1 in retrieval as a fallback.

**Top-level body of `search_all_memories`**:

```rust
let t0 = Instant::now();
let q_bytes = serialize_f32_query(embedding);
let (mem, epi, pro) = tokio::try_join!(
    async {
        match scope.memory {
            Some(k) => knn_memory(store, &q_bytes, agent_id, k).await,
            None => Ok(Vec::new()),
        }
    },
    async {
        match scope.episodic {
            Some(k) => knn_episodic(store, &q_bytes, agent_id, k).await,
            None => Ok(Vec::new()),
        }
    },
    async {
        match scope.procedural {
            Some(k) => knn_procedural(store, &q_bytes, agent_id, k).await,
            None => Ok(Vec::new()),
        }
    },
)?;

let mut all: Vec<RankedHit> = mem.into_iter().chain(epi).chain(pro).collect();
// total_cmp avoids NaN panic (doc 6:896 used unwrap; we harden).
all.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
all.truncate(k_final);

log::debug!(
    "retrieval={}ms hits={} pool=({mem}+{epi}+{pro}) k_final={k_final}",
    t0.elapsed().as_millis(),
    all.len(),
    mem = scope.memory.unwrap_or(0),
    epi = scope.episodic.unwrap_or(0),
    pro = scope.procedural.unwrap_or(0),
    k_final = k_final,
);
Ok(all)
```

Note: each `knn_*` helper takes the **already-serialized** `&[u8]` query so the LE encoding happens once. Inside each helper, the bytes are cloned into the `tokio_rusqlite` closure via `move` (the closure must own all captures because `Connection::call` requires `'static`).

**Dependencies on other memory modules**: `MemoryStore` (db.rs), `MemoryError` (error.rs), `TableKind` (record.rs).

**Concurrency story**:

- `tokio::try_join!` runs the three async branches concurrently on the tokio runtime. Each branch awaits a `Connection::call(...)` future.
- All three `Connection::call`s land on the **same** `tokio_rusqlite::Connection` background thread — they are serialized at the SQLite layer (per Phase 3 R-NEW-5 + this design's §5). True concurrent KNN across tables is therefore impossible without a second connection. **Acceptable**: doc 6:892-898 also uses `tokio::try_join!` with no concurrency claim — the practical wins are cancellation safety (if one fails, the others abort) and code readability, NOT wall-time parallelism.
- The closures inside `Connection::call` are `'static` and `Send` (they own `Vec<u8>`, `String`, `usize`).

**Send + Sync proof** (compile-time, in `search.rs`):

```rust
const _: fn() = || {
    fn _send_sync<T: Send + Sync>() {}
    _send_sync::<SearchScope>();
    _send_sync::<RankedHit>();
};
```

**Where constructed / destructed**:

- `SearchScope` — short-lived value type, constructed at the `build_memory_prompt` call site (`SearchScope::all_default()`) and dropped after `search_all_memories` returns.
- `RankedHit` — constructed by the row mapper inside each `knn_*` helper, moved through `try_join!` into the merged `Vec`, dropped after `prompt_builder` extracts the fields it needs.

### 2.3 `memory/workspace.rs` — workspace.md file owner

**Public surface**:

```rust
//! Owns the on-disk `agent_workspace.md` file for a (agent_id, project_id) pair.
//! Phase 4: AUTO section regeneration from DB queries + USER section byte preservation.

use std::path::{Path, PathBuf};

use crate::memory::{MemoryError, MemoryStore};

pub(crate) const DEFAULT_PROJECT_SEGMENT: &str = "default";

pub(crate) const AUTO_START: &str = "<!-- AUTO-START -->";
pub(crate) const AUTO_END:   &str = "<!-- AUTO-END -->";
pub(crate) const USER_START: &str = "<!-- USER-START -->";
pub(crate) const USER_END:   &str = "<!-- USER-END -->";

/// Hard cap on the AUTO section's token count (approximation: 4 bytes/token).
/// Per doc 6:1116 — 1500 token budget, recovery order: disputes → focus → skills → arch → prefs.
pub(crate) const AUTO_TOKEN_CAP: usize = 1500;
pub(crate) const AUTO_BYTE_CAP: usize = AUTO_TOKEN_CAP * 4;

/// Stateless façade. Holds only the base directory under which all workspaces live
/// (typically `<state.base_dir>/workspaces`). Trivially `Clone + Send + Sync`.
#[derive(Debug, Clone)]
pub(crate) struct WorkspaceManager {
    workspaces_root: PathBuf,
}

impl WorkspaceManager {
    /// Construct from the WORKSPACES root (caller already joined the "workspaces"
    /// segment to `state.base_dir`). Phase 4 callers: `commands/chat.rs::stream_chat`.
    pub(crate) fn new(workspaces_root: PathBuf) -> Self { Self { workspaces_root } }

    /// Resolves the on-disk path. Pure — no I/O.
    /// `<workspaces_root>/<agent_id>/<project_id | DEFAULT_PROJECT_SEGMENT>/workspace.md`.
    pub(crate) fn workspace_path(
        &self,
        agent_id: &str,
        project_id: Option<&str>,
    ) -> PathBuf;

    /// Regenerates the section between AUTO_START / AUTO_END from DB queries.
    /// The USER_START / USER_END section is preserved BYTE-FOR-BYTE.
    /// If the file does not exist, creates it with both sections (USER section empty).
    /// Errors map to MemoryError::WorkspaceIo (new variant — see §4) or MemoryError::Sqlite.
    pub(crate) async fn regenerate_auto_section(
        &self,
        agent_id: &str,
        project_id: Option<&str>,
        store: &MemoryStore,
    ) -> Result<(), MemoryError>;

    /// Reads the full file (AUTO + USER sections together). Used by prompt_builder.
    /// Returns `Ok(String::new())` when the file does not exist (defensive — first run).
    pub(crate) async fn read(
        &self,
        agent_id: &str,
        project_id: Option<&str>,
    ) -> Result<String, MemoryError>;
}
```

**Internal structure** (private helpers):

- `async fn fetch_auto_rows(store, agent_id, project_id) -> Result<AutoRows, MemoryError>` — runs the FIVE queries from doc 6:1118-1124 in a single `Connection::call`:
  1. `user_preference` rows with `importance >= 8` (`memory` table, agent or global scope).
  2. `architectural_decision` rows scoped to the active `project_id` with `importance >= 8`.
  3. Top-5 `procedural_skill` rows by `access_count` DESC for the agent.
  4. (Phase 4 stub) "Current Session Focus" — empty in Phase 4 (no session-summary writer yet — that's autoDream / Phase 9). Placeholder section header still emitted.
  5. Disputes affecting Tier 1 memories — IDs only (full detail lives in CONFLICTS prompt block). For Phase 4 with zero disputed rows this is always empty.
- `fn render_auto_section(rows: &AutoRows) -> String` — pure function. Emits the markdown shape from doc 6:1091-1109 verbatim. Applies the byte cap by trimming sections in reverse priority (disputes → focus → skills → arch → prefs — per doc 6:1124, prefs are NEVER trimmed).
- `fn split_existing(content: &str) -> Sections` — returns `Sections { auto: Option<(start, end)>, user: Option<(start, end)>, before_auto: &str, between_auto_user: &str, after_user: &str }`. Used by `regenerate_auto_section` to preserve everything outside the AUTO markers byte-for-byte. Naive substring search on the four marker strings (markers are ASCII, unique, hard-coded — no regex).
- `fn render_file(prelude, auto_section, between, user_section, postlude) -> String` — re-assembles. Uses byte slices of the original where possible to maintain byte fidelity for USER content.
- `async fn atomic_write(path: &Path, content: &str) -> Result<(), MemoryError>` — `tokio::fs::create_dir_all(parent)` + `tokio::fs::write(path, content)`. The `tokio::fs::write` is **NOT atomic across crash boundaries** but it IS atomic at the OS syscall level for short writes on common filesystems. Phase 4 single-agent scenario accepts last-writer-wins (R-NEW-6); Phase 5+ may add a per-(agent_id, project_id) `Mutex<()>` if multi-session lands.

**The `AutoRows` struct** (private):

```rust
struct AutoRows {
    user_prefs:        Vec<MemoryRow>,
    arch_decisions:    Vec<MemoryRow>,
    top_skills:        Vec<ProceduralSkillRow>,
    // Phase 4 leaves these empty; the renderer still emits the headers
    // so Phase 5/9 can fill them without changing the file shape.
    current_focus:     Option<String>,
    workspace_disputes: Vec<(i64, i64)>,   // (old_id, new_id) — empty in Phase 4
}
```

**Dependencies on other memory modules**: `MemoryStore`, `MemoryError`, `MemoryRow`, `ProceduralSkillRow`.

**Concurrency story**:

- `WorkspaceManager` holds only `PathBuf` — trivially `Clone + Send + Sync`. The owned `PathBuf` is cheap to clone (single `Arc`-equivalent on Windows). Each `stream_chat` invocation constructs a fresh manager from `state.base_dir`.
- `regenerate_auto_section`: one DB read (`fetch_auto_rows` via `Connection::call`) + one file I/O (`tokio::fs::write` via `tokio::fs::create_dir_all` first). Both await; the future is `Send` because all captures are owned values.
- `read`: one `tokio::fs::read_to_string`. `NotFound` is mapped to `Ok(String::new())` BEFORE returning — callers never see the IO error variant for the common "fresh install" case.
- Compile-time guard at the bottom of `workspace.rs`:

```rust
const _: fn() = || {
    fn _send_sync<T: Send + Sync>() {}
    _send_sync::<WorkspaceManager>();
};
```

**Where constructed / destructed**:

- **Constructed**: at the top of `stream_chat`, BEFORE the agentic loop, as a local: `let workspace_mgr = WorkspaceManager::new(state.base_dir.join("workspaces"));`. Lives for the duration of the `stream_chat` call (~milliseconds to minutes depending on the agentic loop length).
- **Destructed**: implicit drop when `stream_chat` returns. No teardown.

### 2.4 `memory/prompt_builder.rs` — 4-block assembly

**Public surface**:

```rust
//! Assembles the 4-block memory prefix for the system prompt (doc 6:1049-1073).

use std::sync::Arc;

use crate::memory::{
    embedder::Embedder,
    workspace::WorkspaceManager,
    MemoryError, MemoryStore,
};

/// Builds the WORKSPACE / RECENT / LONG-TERM / [ACTIVE CONFLICTS] prefix string.
///
/// Returns `Ok(String::new())` when all four blocks would be empty (no workspace
/// file, no embedder, no DB hits, no disputes) — caller appends nothing in that case.
///
/// Each internal failure is degraded to "the matching block is empty"; the function
/// never returns Err in Phase 4 except if MemoryError::Sqlite escapes from CONFLICTS
/// (which already returns early with empty Vec on error — see §4). The signature
/// keeps `Result<_>` so future phases can promote a fatal-prefix path.
pub(crate) async fn build_memory_prompt(
    store: &MemoryStore,
    workspace: &WorkspaceManager,
    embedder: Option<Arc<dyn Embedder>>,
    agent_id: &str,
    project_id: Option<&str>,
    user_query: &str,
) -> Result<String, MemoryError>;
```

**Internal structure** (private constants + helpers):

```rust
const SEP_WORKSPACE:  &str = "=== WORKSPACE ===";
const SEP_RECENT:     &str = "=== RECENT CONTEXT (Last 7 Days) ===";
const SEP_LONGTERM:   &str = "=== LONG-TERM KNOWLEDGE ===";
const SEP_CONFLICTS:  &str = "=== ACTIVE CONFLICTS (review needed) ===";

const RECENT_WINDOW_SECS: i64 = 7 * 86_400;
const K_FINAL: usize = 10;

/// One row of the CONFLICTS block (per doc 6:644-647).
struct ConflictPair {
    old_id: i64,
    old_created_at: i64,
    old_content: String,
    old_importance: f64,
    old_tier: i64,
    new_id: i64,
    new_created_at: i64,
    new_content: String,
    new_importance: f64,
}

/// Runs the doc 6:654-663 query. Returns Ok(Vec::new()) on SQL error (logged).
async fn fetch_active_conflicts(
    store: &MemoryStore,
    agent_id: &str,
) -> Result<Vec<ConflictPair>, MemoryError>;

/// Pure formatting helpers (no I/O).
fn format_hit_line(hit: &RankedHit) -> String;        // "[YYYY-MM-DD] SOURCE: ... | TYPE: ...\nCONTENT: ..."
fn format_conflict_pair(p: &ConflictPair) -> String;  // "[date] OLD: ... [memory:ID, ...] / [date] NEW: ..."
fn bucket_by_recency(hits: Vec<RankedHit>, now_secs: i64) -> (Vec<RankedHit>, Vec<RankedHit>);
```

**Body of `build_memory_prompt`** (annotated):

```rust
// 1. Read workspace file (may be empty if fresh install or read failure).
let workspace_content = workspace
    .read(agent_id, project_id)
    .await
    .unwrap_or_else(|e| {
        log::warn!("workspace read failed: {e}; treating as empty");
        String::new()
    });

// 2. Compute query embedding ONLY if embedder is available.
let hits: Vec<RankedHit> = match embedder {
    Some(emb) => match emb.embed(user_query).await {
        Ok(q_vec) => match search::search_all_memories(
            store, &q_vec, agent_id, SearchScope::all_default(), K_FINAL,
        ).await {
            Ok(h) => h,
            Err(e) => {
                log::warn!("search_all_memories failed: {e}; no retrieval hits this turn");
                Vec::new()
            }
        },
        Err(e) => {
            log::warn!("embedder.embed failed: {e}; no retrieval hits this turn");
            Vec::new()
        }
    },
    None => {
        log::warn!("embedder not ready; retrieval skipped this turn");
        Vec::new()
    }
};

// 3. Bucket hits into RECENT (created_at within 7 days of now) vs LONG-TERM.
let now_secs = std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map(|d| d.as_secs() as i64)
    .unwrap_or(0);
let (recent, longterm) = bucket_by_recency(hits, now_secs);

// 4. Fetch conflicts (always run the query — only the OUTPUT is conditional per D8).
let conflicts = fetch_active_conflicts(store, agent_id).await.unwrap_or_else(|e| {
    log::warn!("fetch_active_conflicts failed: {e}; CONFLICTS block omitted");
    Vec::new()
});

// 5. Assemble.
let mut out = String::new();
if !workspace_content.is_empty() {
    out.push_str(SEP_WORKSPACE);
    out.push('\n');
    out.push_str(&workspace_content);
    if !workspace_content.ends_with('\n') { out.push('\n'); }
}
if !recent.is_empty() {
    if !out.is_empty() { out.push('\n'); }
    out.push_str(SEP_RECENT);
    out.push('\n');
    for h in &recent { out.push_str(&format_hit_line(h)); out.push('\n'); }
}
if !longterm.is_empty() {
    if !out.is_empty() { out.push('\n'); }
    out.push_str(SEP_LONGTERM);
    out.push('\n');
    for h in &longterm { out.push_str(&format_hit_line(h)); out.push('\n'); }
}
if !conflicts.is_empty() {  // D8 — omit when zero
    if !out.is_empty() { out.push('\n'); }
    out.push_str(SEP_CONFLICTS);
    out.push('\n');
    for p in &conflicts { out.push_str(&format_conflict_pair(p)); out.push('\n'); }
}
Ok(out)
```

**CONFLICTS SQL** (verbatim from doc 6:654-663, adapted):

```sql
SELECT m_old.id, m_old.created_at, m_old.content, m_old.importance, m_old.tier,
       m_new.id, m_new.created_at, m_new.content, m_new.importance
FROM memory m_new
JOIN memory m_old
  ON m_old.id IN (SELECT value FROM json_each(m_new.disputed_against))
WHERE m_new.status   = 'disputed'
  AND m_new.agent_id = ?1
  AND m_new.deleted_at IS NULL
  AND m_old.deleted_at IS NULL;
```

> `json_each(NULL)` returns ZERO rows (R-NEW-7 verified). No NULL guard needed.

**Dependencies**: `MemoryStore`, `MemoryError`, `Embedder` (Phase 2), `WorkspaceManager`, `RankedHit`/`SearchScope`/`search_all_memories` (Phase 4 search.rs).

**Concurrency**: single async function; no compile-time guard needed because no captured value crosses a `try_join!`/`spawn` boundary inside `build_memory_prompt`. The `Embedder: Send + Sync` bound already guarantees the `Arc<dyn Embedder>` is safe across `.await`.

**Where constructed / destructed**: pure function; no struct to manage. Allocated `String` is moved out to the call site.

---

## 3. Data model additions

### 3.1 `RankedHit` (search.rs)

```rust
#[derive(Debug, Clone)]
pub(crate) struct RankedHit {
    pub id: i64,
    pub kind: TableKind,
    pub content: String,
    pub memory_type: String,
    pub tier: i64,
    pub status: String,
    pub score: f64,
    pub distance: f64,        // exposed for Phase 6 (locked in §8 constraint table of proposal)
    pub created_at: i64,      // used by prompt_builder.bucket_by_recency
}
```

**Visibility**: `pub(crate)` per project convention (proposal §11). All fields `pub` so the prompt_builder can destructure without accessors. No `derive(Serialize, Deserialize)` — never crosses an IPC boundary.

### 3.2 `SearchScope` (search.rs)

```rust
#[derive(Debug, Clone, Copy)]
pub(crate) struct SearchScope {
    pub memory:     Option<usize>,
    pub episodic:   Option<usize>,
    pub procedural: Option<usize>,
}
```

Three named ctors as spec'd in §2.2 above. Copy semantics chosen because the struct is 24 bytes (3 × `Option<usize>`) and short-lived.

### 3.3 Workspace marker constants (workspace.rs)

```rust
pub(crate) const DEFAULT_PROJECT_SEGMENT: &str = "default";
pub(crate) const AUTO_START: &str = "<!-- AUTO-START -->";
pub(crate) const AUTO_END:   &str = "<!-- AUTO-END -->";
pub(crate) const USER_START: &str = "<!-- USER-START -->";
pub(crate) const USER_END:   &str = "<!-- USER-END -->";
pub(crate) const AUTO_TOKEN_CAP: usize = 1500;
pub(crate) const AUTO_BYTE_CAP: usize = AUTO_TOKEN_CAP * 4;  // ~6000 bytes
```

All `pub(crate)`. The `DEFAULT_PROJECT_SEGMENT` constant is referenced ONLY from `workspace.rs` in Phase 4; Phase 5+ may import it via `crate::memory::workspace::DEFAULT_PROJECT_SEGMENT` (no re-export from `mod.rs` yet — only added when a second consumer materializes).

### 3.4 Prompt block separators (prompt_builder.rs)

```rust
const SEP_WORKSPACE:  &str = "=== WORKSPACE ===";
const SEP_RECENT:     &str = "=== RECENT CONTEXT (Last 7 Days) ===";
const SEP_LONGTERM:   &str = "=== LONG-TERM KNOWLEDGE ===";
const SEP_CONFLICTS:  &str = "=== ACTIVE CONFLICTS (review needed) ===";
```

**Private to the module** (NOT `pub(crate)`) — they are output details. Tests verify the exact bytes via golden strings in `prompt_builder.rs::tests`.

### 3.5 Block separator format

Per doc 6:1056-1073 + spec REQ:

- Each separator is **its own line** (terminated by `\n`).
- Between separator blocks: ONE blank line (`\n` after the previous block's last content line, then `\n` before the next separator). Implementation: in the assembly loop, before pushing a separator (except the first non-empty block), push `'\n'` if `out` is non-empty (already produces the blank line because the previous block ended with `\n`).
- Per-hit format (RECENT / LONG-TERM): `[YYYY-MM-DD] SOURCE: {source_label} | TYPE: {memory_type}\nCONTENT: {content}` followed by `\n`. The `[YYYY-MM-DD]` uses `chrono`-style format BUT chrono is not a dep — Phase 4 uses `time` crate (already transitive via Tauri/refinery? — verify; if not, manual `format!` from a `time::OffsetDateTime` constructed from the unix seconds). **Design choice**: format inline using a tiny helper that does integer arithmetic on `created_at as i64 / 86_400` plus a Zeller-style date math. AVOIDS new dep. ~15 lines of arithmetic, well-tested by `format_hit_line` unit tests. **Question for orchestrator** if a `time` re-export is preferred — see §11.
- Per-conflict pair format (CONFLICTS) — per doc 6:644-647 verbatim:
  ```
  [{old_date}] OLD: "{old_content}" [memory:{old_id}, importance:{old_imp:.0}, tier:{old_tier}]
  [{new_date}] NEW: "{new_content}" [memory:{new_id}, importance:{new_imp:.0}, status:disputed]
  NOTE: estas memorias se contradicen y aún no resolviste cuál vale.
  Si vas a tomar una decisión que dependa de esto, preguntale al usuario.
  ```
  (One pair = three lines + a trailing blank line before the next pair.)

---

## 4. Error model

Phase 4 EXTENDS the existing `MemoryError` enum with two new variants instead of introducing new module-local error enums. Reasoning: every retrieval/workspace failure is recoverable at the `stream_chat` call site by emitting an empty block — there's no need for per-module error taxonomies that Phase 5+ would then have to convert. One enum keeps the From-chain straightforward.

### 4.1 `MemoryError` additions (in `memory/error.rs`)

Existing variants (per Phase 1): `Sqlite(rusqlite::Error)`, `Migration(refinery::Error)`, `Internal(String)`.

**New in Phase 4** (two variants):

```rust
#[derive(Debug, thiserror::Error)]
pub enum MemoryError {
    // ... existing variants ...

    /// Workspace.md file I/O failure (read/write/create_dir_all).
    /// Path is included for debugability in log lines.
    #[error("workspace I/O failed at {path}: {source}")]
    WorkspaceIo {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// tokio_rusqlite error not directly tied to a SQLite error code
    /// (e.g. background thread closed mid-call). Mapped from
    /// `tokio_rusqlite::Error::ConnectionClosed` / `Close`.
    #[error("memory backend channel error: {0}")]
    Backend(String),
}
```

**Mapping rules** (where conversions happen):

- `rusqlite::Error` → `MemoryError::Sqlite` (existing `From` impl from Phase 1).
- `tokio_rusqlite::Error::Error(rusqlite::Error)` → `MemoryError::Sqlite` via inner.
- `tokio_rusqlite::Error::ConnectionClosed` / `Close(...)` → `MemoryError::Backend` (new). Existing `db.rs::run_migrations` has a bespoke mapping that handles this — Phase 4 generalizes the pattern into the `From` impl on `MemoryError`.
- `std::io::Error` for workspace operations → `MemoryError::WorkspaceIo { path: <known path>, source }`. NOT a blanket `From<io::Error>` because the path is required for the log message — instead, helper functions in `workspace.rs` map explicitly: `.map_err(|e| MemoryError::WorkspaceIo { path: path.clone(), source: e })`.

### 4.2 What `stream_chat` does when retrieval fails

The call site uses `.unwrap_or_default()` on the `build_memory_prompt` result:

```rust
let memory_prefix = prompt_builder::build_memory_prompt(...).await.unwrap_or_default();
```

`unwrap_or_default()` on `Result<String, MemoryError>` yields `String::new()` on `Err`. `build_memory_prompt` ITSELF degrades every internal failure to an empty block + `log::warn!`, so in practice the `Err` branch is unreachable in Phase 4. The `.unwrap_or_default()` is defense-in-depth for Phase 5+ if the function's contract evolves to surface a fatal variant.

The `stream_chat` function itself does NOT propagate any retrieval error to the user. The user sees:

- **Success path**: combined system prompt with memory blocks.
- **Embedder cold**: combined system prompt with WORKSPACE + CONFLICTS blocks only (logged at `warn`).
- **Workspace IO failure**: combined system prompt with RECENT + LONG-TERM + CONFLICTS (logged at `warn`).
- **Total failure**: bare `agent_system_prompt`, identical to pre-Phase-4 behavior.

The user experience degrades **gracefully** from "full RAG" to "no RAG, chat still works" with zero user-visible errors.

### 4.3 Per-call-site error tabulation

| Failure point | Error type | Recovery |
|---------------|------------|----------|
| `embedder.embed(user_query)` returns `EmbedError::NotReady` / `WorkerGone` / `InferenceFailed` | `EmbedError` | `log::warn!` inside `build_memory_prompt`; hits = empty. |
| `Connection::call` for one of `knn_*` queries returns `ConnectionClosed` | `tokio_rusqlite::Error` → `MemoryError::Backend` | `try_join!` propagates; outer `search_all_memories` returns `Err`; `build_memory_prompt` catches and emits empty hits. |
| `Connection::call` for CONFLICTS query returns `Sqlite(_)` | `MemoryError::Sqlite` | `fetch_active_conflicts` returns `Err`; `build_memory_prompt` catches and emits empty conflicts. |
| `tokio::fs::read_to_string(path)` returns `NotFound` | `std::io::Error` | `workspace.read` maps to `Ok(String::new())` BEFORE returning. |
| `tokio::fs::read_to_string(path)` returns `PermissionDenied` etc. | `MemoryError::WorkspaceIo` | `build_memory_prompt` catches and emits empty workspace block. |
| `tokio::fs::write(path)` fails | `MemoryError::WorkspaceIo` | `regenerate_auto_section` returns `Err`; `stream_chat` call site logs `warn!`, chat proceeds (next regen may succeed). The pre-existing file's content is then read by `build_memory_prompt`. |
| `fetch_auto_rows` SQL fails | `MemoryError::Sqlite` | Propagated through `regenerate_auto_section`; same as above. |

---

## 5. Threading & concurrency

### 5.1 UDFs are connection-scoped, not thread-local

`rusqlite::Connection::create_scalar_function` adds the UDF to the connection's function table. The function table is OWNED by the connection. `tokio_rusqlite::Connection::open` moves the underlying `rusqlite::Connection` into a dedicated **background thread** — all subsequent `.call(...)` futures dispatch work onto that thread via a channel.

Consequences for Phase 4:

- The UDF closures registered by `register_score_udfs` execute on that **single background thread** for the connection's entire lifetime.
- There is NO need for `thread_local!` storage — the closures live with the connection, not with the calling task.
- There is NO need for `Mutex` / `RwLock` around the function table — only one thread accesses it.
- The `Send + Sync` requirement on the closures comes from the `create_scalar_function` API itself (it needs the closure to be `Fn + Send + Sync + 'static`). All four UDF closures capture nothing (`||` empty captures), so this is satisfied automatically.

### 5.2 KNN runs on the tokio_rusqlite background thread, serialized with ingest

Both ingestion (Phase 3 `memory_worker`) and retrieval (Phase 4 `search_all_memories` + `fetch_active_conflicts`) call `state.memory_store.conn.call(...)`. Since there is ONE connection and ONE background thread, all these calls are **serialized** at the SQLite layer.

Consequences:

- Phase 4 retrieval can **block** behind a long-running Phase 3 INSERT (sub-millisecond at Phase 4 scale, may grow to 10-30ms at Phase 5+ when AccessTracker batches UPDATE). Documented as R-NEW-5 in proposal; acceptable for v1.
- Phase 4 retrieval CANNOT corrupt Phase 3 inserts and vice versa — the serialization is a correctness guarantee, not a bottleneck for current scale.
- The "parallel" `tokio::try_join!` in `search_all_memories` is semantic (cancellation + ergonomics), not wall-time parallel: the three `knn_*` calls land in the same channel queue and run one at a time on the background thread.

### 5.3 Workspace.md file I/O: tokio::fs (async, non-blocking)

`regenerate_auto_section` uses `tokio::fs::{create_dir_all, write}`. `read` uses `tokio::fs::read_to_string`. All three offload to the tokio blocking thread pool, freeing the main runtime thread.

- File writes are **last-writer-wins** on the OS level (`tokio::fs::write` truncates and rewrites). For Phase 4's single-agent setup there is exactly one writer at a time (the `stream_chat` call site). For Phase 5+ multi-session (e.g. multiple agents in flight), a per-(agent_id, project_id) `Mutex<()>` MAY be added to AppState; the design pre-allocates the slot conceptually but does not require it for Phase 4.
- Reads concurrent with a write may observe partial content on some filesystems. Phase 4 ordering (regen → read, sequential) eliminates this within a single `stream_chat` invocation; cross-invocation interleaving is rare enough to accept.

### 5.4 Compile-time Send + Sync assertions

Following Phase 2/3 precedent, every type that crosses an await boundary in Phase 4 gets a `const _: fn()` guard.

| Type | Bound | Why it holds | Guard location |
|------|-------|--------------|----------------|
| `SearchScope` | `Send + Sync + Copy` | `Option<usize>` × 3 — primitive | `search.rs` |
| `RankedHit` | `Send + Sync` | All fields are owned scalars + `String` + `TableKind` (which is `Copy`) | `search.rs` |
| `WorkspaceManager` | `Send + Sync + Clone` | `PathBuf` is `Send + Sync + Clone` | `workspace.rs` |
| `ConflictPair` (private) | `Send` (Sync not needed — lives in `Vec` inside one async fn) | All owned values | implicit (caught by closure capture if missing) |

Sample (placed at the bottom of `search.rs`):

```rust
const _: fn() = || {
    fn _ss<T: Send + Sync>() {}
    _ss::<SearchScope>();
    _ss::<RankedHit>();
};
```

And at the bottom of `workspace.rs`:

```rust
const _: fn() = || {
    fn _ss<T: Send + Sync>() {}
    fn _clone<T: Clone>() {}
    _ss::<WorkspaceManager>();
    _clone::<WorkspaceManager>();
};
```

No guard is added in `prompt_builder.rs` because it exposes only an async function (the `Send` bound comes implicitly from the captured values, all of which are owned or `Send`).

### 5.5 `Arc<dyn Embedder>` guard pattern (carry-forward Phase 2/3)

`build_memory_prompt` takes `embedder: Option<Arc<dyn Embedder>>` by value. The caller (`stream_chat`) extracts the option via `state.embedder.get().cloned()` — exactly the Phase 2/3 pattern, which is "lock-free read of an `Arc<OnceCell<Arc<dyn Embedder>>>`".

The `.cloned()` produces `Option<Arc<dyn Embedder>>`. This is critical: the `OnceCell::get()` borrows the cell; cloning the inner `Arc` releases the borrow immediately, freeing the cell for concurrent readers and letting the embedder cross the `.await` inside `build_memory_prompt` safely.

---

## 6. Integration points

### 6.1 `commands/chat.rs::stream_chat` — modified pre-loop block

**Pre-loop insertion** (between the existing agent_id capture at line 436 and the `for iteration` at line 539):

The exact insertion point sits AFTER line 532 (`let agent_ctx = AgentContext { ... };`) and BEFORE line 538 (`// === AGENTIC LOOP ===`). This places the memory pre-compute AFTER OAuth refresh / provider build / hook compile (so any of those failing short-circuits BEFORE we pay the retrieval cost) and BEFORE the loop body so the combined prompt is stable across iterations.

Inserted code (Rust pseudo-code mirroring §1's diagram):

```rust
// ── Phase 4 — pre-compute memory prefix ONCE before the agentic loop (D2) ──
let project_id_opt = agent_project_path.as_deref().and_then(|p| {
    if p.is_empty() { None }
    else { Some(crate::memory::derive_project_id(std::path::Path::new(p))) }
});

let workspace_mgr = crate::memory::workspace::WorkspaceManager::new(
    state.base_dir.join("workspaces"),
);

if let Err(e) = workspace_mgr
    .regenerate_auto_section(&agent_id, project_id_opt.as_deref(), &state.memory_store)
    .await
{
    log::warn!("workspace regen failed for agent '{agent_id}': {e}");
}

let memory_prefix = crate::memory::prompt_builder::build_memory_prompt(
    &state.memory_store,
    &workspace_mgr,
    state.embedder.get().cloned(),
    &agent_id,
    project_id_opt.as_deref(),
    &message,  // see §6.2 below — user_query source
)
.await
.unwrap_or_default();

let combined_system_prompt = if memory_prefix.is_empty() {
    agent_system_prompt.clone()
} else {
    format!("{}\n\n{}", memory_prefix, agent_system_prompt)
};

// TODO Phase 5+: re-run retrieval per iteration when AccessTracker emits
// synchronous tier-change events mid-loop (D2 forward-marker).
```

**Loop body change** at line 558 — single token swap:

```rust
// BEFORE
system_prompt: Some(agent_system_prompt.clone()),
// AFTER
system_prompt: Some(combined_system_prompt.clone()),
```

That is the **only** change inside the `for iteration in 0..MAX_TOOL_ITERATIONS` body — preserves D2 (same prefix all iterations) AND minimizes regression surface on the 4 existing `chat.rs` tests.

### 6.2 `user_query` source

The query embedding needs a representative text. Two candidates:

1. **The user message that triggered this `stream_chat` call** — `&message` parameter. Available always (lines 411-417 signature). **Chosen**.
2. The last N turns of the conversation (composite query).

Rationale: option 1 mirrors how a typical RAG system formulates the query — the most recent user intent. Option 2 is a Phase X enhancement that doc 6 does not discuss for v1. For retry paths (`is_retry = true`, line 419), the user message is the same as the previous attempt — retrieval reruns and may produce the same hits (acceptable; chat hot path is the cost center, not retrieval).

Edge case: if `message` is empty (somehow), `embedder.embed("")` returns a vector of zero-length text embedding (FakeEmbedder hashes the empty string; FastEmbed handles it). The hits will likely be poor but the system does not crash — defensive design.

### 6.3 What happens if `memory_prefix` is empty

`memory_prefix.is_empty()` is true when:

- Embedder is None AND workspace file is absent (very first run, before bootstrap finishes downloading the model).
- Embedder is None AND workspace file exists but is empty (extremely unlikely — `regenerate_auto_section` always emits at least the section headers).
- All three blocks are skipped: no hits, no workspace file, no disputes.

When empty, the call site uses `agent_system_prompt.clone()` UNMODIFIED — byte-identical to the pre-Phase-4 behavior. The 4 existing `chat.rs` tests that don't seed memories will see identical `ChatRequest.system_prompt` content and stay green (R-NEW-4 mitigation made concrete).

### 6.4 How `agent_id` and `project_id` are extracted

- `agent_id`: the `stream_chat` parameter (already a `String` per line 412).
- `project_id`: derived from `agent_project_path` (captured at line 431 into the existing `Option<String>` local). The derivation reuses **`crate::memory::derive_project_id`** (Phase 3 pattern; see chat.rs Phase 3 design §4.d for precedent). Phase 4 imports it via the existing re-export from `memory/mod.rs`. The derivation skips when:
  - `agent_project_path` is `None` (agent has no project bound).
  - `agent_project_path` is `Some("")` (the empty-string guard — same idiom as Phase 3).
  In both cases, `project_id_opt = None`, which routes the workspace path through `DEFAULT_PROJECT_SEGMENT` (D9).

### 6.5 `MemoryStore::open()` / `new_for_test()` modifications (db.rs)

Two surgical insertions (one per ctor):

```rust
// db.rs:51-61 — open()
pub async fn open(base_dir: &Path) -> Result<Self, MemoryError> {
    ensure_vec0_registered();
    let _ = std::fs::create_dir_all(base_dir);
    let path = base_dir.join("memory.db");
    let conn = Connection::open(&path).await.map_err(MemoryError::Sqlite)?;
    run_migrations(&conn).await?;
    crate::memory::score::register_score_udfs(&conn).await?;   // ← NEW (D6)
    Ok(Self { conn })
}

// db.rs:69-76 — new_for_test()
#[cfg(test)]
pub async fn new_for_test() -> Result<Self, MemoryError> {
    ensure_vec0_registered();
    let conn = Connection::open_in_memory().await.map_err(MemoryError::Sqlite)?;
    run_migrations(&conn).await?;
    crate::memory::score::register_score_udfs(&conn).await?;   // ← NEW (D6)
    Ok(Self { conn })
}
```

Both await `register_score_udfs(&conn).await?` AFTER `run_migrations` succeeds. The `?` propagates any registration error as `MemoryError::Sqlite` (the only error variant register_score_udfs returns, since the inner `.call().await` maps `tokio_rusqlite::Error` → `MemoryError`).

The R-NEW-9 risk (adding `.await` to construction) is verified mitigated: both ctors are already `async fn` per Phase 1, so no caller signature changes.

### 6.6 `memory/mod.rs` modifications

Append four lines after the existing `pub mod` declarations:

```rust
pub(crate) mod score;
pub(crate) mod search;
pub(crate) mod workspace;
pub(crate) mod prompt_builder;
```

**No new re-exports**. `commands/chat.rs::stream_chat` uses fully-qualified paths (`crate::memory::workspace::WorkspaceManager`, `crate::memory::prompt_builder::build_memory_prompt`). Phase 5+ that needs `DEFAULT_PROJECT_SEGMENT` cross-module can add a `pub(crate) use workspace::DEFAULT_PROJECT_SEGMENT;` then, not now (KISS).

---

## 7. Test seams

### 7.1 FakeEmbedder reuse (search + integration tests)

`FakeEmbedder` already exists in `memory/embedder.rs` as `#[cfg(test)] pub(crate)`. It is reused unchanged by:

- `memory/search.rs::tests` — for `search_all_memories` integration tests.
- `memory/prompt_builder.rs::tests` — to seed the embedder argument.
- `commands/chat.rs::tests` — to drive the integration test that captures `ChatRequest.system_prompt`.

Phase 3's `state.rs::seeded_embedder_cell()` helper already pre-fills the cell with `FakeEmbedder` for test `AppState`s — Phase 4 inherits this with no additions.

### 7.2 In-memory `MemoryStore` (UDF + search tests)

`MemoryStore::new_for_test()` already returns an in-memory store. Phase 4's modification to call `register_score_udfs` means **every** test store has the 4 UDFs available out of the gate — no per-test UDF setup required.

Validation pattern for `score.rs` tests:

```rust
let store = MemoryStore::new_for_test().await.unwrap();
let v: f64 = store.conn.call(|c| {
    c.query_row("SELECT exp_decay(0, 0.023)", [], |r| r.get(0))
}).await.unwrap();
assert!((v - 1.0).abs() < 1e-9);
```

### 7.3 `tempfile::TempDir` (workspace tests)

`tempfile` is already a transitive dep (Phase 1 used it in tests). Pattern:

```rust
#[tokio::test]
async fn workspace_path_with_project_id() {
    let tmp = tempfile::tempdir().unwrap();
    let wm = WorkspaceManager::new(tmp.path().join("workspaces"));
    let p = wm.workspace_path("agent-1", Some("claw-garden-1a2b"));
    assert!(p.ends_with("workspaces/agent-1/claw-garden-1a2b/workspace.md"));
}
```

For tests that exercise file content (USER section byte preservation, 1500-token cap recovery order), the same `TempDir` is used, with `tokio::fs::write` pre-seeding the file and `tokio::fs::read_to_string` verifying after `regenerate_auto_section`.

### 7.4 `prompt_builder` fixture tests (no DB)

Pure-formatting helpers (`format_hit_line`, `format_conflict_pair`, `bucket_by_recency`) are unit-tested as pure functions:

```rust
#[test]
fn format_hit_line_emits_canonical_format() {
    let hit = RankedHit { id: 1, kind: TableKind::Memory, content: "x".into(),
        memory_type: "chat_message".into(), tier: 3, status: "active".into(),
        score: 0.5, distance: 0.2, created_at: 1_700_000_000 };
    let s = format_hit_line(&hit);
    assert!(s.starts_with("[2023-11-14] SOURCE: Memory | TYPE: chat_message"));
    assert!(s.contains("CONTENT: x"));
}
```

The end-to-end `build_memory_prompt` test seeds an in-memory store + temp workspace file + FakeEmbedder, then asserts the full prefix string against a golden fixture (with date substituted to a stable test value via `unix_now_secs`-equivalent injection or a `created_at` derivation that gives a known calendar date).

### 7.5 `stream_chat` integration test (stubbed embedder + fixture corpus)

Three new tests under `commands/chat.rs::tests` per proposal §6:

1. `stream_chat_with_memories_prepends_blocks_to_system_prompt` — hand-INSERT 3 memories via `state.memory_store`, drive `stream_chat`, capture the `ChatRequest.system_prompt` via a test provider (the existing `OllamaProvider` test fixture from Phase 3, extended to capture the request), assert the system prompt starts with `=== WORKSPACE ===` or `=== RECENT CONTEXT (Last 7 Days) ===` (whichever block has content).
2. `stream_chat_with_no_memories_uses_unmodified_system_prompt` — empty store + no workspace file, assert `ChatRequest.system_prompt == Some(agent_system_prompt)` byte-equal.
3. `stream_chat_with_embedder_not_ready_does_not_panic` — replace the `seeded_embedder_cell()` with an empty `OnceCell` (need a test-only `AppState::new_for_test_without_embedder` helper, OR use a test that takes the seeded state and forces the cell to a fresh empty one via a `pub(crate) fn` reset — design choice deferred to sdd-tasks).

The "test provider that captures requests" is the only new test infrastructure; everything else reuses Phase 2/3 helpers.

### 7.6 NO sleep-in-tests

Carry forward Phase 2/3 lesson. All Phase 4 tests assert via direct SQLite queries, `tokio::fs::read_to_string`, or string equality. No `tokio::time::sleep` anywhere. `tokio::time::pause()` + `advance()` not needed (Phase 4 has no timer-based logic).

### 7.7 NO benches

Per D1, no `benches/` directory. Latency observed via `log::debug!`.

---

## 8. Telemetry

All logging uses `log::` macros (43-site codebase convention). `tracing::` is NOT a dependency and MUST NOT be added.

### 8.1 Per-D1: retrieval latency observation

Single placement in `search::search_all_memories`, after `try_join!` succeeds:

```rust
log::debug!(
    "retrieval={}ms hits={} pool=({}+{}+{}) k_final={}",
    t0.elapsed().as_millis(),
    all.len(),
    scope.memory.unwrap_or(0),
    scope.episodic.unwrap_or(0),
    scope.procedural.unwrap_or(0),
    k_final,
);
```

Additional `log::debug!` at the `stream_chat` call site for the **end-to-end memory pre-compute cost**:

```rust
log::debug!("memory prefix build took {}ms, len={}", build_t0.elapsed().as_millis(), memory_prefix.len());
```

These are `debug` level (off by default in production) — they are diagnostics for when a user reports lag, not always-on metrics.

### 8.2 Per-D2 / R-NEW-3: embedder None graceful

In `build_memory_prompt`, when `embedder` is `None`:

```rust
log::warn!("embedder not ready; retrieval skipped this turn (agent_id={agent_id})");
```

When the embedder is `Some(_)` but `embed()` fails:

```rust
log::warn!("embedder.embed failed for agent_id={agent_id}: {e}; no retrieval hits this turn");
```

`warn` level so it surfaces in default log output without polluting `info` (which the codebase uses for state transitions).

### 8.3 Workspace.md failures

Three placements:

```rust
// In workspace.read — only for non-NotFound errors:
log::warn!("workspace read failed at {path:?}: {e}");

// In stream_chat call site, after regenerate_auto_section returns Err:
log::warn!("workspace regen failed for agent '{agent_id}': {e}");

// In build_memory_prompt, if read returns Err (already mapped by workspace.read,
// but doubly defensive):
log::warn!("workspace block omitted due to read failure: {e}");
```

All `warn` level — workspace failures are recoverable but worth surfacing.

### 8.4 CONFLICTS query failure

```rust
log::warn!("fetch_active_conflicts failed for agent '{agent_id}': {e}; CONFLICTS block omitted");
```

### 8.5 NO `log::info!` additions

Phase 4 adds zero `info!` calls. The lifecycle (chat turn, ingest event, etc.) is already covered by Phase 1-3 logging. Adding `info!` for retrieval would clutter the default log stream.

### 8.6 NO `log::error!` additions

Phase 4 has no fatal paths — all failures degrade to empty blocks. `error!` is reserved for "something is wrong, the user should know" (Phase 1 uses it for persistence failures). Retrieval failures are silent-to-user by design.

---

## 9. Risks & mitigations

### 9.1 Carry-forward R-NEW from explore.md / proposal.md

| ID | Risk | Phase 4 design mitigation |
|----|------|---------------------------|
| **R-NEW-1** | Hot path KNN latency spike on cold SSD / HDD | `log::debug!("retrieval={ms}ms hits={n}…")` in `search_all_memories` (D1). First adjustment lever: reduce `SearchScope::all_default()` to `(Some(10), Some(3), Some(3))`. Second lever: `tier > 1` filter is already in (D5) reducing the index scan. |
| **R-NEW-2** | Race between ingest worker and retrieval | Accepted. Documented in §1 lifecycle ("ingest commit is not awaited"). The just-ingested chunk for the current turn won't appear in the current turn's retrieval — that's correct (retrieval reflects committed state). |
| **R-NEW-3** | Embedder cold-start during chat | `if let Some(embedder) = embedder_opt` guard in `build_memory_prompt`. Returns empty hits, `log::warn!`, chat proceeds. The `state.embedder.get().cloned()` pattern is unchanged from Phase 2/3. |
| **R-NEW-4** | `prompt.rs` tests assert content of `compose_system_prompt` | `compose_system_prompt` is NOT modified. Memory prefix is prepended at the `stream_chat` call site via `format!("{}\n\n{}", memory_prefix, agent_system_prompt)`. All 10 `prompt.rs` tests stay green untouched. |
| **R-NEW-5** | `tokio_rusqlite` write contention between ingest worker and retrieval | Accepted for Phase 4 (sub-ms). `tokio_rusqlite::Connection` is `Clone` (channel handle) — Phase 4 does NOT spawn additional connections; the existing serialization at the background thread is the contention surface. Phase 5+ AccessTracker batches via the same connection, mitigated by its async batching design. |
| **R-NEW-6** | Workspace.md corruption on concurrent writes | Accepted for Phase 4 single-agent setup. `tokio::fs::write` is atomic on rename-capable platforms. The sequential regen → read order inside `stream_chat` eliminates intra-invocation races. Phase 5+ may add `Mutex<()>` per (agent_id, project_id). |
| **R-NEW-7** | `json_each()` on NULL `disputed_against` | Verified safe (returns 0 rows). No NULL guard in the CONFLICTS query. |
| **R-NEW-8** | `importance_factor` UDF input clamping | `let imp = imp.clamp(1.0_f64, 10.0);` inside the UDF body. Tested via `score.rs::tests` (`importance_factor(0.0) == 0.55`, `importance_factor(15.0) == 1.0`). |
| **R-NEW-9** | UDF registration adds `.await` to `MemoryStore::open()` | Both ctors are already `async fn` per Phase 1 (verified at `db.rs:51,69`). No caller signature change. Risk mitigated. |
| **R-NEW-10** | `build_memory_prompt` adds TTFT overhead on empty path | Empty path: `workspace.read` is `NotFound → Ok("")` (1 stat syscall, ~sub-ms), `search` skipped because embedder is None or hits empty, `fetch_active_conflicts` is a single indexed query (~sub-ms with zero rows). Total: <2ms on the empty path vs. ~500ms LLM TTFT. Negligible. |

### 9.2 New design-surfaced risks

Two additional risks identified during code re-verification:

| ID | Risk | Mitigation |
|----|------|------------|
| **R-NEW-11** | The CONFLICTS query uses `json_each(...)` from SQLite's JSON1 extension. Phase 1 verified `rusqlite` bundled includes `-DSQLITE_ENABLE_JSON1` by default, BUT this was not exercised by any existing test in the live tree. | Add a Phase 4 unit test in `prompt_builder.rs::tests` that runs the CONFLICTS query against an in-memory store with a hand-INSERTed `status='disputed'` row + `disputed_against = JSON([1])`. If `json_each` is missing, the test fails at query parse time with a clear error message ("no such function: json_each") — easy to diagnose, easy to fix (add `features = ["json"]` to the `rusqlite` crate). |
| **R-NEW-12** | `format_hit_line` needs to render `created_at` (unix seconds) as `[YYYY-MM-DD]`. No date-formatting crate is currently in the dep graph (`chrono`, `time`, `jiff` all absent). | Implement a tiny private `fn unix_secs_to_ymd(ts: i64) -> (i32, u32, u32)` using integer arithmetic (Howard Hinnant's algorithm). Unit-tested with known epochs (e.g. `0 → (1970, 1, 1)`, `1_700_000_000 → (2023, 11, 14)`). ~30 lines, zero dep. Alternative: pull `time = "0.3"` (one new dep). Design recommends zero-dep arithmetic; final call deferred to sdd-tasks (asked in §11). |

---

## 10. Carry-forward patterns from Phase 3 (applied unmodified)

### 10.1 Tuple-return ctor for `(Self, Receiver)`

Phase 3 introduced `AppState::new() -> (Self, UnboundedReceiver<MemoryEvent>)` so callers must explicitly own the receiver end. **Phase 4 does NOT introduce a new receiver** — retrieval is read-only and doesn't need a new channel. The pattern is acknowledged: any future Phase that adds a channel (e.g. Phase 5 AccessTracker's tier-change events) should follow the same tuple-return ctor shape.

### 10.2 `Option<Arc<dyn Trait>>` guard for embedder cell

Lifted unchanged: `state.embedder.get().cloned()` produces `Option<Arc<dyn Embedder>>`, passed by value to `build_memory_prompt`. The `if let Some(...)` guard inside the builder is the exact pattern Phase 3's worker uses (`if let Some(embedder) = embedder_cell.get().cloned() else { warn; continue; }`).

### 10.3 Atomic transaction pattern

Phase 3 used `tokio_rusqlite::Connection::call(|c| { let tx = c.transaction()?; ...; tx.commit()?; Ok(...) })` for the dual INSERT into `memory` + `memory_vectors`. **Phase 4 is read-only** — no transactions are needed for SELECT queries. The pattern is documented here for completeness: if Phase 4's workspace.md regeneration ever needs to write to MULTIPLE tables (it does not), the same wrapping would apply.

### 10.4 `#[cfg(test)] pub(crate)` for FakeEmbedder cross-file reuse

`FakeEmbedder` is `#[cfg(test)] pub(crate)` in `memory/embedder.rs` (Phase 2). Phase 3 verified the cross-file usage works (`use crate::memory::embedder::FakeEmbedder;` from `worker.rs::tests`). Phase 4 follows the same import path from:
- `memory/search.rs::tests`
- `memory/prompt_builder.rs::tests`
- `commands/chat.rs::tests`

No additional gating (no `cfg(any(test, feature = "test-helpers"))`) needed — proposal §11 confirmed Phase 4 sticks with `#[cfg(test)]`.

### 10.5 Compile-time Send/Sync assertion via `const _: fn() = ...`

Applied to `SearchScope`, `RankedHit`, `WorkspaceManager` — see §5.4.

### 10.6 `log::` not `tracing::`

This design replaces Phase 3 design's `tracing::warn!` references with `log::warn!`. Verified at the launch prompt level: `tracing::` is NOT in the dep graph; `log::` has 43 call sites. Phase 4 introduces ZERO new `tracing::` calls.

### 10.7 `pub(crate)` over `pub` unless cross-crate is needed

All four new modules' public types and functions are `pub(crate)`. The four constants in `workspace.rs` (`DEFAULT_PROJECT_SEGMENT`, `AUTO_START`, etc.) are also `pub(crate)` so Phase 5/6 can import them without going through `mod.rs` re-exports.

### 10.8 NEVER `cargo build` during development

Phase 4 uses `cargo check` + `cargo test --lib` exclusively. The design contains no instructions that would require `cargo build` to validate. Cargo dependencies are zero new (proposal §5).

### 10.9 No `Co-Authored-By` in commits, conventional commits only

Carry-forward acknowledgement. All Phase 4 commits use `feat(rag): ...`, `test(rag): ...`, `refactor(rag): ...` format.

---

## 11. Design questions for orchestrator

The proposal locked D1–D9. The following ambiguities surfaced during code re-verification — none break the design, but the orchestrator may want to resolve before sdd-tasks:

1. **Date formatting for `format_hit_line` / `format_conflict_pair`** — implement via in-module integer arithmetic (`unix_secs_to_ymd` helper, ~30 lines, zero dep) OR add `time = "0.3"` to `Cargo.toml`? Proposal §5 says "Zero new dependencies"; design recommends the zero-dep arithmetic path. Confirm.

2. **`user_query` source for the query embedding** — design chose `&message` (the user's current message). Alternatives: (a) last N turns concatenated, (b) `format!("{message} {agent_system_prompt}")` (richer context, more noise). v1 picks (a) — single message — per doc 6's implicit assumption. Confirm or specify.

3. **`SearchScope::all_default()` k_final value at the `build_memory_prompt` call site** — design hardcodes `K_FINAL = 10` constant in `prompt_builder.rs` (a balance between RECENT + LONG-TERM coverage and 1500-token budget for the combined memory prefix). Doc 6 doesn't pin a specific number — it gives `k_pool = 30` (memory) + `k_pool = 5` (each of episodic/procedural) for the per-table pools, but the final merged truncation is implementation-defined. Confirm `K_FINAL = 10` is acceptable, or specify a different value.

4. **`source_label` for `format_hit_line`** — design uses `TableKind` debug-formatted (`"Memory"`, `"Episodic"`, `"Procedural"`). Doc 6's example (line 1062) shows `SOURCE: Chat` — but "Chat" maps to a `memory_type`, not a `TableKind`. The line should likely be `SOURCE: {TableKind:?} | TYPE: {memory_type}`. Confirm format.

5. **Phase 4 `current_focus` placeholder in workspace.md** — design emits the section header (`## Current Session Focus`) with an empty body. Doc 6:1108 expects `[session:current]` content from autoDream (Phase 9). For Phase 4, this is acceptable as a stub, but the orchestrator may prefer omitting the section entirely until Phase 9. Confirm: emit-empty-header vs omit-entirely.

6. **`MemoryError` extension vs. new module-local error enums** — design extends `MemoryError` with `WorkspaceIo` and `Backend`. Proposal §11 lists "either extends or introduces" as a design choice. Confirm extension is the chosen path (recommended for the From-chain simplicity).

7. **`pub(crate) use` re-exports in `memory/mod.rs`** — design adds NONE in Phase 4 (only `pub(crate) mod` declarations). Phase 5/6 that needs `DEFAULT_PROJECT_SEGMENT` cross-module will add the re-export then. Confirm KISS approach.

If the orchestrator does not respond to these before sdd-tasks runs, the design's recommended answers (zero-dep arithmetic, `&message`, K_FINAL=10, `SOURCE: {TableKind:?} | TYPE: {memory_type}`, emit-empty-header, MemoryError extension, no re-exports) are the locked-in defaults.

---

## 12. Done definition

Phase 4 design is consumed by sdd-tasks → sdd-apply → sdd-verify → sdd-archive. The design is done when:

- All 9 LOCKED proposal decisions (D1–D9) are reflected in §2 module structure + §6 integration points.
- ASCII lifecycle (§1) shows every failure branch the failure-mode tests will exercise.
- The error model (§4) maps every recoverable failure to a degraded-empty-block path; only `unwrap_or_default()` at the `stream_chat` call site protects against any future `Err` escape.
- The threading model (§5) confirms UDFs are connection-scoped, KNN serializes on the tokio_rusqlite background thread, and workspace I/O is async via `tokio::fs`.
- The 10 carry-forward R-NEW risks + 2 new R-NEW-11/12 are visible to the orchestrator.
- The 7 design questions in §11 are visible to the orchestrator for resolution OR fall back to the documented defaults.
- No `cargo build` was invoked while drafting the design.
- Zero new dependencies were added or implied.
- All instrumentation uses `log::` macros (zero `tracing::`).
