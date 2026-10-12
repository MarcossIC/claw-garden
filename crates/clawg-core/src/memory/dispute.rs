//! Closing a dispute the contradiction detector opened.
//!
//! The detector marks the newer row `disputed` and names the rows it
//! contradicts in `disputed_against`. Closing a dispute either supersedes one
//! side (`superseded_by` points at the row that stays) or keeps both, and in
//! either case drops each side from the other's `disputed_against`, so that
//! list always names the disputes still open.
//!
//! A row that is not superseded never has its status written directly: every
//! path settles it with [`REST_STATUS`], which reads it off the rows its
//! `disputed_against` still names. That keeps resolving, deleting and the
//! maintenance pass on one rule.

use rusqlite::Connection;
use serde::Serialize;

use crate::memory::clock::TimestampMs;
use crate::memory::{MemoryError, MemoryStore};

/// How a dispute between two rows is closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The kept row stands and the other is superseded by it.
    Supersede,
    /// Both rows stand: the detector paired two claims that do not conflict.
    KeepBoth,
}

#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    #[error("a memory cannot be resolved against itself")]
    SameRow,
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedRow {
    pub id: i64,
    pub status: String,
    pub superseded_by: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Resolution {
    pub keep: ResolvedRow,
    pub other: ResolvedRow,
}

/// What deciding whether two rows can be resolved needs from each of them.
#[derive(Debug, Clone, PartialEq)]
struct PairRow {
    id: i64,
    status: String,
    superseded_by: Option<i64>,
    disputes: Vec<i64>,
}

impl PairRow {
    fn replaced_by(&self) -> Option<i64> {
        (self.status == "superseded").then_some(self.superseded_by).flatten()
    }
}

/// Two rows can be resolved when one names the other in `disputed_against`,
/// or one was superseded by the other — the second is what lets a resolution
/// be undone. A row superseded by a third row is refused: rewriting its
/// `superseded_by` would silently drop that earlier decision.
fn check_pair(keep: &PairRow, other: &PairRow) -> Result<(), ResolveError> {
    for (row, partner) in [(keep, other), (other, keep)] {
        if let Some(by) = row.replaced_by().filter(|by| *by != partner.id) {
            return Err(ResolveError::SupersededElsewhere { id: row.id, by });
        }
    }
    let linked = keep.disputes.contains(&other.id)
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

/// Settles every `disputed` row. A delete that ran before deletes released
/// their challengers left those rows `disputed` with nothing to be resolved
/// against; this is the backfill for them.
pub(crate) fn settle_disputed(c: &Connection, now: TimestampMs) -> rusqlite::Result<usize> {
    c.execute(&settle_sql("status = 'disputed'"), rusqlite::params![now])
}

const SELECT_PAIR_ROW: &str =
    "SELECT id, status, superseded_by, disputed_against FROM memory WHERE id = ?1 AND deleted_at IS NULL";

fn read_pair_row(c: &Connection, id: i64) -> Result<PairRow, ResolveError> {
    use rusqlite::OptionalExtension;
    let row = c
        .query_row(SELECT_PAIR_ROW, [id], |r| {
            Ok(PairRow {
                id: r.get(0)?,
                status: r.get(1)?,
                superseded_by: r.get(2)?,
                disputes: r
                    .get::<_, Option<String>>(3)?
                    .and_then(|s| serde_json::from_str(&s).ok())
                    .unwrap_or_default(),
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
const REVIVE_SQL: &str = "UPDATE memory SET status = 'active', superseded_by = NULL, last_modified_at = ?3 \
                          WHERE id = ?1 AND status = 'superseded' AND superseded_by = ?2";

const SUPERSEDE_SQL: &str = "UPDATE memory SET status = 'superseded', superseded_by = ?2, last_modified_at = ?3 \
                             WHERE id = ?1";

/// Closes the dispute between `keep` and `other` in one transaction, and
/// returns both rows as they stand afterwards.
pub async fn resolve_dispute_core(
    store: &MemoryStore,
    keep: i64,
    other: i64,
    outcome: Outcome,
) -> Result<Resolution, ResolveError> {
    if keep == other {
        return Err(ResolveError::SameRow);
    }
    let now = TimestampMs::now();
    store
        .conn
        .call(move |c| -> Result<Resolution, ResolveError> {
            let tx = c.transaction()?;
            check_pair(&read_pair_row(&tx, keep)?, &read_pair_row(&tx, other)?)?;

            let drop_dispute = drop_dispute_sql();
            tx.execute(&drop_dispute, rusqlite::params![keep, other, now])?;
            tx.execute(&drop_dispute, rusqlite::params![other, keep, now])?;
            tx.execute(REVIVE_SQL, rusqlite::params![keep, other, now])?;
            match outcome {
                Outcome::Supersede => {
                    tx.execute(SUPERSEDE_SQL, rusqlite::params![other, keep, now])?;
                    release_challengers(&tx, other, now)?;
                }
                Outcome::KeepBoth => {
                    tx.execute(REVIVE_SQL, rusqlite::params![other, keep, now])?;
                }
            }
            let settle_one = settle_sql("id = ?2");
            tx.execute(&settle_one, rusqlite::params![now, keep])?;
            tx.execute(&settle_one, rusqlite::params![now, other])?;

            let resolution = Resolution { keep: read_resolved(&tx, keep)?, other: read_resolved(&tx, other)? };
            tx.commit()?;
            Ok(resolution)
        })
        .await
        .map_err(|e| match e {
            tokio_rusqlite::Error::Error(e) => e,
            tokio_rusqlite::Error::ConnectionClosed => ResolveError::Store(MemoryError::Backend(
                "connection closed during resolve_dispute_core".into(),
            )),
            other => ResolveError::Store(MemoryError::Backend(other.to_string())),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::admin::{list_memories_core, ListParams, MemoryKind};
    use crate::memory::search::{search_all_memories, SearchScope};

    fn pair_row(id: i64, status: &str, superseded_by: Option<i64>, disputes: &[i64]) -> PairRow {
        PairRow { id, status: status.into(), superseded_by, disputes: disputes.to_vec() }
    }

    // ── check_pair ──────────────────────────────────────────────────────────

    #[test]
    fn a_dispute_in_either_direction_links_the_pair() {
        let old = pair_row(1, "active", None, &[]);
        let new = pair_row(2, "disputed", None, &[1]);
        assert!(check_pair(&new, &old).is_ok());
        assert!(check_pair(&old, &new).is_ok());
    }

    #[test]
    fn a_superseded_link_in_either_direction_links_the_pair() {
        let winner = pair_row(1, "active", None, &[]);
        let loser = pair_row(2, "superseded", Some(1), &[]);
        assert!(check_pair(&winner, &loser).is_ok());
        assert!(check_pair(&loser, &winner).is_ok());
    }

    #[test]
    fn unrelated_rows_are_not_linked() {
        let a = pair_row(1, "active", None, &[]);
        let b = pair_row(2, "disputed", None, &[3]);
        assert!(matches!(check_pair(&a, &b), Err(ResolveError::NotInDispute { keep: 1, other: 2 })));
    }

    #[test]
    fn a_row_superseded_by_a_third_row_is_refused() {
        let a = pair_row(1, "superseded", Some(9), &[2]);
        let b = pair_row(2, "active", None, &[]);
        assert!(matches!(check_pair(&a, &b), Err(ResolveError::SupersededElsewhere { id: 1, by: 9 })));
        assert!(matches!(check_pair(&b, &a), Err(ResolveError::SupersededElsewhere { id: 1, by: 9 })));
    }

    #[test]
    fn a_stale_superseded_by_on_a_live_row_does_not_link() {
        let a = pair_row(1, "active", Some(2), &[]);
        let b = pair_row(2, "active", None, &[]);
        assert!(matches!(check_pair(&a, &b), Err(ResolveError::NotInDispute { .. })));
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

        let resolution = resolve_dispute_core(&store, new, old, Outcome::Supersede).await.unwrap();

        assert_eq!(resolution.keep, ResolvedRow { id: new, status: "active".into(), superseded_by: None });
        assert_eq!(resolution.other, ResolvedRow { id: old, status: "superseded".into(), superseded_by: Some(new) });
        let kept = row(&store, new).await;
        assert_eq!(kept.disputes, None);
        assert_eq!(kept.last_disputed_at, Some(1000), "the record that a dispute happened stays");
    }

    #[tokio::test]
    async fn the_old_row_can_win() {
        let store = store().await;
        let old = seed(&store, "old", "active", &[], 0).await;
        let new = seed(&store, "new", "disputed", &[old], 1).await;

        let resolution = resolve_dispute_core(&store, old, new, Outcome::Supersede).await.unwrap();

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

        let resolution = resolve_dispute_core(&store, new, first, Outcome::Supersede).await.unwrap();

        assert_eq!(resolution.keep.status, "disputed");
        assert_eq!(row(&store, new).await.disputes, Some(format!("[{second}]")));

        let resolution = resolve_dispute_core(&store, new, second, Outcome::Supersede).await.unwrap();
        assert_eq!(resolution.keep.status, "active");
    }

    #[tokio::test]
    async fn superseding_a_row_releases_the_rows_that_disputed_it() {
        let store = store().await;
        let old = seed(&store, "old", "active", &[], 0).await;
        let new = seed(&store, "new", "disputed", &[old], 1).await;
        let other_challenger = seed(&store, "another", "disputed", &[old], 2).await;

        resolve_dispute_core(&store, new, old, Outcome::Supersede).await.unwrap();

        assert_eq!(row(&store, other_challenger).await.status, "active");
    }

    #[tokio::test]
    async fn a_superseded_row_leaves_search_but_stays_listed() {
        let store = store().await;
        let old = seed(&store, "old", "active", &[], 0).await;
        let new = seed(&store, "new", "disputed", &[old], 1).await;
        resolve_dispute_core(&store, new, old, Outcome::Supersede).await.unwrap();

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

        let resolution = resolve_dispute_core(&store, new, old, Outcome::KeepBoth).await.unwrap();

        assert_eq!(resolution.keep, ResolvedRow { id: new, status: "active".into(), superseded_by: None });
        assert_eq!(resolution.other, ResolvedRow { id: old, status: "active".into(), superseded_by: None });
        assert_eq!(row(&store, new).await.disputes, None);
    }

    #[tokio::test]
    async fn resolving_backwards_undoes_a_resolution() {
        let store = store().await;
        let old = seed(&store, "old", "active", &[], 0).await;
        let new = seed(&store, "new", "disputed", &[old], 1).await;
        resolve_dispute_core(&store, new, old, Outcome::Supersede).await.unwrap();

        let resolution = resolve_dispute_core(&store, old, new, Outcome::Supersede).await.unwrap();
        assert_eq!(resolution.keep, ResolvedRow { id: old, status: "active".into(), superseded_by: None });
        assert_eq!(resolution.other, ResolvedRow { id: new, status: "superseded".into(), superseded_by: Some(old) });

        let resolution = resolve_dispute_core(&store, new, old, Outcome::KeepBoth).await.unwrap();
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
        resolve_dispute_core(&store, first, new, Outcome::Supersede).await.unwrap();

        let resolution = resolve_dispute_core(&store, new, first, Outcome::KeepBoth).await.unwrap();

        assert_eq!(resolution.keep.status, "disputed", "the dispute with `second` was never closed");
    }

    #[tokio::test]
    async fn resolving_twice_is_the_same_as_once() {
        let store = store().await;
        let old = seed(&store, "old", "active", &[], 0).await;
        let new = seed(&store, "new", "disputed", &[old], 1).await;

        let once = resolve_dispute_core(&store, new, old, Outcome::Supersede).await.unwrap();
        let twice = resolve_dispute_core(&store, new, old, Outcome::Supersede).await.unwrap();

        assert_eq!(once, twice);
    }

    #[tokio::test]
    async fn a_row_cannot_be_resolved_against_itself() {
        let store = store().await;
        let id = seed(&store, "x", "disputed", &[], 0).await;
        assert!(matches!(
            resolve_dispute_core(&store, id, id, Outcome::Supersede).await,
            Err(ResolveError::SameRow)
        ));
    }

    #[tokio::test]
    async fn a_missing_or_deleted_row_is_not_found() {
        let store = store().await;
        let old = seed(&store, "old", "active", &[], 0).await;
        let new = seed(&store, "new", "disputed", &[old], 1).await;

        assert!(matches!(
            resolve_dispute_core(&store, new, 999, Outcome::Supersede).await,
            Err(ResolveError::NotFound(999))
        ));
        soft_delete(&store, old).await;
        assert!(matches!(
            resolve_dispute_core(&store, new, old, Outcome::Supersede).await,
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
            resolve_dispute_core(&store, a, b, Outcome::Supersede).await,
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

        resolve_dispute_core(&store, new, old, Outcome::Supersede).await.unwrap();
        let now = TimestampMs::now();
        store.conn.call(move |c| settle_disputed(c, now)).await.unwrap();

        assert_eq!(row(&store, old).await.status, "superseded");
        assert_eq!(row(&store, new).await.status, "active");
        assert_eq!(row(&store, orphan).await.status, "active");
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
