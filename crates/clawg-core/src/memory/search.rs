//! KNN search across memory + episodic + procedural via tokio::try_join!,
//! merged + ranked by the score formula of `score.rs`.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use crate::memory::clock::TimestampMs;
use crate::memory::pieces::SaveLink;
use crate::memory::score::{score as compute_score, RawHit, ScoringConfig};
use crate::memory::sharing::{Reach, Reader};
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
pub struct SearchScope {
    pub memory: Option<usize>,
    pub episodic: Option<usize>,
    pub procedural: Option<usize>,
    pub project_id: Option<String>,
    /// Candidate budget for the lexical (FTS5) channel — `None` keeps the
    /// channel off entirely: no MATCH query and no scoring query are issued.
    pub lexical: Option<usize>,
    /// Whose rows besides its own the searching agent may read. `Own` — the
    /// default here — searches the agent's own rows only.
    pub reach: Reach,
}

impl SearchScope {
    pub fn all_default() -> Self {
        Self {
            memory: Some(30),
            episodic: Some(5),
            procedural: Some(5),
            project_id: None,
            lexical: Some(20),
            reach: Reach::Own,
        }
    }
    /// Scopes this query to `project_id`, owning a copy of it. `None` keeps
    /// the scope unscoped — same as never calling this.
    pub fn with_project(mut self, project_id: Option<&str>) -> Self {
        self.project_id = project_id.map(str::to_owned);
        self
    }
    /// Lets this query read the rows `reach` allows besides the agent's own.
    pub fn with_reach(mut self, reach: Reach) -> Self {
        self.reach = reach;
        self
    }
}

/// Who a search may read, as the JSON object every query below binds once:
/// `reader` always, `authorless` when the rows no agent saved are readable,
/// and `hidden` only when other agents are. One parameter whatever the
/// policy, so no placeholder numbering moves.
fn reach_bind(agent_id: &str, reach: &Reach) -> String {
    let value = match reach {
        Reach::Own => serde_json::json!({ "reader": agent_id, "authorless": true }),
        Reach::AllBut(hidden) => serde_json::json!({ "reader": agent_id, "authorless": true, "hidden": hidden }),
    };
    value.to_string()
}

/// The reach predicate on the author column `$author`, over the object
/// [`reach_bind`] binds to `$param`. Every query that reads by reach spells it
/// through here, so no copy can drift, and a test holds [`Reach::admits`] to
/// the same rule.
macro_rules! reach_predicate {
    ($author:literal, $param:literal) => {
        concat!(
            "(", $author, " = (", $param, " ->> '$.reader') OR (", $author, " IS NULL AND (", $param,
            " ->> '$.authorless') IS NOT NULL) OR ((", $param, " ->> '$.hidden') IS NOT NULL AND ",
            $author, " NOT IN (SELECT value FROM json_each(", $param, ", '$.hidden'))))"
        )
    };
}
pub(crate) use reach_predicate;

/// The reach of `reader` as [`reach_bind`] binds it, or NULL for a call with
/// no reader, which is the user's: [`reader_predicate!`] lets it reach every row.
pub(crate) fn reader_bind(reader: Option<&Reader>) -> Option<String> {
    reader.map(|r| reach_bind(&r.agent_id, &r.reach))
}

/// [`reach_predicate!`] over what [`reader_bind`] binds to `$param`, true for
/// every row when it bound NULL.
macro_rules! reader_predicate {
    ($author:literal, $param:literal) => {
        concat!("(", $param, " IS NULL OR ", $crate::memory::search::reach_predicate!($author, $param), ")")
    };
}
pub(crate) use reader_predicate;

/// The project predicate of `memory` rows under the column prefix `$m`
/// (`"m."`, or `""` without an alias), for the project bound to `$param`: no
/// project reads every row; a project reads its own, the rows without one and
/// every `user_preference`, which describes the person and follows them from
/// project to project.
macro_rules! memory_project_predicate {
    ($m:literal, $param:literal) => {
        concat!(
            "(", $param, " IS NULL OR ", $m, "project_id = ", $param, " OR ", $m, "project_id IS NULL OR ",
            $m, "memory_type = 'user_preference')"
        )
    };
}
pub(crate) use memory_project_predicate;

/// One result row, scored by `score::score`.
///
/// `Serialize` because the `clawg serve` daemon answers retrieval requests
/// with these rows as JSON.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RankedHit {
    pub id: i64,
    pub kind: TableKind,
    pub content: String,
    pub memory_type: String,
    pub tier: i64,
    pub status: String,
    pub score: f64,
    pub distance: f64,   // raw KNN distance — consumed by contradiction.rs to threshold-filter near-duplicate candidates
    pub created_at: TimestampMs, // for RECENT vs LONG-TERM bucketing in prompt_builder
    /// Who saved the row, so a hit from a granted agent can be told apart
    /// from the reader's own.
    pub agent_id: Option<String>,
    /// Which piece of a long save this row is; absent for a save of one piece.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub piece: Option<PiecePosition>,
    /// The row holds every identifier of the query and took the lexical
    /// bonus, so its score says nothing about how close the others are.
    pub exact: bool,
    /// The key a save revises this row under; absent for a row saved without
    /// one and for the tables that have no such column.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic_key: Option<String>,
    /// The project the row was saved in; `None` for a row saved without one.
    /// Sent as `null` rather than left out, so a bridge can tell such a row
    /// from a daemon that predates the field.
    pub project_id: Option<String>,
}

/// Position `piece` (from 1) of a save stored as `pieces` rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct PiecePosition {
    pub piece: i64,
    pub pieces: i64,
}

/// The hits `reader` may read, in their order. The queries already filter by
/// reach; this is the check that does not depend on each of them repeating it.
fn readable_hits(hits: Vec<RankedHit>, reader: &str, reach: &Reach) -> Vec<RankedHit> {
    hits.into_iter().filter(|hit| reach.admits(reader, hit.agent_id.as_deref())).collect()
}

/// Keeps the first, so best ranked, hit of each save and drops its other
/// pieces, so one long text takes one slot. Only `memory` rows are pieces:
/// the other tables number their rows independently.
fn collapse_saves(hits: Vec<RankedHit>, links: &HashMap<i64, SaveLink>) -> Vec<RankedHit> {
    let mut seen = HashSet::new();
    hits.into_iter()
        .filter_map(|mut hit| {
            if hit.kind != TableKind::Memory {
                return Some(hit);
            }
            match links.get(&hit.id) {
                Some(link) if !seen.insert(link.save_id) => None,
                Some(link) => {
                    hit.piece = Some(link.position);
                    Some(hit)
                }
                None => Some(hit),
            }
        })
        .collect()
}

/// The save a row stands for: its link's, or the row alone when it has none.
fn save_of(id: i64, links: &HashMap<i64, SaveLink>) -> (bool, i64) {
    match links.get(&id) {
        Some(link) => (true, link.save_id),
        None => (false, id),
    }
}

/// How many saves the rows `ids` belong to.
fn distinct_saves(ids: impl Iterator<Item = i64>, links: &HashMap<i64, SaveLink>) -> usize {
    ids.map(|id| save_of(id, links)).collect::<HashSet<_>>().len()
}

/// The rows of `hits`, best first, up to the one that completes `k` saves.
/// Siblings of a save already in stay, for `collapse_saves` to drop; what a
/// pool budgets is how many saves it can offer, not how many rows.
fn trim_to_saves(hits: Vec<RankedHit>, links: &HashMap<i64, SaveLink>, k: usize) -> Vec<RankedHit> {
    let mut seen = HashSet::new();
    hits.into_iter()
        .filter(|hit| {
            let save = save_of(hit.id, links);
            seen.contains(&save) || (seen.len() < k && seen.insert(save))
        })
        .collect()
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
#[cfg(any(test, feature = "testing"))]
pub fn serialize_f32_query_pub(v: &[f32]) -> Vec<u8> {
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
const SEARCH_SQL_MEMORY: &str = concat!(
    "
    SELECT m.id, m.content, m.memory_type, m.tier, m.status, knn.distance,
        m.created_at, m.last_accessed_at, m.importance, m.agent_id, m.topic_key, m.project_id
    FROM memory_vectors knn
    JOIN memory m ON m.id = knn.rowid
    WHERE knn.embedding MATCH ?1
      AND knn.k = ?2
      AND m.status IN ('active', 'disputed')
      AND m.deleted_at IS NULL
      AND ",
    reach_predicate!("m.agent_id", "?3"),
    "
      AND ",
    memory_project_predicate!("m.", "?4"),
    "
"
);

// SQL for episodic table KNN. The 0.0077 lambda this table always used no
// longer lives here — it is `DecayLambdas.episode` in score.rs, reached
// through `lambda_for` dispatching on `TableKind::Episodic` rather than on
// the `'episode'` string this query still selects for display.
const SEARCH_SQL_EPISODIC: &str = concat!(
    "
    SELECT epi.id, epi.task, 'episode' AS memory_type, epi.tier, epi.status, knn.distance,
        epi.created_at, epi.last_accessed_at, epi.importance, epi.agent_id, NULL, epi.project_id
    FROM episodic_vectors knn
    JOIN episodic_trajectory epi ON epi.id = knn.rowid
    WHERE knn.embedding MATCH ?1
      AND knn.k = ?2
      AND epi.status IN ('active', 'disputed')
      AND epi.deleted_at IS NULL
      AND ",
    reach_predicate!("epi.agent_id", "?3"),
    "
      AND (?4 IS NULL OR epi.project_id = ?4 OR epi.project_id IS NULL)
"
);

// SQL for procedural table KNN. Same as episodic: 0.0039 now lives in
// `DecayLambdas.procedure`, dispatched by `TableKind::Procedural`.
const SEARCH_SQL_PROCEDURAL: &str = concat!(
    "
    SELECT pro.id, pro.skill_name, 'procedure' AS memory_type, pro.tier, pro.status, knn.distance,
        pro.created_at, pro.last_accessed_at, pro.importance, pro.agent_id, NULL, pro.project_id
    FROM procedural_vectors knn
    JOIN procedural_skill pro ON pro.id = knn.rowid
    WHERE knn.embedding MATCH ?1
      AND knn.k = ?2
      AND pro.status IN ('active', 'disputed')
      AND pro.deleted_at IS NULL
      AND ",
    reach_predicate!("pro.agent_id", "?3"),
    "
      AND (?4 IS NULL OR pro.project_id = ?4 OR pro.project_id IS NULL)
"
);

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
// The LIMIT stops the count at k rather than walking every row the caller
// may read.
const ELIGIBLE_SQL_MEMORY: &str = concat!(
    "
    SELECT COUNT(*) FROM (
        SELECT m.id FROM memory m
        WHERE m.status IN ('active', 'disputed')
          AND m.deleted_at IS NULL
          AND ",
    reach_predicate!("m.agent_id", "?2"),
    "
          AND ",
    memory_project_predicate!("m.", "?3"),
    "
        LIMIT ?1
    )
"
);

const ELIGIBLE_SQL_EPISODIC: &str = concat!(
    "
    SELECT COUNT(*) FROM (
        SELECT epi.id FROM episodic_trajectory epi
        WHERE epi.status IN ('active', 'disputed')
          AND epi.deleted_at IS NULL
          AND ",
    reach_predicate!("epi.agent_id", "?2"),
    "
          AND (?3 IS NULL OR epi.project_id = ?3 OR epi.project_id IS NULL)
        LIMIT ?1
    )
"
);

const ELIGIBLE_SQL_PROCEDURAL: &str = concat!(
    "
    SELECT COUNT(*) FROM (
        SELECT pro.id FROM procedural_skill pro
        WHERE pro.status IN ('active', 'disputed')
          AND pro.deleted_at IS NULL
          AND ",
    reach_predicate!("pro.agent_id", "?2"),
    "
          AND (?3 IS NULL OR pro.project_id = ?3 OR pro.project_id IS NULL)
        LIMIT ?1
    )
"
);

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
    /// How many saves `hits` stand for.
    saves: usize,
    /// The eligible rows counted when the first window came back short,
    /// capped at the window; `None` when it came back full.
    eligible: Option<usize>,
}

impl KnnOutcome {
    /// A table the scope switched off: it asked for nothing, so nothing was
    /// queried and no pass ran.
    fn skipped() -> Self {
        Self { hits: Vec::new(), passes: 0, saves: 0, eligible: None }
    }

    /// Whether recovery happened — a single pass is the ordinary path, and a
    /// table that was never queried did not recover from anything.
    fn retried(&self) -> bool {
        self.passes > 1
    }
}

/// What the `memory` KNN of one search asked for and handed to the merge,
/// before readability, the collapse to one hit per save and the cut. An
/// evaluation reads it to tell a row that never reached the pool from one
/// that reached it and ranked low, and a window crowded by other rows from a
/// scope that holds few.
#[derive(Debug)]
pub struct MemoryPool {
    pub requested: usize,
    pub rows: usize,
    pub saves: usize,
    pub passes: usize,
    pub eligible: Option<usize>,
    pub ids: Vec<i64>,
}

impl MemoryPool {
    fn of(requested: Option<usize>, outcome: &KnnOutcome) -> Self {
        Self {
            requested: requested.unwrap_or(0),
            rows: outcome.hits.len(),
            saves: outcome.saves,
            passes: outcome.passes,
            eligible: outcome.eligible,
            ids: outcome.hits.iter().map(|h| h.id).collect(),
        }
    }
}

/// The window a starved query is re-run with. Four times the pool gives a
/// window crowded by ineligible rows real room to recover without degenerating
/// into a full-table scan on a large corpus.
fn retry_window(k: usize) -> usize {
    k.saturating_mul(4).min(VEC0_MAX_K)
}

/// Runs one table's KNN query, recovering once from a window that the WHERE
/// clause emptied out, or that the pieces of one save filled.
///
/// The second case is a full window that offers fewer than k memories: the
/// pieces of a long save share its head, so a query about it finds them all
/// before anything else, and the collapse to one hit per save runs after the
/// pool is cut. The same single wider pass reaches the rows behind them, and
/// the result is cut at k saves rather than k rows.
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
    agents: &str,
    project_id: Option<&str>,
    k: usize,
    scoring: &ScoringConfig,
    now: TimestampMs,
    lexical_matches: &HashSet<i64>,
) -> Result<KnnOutcome, MemoryError> {
    let q = q_bytes.to_vec();
    let lexical_matches = lexical_matches.clone();
    let agent = agents.to_string();
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
                lexical_matches: &HashSet<i64>,
            ) -> Result<Vec<RawHit>, rusqlite::Error> {
                *passes += 1;
                let mut stmt = c.prepare(table.sql)?;
                let rows = stmt.query_map(
                    rusqlite::params![q, k as i64, agent, proj],
                    |row| {
                        let id = row.get(0)?;
                        Ok(RawHit {
                            id,
                            kind: table.kind,
                            content: row.get(1)?,
                            memory_type: row.get(2)?,
                            tier: row.get(3)?,
                            status: row.get(4)?,
                            distance: row.get(5)?,
                            created_at: row.get(6)?,
                            last_accessed_at: row.get(7)?,
                            importance: row.get(8)?,
                            lexical_match: lexical_matches.contains(&id),
                            agent_id: row.get(9)?,
                            topic_key: row.get(10)?,
                            project_id: row.get(11)?,
                        })
                    },
                )?;
                let mut hits = Vec::new();
                for row in rows {
                    hits.push(row?);
                }
                Ok(hits)
            }

            // Only `memory` rows are pieces of a save. Without the links the
            // pool counts rows, as it did before saves were linked.
            let links_of = |raw: &[RawHit]| {
                if table.kind != TableKind::Memory {
                    return HashMap::new();
                }
                let ids: Vec<i64> = raw.iter().map(|r| r.id).collect();
                crate::memory::pieces::read_links(c, &ids).unwrap_or_else(|e| {
                    log::warn!("knn: reading save links failed, the pool counts rows: {e}");
                    HashMap::new()
                })
            };

            let mut passes = 0usize;
            let mut eligible = None;
            let mut raw = run(c, table, &q, &agent, &proj, k, &mut passes, &lexical_matches)?;
            let mut links = links_of(&raw);
            let widen = if raw.len() < k {
                // Capped at k by the SQL's own LIMIT, so this is
                // `min(k, eligible)` — the most this query could ever return.
                let reachable: i64 = c.query_row(
                    table.eligible_sql,
                    rusqlite::params![k as i64, &agent, &proj],
                    |row| row.get(0),
                )?;
                eligible = Some(reachable as usize);
                raw.len() < reachable as usize
            } else {
                // A full window of one save's pieces offers one memory: the
                // rows past it hold the others.
                distinct_saves(raw.iter().map(|r| r.id), &links) < k
            };
            if widen {
                raw = run(c, table, &q, &agent, &proj, retry_window(k), &mut passes, &lexical_matches)?;
                links = links_of(&raw);
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
                    agent_id: r.agent_id,
                    piece: None,
                    exact: r.lexical_match,
                    topic_key: r.topic_key,
                    project_id: r.project_id,
                })
                .collect();
            // Sorted before the trim, not after: the widened window is cut
            // back to k, and the rows that survive that cut have to be the
            // highest-scoring ones. The ordering used to arrive with the rows
            // because the query produced it; the query no longer computes a
            // score, so the cut would otherwise keep whichever k rows the
            // join happened to emit first.
            hits.sort_by(hit_cmp);
            let hits = trim_to_saves(hits, &links, k);
            let saves = distinct_saves(hits.iter().map(|h| h.id), &links);
            Ok(KnnOutcome { hits, passes, saves, eligible })
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
    crate::memory::admin::format_pool_survival(
        (
            scope.memory.unwrap_or(0),
            scope.episodic.unwrap_or(0),
            scope.procedural.unwrap_or(0),
        ),
        (mem.hits.len(), epi.hits.len(), pro.hits.len()),
        (mem.retried(), epi.retried(), pro.retried()),
    )
}

// ── lexical (FTS5) candidate channel ──
//
// Wired into search_all_memories_inner's 4th try_join! arm below.

/// Maximum number of OR-ed terms an FTS5 MATCH expression may carry, taken
/// from the front of the token list. Bounds the expression so a pasted
/// paragraph cannot build an unbounded OR chain; which tokens survive is
/// deterministic (front-of-string) rather than picked by rarity — ranking
/// tokens by how rare they are is an IDF question this function does not
/// answer.
pub(crate) const MAX_LEXICAL_TOKENS: usize = 16;

/// The FTS5 MATCH expression for `query_text`, or `None` when there is
/// nothing to match on. `None` means no MATCH statement is issued at all:
/// `MATCH ''` is a syntax error, and catching a self-inflicted error is not
/// the same thing as never causing it.
///
/// Tokens are runs of `char::is_alphanumeric() || '_'`; every other
/// character is a separator. FTS5's own operators (`*` `:` `-` `"` `^` `(`
/// `)` `NEAR`) are removed BY CONSTRUCTION rather than escaped — a run of
/// alphanumeric-or-underscore characters can never contain one, so there is
/// no escaping rule to get wrong. Each surviving token is double-quoted (it
/// cannot itself contain a quote, by the same construction) and the tokens
/// are OR-joined: this channel proposes candidates, and requiring every
/// token to match would propose almost nothing.
///
/// A bare word that happens to spell an FTS5 keyword (`NEAR`, `AND`, `OR`,
/// `NOT`) is not recognized as one here — it is just another token, quoted
/// like any other, so it can only ever be searched for literally, never
/// interpreted as the operator.
pub(crate) fn fts_match_expression(query_text: &str, max_tokens: usize) -> Option<String> {
    let tokens: Vec<&str> = fts_tokens(query_text).take(max_tokens).collect();

    if tokens.is_empty() {
        return None;
    }

    Some(tokens.iter().map(|t| format!("\"{t}\"")).collect::<Vec<_>>().join(" OR "))
}

/// Splits `query_text` into the runs of `char::is_alphanumeric() || '_'` that
/// both MATCH builders quote. Nothing it yields can contain an FTS5 operator
/// or a double quote.
pub(crate) fn fts_tokens(query_text: &str) -> impl Iterator<Item = &str> {
    query_text
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|t| !t.is_empty())
}

/// MATCH expression for a person typing into a viewer: every token must be
/// present and each one matches as a prefix (`"tok"*`), so a half-typed word
/// still finds its rows. Unlike [`fts_match_expression`] it is not capped:
/// silently dropping a token would widen the result set the person narrowed.
/// `None` when the query has no token.
pub(crate) fn viewer_match_expression(query_text: &str) -> Option<String> {
    let tokens: Vec<String> = fts_tokens(query_text).map(|t| format!("\"{t}\"*")).collect();

    if tokens.is_empty() {
        return None;
    }

    Some(tokens.join(" AND "))
}

// Candidate query for the lexical channel over `memory`. `bm25()` returns a
// NEGATIVE score in SQLite — more relevant is more negative — so plain
// ascending order (SQLite's default for `ORDER BY`) is best-first. Without
// it, `LIMIT` keeps whichever rows the scan reaches first, which on this
// vtab is the lowest rowids: the oldest memories, close to the worst
// possible selection for a candidate budget.
//
// The live-status and scope predicates are those of `SEARCH_SQL_MEMORY`, the
// scope ones from the same macros. They run here, before the `LIMIT`, so the
// budget is spent on candidates the caller can be shown — a row it may not
// see never occupies one of the `?4` slots. That includes the `superseded`
// versions of a topic: `memory_fts` drops a row only when it is deleted, and
// those versions read almost like the current one.
const LEXICAL_CANDIDATE_SQL: &str = concat!(
    "
    SELECT memory_fts.rowid FROM memory_fts
    JOIN memory m ON m.id = memory_fts.rowid
    WHERE memory_fts MATCH ?1
      AND m.status IN ('active', 'disputed')
      AND m.deleted_at IS NULL
      AND ",
    reach_predicate!("m.agent_id", "?2"),
    "
      AND ",
    memory_project_predicate!("m.", "?3"),
    "
    ORDER BY bm25(memory_fts)
    LIMIT ?4
"
);

/// Hard ceiling on how many candidate rowids one lexical search may carry
/// into the scoring query, whatever budget the caller asked for.
///
/// Every surviving id becomes its own bind parameter in that query's
/// `m.id IN (...)` list, and SQLite refuses to prepare a statement carrying
/// more parameters than `SQLITE_LIMIT_VARIABLE_NUMBER`. Without a ceiling the
/// difference between an ordinary budget and an absurd one is the difference
/// between "fewer candidates than asked for" and "the scoring query does not
/// prepare at all". The second one does not even announce itself: the scoring
/// helper is fail-soft, so it would swallow the failure and contribute
/// nothing — an oversized budget silently switching OFF the channel it was
/// raised to widen. The clamp is what keeps a bad number a smaller answer
/// instead of a missing one.
///
/// 512 sits far below that limit and far above any budget with a purpose:
/// this channel proposes candidates for a final cut measured in tens, and a
/// budget in the thousands would spend more time scoring rows than the dense
/// arms spend finding them.
const MAX_LEXICAL_CANDIDATES: usize = 512;

/// The candidate budget one lexical search actually spends. Separated from
/// its single call site so the ceiling is assertable without a corpus large
/// enough to reach it.
fn lexical_budget(requested: usize) -> usize {
    requested.min(MAX_LEXICAL_CANDIDATES)
}

/// Candidate rowids for `query_text`, best-first by bm25, restricted to what
/// `agent_id`/`project_id` may see. Never fails by contract — the return
/// type carries no `Result` — because a missing table, a MATCH the parser
/// rejects, or a dead connection all mean "this channel contributed
/// nothing", which is strictly weaker than what the dense channel already
/// produced, not a reason to fail a search that would otherwise succeed.
async fn lexical_candidates(
    store: &MemoryStore,
    query_text: &str,
    agents: &str,
    project_id: Option<&str>,
    k: usize,
) -> Vec<i64> {
    let Some(match_expr) = fts_match_expression(query_text, MAX_LEXICAL_TOKENS) else {
        return Vec::new();
    };
    let k = lexical_budget(k);
    let agent = agents.to_string();
    let proj = project_id.map(str::to_owned);

    store
        .conn
        .call(move |c| -> Result<Vec<i64>, rusqlite::Error> {
            let mut stmt = c.prepare(LEXICAL_CANDIDATE_SQL)?;
            let rows = stmt.query_map(
                rusqlite::params![match_expr, agent, proj, k as i64],
                |row| row.get::<_, i64>(0),
            )?;
            let mut ids = Vec::new();
            for row in rows {
                ids.push(row?);
            }
            Ok(ids)
        })
        .await
        .unwrap_or_else(|e| {
            log::warn!("lexical_candidates: channel contributed nothing: {e}");
            Vec::new()
        })
}

/// How many identifiers one query may require of a row. Unlike
/// [`MAX_LEXICAL_TOKENS`] these come from the whole query, so an identifier
/// typed at the end of a long question still counts.
const MAX_IDENTIFIER_TOKENS: usize = 8;

/// The tokens of `query_text` written the way names are written: letters
/// with digits (`getUser4821`), an underscore (`E_AUTH_4821`) or a camelCase
/// hump (`getEstadoLabel`). The embedder reads near-identical names as nearly
/// the same text, so these are the tokens only an exact match can separate.
/// A bare number is not one: years and counts are too common to single a row
/// out.
pub(crate) fn identifier_tokens(query_text: &str) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::new();
    for token in fts_tokens(query_text) {
        if out.len() == MAX_IDENTIFIER_TOKENS {
            break;
        }
        if is_identifier(token) && !out.contains(&token) {
            out.push(token);
        }
    }
    out
}

fn is_identifier(token: &str) -> bool {
    let has_letter = token.chars().any(char::is_alphabetic);
    let has_digit = token.chars().any(char::is_numeric);
    let has_hump = token.chars().zip(token.chars().skip(1)).any(|(a, b)| a.is_lowercase() && b.is_uppercase());
    has_letter && (has_digit || token.contains('_') || has_hump)
}

/// Whether `content` holds `identifier` as written: same characters, any
/// case, not glued to a longer name. FTS5 splits on `_`, so `"user_id"` is a
/// phrase that also matches `user id` and `user-id`; this is the check that
/// tells those apart.
fn holds_identifier(content: &str, identifier: &str) -> bool {
    let content = content.to_lowercase();
    let identifier = identifier.to_lowercase();
    let is_name_char = |c: char| c.is_alphanumeric() || c == '_';
    content.match_indices(&identifier).any(|(at, _)| {
        let before = content[..at].chars().next_back();
        let after = content[at + identifier.len()..].chars().next();
        !before.is_some_and(is_name_char) && !after.is_some_and(is_name_char)
    })
}

// Rows holding every identifier of the query. Joined to `memory` so the
// live-status and `deleted_at` predicates run before the `LIMIT`: a row the
// caller cannot be shown must not use up the budget that decides whether
// the identifier is rare enough to reward.
const IDENTIFIER_MATCH_SQL: &str = concat!(
    "
    SELECT memory_fts.rowid, m.content FROM memory_fts
    JOIN memory m ON m.id = memory_fts.rowid
    WHERE memory_fts MATCH ?1
      AND m.status IN ('active', 'disputed')
      AND m.deleted_at IS NULL
      AND ",
    reach_predicate!("m.agent_id", "?2"),
    "
      AND ",
    memory_project_predicate!("m.", "?3"),
    "
    LIMIT ?4
"
);

/// Ids of the `memory` rows that hold every identifier in `query_text`, or
/// none at all when more rows than `budget` hold them: a name many rows share
/// says nothing about which one is meant, and rewarding the first `budget`
/// of them would be arbitrary. Fail-soft for the same reason as
/// [`lexical_candidates`].
async fn identifier_matches(
    store: &MemoryStore,
    query_text: &str,
    agents: &str,
    project_id: Option<&str>,
    budget: usize,
) -> HashSet<i64> {
    let tokens = identifier_tokens(query_text);
    if tokens.is_empty() {
        return HashSet::new();
    }
    let match_expr = tokens.iter().map(|t| format!("\"{t}\"")).collect::<Vec<_>>().join(" AND ");
    let budget = lexical_budget(budget);
    let agent = agents.to_string();
    let proj = project_id.map(str::to_owned);

    // The phrase can match rows that only split the name, so the window is
    // wider than the budget: those rows are dropped below and must not crowd
    // out the ones that hold it. Past the window the rows cannot all be
    // checked, and no bonus is the safe answer.
    let window = MAX_LEXICAL_CANDIDATES;
    let rows = store
        .conn
        .call(move |c| -> Result<Vec<(i64, String)>, rusqlite::Error> {
            let mut stmt = c.prepare(IDENTIFIER_MATCH_SQL)?;
            let rows = stmt.query_map(
                rusqlite::params![match_expr, agent, proj, window as i64 + 1],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )?;
            rows.collect()
        })
        .await
        .unwrap_or_else(|e| {
            log::warn!("identifier_matches: channel contributed nothing: {e}");
            Vec::new()
        });
    if rows.len() > window {
        return HashSet::new();
    }

    let split_by_fts: Vec<&str> = tokens.into_iter().filter(|t| t.contains('_')).collect();
    let ids: Vec<i64> = rows
        .into_iter()
        .filter(|(_, content)| split_by_fts.iter().all(|t| holds_identifier(content, t)))
        .map(|(id, _)| id)
        .collect();
    if ids.len() > budget {
        return HashSet::new();
    }
    ids.into_iter().collect()
}

// Second-stage scoring query for lexical candidates. The column list and
// every predicate up to (not including) the `IN (...)` clause mirror
// `SEARCH_SQL_MEMORY`, so the row-mapping closure and `compute_score` below
// apply to a lexical-only candidate exactly as they apply to a dense one, by
// construction rather than by discipline.
// `status`/`deleted_at`/`agent_id`/project are all re-applied here even
// though the candidate query already filtered on two of them: this join is
// the last gate before a row reaches a caller, and it has to stand on its
// own — a stale `memory_fts` row must not be enough by itself to leak a
// soft-deleted memory through this channel.
//
// Two places deliberately do NOT mirror `SEARCH_SQL_MEMORY`, and both are
// consequences of the same fact: this query already knows exactly which rows
// it wants.
//
// The first is the `FROM`/`JOIN`. The dense query is driven by vec0 and joins
// `memory` for the row body; this one is driven by `memory` on its rowid, and
// `NOT INDEXED` is what keeps it that way. SQLite's planner works from fixed
// row estimates here — the schema carries no `ANALYZE` — and under those
// estimates an `IN` list of four or more terms scores worse than a lookup
// through `idx_memory_agent_active`, so without `NOT INDEXED` the planner
// drives the join from that index and visits EVERY live row the agent owns,
// applying the id list afterwards as a filter. Same rows out, same distances,
// but a cost proportional to the corpus instead of to the candidate list:
// it grows with every live row the agent owns, while the rowid plan grows
// only with the ids named. The `v.rowid IN` spelling pays far more for the
// same missing hint, since its `memory_vectors` plan line becomes an
// unconstrained scan of the vector table, a separate and larger cost that
// is covered where that spelling is rejected below. `NOT INDEXED` bars
// every index on `memory`
// while explicitly leaving the rowid available, which is the one access path
// a query holding a list of ids ever wants.
//
// The list is spelled `m.id IN` rather than `v.rowid IN` for a related
// reason: measured against this vec0 build, a multi-valued `IN` on the vtab's
// rowid is not accepted as a constraint and stays a post-filter whichever
// table drives the join, so the only spelling that can reach the plan at all
// is the one naming the rowid table. Both halves are needed —
// `v.rowid IN` alone plans as an index scan from two ids up, `m.id IN` alone
// from four — and both are pinned: `the_scoring_statement_reaches_its_rows_
// by_id_at_every_candidate_count` asserts the `memory` plan line AND the
// `memory_vectors` plan line, because `NOT INDEXED` alone forces `memory` to
// `INTEGER PRIMARY KEY` under either spelling of the id list — that line
// can't tell them apart — while the `memory_vectors` line is where `m.id IN`
// (a per-id rowid lookup) and `v.rowid IN` (an unconstrained full scan)
// actually diverge. Nothing about the RESULT of this query can tell any of
// these forms apart — same rows, same distances — which is why the plan has
// to be asserted at all.
//
// The second is how it obtains `distance`. The dense query asks vec0 for its k
// nearest neighbours; this one names the ids it wants and asks for their
// distance directly, via the scalar `vec_distance_l2`. That is not an optimization,
// it is the only form that can answer the question this channel asks.
// A vec0 KNN query's own `rowid IN (...)` is evaluated as an ordinary
// post-filter over whatever the k-nearest scan already returned — measured
// against this vec0 build, and the reason that formulation was rejected — so
// ANY k-nearest formulation silently drops a requested row whose global
// distance rank falls outside `k`, which is precisely the
// identifier-beats-distance case this query exists for. There is no `k`
// large enough to fix that: vec0 refuses a `k` above `VEC0_MAX_K`, so a
// corpus past that many live vectors would lose exactly the far candidates
// the channel promotes. The scalar function has no `k` at all — it computes
// one distance per named id — and returns bit-identical values to the
// `distance` column vec0's own KNN scan reports, which
// `lexical_scoring_distance_is_bit_identical_to_the_dense_knn_distance`
// pins.
//
// `vec_distance_l2` names the metric explicitly, and it matches the metric
// `memory_vectors` was declared with (`vec0(embedding float[384])`, whose
// default is L2). Those two facts have to stay in step: the score formula
// converts this number to a cosine assuming unit vectors and a euclidean
// distance, so a table redeclared with a different `distance_metric` would
// keep both channels running while scoring lexically promoted rows on a
// different scale from every other hit.
//
// What makes that divergence loud instead of silent is not the parity test by
// itself but the corpus it runs on: its rows differ from the query vector in
// every one of the 384 coordinates, which is the only way L1, L2 and L∞ take
// different values at all. On a corpus whose rows differ in a single
// coordinate the three metrics are equal bit for bit, and a parity assertion
// written against it passes no matter which scalar this query names. That is
// not hypothetical: this query passed its own parity test with
// `vec_distance_l1` substituted here, and every other test with it, until
// that fixture was rebuilt to differ in more than one coordinate.
const LEXICAL_SCORE_SQL_PREFIX: &str = concat!(
    "
    SELECT m.id, m.content, m.memory_type, m.tier, m.status,
        vec_distance_l2(v.embedding, ?1) AS distance,
        m.created_at, m.last_accessed_at, m.importance, m.agent_id, m.topic_key, m.project_id
    FROM memory_vectors v
    JOIN memory m NOT INDEXED ON m.id = v.rowid
    WHERE m.status IN ('active', 'disputed')
      AND m.deleted_at IS NULL
      AND ",
    reach_predicate!("m.agent_id", "?2"),
    "
      AND ",
    memory_project_predicate!("m.", "?3"),
    "
"
);

/// Builds the `m.id IN (...)` placeholder list for `n` ids, explicitly
/// numbered starting at `?4` (`LEXICAL_SCORE_SQL_PREFIX` already claims
/// `?1`-`?3`). Never a bare `?`: SQLite assigns a bare `?` the index one
/// greater than the largest index ALREADY SEEN while parsing the statement
/// text left to right — not by where it sits relative to other named
/// placeholders that appear later — so a bare `?` positioned before a named
/// `?2`/`?3` elsewhere in a query would silently steal that index and bind
/// the wrong value with no error. Explicit numbering makes that outcome
/// impossible regardless of where this list is spliced into a larger
/// statement, which a bare `?` cannot guarantee.
fn lexical_rowid_placeholders(n: usize) -> String {
    (0..n).map(|i| format!("?{}", i + 4)).collect::<Vec<_>>().join(", ")
}

/// The complete scoring statement for `n` candidate ids. `score_lexical_candidates`
/// calls this function directly to obtain its SQL rather than assembling its own
/// copy, so today there is exactly one builder and its output is what production
/// prepares. That is a fact about this call site as written, not a guarantee any
/// test enforces: no test observes the literal statement `score_lexical_candidates`
/// hands to SQLite, only the text this function returns and the timing of the
/// production call, so a future edit that has `score_lexical_candidates` build a
/// diverging string instead of calling this function would not turn any test red.
fn lexical_score_sql(n: usize) -> String {
    format!("{LEXICAL_SCORE_SQL_PREFIX} AND m.id IN ({})", lexical_rowid_placeholders(n))
}

/// Scores lexical-channel candidate ids by the exact same distance and
/// `compute_score` formula the dense arms use — see
/// [`LEXICAL_SCORE_SQL_PREFIX`] for why the distance is computed per named
/// id rather than read out of a k-nearest scan.
///
/// Costs one rowid lookup and one distance computation per requested id, and
/// — because of the `NOT INDEXED` join documented on
/// [`LEXICAL_SCORE_SQL_PREFIX`] — that is the whole cost: it does not grow
/// with the number of rows the agent owns. Measured on a 5,000-row corpus, a
/// full 20-candidate budget costs 4-6x what a single candidate costs; through
/// the index-driven plan the same measurement was ~600x, and rising with the
/// corpus size. `scoring_a_full_candidate_budget_costs_the_same_in_a_large_
/// corpus_as_in_a_small_one` is what keeps that true, and it is written as a
/// crossed fixture — many candidates AND a large corpus — because either
/// dimension alone hides the difference completely.
///
/// Never fails by contract — the return type carries no `Result` — for the
/// same reason [`lexical_candidates`] does not. This channel exists to ADD
/// candidates to a search the dense arms have already answered, so there is
/// no failure of it that is worth more than the answer it would destroy by
/// propagating. Being infallible in the SIGNATURE rather than by a `?` a
/// caller happens not to write is what makes that a fact a reader can check
/// at a glance instead of a discipline every future call site has to repeat.
async fn score_lexical_candidates(
    store: &MemoryStore,
    q_bytes: &[u8],
    agents: &str,
    project_id: Option<&str>,
    ids: &[i64],
    scoring: &ScoringConfig,
    now: TimestampMs,
    lexical_matches: &HashSet<i64>,
) -> Vec<RankedHit> {
    // A round-trip skipped, not a correctness fix: `m.id IN ()` is always
    // false on its own, so this guard exists only to avoid paying for a DB
    // call whose answer is already known to be empty.
    if ids.is_empty() {
        return Vec::new();
    }
    let q = q_bytes.to_vec();
    let agent = agents.to_string();
    let proj = project_id.map(str::to_owned);
    let ids = ids.to_vec();
    let scoring = *scoring;
    let lexical_matches = lexical_matches.clone();

    store
        .conn
        .call(move |c| -> Result<Vec<RankedHit>, rusqlite::Error> {
            let sql = lexical_score_sql(ids.len());
            let mut stmt = c.prepare(&sql)?;

            let mut binds: Vec<rusqlite::types::Value> = vec![q.into(), agent.into()];
            binds.push(match proj {
                Some(p) => rusqlite::types::Value::Text(p),
                None => rusqlite::types::Value::Null,
            });
            binds.extend(ids.iter().copied().map(rusqlite::types::Value::Integer));

            let rows = stmt.query_map(rusqlite::params_from_iter(binds), |row| {
                let id = row.get(0)?;
                Ok(RawHit {
                    id,
                    kind: TableKind::Memory,
                    content: row.get(1)?,
                    memory_type: row.get(2)?,
                    tier: row.get(3)?,
                    status: row.get(4)?,
                    distance: row.get(5)?,
                    created_at: row.get(6)?,
                    last_accessed_at: row.get(7)?,
                    importance: row.get(8)?,
                    lexical_match: lexical_matches.contains(&id),
                    agent_id: row.get(9)?,
                    topic_key: row.get(10)?,
                    project_id: row.get(11)?,
                })
            })?;

            let mut hits = Vec::new();
            for row in rows {
                let r = row?;
                hits.push(RankedHit {
                    id: r.id,
                    kind: r.kind,
                    score: compute_score(&scoring, &r, now),
                    content: r.content,
                    memory_type: r.memory_type,
                    tier: r.tier,
                    status: r.status,
                    distance: r.distance,
                    created_at: r.created_at,
                    agent_id: r.agent_id,
                    piece: None,
                    exact: r.lexical_match,
                    topic_key: r.topic_key,
                    project_id: r.project_id,
                });
            }
            Ok(hits)
        })
        .await
        .unwrap_or_else(|e| {
            log::warn!("score_lexical_candidates: channel contributed nothing: {e}");
            Vec::new()
        })
}

/// Does the actual work behind `search_all_memories`, plus hands back the
/// pool-survival line it computed along the way and the `memory` pool it was
/// computed from. Split out so a test can drive the exact call site that
/// builds that line (`format_pool_line` below) through the real orchestration
/// path instead of re-assembling it from outcomes the test constructed itself
/// — the public wrapper is nothing but this function with the last two parts
/// of the tuple dropped, so nothing about the split changes what a caller of
/// `search_all_memories` observes.
async fn search_all_memories_inner(
    store: &MemoryStore,
    embedding: &[f32],
    // The lexical half of the question "what am I asking?" — sits beside
    // `embedding` (the dense half) rather than on `SearchScope`, which only
    // answers "what subset am I allowed to see?".
    query_text: Option<&str>,
    agent_id: &str,
    scope: SearchScope,
    k_final: usize,
    scoring: &ScoringConfig,
) -> Result<(Vec<RankedHit>, String, MemoryPool), MemoryError> {
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
            return Ok((Vec::new(), String::new(), MemoryPool::of(scope.memory, &KnnOutcome::skipped())));
        }
    };

    let t0 = Instant::now();
    let q_bytes = serialize_f32_query(&unit);
    // Borrowed once from `scope`, then handed to every branch below by shared
    // reference — `scope` itself is never moved out of, so this reads fine
    // even though the struct dropped `Copy` when `project_id` was added.
    let project_id = scope.project_id.as_deref();
    let agents = reach_bind(agent_id, &scope.reach);
    let agents = agents.as_str();
    // Read once and shared by every table: no two rows scored by this search
    // may have their decay computed against a different instant, which three
    // independent per-row `strftime('%s','now')` calls could never guarantee.
    let now = TimestampMs::now();

    // Run before the dense arms, not beside them: the bonus has to be in each
    // row's score before `knn_one` trims a widened window back to k. All arms
    // share one connection, so nothing ran in parallel to lose.
    let lexical_matches = match (scope.lexical, query_text) {
        (Some(k), Some(q)) => identifier_matches(store, q, agents, project_id, k).await,
        _ => HashSet::new(),
    };
    // Ids are per table and only `memory` has a lexical index.
    let no_matches = HashSet::new();

    let (mem, epi, pro, lex_ids) = tokio::try_join!(
        async {
            match scope.memory {
                Some(k) => {
                    knn_one(store, KNN_MEMORY, &q_bytes, agents, project_id, k, scoring, now, &lexical_matches).await
                }
                None => Ok(KnnOutcome::skipped()),
            }
        },
        async {
            match scope.episodic {
                Some(k) => {
                    knn_one(store, KNN_EPISODIC, &q_bytes, agents, project_id, k, scoring, now, &no_matches).await
                }
                None => Ok(KnnOutcome::skipped()),
            }
        },
        async {
            match scope.procedural {
                Some(k) => {
                    knn_one(store, KNN_PROCEDURAL, &q_bytes, agents, project_id, k, scoring, now, &no_matches).await
                }
                None => Ok(KnnOutcome::skipped()),
            }
        },
        async {
            // Infallible by construction: `lexical_candidates` never returns
            // `Result` at all, so there is exactly one `Ok(...)` in this arm
            // and nothing here can bubble an FTS5-specific failure into the
            // rest of the search. `(None, _)` and `(_, None)` both mean "the
            // channel has nothing to spend its budget on" — off, or on with
            // no query text to match — and collapse into the same empty
            // result as a sanitizer that yielded zero tokens.
            let ids = match (scope.lexical, query_text) {
                (Some(k), Some(q)) => lexical_candidates(store, q, agents, project_id, k).await,
                _ => Vec::new(),
            };
            Ok::<Vec<i64>, MemoryError>(ids)
        },
    )?;

    // Differenced against memory-kind hits ONLY, never the merged 3-table
    // pool: `memory`, `episodic_trajectory` and `procedural_skill` each keep
    // their own independent autoincrement sequence, so an id from one table
    // can equal an unrelated id from another by coincidence. Differencing
    // against the merged pool would treat that coincidence as "already
    // found" and silently drop a legitimate lexical candidate.
    let dense_ids: HashSet<i64> = mem.hits.iter().map(|h| h.id).collect();
    // An identifier match is a candidate even when the OR query left it out
    // (past its first tokens, or outranked by bm25) and the dense arm missed it.
    let mut lex_ids = lex_ids;
    let mut extra: Vec<i64> = lexical_matches.iter().copied().filter(|id| !lex_ids.contains(id)).collect();
    extra.sort_unstable();
    lex_ids.extend(extra);
    let leftover: Vec<i64> = lex_ids.into_iter().filter(|id| !dense_ids.contains(id)).collect();
    let lexical_only = leftover.len();

    // Skipped entirely when there is nothing left to score — the common case
    // for a query with no lexical signal — rather than issuing a query whose
    // `m.id IN ()` is already known to answer zero rows.
    //
    // No `?` here, and none available: `score_lexical_candidates` returns no
    // `Result` at all, so hop 5 cannot fail this search any more than hop 4
    // can. Both halves of the channel had to be infallible for that to be
    // true — a fail-soft candidate query feeding a fallible scoring query is
    // a channel that still breaks retrieval, just later.
    let lex_hits = if leftover.is_empty() {
        Vec::new()
    } else {
        score_lexical_candidates(store, &q_bytes, agents, project_id, &leftover, scoring, now, &lexical_matches).await
    };

    // Built before the merge below consumes the per-table hits, which is also
    // what lets a test assemble it from real outcomes instead of from numbers
    // it invented: the requested/returned pairing lives in one place rather
    // than being reassembled at the log call.
    let pool_survival = format_pool_line(&scope, &mem, &epi, &pro);
    let channel_attribution =
        crate::memory::admin::format_channel_attribution(mem.hits.len(), lexical_only);
    let telemetry = format!("{pool_survival} {channel_attribution}");
    let memory_pool = MemoryPool::of(scope.memory, &mem);

    // A promoted lexical candidate competes for the same k_final slots as
    // every dense hit and can displace one — there is no reserved quota. It
    // is scored by the identical formula on an identical distance, so it can
    // only win by outscoring what it displaces. The union happens strictly
    // before this sort/truncate, so a lexical candidate is never exempt from
    // the ordering that decides who survives.
    let mut all: Vec<RankedHit> =
        mem.hits.into_iter().chain(epi.hits).chain(pro.hits).chain(lex_hits).collect();
    all.sort_by(hit_cmp);
    let all = readable_hits(all, agent_id, &scope.reach);
    let memory_ids: Vec<i64> = all.iter().filter(|h| h.kind == TableKind::Memory).map(|h| h.id).collect();
    // Without the links the pieces stay separate hits, as before they were
    // linked; a search never fails over them.
    let mut all = match store.save_links(memory_ids).await {
        Ok(links) => collapse_saves(all, &links),
        Err(e) => {
            log::warn!("search: reading save links failed, pieces stay separate: {e}");
            all
        }
    };
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

    // Tier-per-hit, pool-survival and channel-attribution logging — appended
    // to the existing retrieval debug line via pure, unit-tested helpers.
    // `snapshot()` sits inside the macro args deliberately: it is the one
    // part of this line that's fine to skip sorting for when debug logging
    // is off.
    log::debug!(
        "retrieval={}ms hits={} {} k_final={} dist={} {}",
        elapsed.as_millis(),
        all.len(),
        telemetry,
        k_final,
        crate::memory::admin::format_tier_distribution(&all),
        store.knn_latency().snapshot(),
    );

    Ok((all, telemetry, memory_pool))
}

/// Top-level KNN orchestrator. Returns up to `k_final` hits sorted DESC by score.
pub async fn search_all_memories(
    store: &MemoryStore,
    embedding: &[f32],
    query_text: Option<&str>,
    agent_id: &str,
    scope: SearchScope,
    k_final: usize,
) -> Result<Vec<RankedHit>, MemoryError> {
    let (hits, _pool_survival, _memory_pool) =
        search_all_memories_inner(store, embedding, query_text, agent_id, scope, k_final, &ScoringConfig::DEFAULT)
            .await?;
    Ok(hits)
}

/// The lexical channel on its own: `agent_id`'s rowids best-first by bm25,
/// with no vector read and nothing scored. A floor to compare the full
/// ranking against, so it uses the same `MATCH` expression and order.
#[cfg(feature = "eval")]
pub(crate) async fn lexical_channel_only(store: &MemoryStore, query_text: &str, agent_id: &str, k: usize) -> Vec<i64> {
    lexical_candidates(store, query_text, &reach_bind(agent_id, &Reach::Own), None, k).await
}

/// Same orchestration as [`search_all_memories`], with the scoring config
/// exposed to the caller instead of pinned to `ScoringConfig::DEFAULT`. Every
/// production call site keeps using the unparameterized wrapper above — this
/// one exists for `rag-eval`, which needs to re-search a seeded corpus under
/// several configs without re-seeding, and reads the `memory` pool beside the
/// hits.
#[cfg(feature = "eval")]
pub(crate) async fn search_all_memories_with_scoring(
    store: &MemoryStore,
    embedding: &[f32],
    query_text: Option<&str>,
    agent_id: &str,
    scope: SearchScope,
    k_final: usize,
    scoring: &ScoringConfig,
) -> Result<(Vec<RankedHit>, MemoryPool), MemoryError> {
    let (hits, _pool_survival, memory_pool) =
        search_all_memories_inner(store, embedding, query_text, agent_id, scope, k_final, scoring).await?;
    Ok((hits, memory_pool))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::db::MemoryStore;
    use crate::memory::embedder::{EmbedRole, Embedder};
    use rusqlite::params;

    /// The bind every agent-filtered query takes, for an agent with no grants.
    fn only(agent: &str) -> String {
        reach_bind(agent, &Reach::Own)
    }

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
        assert_eq!(s.project_id, None);
        // Every production search reaches the store through this constructor,
        // so this one field is what decides whether the lexical channel runs
        // in the shipped app at all. Asserted as an exact value, not merely
        // `is_some()`: a budget of 1 leaves the channel nominally on while
        // starving it down to a single candidate, which is indistinguishable
        // from off for every fixture that matters.
        assert_eq!(s.lexical, Some(20));
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
            agent_id: None,
            piece: None,
            exact: false,
            topic_key: None,
            project_id: None,
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
            agent_id: None,
            piece: None,
            exact: false,
            topic_key: None,
            project_id: None,
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
                agent_id: None,
                piece: None,
                exact: false,
                topic_key: None,
                project_id: None,
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
        let hits = search_all_memories(&store, &q_vec, None, "agent-test", SearchScope::all_default(), 10)
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
            knn_one(&store, KNN_MEMORY, &emb, &only("agent-test"), None, 30, &ScoringConfig::DEFAULT, TimestampMs::now(), &HashSet::new()).await.unwrap().hits.len(),
            4,
            "the tier-1 row must survive inside the query, before any merge or truncation"
        );
    }

    // `procedural_skill` rows are born at tier 1 (`compute_tier` pins that
    // type there) and nothing demotes them, so a pool that skipped tier 1
    // returned zero skills for every query ever issued — every stored skill
    // was unreachable through this channel.
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
            knn_one(&store, KNN_PROCEDURAL, &q_bytes, &only("agent-skill"), None, 5, &ScoringConfig::DEFAULT, TimestampMs::now(), &HashSet::new())
                .await
                .unwrap()
                .hits
                .len(),
            1,
            "the procedural pool must return the skill row"
        );

        let hits = search_all_memories(&store, &q_vec, None, "agent-skill", SearchScope::all_default(), 10)
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
        let hits = search_all_memories(&store, &query, None, "agent-normalize", SearchScope::all_default(), 10)
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
        let hits = search_all_memories(&store, &zero_query, None, "agent-zero-query", SearchScope::all_default(), 10)
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
        let hits = search_all_memories(&store, &q_vec, None, "agent-test", SearchScope::all_default(), 10)
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
        let hits = search_all_memories(&store, &q_vec, None, "agent-test", SearchScope::all_default(), 10)
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
        let hits = search_all_memories(&store, &q_vec, None, agent, SearchScope::all_default(), 10)
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
        let hits = search_all_memories(&store, &q_vec, None, agent, SearchScope::all_default(), 10)
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
    async fn disputed_status_does_not_change_score() {
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
        let active_hits = search_all_memories(&store, &q_vec, None, agent, SearchScope::all_default(), 10)
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

        let all_hits = search_all_memories(&store, &q_vec, None, agent, SearchScope::all_default(), 10)
            .await.unwrap();
        let disputed_score = all_hits.iter().find(|h| h.id == disputed_id).unwrap().score;

        assert_eq!(disputed_score, active_score,
            "a disputed row must rank like an active one");
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
        let hits = search_all_memories(&store, &q_vec, None, agent, SearchScope::all_default(), 3)
            .await.unwrap();

        assert_eq!(hits.len(), 3);
        assert!(hits[0].score >= hits[1].score, "must be sorted DESC");
        assert!(hits[1].score >= hits[2].score, "must be sorted DESC");
    }

    #[tokio::test]
    async fn search_respects_scope_none_skips_table() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let q_vec = unit_basis_vec();
        let scope = SearchScope { memory: None, episodic: None, procedural: None, project_id: None, lexical: None, reach: Reach::Own };
        let hits = search_all_memories(&store, &q_vec, None, "agent-x", scope, 10)
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
        let hits = search_all_memories(&store, &q_vec, None, agent, SearchScope::all_default(), 2)
            .await.unwrap();

        assert_eq!(hits.len(), 2, "k_final=2 must return exactly 2 hits");

        // The assertion above can't distinguish correct truncation from a
        // pool that was already short before truncation ran. Asserting the
        // pre-truncation pool size directly proves k_final is what cuts the
        // result, not a starved pool.
        assert_eq!(
            knn_one(&store, KNN_MEMORY, &emb, &only(agent), None, 30, &ScoringConfig::DEFAULT, TimestampMs::now(), &HashSet::new()).await.unwrap().hits.len(),
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

        search_all_memories(&store_a, &q_vec, None, "agent-iso-a", SearchScope::all_default(), 10)
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

        search_all_memories(&store, &q_vec, None, "agent-iso-clone", SearchScope::all_default(), 10)
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
            search_all_memories(&store, &q_vec, None, "agent-iso-n", SearchScope::all_default(), 10)
                .await
                .unwrap();
        }

        assert_eq!(store.knn_latency().snapshot().n, 3);
    }

    #[tokio::test]
    async fn search_empty_db_returns_empty_vec() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let q_vec = unit_basis_vec();
        let hits = search_all_memories(&store, &q_vec, None, "agent-x", SearchScope::all_default(), 10)
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

        let hits = search_all_memories(&store, &q_vec, None, agent, SearchScope::all_default(), 3)
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

        let hits = search_all_memories(&store, &emb, None, "agent-a", SearchScope::all_default(), 10)
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

        let hits = search_all_memories(&store, &emb, None, "agent-b", SearchScope::all_default(), 10)
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
        let hits = search_all_memories(&store, &q_vec, None, "agent-scope", scope, 10).await.unwrap();

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
        let hits = search_all_memories(&store, &emb, None, "agent-scope", scope, 10).await.unwrap();

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
        let hits = search_all_memories(&store, &emb, None, "agent-scope", scope, 10).await.unwrap();

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
        let hits = search_all_memories(&store, &q_vec, None, "agent-scope-null", scope, 10).await.unwrap();

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
        let hits = search_all_memories(&store, &emb, None, "agent-scope-null", scope, 10).await.unwrap();

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
        let hits = search_all_memories(&store, &emb, None, "agent-scope-null", scope, 10).await.unwrap();

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
        let hits = search_all_memories(&store, &q_vec, None, "agent-pref-cross", scope, 10).await.unwrap();

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

        let outcome = knn_one(&store, KNN_MEMORY, &near, &only(agent), Some("P1"), 30, &ScoringConfig::DEFAULT, TimestampMs::now(), &HashSet::new())
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
    // rows, rag-eval included. The `?4 IS NULL` disjunct in the SQL is what
    // guarantees it: breaking that disjunct makes this test fail.
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
        let hits = search_all_memories(&store, &q_vec, None, "agent-unscoped", SearchScope::all_default(), 10)
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
                Ok(c.query_row(ELIGIBLE_SQL_MEMORY, params![k, only(&agent), proj], |row| {
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

        let outcome = knn_one(&store, KNN_MEMORY, &emb, &only("agent-full"), Some("P1"), 5, &ScoringConfig::DEFAULT, TimestampMs::now(), &HashSet::new())
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

        let outcome = knn_one(&store, KNN_MEMORY, &emb, &only("agent-small"), Some("P1"), 30, &ScoringConfig::DEFAULT, TimestampMs::now(), &HashSet::new())
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

        let outcome = knn_one(&store, KNN_MEMORY, &near, &only("agent-starved"), Some("P1"), 30, &ScoringConfig::DEFAULT, TimestampMs::now(), &HashSet::new())
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

        let outcome = knn_one(&store, KNN_MEMORY, &near, &only("agent-trunc"), Some("P1"), 10, &ScoringConfig::DEFAULT, TimestampMs::now(), &HashSet::new())
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
            lexical: None,
            reach: Reach::Own,
        };
        let k = scope.memory.unwrap();
        // 30 foreign rows fill the first window exactly; 40 eligible rows sit
        // behind them, so the recovery can hand back a genuinely full pool.
        bulk_insert_memory_rows(&store, "agent-telemetry", Some("P2"), 30, near.clone()).await;
        bulk_insert_memory_rows(&store, "agent-telemetry", Some("P1"), 40, far).await;

        let mem = knn_one(&store, KNN_MEMORY, &near, &only("agent-telemetry"), Some("P1"), k, &ScoringConfig::DEFAULT, TimestampMs::now(), &HashSet::new())
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
            lexical: None,
            reach: Reach::Own,
        };
        bulk_insert_memory_rows(&store, "agent-telemetry-wired", Some("P2"), 30, serialize_embedding(&near)).await;
        bulk_insert_memory_rows(&store, "agent-telemetry-wired", Some("P1"), 40, serialize_embedding(&far)).await;

        let (hits, line, _) =
            search_all_memories_inner(&store, &near, None, "agent-telemetry-wired", scope, 30, &ScoringConfig::DEFAULT)
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

        let outcome = knn_one(&store, KNN_MEMORY, &near, &only("agent-deleted"), Some("P1"), 30, &ScoringConfig::DEFAULT, TimestampMs::now(), &HashSet::new())
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

        let outcome = knn_one(&store, KNN_MEMORY, &near, &only("agent-shared"), Some("P1"), 30, &ScoringConfig::DEFAULT, TimestampMs::now(), &HashSet::new())
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
    // score a row actually earns once importance/tier are applied. A row
    // that arrives CLOSER — and so earlier in that stream — can still score
    // LOWER than one that arrives farther away. Here NEAR is tier 3 and
    // unimportant, FAR is tier 1 and maximally important, and FAR's score
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

        // NEAR: closest of the caller's own rows, but low tier and
        // unimportant — its score has to lose despite arriving first.
        let near_emb = serialize_embedding(&vec_at_cosine(0.999));
        let near_id = insert_memory_row(
            &store, agent, "near but low score", "chat_message", 3, 1.0, "active", near_emb,
        ).await;

        // FAR: farther from the query than NEAR, but top tier and maximally
        // important — its score has to win despite arriving second. Kept
        // above `ScoringConfig::DEFAULT`'s cosine re-scale floor on purpose:
        // a cosine below that floor clamps to a score of exactly zero
        // regardless of importance (a separate, already-covered behavior),
        // which would make this fixture test the floor instead of the thing
        // it exists to test — truncation picking the true top score rather
        // than arrival order.
        let far_emb = serialize_embedding(&vec_at_cosine(0.95));
        let far_id = insert_memory_row(
            &store, agent, "far but high score", "chat_message", 1, 10.0, "active", far_emb,
        ).await;

        let q_bytes = serialize_embedding(&unit_basis_vec());
        let outcome = knn_one(&store, KNN_MEMORY, &q_bytes, &only(agent), Some("P1"), 1, &ScoringConfig::DEFAULT, TimestampMs::now(), &HashSet::new())
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

        let outcome = knn_one(&store, KNN_MEMORY, &near, &only("agent-ceiling"), Some("P1"), k, &ScoringConfig::DEFAULT, TimestampMs::now(), &HashSet::new())
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
        let outcome = knn_one(&store, KNN_EPISODIC, &q_bytes, &only("agent-epi-retry"), Some("P1"), 5, &ScoringConfig::DEFAULT, TimestampMs::now(), &HashSet::new())
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
        let outcome = knn_one(&store, KNN_PROCEDURAL, &q_bytes, &only("agent-pro-retry"), Some("P1"), 5, &ScoringConfig::DEFAULT, TimestampMs::now(), &HashSet::new())
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
        let epi = knn_one(&store, KNN_EPISODIC, &q_bytes, &only("agent-x"), Some("P1"), 5, &ScoringConfig::DEFAULT, TimestampMs::now(), &HashSet::new()).await.unwrap();
        let pro = knn_one(&store, KNN_PROCEDURAL, &q_bytes, &only("agent-x"), Some("P1"), 5, &ScoringConfig::DEFAULT, TimestampMs::now(), &HashSet::new()).await.unwrap();

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

        let hits = search_all_memories(&store, &emb, None, "agent-uns-epi", SearchScope::all_default(), 10)
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

        let hits = search_all_memories(&store, &emb, None, "agent-uns-pro", SearchScope::all_default(), 10)
            .await
            .unwrap();

        for id in [p1, p2, none] {
            assert!(
                hits.iter().any(|h| h.id == id && h.kind == TableKind::Procedural),
                "unscoped query must return procedural row {id} regardless of its project_id"
            );
        }
    }

    // ── lexical (FTS5) sanitizer ──

    #[test]
    fn viewer_match_requires_every_token() {
        assert_eq!(
            viewer_match_expression("Config núcleo_2"),
            Some("\"Config\"* AND \"núcleo_2\"*".to_string())
        );
    }

    #[test]
    fn punctuation_only_query_has_no_expression() {
        assert_eq!(viewer_match_expression("-^()\"* :"), None);
        assert_eq!(viewer_match_expression("   "), None);
    }

    #[test]
    fn viewer_match_keeps_fts_keywords_literal() {
        assert_eq!(
            viewer_match_expression("find NEAR OR bug"),
            Some("\"find\"* AND \"NEAR\"* AND \"OR\"* AND \"bug\"*".to_string())
        );
    }

    #[test]
    fn fts_match_expression_folds_accented_spanish_text_into_ored_tokens() {
        assert_eq!(
            fts_match_expression("configuración del núcleo", MAX_LEXICAL_TOKENS),
            Some("\"configuración\" OR \"del\" OR \"núcleo\"".to_string())
        );
    }

    #[test]
    fn fts_match_expression_splits_hyphenated_identifier() {
        assert_eq!(
            fts_match_expression("GPX-Store", MAX_LEXICAL_TOKENS),
            Some("\"GPX\" OR \"Store\"".to_string())
        );
    }

    #[test]
    fn fts_match_expression_splits_colon_separated_terms() {
        assert_eq!(
            fts_match_expression("foo:bar", MAX_LEXICAL_TOKENS),
            Some("\"foo\" OR \"bar\"".to_string())
        );
    }

    #[test]
    fn fts_match_expression_near_keyword_survives_quoted_as_a_literal_token() {
        assert_eq!(
            fts_match_expression("find NEAR the bug", MAX_LEXICAL_TOKENS),
            Some("\"find\" OR \"NEAR\" OR \"the\" OR \"bug\"".to_string())
        );
    }

    #[test]
    fn fts_match_expression_unbalanced_quote_is_stripped_as_a_separator() {
        assert_eq!(
            fts_match_expression("\"unbalanced", MAX_LEXICAL_TOKENS),
            Some("\"unbalanced\"".to_string())
        );
    }

    #[test]
    fn fts_match_expression_lone_asterisk_yields_none() {
        assert_eq!(fts_match_expression("*", MAX_LEXICAL_TOKENS), None);
    }

    #[test]
    fn fts_match_expression_empty_query_yields_none() {
        assert_eq!(fts_match_expression("", MAX_LEXICAL_TOKENS), None);
    }

    // Symbol characters only, no alphanumeric word among them. A query that
    // ALSO contains the bare word "NEAR" is a different case: "NEAR" is
    // alphanumeric, so it survives as its own quoted token (see the
    // NEAR-as-literal test above) and the token list is not empty there.
    #[test]
    fn fts_match_expression_operator_soup_with_no_alphanumeric_yields_none() {
        assert_eq!(fts_match_expression("-^()\"* :", MAX_LEXICAL_TOKENS), None);
    }

    #[test]
    fn fts_match_expression_double_underscore_identifier_is_one_token() {
        assert_eq!(
            fts_match_expression("V8__memory_fts", MAX_LEXICAL_TOKENS),
            Some("\"V8__memory_fts\"".to_string())
        );
    }

    #[test]
    fn fts_match_expression_truncates_to_max_tokens_from_the_front() {
        assert_eq!(
            fts_match_expression("one two three four five", 2),
            Some("\"one\" OR \"two\"".to_string())
        );
    }

    /// The truncation test above passes `2` explicitly and never touches the
    /// shipped `MAX_LEXICAL_TOKENS` — a query with 17 distinct tokens is one
    /// more than the constant's current value of 16, so this fails if the
    /// constant grows (all 17 would survive) or shrinks (fewer than 16 would
    /// survive) in either direction, unlike a test that supplies its own limit.
    #[test]
    fn fts_match_expression_caps_at_the_shipped_max_lexical_tokens_constant() {
        let query = (1..=17).map(|i| format!("tok{i}")).collect::<Vec<_>>().join(" ");
        let expr = fts_match_expression(&query, MAX_LEXICAL_TOKENS)
            .expect("17 alphanumeric tokens must yield a non-empty expression");

        for i in 1..=16 {
            assert!(
                expr.contains(&format!("\"tok{i}\"")),
                "token {i} of 17 must survive under the shipped MAX_LEXICAL_TOKENS; expr={expr}"
            );
        }
        assert!(
            !expr.contains("\"tok17\""),
            "the 17th token must be truncated away by the shipped MAX_LEXICAL_TOKENS; expr={expr}"
        );
    }

    // `LEXICAL_SCORE_SQL_PREFIX` claims ?1-?3, so the generated list must
    // start at ?4 and number every hole explicitly — pinned as a string
    // equality so a regression to bare `?` is caught here. Verified
    // empirically that a bare `?` would not corrupt a bind for this exact
    // placement today: SQLite's own auto-numbering happens to land on the
    // same indices, since nothing upstream of this list introduces a gap.
    // Explicit numbering is kept anyway as the defensively-correct choice —
    // that auto-numbering rule depends on textual order, not on placement,
    // so it would silently break the day a placeholder is added or
    // reordered ahead of this one. This assertion is also what makes the
    // offset and the prefix's parameter count impossible to change
    // independently: the two only agree while both are edited together.
    #[test]
    fn lexical_rowid_placeholders_are_explicitly_numbered_from_four() {
        assert_eq!(lexical_rowid_placeholders(0), "");
        assert_eq!(lexical_rowid_placeholders(1), "?4");
        assert_eq!(lexical_rowid_placeholders(3), "?4, ?5, ?6");
    }

    // The clamp is only reachable through a caller-supplied budget, which no
    // production call site sets above 20 — so without this it is a defensive
    // check nothing exercises, and removing it would go unnoticed until a
    // scope built somewhere else fed a large number straight into a bind
    // list SQLite then refuses to prepare.
    #[test]
    fn lexical_budget_is_clamped_to_the_bind_parameter_ceiling() {
        assert_eq!(lexical_budget(20), 20, "an ordinary budget passes through untouched");
        assert_eq!(lexical_budget(MAX_LEXICAL_CANDIDATES), MAX_LEXICAL_CANDIDATES);
        assert_eq!(lexical_budget(MAX_LEXICAL_CANDIDATES + 1), MAX_LEXICAL_CANDIDATES);
        assert_eq!(lexical_budget(usize::MAX), MAX_LEXICAL_CANDIDATES);
    }

    // Two halves of one claim, deliberately in the same test because either
    // alone is worthless: the clamp really does bound what the candidate
    // query returns (so the ceiling is not merely arithmetic in a helper),
    // and a bind list of exactly that many ids really does prepare and run
    // (so the ceiling is a number SQLite accepts, not a guess).
    #[tokio::test]
    async fn a_budget_past_the_ceiling_is_clamped_and_the_clamped_list_still_scores() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let agent = "agent-budget-ceiling";
        let count = MAX_LEXICAL_CANDIDATES + 88;
        insert_rows_at_increasing_distance(&store, agent, "ceiling_shared_token row", count).await;

        let ids = lexical_candidates(&store, "ceiling_shared_token", &only(agent), None, usize::MAX).await;
        assert_eq!(
            ids.len(),
            MAX_LEXICAL_CANDIDATES,
            "{count} rows match, so an unclamped budget would return all of them"
        );

        let hits = score_lexical_candidates(
            &store,
            &serialize_f32_query(&unit_basis_vec()),
            &only(agent),
            None,
            &ids,
            &ScoringConfig::DEFAULT,
            TimestampMs::now(), &HashSet::new(),
        )
        .await;
        assert_eq!(
            hits.len(),
            MAX_LEXICAL_CANDIDATES,
            "a full-ceiling bind list must still prepare and run"
        );
    }

    // ── lexical candidate query (FTS5 MATCH + scope) ──

    // Agent B's rows are short (just the token), so bm25 ranks every one of
    // them ahead of agent A's longer rows. If the scope predicate were
    // applied AFTER the scan instead of inside the WHERE clause, the top 20
    // by bm25 across both agents would be entirely B's rows and agent A's
    // query would come back empty — a post-scan filter returns 0, not 20.
    #[tokio::test]
    async fn lexical_candidate_query_applies_scope_before_limit() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let emb = unit_basis_embedding();

        for i in 0..30 {
            insert_memory_row(
                &store, "agent-b", "shared_tok", "chat_message", 3, 5.0, "active", emb.clone(),
            ).await;
            let _ = i;
        }
        let mut a_ids = Vec::new();
        for i in 0..20 {
            a_ids.push(
                insert_memory_row(
                    &store,
                    "agent-a",
                    &format!("shared_tok padding filler words number {i} to dilute the bm25 rank"),
                    "chat_message",
                    3,
                    5.0,
                    "active",
                    emb.clone(),
                )
                .await,
            );
        }

        let ids = lexical_candidates(&store, "shared_tok", &only("agent-a"), None, 20).await;

        assert_eq!(
            ids.len(),
            20,
            "agent A has exactly 20 permitted matches; the budget must not be spent on agent B's better-ranked rows"
        );
        for id in &ids {
            assert!(a_ids.contains(id), "candidate {id} does not belong to agent A");
        }
    }

    // 20 noisy rows are inserted first (lower rowids), each a weak bm25
    // match. The single best match is inserted LAST (highest rowid) and
    // contains nothing but the token. Without `ORDER BY bm25(memory_fts)` a
    // plain `LIMIT 20` keeps the lowest rowids and drops it; with it, the
    // best match bubbles into the window despite arriving last.
    #[tokio::test]
    async fn lexical_candidate_query_order_by_bm25_pulls_best_match_into_window() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let emb = unit_basis_embedding();
        let agent = "agent-bm25-order";

        for i in 0..20 {
            insert_memory_row(
                &store,
                agent,
                &format!("unique_tok padding filler words number {i} to dilute the bm25 rank even further with more text"),
                "chat_message",
                3,
                5.0,
                "active",
                emb.clone(),
            )
            .await;
        }
        let best_id = insert_memory_row(
            &store, agent, "unique_tok", "chat_message", 3, 5.0, "active", emb.clone(),
        ).await;

        let ids = lexical_candidates(&store, "unique_tok", &only(agent), None, 20).await;

        assert!(
            ids.contains(&best_id),
            "the best bm25 match must survive the LIMIT even though it was inserted last \
             (highest rowid) — without ORDER BY bm25(...) a plain LIMIT keeps the lowest \
             rowids and drops it"
        );
    }

    // A topic revised with `topic_key` leaves its old versions in `memory_fts`
    // (only a delete takes a row out), each nearly the text of the current one.
    // Shorter, they outrank it by bm25; filtered after the `LIMIT`, they would
    // fill the budget and leave the current version out.
    #[tokio::test]
    async fn superseded_versions_do_not_spend_the_lexical_budget() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let emb = unit_basis_embedding();
        let agent = "agent-revised";
        for i in 0..3 {
            insert_memory_row(
                &store, agent, &format!("revtok version {i}"), "semantic_fact", 3, 5.0, "superseded", emb.clone(),
            )
            .await;
        }
        let current = insert_memory_row(
            &store, agent, "revtok version 3, the wording that holds today", "semantic_fact", 3, 5.0, "active", emb,
        )
        .await;

        let ids = lexical_candidates(&store, "revtok", &only(agent), None, 2).await;

        assert_eq!(ids, vec![current]);
    }

    /// The candidate query is driven by the MATCH and reaches `memory` by rowid
    /// for each match: driven from an index on `memory` instead, it would visit
    /// every live row to test the MATCH on each.
    #[tokio::test]
    async fn the_lexical_candidate_query_is_driven_by_the_match() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let plan = store
            .conn
            .call(|c| -> Result<Vec<String>, rusqlite::Error> {
                let mut stmt = c.prepare(&format!("EXPLAIN QUERY PLAN {LEXICAL_CANDIDATE_SQL}"))?;
                let rows = stmt.query_map(
                    rusqlite::params!["\"tok\"", only("plan-probe"), Option::<String>::None, 20],
                    |r| r.get::<_, String>(3),
                )?;
                rows.collect()
            })
            .await
            .unwrap();

        let fts = plan.iter().position(|l| l.contains("memory_fts VIRTUAL TABLE"));
        let memory = plan.iter().position(|l| l.contains(" m USING INTEGER PRIMARY KEY"));
        assert!(
            matches!((fts, memory), (Some(f), Some(m)) if f < m),
            "expected memory_fts as the outer loop and `memory` by rowid, got {plan:#?}"
        );
    }

    #[tokio::test]
    async fn lexical_candidates_folds_accents_between_query_and_stored_content() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let emb = unit_basis_embedding();
        let id = insert_memory_row(
            &store,
            "agent-accent",
            "informe de configuración del sistema",
            "chat_message",
            3,
            5.0,
            "active",
            emb,
        ).await;

        let ids = lexical_candidates(&store, "configuracion", &only("agent-accent"), None, 20).await;

        assert!(
            ids.contains(&id),
            "an unaccented query must reach a row stored with the accented form — \
             unicode61 remove_diacritics 2 folds both at index and query time"
        );
    }

    #[tokio::test]
    async fn lexical_candidate_query_user_preference_crosses_project_boundary() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let emb = unit_basis_embedding();
        let id = insert_memory_row_with_project(
            &store, "agent-pref-lex", Some("P2"), "user_preference", "pref_tok content", emb,
        ).await;

        let ids = lexical_candidates(&store, "pref_tok", &only("agent-pref-lex"), Some("P1"), 20).await;

        assert!(
            ids.contains(&id),
            "a user_preference row must remain visible across project boundaries in the \
             lexical channel too, exactly as it already does in the dense KNN channel"
        );
    }

    #[tokio::test]
    async fn lexical_candidates_fails_soft_when_table_missing() {
        let store = MemoryStore::new_for_test().await.unwrap();
        store.conn.call(|c| c.execute("DROP TABLE memory_fts", [])).await.unwrap();

        let ids = lexical_candidates(&store, "anything", &only("agent-x"), None, 20).await;

        assert!(
            ids.is_empty(),
            "a missing memory_fts table must degrade to an empty channel, never panic or propagate an error"
        );
    }

    // ── lexical scoring query (rowid-driven `m.id IN (...)`) ──

    // Both rows share the same embedding, importance, tier, status and
    // last_accessed_at, so any score difference is attributable only to
    // which query path computed it — the dense KNN query or the lexical
    // scoring query.
    #[tokio::test]
    async fn score_lexical_candidates_matches_the_dense_scoring_formula_exactly() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let emb = unit_basis_embedding();
        let agent = "agent-score-parity";

        let dense_id = insert_memory_row(&store, agent, "dense row", "chat_message", 3, 5.0, "active", emb.clone()).await;
        let lexical_id = insert_memory_row(&store, agent, "lexical row", "chat_message", 3, 5.0, "active", emb.clone()).await;

        let now = TimestampMs::now();
        let dense_hits = knn_one(&store, KNN_MEMORY, &emb, &only(agent), None, 30, &ScoringConfig::DEFAULT, now, &HashSet::new())
            .await
            .unwrap();
        let dense_score = dense_hits.hits.iter().find(|h| h.id == dense_id).unwrap().score;

        let lexical_hits = score_lexical_candidates(&store, &emb, &only(agent), None, &[lexical_id], &ScoringConfig::DEFAULT, now, &HashSet::new())
            .await;
        let lexical_score = lexical_hits.iter().find(|h| h.id == lexical_id).unwrap().score;

        assert_eq!(
            dense_score, lexical_score,
            "identical distance/importance/recency must score identically regardless of which channel found the candidate"
        );
    }

    // Requests a non-contiguous subset of ids so a placeholder-index mistake
    // would surface as either the wrong rows coming back or a bind error.
    #[tokio::test]
    async fn score_lexical_candidates_returns_exactly_the_requested_ids_with_multiple_placeholders() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let emb = unit_basis_embedding();
        let agent = "agent-multi-ids";

        let mut ids = Vec::new();
        for i in 0..5 {
            ids.push(insert_memory_row(&store, agent, &format!("row {i}"), "chat_message", 3, 5.0, "active", emb.clone()).await);
        }
        let requested = vec![ids[0], ids[2], ids[4]];

        let hits = score_lexical_candidates(&store, &emb, &only(agent), None, &requested, &ScoringConfig::DEFAULT, TimestampMs::now(), &HashSet::new())
            .await;

        let mut returned: Vec<i64> = hits.iter().map(|h| h.id).collect();
        returned.sort();
        let mut expected = requested.clone();
        expected.sort();
        assert_eq!(returned, expected, "exactly the requested ids, and only those, must come back");
    }

    // The id is passed directly, as if a candidate query had somehow
    // surfaced it (a stale `memory_fts` row, or any other source) — proving
    // the scoring join's own `deleted_at` predicate is independently
    // sufficient, not merely redundant with the FTS removal trigger.
    #[tokio::test]
    async fn score_lexical_candidates_excludes_soft_deleted_rows_even_when_requested_by_id() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let emb = unit_basis_embedding();
        let agent = "agent-score-softdel";
        let id = insert_memory_row(&store, agent, "will be deleted", "chat_message", 3, 5.0, "active", emb.clone()).await;
        soft_delete_keeping_vector(&store, id).await;

        let hits = score_lexical_candidates(&store, &emb, &only(agent), None, &[id], &ScoringConfig::DEFAULT, TimestampMs::now(), &HashSet::new())
            .await;

        assert!(hits.is_empty(), "a soft-deleted memory must never be scored, even if its id reaches this query directly");
    }

    // The comment above `LEXICAL_SCORE_SQL_PREFIX` claims all four scope
    // predicates are re-applied because this join is the last gate before a
    // row reaches a caller and has to stand on its own. The soft-delete case
    // above pins one of them; these three pin the rest, so the claim is an
    // asserted property of the query rather than a description of it. Each
    // passes the id in directly, as a stale `memory_fts` row or a bug in the
    // candidate query would.
    #[tokio::test]
    async fn score_lexical_candidates_never_crosses_an_agent_boundary() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let emb = unit_basis_embedding();
        let other = insert_memory_row(
            &store, "agent-owner", "another agent's row", "chat_message", 3, 5.0, "active", emb.clone(),
        )
        .await;

        let hits = score_lexical_candidates(
            &store, &emb, &only("agent-intruder"), None, &[other], &ScoringConfig::DEFAULT, TimestampMs::now(), &HashSet::new(),
        )
        .await;

        assert!(hits.is_empty(), "a row belonging to another agent must never be scored for this caller");
    }

    #[tokio::test]
    async fn score_lexical_candidates_excludes_rows_outside_the_live_status_set() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let emb = unit_basis_embedding();
        let agent = "agent-score-status";
        let superseded = insert_memory_row(
            &store, agent, "an outdated fact", "semantic_fact", 3, 5.0, "superseded", emb.clone(),
        )
        .await;

        let hits = score_lexical_candidates(
            &store, &emb, &only(agent), None, &[superseded], &ScoringConfig::DEFAULT, TimestampMs::now(), &HashSet::new(),
        )
        .await;

        assert!(hits.is_empty(), "only 'active' and 'disputed' rows are eligible to reach a caller");
    }

    // The disjunct is an EXEMPTION, so the killing evidence is a row that must
    // come back, not one that must not: dropping it would make this channel
    // stricter than the dense one about which project-scoped rows are
    // permitted, and a user preference would go missing lexically while
    // surfacing densely.
    #[tokio::test]
    async fn score_lexical_candidates_keeps_the_user_preference_project_exemption() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let emb = unit_basis_embedding();
        let agent = "agent-score-pref";
        let pref = insert_memory_row_with_project(
            &store, agent, Some("project-other"), "user_preference", "always answer in Spanish", emb.clone(),
        )
        .await;

        let hits = score_lexical_candidates(
            &store, &emb, &only(agent), Some("project-current"), &[pref], &ScoringConfig::DEFAULT, TimestampMs::now(), &HashSet::new(),
        )
        .await;

        assert_eq!(
            hits.iter().map(|h| h.id).collect::<Vec<_>>(),
            vec![pref],
            "a user preference reaches every project, exactly as it does through the dense arm"
        );
    }

    #[tokio::test]
    async fn score_lexical_candidates_returns_empty_for_an_empty_id_list() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let emb = unit_basis_embedding();
        let hits = score_lexical_candidates(&store, &emb, &only("agent-empty"), None, &[], &ScoringConfig::DEFAULT, TimestampMs::now(), &HashSet::new())
            .await;
        assert!(hits.is_empty());
    }

    /// Reads BOTH the `memory` and `memory_vectors` lines out of
    /// `EXPLAIN QUERY PLAN` for the real scoring statement at `n` candidate
    /// ids. Both are needed: `NOT INDEXED` pins `memory` to
    /// `INTEGER PRIMARY KEY` under every spelling of the id list, so that
    /// line alone cannot tell `m.id IN` apart from `v.rowid IN` — only the
    /// `memory_vectors` line does, and it is the one that actually flips
    /// between a per-id rowid lookup and an unconstrained full scan.
    async fn scoring_plan_lines_for(store: &MemoryStore, n: usize) -> (String, String) {
        let sql = format!("EXPLAIN QUERY PLAN {}", lexical_score_sql(n));
        store
            .conn
            .call(move |c| -> Result<(String, String), rusqlite::Error> {
                let mut stmt = c.prepare(&sql)?;
                let mut binds: Vec<rusqlite::types::Value> = vec![
                    rusqlite::types::Value::Blob(serialize_f32_query(&unit_basis_vec())),
                    rusqlite::types::Value::Text(only("plan-probe")),
                    rusqlite::types::Value::Null,
                ];
                binds.extend((0..n).map(|i| rusqlite::types::Value::Integer(i as i64 + 1)));
                let rows =
                    stmt.query_map(rusqlite::params_from_iter(binds), |r| r.get::<_, String>(3))?;
                let mut plan = Vec::new();
                for r in rows {
                    plan.push(r?);
                }
                let memory_line = plan
                    .iter()
                    .find(|l| l.ends_with(" m") || l.contains(" m "))
                    .cloned()
                    .unwrap_or_else(|| "no line for `memory` in the plan".to_string());
                let vector_line = plan
                    .iter()
                    .find(|l| l.ends_with(" v") || l.contains(" v "))
                    .cloned()
                    .unwrap_or_else(|| "no line for `memory_vectors` in the plan".to_string());
                Ok((memory_line, vector_line))
            })
            .await
            .unwrap()
    }

    /// This query knows the exact ids it wants, so the only defensible way for
    /// it to reach them is by id. The plan is asserted rather than inferred
    /// because nothing else can see the difference: every other formulation
    /// returns the identical rows with identical distances and differs only in
    /// how many rows it had to visit to produce them — a defect with no
    /// observable output, which is why it survived a full green suite once.
    ///
    /// Two planner facts make this worth pinning at several `n` rather than
    /// one. SQLite rewrites a single-element `IN (?)` into `rowid = ?`, so a
    /// one-id probe takes the fast path under EVERY formulation and proves
    /// nothing about the others. And with no `ANALYZE` in this schema the
    /// planner works from fixed row estimates, under which an `IN` list of
    /// four or more terms looks more expensive than a lookup through
    /// `idx_memory_agent_active` — so a form that plans correctly at two ids
    /// can still fall back to scanning the agent's rows at four. `NOT INDEXED`
    /// is what removes that choice: it bars every index on `memory` while
    /// explicitly leaving the rowid available, which is the one access path
    /// this statement ever wants.
    ///
    /// Both halves of the join have to be checked. `NOT INDEXED` alone already
    /// forces the `memory` line to `INTEGER PRIMARY KEY` regardless of whether
    /// the id list is spelled `m.id IN` or `v.rowid IN` — measured, both
    /// spellings produce that exact line — so the `memory` assertion by
    /// itself cannot distinguish them. What does distinguish them is the
    /// `memory_vectors` line: `m.id IN` drives the join from `memory` and
    /// looks each vector up by rowid (`INDEX 3:` in this vec0 build);
    /// `v.rowid IN` drives it from `memory_vectors` instead and, because a
    /// multi-valued `IN` is not accepted as a vec0 constraint, degrades to an
    /// unconstrained scan of every live vector (`INDEX 0:1`) at every
    /// candidate count above one.
    #[tokio::test]
    async fn the_scoring_statement_reaches_its_rows_by_id_at_every_candidate_count() {
        let store = MemoryStore::new_for_test().await.unwrap();
        for n in [1, 2, 4, 8, 20, MAX_LEXICAL_CANDIDATES] {
            let (memory_line, vector_line) = scoring_plan_lines_for(&store, n).await;
            assert!(
                memory_line.contains("INTEGER PRIMARY KEY"),
                "at {n} candidate ids the scoring query reaches `memory` by `{memory_line}` \
                 instead of by rowid, which makes its cost a function of how many rows the agent \
                 owns rather than of how many candidates were asked for"
            );
            assert!(
                vector_line.contains("INDEX 3:"),
                "at {n} candidate ids the scoring query reaches `memory_vectors` by \
                 `{vector_line}` instead of a per-id rowid lookup — `NOT INDEXED` alone does not \
                 catch this, since it only ever constrains `memory`'s own access path, not the \
                 vtab's"
            );
        }
    }

    /// The two dimensions that matter here are the candidate count and the
    /// size of the corpus around it, and they have to be crossed: a large
    /// corpus queried for ONE id hides the defect behind SQLite's
    /// `IN (?)` → `rowid = ?` rewrite, and a full candidate list over a small
    /// corpus hides it behind the corpus being small. Production crosses them
    /// every day — `SearchScope::all_default()` spends a budget of 20 against
    /// however many memories an agent has accumulated.
    ///
    /// The bound is expressed against this machine's own measurement of the
    /// same query doing the same work for a single candidate, so it constrains
    /// the SHAPE of the cost rather than the speed of the hardware it is
    /// measured on: scoring 20 candidates is 20 lookups, so it costs a small
    /// multiple of one, not a multiple in the hundreds. Measured on this
    /// machine, repeated runs of the shipped query land the 20-vs-1 ratio
    /// between 2x and 5x — noisy because the 1-candidate side is a single
    /// short-lived query where scheduler jitter dominates, but never close to
    /// double digits. 25x leaves 5-12x headroom over that observed range,
    /// which is enough to absorb a loaded CI machine without chasing a bound
    /// so tight it flakes on noise alone, while still failing hard on a query
    /// that is pricing itself by corpus size instead of candidate count: the
    /// two regressions this bound has ever needed to catch measured 37x and
    /// 287x. Both are also caught by
    /// `the_scoring_statement_reaches_its_rows_by_id_at_every_candidate_count`
    /// now that it reads the `memory_vectors` plan line — this bound is kept
    /// as a second, independent signal (a plan can be pinned wrong; a
    /// multi-millisecond query cannot look fast), not because either
    /// regression is otherwise invisible.
    #[tokio::test]
    async fn scoring_a_full_candidate_budget_costs_the_same_in_a_large_corpus_as_in_a_small_one() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let agent = "agent-crossed-dimensions";
        let corpus = 5_000;
        let ids = insert_rows_at_increasing_distance(&store, agent, "crossed row", corpus).await;
        let q_bytes = serialize_f32_query(&unit_basis_vec());
        let budget = 20;
        let requested: Vec<i64> = ids.iter().rev().take(budget).copied().collect();

        let time_it = |batch: Vec<i64>| {
            let store = &store;
            let q = q_bytes.clone();
            async move {
                let mut best = std::time::Duration::from_secs(3600);
                let mut hits = Vec::new();
                for _ in 0..3 {
                    let t = Instant::now();
                    hits = score_lexical_candidates(
                        store,
                        &q,
                        &only(agent),
                        None,
                        &batch,
                        &ScoringConfig::DEFAULT,
                        TimestampMs::now(), &HashSet::new(),
                    )
                    .await;
                    best = best.min(t.elapsed());
                }
                (best, hits)
            }
        };

        let (one, _) = time_it(vec![requested[0]]).await;
        let (many, hits) = time_it(requested.clone()).await;

        let mut returned: Vec<i64> = hits.iter().map(|h| h.id).collect();
        returned.sort_unstable();
        let mut expected = requested.clone();
        expected.sort_unstable();
        assert_eq!(returned, expected, "every requested candidate must still be scored");

        assert!(
            many < one * 25,
            "scoring {budget} candidates out of {corpus} rows took {many:?}, {:.0}x the {one:?} \
             this machine needs for one of them — the query is being priced by the size of the \
             corpus, not by the number of candidates it was handed",
            many.as_secs_f64() / one.as_secs_f64()
        );
    }

    // ── lexical channel wiring (4th try_join! arm inside search_all_memories_inner) ──

    /// Component-wise negation — cosine ≈ -1 after normalization, exactly
    /// antipodal to `v`. Mirrors `contradiction.rs`'s `far()` fixture (same
    /// negation construction), NOT this module's own `off_axis_vec()` (cosine
    /// 0.9): the identifier-promotion scenario needs a vector the dense
    /// channel unambiguously loses, not merely a weaker match.
    fn far_vec(v: &[f32]) -> Vec<f32> {
        v.iter().map(|x| -x).collect()
    }

    /// Seeds `count` memory rows whose distance to the `[1,0,0,…]` query
    /// vector strictly increases with insertion order, so "the row at global
    /// distance rank N" is a fact the fixture states rather than one the test
    /// has to discover. Returns the ids in that same order, so the LAST id is
    /// always the farthest row in the store.
    ///
    /// Every one of the 384 coordinates differs from the query vector, and
    /// that is load-bearing rather than decorative. A row differing in a
    /// SINGLE coordinate has, by elementary arithmetic, the same L1, L2 and
    /// L∞ distance to the query — one term, one absolute value, one square
    /// root of one square — so a fixture built that way cannot distinguish
    /// the metric under test from any other metric, and every distance
    /// assertion written against it holds no matter which one the query
    /// names. Spreading the offset over all 384 coordinates separates them by
    /// a factor of `sqrt(384)` ≈ 19.6, which no rounding can hide.
    async fn insert_rows_at_increasing_distance(
        store: &MemoryStore,
        agent_id: &str,
        content_prefix: &str,
        count: usize,
    ) -> Vec<i64> {
        let agent = agent_id.to_string();
        let prefix = content_prefix.to_string();
        store
            .conn
            .call(move |c| -> Result<Vec<i64>, rusqlite::Error> {
                let tx = c.transaction()?;
                let mut ids = Vec::with_capacity(count);
                for i in 0..count {
                    tx.execute(
                        "INSERT INTO memory (scope, agent_id, content, memory_type,
                         source, created_at, created_by_type, created_by_ref,
                         last_modified_at, last_accessed_at, importance, tier, status)
                         VALUES ('agent', ?1, ?2, 'chat_message', 'chat',
                         0, 'user', ?1, 0, 0, 5.0, 3, 'active')",
                        params![agent, format!("{prefix} {i}")],
                    )?;
                    let id = tx.last_insert_rowid();
                    // The difference from `[1,0,…]` is `t` in EVERY
                    // coordinate, with `t = (i+1) * 1e-5` and alternating
                    // signs so the offset is not a uniform shift either.
                    // L2 distance is therefore `t * sqrt(384)` and L1 is
                    // `t * 384`: both strictly increasing with insertion
                    // order, and 19.6x apart from each other. The L2 step
                    // between consecutive rows is ~1.96e-4, ~1600x f32's own
                    // resolution near 1.0, so no two rows collapse onto the
                    // same distance.
                    let t = (i as f32 + 1.0) * 1e-5;
                    let mut v = vec![t; 384];
                    v[0] = 1.0 - t;
                    for (j, slot) in v.iter_mut().enumerate().skip(1) {
                        if j % 2 == 1 {
                            *slot = -t;
                        }
                    }
                    tx.execute(
                        "INSERT INTO memory_vectors(rowid, embedding) VALUES (?, ?)",
                        params![id, serialize_embedding(&v)],
                    )?;
                    ids.push(id);
                }
                tx.commit()?;
                Ok(ids)
            })
            .await
            .unwrap()
    }

    /// The scoring query must reach a candidate regardless of where it sits in
    /// the store's global distance ordering. That is the entire reason this
    /// channel exists: an exact identifier match on a semantically distant row
    /// is, by definition, a row the distance ordering ranks badly. A scoring
    /// query that can only see the nearest N rows re-imposes the very ranking
    /// the lexical channel was added to bypass — silently, because the
    /// candidate id is simply absent from the result instead of erroring.
    ///
    /// The corpus is deliberately larger than vec0's own maximum `k` (4096):
    /// any implementation that scores candidates by running a k-nearest scan
    /// and filtering it has a ceiling somewhere, and this fixture puts the
    /// requested row above the highest ceiling such an implementation can
    /// have.
    #[tokio::test]
    async fn score_lexical_candidates_reaches_the_farthest_row_in_a_corpus_larger_than_vec0_max_k() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let agent = "agent-beyond-k-ceiling";
        let ids = insert_rows_at_increasing_distance(&store, agent, "beyond ceiling", VEC0_MAX_K + 1).await;
        let farthest = *ids.last().unwrap();

        let q_bytes = serialize_f32_query(&unit_basis_vec());
        let hits = score_lexical_candidates(
            &store,
            &q_bytes,
            &only(agent),
            None,
            &[farthest],
            &ScoringConfig::DEFAULT,
            TimestampMs::now(), &HashSet::new(),
        )
        .await;

        assert_eq!(
            hits.iter().map(|h| h.id).collect::<Vec<_>>(),
            vec![farthest],
            "the requested candidate is the farthest of {} rows; a scoring query that only \
             sees the nearest slice of the corpus drops exactly the rows this channel promotes",
            ids.len()
        );
    }

    /// The scoring query reports a distance the score formula consumes as if
    /// it came from the dense arms, so the two must be the SAME number, not
    /// merely two plausible ones. They agree only while the metric this query
    /// names matches the metric `memory_vectors` was declared with; declaring
    /// that table with a different `distance_metric`, or naming a different
    /// scalar in the scoring query, would leave both paths running without
    /// error and silently re-scale every lexically promoted row against the
    /// rest of the result set.
    ///
    /// The first assertion below is what gives that claim teeth. Bit-identity
    /// against the dense arm only constrains the metric if the fixture's rows
    /// can actually tell two metrics apart, and a fixture whose rows differ
    /// from the query in one coordinate cannot: L1, L2 and L∞ coincide there
    /// exactly, so a scoring query naming any of them passes. So this test
    /// first measures L1 against L2 on its own corpus and refuses to proceed
    /// unless they diverge — the fixture is under test before the query is.
    #[tokio::test]
    async fn lexical_scoring_distance_is_bit_identical_to_the_dense_knn_distance() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let agent = "agent-distance-parity";
        let ids = insert_rows_at_increasing_distance(&store, agent, "parity row", 40).await;
        let q_bytes = serialize_f32_query(&unit_basis_vec());

        let q_probe = q_bytes.clone();
        let separation: Vec<(f64, f64)> = store
            .conn
            .call(move |c| -> Result<Vec<(f64, f64)>, rusqlite::Error> {
                let mut stmt = c.prepare(
                    "SELECT vec_distance_l1(v.embedding, ?1), vec_distance_l2(v.embedding, ?1)
                     FROM memory_vectors v",
                )?;
                let rows = stmt.query_map(params![q_probe], |r| Ok((r.get(0)?, r.get(1)?)))?;
                rows.collect()
            })
            .await
            .unwrap();
        assert_eq!(separation.len(), ids.len(), "every seeded row must be probed");
        for (l1, l2) in &separation {
            assert!(
                l1 > &(l2 * 10.0),
                "this corpus cannot distinguish one metric from another (l1={l1}, l2={l2}), so \
                 the parity assertion below would hold for a scoring query naming any of them"
            );
        }

        let lex = score_lexical_candidates(
            &store,
            &q_bytes,
            &only(agent),
            None,
            &ids,
            &ScoringConfig::DEFAULT,
            TimestampMs::now(), &HashSet::new(),
        )
        .await;

        let q = q_bytes.clone();
        let knn: std::collections::HashMap<i64, f64> = store
            .conn
            .call(move |c| -> Result<std::collections::HashMap<i64, f64>, rusqlite::Error> {
                let mut stmt = c.prepare(
                    "SELECT knn.rowid, knn.distance FROM memory_vectors knn
                     WHERE knn.embedding MATCH ?1 AND knn.k = 40",
                )?;
                let rows = stmt.query_map(params![q], |r| Ok((r.get(0)?, r.get(1)?)))?;
                let mut out = std::collections::HashMap::new();
                for r in rows {
                    let (id, d) = r?;
                    out.insert(id, d);
                }
                Ok(out)
            })
            .await
            .unwrap();

        assert_eq!(lex.len(), ids.len(), "every seeded row must be scored");
        for h in &lex {
            let dense = knn.get(&h.id).copied().expect("every seeded row is inside the dense window");
            assert_eq!(
                h.distance.to_bits(),
                dense.to_bits(),
                "row {} scored at distance {} lexically but {} densely",
                h.id,
                h.distance,
                dense
            );
        }
    }


    // `memory` and `episodic_trajectory` each keep their own independent
    // autoincrement sequence, so their first-ever row both land on id 1 in a
    // fresh store — a coincidence, not a bug, and the dedup step must not
    // treat it as one. The episodic row (near, dense-visible) and M (far,
    // lexical-only) share that same numeric id but different `TableKind`s;
    // differencing lexical ids against the MERGED pool would see id 1
    // "already found" via the episodic hit and wrongly drop M.
    #[tokio::test]
    async fn search_all_memories_inner_dedup_ignores_cross_table_id_collisions() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let near = unit_basis_vec();
        let far = far_vec(&near);
        let agent = "agent-id-collision";

        let epi_id = insert_episodic_row_for_knn(&store, agent, "collision episode", near.clone()).await;
        let m_id = insert_memory_row(
            &store,
            agent,
            "collision_unique_tok content",
            "chat_message",
            3,
            5.0,
            "active",
            serialize_embedding(&far),
        )
        .await;
        assert_eq!(epi_id, m_id, "test setup: the two independent autoincrement sequences must actually collide");

        bulk_insert_memory_rows(&store, agent, None, 20, serialize_embedding(&near)).await;

        let scope = SearchScope {
            memory: Some(20),
            episodic: Some(5),
            procedural: None,
            project_id: None,
            lexical: Some(20),
            reach: Reach::Own,
        };
        let (hits, _telemetry, _) = search_all_memories_inner(
            &store,
            &near,
            Some("collision_unique_tok"),
            agent,
            scope,
            30,
            &ScoringConfig::DEFAULT,
        )
        .await
        .unwrap();

        assert!(
            hits.iter().any(|h| h.id == m_id && h.kind == TableKind::Memory),
            "M must survive dedup even though its numeric id collides with an unrelated episodic hit"
        );
    }

    // The scenario the fail-soft contract exists for, exercised through the
    // real wired call site rather than `lexical_candidates` alone: a missing
    // `memory_fts` table must still leave `search_all_memories_inner`
    // returning `Ok` with exactly the dense-only result.
    #[tokio::test]
    async fn search_all_memories_inner_fails_soft_when_memory_fts_is_missing() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let near = unit_basis_vec();
        let agent = "agent-failsoft-wired";
        bulk_insert_memory_rows(&store, agent, None, 10, serialize_embedding(&near)).await;

        store.conn.call(|c| c.execute("DROP TABLE memory_fts", [])).await.unwrap();

        let scope_on = SearchScope { lexical: Some(20), ..SearchScope::all_default() };
        let (hits_on, _t_on, _) = search_all_memories_inner(
            &store, &near, Some("bulk"), agent, scope_on, 30, &ScoringConfig::DEFAULT,
        )
        .await
        .unwrap();

        let scope_off = SearchScope { lexical: None, ..SearchScope::all_default() };
        let (hits_off, _t_off, _) = search_all_memories_inner(
            &store, &near, Some("bulk"), agent, scope_off, 30, &ScoringConfig::DEFAULT,
        )
        .await
        .unwrap();

        let ids_on: Vec<i64> = hits_on.iter().map(|h| h.id).collect();
        let ids_off: Vec<i64> = hits_off.iter().map(|h| h.id).collect();
        assert_eq!(
            ids_on, ids_off,
            "a missing memory_fts table must degrade to exactly the dense-only result, never Err"
        );
    }

    // `lex_only` answers "how many rows did this channel add that the dense
    // one had never seen", so the only fixture that can tell it apart from
    // "how many candidates did this channel propose" is one where those two
    // numbers DIFFER — a lexical candidate the dense arm had already found.
    // Every other fixture in this module has the dense window either
    // containing all the lexical candidates or none of them, and on those the
    // pre-dedup and post-dedup counts are the same number, so the dedup step
    // could be deleted from the attribution entirely and nothing would say so.
    //
    // Both rows carry the shared token, so the channel proposes 2. The dense
    // window is deliberately one row wide and A is the only row near the query,
    // so the dense arm finds A and misses B. 2 proposed, 1 new: the two
    // readings of `lex_only` are 2 and 1 here, and only one of them is the
    // number this telemetry claims to report.
    #[tokio::test]
    async fn lexical_only_counts_what_the_dense_channel_had_not_already_found() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let near = unit_basis_vec();
        let far = far_vec(&near);
        let agent = "agent-dedup-attribution";

        let a_id = insert_memory_row(
            &store, agent, "alpha carrying dedupattr_tok", "chat_message", 3, 5.0, "active",
            serialize_embedding(&near),
        )
        .await;
        let b_id = insert_memory_row(
            &store, agent, "beta carrying dedupattr_tok", "chat_message", 3, 5.0, "active",
            serialize_embedding(&far),
        )
        .await;

        // Asserted, not assumed: if the token ever stopped matching both rows
        // the test below would still pass while measuring nothing.
        let proposed = lexical_candidates(&store, "dedupattr_tok", &only(agent), None, 20).await;
        assert_eq!(
            proposed.len(),
            2,
            "the fixture only distinguishes the two counts while the channel proposes both rows"
        );

        let scope = SearchScope {
            memory: Some(1),
            episodic: None,
            procedural: None,
            project_id: None,
            lexical: Some(20),
            reach: Reach::Own,
        };
        let (hits, telemetry, _) = search_all_memories_inner(
            &store,
            &near,
            Some("dedupattr_tok"),
            agent,
            scope,
            25,
            &ScoringConfig::DEFAULT,
        )
        .await
        .unwrap();

        assert!(
            hits.iter().any(|h| h.id == a_id) && hits.iter().any(|h| h.id == b_id),
            "both rows must reach the caller — A densely, B lexically"
        );
        assert!(
            telemetry.contains("lex_only=1"),
            "the channel proposed 2 candidates but only B was new to the dense channel, so the \
             attribution is 1; reporting 2 would credit this channel with a row it did not add: \
             {telemetry}"
        );
    }

    // The other half of the fail-soft contract, and the harder half: the
    // candidate query failing is only the first way this channel can break.
    // Once candidates exist, the SCORING query runs too, and it touches a
    // different table — so a store where `memory_fts` is perfectly healthy and
    // `memory_vectors` is not drives the channel all the way to hop 5 and then
    // fails there. Every scope arm except the lexical one is off here on
    // purpose: with the dense arms on, they would hit the same broken table
    // first and this assertion would be about them instead.
    //
    // What must survive is the contract, not the result: a channel that exists
    // to ADD candidates has nothing to offer a search it makes fail, so the
    // worst it may ever do is contribute nothing.
    #[tokio::test]
    async fn search_all_memories_inner_fails_soft_when_the_scoring_query_cannot_run() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let near = unit_basis_vec();
        let agent = "agent-failsoft-scoring";
        insert_memory_row(
            &store,
            agent,
            "row carrying scoringfailsoft_tok",
            "chat_message",
            3,
            5.0,
            "active",
            serialize_embedding(&near),
        )
        .await;

        // Dropped after the row is indexed, so `memory_fts` still promotes the
        // candidate and the scoring query is genuinely reached.
        store.conn.call(|c| c.execute("DROP TABLE memory_vectors", [])).await.unwrap();

        let scope = SearchScope {
            memory: None,
            episodic: None,
            procedural: None,
            lexical: Some(20),
            ..SearchScope::all_default()
        };
        let result = search_all_memories_inner(
            &store,
            &near,
            Some("scoringfailsoft_tok"),
            agent,
            scope,
            30,
            &ScoringConfig::DEFAULT,
        )
        .await;

        let (hits, _telemetry, _) = result.expect(
            "a scoring query that cannot run means the lexical channel contributed nothing — \
             never that the whole search failed",
        );
        assert!(hits.is_empty(), "nothing was scorable, so nothing may be returned");
    }

    // 20 near rows fill the dense window (scope.memory = 20) so M — whose
    // vector is antipodal to the query — cannot survive on distance alone.
    // Only the lexical channel, matching M's unique identifier token, can
    // still surface it. The `lexical: None` control call proves the dense
    // channel really does miss it on its own, isolating what this test
    // claims to demonstrate.
    #[tokio::test]
    async fn search_all_memories_inner_lexical_channel_surfaces_identifier_dense_misses() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let near = unit_basis_vec();
        let far = far_vec(&near);
        let agent = "agent-identifier-promotion";

        bulk_insert_memory_rows(&store, agent, None, 20, serialize_embedding(&near)).await;
        let m_id = insert_memory_row(
            &store,
            agent,
            "contains GPX-Store-7f3a as its unique identifier",
            "chat_message",
            3,
            5.0,
            "active",
            serialize_embedding(&far),
        )
        .await;

        let scope = SearchScope {
            memory: Some(20),
            episodic: None,
            procedural: None,
            project_id: None,
            lexical: Some(20),
            reach: Reach::Own,
        };

        let (hits, telemetry, _) = search_all_memories_inner(
            &store,
            &near,
            Some("GPX-Store-7f3a"),
            agent,
            scope.clone(),
            25,
            &ScoringConfig::DEFAULT,
        )
        .await
        .unwrap();
        assert!(
            hits.iter().any(|h| h.id == m_id),
            "the lexical channel must surface M even though it is semantically antipodal to the query"
        );
        assert!(
            telemetry.contains("lex_only=1"),
            "exactly one candidate (M) was promoted by the lexical channel and not seen by the dense one: {telemetry}"
        );

        let scope_off = SearchScope { lexical: None, ..scope };
        let (hits_off, telemetry_off, _) = search_all_memories_inner(
            &store,
            &near,
            Some("GPX-Store-7f3a"),
            agent,
            scope_off,
            25,
            &ScoringConfig::DEFAULT,
        )
        .await
        .unwrap();
        assert!(
            !hits_off.iter().any(|h| h.id == m_id),
            "control: with the channel off, dense distance alone must not find M"
        );
        assert!(
            telemetry_off.contains("lex_only=0"),
            "the channel never ran, so it cannot have promoted anything: {telemetry_off}"
        );
    }

    // Zero lexical overlap must be a strict no-op: the same ids, in the same
    // order, with bit-exact scores, whether the channel is on or off. This is
    // the assertion that matters once a lexical candidate can legitimately
    // displace a dense hit — it proves the 4th arm never changes a result it
    // has nothing new to contribute to.
    #[tokio::test]
    async fn search_all_memories_inner_no_lexical_overlap_is_byte_identical_to_channel_off() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let near = unit_basis_vec();
        let agent = "agent-no-overlap";
        bulk_insert_memory_rows(&store, agent, None, 10, serialize_embedding(&near)).await;

        let scope_on = SearchScope::all_default();
        let (hits_on, telemetry_on, _) = search_all_memories_inner(
            &store,
            &near,
            Some("zzz_token_with_no_overlap_in_the_corpus"),
            agent,
            scope_on,
            30,
            &ScoringConfig::DEFAULT,
        )
        .await
        .unwrap();
        assert!(
            telemetry_on.contains("lex_only=0"),
            "the channel ran (it's on) but found nothing new: {telemetry_on}"
        );

        let scope_off = SearchScope { lexical: None, ..SearchScope::all_default() };
        let (hits_off, _t_off, _) = search_all_memories_inner(
            &store,
            &near,
            Some("zzz_token_with_no_overlap_in_the_corpus"),
            agent,
            scope_off,
            30,
            &ScoringConfig::DEFAULT,
        )
        .await
        .unwrap();

        let ids_on: Vec<i64> = hits_on.iter().map(|h| h.id).collect();
        let ids_off: Vec<i64> = hits_off.iter().map(|h| h.id).collect();
        assert_eq!(ids_on, ids_off, "same ids, same order, with or without the channel");
        for (a, b) in hits_on.iter().zip(hits_off.iter()) {
            assert_eq!(a.score, b.score, "scores must be bit-exact across both runs");
        }
    }

    // `scope.lexical = None` must skip the channel entirely, not merely find
    // nothing. Setup deliberately leaves a row (M) that the lexical channel
    // WOULD surface if it ran at all — matching token, antipodal vector, same
    // shape as the identifier-promotion fixture above — so a mutation that
    // removes the `None`-skips branch is caught by the first assertion alone,
    // regardless of what the table happens to contain.
    #[tokio::test]
    async fn search_all_memories_inner_lexical_none_skips_the_channel_entirely() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let near = unit_basis_vec();
        let far = far_vec(&near);
        let agent = "agent-kill-switch";

        bulk_insert_memory_rows(&store, agent, None, 20, serialize_embedding(&near)).await;
        let m_id = insert_memory_row(
            &store,
            agent,
            "contains kill_switch_unique_token as its identifier",
            "chat_message",
            3,
            5.0,
            "active",
            serialize_embedding(&far),
        )
        .await;

        let scope = SearchScope {
            memory: Some(20),
            episodic: None,
            procedural: None,
            project_id: None,
            lexical: None,
            reach: Reach::Own,
        };

        let (hits_present, _t1, _) = search_all_memories_inner(
            &store,
            &near,
            Some("kill_switch_unique_token"),
            agent,
            scope.clone(),
            25,
            &ScoringConfig::DEFAULT,
        )
        .await
        .unwrap();
        assert!(
            !hits_present.iter().any(|h| h.id == m_id),
            "lexical: None must not surface M even though memory_fts holds a matching row — \
             the channel must be entirely skipped, not merely find nothing"
        );

        store.conn.call(|c| c.execute("DROP TABLE memory_fts", [])).await.unwrap();
        let (hits_dropped, _t2, _) = search_all_memories_inner(
            &store,
            &near,
            Some("kill_switch_unique_token"),
            agent,
            scope,
            25,
            &ScoringConfig::DEFAULT,
        )
        .await
        .unwrap();

        let ids_present: Vec<i64> = hits_present.iter().map(|h| h.id).collect();
        let ids_dropped: Vec<i64> = hits_dropped.iter().map(|h| h.id).collect();
        assert_eq!(
            ids_present, ids_dropped,
            "table-present + lexical:None must behave identically to table-dropped + lexical:None — \
             proving the table is never touched when the channel is off"
        );
    }

    #[tokio::test]
    async fn soft_delete_removes_the_row_from_memory_fts() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let emb = unit_basis_embedding();
        let agent = "agent-trigger-removal";
        insert_memory_row(
            &store, agent, "unique_trigger_removal_token content", "chat_message", 3, 5.0, "active", emb,
        )
        .await;

        let ids_before = lexical_candidates(&store, "unique_trigger_removal_token", &only(agent), None, 20).await;
        assert_eq!(ids_before.len(), 1, "test setup: the row must be lexically reachable before deletion");
        let id = ids_before[0];

        crate::memory::admin::delete_memory_core(
            &store,
            id,
            crate::memory::admin::MemoryKind::Memory,
            None,
        )
        .await
        .unwrap();

        let count: i64 = store
            .conn
            .call(|c| {
                c.query_row(
                    "SELECT COUNT(*) FROM memory_fts WHERE memory_fts MATCH '\"unique_trigger_removal_token\"'",
                    [],
                    |row| row.get(0),
                )
            })
            .await
            .unwrap();
        assert_eq!(
            count, 0,
            "the AFTER UPDATE OF deleted_at trigger must remove the row from memory_fts on soft delete"
        );
    }

    // Soft delete, then hard delete the same row: the AFTER DELETE trigger's
    // `WHEN old.deleted_at IS NULL` guard must keep the hard delete from
    // subtracting postings that the soft-delete trigger already subtracted.
    // Proven indirectly, through an unrelated survivor row: a corrupted
    // external-content index would either error on the next MATCH statement
    // (surfacing as a panicking `.unwrap()` below) or lose track of rows it
    // still holds.
    #[tokio::test]
    async fn soft_then_hard_delete_does_not_double_subtract_from_memory_fts() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let emb = unit_basis_embedding();
        let agent = "agent-trigger-balance";
        let id = insert_memory_row(
            &store, agent, "balance_token content", "chat_message", 3, 5.0, "active", emb.clone(),
        )
        .await;
        let survivor = insert_memory_row(
            &store, agent, "survivor_token content", "chat_message", 3, 5.0, "active", emb,
        )
        .await;

        crate::memory::admin::delete_memory_core(
            &store,
            id,
            crate::memory::admin::MemoryKind::Memory,
            None,
        )
        .await
        .unwrap();
        store
            .conn
            .call(move |c| c.execute("DELETE FROM memory WHERE id = ?1", params![id]))
            .await
            .unwrap();

        let ids = lexical_candidates(&store, "survivor_token", &only(agent), None, 20).await;
        assert!(
            ids.contains(&survivor),
            "the index must remain intact for an unrelated row after a soft-then-hard delete sequence"
        );
    }

    // The end-to-end pairing for the identifier-promotion fixture: the pure
    // formatter, called directly with the same counts the wired call site
    // computed, must render the identical substring the real telemetry line
    // carries.
    #[tokio::test]
    async fn telemetry_lex_only_matches_format_channel_attribution_called_directly() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let near = unit_basis_vec();
        let far = far_vec(&near);
        let agent = "agent-attribution-parity";

        bulk_insert_memory_rows(&store, agent, None, 20, serialize_embedding(&near)).await;
        insert_memory_row(
            &store,
            agent,
            "contains attribution_parity_token as its identifier",
            "chat_message",
            3,
            5.0,
            "active",
            serialize_embedding(&far),
        )
        .await;

        let scope = SearchScope {
            memory: Some(20),
            episodic: None,
            procedural: None,
            project_id: None,
            lexical: Some(20),
            reach: Reach::Own,
        };
        let (_hits, telemetry, _) = search_all_memories_inner(
            &store,
            &near,
            Some("attribution_parity_token"),
            agent,
            scope,
            25,
            &ScoringConfig::DEFAULT,
        )
        .await
        .unwrap();

        let expected = crate::memory::admin::format_channel_attribution(20, 1);
        assert!(
            telemetry.contains(&expected),
            "the wired telemetry line must contain exactly what the formatter renders for (dense=20, lex_only=1): {telemetry}"
        );
    }

    // ── exact identifiers ──

    #[test]
    fn identifier_tokens_keep_mixed_snake_and_camel_tokens_in_order() {
        let tokens = identifier_tokens("qué hace getEstadoLabel con E_AUTH_4821 y getUser4821 en 2026");
        assert_eq!(tokens, vec!["getEstadoLabel", "E_AUTH_4821", "getUser4821"]);
    }

    #[test]
    fn identifier_tokens_skip_bare_numbers_and_plain_words() {
        assert!(identifier_tokens("how do we deploy the importer").is_empty());
        assert!(identifier_tokens("what changed in 2026 on page 3").is_empty());
    }

    #[test]
    fn identifier_tokens_read_the_whole_query() {
        let query = "the a of to in is it on at by be as or an we do so if up no parseLedger7731";
        assert!(fts_tokens(query).count() > MAX_LEXICAL_TOKENS);
        assert_eq!(identifier_tokens(query), vec!["parseLedger7731"]);
    }

    #[test]
    fn identifier_tokens_are_distinct_and_capped() {
        let query = (0..12).map(|i| format!("id{i} id{i}")).collect::<Vec<_>>().join(" ");
        let tokens = identifier_tokens(&query);
        assert_eq!(tokens.len(), MAX_IDENTIFIER_TOKENS);
        assert_eq!(tokens[0], "id0");
        assert_eq!(tokens[1], "id1");
    }

    fn without_bonus() -> ScoringConfig {
        ScoringConfig { lexical_bonus: 0.0, ..ScoringConfig::DEFAULT }
    }

    /// Seven siblings that read like the query and carry identifiers a few
    /// digits away from the queried one, plus the one row that holds it but
    /// sits farther from the query vector. Returns that row's id.
    async fn seed_identifier_family(store: &MemoryStore, agent: &str) -> i64 {
        for digits in ["4812", "4821", "4128", "4281", "4813", "4811", "4218"] {
            insert_memory_row(
                store,
                agent,
                &format!("Helper `resolveLedger{digits}` retries the request twice."),
                "code_block",
                3,
                5.0,
                "active",
                serialize_embedding(&vec_at_cosine(0.97)),
            )
            .await;
        }
        insert_memory_row(
            store,
            agent,
            "The cleanup branch kept `resolveLedger4182` because the report still imports it.",
            "code_block",
            3,
            5.0,
            "active",
            serialize_embedding(&vec_at_cosine(0.92)),
        )
        .await
    }

    async fn search_with(
        store: &MemoryStore,
        query: Option<&str>,
        agent: &str,
        scope: SearchScope,
        scoring: &ScoringConfig,
    ) -> Vec<RankedHit> {
        search_all_memories_inner(store, &unit_basis_vec(), query, agent, scope, 10, scoring)
            .await
            .unwrap()
            .0
    }

    fn scores(hits: &[RankedHit]) -> Vec<(i64, u64)> {
        hits.iter().map(|h| (h.id, h.score.to_bits())).collect()
    }

    #[tokio::test]
    async fn an_exact_identifier_outranks_its_closer_siblings() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let agent = "agent-identifier-family";
        let expected = seed_identifier_family(&store, agent).await;
        let query = Some("where is resolveLedger4182 used");

        let control = search_with(&store, query, agent, SearchScope::all_default(), &without_bonus()).await;
        assert_ne!(control[0].id, expected, "without the bonus the closer siblings must win");

        let hits = search_with(&store, query, agent, SearchScope::all_default(), &ScoringConfig::DEFAULT).await;
        assert_eq!(hits[0].id, expected, "the row holding the queried identifier must rank first");
    }

    #[tokio::test]
    async fn a_row_holding_the_query_identifier_comes_back_exact() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let agent = "agent-identifier-exact";
        let expected = seed_identifier_family(&store, agent).await;

        let query = Some("where is resolveLedger4182 used");
        let hits = search_with(&store, query, agent, SearchScope::all_default(), &ScoringConfig::DEFAULT).await;
        let exact: Vec<i64> = hits.iter().filter(|h| h.exact).map(|h| h.id).collect();
        assert_eq!(exact, vec![expected]);

        let query = Some("which helper retries the request twice");
        let natural = search_with(&store, query, agent, SearchScope::all_default(), &ScoringConfig::DEFAULT).await;
        assert!(natural.iter().all(|h| !h.exact), "a query without identifiers marks nothing");
    }

    /// The row the dense pool misses reaches the results through the lexical
    /// candidates, which build their hits apart from the dense ones.
    #[tokio::test]
    async fn a_lexical_candidate_holding_the_identifier_comes_back_exact() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let near = unit_basis_vec();
        let agent = "agent-lexical-exact";
        bulk_insert_memory_rows(&store, agent, None, 20, serialize_embedding(&near)).await;
        let far = insert_memory_row(
            &store, agent, "kept parseLedger7731 on the old path", "chat_message", 3, 5.0, "active",
            serialize_embedding(&far_vec(&near)),
        )
        .await;

        let scope = SearchScope { memory: Some(20), episodic: None, procedural: None, ..SearchScope::all_default() };
        let query = Some("parseLedger7731");
        let hits = search_all_memories_inner(&store, &near, query, agent, scope, 25, &ScoringConfig::DEFAULT)
            .await
            .unwrap()
            .0;

        let exact: Vec<i64> = hits.iter().filter(|h| h.exact).map(|h| h.id).collect();
        assert_eq!(exact, vec![far]);
    }

    #[tokio::test]
    async fn an_identifier_past_the_sixteenth_token_still_reaches_its_row() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let near = unit_basis_vec();
        let agent = "agent-late-identifier";
        bulk_insert_memory_rows(&store, agent, None, 20, serialize_embedding(&near)).await;
        let m_id = insert_memory_row(
            &store,
            agent,
            "kept parseLedger7731 on the old path",
            "chat_message",
            3,
            5.0,
            "active",
            serialize_embedding(&far_vec(&near)),
        )
        .await;
        let query = "the a of to in is it on at by be as or an we do so if up no parseLedger7731";
        assert!(fts_tokens(query).count() > MAX_LEXICAL_TOKENS);

        let scope = SearchScope { memory: Some(20), episodic: None, procedural: None, ..SearchScope::all_default() };
        let hits = search_all_memories_inner(&store, &near, Some(query), agent, scope, 25, &ScoringConfig::DEFAULT)
            .await
            .unwrap()
            .0;

        let m = hits.iter().find(|h| h.id == m_id).expect("the identifier's row must be a candidate");
        assert!(m.score > 0.0, "a far row holding the identifier must score above zero, got {}", m.score);
    }

    #[tokio::test]
    async fn no_bonus_without_the_lexical_channel_or_query_text() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let agent = "agent-identifier-off";
        seed_identifier_family(&store, agent).await;
        let query = Some("where is resolveLedger4182 used");
        let off = SearchScope { lexical: None, ..SearchScope::all_default() };

        let with = search_with(&store, query, agent, off.clone(), &ScoringConfig::DEFAULT).await;
        let without = search_with(&store, query, agent, off, &without_bonus()).await;
        assert_eq!(scores(&with), scores(&without), "channel off");

        let with = search_with(&store, None, agent, SearchScope::all_default(), &ScoringConfig::DEFAULT).await;
        let without = search_with(&store, None, agent, SearchScope::all_default(), &without_bonus()).await;
        assert_eq!(scores(&with), scores(&without), "no query text");
    }

    async fn seed_shared_identifier(store: &MemoryStore, agent: &str, count: usize) {
        for i in 0..count {
            insert_memory_row(
                store,
                agent,
                &format!("note {i} about getUser42"),
                "chat_message",
                3,
                5.0,
                "active",
                serialize_embedding(&vec_at_cosine(0.95)),
            )
            .await;
        }
    }

    #[tokio::test]
    async fn an_identifier_shared_by_more_rows_than_the_budget_gives_no_bonus() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let agent = "agent-shared-identifier";
        let scope = SearchScope { lexical: Some(20), ..SearchScope::all_default() };

        seed_shared_identifier(&store, agent, 20).await;
        let with = search_with(&store, Some("getUser42"), agent, scope.clone(), &ScoringConfig::DEFAULT).await;
        let without = search_with(&store, Some("getUser42"), agent, scope.clone(), &without_bonus()).await;
        assert_ne!(scores(&with), scores(&without), "twenty rows still fit the budget and get the bonus");

        seed_shared_identifier(&store, agent, 1).await;
        let with = search_with(&store, Some("getUser42"), agent, scope.clone(), &ScoringConfig::DEFAULT).await;
        let without = search_with(&store, Some("getUser42"), agent, scope, &without_bonus()).await;
        assert_eq!(scores(&with), scores(&without), "twenty-one rows do not single anything out");
    }

    #[tokio::test]
    async fn a_query_without_identifiers_scores_exactly_as_without_the_bonus() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let agent = "agent-natural-query";
        seed_identifier_family(&store, agent).await;
        let query = Some("which helper retries the request twice");

        let with = search_with(&store, query, agent, SearchScope::all_default(), &ScoringConfig::DEFAULT).await;
        let without = search_with(&store, query, agent, SearchScope::all_default(), &without_bonus()).await;
        assert!(!with.is_empty());
        assert_eq!(scores(&with), scores(&without));
    }

    #[tokio::test]
    async fn rows_the_caller_cannot_see_neither_match_nor_count_against_the_budget() {
        let store = MemoryStore::new_for_test().await.unwrap();
        seed_shared_identifier(&store, "agent-other", 21).await;
        let deleted = insert_memory_row(
            &store, "agent-own", "deleted getUser42", "chat_message", 3, 5.0, "active",
            serialize_embedding(&vec_at_cosine(0.95)),
        )
        .await;
        store
            .conn
            .call(move |c| c.execute("UPDATE memory SET status = 'superseded' WHERE id = ?1", params![deleted]))
            .await
            .unwrap();
        let own = insert_memory_row(
            &store, "agent-own", "own note about getUser42", "chat_message", 3, 5.0, "active",
            serialize_embedding(&vec_at_cosine(0.95)),
        )
        .await;
        let scope = SearchScope { lexical: Some(1), ..SearchScope::all_default() };

        let with = search_with(&store, Some("getUser42"), "agent-own", scope.clone(), &ScoringConfig::DEFAULT).await;
        let without = search_with(&store, Some("getUser42"), "agent-own", scope, &without_bonus()).await;
        let score_of = |hits: &[RankedHit]| hits.iter().find(|h| h.id == own).map(|h| h.score).unwrap();
        assert!(score_of(&with) > score_of(&without), "the caller's only visible match gets the bonus");
    }

    #[test]
    fn an_underscore_identifier_is_held_only_as_written() {
        assert!(holds_identifier("we keep `user_id` in the row", "user_id"));
        assert!(holds_identifier("USER_ID is the key", "user_id"));
        assert!(!holds_identifier("the user id is the key", "user_id"));
        assert!(!holds_identifier("the user-id is the key", "user_id"));
        assert!(!holds_identifier("the old_user_id stays", "user_id"));
        assert!(!holds_identifier("the user_id2 stays", "user_id"));
    }

    #[tokio::test]
    async fn words_split_like_an_underscore_identifier_neither_match_nor_use_the_budget() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let agent = "agent-snake-case";
        for i in 0..25 {
            let split = if i % 2 == 0 { "snake case" } else { "snake-case" };
            let content = format!("note {i} in {split}");
            insert_memory_row(
                &store, agent, &content, "chat_message", 3, 5.0, "active",
                serialize_embedding(&vec_at_cosine(0.95)),
            )
            .await;
        }
        let own = insert_memory_row(
            &store, agent, "rename it to snake_case", "chat_message", 3, 5.0, "active",
            serialize_embedding(&vec_at_cosine(0.95)),
        )
        .await;

        let matches = identifier_matches(&store, "snake_case", &only(agent), None, 20).await;
        assert_eq!(matches, HashSet::from([own]));
    }

    #[tokio::test]
    async fn a_failing_identifier_query_leaves_the_search_unboosted() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let agent = "agent-identifier-failsoft";
        seed_identifier_family(&store, agent).await;
        store.conn.call(|c| c.execute("DROP TABLE memory_fts", [])).await.unwrap();
        let query = Some("where is resolveLedger4182 used");

        let with = search_with(&store, query, agent, SearchScope::all_default(), &ScoringConfig::DEFAULT).await;
        let without = search_with(&store, query, agent, SearchScope::all_default(), &without_bonus()).await;
        assert!(!with.is_empty());
        assert_eq!(scores(&with), scores(&without));
    }

    // ── sharing between agents ──

    fn axis(i: usize) -> Vec<f32> {
        let mut v = vec![0.0_f32; 384];
        v[i] = 1.0;
        v
    }

    fn dense_only() -> SearchScope {
        SearchScope { lexical: None, ..SearchScope::all_default() }
    }

    fn lexical_only() -> SearchScope {
        SearchScope { memory: None, episodic: None, procedural: None, ..SearchScope::all_default() }
    }

    fn ids_of(hits: &[RankedHit]) -> Vec<i64> {
        hits.iter().map(|h| h.id).collect()
    }

    fn everyone() -> Reach {
        Reach::AllBut(Vec::new())
    }

    #[tokio::test]
    async fn another_agents_row_is_found_in_every_channel() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let theirs =
            insert_memory_row(&store, "a", "el deploy usa flyio_4821", "semantic_fact", 3, 5.0, "active", serialize_embedding(&axis(0)))
                .await;
        let query = "flyio_4821";

        for (channel, scope) in [("dense", dense_only()), ("lexical", lexical_only())] {
            let alone = search_all_memories(&store, &axis(0), Some(query), "b", scope.clone(), 10).await.unwrap();
            assert!(!ids_of(&alone).contains(&theirs), "{channel}: reaching only its own, b must not see a's row");

            let shared = search_all_memories(&store, &axis(0), Some(query), "b", scope.with_reach(everyone()), 10).await.unwrap();
            assert!(ids_of(&shared).contains(&theirs), "{channel}: reaching every agent, b must find a's row");
        }
    }

    /// A `global` row: the schema's shape for a `memory` row with neither an
    /// author nor a project.
    async fn insert_global_row(store: &MemoryStore, content: &str, embedding: Vec<u8>) -> i64 {
        let content = content.to_string();
        store
            .conn
            .call(move |c| -> Result<i64, rusqlite::Error> {
                let tx = c.transaction()?;
                tx.execute(
                    "INSERT INTO memory (scope, agent_id, content, memory_type, source,
                     created_at, created_by_type, created_by_ref,
                     last_modified_at, last_accessed_at, importance, tier, status)
                     VALUES ('global', NULL, ?1, 'semantic_fact', 'chat', 0, 'user', 'import', 0, 0, 5.0, 3, 'active')",
                    params![content],
                )?;
                let id = tx.last_insert_rowid();
                tx.execute("INSERT INTO memory_vectors(rowid, embedding) VALUES (?, ?)", params![id, embedding])?;
                tx.commit()?;
                Ok(id)
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn a_row_no_agent_saved_is_found_whatever_the_policy() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let global = insert_global_row(&store, "el deploy usa flyio_4821", serialize_embedding(&axis(0))).await;

        for (channel, scope) in [("dense", dense_only()), ("lexical", lexical_only())] {
            for reach in [Reach::Own, everyone(), Reach::AllBut(vec!["a".to_string()])] {
                let hits =
                    search_all_memories(&store, &axis(0), Some("flyio_4821"), "b", scope.clone().with_reach(reach.clone()), 10).await.unwrap();
                assert!(ids_of(&hits).contains(&global), "{channel}, {reach:?}: no exclusion names a row without an author");
            }
        }
    }

    #[tokio::test]
    async fn a_hidden_author_stays_out_in_every_channel() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let theirs =
            insert_memory_row(&store, "a", "el deploy usa flyio_4821", "semantic_fact", 3, 5.0, "active", serialize_embedding(&axis(0)))
                .await;
        let other =
            insert_memory_row(&store, "c", "el deploy usa flyio_4821 también", "semantic_fact", 3, 5.0, "active", serialize_embedding(&axis(0)))
                .await;

        for (channel, scope) in [("dense", dense_only()), ("lexical", lexical_only())] {
            let reach = Reach::AllBut(vec!["a".to_string()]);
            let hits = ids_of(&search_all_memories(&store, &axis(0), Some("flyio_4821"), "b", scope.with_reach(reach), 10).await.unwrap());
            assert!(!hits.contains(&theirs), "{channel}: a is hidden from b");
            assert!(hits.contains(&other), "{channel}: c is not");
        }
    }

    #[tokio::test]
    async fn an_exclusion_does_not_work_backwards() {
        use crate::memory::sharing::SharingPolicy;
        let store = MemoryStore::new_for_test().await.unwrap();
        let mine = insert_memory_row(&store, "a", "fila de a", "semantic_fact", 3, 5.0, "active", serialize_embedding(&axis(0))).await;
        let yours = insert_memory_row(&store, "b", "fila de b", "semantic_fact", 3, 5.0, "active", serialize_embedding(&axis(0))).await;
        let policy = SharingPolicy::parse(r#"{"version":1,"hides":{"b":["a"]}}"#).unwrap();

        let for_b = SearchScope::all_default().with_reach(policy.reach_for("b"));
        let hits = ids_of(&search_all_memories(&store, &axis(0), None, "b", for_b, 10).await.unwrap());
        assert_eq!(hits, [yours], "b does not read a");

        let for_a = SearchScope::all_default().with_reach(policy.reach_for("a"));
        let mut hits = ids_of(&search_all_memories(&store, &axis(0), None, "a", for_a, 10).await.unwrap());
        hits.sort_unstable();
        assert_eq!(hits, [mine, yours], "a still reads b");
    }

    #[tokio::test]
    async fn an_agent_never_hides_its_own_rows() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let mine = insert_memory_row(&store, "b", "propia", "semantic_fact", 3, 5.0, "active", serialize_embedding(&axis(0))).await;
        let scope = SearchScope::all_default().with_reach(Reach::AllBut(vec!["b".to_string()]));
        let hits = ids_of(&search_all_memories(&store, &axis(0), None, "b", scope, 10).await.unwrap());
        assert_eq!(hits, [mine]);
    }

    #[tokio::test]
    async fn shared_rows_obey_the_project_filter() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let emb = serialize_embedding(&axis(0));
        insert_memory_row_with_project(&store, "a", Some("P1"), "semantic_fact", "dato de P1", emb.clone()).await;
        let pref = insert_memory_row_with_project(&store, "a", Some("P1"), "user_preference", "prefiere tabs", emb.clone()).await;
        let unscoped = insert_memory_row_with_project(&store, "a", None, "semantic_fact", "dato sin proyecto", emb).await;

        let scope = SearchScope::all_default().with_project(Some("P2")).with_reach(everyone());
        let mut hits = ids_of(&search_all_memories(&store, &axis(0), None, "b", scope, 10).await.unwrap());
        hits.sort_unstable();
        assert_eq!(hits, [pref, unscoped], "a's P1 fact stays out of P2; its preference and unscoped row come through");
    }

    #[tokio::test]
    async fn the_eligible_count_follows_the_reach() {
        let store = MemoryStore::new_for_test().await.unwrap();
        for i in 0..3 {
            insert_memory_row(&store, "a", &format!("fila {i}"), "semantic_fact", 3, 5.0, "active", serialize_embedding(&axis(i))).await;
        }
        insert_memory_row(&store, "c", "fila de c", "semantic_fact", 3, 5.0, "active", serialize_embedding(&axis(3))).await;
        let count = |reach: Reach| {
            let store = &store;
            async move {
                let agents = reach_bind("b", &reach);
                store
                    .conn
                    .call(move |c| -> Result<i64, rusqlite::Error> {
                        c.query_row(ELIGIBLE_SQL_MEMORY, params![30_i64, agents, Option::<String>::None], |r| r.get(0))
                    })
                    .await
                    .unwrap()
            }
        };
        assert_eq!(count(Reach::Own).await, 0);
        assert_eq!(count(everyone()).await, 4);
        assert_eq!(count(Reach::AllBut(vec!["a".to_string()])).await, 1);
    }

    #[tokio::test]
    async fn reaching_only_its_own_leaves_the_search_unchanged() {
        let store = MemoryStore::new_for_test().await.unwrap();
        for i in 0..4 {
            insert_memory_row(&store, "b", &format!("propia {i}"), "semantic_fact", 3, 5.0, "active", serialize_embedding(&axis(i))).await;
        }
        let run = |scope: SearchScope| {
            let store = &store;
            async move {
                search_all_memories(store, &axis(1), Some("propia"), "b", scope, 10)
                    .await
                    .unwrap()
                    .iter()
                    .map(|h| (h.id, h.score.to_bits()))
                    .collect::<Vec<_>>()
            }
        };
        let plain = run(SearchScope::all_default()).await;
        assert_eq!(plain.len(), 4);
        assert_eq!(run(SearchScope::all_default().with_reach(everyone())).await, plain, "with no one else's rows, reaching everyone adds nothing");
    }

    #[tokio::test]
    async fn a_hit_carries_its_author() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let mine = insert_memory_row(&store, "b", "propia", "semantic_fact", 3, 5.0, "active", serialize_embedding(&axis(0))).await;
        let theirs = insert_memory_row(&store, "a", "ajena", "semantic_fact", 3, 5.0, "active", serialize_embedding(&axis(0))).await;
        let scope = SearchScope::all_default().with_reach(everyone());
        let hits = search_all_memories(&store, &axis(0), Some("ajena"), "b", scope, 10).await.unwrap();
        let author = |id: i64| hits.iter().find(|h| h.id == id).and_then(|h| h.agent_id.clone());
        assert_eq!(author(mine).as_deref(), Some("b"));
        assert_eq!(author(theirs).as_deref(), Some("a"));
    }

    // ── one hit per save ─────────────────────────────────────────────

    fn memory_hit(id: i64, score: f64) -> RankedHit {
        RankedHit {
            id,
            kind: TableKind::Memory,
            content: format!("row {id}"),
            memory_type: "chat_message".to_string(),
            tier: 3,
            status: "active".to_string(),
            score,
            distance: 0.0,
            created_at: TimestampMs::from_unix_millis(0),
            agent_id: None,
            piece: None,
            exact: false,
            topic_key: None,
            project_id: None,
        }
    }

    fn link(save_id: i64, piece: i64, pieces: i64) -> SaveLink {
        SaveLink { save_id, position: PiecePosition { piece, pieces } }
    }

    fn ids_and_pieces(hits: &[RankedHit]) -> Vec<(i64, Option<(i64, i64)>)> {
        hits.iter().map(|h| (h.id, h.piece.map(|p| (p.piece, p.pieces)))).collect()
    }

    #[test]
    fn only_the_best_piece_of_a_save_survives() {
        let hits = vec![memory_hit(12, 0.9), memory_hit(5, 0.8), memory_hit(11, 0.7), memory_hit(10, 0.6)];
        let links = HashMap::from([(10, link(1, 1, 3)), (11, link(1, 2, 3)), (12, link(1, 3, 3))]);

        let collapsed = collapse_saves(hits, &links);

        assert_eq!(ids_and_pieces(&collapsed), vec![(12, Some((3, 3))), (5, None)]);
    }

    #[test]
    fn rows_without_links_keep_their_place() {
        let mut episode = memory_hit(11, 0.75);
        episode.kind = TableKind::Episodic;
        let hits = vec![memory_hit(3, 0.9), episode, memory_hit(2, 0.7), memory_hit(1, 0.6)];
        // An episode shares its id with a linked memory row by coincidence;
        // only `memory` rows are pieces.
        let links = HashMap::from([(11, link(4, 2, 2)), (2, link(7, 1, 2))]);

        let collapsed = collapse_saves(hits, &links);

        assert_eq!(ids_and_pieces(&collapsed), vec![(3, None), (11, None), (2, Some((1, 2))), (1, None)]);
        assert_eq!(collapsed[1].kind, TableKind::Episodic);
    }

    /// `Reach::admits` and the SQL predicate are two spellings of one rule:
    /// LAST SESSION and the last check on the hits read the first, every query
    /// the second.
    #[test]
    fn the_reach_predicate_agrees_with_admits() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let sql = concat!("SELECT count(*) FROM (SELECT ?2 AS agent_id) t WHERE ", reach_predicate!("t.agent_id", "?1"));
        let all_but = |names: &[&str]| Reach::AllBut(names.iter().map(|n| n.to_string()).collect());
        for reach in [Reach::Own, all_but(&[]), all_but(&["b"]), all_but(&["b", "c"])] {
            for author in [Some("a"), Some("b"), Some("c"), None] {
                let in_sql: i64 = conn.query_row(sql, params![reach_bind("a", &reach), author], |r| r.get(0)).unwrap();
                assert_eq!(in_sql == 1, reach.admits("a", author), "{reach:?} reading a row of {author:?}");
            }
        }
        let sql = concat!("SELECT count(*) FROM (SELECT ?2 AS agent_id) t WHERE ", reader_predicate!("t.agent_id", "?1"));
        for author in [Some("a"), Some("b"), None] {
            let in_sql: i64 = conn.query_row(sql, params![reader_bind(None), author], |r| r.get(0)).unwrap();
            assert_eq!(in_sql, 1, "a call with no reader reaches a row of {author:?}");
            let hidden = Reader::new("a", all_but(&["b"]));
            let in_sql: i64 = conn.query_row(sql, params![reader_bind(Some(&hidden)), author], |r| r.get(0)).unwrap();
            assert_eq!(in_sql == 1, hidden.admits(author), "a reader reading a row of {author:?}");
        }
    }

    #[test]
    fn a_hit_the_reader_may_not_see_is_dropped_and_the_order_kept() {
        let by = |id: i64, author: Option<&str>| {
            let mut hit = memory_hit(id, 1.0 - id as f64 / 10.0);
            hit.agent_id = author.map(str::to_string);
            hit
        };
        let hits = vec![by(1, Some("hidden")), by(2, Some("me")), by(3, None), by(4, Some("visible")), by(5, Some("hidden"))];

        let reach = Reach::AllBut(vec!["hidden".to_string()]);
        let kept = readable_hits(hits.clone(), "me", &reach);
        assert_eq!(kept.iter().map(|h| h.id).collect::<Vec<_>>(), vec![2, 3, 4]);

        let own = readable_hits(hits, "me", &Reach::Own);
        assert_eq!(own.iter().map(|h| h.id).collect::<Vec<_>>(), vec![2, 3]);
    }

    async fn seed_linked_save(store: &MemoryStore, agent: &str) -> Vec<i64> {
        let mut ids = Vec::new();
        for (i, cos) in [0.99_f32, 0.98, 0.97].into_iter().enumerate() {
            ids.push(
                insert_memory_row(
                    store, agent, &format!("piece {i} of the ledger notes"), "chat_message", 3, 5.0, "active",
                    serialize_embedding(&vec_at_cosine(cos)),
                )
                .await,
            );
        }
        store
            .link_pieces(
                ids.iter()
                    .map(|&memory_id| crate::memory::pieces::StoredPiece { memory_id, context: String::new() })
                    .collect(),
            )
            .await
            .unwrap();
        ids
    }

    async fn seed_others(store: &MemoryStore, agent: &str) -> Vec<i64> {
        let mut ids = Vec::new();
        for (i, cos) in [0.96_f32, 0.95, 0.94].into_iter().enumerate() {
            ids.push(
                insert_memory_row(
                    store, agent, &format!("unrelated memory {i}"), "chat_message", 3, 5.0, "active",
                    serialize_embedding(&vec_at_cosine(cos)),
                )
                .await,
            );
        }
        ids
    }

    #[tokio::test]
    async fn freed_slots_go_to_other_memories() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let agent = "agent-one-hit-per-save";
        let save = seed_linked_save(&store, agent).await;
        let others = seed_others(&store, agent).await;

        let (hits, _, _) = search_all_memories_inner(
            &store, &unit_basis_vec(), None, agent, SearchScope::all_default(), 3, &ScoringConfig::DEFAULT,
        )
        .await
        .unwrap();

        assert_eq!(ids_and_pieces(&hits), vec![(save[0], Some((1, 3))), (others[0], None), (others[1], None)]);
    }

    #[tokio::test]
    async fn the_memory_pool_counts_saves_and_reports_its_eligible_rows() {
        let only_memory = SearchScope { memory: Some(30), episodic: None, procedural: None, ..SearchScope::all_default() };

        // A scope with fewer rows than the window: the count of what it holds
        // is read, and the three pieces of one save are one save.
        let store = MemoryStore::new_for_test().await.unwrap();
        let agent = "agent-pool-small";
        let mut seeded = seed_linked_save(&store, agent).await;
        seeded.extend(seed_others(&store, agent).await);
        let (_, _, pool) = search_all_memories_inner(
            &store, &unit_basis_vec(), None, agent, only_memory.clone(), 10, &ScoringConfig::DEFAULT,
        )
        .await
        .unwrap();
        let mut ids = pool.ids.clone();
        ids.sort_unstable();
        seeded.sort_unstable();
        assert_eq!(
            (pool.requested, pool.rows, pool.saves, pool.passes, pool.eligible),
            (30, 6, 4, 1, Some(6))
        );
        assert_eq!(ids, seeded);

        // A window the other project's rows crowded: the scope holds more
        // than the window, and only the wider pass reaches it.
        let store = MemoryStore::new_for_test().await.unwrap();
        let agent = "agent-pool-crowded";
        bulk_insert_memory_rows(&store, agent, Some("P2"), 30, serialize_embedding(&unit_basis_vec())).await;
        bulk_insert_memory_rows(&store, agent, Some("P1"), 40, serialize_embedding(&off_axis_vec())).await;
        let (_, _, pool) = search_all_memories_inner(
            &store, &unit_basis_vec(), None, agent, only_memory.clone().with_project(Some("P1")), 10,
            &ScoringConfig::DEFAULT,
        )
        .await
        .unwrap();
        assert_eq!((pool.rows, pool.saves, pool.passes, pool.eligible), (30, 30, 2, Some(30)));

        // A full window of distinct saves never reads the count.
        let store = MemoryStore::new_for_test().await.unwrap();
        let agent = "agent-pool-full";
        bulk_insert_memory_rows(&store, agent, None, 35, serialize_embedding(&unit_basis_vec())).await;
        let (_, _, pool) =
            search_all_memories_inner(&store, &unit_basis_vec(), None, agent, only_memory, 10, &ScoringConfig::DEFAULT)
                .await
                .unwrap();
        assert_eq!((pool.rows, pool.saves, pool.passes, pool.eligible), (30, 30, 1, None));
    }

    #[test]
    fn distinct_saves_counts_unlinked_rows_as_their_own() {
        let links = HashMap::from([(10, link(1, 1, 3)), (11, link(1, 2, 3)), (20, link(2, 1, 2))]);

        assert_eq!(distinct_saves([10, 11, 5, 20, 6].into_iter(), &links), 4);
        assert_eq!(distinct_saves([10, 11].into_iter(), &links), 1);
        assert_eq!(distinct_saves(std::iter::empty(), &links), 0);
    }

    #[test]
    fn trim_to_saves_keeps_the_rows_up_to_k_saves() {
        let hits = vec![
            memory_hit(10, 0.9),
            memory_hit(11, 0.8),
            memory_hit(5, 0.7),
            memory_hit(12, 0.6),
            memory_hit(6, 0.5),
            memory_hit(7, 0.4),
        ];
        let links = HashMap::from([(10, link(1, 1, 3)), (11, link(1, 2, 3)), (12, link(1, 3, 3))]);

        let trimmed = trim_to_saves(hits.clone(), &links, 3);
        let none = trim_to_saves(hits, &HashMap::new(), 3);

        assert_eq!(trimmed.iter().map(|h| h.id).collect::<Vec<_>>(), vec![10, 11, 5, 12, 6]);
        assert_eq!(none.iter().map(|h| h.id).collect::<Vec<_>>(), vec![10, 11, 5], "without links, k rows");
    }

    #[tokio::test]
    async fn a_save_with_more_pieces_than_the_pool_leaves_room_for_other_memories() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let agent = "agent-crowded-pool";
        let mut pieces = Vec::new();
        for i in 0..40 {
            let cos = 0.999 - 0.0001 * i as f32;
            pieces.push(
                insert_memory_row(
                    &store, agent, &format!("piece {i} of a long import"), "chat_message", 3, 5.0, "active",
                    serialize_embedding(&vec_at_cosine(cos)),
                )
                .await,
            );
        }
        store
            .link_pieces(
                pieces
                    .iter()
                    .map(|&memory_id| crate::memory::pieces::StoredPiece { memory_id, context: String::new() })
                    .collect(),
            )
            .await
            .unwrap();
        let mut others = Vec::new();
        for i in 0..10 {
            let cos = 0.95 - 0.001 * i as f32;
            others.push(
                insert_memory_row(
                    &store, agent, &format!("another memory {i}"), "chat_message", 3, 5.0, "active",
                    serialize_embedding(&vec_at_cosine(cos)),
                )
                .await,
            );
        }

        let (hits, _, _) = search_all_memories_inner(
            &store, &unit_basis_vec(), None, agent, SearchScope::all_default(), 10, &ScoringConfig::DEFAULT,
        )
        .await
        .unwrap();

        let mut expected = vec![(pieces[0], Some((1, 40)))];
        expected.extend(others[..9].iter().map(|&id| (id, None)));
        assert_eq!(ids_and_pieces(&hits), expected);
    }

    #[tokio::test]
    async fn a_link_read_failure_leaves_hits_uncollapsed() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let agent = "agent-link-failsoft";
        let save = seed_linked_save(&store, agent).await;
        store.conn.call(|c| c.execute("DROP TABLE memory_piece", [])).await.unwrap();

        let (hits, _, _) = search_all_memories_inner(
            &store, &unit_basis_vec(), None, agent, SearchScope::all_default(), 3, &ScoringConfig::DEFAULT,
        )
        .await
        .unwrap();

        assert_eq!(ids_and_pieces(&hits), save.iter().map(|&id| (id, None)).collect::<Vec<_>>());
    }
}
