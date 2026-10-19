//! What an agent reads to draw conclusions from its memories.
//!
//! A conclusion is an ordinary `memory` row linked in `memory_derivation` to
//! the rows it was drawn from. Nothing here writes one: the agent does, with an
//! explicit save that names its sources. This module answers the two questions
//! around that save: which memories no live conclusion covers yet, and how many
//! of them arrived since the agent last looked.
//!
//! A scope is one project, or the whole store when no project is given. Rows
//! without a project belong only to the whole store. Within a scope an agent
//! sees only its own rows, as in search; without an agent the caller is the
//! store owner and sees every agent's.

use serde::Serialize;

use crate::memory::admin::{map_memory_row, MemoryAdminRow, SELECT_MEMORY};
use crate::memory::clock::TimestampMs;
use crate::memory::{MemoryError, MemoryStore};

/// Pending memories returned when the caller does not say how many.
pub const DEFAULT_PENDING_LIMIT: u32 = 30;
/// Most pending memories one call returns.
pub const MAX_PENDING_LIMIT: u32 = 100;
/// Existing conclusions returned with them, newest first: enough for the agent
/// to see what it already concluded without crowding out the memories.
pub const CONCLUSIONS_LIMIT: u32 = 20;

/// A live `memory` row that is not a conclusion and that no live conclusion
/// of the same agent was drawn from. A deleted or superseded conclusion stops
/// covering its sources, so they become pending again; one by another agent
/// never covered them, since this agent cannot see it.
const PENDING: &str = "deleted_at IS NULL AND status <> 'superseded' \
     AND NOT EXISTS (SELECT 1 FROM memory_derivation d WHERE d.memory_id = memory.id) \
     AND NOT EXISTS (SELECT 1 FROM memory_derivation d JOIN memory c ON c.id = d.memory_id \
                     WHERE d.source_id = memory.id AND c.agent_id IS memory.agent_id \
                       AND c.deleted_at IS NULL AND c.status <> 'superseded')";

const CONCLUSION: &str = "deleted_at IS NULL AND status <> 'superseded' \
     AND EXISTS (SELECT 1 FROM memory_derivation d WHERE d.memory_id = memory.id)";

/// The source `d.source_id` is lost: no live row holds it any more, because
/// it was deleted, replaced or purged. The SQL side of [`SourceState::of`].
const LOST_SOURCE: &str = "NOT EXISTS (SELECT 1 FROM memory s WHERE s.id = d.source_id \
     AND s.deleted_at IS NULL AND s.status <> 'superseded')";

/// Conclusions to review returned at most: each line names what it lost, so
/// they are longer than the conclusions listed beside them.
pub const REVIEW_LIMIT: u32 = 10;

fn to_review_condition() -> String {
    format!(
        "{CONCLUSION} AND EXISTS (SELECT 1 FROM memory_derivation d \
         WHERE d.memory_id = memory.id AND {LOST_SOURCE})"
    )
}

/// What became of one source of a conclusion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceState {
    /// Still live: `active`, or `disputed`, which says nothing about whether
    /// it is true.
    Live,
    Deleted,
    Superseded,
    /// Its row is gone; the link names an id no row will hold again.
    Purged,
}

impl SourceState {
    /// From the source's row as `(deleted, status)`, `None` when it has none.
    pub fn of(row: Option<(bool, &str)>) -> Self {
        match row {
            None => Self::Purged,
            Some((true, _)) => Self::Deleted,
            Some((false, "superseded")) => Self::Superseded,
            Some(_) => Self::Live,
        }
    }
}

/// A conclusion is to review once one source is lost: the user deleting or
/// replacing a memory is a statement about it, and only whoever reads the
/// conclusion can tell whether what is left still carries it.
pub fn needs_review(sources: &[SourceState]) -> bool {
    sources.iter().any(|s| *s != SourceState::Live)
}

#[derive(Debug, Clone, Serialize)]
pub struct LostSource {
    pub id: i64,
    pub state: SourceState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<i64>,
}

/// A live conclusion with a lost source.
#[derive(Debug, Clone, Serialize)]
pub struct ConclusionReview {
    pub conclusion: MemoryAdminRow,
    /// Sources still live, of the `derived_from` of the conclusion.
    pub live: usize,
    pub lost: Vec<LostSource>,
}

#[derive(Debug, Clone, Default)]
pub struct SynthesisParams {
    /// `None` is the store owner, who reviews every agent's memories.
    pub agent_id: Option<String>,
    /// `None` reviews the whole store.
    pub project_id: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Synthesis {
    /// Newest first, at most the requested limit.
    pub pending: Vec<MemoryAdminRow>,
    /// Every pending memory of the scope.
    pub pending_total: usize,
    pub conclusions: Vec<MemoryAdminRow>,
    /// Newest first, at most [`REVIEW_LIMIT`].
    pub to_review: Vec<ConclusionReview>,
    pub to_review_total: usize,
}

#[derive(Debug, Clone, Copy)]
struct Scope<'a> {
    agent: Option<&'a str>,
    project: Option<&'a str>,
}

impl Scope<'_> {
    /// The `synthesis_review` key of the scope.
    fn key(&self) -> (&str, &str) {
        (self.agent.unwrap_or(""), self.project.unwrap_or(""))
    }

    /// The scope filter, appended to a `WHERE` that already holds one
    /// condition. Agent and project are bound parameters.
    fn push(&self, sql: &mut String, params: &mut Vec<rusqlite::types::Value>) {
        if let Some(a) = self.agent {
            sql.push_str(" AND agent_id = ?");
            params.push(rusqlite::types::Value::Text(a.to_string()));
        }
        if let Some(p) = self.project {
            sql.push_str(" AND project_id = ?");
            params.push(rusqlite::types::Value::Text(p.to_string()));
        }
    }
}

fn rows(
    c: &rusqlite::Connection,
    condition: &str,
    scope: Scope<'_>,
    limit: u32,
) -> rusqlite::Result<Vec<MemoryAdminRow>> {
    let mut sql = format!("{SELECT_MEMORY} WHERE {condition}");
    let mut params = Vec::new();
    scope.push(&mut sql, &mut params);
    sql.push_str(" ORDER BY created_at DESC, id DESC LIMIT ?");
    params.push(rusqlite::types::Value::Integer(limit.into()));
    let mut stmt = c.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(params), map_memory_row)?;
    rows.collect()
}

fn count(c: &rusqlite::Connection, condition: &str, scope: Scope<'_>, after: Option<i64>) -> rusqlite::Result<usize> {
    let mut sql = format!("SELECT COUNT(*) FROM memory WHERE {condition}");
    let mut params = Vec::new();
    scope.push(&mut sql, &mut params);
    if let Some(after) = after {
        sql.push_str(" AND created_at > ?");
        params.push(rusqlite::types::Value::Integer(after));
    }
    let n: i64 = c.query_row(&sql, rusqlite::params_from_iter(params), |r| r.get(0))?;
    Ok(n as usize)
}

/// Each conclusion of `conclusions` with what became of its sources.
fn reviews(c: &rusqlite::Connection, conclusions: Vec<MemoryAdminRow>) -> rusqlite::Result<Vec<ConclusionReview>> {
    let mut stmt = c.prepare_cached(
        "SELECT d.source_id, s.id IS NOT NULL, s.deleted_at IS NOT NULL, s.status, s.superseded_by \
         FROM memory_derivation d LEFT JOIN memory s ON s.id = d.source_id \
         WHERE d.memory_id = ?1 ORDER BY d.source_id",
    )?;
    let mut found = Vec::new();
    for conclusion in conclusions {
        let sources = stmt
            .query_map([conclusion.id], |r| {
                let exists: bool = r.get(1)?;
                let deleted: bool = r.get(2)?;
                let status: Option<String> = r.get(3)?;
                let state = SourceState::of(exists.then_some((deleted, status.as_deref().unwrap_or(""))));
                let superseded_by = if state == SourceState::Superseded { r.get(4)? } else { None };
                Ok(LostSource { id: r.get(0)?, state, superseded_by })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let states: Vec<SourceState> = sources.iter().map(|s| s.state).collect();
        // `LOST_SOURCE` picked the conclusion; the rule has the last word, so
        // the two can never list one the other would not.
        if !needs_review(&states) {
            continue;
        }
        let (live, lost): (Vec<_>, Vec<_>) = sources.into_iter().partition(|s| s.state == SourceState::Live);
        found.push(ConclusionReview { conclusion, live: live.len(), lost });
    }
    Ok(found)
}

fn reviewed_at(c: &rusqlite::Connection, scope: Scope<'_>) -> rusqlite::Result<Option<i64>> {
    use rusqlite::OptionalExtension;
    let (agent, project) = scope.key();
    c.query_row(
        "SELECT reviewed_at FROM synthesis_review WHERE agent_id = ?1 AND scope_key = ?2",
        [agent, project],
        |r| r.get(0),
    )
    .optional()
}

fn backend(context: &'static str) -> impl Fn(tokio_rusqlite::Error) -> MemoryError {
    move |e| match e {
        tokio_rusqlite::Error::Error(re) => MemoryError::Sqlite(re),
        other => MemoryError::Backend(format!("{context}: {other}")),
    }
}

/// The memories of a scope that no conclusion covers yet, with the
/// conclusions already drawn there, and the review recorded as of `now`.
///
/// Reads and records in one transaction, so the total, the list and the
/// review all describe the same state. Writes nothing else: looking at a
/// memory to summarise it is not using it, so no access is recorded.
pub async fn synthesis_core(
    store: &MemoryStore,
    params: SynthesisParams,
    now: TimestampMs,
) -> Result<Synthesis, MemoryError> {
    let limit = params.limit.unwrap_or(DEFAULT_PENDING_LIMIT).clamp(1, MAX_PENDING_LIMIT);
    store
        .conn
        .call(move |c| -> rusqlite::Result<Synthesis> {
            let tx = c.transaction()?;
            let scope = Scope { agent: params.agent_id.as_deref(), project: params.project_id.as_deref() };
            let to_review = to_review_condition();
            let found = Synthesis {
                pending: rows(&tx, PENDING, scope, limit)?,
                pending_total: count(&tx, PENDING, scope, None)?,
                conclusions: rows(&tx, CONCLUSION, scope, CONCLUSIONS_LIMIT)?,
                to_review: reviews(&tx, rows(&tx, &to_review, scope, REVIEW_LIMIT)?)?,
                to_review_total: count(&tx, &to_review, scope, None)?,
            };
            let (agent, project) = scope.key();
            tx.execute(
                "INSERT INTO synthesis_review (agent_id, scope_key, reviewed_at) VALUES (?1, ?2, ?3) \
                 ON CONFLICT (agent_id, scope_key) DO UPDATE SET reviewed_at = excluded.reviewed_at",
                rusqlite::params![agent, project, now],
            )?;
            tx.commit()?;
            Ok(found)
        })
        .await
        .map_err(backend("synthesis"))
}

/// How many pending memories of a scope arrived after its last review by the
/// same agent; every pending memory when the scope was never reviewed.
pub async fn unreviewed_count(
    store: &MemoryStore,
    agent_id: Option<&str>,
    project_id: Option<&str>,
) -> Result<usize, MemoryError> {
    let agent_id = agent_id.map(str::to_owned);
    let project_id = project_id.map(str::to_owned);
    store
        .conn
        .call(move |c| -> rusqlite::Result<usize> {
            let scope = Scope { agent: agent_id.as_deref(), project: project_id.as_deref() };
            let after = reviewed_at(c, scope)?;
            count(c, PENDING, scope, after)
        })
        .await
        .map_err(backend("unreviewed count"))
}

/// The ids among `ids` that name a `memory` row not deleted, ascending. With
/// an agent, another agent's row counts as missing, like an id that is not
/// there.
pub async fn existing_memory_ids(
    store: &MemoryStore,
    ids: &[i64],
    agent_id: Option<&str>,
) -> Result<Vec<i64>, MemoryError> {
    let ids = ids.to_vec();
    let agent_id = agent_id.map(str::to_owned);
    store
        .conn
        .call(move |c| -> rusqlite::Result<Vec<i64>> {
            let mut stmt = c.prepare_cached(
                "SELECT id FROM memory WHERE id = ?1 AND deleted_at IS NULL \
                 AND (?2 IS NULL OR agent_id = ?2)",
            )?;
            let mut found = Vec::new();
            for id in ids {
                if stmt.exists(rusqlite::params![id, agent_id])? {
                    found.push(id);
                }
            }
            found.sort_unstable();
            found.dedup();
            Ok(found)
        })
        .await
        .map_err(backend("source lookup"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::admin::{delete_memory_core, MemoryKind};

    async fn seed(store: &MemoryStore, project: Option<&str>, content: &str, created_at: i64) -> i64 {
        seed_as(store, "a", project, content, created_at).await
    }

    async fn seed_as(store: &MemoryStore, agent: &str, project: Option<&str>, content: &str, created_at: i64) -> i64 {
        let agent = agent.to_owned();
        let project = project.map(str::to_owned);
        let content = content.to_owned();
        store
            .conn
            .call(move |c| -> rusqlite::Result<i64> {
                c.execute(
                    "INSERT INTO memory \
                     (scope, agent_id, project_id, content, memory_type, source, \
                      created_at, created_by_type, created_by_ref, \
                      last_modified_at, last_accessed_at, importance, tier, status) \
                     VALUES ('agent', ?4, ?1, ?2, 'semantic_fact', 'explicit', \
                      ?3, 'user', ?4, ?3, ?3, 5.0, 3, 'active')",
                    rusqlite::params![project, content, created_at, agent],
                )?;
                Ok(c.last_insert_rowid())
            })
            .await
            .unwrap()
    }

    async fn conclude(store: &MemoryStore, project: Option<&str>, sources: &[i64], created_at: i64) -> i64 {
        let id = seed(store, project, "a conclusion", created_at).await;
        let sources = sources.to_vec();
        store
            .conn
            .call(move |c| -> rusqlite::Result<()> {
                for s in sources {
                    c.execute(
                        "INSERT INTO memory_derivation (memory_id, source_id, created_at) VALUES (?1, ?2, 0)",
                        [id, s],
                    )?;
                }
                Ok(())
            })
            .await
            .unwrap();
        id
    }

    fn in_project(p: &str) -> SynthesisParams {
        SynthesisParams { project_id: Some(p.into()), ..Default::default() }
    }

    fn ids(rows: &[MemoryAdminRow]) -> Vec<i64> {
        rows.iter().map(|r| r.id).collect()
    }

    #[tokio::test]
    async fn synthesis_lists_what_no_conclusion_covers() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let a = seed(&store, Some("p"), "tabs in go", 1).await;
        let b = seed(&store, Some("p"), "tabs in rust", 2).await;
        let e = seed(&store, Some("p"), "the deploy runs on fridays", 3).await;
        let c = conclude(&store, Some("p"), &[a, b], 4).await;

        let found = synthesis_core(&store, in_project("p"), TimestampMs::from_unix_millis(10)).await.unwrap();

        assert_eq!(ids(&found.pending), vec![e]);
        assert_eq!(found.pending_total, 1);
        assert_eq!(ids(&found.conclusions), vec![c]);
        assert_eq!(found.conclusions[0].derived_from, Some(vec![a, b]));
    }

    #[tokio::test]
    async fn deleting_a_conclusion_makes_its_sources_pending_again() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let a = seed(&store, Some("p"), "tabs in go", 1).await;
        let b = seed(&store, Some("p"), "tabs in rust", 2).await;
        let e = seed(&store, Some("p"), "the deploy runs on fridays", 3).await;
        let c = conclude(&store, Some("p"), &[a, b], 4).await;

        delete_memory_core(&store, c, MemoryKind::Memory, None).await.unwrap();
        let found = synthesis_core(&store, in_project("p"), TimestampMs::from_unix_millis(10)).await.unwrap();

        assert_eq!(ids(&found.pending), vec![e, b, a]);
        assert!(found.conclusions.is_empty());
        let links: i64 = store
            .conn
            .call(|c| c.query_row("SELECT COUNT(*) FROM memory_derivation", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(links, 2, "deleting a conclusion keeps the record of where it came from");
    }

    #[tokio::test]
    async fn a_restored_conclusion_covers_its_sources_again() {
        use crate::memory::admin::restore_memory_core;
        use crate::memory::embedder::FakeEmbedder;
        let store = MemoryStore::new_for_test().await.unwrap();
        let a = seed(&store, Some("p"), "tabs in go", 1).await;
        let b = seed(&store, Some("p"), "tabs in rust", 2).await;
        let c = conclude(&store, Some("p"), &[a, b], 3).await;
        let lone = seed(&store, Some("p"), "the deploy runs on fridays", 4).await;
        delete_memory_core(&store, c, MemoryKind::Memory, None).await.unwrap();
        delete_memory_core(&store, a, MemoryKind::Memory, None).await.unwrap();
        delete_memory_core(&store, lone, MemoryKind::Memory, None).await.unwrap();

        for id in [c, a, lone] {
            restore_memory_core(&store, &FakeEmbedder::default(), MemoryKind::Memory, id, None).await.unwrap();
        }
        let found = synthesis_core(&store, in_project("p"), TimestampMs::from_unix_millis(10)).await.unwrap();

        assert_eq!(ids(&found.conclusions), vec![c]);
        assert_eq!(found.conclusions[0].derived_from, Some(vec![a, b]));
        assert_eq!(ids(&found.pending), vec![lone], "the restored source is covered, the lone one is pending");
    }

    #[tokio::test]
    async fn synthesis_stays_in_its_project() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let mine = seed(&store, Some("p"), "mine", 1).await;
        let theirs = seed(&store, Some("q"), "theirs", 2).await;
        let nobodys = seed(&store, None, "nobody's", 3).await;

        let here = synthesis_core(&store, in_project("p"), TimestampMs::from_unix_millis(10)).await.unwrap();
        let everywhere =
            synthesis_core(&store, SynthesisParams::default(), TimestampMs::from_unix_millis(10)).await.unwrap();

        assert_eq!(ids(&here.pending), vec![mine]);
        assert_eq!(ids(&everywhere.pending), vec![nobodys, theirs, mine]);
    }

    #[tokio::test]
    async fn another_agents_conclusion_does_not_cover_a_memory() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let mine = seed_as(&store, "a", Some("p"), "mine", 1).await;
        let theirs = seed_as(&store, "b", Some("p"), "their conclusion", 2).await;
        store
            .conn
            .call(move |c| {
                c.execute(
                    "INSERT INTO memory_derivation (memory_id, source_id, created_at) VALUES (?1, ?2, 0)",
                    [theirs, mine],
                )
            })
            .await
            .unwrap();
        let as_a = SynthesisParams { agent_id: Some("a".into()), ..in_project("p") };

        let found = synthesis_core(&store, as_a, TimestampMs::from_unix_millis(10)).await.unwrap();

        assert_eq!(ids(&found.pending), vec![mine], "a conclusion the agent cannot see covers nothing for it");
    }

    #[tokio::test]
    async fn synthesis_records_the_review_and_nothing_else() {
        let store = MemoryStore::new_for_test().await.unwrap();
        seed(&store, Some("p"), "one", 1).await;
        let snapshot = |c: &mut rusqlite::Connection| -> rusqlite::Result<String> {
            c.query_row(
                "SELECT group_concat(json_array(id, status, access_count, last_accessed_at, tier), ';') FROM memory",
                [],
                |r| r.get(0),
            )
        };
        let before = store.conn.call(snapshot).await.unwrap();

        synthesis_core(&store, in_project("p"), TimestampMs::from_unix_millis(10)).await.unwrap();
        synthesis_core(&store, in_project("p"), TimestampMs::from_unix_millis(20)).await.unwrap();

        assert_eq!(store.conn.call(snapshot).await.unwrap(), before);
        let reviews: Vec<(String, String, i64)> = store
            .conn
            .call(|c| {
                let mut stmt = c.prepare("SELECT agent_id, scope_key, reviewed_at FROM synthesis_review")?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect();
                rows
            })
            .await
            .unwrap();
        assert_eq!(reviews, vec![(String::new(), "p".to_string(), 20)]);
    }

    #[tokio::test]
    async fn unreviewed_counts_what_arrived_after_the_last_review() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let a = seed(&store, Some("p"), "one", 1).await;
        seed(&store, Some("p"), "two", 2).await;
        seed(&store, Some("q"), "elsewhere", 3).await;
        assert_eq!(unreviewed_count(&store, None, Some("p")).await.unwrap(), 2, "never reviewed: every pending one");
        assert_eq!(unreviewed_count(&store, None, None).await.unwrap(), 3);

        synthesis_core(&store, in_project("p"), TimestampMs::from_unix_millis(10)).await.unwrap();
        assert_eq!(unreviewed_count(&store, None, Some("p")).await.unwrap(), 0);
        assert_eq!(unreviewed_count(&store, None, None).await.unwrap(), 3, "reviewing one project is not reviewing all");

        seed(&store, Some("p"), "three", 11).await;
        let b = seed(&store, Some("p"), "four", 12).await;
        assert_eq!(unreviewed_count(&store, None, Some("p")).await.unwrap(), 2);

        conclude(&store, Some("p"), &[a, b], 13).await;
        assert_eq!(unreviewed_count(&store, None, Some("p")).await.unwrap(), 1, "a conclusion and its sources are not pending");
    }

    #[tokio::test]
    async fn existing_ids_leave_out_unknown_and_deleted_rows() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let a = seed(&store, None, "kept", 1).await;
        let d = seed(&store, None, "deleted", 2).await;
        delete_memory_core(&store, d, MemoryKind::Memory, None).await.unwrap();

        assert_eq!(existing_memory_ids(&store, &[999_999, d, a, a], None).await.unwrap(), vec![a]);
    }

    #[tokio::test]
    async fn existing_ids_leave_out_another_agents_rows() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let mine = seed_as(&store, "a", None, "mine", 1).await;
        let theirs = seed_as(&store, "b", None, "theirs", 2).await;

        assert_eq!(existing_memory_ids(&store, &[mine, theirs], Some("a")).await.unwrap(), vec![mine]);
        assert_eq!(existing_memory_ids(&store, &[mine, theirs], None).await.unwrap(), vec![mine, theirs]);
    }

    async fn supersede(store: &MemoryStore, id: i64, by: i64, at: i64) {
        store
            .conn
            .call(move |c| {
                c.execute(
                    "UPDATE memory SET status = 'superseded', superseded_by = ?2, \
                     supersede_cause = 'revision', last_modified_at = ?3 WHERE id = ?1",
                    [id, by, at],
                )
            })
            .await
            .unwrap();
    }

    fn lost(review: &ConclusionReview) -> Vec<(i64, SourceState, Option<i64>)> {
        review.lost.iter().map(|l| (l.id, l.state, l.superseded_by)).collect()
    }

    #[test]
    fn source_state_cases() {
        use SourceState::*;
        let cases = [
            (None, Purged),
            (Some((true, "active")), Deleted),
            (Some((true, "superseded")), Deleted),
            (Some((false, "superseded")), Superseded),
            (Some((false, "active")), Live),
            (Some((false, "disputed")), Live),
        ];
        for (row, want) in cases {
            assert_eq!(SourceState::of(row), want, "{row:?}");
        }
    }

    #[test]
    fn review_needs_one_lost_source() {
        use SourceState::*;
        assert!(!needs_review(&[Live, Live]));
        assert!(needs_review(&[Live, Live, Deleted]));
        assert!(needs_review(&[Superseded]));
        assert!(needs_review(&[Purged, Live]));
    }

    #[tokio::test]
    async fn a_conclusion_with_a_deleted_source_is_to_review() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let a = seed(&store, Some("p"), "tabs in go", 1).await;
        let b = seed(&store, Some("p"), "tabs in rust", 2).await;
        let c = conclude(&store, Some("p"), &[a, b], 3).await;

        delete_memory_core(&store, a, MemoryKind::Memory, None).await.unwrap();
        let found = synthesis_core(&store, in_project("p"), TimestampMs::from_unix_millis(10)).await.unwrap();

        assert_eq!(found.to_review_total, 1);
        assert_eq!(found.to_review.len(), 1);
        assert_eq!(found.to_review[0].conclusion.id, c);
        assert_eq!(found.to_review[0].live, 1);
        assert_eq!(lost(&found.to_review[0]), vec![(a, SourceState::Deleted, None)]);
        assert_eq!(ids(&found.conclusions), vec![c], "a conclusion to review is still a live conclusion");
    }

    #[tokio::test]
    async fn a_conclusion_with_every_source_live_is_not() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let a = seed(&store, Some("p"), "tabs in go", 1).await;
        let b = seed(&store, Some("p"), "tabs in rust", 2).await;
        conclude(&store, Some("p"), &[a, b], 3).await;
        store
            .conn
            .call(move |c| c.execute("UPDATE memory SET status = 'disputed' WHERE id = ?1", [a]))
            .await
            .unwrap();

        let found = synthesis_core(&store, in_project("p"), TimestampMs::from_unix_millis(10)).await.unwrap();

        assert!(found.to_review.is_empty(), "a disputed source is still live");
        assert_eq!(found.to_review_total, 0);
    }

    #[tokio::test]
    async fn restoring_the_source_takes_the_conclusion_off_the_list() {
        use crate::memory::admin::restore_memory_core;
        use crate::memory::embedder::FakeEmbedder;
        let store = MemoryStore::new_for_test().await.unwrap();
        let a = seed(&store, Some("p"), "tabs in go", 1).await;
        conclude(&store, Some("p"), &[a], 2).await;
        delete_memory_core(&store, a, MemoryKind::Memory, None).await.unwrap();

        restore_memory_core(&store, &FakeEmbedder::default(), MemoryKind::Memory, a, None).await.unwrap();
        let found = synthesis_core(&store, in_project("p"), TimestampMs::from_unix_millis(10)).await.unwrap();

        assert!(found.to_review.is_empty());
    }

    #[tokio::test]
    async fn a_replaced_source_names_its_replacement() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let a = seed(&store, Some("p"), "the deploy runs on fridays", 1).await;
        let c = conclude(&store, Some("p"), &[a], 2).await;
        let a2 = seed(&store, Some("p"), "the deploy runs on mondays", 3).await;
        supersede(&store, a, a2, 3).await;

        let found = synthesis_core(&store, in_project("p"), TimestampMs::from_unix_millis(10)).await.unwrap();

        assert_eq!(found.to_review[0].conclusion.id, c);
        assert_eq!(found.to_review[0].live, 0);
        assert_eq!(lost(&found.to_review[0]), vec![(a, SourceState::Superseded, Some(a2))]);
    }

    #[tokio::test]
    async fn the_review_list_is_capped_newest_first() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let gone = seed(&store, Some("p"), "gone", 1).await;
        let mut made = Vec::new();
        for i in 0..(REVIEW_LIMIT as i64 + 2) {
            made.push(conclude(&store, Some("p"), &[gone], 2 + i).await);
        }
        delete_memory_core(&store, gone, MemoryKind::Memory, None).await.unwrap();

        let found = synthesis_core(&store, in_project("p"), TimestampMs::from_unix_millis(100)).await.unwrap();

        assert_eq!(found.to_review_total, made.len());
        let listed: Vec<i64> = found.to_review.iter().map(|r| r.conclusion.id).collect();
        let newest: Vec<i64> = made.iter().rev().take(REVIEW_LIMIT as usize).copied().collect();
        assert_eq!(listed, newest);
    }

    #[tokio::test]
    async fn another_agents_conclusion_is_not_to_review() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let theirs = seed_as(&store, "b", Some("p"), "their memory", 1).await;
        let conclusion = seed_as(&store, "b", Some("p"), "their conclusion", 2).await;
        store
            .conn
            .call(move |c| {
                c.execute(
                    "INSERT INTO memory_derivation (memory_id, source_id, created_at) VALUES (?1, ?2, 0)",
                    [conclusion, theirs],
                )
            })
            .await
            .unwrap();
        delete_memory_core(&store, theirs, MemoryKind::Memory, None).await.unwrap();
        let as_a = SynthesisParams { agent_id: Some("a".into()), ..in_project("p") };

        let found = synthesis_core(&store, as_a, TimestampMs::from_unix_millis(10)).await.unwrap();

        assert!(found.to_review.is_empty());
        assert_eq!(found.to_review_total, 0);
    }

    #[tokio::test]
    async fn synthesis_stays_with_its_agent() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let mine = seed_as(&store, "a", Some("p"), "mine", 1).await;
        let theirs = seed_as(&store, "b", Some("p"), "theirs", 2).await;
        let as_a = SynthesisParams { agent_id: Some("a".into()), ..in_project("p") };

        let found = synthesis_core(&store, as_a, TimestampMs::from_unix_millis(10)).await.unwrap();
        let owner = synthesis_core(&store, in_project("p"), TimestampMs::from_unix_millis(10)).await.unwrap();

        assert_eq!(ids(&found.pending), vec![mine]);
        assert_eq!(found.pending_total, 1);
        assert_eq!(ids(&owner.pending), vec![theirs, mine]);
        assert_eq!(unreviewed_count(&store, Some("a"), Some("p")).await.unwrap(), 0);
        assert_eq!(
            unreviewed_count(&store, Some("b"), Some("p")).await.unwrap(),
            1,
            "one agent's review is not another's"
        );
    }
}
