//! Judging the pairs the contradiction detector found.
//!
//! Every pair is a `memory_conflict` row. An escalated pair is also named in
//! the newer row's `disputed_against`, which makes that row `disputed` and
//! puts the pair in ACTIVE CONFLICTS. A verdict either supersedes one side
//! (`superseded_by` points at the row that stays), keeps both, or escalates
//! the pair; the first two drop each side from the other's
//! `disputed_against`, so that list always names the disputes still open, and
//! the pair keeps the verdict.
//!
//! A row that is not superseded never has its status written directly: every
//! path settles it with [`REST_STATUS`], which reads it off the rows its
//! `disputed_against` still names. That keeps resolving, deleting and the
//! maintenance pass on one rule.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::memory::clock::TimestampMs;
use crate::memory::provenance::mark;
use crate::memory::redact::redact;
use crate::memory::search::{memory_project_predicate, reader_bind, reader_predicate};
use crate::memory::sharing::Reader;
use crate::memory::store_insert::MemoryType;
use crate::memory::{MemoryError, MemoryStore};

/// How a dispute between two rows is closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The kept row stands and the other is superseded by it.
    Supersede,
    /// Both rows stand: the detector paired two claims that do not conflict.
    KeepBoth,
}

/// What a caller says about a pair the detector found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// `keep` replaces `other`, which stops being retrieved.
    Supersedes,
    /// The two do not conflict, related or not.
    Compatible,
    /// Both hold, each in its own scope; the reason says which.
    Scoped,
    /// They do conflict, and the user has to say which holds.
    Conflicts,
}

impl Verdict {
    pub const ALL: [Verdict; 4] = [Self::Supersedes, Self::Compatible, Self::Scoped, Self::Conflicts];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Supersedes => "supersedes",
            Self::Compatible => "compatible",
            Self::Scoped => "scoped",
            Self::Conflicts => "conflicts",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown verdict {0:?}; expected one of {list}", list = Verdict::ALL.map(Verdict::as_str).join(", "))]
pub struct UnknownVerdict(pub String);

impl std::str::FromStr for Verdict {
    type Err = UnknownVerdict;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Verdict::ALL.into_iter().find(|v| v.as_str() == s).ok_or_else(|| UnknownVerdict(s.to_string()))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    #[error("a memory cannot be resolved against itself")]
    SameRow,
    #[error("a scoped verdict needs a reason that says where each memory holds")]
    ReasonRequired,
    #[error(
        "memory {other} is a preference or a decision, so {keep} supersedes it only on the user's word: \
         judge the pair `conflicts` and ask the user, then call again with `user_answered` and their \
         answer in `reason`"
    )]
    NeedsUser { keep: i64, other: i64 },
    #[error("`user_answered` needs the user's answer in `reason`")]
    AnswerRequired,
    #[error("no memory row with id {0}")]
    NotFound(i64),
    #[error("memories {keep} and {other} are not in dispute with each other")]
    NotInDispute { keep: i64, other: i64 },
    #[error("memory {id} was replaced by memory {by}; resolve that pair instead")]
    SupersededElsewhere { id: i64, by: i64 },
    #[error(transparent)]
    Store(#[from] MemoryError),
}

impl From<rusqlite::Error> for ResolveError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Store(MemoryError::Sqlite(e))
    }
}

/// One side of a resolved pair, as it stands after the resolution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedRow {
    pub id: i64,
    pub status: String,
    pub superseded_by: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resolution {
    pub keep: ResolvedRow,
    pub other: ResolvedRow,
    /// The state the pair was left in, `judged` or `escalated`. Absent from
    /// a daemon that predates the pair table.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pair: Option<String>,
}

/// What deciding whether two rows can be resolved needs from each of them.
#[derive(Debug, Clone, PartialEq)]
struct PairRow {
    id: i64,
    status: String,
    superseded_by: Option<i64>,
    disputes: Vec<i64>,
    memory_type: String,
}

impl PairRow {
    fn replaced_by(&self) -> Option<i64> {
        (self.status == "superseded").then_some(self.superseded_by).flatten()
    }

    fn protected(&self) -> bool {
        MemoryType::is_protected_name(&self.memory_type)
    }
}

/// Two rows can be resolved when the detector paired them (`noted`), when one
/// names the other in `disputed_against`, or when one was superseded by the
/// other — the last is what lets a resolution be undone. A row superseded by
/// a third row is refused: rewriting its `superseded_by` would silently drop
/// that earlier decision.
fn check_pair(keep: &PairRow, other: &PairRow, noted: bool) -> Result<(), ResolveError> {
    for (row, partner) in [(keep, other), (other, keep)] {
        if let Some(by) = row.replaced_by().filter(|by| *by != partner.id) {
            return Err(ResolveError::SupersededElsewhere { id: row.id, by });
        }
    }
    let linked = noted
        || keep.disputes.contains(&other.id)
        || other.disputes.contains(&keep.id)
        || keep.replaced_by() == Some(other.id)
        || other.replaced_by() == Some(keep.id);
    if linked {
        Ok(())
    } else {
        Err(ResolveError::NotInDispute { keep: keep.id, other: other.id })
    }
}

/// The rows `memory.disputed_against` names. A value that is not valid JSON
/// names nothing instead of failing the statement: these run inside deletes,
/// and one malformed row must not make every other row undeletable.
const NAMED_DISPUTES: &str = "json_each(CASE WHEN json_valid(memory.disputed_against) \
                              THEN memory.disputed_against END)";

/// The status a row that is not superseded rests at: `disputed` while
/// `disputed_against` names a live row, `active` otherwise. A deleted or
/// superseded row no longer counts, which is what releases its challengers.
fn rest_status() -> String {
    format!(
        "CASE WHEN EXISTS (SELECT 1 FROM {NAMED_DISPUTES} j JOIN memory p ON p.id = j.value \
         WHERE p.deleted_at IS NULL AND p.status <> 'superseded') \
         THEN 'disputed' ELSE 'active' END"
    )
}

/// Settles the rows `predicate` selects. Only rows whose status actually
/// changes are written, so a pass with nothing to fix writes nothing. `?1` is
/// the timestamp; the predicate may bind `?2`.
fn settle_sql(predicate: &str) -> String {
    let rest = rest_status();
    format!(
        "UPDATE memory SET status = {rest}, last_modified_at = ?1 \
         WHERE {predicate} AND deleted_at IS NULL AND status IN ('active', 'disputed') \
         AND status <> {rest}"
    )
}

/// Releases the rows disputing row `id` once it has been deleted or
/// superseded: each goes back to `active` unless another dispute keeps it
/// `disputed`. Runs on the caller's connection so it joins the caller's
/// transaction.
pub(crate) fn release_challengers(c: &Connection, id: i64, now: TimestampMs) -> rusqlite::Result<usize> {
    let sql = settle_sql(&format!(
        "status = 'disputed' AND EXISTS (SELECT 1 FROM {NAMED_DISPUTES} WHERE value = ?2)"
    ));
    c.execute(&sql, rusqlite::params![now, id])
}

/// Settles every row whose `disputed_against` names row `id`, once `id` is
/// live again: a restored row brings back the escalated pairs it was part of.
pub(crate) fn settle_challengers(c: &Connection, id: i64, now: TimestampMs) -> rusqlite::Result<usize> {
    let sql = settle_sql(&format!("EXISTS (SELECT 1 FROM {NAMED_DISPUTES} WHERE value = ?2)"));
    c.execute(&sql, rusqlite::params![now, id])
}

/// Settles row `id` alone. A row coming back from deletion may name rows
/// that were deleted or superseded while it was gone.
pub(crate) fn settle_row(c: &Connection, id: i64, now: TimestampMs) -> rusqlite::Result<usize> {
    c.execute(&settle_sql("id = ?2"), rusqlite::params![now, id])
}

/// Settles every `disputed` row. A delete that ran before deletes released
/// their challengers left those rows `disputed` with nothing to be resolved
/// against; this is the backfill for them.
pub(crate) fn settle_disputed(c: &Connection, now: TimestampMs) -> rusqlite::Result<usize> {
    c.execute(&settle_sql("status = 'disputed'"), rusqlite::params![now])
}

const SELECT_PAIR_ROW: &str = concat!(
    "SELECT id, status, superseded_by, disputed_against, memory_type FROM memory \
     WHERE id = ?1 AND deleted_at IS NULL AND ",
    reader_predicate!("agent_id", "?2")
);

/// A row out of the reader's reach (bound as `reach_bind` binds it) reads as
/// missing, so a reader learns nothing about ids the policy hides from it.
fn read_pair_row(c: &Connection, id: i64, reach: Option<&str>) -> Result<PairRow, ResolveError> {
    use rusqlite::OptionalExtension;
    let row = c
        .query_row(SELECT_PAIR_ROW, rusqlite::params![id, reach], |r| {
            Ok(PairRow {
                id: r.get(0)?,
                status: r.get(1)?,
                superseded_by: r.get(2)?,
                disputes: r
                    .get::<_, Option<String>>(3)?
                    .and_then(|s| serde_json::from_str(&s).ok())
                    .unwrap_or_default(),
                memory_type: r.get(4)?,
            })
        })
        .optional()?;
    row.ok_or(ResolveError::NotFound(id))
}

fn read_resolved(c: &Connection, id: i64) -> rusqlite::Result<ResolvedRow> {
    c.query_row("SELECT id, status, superseded_by FROM memory WHERE id = ?1", [id], |r| {
        Ok(ResolvedRow { id: r.get(0)?, status: r.get(1)?, superseded_by: r.get(2)? })
    })
}

/// Drops `?2` from `?1`'s `disputed_against`, writing NULL when nothing is
/// left, the same value a row inserted without a dispute carries.
fn drop_dispute_sql() -> String {
    format!(
        "UPDATE memory SET disputed_against = \
           (SELECT CASE WHEN COUNT(*) = 0 THEN NULL ELSE json_group_array(value) END \
            FROM {NAMED_DISPUTES} WHERE value <> ?2), \
         last_modified_at = ?3 \
         WHERE id = ?1 AND EXISTS (SELECT 1 FROM {NAMED_DISPUTES} WHERE value = ?2)"
    )
}

/// Brings `?1` back from being superseded by `?2`. Its status is settled
/// afterwards; `active` here is only the starting point.
pub(crate) const REVIVE_SQL: &str = "UPDATE memory SET status = 'active', superseded_by = NULL, supersede_cause = NULL, \
                          last_modified_at = ?3 \
                          WHERE id = ?1 AND status = 'superseded' AND superseded_by = ?2";

/// Supersedes `?1` by `?2` for the reason `?4`, `revision` or `verdict`. A row
/// a delete had brought back is no longer that delete's to take back.
pub(crate) const SUPERSEDE_SQL: &str = "UPDATE memory SET status = 'superseded', superseded_by = ?2, \
                             supersede_cause = ?4, revived_from = NULL, last_modified_at = ?3 \
                             WHERE id = ?1";

/// Adds `?2` to `?1`'s `disputed_against` unless it is there already.
fn add_dispute_sql() -> String {
    format!(
        "UPDATE memory SET disputed_against = \
           (SELECT json_group_array(value) FROM \
              (SELECT value FROM {NAMED_DISPUTES} UNION SELECT ?2)), \
         last_disputed_at = ?3, last_modified_at = ?3 \
         WHERE id = ?1 AND NOT EXISTS (SELECT 1 FROM {NAMED_DISPUTES} WHERE value = ?2)"
    )
}

/// Names row `against` in row `row`'s `disputed_against`, inside the
/// caller's transaction. The row's status is settled afterwards.
pub(crate) fn add_dispute_in(c: &Connection, row: i64, against: i64, now: TimestampMs) -> rusqlite::Result<usize> {
    c.execute(&add_dispute_sql(), rusqlite::params![row, against, now])
}

/// Records the pair's state after a verdict. A pair with no row (two rows
/// linked only by `superseded_by`, from before the pair table) gets one.
const RECORD_PAIR_SQL: &str = "INSERT INTO memory_conflict \
     (new_id, old_id, distance, state, verdict, reason, created_at, judged_at) \
     VALUES (?1, ?2, NULL, ?3, ?4, ?5, ?6, ?7) \
     ON CONFLICT (new_id, old_id) DO UPDATE SET \
       state = excluded.state, verdict = excluded.verdict, reason = excluded.reason, \
       judged_at = excluded.judged_at";

/// [`judge_pair`] with the outcome a caller that predates verdicts sends.
pub async fn resolve_dispute_core(
    store: &MemoryStore,
    keep: i64,
    other: i64,
    outcome: Outcome,
    reader: Option<&Reader>,
) -> Result<Resolution, ResolveError> {
    let verdict = match outcome {
        Outcome::Supersede => Verdict::Supersedes,
        Outcome::KeepBoth => Verdict::Compatible,
    };
    judge_pair(store, keep, other, verdict, None, false, reader).await
}

/// Judges the pair `keep`, `other` in one transaction, and returns both rows
/// as they stand afterwards. Both rows must be within `reader`'s reach, of
/// whichever author. `keep` only matters for `supersedes`: it is the row that stays.
///
/// `user_answered` is the caller's declaration that the user said which
/// holds, with their answer in `reason`. Superseding a preference or a
/// decision needs it whatever state the pair is in; it declares, it does not
/// authenticate, and the pair keeps the answer marked as declared.
pub async fn judge_pair(
    store: &MemoryStore,
    keep: i64,
    other: i64,
    verdict: Verdict,
    reason: Option<String>,
    user_answered: bool,
    reader: Option<&Reader>,
) -> Result<Resolution, ResolveError> {
    if keep == other {
        return Err(ResolveError::SameRow);
    }
    // Masked as a save is: the pair lives in memory.db and in every snapshot.
    let reason = reason.map(|r| r.trim().to_string()).filter(|r| !r.is_empty()).map(|r| redact(&r).text);
    if verdict == Verdict::Scoped && reason.is_none() {
        return Err(ResolveError::ReasonRequired);
    }
    let reason = match (user_answered, reason) {
        (true, None) => return Err(ResolveError::AnswerRequired),
        (true, Some(r)) => Some(format!("user answered: {r}")),
        (false, r) => r,
    };
    let now = TimestampMs::now();
    let reach = reader_bind(reader);
    let (new, old) = (keep.max(other), keep.min(other));
    store
        .conn
        .call(move |c| -> Result<Resolution, ResolveError> {
            use rusqlite::OptionalExtension;
            let tx = c.transaction()?;
            let keep_row = read_pair_row(&tx, keep, reach.as_deref())?;
            let other_row = read_pair_row(&tx, other, reach.as_deref())?;
            let state: Option<String> = tx
                .query_row(
                    "SELECT state FROM memory_conflict WHERE new_id = ?1 AND old_id = ?2",
                    [new, old],
                    |r| r.get(0),
                )
                .optional()?;
            check_pair(&keep_row, &other_row, state.is_some())?;
            if verdict == Verdict::Supersedes && other_row.protected() && !user_answered {
                return Err(ResolveError::NeedsUser { keep, other });
            }

            let drop_dispute = drop_dispute_sql();
            if verdict != Verdict::Conflicts {
                tx.execute(&drop_dispute, rusqlite::params![keep, other, now])?;
                tx.execute(&drop_dispute, rusqlite::params![other, keep, now])?;
            }
            tx.execute(REVIVE_SQL, rusqlite::params![keep, other, now])?;
            match verdict {
                Verdict::Supersedes => {
                    tx.execute(SUPERSEDE_SQL, rusqlite::params![other, keep, now, "verdict"])?;
                    release_challengers(&tx, other, now)?;
                }
                Verdict::Compatible | Verdict::Scoped => {
                    tx.execute(REVIVE_SQL, rusqlite::params![other, keep, now])?;
                }
                Verdict::Conflicts => {
                    tx.execute(REVIVE_SQL, rusqlite::params![other, keep, now])?;
                    tx.execute(&add_dispute_sql(), rusqlite::params![new, old, now])?;
                }
            }
            let settle_one = settle_sql("id = ?2");
            tx.execute(&settle_one, rusqlite::params![now, keep])?;
            tx.execute(&settle_one, rusqlite::params![now, other])?;

            let (pair, judged, judged_at) = match verdict {
                Verdict::Conflicts => ("escalated", None, None),
                v => ("judged", Some(v.as_str()), Some(now)),
            };
            tx.execute(RECORD_PAIR_SQL, rusqlite::params![new, old, pair, judged, reason, now, judged_at])?;

            let resolution = Resolution {
                keep: read_resolved(&tx, keep)?,
                other: read_resolved(&tx, other)?,
                pair: Some(pair.to_string()),
            };
            tx.commit()?;
            Ok(resolution)
        })
        .await
        .map_err(|e| match e {
            tokio_rusqlite::Error::Error(e) => e,
            tokio_rusqlite::Error::ConnectionClosed => {
                ResolveError::Store(MemoryError::Backend("connection closed during judge_pair".into()))
            }
            other => ResolveError::Store(MemoryError::Backend(other.to_string())),
        })
}

/// A pair waiting for the agent's verdict.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenPair {
    pub new_id: i64,
    pub old_id: i64,
    pub distance: Option<f64>,
    pub created_at: i64,
    /// What a reader shows of each side's origin (`provenance::mark`);
    /// absent from a daemon that predates it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_mark: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_mark: Option<String>,
}

/// The pairs not judged yet, among live rows.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OpenPairs {
    pub pending: i64,
    pub escalated: i64,
    /// The oldest pending pairs, up to the limit asked for.
    pub pairs: Vec<OpenPair>,
}

/// A pair counts while both rows are live, its newer row in the project
/// bound to `?3` as a search reads it (no project: the whole store), and both
/// rows within the reader's reach bound to `?1`, of whichever author.
const OPEN_PAIR_FROM: &str = concat!(
    "FROM memory_conflict p \
     JOIN memory n ON n.id = p.new_id JOIN memory o ON o.id = p.old_id \
     WHERE n.deleted_at IS NULL AND o.deleted_at IS NULL \
       AND n.status <> 'superseded' AND o.status <> 'superseded' AND ",
    reader_predicate!("n.agent_id", "?1"),
    " AND ",
    reader_predicate!("o.agent_id", "?1"),
    " AND ",
    memory_project_predicate!("n.", "?3")
);

/// Counts the open pairs `reader` reaches in `project_id` and lists the
/// `limit` oldest pending ones.
pub async fn open_pairs(
    store: &MemoryStore,
    reader: Option<&Reader>,
    project_id: Option<&str>,
    limit: usize,
) -> Result<OpenPairs, MemoryError> {
    let reach = reader_bind(reader);
    let project = project_id.map(str::to_owned);
    store
        .conn
        .call(move |c| -> rusqlite::Result<OpenPairs> {
            let (pending, escalated) = c.query_row(
                &format!(
                    "SELECT COALESCE(SUM(p.state = 'pending'), 0), COALESCE(SUM(p.state = 'escalated'), 0) \
                     {OPEN_PAIR_FROM}"
                ),
                rusqlite::params![reach, Option::<i64>::None, project],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            let mut stmt = c.prepare(&format!(
                "SELECT p.new_id, p.old_id, p.distance, p.created_at, n.source, n.created_by_type, \
                 o.source, o.created_by_type {OPEN_PAIR_FROM} AND p.state = 'pending' \
                 ORDER BY p.created_at, p.new_id, p.old_id LIMIT ?2"
            ))?;
            let side = |r: &rusqlite::Row<'_>, at: usize| -> rusqlite::Result<Option<String>> {
                let (source, by): (String, String) = (r.get(at)?, r.get(at + 1)?);
                Ok(mark(&source, &by).map(str::to_owned))
            };
            let pairs = stmt
                .query_map(rusqlite::params![reach, limit as i64, project], |r| {
                    Ok(OpenPair {
                        new_id: r.get(0)?,
                        old_id: r.get(1)?,
                        distance: r.get(2)?,
                        created_at: r.get(3)?,
                        new_mark: side(r, 4)?,
                        old_mark: side(r, 6)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(OpenPairs { pending, escalated, pairs })
        })
        .await
        .map_err(|e| match e {
            tokio_rusqlite::Error::Error(re) => MemoryError::Sqlite(re),
            other => MemoryError::Backend(format!("open_pairs: {other}")),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::sharing::Reach;
    use crate::memory::admin::{list_memories_core, ListParams, MemoryKind};

    fn reader(agent: &str) -> Reader {
        Reader::new(agent, Reach::AllBut(Vec::new()))
    }
    use crate::memory::search::{search_all_memories, SearchScope};

    fn pair_row(id: i64, status: &str, superseded_by: Option<i64>, disputes: &[i64]) -> PairRow {
        PairRow { id, status: status.into(), superseded_by, disputes: disputes.to_vec(), memory_type: "semantic_fact".into() }
    }

    // ── check_pair ──────────────────────────────────────────────────────────

    #[test]
    fn a_dispute_in_either_direction_links_the_pair() {
        let old = pair_row(1, "active", None, &[]);
        let new = pair_row(2, "disputed", None, &[1]);
        assert!(check_pair(&new, &old, false).is_ok());
        assert!(check_pair(&old, &new, false).is_ok());
    }

    #[test]
    fn a_superseded_link_in_either_direction_links_the_pair() {
        let winner = pair_row(1, "active", None, &[]);
        let loser = pair_row(2, "superseded", Some(1), &[]);
        assert!(check_pair(&winner, &loser, false).is_ok());
        assert!(check_pair(&loser, &winner, false).is_ok());
    }

    #[test]
    fn a_noted_pair_links_the_rows() {
        let a = pair_row(1, "active", None, &[]);
        let b = pair_row(2, "active", None, &[]);
        assert!(check_pair(&a, &b, true).is_ok());
    }

    #[test]
    fn unrelated_rows_are_not_linked() {
        let a = pair_row(1, "active", None, &[]);
        let b = pair_row(2, "disputed", None, &[3]);
        assert!(matches!(check_pair(&a, &b, false), Err(ResolveError::NotInDispute { keep: 1, other: 2 })));
    }

    #[test]
    fn a_row_superseded_by_a_third_row_is_refused() {
        let a = pair_row(1, "superseded", Some(9), &[2]);
        let b = pair_row(2, "active", None, &[]);
        assert!(matches!(check_pair(&a, &b, false), Err(ResolveError::SupersededElsewhere { id: 1, by: 9 })));
        assert!(matches!(check_pair(&b, &a, false), Err(ResolveError::SupersededElsewhere { id: 1, by: 9 })));
    }

    #[test]
    fn a_stale_superseded_by_on_a_live_row_does_not_link() {
        let a = pair_row(1, "active", Some(2), &[]);
        let b = pair_row(2, "active", None, &[]);
        assert!(matches!(check_pair(&a, &b, false), Err(ResolveError::NotInDispute { .. })));
    }

    // ── against the store ───────────────────────────────────────────────────

    fn basis(i: usize) -> Vec<f32> {
        let mut v = vec![0.0_f32; crate::memory::vector::EMBEDDING_DIM];
        v[i] = 1.0;
        v
    }

    /// Inserts a row the way the detector leaves it: `status`, the
    /// `disputed_against` JSON and `last_disputed_at`, with a vector along
    /// axis `axis` so search can reach it.
    async fn seed(store: &MemoryStore, content: &str, status: &str, disputes: &[i64], axis: usize) -> i64 {
        let content = content.to_string();
        let status = status.to_string();
        let disputes = (!disputes.is_empty()).then(|| serde_json::to_string(disputes).unwrap());
        let bytes: Vec<u8> = basis(axis).iter().flat_map(|f| f.to_le_bytes()).collect();
        store
            .conn
            .call(move |c| -> Result<i64, rusqlite::Error> {
                let tx = c.transaction()?;
                tx.execute(
                    "INSERT INTO memory (scope, agent_id, content, memory_type, source, created_at, \
                     created_by_type, created_by_ref, last_modified_at, last_accessed_at, importance, \
                     tier, status, disputed_against, last_disputed_at) \
                     VALUES ('agent', 'agent-d', ?1, 'semantic_fact', 'explicit', 1000, 'user', \
                     'agent-d', 1000, 1000, 5.0, 3, ?2, ?3, CASE WHEN ?3 IS NULL THEN NULL ELSE 1000 END)",
                    rusqlite::params![content, status, disputes],
                )?;
                let id = tx.last_insert_rowid();
                tx.execute(
                    "INSERT INTO memory_vectors(rowid, embedding) VALUES (?1, ?2)",
                    rusqlite::params![id, bytes],
                )?;
                tx.commit()?;
                Ok(id)
            })
            .await
            .unwrap()
    }

    #[derive(Debug, PartialEq)]
    struct Row {
        status: String,
        superseded_by: Option<i64>,
        disputes: Option<String>,
        last_disputed_at: Option<i64>,
    }

    async fn row(store: &MemoryStore, id: i64) -> Row {
        store
            .conn
            .call(move |c| {
                c.query_row(
                    "SELECT status, superseded_by, disputed_against, last_disputed_at FROM memory WHERE id = ?1",
                    [id],
                    |r| {
                        Ok(Row {
                            status: r.get(0)?,
                            superseded_by: r.get(1)?,
                            disputes: r.get(2)?,
                            last_disputed_at: r.get(3)?,
                        })
                    },
                )
            })
            .await
            .unwrap()
    }

    async fn soft_delete(store: &MemoryStore, id: i64) {
        store
            .conn
            .call(move |c| c.execute("UPDATE memory SET deleted_at = 5000 WHERE id = ?1", [id]))
            .await
            .unwrap();
    }

    async fn store() -> MemoryStore {
        MemoryStore::new_for_test().await.unwrap()
    }

    #[tokio::test]
    async fn the_kept_row_wins_and_the_other_is_superseded() {
        let store = store().await;
        let old = seed(&store, "the API listens on 8080", "active", &[], 0).await;
        let new = seed(&store, "the API listens on 9090", "disputed", &[old], 1).await;

        let resolution = resolve_dispute_core(&store, new, old, Outcome::Supersede, None).await.unwrap();

        assert_eq!(resolution.keep, ResolvedRow { id: new, status: "active".into(), superseded_by: None });
        assert_eq!(resolution.other, ResolvedRow { id: old, status: "superseded".into(), superseded_by: Some(new) });
        let kept = row(&store, new).await;
        assert_eq!(kept.disputes, None);
        assert_eq!(kept.last_disputed_at, Some(1000), "the record that a dispute happened stays");
    }

    async fn cause(store: &MemoryStore, id: i64) -> Option<String> {
        store
            .conn
            .call(move |c| c.query_row("SELECT supersede_cause FROM memory WHERE id = ?1", [id], |r| r.get(0)))
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn a_verdict_records_why_it_replaced() {
        let store = store().await;
        let old = seed(&store, "the API listens on 8080", "active", &[], 0).await;
        let new = seed(&store, "the API listens on 9090", "disputed", &[old], 1).await;

        resolve_dispute_core(&store, new, old, Outcome::Supersede, None).await.unwrap();
        assert_eq!(cause(&store, old).await.as_deref(), Some("verdict"));

        resolve_dispute_core(&store, old, new, Outcome::Supersede, None).await.unwrap();
        assert_eq!(cause(&store, old).await, None, "the row brought back carries no cause");
        assert_eq!(cause(&store, new).await.as_deref(), Some("verdict"));
    }

    #[tokio::test]
    async fn the_old_row_can_win() {
        let store = store().await;
        let old = seed(&store, "old", "active", &[], 0).await;
        let new = seed(&store, "new", "disputed", &[old], 1).await;

        let resolution = resolve_dispute_core(&store, old, new, Outcome::Supersede, None).await.unwrap();

        assert_eq!(resolution.keep, ResolvedRow { id: old, status: "active".into(), superseded_by: None });
        assert_eq!(resolution.other, ResolvedRow { id: new, status: "superseded".into(), superseded_by: Some(old) });
        assert_eq!(row(&store, new).await.disputes, None);
    }

    #[tokio::test]
    async fn a_row_stays_disputed_while_another_dispute_is_open() {
        let store = store().await;
        let first = seed(&store, "first", "active", &[], 0).await;
        let second = seed(&store, "second", "active", &[], 1).await;
        let new = seed(&store, "new", "disputed", &[first, second], 2).await;

        let resolution = resolve_dispute_core(&store, new, first, Outcome::Supersede, None).await.unwrap();

        assert_eq!(resolution.keep.status, "disputed");
        assert_eq!(row(&store, new).await.disputes, Some(format!("[{second}]")));

        let resolution = resolve_dispute_core(&store, new, second, Outcome::Supersede, None).await.unwrap();
        assert_eq!(resolution.keep.status, "active");
    }

    #[tokio::test]
    async fn superseding_a_row_releases_the_rows_that_disputed_it() {
        let store = store().await;
        let old = seed(&store, "old", "active", &[], 0).await;
        let new = seed(&store, "new", "disputed", &[old], 1).await;
        let other_challenger = seed(&store, "another", "disputed", &[old], 2).await;

        resolve_dispute_core(&store, new, old, Outcome::Supersede, None).await.unwrap();

        assert_eq!(row(&store, other_challenger).await.status, "active");
    }

    #[tokio::test]
    async fn a_superseded_row_leaves_search_but_stays_listed() {
        let store = store().await;
        let old = seed(&store, "old", "active", &[], 0).await;
        let new = seed(&store, "new", "disputed", &[old], 1).await;
        resolve_dispute_core(&store, new, old, Outcome::Supersede, None).await.unwrap();

        let hits = search_all_memories(&store, &basis(0), None, "agent-d", SearchScope::all_default(), 10)
            .await
            .unwrap();
        assert!(hits.iter().all(|h| h.id != old), "{hits:?}");
        assert!(hits.iter().any(|h| h.id == new), "{hits:?}");

        let listed = list_memories_core(&store, ListParams { kind: Some(MemoryKind::Memory), ..Default::default() })
            .await
            .unwrap();
        let row = listed.iter().find(|r| r.id == old).expect("the superseded row is listed");
        assert_eq!((row.status.as_str(), row.superseded_by), ("superseded", Some(new)));
    }

    #[tokio::test]
    async fn keeping_both_closes_the_dispute_without_superseding() {
        let store = store().await;
        let old = seed(&store, "old", "active", &[], 0).await;
        let new = seed(&store, "new", "disputed", &[old], 1).await;

        let resolution = resolve_dispute_core(&store, new, old, Outcome::KeepBoth, None).await.unwrap();

        assert_eq!(resolution.keep, ResolvedRow { id: new, status: "active".into(), superseded_by: None });
        assert_eq!(resolution.other, ResolvedRow { id: old, status: "active".into(), superseded_by: None });
        assert_eq!(row(&store, new).await.disputes, None);
    }

    #[tokio::test]
    async fn resolving_backwards_undoes_a_resolution() {
        let store = store().await;
        let old = seed(&store, "old", "active", &[], 0).await;
        let new = seed(&store, "new", "disputed", &[old], 1).await;
        resolve_dispute_core(&store, new, old, Outcome::Supersede, None).await.unwrap();

        let resolution = resolve_dispute_core(&store, old, new, Outcome::Supersede, None).await.unwrap();
        assert_eq!(resolution.keep, ResolvedRow { id: old, status: "active".into(), superseded_by: None });
        assert_eq!(resolution.other, ResolvedRow { id: new, status: "superseded".into(), superseded_by: Some(old) });

        let resolution = resolve_dispute_core(&store, new, old, Outcome::KeepBoth, None).await.unwrap();
        assert_eq!(resolution.keep.status, "active");
        assert_eq!(resolution.other.status, "active");
        assert_eq!(resolution.keep.superseded_by, None);
    }

    #[tokio::test]
    async fn a_revived_row_settles_against_the_disputes_it_still_names() {
        let store = store().await;
        let first = seed(&store, "first", "active", &[], 0).await;
        let second = seed(&store, "second", "active", &[], 1).await;
        let new = seed(&store, "new", "disputed", &[first, second], 2).await;
        resolve_dispute_core(&store, first, new, Outcome::Supersede, None).await.unwrap();

        let resolution = resolve_dispute_core(&store, new, first, Outcome::KeepBoth, None).await.unwrap();

        assert_eq!(resolution.keep.status, "disputed", "the dispute with `second` was never closed");
    }

    #[tokio::test]
    async fn resolving_twice_is_the_same_as_once() {
        let store = store().await;
        let old = seed(&store, "old", "active", &[], 0).await;
        let new = seed(&store, "new", "disputed", &[old], 1).await;

        let once = resolve_dispute_core(&store, new, old, Outcome::Supersede, None).await.unwrap();
        let twice = resolve_dispute_core(&store, new, old, Outcome::Supersede, None).await.unwrap();

        assert_eq!(once, twice);
    }

    #[tokio::test]
    async fn a_row_cannot_be_resolved_against_itself() {
        let store = store().await;
        let id = seed(&store, "x", "disputed", &[], 0).await;
        assert!(matches!(
            resolve_dispute_core(&store, id, id, Outcome::Supersede, None).await,
            Err(ResolveError::SameRow)
        ));
    }

    #[tokio::test]
    async fn a_pair_with_a_hidden_side_is_not_found() {
        let store = store().await;
        let old = seed(&store, "old", "active", &[], 0).await;
        let new = seed(&store, "new", "disputed", &[old], 1).await;

        let hidden = Reader::new("agent-x", Reach::AllBut(vec!["agent-d".into()]));
        assert!(matches!(
            resolve_dispute_core(&store, new, old, Outcome::Supersede, Some(&hidden)).await,
            Err(ResolveError::NotFound(id)) if id == new
        ));
        assert_eq!(row(&store, new).await.status, "disputed", "a refused resolution changes nothing");
        assert_eq!(row(&store, old).await.status, "active");
    }

    #[tokio::test]
    async fn another_agent_judges_a_pair_it_reaches() {
        let store = store().await;
        let old = seed(&store, "old", "active", &[], 0).await;
        let new = seed(&store, "new", "disputed", &[old], 1).await;

        let other = Reader::new("agent-x", Reach::AllBut(Vec::new()));
        let resolution = resolve_dispute_core(&store, new, old, Outcome::Supersede, Some(&other)).await.unwrap();
        assert_eq!(resolution.other.status, "superseded");
    }

    #[tokio::test]
    async fn a_missing_or_deleted_row_is_not_found() {
        let store = store().await;
        let old = seed(&store, "old", "active", &[], 0).await;
        let new = seed(&store, "new", "disputed", &[old], 1).await;

        assert!(matches!(
            resolve_dispute_core(&store, new, 999, Outcome::Supersede, None).await,
            Err(ResolveError::NotFound(999))
        ));
        soft_delete(&store, old).await;
        assert!(matches!(
            resolve_dispute_core(&store, new, old, Outcome::Supersede, None).await,
            Err(ResolveError::NotFound(id)) if id == old
        ));
        assert_eq!(row(&store, new).await.status, "disputed", "a refused resolution changes nothing");
    }

    #[tokio::test]
    async fn rows_without_a_dispute_are_refused() {
        let store = store().await;
        let a = seed(&store, "a", "active", &[], 0).await;
        let b = seed(&store, "b", "active", &[], 1).await;

        assert!(matches!(
            resolve_dispute_core(&store, a, b, Outcome::Supersede, None).await,
            Err(ResolveError::NotInDispute { .. })
        ));
        assert_eq!(row(&store, b).await.status, "active");
    }

    #[tokio::test]
    async fn disputes_written_by_the_previous_version_resolve_and_release() {
        let store = store().await;
        let old = seed(&store, "old", "active", &[], 0).await;
        let new = seed(&store, "new", "disputed", &[old], 1).await;
        let gone = seed(&store, "gone", "active", &[], 2).await;
        let orphan = seed(&store, "orphan", "disputed", &[gone], 3).await;
        soft_delete(&store, gone).await;

        for (axis, id) in [(1, new), (3, orphan)] {
            let hits = search_all_memories(&store, &basis(axis), None, "agent-d", SearchScope::all_default(), 10)
                .await
                .unwrap();
            assert_eq!(hits.first().map(|h| h.id), Some(id), "{hits:?}");
        }

        resolve_dispute_core(&store, new, old, Outcome::Supersede, None).await.unwrap();
        let now = TimestampMs::now();
        store.conn.call(move |c| settle_disputed(c, now)).await.unwrap();

        assert_eq!(row(&store, old).await.status, "superseded");
        assert_eq!(row(&store, new).await.status, "active");
        assert_eq!(row(&store, orphan).await.status, "active");
    }

    // ── verdicts ────────────────────────────────────────────────────────────

    async fn seed_typed(store: &MemoryStore, memory_type: &str, status: &str, disputes: &[i64], axis: usize) -> i64 {
        let id = seed(store, &format!("claim on axis {axis}"), status, disputes, axis).await;
        let memory_type = memory_type.to_string();
        store
            .conn
            .call(move |c| c.execute("UPDATE memory SET memory_type = ?2 WHERE id = ?1", rusqlite::params![id, memory_type]))
            .await
            .unwrap();
        id
    }

    async fn note_pair(store: &MemoryStore, new: i64, old: i64, state: &str) {
        let state = state.to_string();
        store
            .conn
            .call(move |c| {
                c.execute(
                    "INSERT INTO memory_conflict (new_id, old_id, distance, state, created_at) VALUES (?1, ?2, 0.3, ?3, 1000)",
                    rusqlite::params![new, old, state],
                )
            })
            .await
            .unwrap();
    }

    #[derive(Debug, PartialEq)]
    struct Pair {
        state: String,
        verdict: Option<String>,
        reason: Option<String>,
        judged: bool,
    }

    async fn pair_of(store: &MemoryStore, new: i64, old: i64) -> Pair {
        store
            .conn
            .call(move |c| {
                c.query_row(
                    "SELECT state, verdict, reason, judged_at IS NOT NULL FROM memory_conflict \
                     WHERE new_id = ?1 AND old_id = ?2",
                    [new, old],
                    |r| Ok(Pair { state: r.get(0)?, verdict: r.get(1)?, reason: r.get(2)?, judged: r.get(3)? }),
                )
            })
            .await
            .unwrap()
    }

    fn judged(verdict: &str, reason: Option<&str>) -> Pair {
        Pair { state: "judged".into(), verdict: Some(verdict.into()), reason: reason.map(str::to_owned), judged: true }
    }

    #[tokio::test]
    async fn supersedes_keeps_the_superseding_effect_and_judges_the_pair() {
        let store = store().await;
        let old = seed(&store, "the API listens on 8080", "active", &[], 0).await;
        let new = seed(&store, "the API listens on 9090", "disputed", &[old], 1).await;
        note_pair(&store, new, old, "escalated").await;

        let resolution = judge_pair(&store, new, old, Verdict::Supersedes, None, false, None).await.unwrap();

        assert_eq!(resolution.other, ResolvedRow { id: old, status: "superseded".into(), superseded_by: Some(new) });
        assert_eq!(resolution.pair.as_deref(), Some("judged"));
        assert_eq!(pair_of(&store, new, old).await, judged("supersedes", None));

        let undone = judge_pair(&store, old, new, Verdict::Compatible, None, false, None).await.unwrap();
        assert_eq!((undone.keep.status.as_str(), undone.other.status.as_str()), ("active", "active"));
        assert_eq!(pair_of(&store, new, old).await, judged("compatible", None));
    }

    #[tokio::test]
    async fn compatible_and_scoped_close_without_touching_the_ranking() {
        let store = store().await;
        let old = seed(&store, "old", "active", &[], 0).await;
        let a = seed(&store, "a", "active", &[], 1).await;
        let b = seed(&store, "b", "active", &[], 2).await;
        note_pair(&store, a, old, "pending").await;
        note_pair(&store, b, old, "pending").await;

        judge_pair(&store, a, old, Verdict::Compatible, None, false, None).await.unwrap();
        judge_pair(&store, old, b, Verdict::Scoped, Some("one is the dev profile".into()), false, None).await.unwrap();

        assert_eq!(pair_of(&store, a, old).await, judged("compatible", None));
        assert_eq!(pair_of(&store, b, old).await, judged("scoped", Some("one is the dev profile")));
        for id in [old, a, b] {
            assert_eq!(row(&store, id).await.status, "active");
        }
        let hits = search_all_memories(&store, &basis(0), None, "agent-d", SearchScope::all_default(), 10)
            .await
            .unwrap();
        assert!(hits.iter().any(|h| h.id == old), "{hits:?}");
    }

    #[tokio::test]
    async fn a_verdict_reason_is_stored_masked() {
        let store = store().await;
        let old = seed(&store, "old", "active", &[], 0).await;
        let new = seed(&store, "new", "active", &[], 1).await;
        note_pair(&store, new, old, "pending").await;
        let key: String = "Q7ZX".chars().cycle().take(16).collect();

        let reason = format!("the staging key AKIA{key} is only for ci");
        judge_pair(&store, new, old, Verdict::Scoped, Some(reason), false, None).await.unwrap();

        let stored = "the staging key [REDACTED:aws_access_key] is only for ci";
        assert_eq!(pair_of(&store, new, old).await, judged("scoped", Some(stored)));
    }

    #[tokio::test]
    async fn scoped_without_a_reason_is_refused() {
        let store = store().await;
        let old = seed(&store, "old", "active", &[], 0).await;
        let new = seed(&store, "new", "active", &[], 1).await;
        note_pair(&store, new, old, "pending").await;

        for reason in [None, Some("  ".to_string())] {
            assert!(matches!(
                judge_pair(&store, new, old, Verdict::Scoped, reason, false, None).await,
                Err(ResolveError::ReasonRequired)
            ));
        }
        assert_eq!(pair_of(&store, new, old).await.state, "pending");
    }

    #[tokio::test]
    async fn conflicts_escalates_a_pending_pair() {
        let store = store().await;
        let old = seed(&store, "old", "active", &[], 0).await;
        let new = seed(&store, "new", "active", &[], 1).await;
        note_pair(&store, new, old, "pending").await;

        let resolution = judge_pair(&store, old, new, Verdict::Conflicts, Some("ports differ".into()), false, None).await.unwrap();

        assert_eq!(resolution.pair.as_deref(), Some("escalated"));
        let newer = row(&store, new).await;
        assert_eq!((newer.status.as_str(), newer.disputes), ("disputed", Some(format!("[{old}]"))));
        let pair = pair_of(&store, new, old).await;
        assert_eq!((pair.state.as_str(), pair.verdict, pair.judged), ("escalated", None, false));

        let again = judge_pair(&store, new, old, Verdict::Conflicts, None, false, None).await.unwrap();
        assert_eq!(row(&store, new).await.disputes, Some(format!("[{old}]")), "named once");
        assert_eq!(again.keep.status, "disputed");
    }

    async fn supersede_undeclared(store: &MemoryStore, keep: i64, other: i64) -> Result<Resolution, ResolveError> {
        judge_pair(store, keep, other, Verdict::Supersedes, None, false, None).await
    }

    #[tokio::test]
    async fn a_protected_row_is_not_superseded_after_conflicts() {
        let store = store().await;
        let old = seed_typed(&store, "user_preference", "active", &[], 0).await;
        let new = seed_typed(&store, "semantic_fact", "active", &[], 1).await;
        note_pair(&store, new, old, "pending").await;

        assert!(matches!(
            supersede_undeclared(&store, new, old).await,
            Err(ResolveError::NeedsUser { other, .. }) if other == old
        ));
        assert_eq!(row(&store, old).await.status, "active", "a refused verdict changes nothing");

        judge_pair(&store, new, old, Verdict::Conflicts, None, false, None).await.unwrap();
        assert!(matches!(supersede_undeclared(&store, new, old).await, Err(ResolveError::NeedsUser { .. })));
        assert_eq!(row(&store, old).await.status, "active");
        assert_eq!(pair_of(&store, new, old).await.state, "escalated");
    }

    #[tokio::test]
    async fn a_pair_escalated_at_save_does_not_open_the_guard() {
        let store = store().await;
        let old = seed_typed(&store, "architectural_decision", "active", &[], 0).await;
        let new = seed_typed(&store, "semantic_fact", "disputed", &[old], 1).await;
        note_pair(&store, new, old, "escalated").await;

        assert!(matches!(supersede_undeclared(&store, new, old).await, Err(ResolveError::NeedsUser { .. })));
        assert_eq!(row(&store, old).await.status, "active");
        assert_eq!(row(&store, new).await.status, "disputed");
        assert_eq!(pair_of(&store, new, old).await.state, "escalated");
    }

    #[tokio::test]
    async fn a_judged_pair_does_not_open_the_guard() {
        let store = store().await;
        let old = seed_typed(&store, "user_preference", "active", &[], 0).await;
        let new = seed_typed(&store, "semantic_fact", "active", &[], 1).await;
        note_pair(&store, new, old, "pending").await;
        judge_pair(&store, new, old, Verdict::Compatible, None, false, None).await.unwrap();

        assert!(matches!(supersede_undeclared(&store, new, old).await, Err(ResolveError::NeedsUser { .. })));
        assert_eq!(row(&store, old).await.status, "active");
        assert_eq!(pair_of(&store, new, old).await, judged("compatible", None));
    }

    #[tokio::test]
    async fn a_pair_without_a_row_does_not_open_the_guard() {
        let store = store().await;
        let old = seed_typed(&store, "user_preference", "active", &[], 0).await;
        let new = seed_typed(&store, "semantic_fact", "disputed", &[old], 1).await;

        assert!(matches!(supersede_undeclared(&store, new, old).await, Err(ResolveError::NeedsUser { .. })));
        assert_eq!(row(&store, old).await.status, "active");
        assert_eq!(row(&store, new).await.disputes, Some(format!("[{old}]")));
    }

    #[tokio::test]
    async fn a_protected_keep_needs_no_answer() {
        let store = store().await;
        let old = seed_typed(&store, "user_preference", "active", &[], 0).await;
        let new = seed_typed(&store, "semantic_fact", "active", &[], 1).await;
        note_pair(&store, new, old, "escalated").await;
        judge_pair(&store, new, old, Verdict::Supersedes, Some("tabs now".into()), true, None).await.unwrap();

        let undone = supersede_undeclared(&store, old, new).await.unwrap();

        assert_eq!(undone.keep, ResolvedRow { id: old, status: "active".into(), superseded_by: None });
        assert_eq!(undone.other, ResolvedRow { id: new, status: "superseded".into(), superseded_by: Some(old) });

        let fact = seed_typed(&store, "semantic_fact", "active", &[], 2).await;
        let decision = seed_typed(&store, "architectural_decision", "active", &[], 3).await;
        note_pair(&store, decision, fact, "pending").await;
        assert!(supersede_undeclared(&store, decision, fact).await.is_ok(), "only the row that loses is guarded");
    }

    #[tokio::test]
    async fn a_declared_answer_supersedes_and_is_recorded() {
        let store = store().await;
        let old = seed_typed(&store, "user_preference", "active", &[], 0).await;
        let new = seed_typed(&store, "user_preference", "disputed", &[old], 1).await;
        note_pair(&store, new, old, "escalated").await;

        let reason = Some("the user now prefers tabs".to_string());
        let resolution = judge_pair(&store, new, old, Verdict::Supersedes, reason, true, None).await.unwrap();

        assert_eq!(resolution.other, ResolvedRow { id: old, status: "superseded".into(), superseded_by: Some(new) });
        assert_eq!(pair_of(&store, new, old).await, judged("supersedes", Some("user answered: the user now prefers tabs")));
    }

    #[tokio::test]
    async fn a_declared_answer_needs_a_reason() {
        let store = store().await;
        let old = seed_typed(&store, "user_preference", "active", &[], 0).await;
        let new = seed_typed(&store, "semantic_fact", "active", &[], 1).await;
        note_pair(&store, new, old, "escalated").await;

        for reason in [None, Some(" ".to_string())] {
            assert!(matches!(
                judge_pair(&store, new, old, Verdict::Supersedes, reason, true, None).await,
                Err(ResolveError::AnswerRequired)
            ));
        }
        assert_eq!(row(&store, old).await.status, "active");
        assert_eq!(pair_of(&store, new, old).await.state, "escalated");
    }

    #[tokio::test]
    async fn a_pending_pair_links_the_rows() {
        let store = store().await;
        let old = seed(&store, "old", "active", &[], 0).await;
        let new = seed(&store, "new", "active", &[], 1).await;
        assert!(matches!(
            judge_pair(&store, new, old, Verdict::Compatible, None, false, None).await,
            Err(ResolveError::NotInDispute { .. })
        ));

        note_pair(&store, new, old, "pending").await;

        assert!(judge_pair(&store, new, old, Verdict::Compatible, None, false, None).await.is_ok());
    }

    #[tokio::test]
    async fn the_old_resolve_records_its_verdict() {
        let store = store().await;
        let old = seed(&store, "old", "active", &[], 0).await;
        let new = seed(&store, "new", "disputed", &[old], 1).await;

        resolve_dispute_core(&store, new, old, Outcome::KeepBoth, None).await.unwrap();

        assert_eq!(pair_of(&store, new, old).await, judged("compatible", None));
    }

    #[tokio::test]
    async fn open_pairs_count_and_list_the_live_pending_ones() {
        let store = store().await;
        let old = seed(&store, "old", "active", &[], 0).await;
        let a = seed(&store, "a", "active", &[], 1).await;
        let b = seed(&store, "b", "disputed", &[old], 2).await;
        let gone = seed(&store, "gone", "active", &[], 3).await;
        note_pair(&store, a, old, "pending").await;
        note_pair(&store, b, old, "escalated").await;
        note_pair(&store, gone, old, "pending").await;
        soft_delete(&store, gone).await;

        let open = open_pairs(&store, Some(&reader("agent-d")), None, 5).await.unwrap();

        assert_eq!((open.pending, open.escalated), (1, 1));
        assert_eq!(open.pairs.iter().map(|p| (p.new_id, p.old_id)).collect::<Vec<_>>(), vec![(a, old)]);
    }

    #[tokio::test]
    async fn open_pairs_count_by_project_and_reach() {
        let store = store().await;
        let old = seed(&store, "old", "active", &[], 0).await;
        let here = seed(&store, "here", "active", &[], 1).await;
        let there = seed(&store, "there", "active", &[], 2).await;
        note_pair(&store, here, old, "pending").await;
        note_pair(&store, there, old, "pending").await;
        store
            .conn
            .call(move |c| {
                c.execute("UPDATE memory SET project_id = 'p' WHERE id = ?1", [here])?;
                c.execute("UPDATE memory SET project_id = 'q' WHERE id = ?1", [there])
            })
            .await
            .unwrap();

        let another = reader("agent-x");
        assert_eq!(open_pairs(&store, Some(&another), None, 5).await.unwrap().pending, 2, "another agent judges them too");
        let in_p = open_pairs(&store, Some(&another), Some("p"), 5).await.unwrap();
        assert_eq!(in_p.pairs.iter().map(|p| p.new_id).collect::<Vec<_>>(), vec![here], "only the project's");
        let hiding = Reader::new("agent-x", Reach::AllBut(vec!["agent-d".into()]));
        assert_eq!(open_pairs(&store, Some(&hiding), None, 5).await.unwrap().pending, 0, "a pair it cannot read is not counted");
    }

    #[tokio::test]
    async fn open_pairs_carry_each_sides_mark() {
        let store = store().await;
        let old = seed(&store, "use Postgres for the ledger", "active", &[], 0).await;
        let new = seed(&store, "the ledger seems to use SQLite", "active", &[], 1).await;
        store
            .conn
            .call(move |c| c.execute("UPDATE memory SET created_by_type = 'agent' WHERE id = ?1", [new]))
            .await
            .unwrap();
        note_pair(&store, new, old, "pending").await;

        let open = open_pairs(&store, Some(&reader("agent-d")), None, 5).await.unwrap();

        let pair = &open.pairs[0];
        assert_eq!((pair.new_mark.as_deref(), pair.old_mark.as_deref()), (Some("inferred"), None));
        assert_eq!(pair_of(&store, new, old).await.state, "pending", "the origin judges nothing");
    }

    #[test]
    fn every_verdict_parses_back_to_itself() {
        for v in Verdict::ALL {
            assert_eq!(v.as_str().parse::<Verdict>().unwrap(), v);
        }
        let err = "keep_both".parse::<Verdict>().unwrap_err().to_string();
        assert!(err.contains("supersedes, compatible, scoped, conflicts"), "{err}");
    }

    #[tokio::test]
    async fn a_malformed_dispute_list_names_nothing() {
        let store = store().await;
        let old = seed(&store, "old", "active", &[], 0).await;
        let new = seed(&store, "new", "disputed", &[old], 1).await;
        store
            .conn
            .call(move |c| c.execute("UPDATE memory SET disputed_against = 'not json' WHERE id = ?1", [new]))
            .await
            .unwrap();

        let now = TimestampMs::now();
        let released = store.conn.call(move |c| release_challengers(c, old, now)).await.unwrap();

        assert_eq!(released, 0);
    }
}
