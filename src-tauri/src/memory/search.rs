//! KNN search across memory + episodic + procedural via tokio::try_join!,
//! merged + ranked by the score formula computed in SQL.

use std::time::Instant;

use crate::memory::clock::TimestampMs;
use crate::memory::score::{score as compute_score, RawHit, ScoringConfig};
use crate::memory::vector::to_unit;
use crate::memory::{MemoryError, MemoryStore, TableKind};

/// Per-table k_pool selection. `None` means "skip this table entirely".
///
/// `project_id: None` means unscoped — every caller that does not thread a
/// project through (rag-eval, and any test that builds a scope directly)
/// keeps seeing the exact pre-scoping row set, because the SQL guard this
/// drives is `?4 IS NULL OR ...`, a tautology when unset. `Copy` is dropped
/// because `project_id` owns a `String`; every constructor still hands back
/// an independent value, so callers pay nothing for it.
#[derive(Debug, Clone)]
pub(crate) struct SearchScope {
    pub memory: Option<usize>,
    pub episodic: Option<usize>,
    pub procedural: Option<usize>,
    pub project_id: Option<String>,
}

impl SearchScope {
    pub(crate) fn all_default() -> Self {
        Self {
            memory: Some(30),
            episodic: Some(5),
            procedural: Some(5),
            project_id: None,
        }
    }
    /// Scopes this query to `project_id`, owning a copy of it. `None` keeps
    /// the scope unscoped — same as never calling this.
    pub(crate) fn with_project(mut self, project_id: Option<&str>) -> Self {
        self.project_id = project_id.map(str::to_owned);
        self
    }
}

/// One result row after the score formula has been applied SQL-side.
#[derive(Debug, Clone)]
pub(crate) struct RankedHit {
    pub id: i64,
    pub kind: TableKind,
    pub content: String,
    pub memory_type: String,
    pub tier: i64,
    pub status: String,
    pub score: f64,
    pub distance: f64,   // raw KNN distance — consumed by contradiction.rs to threshold-filter near-duplicate candidates
    pub created_at: TimestampMs, // for RECENT vs LONG-TERM bucketing in prompt_builder
}

// Compile-time Send + Sync assertions
const _: fn() = || {
    fn _ss<T: Send + Sync>() {}
    _ss::<SearchScope>();
    _ss::<RankedHit>();
};

/// Score first, then a key that cannot repeat: id is unique per table and
/// kind separates the three, so two hits can never compare Equal. Without
/// the second half, rows that score identically would land in whatever
/// order the join emitted them, and the same query could answer differently
/// on two runs.
fn hit_cmp(a: &RankedHit, b: &RankedHit) -> std::cmp::Ordering {
    b.score
        .partial_cmp(&a.score)
        .unwrap_or(std::cmp::Ordering::Equal)
        .then_with(|| a.kind.cmp(&b.kind))
        .then_with(|| a.id.cmp(&b.id))
}

/// Encode query vector as little-endian bytes for vec0 MATCH operator.
fn serialize_f32_query(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// Public test helper — exposes serialize_f32_query for cross-module test use.
#[cfg(test)]
pub(crate) fn serialize_f32_query_pub(v: &[f32]) -> Vec<u8> {
    serialize_f32_query(v)
}

// SQL for memory table KNN
// The neighbour count reaches vec0 as a constraint on the virtual table itself
// (`knn.k = ?2`), not as a LIMIT and not through a subquery: across a join a
// LIMIT would bound the already-filtered result, which is a different number
// than the one vec0 needs to size its scan. Everything the WHERE clause below
// tests — status, deleted_at, agent, project — is therefore applied to rows
// vec0 has ALREADY cut down to k, so k is a candidate-pool budget rather than
// the number of results. `knn_one` documents what happens when those predicates
// empty that pool.
//
// Nine raw columns, no UDF calls, no `strftime`, no `ORDER BY`: the score
// this used to compute inline now happens in Rust (`score.rs`), which reads
// `last_accessed_at` and `importance` straight off these rows instead of a
// SQL-computed column. Rows still arrive in vec0's own distance order, not
// score order — `knn_one` is what turns this into a score-ordered result.
const SEARCH_SQL_MEMORY: &str = "
    SELECT m.id, m.content, m.memory_type, m.tier, m.status, knn.distance,
        m.created_at, m.last_accessed_at, m.importance
    FROM memory_vectors knn
    JOIN memory m ON m.id = knn.rowid
    WHERE knn.embedding MATCH ?1
      AND knn.k = ?2
      AND m.status IN ('active', 'disputed')
      AND m.deleted_at IS NULL
      AND m.agent_id = ?3
      AND (?4 IS NULL OR m.project_id = ?4 OR m.project_id IS NULL OR m.memory_type = 'user_preference')
";

// SQL for episodic table KNN. The 0.0077 lambda this table always used no
// longer lives here — it is `DecayLambdas.episode` in score.rs, reached
// through `lambda_for` dispatching on `TableKind::Episodic` rather than on
// the `'episode'` string this query still selects for display.
const SEARCH_SQL_EPISODIC: &str = "
    SELECT epi.id, epi.task, 'episode' AS memory_type, epi.tier, epi.status, knn.distance,
        epi.created_at, epi.last_accessed_at, epi.importance
    FROM episodic_vectors knn
    JOIN episodic_trajectory epi ON epi.id = knn.rowid
    WHERE knn.embedding MATCH ?1
      AND knn.k = ?2
      AND epi.status IN ('active', 'disputed')
      AND epi.deleted_at IS NULL
      AND epi.agent_id = ?3
      AND (?4 IS NULL OR epi.project_id = ?4 OR epi.project_id IS NULL)
";

// SQL for procedural table KNN. Same as episodic: 0.0039 now lives in
// `DecayLambdas.procedure`, dispatched by `TableKind::Procedural`.
const SEARCH_SQL_PROCEDURAL: &str = "
    SELECT pro.id, pro.skill_name, 'procedure' AS memory_type, pro.tier, pro.status, knn.distance,
        pro.created_at, pro.last_accessed_at, pro.importance
    FROM procedural_vectors knn
    JOIN procedural_skill pro ON pro.id = knn.rowid
    WHERE knn.embedding MATCH ?1
      AND knn.k = ?2
      AND pro.status IN ('active', 'disputed')
      AND pro.deleted_at IS NULL
      AND pro.agent_id = ?3
      AND (?4 IS NULL OR pro.project_id = ?4 OR pro.project_id IS NULL)
";

// How many rows THIS caller could receive at most, counted up to a ceiling of
// `?1` (the pool size) because nothing above that changes any decision.
//
// Counting the vector table instead would answer a different question — "does
// the corpus hold k vectors" — and that question is blind to who is asking.
// On a database holding several projects' rows for one agent it is answered
// "yes" permanently, while the caller's own eligible set may be three rows
// wide, so a pool that can never be filled reads as a pool that was starved.
// These predicates are the same ones the KNN query applies after vec0's cut,
// so the number they produce is the ceiling on what any window could return.
//
// `agent_id = ?2` plus `deleted_at IS NULL` is exactly the leading column and
// the partial predicate of each table's own index — `idx_memory_agent_active`,
// `idx_epi_agent_active`, `idx_proc_agent_active` (`V1__init.sql:37`, `:96`,
// `:147`) — so the count rides the matching index for whichever table this
// constant serves, and the LIMIT stops it at k rather than walking the
// agent's whole history.
const ELIGIBLE_SQL_MEMORY: &str = "
    SELECT COUNT(*) FROM (
        SELECT m.id FROM memory m
        WHERE m.status IN ('active', 'disputed')
          AND m.deleted_at IS NULL
          AND m.agent_id = ?2
          AND (?3 IS NULL OR m.project_id = ?3 OR m.project_id IS NULL OR m.memory_type = 'user_preference')
        LIMIT ?1
    )
";

const ELIGIBLE_SQL_EPISODIC: &str = "
    SELECT COUNT(*) FROM (
        SELECT epi.id FROM episodic_trajectory epi
        WHERE epi.status IN ('active', 'disputed')
          AND epi.deleted_at IS NULL
          AND epi.agent_id = ?2
          AND (?3 IS NULL OR epi.project_id = ?3 OR epi.project_id IS NULL)
        LIMIT ?1
    )
";

const ELIGIBLE_SQL_PROCEDURAL: &str = "
    SELECT COUNT(*) FROM (
        SELECT pro.id FROM procedural_skill pro
        WHERE pro.status IN ('active', 'disputed')
          AND pro.deleted_at IS NULL
          AND pro.agent_id = ?2
          AND (?3 IS NULL OR pro.project_id = ?3 OR pro.project_id IS NULL)
        LIMIT ?1
    )
";

/// vec0 refuses to bind a `k` above this — it is the engine's own ceiling, not
/// a policy choice made here. A widened window has to be clamped to it, or the
/// second pass fails outright instead of simply returning fewer rows.
const VEC0_MAX_K: usize = 4096;

/// Everything that differs between the three vector-backed tables. Every field
/// is static, so the descriptor is a `Copy` value the row-mapping closure can
/// capture — one function over three descriptors instead of three functions
/// that have to be kept identical by hand.
#[derive(Clone, Copy)]
struct KnnTable {
    sql: &'static str,
    eligible_sql: &'static str,
    kind: TableKind,
    /// Names the table in the "connection closed" message. The three branches
    /// run concurrently, so a single shared message would leave the one fact
    /// worth logging — which table lost the connection — unrecoverable.
    label: &'static str,
}

const KNN_MEMORY: KnnTable = KnnTable {
    sql: SEARCH_SQL_MEMORY,
    eligible_sql: ELIGIBLE_SQL_MEMORY,
    kind: TableKind::Memory,
    label: "knn_memory",
};

const KNN_EPISODIC: KnnTable = KnnTable {
    sql: SEARCH_SQL_EPISODIC,
    eligible_sql: ELIGIBLE_SQL_EPISODIC,
    kind: TableKind::Episodic,
    label: "knn_episodic",
};

const KNN_PROCEDURAL: KnnTable = KnnTable {
    sql: SEARCH_SQL_PROCEDURAL,
    eligible_sql: ELIGIBLE_SQL_PROCEDURAL,
    kind: TableKind::Procedural,
    label: "knn_procedural",
};

/// One table's contribution to the merged result, plus how many KNN queries
/// producing it took. Carried out rather than logged here so the retrieval
/// line stays a single record of the whole search.
///
/// A count rather than a "did it retry" flag, because the row set cannot stand
/// in for it: a recovery that widened the window repeatedly would return the
/// same rows as one that widened it once, and a flag would read `true` for
/// both. The number is the only thing that separates them, so it is the thing
/// tests assert on.
struct KnnOutcome {
    hits: Vec<RankedHit>,
    passes: usize,
}

impl KnnOutcome {
    /// A table the scope switched off: it asked for nothing, so nothing was
    /// queried and no pass ran.
    fn skipped() -> Self {
        Self { hits: Vec::new(), passes: 0 }
    }

    /// Whether recovery happened — a single pass is the ordinary path, and a
    /// table that was never queried did not recover from anything.
    fn retried(&self) -> bool {
        self.passes > 1
    }
}

/// The window a starved query is re-run with. Four times the pool gives a
/// window crowded by ineligible rows real room to recover without degenerating
/// into a full-table scan on a large corpus.
fn retry_window(k: usize) -> usize {
    k.saturating_mul(4).min(VEC0_MAX_K)
}

/// Runs one table's KNN query, recovering once from a window that the WHERE
/// clause emptied out.
///
/// vec0 hands the join exactly `min(k, live vectors)` rows, and every predicate
/// in the SQL above is applied after that cut, so a short result has two very
/// different causes and only one of them a second query can fix. The one that
/// separates them is how many rows this caller is eligible for at all — not
/// how many the table holds. On a database carrying several projects for one
/// agent those two numbers come apart permanently: the table stays full while
/// the caller's own set stays small, and a rule reading the table alone would
/// buy a second scan on every search forever to re-find rows the first scan
/// had already delivered.
///
/// So the shortfall is measured against what was reachable:
///
/// - the pool came back full → nothing was filtered, and the count is not even
///   queried;
/// - fewer came back, but that is everything the caller is eligible for → the
///   corpus does not hold the rest, whether because it is young or because the
///   rest belongs to someone else. A wider window returns the identical rows;
/// - fewer came back than the caller is eligible for → the window was crowded
///   by rows the join then threw away, and eligible rows are sitting past the
///   k-th neighbour where only a wider window reaches them.
///
/// What this does NOT promise is that the second pass is rare for every
/// caller. A caller whose eligible rows persistently rank behind a crowd of
/// ineligible ones will keep paying for the retry, and may still come back
/// short: widening is four times the pool, not the whole table. What it does
/// guarantee is that nobody pays for a second pass that provably cannot return
/// a single extra row.
///
/// That guarantee holds only if `reachable` never overcounts what a wider
/// window could actually surface, which needs every row it counts in the
/// parent table to have a matching row in the vector table — a parent row
/// without one would inflate `reachable` past what KNN can ever return, and
/// the caller would pay a second pass on every one of its searches that a
/// third or fourth pass could not fix either. Nothing in the schema enforces
/// that pairing: no trigger, no foreign key ties `memory_vectors` (or the
/// episodic/procedural equivalents) to its parent table. It holds today
/// because every production insert path writes the parent row and its vector
/// row together, in one transaction (`store_insert.rs`'s
/// `insert_memory_with_vector`, `insert_episodic_trajectory`, and the
/// procedural promote path, which short-circuits into `SkippedNoEmbedding`
/// before touching the parent row when there is no vector to pair it with),
/// and a delete only removes a row's vector after that row is already
/// soft-deleted, so a live row is never left unpaired. A future insert path
/// that skipped that pairing would break this invariant silently.
///
/// The recovery is a single branch, deliberately, not a loop that widens until
/// it fills the pool: the second pass either finds the missing rows or the
/// corpus does not hold them, and iterating would burn scans discovering that
/// a third time. `KnnOutcome::passes` carries how many queries actually ran,
/// so that stays a measured property rather than a claim. The widened result
/// is trimmed back to k so the pool stays a budget for finding candidates
/// rather than a licence to return more of them.
async fn knn_one(
    store: &MemoryStore,
    table: KnnTable,
    q_bytes: &[u8],
    agent_id: &str,
    project_id: Option<&str>,
    k: usize,
    scoring: &ScoringConfig,
    now: TimestampMs,
) -> Result<KnnOutcome, MemoryError> {
    let q = q_bytes.to_vec();
    let agent = agent_id.to_string();
    let proj = project_id.map(str::to_owned);
    let scoring = *scoring;
    store
        .conn
        .call(move |c| {
            // Probe and both passes share one hop to the DB thread. `run` is a
            // free function rather than a closure so the row mapping exists in
            // exactly one place regardless of how many passes happen.
            //
            // It owns the pass counter instead of the caller bumping it at each
            // call site: counted here, the number cannot drift from the number
            // of queries actually issued, which is the whole reason it is
            // reported at all.
            fn run(
                c: &rusqlite::Connection,
                table: KnnTable,
                q: &[u8],
                agent: &str,
                proj: &Option<String>,
                k: usize,
                passes: &mut usize,
            ) -> Result<Vec<RawHit>, rusqlite::Error> {
                *passes += 1;
                let mut stmt = c.prepare(table.sql)?;
                let rows = stmt.query_map(
                    rusqlite::params![q, k as i64, agent, proj],
                    |row| {
                        Ok(RawHit {
                            id: row.get(0)?,
                            kind: table.kind,
                            content: row.get(1)?,
                            memory_type: row.get(2)?,
                            tier: row.get(3)?,
                            status: row.get(4)?,
                            distance: row.get(5)?,
                            created_at: row.get(6)?,
                            last_accessed_at: row.get(7)?,
                            importance: row.get(8)?,
                        })
                    },
                )?;
                let mut hits = Vec::new();
                for row in rows {
                    hits.push(row?);
                }
                Ok(hits)
            }

            let mut passes = 0usize;
            let mut raw = run(c, table, &q, &agent, &proj, k, &mut passes)?;
            if raw.len() < k {
                // Capped at k by the SQL's own LIMIT, so this is
                // `min(k, eligible)` — the most this query could ever return.
                let reachable: i64 = c.query_row(
                    table.eligible_sql,
                    rusqlite::params![k as i64, &agent, &proj],
                    |row| row.get(0),
                )?;
                if raw.len() < reachable as usize {
                    raw = run(c, table, &q, &agent, &proj, retry_window(k), &mut passes)?;
                }
            }
            let mut hits: Vec<RankedHit> = raw
                .into_iter()
                .map(|r| RankedHit {
                    id: r.id,
                    kind: r.kind,
                    score: compute_score(&scoring, &r, now),
                    content: r.content,
                    memory_type: r.memory_type,
                    tier: r.tier,
                    status: r.status,
                    distance: r.distance,
                    created_at: r.created_at,
                })
                .collect();
            // Sorted before the trim, not after: the widened window is cut
            // back to k, and the rows that survive that cut have to be the
            // highest-scoring ones. The ordering used to arrive with the rows
            // because the query produced it; the query no longer computes a
            // score, so the cut would otherwise keep whichever k rows the
            // join happened to emit first.
            hits.sort_by(hit_cmp);
            hits.truncate(k);
            Ok(KnnOutcome { hits, passes })
        })
        .await
        .map_err(|e| match e {
            tokio_rusqlite::Error::Error(re) => MemoryError::Sqlite(re),
            tokio_rusqlite::Error::ConnectionClosed => {
                MemoryError::Backend(format!("connection closed during {}", table.label))
            }
            other => MemoryError::Backend(other.to_string()),
        })
}

/// Renders the pool-survival fragment of the retrieval line from what each
/// table was asked for and what it gave back.
///
/// The requested side reads the scope, never the window a starved table was
/// re-queried with: the wider window is a means of filling the pool the caller
/// asked for, and reporting it would move the denominator under the ratio so
/// that a rescued search reads as the worst one in the log. A table the scope
/// switched off contributes 0 on both sides — it asked for nothing and got
/// nothing, which is not a survival rate of zero.
fn format_pool_line(
    scope: &SearchScope,
    mem: &KnnOutcome,
    epi: &KnnOutcome,
    pro: &KnnOutcome,
) -> String {
    crate::commands::memory_admin::format_pool_survival(
        (
            scope.memory.unwrap_or(0),
            scope.episodic.unwrap_or(0),
            scope.procedural.unwrap_or(0),
        ),
        (mem.hits.len(), epi.hits.len(), pro.hits.len()),
        (mem.retried(), epi.retried(), pro.retried()),
    )
}

/// Does the actual work behind `search_all_memories`, plus hands back the
/// pool-survival line it computed along the way. Split out so a test can
/// drive the exact call site that builds that line (`format_pool_line`
/// below) through the real orchestration path instead of re-assembling it
/// from outcomes the test constructed itself — the public wrapper is nothing
/// but this function with the second half of the tuple dropped, so nothing
/// about the split changes what a caller of `search_all_memories` observes.
async fn search_all_memories_inner(
    store: &MemoryStore,
    embedding: &[f32],
    agent_id: &str,
    scope: SearchScope,
    k_final: usize,
    scoring: &ScoringConfig,
) -> Result<(Vec<RankedHit>, String), MemoryError> {
    // Same funnel every caller shares (prompt_builder.rs, contradiction.rs,
    // eval_api.rs): an unusable query vector means "no direction to search
    // with", which is a caller-input problem, not a store failure — so this
    // returns an empty result set rather than propagating `Err` and aborting
    // whatever multi-step flow called it (eval_api.rs in particular turns a
    // hard `Err` here into an aborted eval run).
    let unit = match to_unit(embedding) {
        Ok(v) => v,
        Err(e) => {
            log::warn!("search_all_memories: discarding unusable query vector: {e}");
            return Ok((Vec::new(), String::new()));
        }
    };

    let t0 = Instant::now();
    let q_bytes = serialize_f32_query(&unit);
    // Borrowed once from `scope`, then handed to every branch below by shared
    // reference — `scope` itself is never moved out of, so this reads fine
    // even though the struct dropped `Copy` when `project_id` was added.
    let project_id = scope.project_id.as_deref();
    // Read once and shared by every table: no two rows scored by this search
    // may have their decay computed against a different instant, which three
    // independent per-row `strftime('%s','now')` calls could never guarantee.
    let now = TimestampMs::now();

    let (mem, epi, pro) = tokio::try_join!(
        async {
            match scope.memory {
                Some(k) => knn_one(store, KNN_MEMORY, &q_bytes, agent_id, project_id, k, scoring, now).await,
                None => Ok(KnnOutcome::skipped()),
            }
        },
        async {
            match scope.episodic {
                Some(k) => knn_one(store, KNN_EPISODIC, &q_bytes, agent_id, project_id, k, scoring, now).await,
                None => Ok(KnnOutcome::skipped()),
            }
        },
        async {
            match scope.procedural {
                Some(k) => knn_one(store, KNN_PROCEDURAL, &q_bytes, agent_id, project_id, k, scoring, now).await,
                None => Ok(KnnOutcome::skipped()),
            }
        },
    )?;

    // Built before the merge below consumes the per-table hits, which is also
    // what lets a test assemble it from real outcomes instead of from numbers
    // it invented: the requested/returned pairing lives in one place rather
    // than being reassembled at the log call.
    let pool_survival = format_pool_line(&scope, &mem, &epi, &pro);
    let mut all: Vec<RankedHit> = mem.hits.into_iter().chain(epi.hits).chain(pro.hits).collect();
    all.sort_by(hit_cmp);
    all.truncate(k_final);

    // Read once so the per-call number logged below and the sample fed into
    // the latency window describe the same instant — a second read would
    // measure a different span.
    let elapsed = t0.elapsed();
    // Recorded unconditionally, not from inside the log::debug! args below:
    // that macro short-circuits argument evaluation when debug logging is
    // off, which is fine for the percentile snapshot but would leave the
    // window empty while debug logging is off and reporting `warming` for a
    // long time after someone turns it on.
    store.knn_latency().record(elapsed);

    // Tier-per-hit and pool-survival logging — appended to the existing
    // retrieval debug line via pure, unit-tested helpers. `snapshot()` sits
    // inside the macro args deliberately: it is the one part of this line
    // that's fine to skip sorting for when debug logging is off.
    log::debug!(
        "retrieval={}ms hits={} {} k_final={} dist={} {}",
        elapsed.as_millis(),
        all.len(),
        pool_survival,
        k_final,
        crate::commands::memory_admin::format_tier_distribution(&all),
        store.knn_latency().snapshot(),
    );

    Ok((all, pool_survival))
}

/// Top-level KNN orchestrator. Returns up to `k_final` hits sorted DESC by score.
pub(crate) async fn search_all_memories(
    store: &MemoryStore,
    embedding: &[f32],
    agent_id: &str,
    scope: SearchScope,
    k_final: usize,
) -> Result<Vec<RankedHit>, MemoryError> {
    let (hits, _pool_survival) =
        search_all_memories_inner(store, embedding, agent_id, scope, k_final, &ScoringConfig::DEFAULT).await?;
    Ok(hits)
}

/// Same orchestration as [`search_all_memories`], with the scoring config
/// exposed to the caller instead of pinned to `ScoringConfig::DEFAULT`. Every
/// production call site keeps using the unparameterized wrapper above — this
/// one exists for `rag-eval`, which needs to re-search a seeded corpus under
/// several configs without re-seeding.
pub(crate) async fn search_all_memories_with_scoring(
    store: &MemoryStore,
    embedding: &[f32],
    agent_id: &str,
    scope: SearchScope,
    k_final: usize,
    scoring: &ScoringConfig,
) -> Result<Vec<RankedHit>, MemoryError> {
    let (hits, _pool_survival) =
        search_all_memories_inner(store, embedding, agent_id, scope, k_final, scoring).await?;
    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::db::MemoryStore;
    use crate::memory::embedder::{EmbedRole, Embedder};
    use rusqlite::params;

    fn serialize_embedding(v: &[f32]) -> Vec<u8> {
        v.iter().flat_map(|f| f.to_le_bytes()).collect()
    }

    /// A unit basis vector `[1.0, 0.0, …]` — the "query == stored, so the
    /// similarity factor is neutral" fixture. An all-zero vector no longer
    /// serves that purpose: `to_unit` rejects zero magnitude, so both sides
    /// of every self-similarity test below use this basis vector instead —
    /// distance stays exactly 0 and `cosine_from_l2_unit(0.0) = 1.0`, the
    /// same neutral factor an unnormalized `(1.0 - 0)` produced before.
    fn unit_basis_vec() -> Vec<f32> {
        let mut v = vec![0.0_f32; 384];
        v[0] = 1.0;
        v
    }

    fn unit_basis_embedding() -> Vec<u8> {
        serialize_embedding(&unit_basis_vec())
    }

    async fn insert_memory_row(
        store: &MemoryStore,
        agent_id: &str,
        content: &str,
        memory_type: &str,
        tier: i64,
        importance: f64,
        status: &str,
        embedding: Vec<u8>,
    ) -> i64 {
        let agent = agent_id.to_string();
        let content = content.to_string();
        let memory_type = memory_type.to_string();
        let status = status.to_string();
        store
            .conn
            .call(move |c| -> Result<i64, rusqlite::Error> {
                let tx = c.transaction()?;
                tx.execute(
                    "INSERT INTO memory (scope, agent_id, content, memory_type, source,
                     created_at, created_by_type, created_by_ref,
                     last_modified_at, last_accessed_at, importance, tier, status)
                     VALUES ('agent', ?1, ?2, ?3, 'chat', 0, 'user', ?1, 0, 0, ?4, ?5, ?6)",
                    params![agent, content, memory_type, importance, tier, status],
                )?;
                let id = tx.last_insert_rowid();
                tx.execute(
                    "INSERT INTO memory_vectors(rowid, embedding) VALUES (?, ?)",
                    params![id, embedding],
                )?;
                tx.commit()?;
                Ok(id)
            })
            .await
            .unwrap()
    }

    #[test]
    fn scope_all_default_values() {
        let s = SearchScope::all_default();
        assert_eq!(s.memory, Some(30));
        assert_eq!(s.episodic, Some(5));
        assert_eq!(s.procedural, Some(5));
    }

    #[test]
    fn search_scope_with_project_sets_field() {
        let s = SearchScope::all_default().with_project(Some("proj-1"));
        assert_eq!(s.project_id, Some("proj-1".to_string()));
    }

    #[test]
    fn ctors_default_to_no_project() {
        assert_eq!(SearchScope::all_default().project_id, None);
    }

    #[test]
    fn ranked_hit_is_send() {
        fn _s<T: Send>() {}
        _s::<RankedHit>();
    }

    #[test]
    fn ranked_hit_sort_descending_by_score() {
        let h1 = RankedHit {
            id: 1,
            kind: TableKind::Memory,
            content: "a".into(),
            memory_type: "chat_message".into(),
            tier: 3,
            status: "active".into(),
            score: 0.9,
            distance: 0.1,
            created_at: TimestampMs::from_unix_millis(0),
        };
        let h2 = RankedHit {
            id: 2,
            kind: TableKind::Memory,
            content: "b".into(),
            memory_type: "chat_message".into(),
            tier: 3,
            status: "active".into(),
            score: 0.3,
            distance: 0.7,
            created_at: TimestampMs::from_unix_millis(0),
        };
        let mut hits = vec![h2, h1];
        hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        assert!((hits[0].score - 0.9).abs() < 1e-9);
    }

    // Score first, then a key that cannot repeat: id is unique per table and
    // kind separates the three, so no two distinct hits may compare Equal —
    // ties on score alone (same id, different kind is impossible in
    // practice, but the comparator must still resolve it) fall through to
    // kind, and ties on both score and kind fall through to id.
    #[test]
    fn hit_cmp_is_a_strict_total_order_even_on_score_ties() {
        use std::cmp::Ordering;

        fn make(id: i64, kind: TableKind, score: f64) -> RankedHit {
            RankedHit {
                id,
                kind,
                content: String::new(),
                memory_type: String::new(),
                tier: 3,
                status: "active".into(),
                score,
                distance: 0.0,
                created_at: TimestampMs::from_unix_millis(0),
            }
        }

        let a = make(1, TableKind::Memory, 0.5);
        let b = make(2, TableKind::Memory, 0.5); // tied score+kind, id breaks it
        let c = make(1, TableKind::Episodic, 0.5); // tied score+id, kind breaks it
        let d = make(3, TableKind::Procedural, 0.9); // higher score sorts first

        assert_eq!(hit_cmp(&a, &b), Ordering::Less, "same score+kind: lower id sorts first");
        assert_eq!(hit_cmp(&b, &a), Ordering::Greater);
        assert_eq!(hit_cmp(&a, &c), Ordering::Less, "same score+id: Memory sorts before Episodic");
        assert_eq!(hit_cmp(&c, &a), Ordering::Greater);
        assert_eq!(hit_cmp(&d, &a), Ordering::Less, "higher score sorts first regardless of kind/id");

        assert_ne!(hit_cmp(&a, &b), Ordering::Equal, "distinct hits must never compare Equal");
        assert_ne!(hit_cmp(&a, &c), Ordering::Equal, "distinct hits must never compare Equal");
        assert_eq!(hit_cmp(&a, &a), Ordering::Equal, "a hit compares Equal to itself");
    }

    #[test]
    fn serialize_f32_query_produces_le_bytes() {
        let v = vec![1.0_f32, 0.0_f32];
        let bytes = serialize_f32_query(&v);
        assert_eq!(&bytes[0..4], &1.0_f32.to_le_bytes());
    }

    #[test]
    fn serialize_f32_query_length_matches() {
        let v = vec![0.0_f32; 384];
        let bytes = serialize_f32_query(&v);
        assert_eq!(bytes.len(), 384 * 4);
    }

    // Tier 1 is the hottest tier, not an excluded one. A row lands there by
    // being important or by being a type that is always kept close, and the
    // KNN pool has to be able to reach it — the deterministic workspace
    // channel only carries a fixed slice of tier 1 (importance >= 8 and the
    // top few skills), so anything the pool refuses to scan is delivered by
    // nobody at all.
    #[tokio::test]
    async fn tier1_rows_are_reachable_by_knn() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let emb = unit_basis_embedding();

        // Insert one tier=1 row and three tier=3 rows
        insert_memory_row(&store, "agent-test", "tier1 content", "chat_message", 1, 5.0, "active", emb.clone()).await;
        insert_memory_row(&store, "agent-test", "tier3 a", "chat_message", 3, 5.0, "active", emb.clone()).await;
        insert_memory_row(&store, "agent-test", "tier3 b", "chat_message", 3, 5.0, "active", emb.clone()).await;
        insert_memory_row(&store, "agent-test", "tier3 c", "chat_message", 3, 5.0, "active", emb.clone()).await;

        let q_vec = unit_basis_vec();
        let hits = search_all_memories(&store, &q_vec, "agent-test", SearchScope::all_default(), 10)
            .await
            .unwrap();

        assert!(
            hits.iter().any(|h| h.tier == 1),
            "a tier=1 row must be reachable by KNN"
        );
        assert_eq!(hits.len(), 4, "expected all 4 rows, tier-1 included");

        // The assertion above can't distinguish "the query correctly returned
        // 4" from "the query returned something else and a later truncation
        // coincidentally landed on 4". Calling the pool query directly, before
        // any merge or truncation, tests the scan set in isolation.
        assert_eq!(
            knn_one(&store, KNN_MEMORY, &emb, "agent-test", None, 30, &ScoringConfig::DEFAULT, TimestampMs::now()).await.unwrap().hits.len(),
            4,
            "the tier-1 row must survive inside the query, before any merge or truncation"
        );
    }

    // `procedural_skill` rows are born at tier 1 (`compute_tier` pins that
    // type there) and nothing demotes them, so a pool that skipped tier 1
    // returned zero skills for every query ever issued — the miner's whole
    // output was unreachable through this channel.
    #[tokio::test]
    async fn knn_procedural_returns_skill_rows() {
        use crate::memory::embedder::FakeEmbedder;
        use crate::memory::store_insert::{NewProceduralSkillRecord, PromoteOutcome};

        let store = MemoryStore::new_for_test().await.unwrap();
        let embedder = FakeEmbedder::default();
        let trigger = "un test falla después de editar";
        let q_vec = embedder.embed_with_role(trigger, EmbedRole::Passage).await.unwrap();

        let outcome = store
            .promote_to_procedural_skill(
                NewProceduralSkillRecord {
                    agent_id: "agent-skill".to_string(),
                    project_id: None,
                    skill_name: "reparar-test-roto".to_string(),
                    trigger: trigger.to_string(),
                    tool_sequence_json: "[\"read\",\"edit\",\"test\"]".to_string(),
                    source_trajectory_ids: vec![1, 2, 3],
                    importance: 8.0,
                    now_ms: 1_700_000_000_000,
                },
                Some(q_vec.clone()),
            )
            .await
            .unwrap();
        let id = match outcome {
            PromoteOutcome::Inserted(id) => id,
            other => panic!("expected a fresh insert, got {other:?}"),
        };

        let q_bytes = serialize_f32_query(&to_unit(&q_vec).unwrap());
        assert_eq!(
            knn_one(&store, KNN_PROCEDURAL, &q_bytes, "agent-skill", None, 5, &ScoringConfig::DEFAULT, TimestampMs::now())
                .await
                .unwrap()
                .hits
                .len(),
            1,
            "the procedural pool must return the skill row"
        );

        let hits = search_all_memories(&store, &q_vec, "agent-skill", SearchScope::all_default(), 10)
            .await
            .unwrap();
        let hit = hits
            .iter()
            .find(|h| h.id == id && h.kind == TableKind::Procedural)
            .expect("the skill must reach the merged result set");
        assert_eq!(hit.tier, 1, "skills live at tier 1 — that is the point of this test");
        assert_eq!(hit.memory_type, "procedure");
    }

    // A query vector reaching `search_all_memories` at any magnitude must be
    // compared in the same unit space every stored vector lives in — otherwise
    // "distance" mixes an arbitrary caller-side scale with the store's, which
    // is meaningless. Stores a unit vector directly (bypassing the insert
    // path's own normalization, which is under test elsewhere) and queries
    // with the same direction scaled ×10.
    #[tokio::test]
    async fn non_unit_query_is_normalized_by_search() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let mut unit = vec![0.0_f32; 384];
        unit[0] = 1.0;
        let stored_bytes = serialize_embedding(&unit);
        let id = insert_memory_row(
            &store, "agent-normalize", "content", "chat_message", 3, 5.0, "active", stored_bytes,
        )
        .await;

        let query: Vec<f32> = unit.iter().map(|x| x * 10.0).collect();
        let hits = search_all_memories(&store, &query, "agent-normalize", SearchScope::all_default(), 10)
            .await
            .unwrap();

        let hit = hits.iter().find(|h| h.id == id).expect("expected the row to be retrievable");
        assert!(
            hit.distance < 1e-3,
            "query vector must be normalized before the KNN match: distance={}",
            hit.distance
        );
    }

    // The query-side counterpart of insertion's zero-vector rejection
    // (store_insert.rs). `to_unit` rejects a zero-magnitude query vector at
    // search.rs:302-308, and that branch fails soft (`warn!` + `Ok(vec![])`)
    // rather than propagating `Err` — `eval_api.rs` turns an `Err` here into
    // an aborted eval run, so an unusable query vector must degrade to "no
    // hits" instead of tearing down the caller. A row is inserted first so
    // the assertion proves the search legitimately found nothing (not that
    // the table was empty to begin with).
    #[tokio::test]
    async fn zero_query_vector_is_discarded_fail_soft_not_propagated_as_error() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let emb = unit_basis_embedding();
        insert_memory_row(&store, "agent-zero-query", "content", "chat_message", 3, 5.0, "active", emb).await;

        let zero_query = vec![0.0_f32; 384];
        let hits = search_all_memories(&store, &zero_query, "agent-zero-query", SearchScope::all_default(), 10)
            .await
            .expect("an unusable query vector must fail soft (Ok(vec![])), not propagate Err");

        assert!(
            hits.is_empty(),
            "a zero query vector carries no direction to search with, so it must return zero hits, \
             not the row above at some fallback distance"
        );
    }

    #[tokio::test]
    async fn superseded_rows_excluded_from_results() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let emb = unit_basis_embedding();

        insert_memory_row(&store, "agent-test", "superseded", "chat_message", 3, 5.0, "superseded", emb.clone()).await;
        insert_memory_row(&store, "agent-test", "active", "chat_message", 3, 5.0, "active", emb.clone()).await;

        let q_vec = unit_basis_vec();
        let hits = search_all_memories(&store, &q_vec, "agent-test", SearchScope::all_default(), 10)
            .await
            .unwrap();

        assert!(hits.iter().all(|h| h.status != "superseded"), "superseded rows must not appear");
        assert_eq!(hits.len(), 1);
    }

    #[tokio::test]
    async fn disputed_rows_included_with_degraded_score() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let emb = unit_basis_embedding();

        insert_memory_row(&store, "agent-test", "disputed content", "chat_message", 3, 5.0, "disputed", emb.clone()).await;
        insert_memory_row(&store, "agent-test", "active content", "chat_message", 3, 5.0, "active", emb.clone()).await;

        let q_vec = unit_basis_vec();
        let hits = search_all_memories(&store, &q_vec, "agent-test", SearchScope::all_default(), 10)
            .await
            .unwrap();

        let disputed = hits.iter().find(|h| h.status == "disputed");
        assert!(disputed.is_some(), "disputed row must appear in results");
    }

    #[tokio::test]
    async fn score_formula_integration_spot_check() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let emb = unit_basis_embedding();

        // Insert row with tier=2, memory_type='chat_message', importance=5.0, status='active'
        // The `last_accessed_at` is set to 0 in insert helper, so age will be large → use a recent value
        let agent = "agent-spot";
        let emb_clone = emb.clone();
        store.conn.call(move |c| -> Result<(), rusqlite::Error> {
            let tx = c.transaction()?;
            tx.execute(
                "INSERT INTO memory (scope, agent_id, content, memory_type, source,
                 created_at, created_by_type, created_by_ref,
                 last_modified_at, last_accessed_at, importance, tier, status)
                 VALUES ('agent', ?1, 'spot check content', 'chat_message', 'chat',
                 0, 'user', ?1, 0, (strftime('%s','now')*1000) - 86400000, 5.0, 2, 'active')",
                params![agent],
            )?;
            let id = tx.last_insert_rowid();
            tx.execute(
                "INSERT INTO memory_vectors(rowid, embedding) VALUES (?, ?)",
                params![id, emb_clone],
            )?;
            tx.commit()?;
            Ok(())
        }).await.unwrap();

        let q_vec = unit_basis_vec();
        let hits = search_all_memories(&store, &q_vec, agent, SearchScope::all_default(), 10)
            .await
            .unwrap();

        assert!(!hits.is_empty(), "expected at least 1 hit");
        let score = hits[0].score;
        // score ≈ cosine * MAX(tier_floor, exp(-lambda*age_days)) * importance * dispute,
        // computed in Rust by score() (memory/score.rs) against ScoringConfig::DEFAULT —
        // distance ≈ 0 gives cosine ≈ 1.0 both before and after the cosine-floor
        // re-scale (a floor rescale maps 1.0 to 1.0 regardless of the floor), decay
        // over 1 day at the chat_message lambda (0.023) ≈ 0.977 clears the tier-2
        // floor (0.2), importance at 5.0 = 0.8333... + 5.0/60.0 ≈ 0.9167, dispute for
        // 'active' = 1.0. So score ≈ 1.0 * 0.977 * 0.9167 * 1.0 ≈ 0.896.
        assert!(score > 0.0 && score <= 1.0, "score out of range: {score}");
        assert!(score > 0.5, "expected score > 0.5, got {score}");
    }

    #[tokio::test]
    async fn fresh_last_accessed_outranks_stale_last_accessed_when_otherwise_identical() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let emb = unit_basis_embedding();
        let agent = "agent-recency-discriminates";

        // Both rows share the same embedding, tier, importance, status and
        // memory_type — last_accessed_at is the only input that differs, so any
        // score gap between them is attributable to the decay term alone.
        let emb_fresh = emb.clone();
        let a_fresh = agent.to_string();
        let fresh_id = store.conn.call(move |c| -> Result<i64, rusqlite::Error> {
            let tx = c.transaction()?;
            tx.execute(
                "INSERT INTO memory (scope, agent_id, content, memory_type, source,
                 created_at, created_by_type, created_by_ref,
                 last_modified_at, last_accessed_at, importance, tier, status)
                 VALUES ('agent', ?1, 'fresh row', 'chat_message', 'chat',
                 0, 'user', ?1, 0, strftime('%s','now') * 1000, 5.0, 2, 'active')",
                params![a_fresh],
            )?;
            let id = tx.last_insert_rowid();
            tx.execute(
                "INSERT INTO memory_vectors(rowid, embedding) VALUES (?, ?)",
                params![id, emb_fresh],
            )?;
            tx.commit()?;
            Ok(id)
        }).await.unwrap();

        let emb_stale = emb.clone();
        let a_stale = agent.to_string();
        let stale_id = store.conn.call(move |c| -> Result<i64, rusqlite::Error> {
            let tx = c.transaction()?;
            tx.execute(
                "INSERT INTO memory (scope, agent_id, content, memory_type, source,
                 created_at, created_by_type, created_by_ref,
                 last_modified_at, last_accessed_at, importance, tier, status)
                 VALUES ('agent', ?1, 'stale row', 'chat_message', 'chat',
                 0, 'user', ?1, 0, (strftime('%s','now') * 1000) - (30 * 86400000), 5.0, 2, 'active')",
                params![a_stale],
            )?;
            let id = tx.last_insert_rowid();
            tx.execute(
                "INSERT INTO memory_vectors(rowid, embedding) VALUES (?, ?)",
                params![id, emb_stale],
            )?;
            tx.commit()?;
            Ok(id)
        }).await.unwrap();

        let q_vec = unit_basis_vec();
        let hits = search_all_memories(&store, &q_vec, agent, SearchScope::all_default(), 10)
            .await
            .unwrap();

        let fresh_hit = hits.iter().find(|h| h.id == fresh_id).expect("fresh row must be retrievable");
        let stale_hit = hits.iter().find(|h| h.id == stale_id).expect("stale row must be retrievable");

        // Identical embeddings mean the KNN distance contribution is identical
        // too — isolating last_accessed_at as the only variable left standing
        // between the two scores.
        assert_eq!(
            fresh_hit.distance, stale_hit.distance,
            "both rows share the same embedding, so distance must be identical"
        );
        assert!(
            fresh_hit.score > stale_hit.score,
            "a row accessed moments ago must score strictly higher than an otherwise-identical \
             row accessed 30 days ago: fresh={} stale={}",
            fresh_hit.score,
            stale_hit.score
        );
    }

    #[tokio::test]
    async fn disputed_status_reduces_score_by_factor_0_6() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let emb = unit_basis_embedding();
        let agent = "agent-dispute-test";

        // Insert active version
        let emb2 = emb.clone();
        let a2 = agent.to_string();
        let active_id = store.conn.call(move |c| -> Result<i64, rusqlite::Error> {
            let tx = c.transaction()?;
            tx.execute(
                "INSERT INTO memory (scope, agent_id, content, memory_type, source,
                 created_at, created_by_type, created_by_ref,
                 last_modified_at, last_accessed_at, importance, tier, status)
                 VALUES ('agent', ?1, 'active version', 'chat_message', 'chat',
                 0, 'user', ?1, 0, (strftime('%s','now')*1000) - 86400000, 5.0, 3, 'active')",
                params![a2],
            )?;
            let id = tx.last_insert_rowid();
            tx.execute("INSERT INTO memory_vectors(rowid, embedding) VALUES (?, ?)", params![id, emb2])?;
            tx.commit()?;
            Ok(id)
        }).await.unwrap();

        let q_vec = unit_basis_vec();
        let active_hits = search_all_memories(&store, &q_vec, agent, SearchScope::all_default(), 10)
            .await.unwrap();
        let active_score = active_hits.iter().find(|h| h.id == active_id).unwrap().score;

        // Insert disputed version
        let emb3 = emb.clone();
        let a3 = agent.to_string();
        let disputed_id = store.conn.call(move |c| -> Result<i64, rusqlite::Error> {
            let tx = c.transaction()?;
            tx.execute(
                "INSERT INTO memory (scope, agent_id, content, memory_type, source,
                 created_at, created_by_type, created_by_ref,
                 last_modified_at, last_accessed_at, importance, tier, status)
                 VALUES ('agent', ?1, 'disputed version', 'chat_message', 'chat',
                 0, 'user', ?1, 0, (strftime('%s','now')*1000) - 86400000, 5.0, 3, 'disputed')",
                params![a3],
            )?;
            let id = tx.last_insert_rowid();
            tx.execute("INSERT INTO memory_vectors(rowid, embedding) VALUES (?, ?)", params![id, emb3])?;
            tx.commit()?;
            Ok(id)
        }).await.unwrap();

        let all_hits = search_all_memories(&store, &q_vec, agent, SearchScope::all_default(), 10)
            .await.unwrap();
        let disputed_score = all_hits.iter().find(|h| h.id == disputed_id).unwrap().score;

        assert!(disputed_score <= active_score * 0.61,
            "disputed score {disputed_score} should be <= 0.6 * active score {active_score}");
    }

    #[tokio::test]
    async fn search_returns_results_sorted_by_score_desc() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let emb = unit_basis_embedding();
        let agent = "agent-sort";

        for i in 0..3i64 {
            let emb_clone = emb.clone();
            let a = agent.to_string();
            let imp = (i + 1) as f64 * 2.0 + 4.0; // 6.0, 8.0, 10.0
            store.conn.call(move |c| -> Result<(), rusqlite::Error> {
                let tx = c.transaction()?;
                tx.execute(
                    "INSERT INTO memory (scope, agent_id, content, memory_type, source,
                     created_at, created_by_type, created_by_ref,
                     last_modified_at, last_accessed_at, importance, tier, status)
                     VALUES ('agent', ?1, 'content', 'chat_message', 'chat',
                     0, 'user', ?1, 0, (strftime('%s','now')*1000) - 86400000, ?2, 3, 'active')",
                    params![a, imp],
                )?;
                let id = tx.last_insert_rowid();
                tx.execute("INSERT INTO memory_vectors(rowid, embedding) VALUES (?, ?)", params![id, emb_clone])?;
                tx.commit()?;
                Ok(())
            }).await.unwrap();
        }

        let q_vec = unit_basis_vec();
        let hits = search_all_memories(&store, &q_vec, agent, SearchScope::all_default(), 3)
            .await.unwrap();

        assert_eq!(hits.len(), 3);
        assert!(hits[0].score >= hits[1].score, "must be sorted DESC");
        assert!(hits[1].score >= hits[2].score, "must be sorted DESC");
    }

    #[tokio::test]
    async fn search_respects_scope_none_skips_table() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let q_vec = unit_basis_vec();
        let scope = SearchScope { memory: None, episodic: None, procedural: None, project_id: None };
        let hits = search_all_memories(&store, &q_vec, "agent-x", scope, 10)
            .await.unwrap();
        assert!(hits.is_empty(), "all scopes None → empty results");
    }

    #[tokio::test]
    async fn search_truncates_to_k_final() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let emb = unit_basis_embedding();
        let agent = "agent-trunc";

        for _ in 0..5 {
            let emb_clone = emb.clone();
            let a = agent.to_string();
            store.conn.call(move |c| -> Result<(), rusqlite::Error> {
                let tx = c.transaction()?;
                tx.execute(
                    "INSERT INTO memory (scope, agent_id, content, memory_type, source,
                     created_at, created_by_type, created_by_ref,
                     last_modified_at, last_accessed_at, importance, tier, status)
                     VALUES ('agent', ?1, 'content', 'chat_message', 'chat',
                     0, 'user', ?1, 0, 0, 5.0, 3, 'active')",
                    params![a],
                )?;
                let id = tx.last_insert_rowid();
                tx.execute("INSERT INTO memory_vectors(rowid, embedding) VALUES (?, ?)", params![id, emb_clone])?;
                tx.commit()?;
                Ok(())
            }).await.unwrap();
        }

        let q_vec = unit_basis_vec();
        let hits = search_all_memories(&store, &q_vec, agent, SearchScope::all_default(), 2)
            .await.unwrap();

        assert_eq!(hits.len(), 2, "k_final=2 must return exactly 2 hits");

        // The assertion above can't distinguish correct truncation from a
        // pool that was already short before truncation ran. Asserting the
        // pre-truncation pool size directly proves k_final is what cuts the
        // result, not a starved pool.
        assert_eq!(
            knn_one(&store, KNN_MEMORY, &emb, agent, None, 30, &ScoringConfig::DEFAULT, TimestampMs::now()).await.unwrap().hits.len(),
            5,
            "all five must survive the query; k_final is what cuts the result, not the pool"
        );
    }

    // ── KNN latency isolation and recording ──

    #[tokio::test]
    async fn two_stores_do_not_share_latency_samples() {
        let store_a = MemoryStore::new_for_test().await.unwrap();
        let store_b = MemoryStore::new_for_test().await.unwrap();
        let q_vec = unit_basis_vec();

        search_all_memories(&store_a, &q_vec, "agent-iso-a", SearchScope::all_default(), 10)
            .await
            .unwrap();

        assert_eq!(
            store_b.knn_latency().snapshot().n,
            0,
            "a separate store must not observe another store's latency samples"
        );
    }

    #[tokio::test]
    async fn cloned_store_shares_latency_samples() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let clone = store.clone();
        let q_vec = unit_basis_vec();

        search_all_memories(&store, &q_vec, "agent-iso-clone", SearchScope::all_default(), 10)
            .await
            .unwrap();

        assert_eq!(
            clone.knn_latency().snapshot().n,
            1,
            "a clone addresses the same database, so it must observe the same recorder"
        );
    }

    #[tokio::test]
    async fn each_search_records_exactly_one_sample() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let q_vec = unit_basis_vec();

        for _ in 0..3 {
            search_all_memories(&store, &q_vec, "agent-iso-n", SearchScope::all_default(), 10)
                .await
                .unwrap();
        }

        assert_eq!(store.knn_latency().snapshot().n, 3);
    }

    #[tokio::test]
    async fn search_empty_db_returns_empty_vec() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let q_vec = unit_basis_vec();
        let hits = search_all_memories(&store, &q_vec, "agent-x", SearchScope::all_default(), 10)
            .await.unwrap();
        assert!(hits.is_empty());
    }

    #[tokio::test]
    async fn integration_5_rows_fake_embedder_top3_sorted() {
        use crate::memory::embedder::FakeEmbedder;
        let store = MemoryStore::new_for_test().await.unwrap();
        let agent = "agent-integration";

        // Use FakeEmbedder to generate an embedding for a fixed text and store
        // that SAME direction so distance will be small/zero (≈ same embedding).
        // `insert_memory_row` bypasses `insert_memory_with_vector` (raw SQL, no
        // `to_unit`), but `search_all_memories` now normalizes the query side —
        // so the stored fixture is normalized here too, or the two would compare
        // FakeEmbedder's raw ~11.3 magnitude against a unit query and never match.
        let embedder = FakeEmbedder::default();
        let q_vec = embedder.embed_with_role("test", EmbedRole::Passage).await.unwrap();
        let emb_bytes = serialize_f32_query(&to_unit(&q_vec).unwrap());

        // Insert 5 rows: 2 tier=3, 1 tier=1, 2 tier=2
        let rows = vec![
            ("tier3-a", "chat_message", 3i64, 5.0f64, "active"),
            ("tier3-b", "user_preference", 3, 7.0, "active"),
            ("tier1-c", "chat_message", 1, 9.0, "active"),
            ("tier2-d", "chat_message", 2, 5.0, "active"),
            ("tier2-e", "user_preference", 2, 8.0, "active"),
        ];

        for (content, mtype, tier, imp, status) in rows {
            insert_memory_row(&store, agent, content, mtype, tier, imp, status, emb_bytes.clone()).await;
        }

        let hits = search_all_memories(&store, &q_vec, agent, SearchScope::all_default(), 3)
            .await.unwrap();

        // All five rows are candidates — this asserts the merge-and-truncate
        // contract, not which tier survives it. Which tiers the pool scans is
        // asserted directly in `tier1_rows_are_reachable_by_knn`.
        assert_eq!(hits.len(), 3, "expected exactly 3 hits");
        // When query vec == stored vec, distance ≈ 0, so (1-distance) ≈ 1 → score > 0
        assert!(hits.iter().all(|h| h.score > 0.0), "scores must be positive");
        assert!(hits[0].score >= hits[1].score, "sorted DESC");
        assert!(hits[1].score >= hits[2].score, "sorted DESC");
    }

    // ── episodic_trajectory KNN retrieval ──
    //
    // No production code change here: SEARCH_SQL_EPISODIC's `epi.agent_id = ?3`
    // filter already supports `episodic_trajectory` rows without modification.
    // These tests exist to prove that, not to add new behavior.

    async fn insert_episodic_row_for_knn(
        store: &MemoryStore,
        agent_id: &str,
        task: &str,
        embedding: Vec<f32>,
    ) -> i64 {
        use crate::memory::store_insert::{CreatedByType, NewEpisodicTrajectoryRecord};
        let record = NewEpisodicTrajectoryRecord {
            scope: "agent",
            agent_id: Some(agent_id.to_string()),
            project_id: None,
            session_id: format!("sess-knn-{agent_id}"),
            task: task.to_string(),
            tool_calls_json: "[]".to_string(),
            errors_json: "[]".to_string(),
            resolution: None,
            success_score: 0.9,
            created_by_type: CreatedByType::System,
            created_by_ref: agent_id.to_string(),
            importance: 5.0,
            created_at_ms: 0,
        };
        store.insert_episodic_trajectory(record, embedding).await.unwrap()
    }

    // Row with agent_id="agent-a" is KNN-retrievable when the query is
    // searched with the SAME agent_id; surfaced as memory_type='episode'.
    #[tokio::test]
    async fn episodic_trajectory_retrievable_by_same_agent_id() {
        use crate::memory::embedder::FakeEmbedder;
        let store = MemoryStore::new_for_test().await.unwrap();
        let embedder = FakeEmbedder::default();
        let task = "depurar timeout de red";
        let emb = embedder.embed_with_role(task, EmbedRole::Passage).await.unwrap();
        let id = insert_episodic_row_for_knn(&store, "agent-a", task, emb.clone()).await;

        let hits = search_all_memories(&store, &emb, "agent-a", SearchScope::all_default(), 10)
            .await
            .unwrap();

        let hit = hits
            .iter()
            .find(|h| h.id == id)
            .expect("expected episodic row to be retrievable for the same agent_id");
        assert_eq!(hit.memory_type, "episode", "episodic rows must surface memory_type='episode'");
    }

    // The SAME row is NOT retrievable for a DIFFERENT agent_id — expected
    // behavior (filter exclusion), not a regression.
    #[tokio::test]
    async fn episodic_trajectory_not_retrievable_by_different_agent_id() {
        use crate::memory::embedder::FakeEmbedder;
        let store = MemoryStore::new_for_test().await.unwrap();
        let embedder = FakeEmbedder::default();
        let task = "depurar timeout de red";
        let emb = embedder.embed_with_role(task, EmbedRole::Passage).await.unwrap();
        let id = insert_episodic_row_for_knn(&store, "agent-a", task, emb.clone()).await;

        let hits = search_all_memories(&store, &emb, "agent-b", SearchScope::all_default(), 10)
            .await
            .unwrap();

        assert!(
            !hits.iter().any(|h| h.id == id),
            "row must NOT be retrievable for a different agent_id — each KNN query filters \
             agent_id = ?3, so cross-agent isolation holds even for episodic rows"
        );
    }

    // ── project scoping ──

    async fn insert_memory_row_with_project(
        store: &MemoryStore,
        agent_id: &str,
        project_id: Option<&str>,
        memory_type: &str,
        content: &str,
        embedding: Vec<u8>,
    ) -> i64 {
        let agent = agent_id.to_string();
        let project = project_id.map(|p| p.to_string());
        let memory_type = memory_type.to_string();
        let content = content.to_string();
        store
            .conn
            .call(move |c| -> Result<i64, rusqlite::Error> {
                let tx = c.transaction()?;
                tx.execute(
                    "INSERT INTO memory (scope, agent_id, project_id, content, memory_type, source,
                     created_at, created_by_type, created_by_ref,
                     last_modified_at, last_accessed_at, importance, tier, status)
                     VALUES ('agent', ?1, ?2, ?3, ?4, 'chat',
                     0, 'user', ?1, 0, 0, 5.0, 3, 'active')",
                    params![agent, project, content, memory_type],
                )?;
                let id = tx.last_insert_rowid();
                tx.execute(
                    "INSERT INTO memory_vectors(rowid, embedding) VALUES (?, ?)",
                    params![id, embedding],
                )?;
                tx.commit()?;
                Ok(id)
            })
            .await
            .unwrap()
    }

    async fn insert_episodic_row_with_project(
        store: &MemoryStore,
        agent_id: &str,
        project_id: Option<&str>,
        session_id: &str,
        embedding: Vec<f32>,
    ) -> i64 {
        use crate::memory::store_insert::{CreatedByType, NewEpisodicTrajectoryRecord};
        let record = NewEpisodicTrajectoryRecord {
            scope: "agent",
            agent_id: Some(agent_id.to_string()),
            project_id: project_id.map(|p| p.to_string()),
            session_id: session_id.to_string(),
            task: "scoped task".to_string(),
            tool_calls_json: "[]".to_string(),
            errors_json: "[]".to_string(),
            resolution: None,
            success_score: 0.9,
            created_by_type: CreatedByType::System,
            created_by_ref: agent_id.to_string(),
            importance: 5.0,
            created_at_ms: 0,
        };
        store.insert_episodic_trajectory(record, embedding).await.unwrap()
    }

    async fn insert_procedural_row_with_project(
        store: &MemoryStore,
        agent_id: &str,
        project_id: Option<&str>,
        skill_name: &str,
        embedding: Vec<f32>,
    ) -> i64 {
        use crate::memory::store_insert::{NewProceduralSkillRecord, PromoteOutcome};
        let outcome = store
            .promote_to_procedural_skill(
                NewProceduralSkillRecord {
                    agent_id: agent_id.to_string(),
                    project_id: project_id.map(|p| p.to_string()),
                    skill_name: skill_name.to_string(),
                    trigger: format!("trigger-{skill_name}"),
                    // Dedup key is (agent_id, tool_sequence): distinct per skill_name
                    // so two rows for the same agent in this test don't collapse
                    // into one UPDATE instead of two separate inserts.
                    tool_sequence_json: format!("[\"read\",\"{skill_name}\"]"),
                    source_trajectory_ids: vec![1],
                    importance: 8.0,
                    now_ms: 0,
                },
                Some(embedding),
            )
            .await
            .unwrap();
        match outcome {
            PromoteOutcome::Inserted(id) => id,
            other => panic!("expected a fresh insert, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn foreign_project_row_not_returned_memory() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let emb = unit_basis_embedding();
        let own = insert_memory_row_with_project(
            &store, "agent-scope", Some("P1"), "chat_message", "own project row", emb.clone(),
        ).await;
        let foreign = insert_memory_row_with_project(
            &store, "agent-scope", Some("P2"), "chat_message", "foreign project row", emb.clone(),
        ).await;

        let q_vec = unit_basis_vec();
        let scope = SearchScope::all_default().with_project(Some("P1"));
        let hits = search_all_memories(&store, &q_vec, "agent-scope", scope, 10).await.unwrap();

        assert!(hits.iter().any(|h| h.id == own), "own-project memory row must appear");
        assert!(
            !hits.iter().any(|h| h.id == foreign),
            "foreign-project memory row must not appear when scoped to a different project"
        );
    }

    #[tokio::test]
    async fn foreign_project_row_not_returned_episodic() {
        use crate::memory::embedder::FakeEmbedder;
        let store = MemoryStore::new_for_test().await.unwrap();
        let embedder = FakeEmbedder::default();
        let emb = embedder.embed_with_role("scoped episodic task", EmbedRole::Passage).await.unwrap();
        let own = insert_episodic_row_with_project(
            &store, "agent-scope", Some("P1"), "sess-epi-own", emb.clone(),
        ).await;
        let foreign = insert_episodic_row_with_project(
            &store, "agent-scope", Some("P2"), "sess-epi-foreign", emb.clone(),
        ).await;

        let scope = SearchScope::all_default().with_project(Some("P1"));
        let hits = search_all_memories(&store, &emb, "agent-scope", scope, 10).await.unwrap();

        assert!(hits.iter().any(|h| h.id == own && h.kind == TableKind::Episodic), "own-project episodic row must appear");
        assert!(
            !hits.iter().any(|h| h.id == foreign && h.kind == TableKind::Episodic),
            "foreign-project episodic row must not appear when scoped to a different project"
        );
    }

    #[tokio::test]
    async fn foreign_project_row_not_returned_procedural() {
        use crate::memory::embedder::FakeEmbedder;
        let store = MemoryStore::new_for_test().await.unwrap();
        let embedder = FakeEmbedder::default();
        let emb = embedder.embed_with_role("scoped procedural trigger", EmbedRole::Passage).await.unwrap();
        let own = insert_procedural_row_with_project(
            &store, "agent-scope", Some("P1"), "skill-own", emb.clone(),
        ).await;
        let foreign = insert_procedural_row_with_project(
            &store, "agent-scope", Some("P2"), "skill-foreign", emb.clone(),
        ).await;

        let scope = SearchScope::all_default().with_project(Some("P1"));
        let hits = search_all_memories(&store, &emb, "agent-scope", scope, 10).await.unwrap();

        assert!(hits.iter().any(|h| h.id == own && h.kind == TableKind::Procedural), "own-project procedural row must appear");
        assert!(
            !hits.iter().any(|h| h.id == foreign && h.kind == TableKind::Procedural),
            "foreign-project procedural row must not appear when scoped to a different project"
        );
    }

    #[tokio::test]
    async fn null_project_row_visible_from_scoped_query_memory() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let emb = unit_basis_embedding();
        let id = insert_memory_row_with_project(
            &store, "agent-scope-null", None, "chat_message", "null project row", emb.clone(),
        ).await;

        let q_vec = unit_basis_vec();
        let scope = SearchScope::all_default().with_project(Some("P1"));
        let hits = search_all_memories(&store, &q_vec, "agent-scope-null", scope, 10).await.unwrap();

        assert!(
            hits.iter().any(|h| h.id == id),
            "a NULL-project memory row must remain visible from a scoped query"
        );
    }

    #[tokio::test]
    async fn null_project_row_visible_from_scoped_query_episodic() {
        use crate::memory::embedder::FakeEmbedder;
        let store = MemoryStore::new_for_test().await.unwrap();
        let embedder = FakeEmbedder::default();
        let emb = embedder.embed_with_role("scoped null episodic task", EmbedRole::Passage).await.unwrap();
        let id = insert_episodic_row_with_project(
            &store, "agent-scope-null", None, "sess-epi-null", emb.clone(),
        ).await;

        let scope = SearchScope::all_default().with_project(Some("P1"));
        let hits = search_all_memories(&store, &emb, "agent-scope-null", scope, 10).await.unwrap();

        assert!(
            hits.iter().any(|h| h.id == id && h.kind == TableKind::Episodic),
            "a NULL-project episodic row must remain visible from a scoped query"
        );
    }

    #[tokio::test]
    async fn null_project_row_visible_from_scoped_query_procedural() {
        use crate::memory::embedder::FakeEmbedder;
        let store = MemoryStore::new_for_test().await.unwrap();
        let embedder = FakeEmbedder::default();
        let emb = embedder.embed_with_role("scoped null procedural trigger", EmbedRole::Passage).await.unwrap();
        let id = insert_procedural_row_with_project(
            &store, "agent-scope-null", None, "skill-null", emb.clone(),
        ).await;

        let scope = SearchScope::all_default().with_project(Some("P1"));
        let hits = search_all_memories(&store, &emb, "agent-scope-null", scope, 10).await.unwrap();

        assert!(
            hits.iter().any(|h| h.id == id && h.kind == TableKind::Procedural),
            "a NULL-project procedural row must remain visible from a scoped query"
        );
    }

    #[tokio::test]
    async fn user_preference_crosses_project_boundary() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let emb = unit_basis_embedding();
        let id = insert_memory_row_with_project(
            &store, "agent-pref-cross", Some("P2"), "user_preference", "pref content", emb.clone(),
        ).await;

        let q_vec = unit_basis_vec();
        let scope = SearchScope::all_default().with_project(Some("P1"));
        let hits = search_all_memories(&store, &q_vec, "agent-pref-cross", scope, 10).await.unwrap();

        assert!(
            hits.iter().any(|h| h.id == id),
            "a user_preference row must remain visible across project boundaries"
        );
    }

    // The test above proves the carve-out lets a foreign `user_preference` row
    // through the KNN join, but it does so with a one-row corpus where the
    // shortfall gate never depends on the carve-out: `reachable` is 1 with it
    // and 0 without, `hits.len()` is 1 either way, and `1 < 1` / `1 < 0` are
    // both false, so no retry fires and the assertion above passes whether or
    // not `ELIGIBLE_SQL_MEMORY` carries the disjunct. This starves the first
    // window on purpose instead: a crowd of 30 foreign, ineligible rows sits
    // closer to the query than the one foreign `user_preference` row, so vec0's
    // first k-window is entirely crowd, the WHERE clause throws all of it away,
    // and `hits.len()` comes back 0. Only the carve-out then makes `reachable`
    // count the pref row (1), which is what makes `0 < reachable` true and
    // fires the second pass that actually reaches past the crowd to find it.
    // Drop the disjunct from `ELIGIBLE_SQL_MEMORY` and `reachable` drops to 0
    // too, the gate never opens, and the row is lost with no error raised.
    #[tokio::test]
    async fn user_preference_carve_out_gates_the_retry_that_recovers_it() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let near = unit_basis_embedding();
        let far = off_axis_embedding();
        let agent = "agent-pref-starved";

        bulk_insert_memory_rows(&store, agent, Some("P2"), 30, near.clone()).await;
        let pref_id = insert_memory_row_with_project(
            &store, agent, Some("P2"), "user_preference", "pref content", far,
        ).await;

        let outcome = knn_one(&store, KNN_MEMORY, &near, agent, Some("P1"), 30, &ScoringConfig::DEFAULT, TimestampMs::now())
            .await
            .unwrap();

        assert_eq!(
            outcome.passes, 2,
            "the carve-out is the only thing that makes reachable (1) exceed \
             hits.len() (0) here; without it the retry never fires"
        );
        assert!(
            outcome.hits.iter().any(|h| h.id == pref_id),
            "the retry must recover the foreign user_preference row the crowd pushed past the k-th neighbour"
        );
    }

    // A caller that never opts into project scoping must see the exact
    // pre-scoping row set — otherwise every unscoped query silently loses
    // rows the moment scoping ships, rag-eval included. This passes with
    // zero production code above it (the `?4 IS NULL` disjunct is already
    // live by the time this test file runs), so its RED comes from
    // deliberately breaking that disjunct in the SQL below, observing the
    // failure, then reverting — the disjunct existing is the fact under
    // test, not new behavior.
    #[tokio::test]
    async fn unscoped_caller_sees_full_pre_scoping_row_set_memory() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let emb = unit_basis_embedding();
        let p1 = insert_memory_row_with_project(
            &store, "agent-unscoped", Some("P1"), "chat_message", "p1 row", emb.clone(),
        ).await;
        let p2 = insert_memory_row_with_project(
            &store, "agent-unscoped", Some("P2"), "chat_message", "p2 row", emb.clone(),
        ).await;
        let none = insert_memory_row_with_project(
            &store, "agent-unscoped", None, "chat_message", "no-project row", emb.clone(),
        ).await;

        let q_vec = unit_basis_vec();
        let hits = search_all_memories(&store, &q_vec, "agent-unscoped", SearchScope::all_default(), 10)
            .await
            .unwrap();

        for id in [p1, p2, none] {
            assert!(hits.iter().any(|h| h.id == id), "unscoped query must return row {id} regardless of its project_id");
        }
    }

    // ── candidate-pool probe and bounded retry ──

    async fn count_eligible_memory(
        store: &MemoryStore,
        agent_id: &str,
        project_id: Option<&str>,
        k: i64,
    ) -> i64 {
        let agent = agent_id.to_string();
        let proj = project_id.map(str::to_owned);
        store
            .conn
            .call(move |c| -> Result<i64, tokio_rusqlite::Error> {
                Ok(c.query_row(ELIGIBLE_SQL_MEMORY, params![k, agent, proj], |row| {
                    row.get::<_, i64>(0)
                })?)
            })
            .await
            .unwrap()
    }

    // The retry trigger has to tell "the join filtered rows out of a full
    // k-window" apart from "this caller has nothing more to give", and only
    // the second question can be answered without re-running the search. The
    // count answers it over the parent table with the search's own predicates,
    // capped at the pool size, so it never scans past the point where the
    // answer stops changing.
    #[tokio::test]
    async fn eligible_count_is_scoped_to_the_caller_and_capped_at_the_pool() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let emb = unit_basis_embedding();
        for i in 0..3 {
            insert_memory_row_with_project(
                &store, "agent-probe", Some("P1"), "chat_message", &format!("own row {i}"), emb.clone(),
            ).await;
        }
        for i in 0..20 {
            insert_memory_row_with_project(
                &store, "agent-probe", Some("P2"), "chat_message", &format!("foreign row {i}"), emb.clone(),
            ).await;
        }
        for i in 0..7 {
            insert_memory_row_with_project(
                &store, "other-agent", Some("P1"), "chat_message", &format!("other agent row {i}"), emb.clone(),
            ).await;
        }

        assert_eq!(
            count_eligible_memory(&store, "agent-probe", Some("P1"), 30).await,
            3,
            "a table full of rows belonging to another project or another agent leaves this caller three"
        );
        assert_eq!(
            count_eligible_memory(&store, "agent-probe", None, 30).await,
            23,
            "an unscoped caller is eligible for every row of its own agent"
        );
        assert_eq!(
            count_eligible_memory(&store, "agent-probe", None, 5).await,
            5,
            "the count must stop at the pool size, not walk the agent's whole history"
        );
    }

    /// A unit vector measurably off the query direction, so rows carrying it
    /// always lose the k-window race against `unit_basis_vec`. Ties would make
    /// every starvation assertion below depend on vec0's internal iteration
    /// order instead of on distance.
    fn off_axis_vec() -> Vec<f32> {
        let mut v = vec![0.0_f32; 384];
        v[0] = 0.9;
        v[1] = (1.0_f32 - 0.81).sqrt();
        v
    }

    fn off_axis_embedding() -> Vec<u8> {
        serialize_embedding(&off_axis_vec())
    }

    async fn bulk_insert_memory_rows(
        store: &MemoryStore,
        agent_id: &str,
        project_id: Option<&str>,
        count: usize,
        embedding: Vec<u8>,
    ) -> Vec<i64> {
        let agent = agent_id.to_string();
        let project = project_id.map(|p| p.to_string());
        store
            .conn
            .call(move |c| -> Result<Vec<i64>, rusqlite::Error> {
                let tx = c.transaction()?;
                let mut ids = Vec::with_capacity(count);
                for i in 0..count {
                    tx.execute(
                        "INSERT INTO memory (scope, agent_id, project_id, content, memory_type,
                         source, created_at, created_by_type, created_by_ref,
                         last_modified_at, last_accessed_at, importance, tier, status)
                         VALUES ('agent', ?1, ?2, ?3, 'chat_message', 'chat',
                         0, 'user', ?1, 0, 0, 5.0, 3, 'active')",
                        params![agent, project, format!("bulk row {i}")],
                    )?;
                    let id = tx.last_insert_rowid();
                    tx.execute(
                        "INSERT INTO memory_vectors(rowid, embedding) VALUES (?, ?)",
                        params![id, embedding],
                    )?;
                    ids.push(id);
                }
                tx.commit()?;
                Ok(ids)
            })
            .await
            .unwrap()
    }

    #[test]
    fn retry_window_widens_fourfold_up_to_the_engine_ceiling() {
        assert_eq!(retry_window(30), 120);
        assert_eq!(retry_window(1024), 4096);
        assert_eq!(
            retry_window(2000),
            VEC0_MAX_K,
            "8000 is past what vec0 will accept as k, so the widened window has to stop here"
        );
        assert_eq!(retry_window(VEC0_MAX_K), VEC0_MAX_K);
    }

    // A full pool proves nothing was filtered: the query asked for k and got k,
    // so there is no evidence of a starved window and no reason to pay for a
    // second scan.
    #[tokio::test]
    async fn knn_no_retry_when_pool_is_full() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let emb = unit_basis_embedding();
        bulk_insert_memory_rows(&store, "agent-full", Some("P1"), 10, emb.clone()).await;

        let outcome = knn_one(&store, KNN_MEMORY, &emb, "agent-full", Some("P1"), 5, &ScoringConfig::DEFAULT, TimestampMs::now())
            .await
            .unwrap();

        assert_eq!(outcome.hits.len(), 5, "a full window returns exactly k rows");
        assert_eq!(
            outcome.passes, 1,
            "nothing was filtered, so a wider window cannot find more"
        );
    }

    // The case that decides whether this feature costs one query or two on
    // every search this app runs: a corpus smaller than k. vec0 already handed
    // back every vector it has, so widening the window returns the identical
    // rows — retrying here would double the query cost of the common case for
    // no gain whatsoever.
    #[tokio::test]
    async fn knn_no_retry_on_small_corpus() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let emb = unit_basis_embedding();
        bulk_insert_memory_rows(&store, "agent-small", Some("P1"), 8, emb.clone()).await;

        let outcome = knn_one(&store, KNN_MEMORY, &emb, "agent-small", Some("P1"), 30, &ScoringConfig::DEFAULT, TimestampMs::now())
            .await
            .unwrap();

        assert_eq!(outcome.hits.len(), 8, "every row in the corpus is eligible");
        assert_eq!(
            outcome.passes, 1,
            "the shortfall is the corpus being small, not the window being crowded"
        );
    }

    // The failure this mechanism exists for: the k nearest vectors all belong
    // to another project, so the join throws the entire window away and the
    // caller's own rows — which do exist — are delivered by nobody.
    #[tokio::test]
    async fn knn_retry_fires_once_when_pool_saturated() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let near = unit_basis_embedding();
        let far = off_axis_embedding();
        bulk_insert_memory_rows(&store, "agent-starved", Some("P2"), 100, near.clone()).await;
        let own = bulk_insert_memory_rows(&store, "agent-starved", Some("P1"), 5, far).await;

        let outcome = knn_one(&store, KNN_MEMORY, &near, "agent-starved", Some("P1"), 30, &ScoringConfig::DEFAULT, TimestampMs::now())
            .await
            .unwrap();

        for id in &own {
            assert!(
                outcome.hits.iter().any(|h| h.id == *id),
                "row {id} belongs to the caller's project and must survive the crowding"
            );
        }
        // Two queries, not three and not a convergence loop. The counter is
        // incremented by the query function itself, so this number is how many
        // times the table was actually scanned — a variant that kept widening
        // would land on the same 5 rows and would be caught here and nowhere
        // else, because the hits are identical either way.
        assert_eq!(
            outcome.passes, 2,
            "recovery is one extra pass; the row set alone cannot tell two passes from five"
        );
        assert_eq!(outcome.hits.len(), 5, "the eligible set is 5, so a full pool was never reachable");
    }

    // The widened window is a budget for finding eligible rows, not a licence
    // to return four times as many. Letting it through would silently inflate
    // one table's share of the merged result and detach the pool-survival
    // denominator from what was actually asked for.
    #[tokio::test]
    async fn retry_result_truncated_to_original_k() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let near = unit_basis_embedding();
        let far = off_axis_embedding();
        bulk_insert_memory_rows(&store, "agent-trunc", Some("P2"), 10, near.clone()).await;
        bulk_insert_memory_rows(&store, "agent-trunc", Some("P1"), 50, far).await;

        let outcome = knn_one(&store, KNN_MEMORY, &near, "agent-trunc", Some("P1"), 10, &ScoringConfig::DEFAULT, TimestampMs::now())
            .await
            .unwrap();

        assert_eq!(outcome.passes, 2);
        assert_eq!(
            outcome.hits.len(),
            10,
            "the retry found more eligible rows than requested and must hand back only k"
        );
    }

    // The denominator in the retrieval line is what makes a starved search
    // readable at all, and a retry is exactly when it is easiest to get wrong:
    // the second query asks vec0 for four times the pool, and reporting that
    // number would turn a rescued search into the worst-looking line in the
    // log. This drives a real starved table through a real recovery and reads
    // the line that comes out, rather than handing the formatter numbers a
    // test made up.
    #[tokio::test]
    async fn retrieval_line_reports_the_requested_pool_after_a_real_recovery() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let near = unit_basis_embedding();
        let far = off_axis_embedding();
        let scope = SearchScope {
            memory: Some(30),
            episodic: None,
            procedural: None,
            project_id: Some("P1".to_string()),
        };
        let k = scope.memory.unwrap();
        // 30 foreign rows fill the first window exactly; 40 eligible rows sit
        // behind them, so the recovery can hand back a genuinely full pool.
        bulk_insert_memory_rows(&store, "agent-telemetry", Some("P2"), 30, near.clone()).await;
        bulk_insert_memory_rows(&store, "agent-telemetry", Some("P1"), 40, far).await;

        let mem = knn_one(&store, KNN_MEMORY, &near, "agent-telemetry", Some("P1"), k, &ScoringConfig::DEFAULT, TimestampMs::now())
            .await
            .unwrap();
        assert_eq!(mem.passes, 2, "test setup: the window has to have been starved");

        let line = format_pool_line(&scope, &mem, &KnnOutcome::skipped(), &KnnOutcome::skipped());

        assert!(
            line.contains("pool_req=(30+0+0)"),
            "the denominator is what the caller asked for, never the widened window: {line}"
        );
        assert!(
            line.contains("pool_ret=(30+0+0)"),
            "enough eligible rows existed, so the recovered pool is full: {line}"
        );
        assert!(
            line.contains("retry=(mem)"),
            "a line whose numbers were reached through a second pass has to say so: {line}"
        );
    }

    // The test above drives `format_pool_line` directly with an outcome it
    // built by hand — it pins the formatter, not the one-line wiring at the
    // real call site inside `search_all_memories_inner` that hands that
    // formatter the actual `mem`/`epi`/`pro` outcomes from a live search. A
    // mutation there (wrong table in the wrong slot, wrong scope) would
    // change nothing this driven-by-hand test could see. This drives the same
    // starved-then-recovered fixture through the public orchestrator instead,
    // so it is the real per-search outcomes that reach the formatter.
    #[tokio::test]
    async fn search_all_memories_wiring_reports_pool_line_from_a_real_recovery() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let near = unit_basis_vec();
        let far = off_axis_vec();
        let scope = SearchScope {
            memory: Some(30),
            episodic: None,
            procedural: None,
            project_id: Some("P1".to_string()),
        };
        bulk_insert_memory_rows(&store, "agent-telemetry-wired", Some("P2"), 30, serialize_embedding(&near)).await;
        bulk_insert_memory_rows(&store, "agent-telemetry-wired", Some("P1"), 40, serialize_embedding(&far)).await;

        let (hits, line) =
            search_all_memories_inner(&store, &near, "agent-telemetry-wired", scope, 30, &ScoringConfig::DEFAULT)
                .await
                .unwrap();

        assert_eq!(
            hits.len(),
            30,
            "the recovery has to reach the caller's own rows through the real orchestrator, not just knn_one"
        );
        assert!(
            line.contains("pool_req=(30+0+0)"),
            "the line built at the real call site must still read the requested pool: {line}"
        );
        assert!(
            line.contains("pool_ret=(30+0+0)"),
            "the line built at the real call site must still read what came back: {line}"
        );
        assert!(
            line.contains("retry=(mem)"),
            "the line built at the real call site must still say which table retried: {line}"
        );
    }

    /// Soft-deletes a row and leaves its vector in place, the way rows deleted
    /// before the delete path pruned vectors still sit in the database today.
    /// That is the only state in which soft-deleted rows can crowd a window,
    /// so it is the state worth searching against.
    async fn soft_delete_keeping_vector(store: &MemoryStore, id: i64) {
        store
            .conn
            .call(move |c| c.execute("UPDATE memory SET deleted_at = 1 WHERE id = ?1", params![id]))
            .await
            .unwrap();
    }

    // Deleted rows keep their place in the vector index until something sweeps
    // them, so on a corpus with a real deletion history they are exactly the
    // kind of crowd that empties a window: nearest by distance, invisible by
    // predicate. The pool the caller receives has to be full of live rows
    // anyway — the deletions are the store's history, not a reduction of what
    // the caller asked for.
    #[tokio::test]
    async fn knn_returns_a_full_live_pool_despite_soft_deleted_rows_in_the_index() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let near = unit_basis_embedding();
        let far = off_axis_embedding();
        let deleted =
            bulk_insert_memory_rows(&store, "agent-deleted", Some("P1"), 20, near.clone()).await;
        let live = bulk_insert_memory_rows(&store, "agent-deleted", Some("P1"), 30, far).await;
        for id in &deleted {
            soft_delete_keeping_vector(&store, *id).await;
        }

        let outcome = knn_one(&store, KNN_MEMORY, &near, "agent-deleted", Some("P1"), 30, &ScoringConfig::DEFAULT, TimestampMs::now())
            .await
            .unwrap();

        assert_eq!(
            outcome.hits.len(),
            30,
            "every live row was reachable, so the pool must come back full"
        );
        for id in &deleted {
            assert!(
                !outcome.hits.iter().any(|h| h.id == *id),
                "soft-deleted row {id} must not reach the caller"
            );
        }
        for id in &live {
            assert!(
                outcome.hits.iter().any(|h| h.id == *id),
                "live row {id} belongs in the pool"
            );
        }
    }

    // One agent, several projects, one database — the arrangement this whole
    // scoping effort exists to serve. The caller owns three rows out of a
    // thousand, so no window of any width can fill a pool of thirty. Widening
    // is not a recovery here, it is a second scan bought on every search this
    // caller will ever run, forever, to re-find the same three rows the first
    // scan already returned.
    #[tokio::test]
    async fn knn_does_not_retry_when_the_caller_already_has_every_row_it_could_get() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let near = unit_basis_embedding();
        let far = off_axis_embedding();
        // The caller's own rows sit closest to the query, so the first pass
        // already delivers all three: the shortfall is the eligible set being
        // small, not the window being crowded.
        let own = bulk_insert_memory_rows(&store, "agent-shared", Some("P1"), 3, near.clone()).await;
        bulk_insert_memory_rows(&store, "agent-shared", Some("P2"), 1000, far).await;

        let outcome = knn_one(&store, KNN_MEMORY, &near, "agent-shared", Some("P1"), 30, &ScoringConfig::DEFAULT, TimestampMs::now())
            .await
            .unwrap();

        assert_eq!(outcome.hits.len(), 3, "three rows is everything this caller owns");
        for id in &own {
            assert!(outcome.hits.iter().any(|h| h.id == *id), "row {id} must be returned");
        }
        assert_eq!(
            outcome.passes, 1,
            "a full corpus around a tiny eligible set must not buy a second scan on every search"
        );
    }

    // vec0 rejects a k above its own ceiling outright, so a pool size past a
    // quarter of that ceiling turns the widened retry into a hard error unless
    // the window is clamped.
    /// A unit vector at a controlled cosine similarity to `unit_basis_vec`
    /// (`[1,0,0,…]`), so a fixture can pick an exact distance instead of
    /// relying on `off_axis_vec`'s one fixed angle.
    fn vec_at_cosine(cos_theta: f32) -> Vec<f32> {
        let mut v = vec![0.0_f32; 384];
        v[0] = cos_theta;
        v[1] = (1.0_f32 - cos_theta * cos_theta).sqrt();
        v
    }

    // The trap the SQL `ORDER BY score DESC` quietly covered for: the
    // widened retry window comes back ordered by raw distance, not by the
    // score a row actually earns once importance/status are applied. A row
    // that arrives CLOSER — and so earlier in that stream — can still score
    // LOWER than one that arrives farther away. Here NEAR is disputed and
    // unimportant, FAR is active and maximally important, and FAR's score
    // comes out well above NEAR's despite losing on raw distance.
    // `knn_one` must score every row, order them by the real comparator, and
    // only then cut to k — truncating on arrival order would keep NEAR and
    // lose the row that actually ranks first.
    //
    // This is an approval test: it is written and confirmed green against
    // today's SQL-sorted `knn_one` (`ORDER BY score DESC` already gets this
    // right), and it MUST stay green once scoring moves to Rust and that
    // clause is gone. If it goes red at that point, the port introduced the
    // exact candidate-set bug it promised not to.
    #[tokio::test]
    async fn knn_one_retry_path_keeps_the_true_top_k_by_score_not_join_arrival_order() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let agent = "agent-retry-order";

        // Two exact-match rows in a foreign project saturate the first
        // pass's k=1 window entirely, forcing the retry.
        let crowd = serialize_embedding(&unit_basis_vec());
        for i in 0..2 {
            insert_memory_row_with_project(
                &store, agent, Some("P2"), "chat_message", &format!("crowd {i}"), crowd.clone(),
            ).await;
        }

        // NEAR: closest of the caller's own rows, but disputed and
        // unimportant — its score has to lose despite arriving first.
        let near_emb = serialize_embedding(&vec_at_cosine(0.999));
        let near_id = insert_memory_row(
            &store, agent, "near but low score", "chat_message", 3, 1.0, "disputed", near_emb,
        ).await;

        // FAR: farther from the query than NEAR, but active and maximally
        // important — its score has to win despite arriving second. Kept
        // above `ScoringConfig::DEFAULT`'s cosine re-scale floor on purpose:
        // a cosine below that floor clamps to a score of exactly zero
        // regardless of importance (a separate, already-covered behavior),
        // which would make this fixture test the floor instead of the thing
        // it exists to test — truncation picking the true top score rather
        // than arrival order.
        let far_emb = serialize_embedding(&vec_at_cosine(0.95));
        let far_id = insert_memory_row(
            &store, agent, "far but high score", "chat_message", 3, 10.0, "active", far_emb,
        ).await;

        let q_bytes = serialize_embedding(&unit_basis_vec());
        let outcome = knn_one(&store, KNN_MEMORY, &q_bytes, agent, Some("P1"), 1, &ScoringConfig::DEFAULT, TimestampMs::now())
            .await
            .unwrap();

        assert_eq!(outcome.passes, 2, "test setup: the crowd must starve the first pass");
        assert_eq!(
            outcome.hits.len(),
            1,
            "k=1 must still cut the widened retry back to exactly one row"
        );
        assert_eq!(
            outcome.hits[0].id, far_id,
            "the higher-scoring row must survive truncation even though the \
             lower-scoring row (id={near_id}) arrived first in raw distance order"
        );
    }

    #[tokio::test]
    async fn knn_retry_window_stays_within_engine_ceiling() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let near = unit_basis_embedding();
        let far = off_axis_embedding();
        let k = 1100;
        bulk_insert_memory_rows(&store, "agent-ceiling", Some("P2"), k, near.clone()).await;
        let own = bulk_insert_memory_rows(&store, "agent-ceiling", Some("P1"), 5, far).await;

        let outcome = knn_one(&store, KNN_MEMORY, &near, "agent-ceiling", Some("P1"), k, &ScoringConfig::DEFAULT, TimestampMs::now())
            .await
            .expect("the widened window must stay bindable");

        assert_eq!(outcome.passes, 2);
        for id in &own {
            assert!(outcome.hits.iter().any(|h| h.id == *id), "row {id} must be recovered");
        }
    }

    // The recovery is wired per table through a descriptor, and only the
    // memory branch was ever driven through a starved window. A descriptor
    // pointing at the wrong table's count is a silent mistake as long as no
    // test makes the other two tables recover anything: the memory table is
    // empty in these fixtures, so borrowing its count would report nothing
    // eligible and the rows below would never come back.
    #[tokio::test]
    async fn knn_retry_recovers_episodic_rows_crowded_out_of_the_window() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let near = unit_basis_vec();
        let far = off_axis_vec();
        for i in 0..15 {
            insert_episodic_row_with_project(
                &store, "agent-epi-retry", Some("P2"), &format!("sess-foreign-{i}"), near.clone(),
            ).await;
        }
        let mut own = Vec::new();
        for i in 0..3 {
            own.push(
                insert_episodic_row_with_project(
                    &store, "agent-epi-retry", Some("P1"), &format!("sess-own-{i}"), far.clone(),
                ).await,
            );
        }

        let q_bytes = serialize_embedding(&near);
        let outcome = knn_one(&store, KNN_EPISODIC, &q_bytes, "agent-epi-retry", Some("P1"), 5, &ScoringConfig::DEFAULT, TimestampMs::now())
            .await
            .unwrap();

        assert_eq!(outcome.passes, 2, "the first window held nothing this caller may see");
        for id in &own {
            assert!(
                outcome.hits.iter().any(|h| h.id == *id && h.kind == TableKind::Episodic),
                "episodic row {id} must be recovered by the wider window"
            );
        }
    }

    #[tokio::test]
    async fn knn_retry_recovers_procedural_rows_crowded_out_of_the_window() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let near = unit_basis_vec();
        let far = off_axis_vec();
        for i in 0..15 {
            insert_procedural_row_with_project(
                &store, "agent-pro-retry", Some("P2"), &format!("skill-foreign-{i}"), near.clone(),
            ).await;
        }
        let mut own = Vec::new();
        for i in 0..3 {
            own.push(
                insert_procedural_row_with_project(
                    &store, "agent-pro-retry", Some("P1"), &format!("skill-own-{i}"), far.clone(),
                ).await,
            );
        }

        let q_bytes = serialize_embedding(&near);
        let outcome = knn_one(&store, KNN_PROCEDURAL, &q_bytes, "agent-pro-retry", Some("P1"), 5, &ScoringConfig::DEFAULT, TimestampMs::now())
            .await
            .unwrap();

        assert_eq!(outcome.passes, 2, "the first window held nothing this caller may see");
        for id in &own {
            assert!(
                outcome.hits.iter().any(|h| h.id == *id && h.kind == TableKind::Procedural),
                "procedural row {id} must be recovered by the wider window"
            );
        }
    }

    #[tokio::test]
    async fn knn_one_reports_no_retry_for_episodic_and_procedural_small_corpora() {
        use crate::memory::embedder::FakeEmbedder;
        let store = MemoryStore::new_for_test().await.unwrap();
        let embedder = FakeEmbedder::default();
        // Normalized before it is used on either side. The insert path unit-
        // normalizes what it stores, so a raw query vector would be compared
        // against something that no longer points the same length — distances
        // stop being comparable to the cosine the score formula assumes, and
        // any ordering read off them is an accident of the fixture.
        let emb = to_unit(&embedder.embed_with_role("cross table outcome", EmbedRole::Passage).await.unwrap()).unwrap();
        insert_episodic_row_with_project(&store, "agent-x", Some("P1"), "sess-x", emb.clone()).await;
        insert_procedural_row_with_project(&store, "agent-x", Some("P1"), "skill-x", emb.clone()).await;

        let q_bytes = serialize_f32_query(&emb);
        let epi = knn_one(&store, KNN_EPISODIC, &q_bytes, "agent-x", Some("P1"), 5, &ScoringConfig::DEFAULT, TimestampMs::now()).await.unwrap();
        let pro = knn_one(&store, KNN_PROCEDURAL, &q_bytes, "agent-x", Some("P1"), 5, &ScoringConfig::DEFAULT, TimestampMs::now()).await.unwrap();

        assert_eq!(epi.hits.len(), 1);
        assert_eq!(pro.hits.len(), 1);
        assert_eq!(epi.passes, 1);
        assert_eq!(pro.passes, 1);
    }

    #[tokio::test]
    async fn unscoped_caller_sees_full_pre_scoping_row_set_episodic() {
        use crate::memory::embedder::FakeEmbedder;
        let store = MemoryStore::new_for_test().await.unwrap();
        let embedder = FakeEmbedder::default();
        let emb = embedder.embed_with_role("unscoped episodic task", EmbedRole::Passage).await.unwrap();
        let p1 = insert_episodic_row_with_project(&store, "agent-uns-epi", Some("P1"), "sess-own", emb.clone()).await;
        let p2 = insert_episodic_row_with_project(&store, "agent-uns-epi", Some("P2"), "sess-foreign", emb.clone()).await;
        let none = insert_episodic_row_with_project(&store, "agent-uns-epi", None, "sess-unowned", emb.clone()).await;

        let hits = search_all_memories(&store, &emb, "agent-uns-epi", SearchScope::all_default(), 10)
            .await
            .unwrap();

        for id in [p1, p2, none] {
            assert!(
                hits.iter().any(|h| h.id == id && h.kind == TableKind::Episodic),
                "unscoped query must return episodic row {id} regardless of its project_id"
            );
        }
    }

    #[tokio::test]
    async fn unscoped_caller_sees_full_pre_scoping_row_set_procedural() {
        use crate::memory::embedder::FakeEmbedder;
        let store = MemoryStore::new_for_test().await.unwrap();
        let embedder = FakeEmbedder::default();
        let emb = embedder.embed_with_role("unscoped procedural trigger", EmbedRole::Passage).await.unwrap();
        let p1 = insert_procedural_row_with_project(&store, "agent-uns-pro", Some("P1"), "skill-own", emb.clone()).await;
        let p2 = insert_procedural_row_with_project(&store, "agent-uns-pro", Some("P2"), "skill-foreign", emb.clone()).await;
        let none = insert_procedural_row_with_project(&store, "agent-uns-pro", None, "skill-unowned", emb.clone()).await;

        let hits = search_all_memories(&store, &emb, "agent-uns-pro", SearchScope::all_default(), 10)
            .await
            .unwrap();

        for id in [p1, p2, none] {
            assert!(
                hits.iter().any(|h| h.id == id && h.kind == TableKind::Procedural),
                "unscoped query must return procedural row {id} regardless of its project_id"
            );
        }
    }
}
