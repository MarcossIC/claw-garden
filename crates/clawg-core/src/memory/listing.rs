//! Reading the rows of `memory`: one by id, a filtered page of them, and
//! their counts.
//!
//! The `pub async fn *_core(store: &MemoryStore, ...)` functions here are
//! directly testable via `MemoryStore::new_for_test()`.

use serde::Serialize;

use crate::memory::error::{ClosedAs, failed_call};
use crate::memory::search::{reach_predicate, reader_bind, viewer_match_expression};
use crate::memory::sharing::Reader;
use crate::memory::{MemoryError, MemoryStore};

// ---------------------------------------------------------------------------
// list_memories — the rows of `memory`, filtered and paged in SQL. `summary`
// is the row's `content`.
// ---------------------------------------------------------------------------

// MemoryAdminRow keeps bare i64 millisecond fields. It crosses into the
// daemon's answers through Serialize, and the millisecond newtype the memory
// module uses internally deliberately implements neither Serialize nor
// Deserialize: a tuple struct that derived it would emit {"0":1700000000000}
// where a reader expects a bare number — it compiles, it runs, and nothing
// logs. Leaving it untyped is what turns that mistake into a compile error
// instead of a wire-format surprise. If a timestamp here ever needs the
// newtype it needs #[serde(transparent)] and a test asserting the emitted
// JSON is a bare integer, in the same change.

/// A single row from the `list_memories` inspection command.
#[derive(Debug, Clone, Serialize)]
pub struct MemoryAdminRow {
    pub id: i64,
    pub scope: String,
    pub agent_id: Option<String>,
    pub project_id: Option<String>,
    pub memory_type: String,
    pub summary: String,
    pub tier: i64,
    pub importance: f64,
    pub status: String,
    pub access_count: i64,
    pub created_at: i64,
    pub last_accessed_at: i64,
    pub superseded_by: Option<i64>,
    pub deleted_at: Option<i64>,
    // Provenance: where a stored fact came from.
    pub source: Option<String>,
    pub source_session_id: Option<String>,
    pub created_by_type: Option<String>,
    pub created_by_ref: Option<String>,
    pub last_modified_at: Option<i64>,
    pub update_count: Option<i64>,
    pub dispute_count: Option<i64>,
    /// Raw JSON text of the ids this row disputes.
    pub disputed_against: Option<String>,
    pub last_disputed_at: Option<i64>,
    /// The `memory` rows this one was drawn from, ascending. Only a
    /// conclusion has them; every other row omits the field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub derived_from: Option<Vec<i64>>,
    /// The topic key the row was saved under.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic_key: Option<String>,
}

/// Filter/pagination params for `list_memories_core`. `agent_id`,
/// `project_id` and `status` are exact matches.
///
/// `agent_id` filters by author; `reader` is the agent the listing is for,
/// which sees what the sharing policy lets it reach. Without one the listing
/// is the user's and reaches every row.
#[derive(Debug, Clone, Default)]
pub struct ListParams {
    pub reader: Option<Reader>,
    pub tier: Option<i64>,
    pub memory_type: Option<String>,
    pub agent_id: Option<String>,
    pub project_id: Option<String>,
    /// Only the rows saved in no project, which a search reaches from every
    /// project; the caller does not set `project_id` with it.
    pub no_project: bool,
    pub status: Option<String>,
    /// The project the row had in the tool it came from, compared whole.
    pub origin_project: Option<String>,
    /// Free text over `memory.content`: every token must match, each as a
    /// prefix, ignoring case and accents. A query with no token filters nothing.
    pub query: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
    /// Lists the deleted rows instead of the live ones, the most recently
    /// deleted first, so a delete can be found and undone.
    pub deleted: bool,
}

/// Whether `query` holds anything `ListParams::query` can search for. A query
/// that does not (only punctuation, only spaces) filters nothing, so a caller
/// that cares can refuse it instead of showing an unfiltered list as a result.
pub fn query_has_tokens(query: &str) -> bool {
    viewer_match_expression(query).is_some()
}

/// `ListParams` after validation and MATCH-expression building, shaped for
/// the SQL builders. Every value reaches SQL as a bound parameter.
#[derive(Default)]
struct RowFilter {
    /// The reader's reach as `reader_bind` binds it.
    reach: Option<String>,
    tier: Option<i64>,
    memory_type: Option<String>,
    agent_id: Option<String>,
    project_id: Option<String>,
    no_project: bool,
    status: Option<String>,
    origin_project: Option<String>,
    match_expr: Option<String>,
    deleted: bool,
}

impl RowFilter {
    /// The `WHERE` that picks live or deleted rows, and the order they list in.
    fn lifecycle(&self) -> (&'static str, &'static str) {
        if self.deleted {
            (" WHERE deleted_at IS NOT NULL", " ORDER BY deleted_at DESC, id DESC LIMIT ? OFFSET ?")
        } else {
            (" WHERE deleted_at IS NULL", " ORDER BY created_at DESC, id DESC LIMIT ? OFFSET ?")
        }
    }
}

/// Appends the filters of `f`. Pure: no I/O, so the SQL shape is testable
/// without a connection.
fn push_filters(sql: &mut String, params: &mut Vec<rusqlite::types::Value>, f: &RowFilter) {
    use rusqlite::types::Value;
    if let Some(reach) = &f.reach {
        sql.push_str(concat!(" AND ", reach_predicate!("agent_id", "?")));
        // `reach_predicate!` names its parameter four times; an anonymous `?`
        // is a new parameter each time.
        for _ in 0..4 {
            params.push(Value::Text(reach.clone()));
        }
    }
    if let Some(t) = f.tier {
        sql.push_str(" AND tier = ?");
        params.push(Value::Integer(t));
    }
    if let Some(a) = &f.agent_id {
        sql.push_str(" AND agent_id = ?");
        params.push(Value::Text(a.clone()));
    }
    if let Some(p) = &f.project_id {
        sql.push_str(" AND project_id = ?");
        params.push(Value::Text(p.clone()));
    }
    if f.no_project {
        sql.push_str(" AND project_id IS NULL");
    }
    if let Some(s) = &f.status {
        sql.push_str(" AND status = ?");
        params.push(Value::Text(s.clone()));
    }
    if let Some(mt) = &f.memory_type {
        sql.push_str(" AND memory_type = ?");
        params.push(Value::Text(mt.clone()));
    }
    if let Some(o) = &f.origin_project {
        sql.push_str(" AND origin_project = ?");
        params.push(Value::Text(o.clone()));
    }
    if let Some(expr) = &f.match_expr {
        // No LIMIT inside: a cap here would drop matches before the live,
        // filtered rows are chosen and make the page depend on index order.
        sql.push_str(" AND id IN (SELECT rowid FROM memory_fts WHERE memory_fts MATCH ?)");
        params.push(Value::Text(expr.clone()));
    }
}

/// Lists rows of `memory`, `created_at DESC` (or `deleted_at DESC` for
/// deleted rows), with `LIMIT`/`OFFSET` pushed down to SQL. Only live rows
/// are listed unless `deleted` asks for the deleted ones instead; `status` is
/// only filtered when asked — `superseded` rows stay visible by default
/// (inspection is an active inventory with superseded shown, distinct from
/// retrieval's stricter filter).
pub async fn list_memories_core(
    store: &MemoryStore,
    params: ListParams,
) -> Result<Vec<MemoryAdminRow>, MemoryError> {
    let limit = params.limit.unwrap_or(50).clamp(1, 100) as i64;
    let offset = params.offset.unwrap_or(0) as i64;
    let filter = RowFilter {
        reach: reader_bind(params.reader.as_ref()),
        tier: params.tier,
        memory_type: params.memory_type,
        agent_id: params.agent_id,
        project_id: params.project_id,
        no_project: params.no_project,
        status: params.status,
        origin_project: params.origin_project,
        match_expr: params.query.as_deref().and_then(viewer_match_expression),
        deleted: params.deleted,
    };

    store
        .conn
        .call(move |c| query_memory_rows(c, &filter, limit, offset))
        .await
        .map_err(failed_call(ClosedAs::During("list_memories_core")))
}

// Column order here is the contract with `map_memory_row` below.
pub(crate) const SELECT_MEMORY: &str = "SELECT id, scope, agent_id, project_id, memory_type, content, \
     tier, importance, status, access_count, created_at, last_accessed_at, superseded_by, \
     deleted_at, source, source_session_id, created_by_type, created_by_ref, \
     last_modified_at, update_count, dispute_count, disputed_against, last_disputed_at, \
     (SELECT json_group_array(source_id) FROM memory_derivation WHERE memory_id = memory.id), \
     topic_key \
     FROM memory";

/// Reads one row by id for inspection. Unlike the list it also
/// returns soft-deleted rows, reporting them through `deleted_at`: someone
/// holding an id from an earlier listing should learn the row was deleted,
/// not that it never existed. Read-only on purpose — it records no access, so
/// looking at a row never promotes it.
pub async fn get_memory_core(store: &MemoryStore, id: i64) -> Result<Option<MemoryAdminRow>, MemoryError> {
    store
        .conn
        .call(move |c| -> rusqlite::Result<Option<MemoryAdminRow>> {
            use rusqlite::OptionalExtension;
            c.query_row(&format!("{SELECT_MEMORY} WHERE id = ?1"), [id], map_memory_row).optional()
        })
        .await
        .map_err(failed_call(ClosedAs::During("get_memory_core")))
}

fn query_memory_rows(
    c: &rusqlite::Connection,
    filter: &RowFilter,
    limit: i64,
    offset: i64,
) -> rusqlite::Result<Vec<MemoryAdminRow>> {
    let (lifecycle, order) = filter.lifecycle();
    let mut sql = format!("{SELECT_MEMORY}{lifecycle}");
    let mut params: Vec<rusqlite::types::Value> = Vec::new();
    push_filters(&mut sql, &mut params, filter);
    sql.push_str(order);
    params.push(rusqlite::types::Value::Integer(limit));
    params.push(rusqlite::types::Value::Integer(offset));

    let mut stmt = c.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(params), map_memory_row)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

pub(crate) fn map_memory_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemoryAdminRow> {
    Ok(MemoryAdminRow {
        id: row.get(0)?,
        scope: row.get(1)?,
        agent_id: row.get(2)?,
        project_id: row.get(3)?,
        memory_type: row.get(4)?,
        summary: row.get(5)?,
        tier: row.get(6)?,
        importance: row.get(7)?,
        status: row.get(8)?,
        access_count: row.get(9)?,
        created_at: row.get(10)?,
        last_accessed_at: row.get(11)?,
        superseded_by: row.get(12)?,
        deleted_at: row.get(13)?,
        source: Some(row.get(14)?),
        source_session_id: row.get(15)?,
        created_by_type: Some(row.get(16)?),
        created_by_ref: Some(row.get(17)?),
        last_modified_at: Some(row.get(18)?),
        update_count: Some(row.get(19)?),
        dispute_count: Some(row.get(20)?),
        disputed_against: row.get(21)?,
        last_disputed_at: row.get(22)?,
        derived_from: sources_from_json(&row.get::<_, String>(23)?),
        topic_key: row.get(24)?,
    })
}

/// Reads the `json_group_array` of a row's sources: `None` for a row with
/// none, so only a conclusion carries the field. Sorted here because an
/// aggregate's order is not something SQLite promises.
fn sources_from_json(json: &str) -> Option<Vec<i64>> {
    let mut ids: Vec<i64> = serde_json::from_str(json).unwrap_or_default();
    ids.sort_unstable();
    Some(ids).filter(|ids| !ids.is_empty())
}

// ---------------------------------------------------------------------------
// memory_stats — what is stored, at a glance. Live rows only, with the
// soft-deleted ones reported apart so a total is never silently inflated by
// rows nothing can return any more.
// ---------------------------------------------------------------------------

/// Optional narrowing applied to every figure in [`MemoryStats`].
#[derive(Debug, Clone, Default)]
pub struct StatsParams {
    pub agent_id: Option<String>,
    pub project_id: Option<String>,
}

/// One bucket of a grouping. `key: None` is the NULL bucket (rows with no
/// project, for instance), kept distinct from any real value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Count {
    pub key: Option<String>,
    pub count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StatsTotals {
    pub memory: u64,
    /// Soft-deleted rows; not part of the live count.
    pub deleted: u64,
}

/// Snapshot of the store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MemoryStats {
    pub totals: StatsTotals,
    pub by_type: Vec<Count>,
    pub by_tier: Vec<Count>,
    pub by_status: Vec<Count>,
    pub by_agent: Vec<Count>,
    pub by_project: Vec<Count>,
    /// Newest `created_at` (ms) among live rows.
    pub newest_created_at: Option<i64>,
}

/// The `memory` column a grouping is computed over. An enum with an exhaustive
/// match, so the SQL fragment is always one of these literals and never text
/// that came from a caller.
#[derive(Clone, Copy)]
enum GroupBy {
    Type,
    Tier,
    Status,
    Agent,
    Project,
}

impl GroupBy {
    fn column(self) -> &'static str {
        match self {
            GroupBy::Type => "memory_type",
            GroupBy::Tier => "CAST(tier AS TEXT)",
            GroupBy::Status => "status",
            GroupBy::Agent => "agent_id",
            GroupBy::Project => "project_id",
        }
    }
}

fn stats_where(deleted: bool, filter: &RowFilter) -> (String, Vec<rusqlite::types::Value>) {
    let mut sql = String::from(if deleted {
        " WHERE deleted_at IS NOT NULL"
    } else {
        " WHERE deleted_at IS NULL"
    });
    let mut params = Vec::new();
    push_filters(&mut sql, &mut params, filter);
    (sql, params)
}

fn count_rows(tx: &rusqlite::Transaction<'_>, deleted: bool, filter: &RowFilter) -> rusqlite::Result<u64> {
    let (clause, params) = stats_where(deleted, filter);
    tx.query_row(
        &format!("SELECT COUNT(*) FROM memory{clause}"),
        rusqlite::params_from_iter(params),
        |r| r.get(0),
    )
}

fn group_memory(
    tx: &rusqlite::Transaction<'_>,
    by: GroupBy,
    filter: &RowFilter,
) -> rusqlite::Result<Vec<Count>> {
    let (clause, params) = stats_where(false, filter);
    let col = by.column();
    let mut stmt = tx.prepare(&format!(
        "SELECT {col}, COUNT(*) FROM memory{clause} GROUP BY 1 ORDER BY 2 DESC, 1 ASC"
    ))?;
    let rows = stmt.query_map(rusqlite::params_from_iter(params), |r| {
        Ok(Count { key: r.get(0)?, count: r.get(1)? })
    })?;
    rows.collect()
}

/// Computes [`MemoryStats`] inside a single transaction so the totals and the
/// groupings describe the same instant even while the ingest worker writes.
pub async fn memory_stats_core(
    store: &MemoryStore,
    params: StatsParams,
) -> Result<MemoryStats, MemoryError> {
    let filter = RowFilter {
        agent_id: params.agent_id,
        project_id: params.project_id,
        ..Default::default()
    };

    store
        .conn
        .call(move |c| -> rusqlite::Result<MemoryStats> {
            let tx = c.transaction()?;
            let (clause, params) = stats_where(false, &filter);
            let newest: Option<i64> = tx.query_row(
                &format!("SELECT MAX(created_at) FROM memory{clause}"),
                rusqlite::params_from_iter(params),
                |r| r.get(0),
            )?;
            let stats = MemoryStats {
                totals: StatsTotals {
                    memory: count_rows(&tx, false, &filter)?,
                    deleted: count_rows(&tx, true, &filter)?,
                },
                by_type: group_memory(&tx, GroupBy::Type, &filter)?,
                by_tier: group_memory(&tx, GroupBy::Tier, &filter)?,
                by_status: group_memory(&tx, GroupBy::Status, &filter)?,
                by_agent: group_memory(&tx, GroupBy::Agent, &filter)?,
                by_project: group_memory(&tx, GroupBy::Project, &filter)?,
                newest_created_at: newest,
            };
            tx.commit()?;
            Ok(stats)
        })
        .await
        .map_err(failed_call(ClosedAs::During("memory_stats_core")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::admin::delete_memory_core;
    use crate::memory::sharing::Reach;

    /// `agent` as a policy that hides nothing lets it act.
    fn shared(agent: &str) -> Reader {
        Reader::new(agent, Reach::AllBut(Vec::new()))
    }

    /// `agent` with `author`'s rows hidden from it.
    fn hiding(agent: &str, author: &str) -> Reader {
        Reader::new(agent, Reach::AllBut(vec![author.to_string()]))
    }

    // ── list_memories_core ──────────────────────────

    async fn seed_memory_full(
        store: &MemoryStore,
        memory_type: &str,
        tier: i64,
        status: &str,
        deleted: bool,
        created_at: i64,
    ) -> i64 {
        let mt = memory_type.to_string();
        let status = status.to_string();
        let deleted_at: Option<i64> = if deleted { Some(1) } else { None };
        store
            .conn
            .call(move |c| -> rusqlite::Result<i64> {
                c.execute(
                    "INSERT INTO memory \
                     (scope, agent_id, content, memory_type, tier, importance, status, \
                      created_at, last_modified_at, last_accessed_at, \
                      created_by_type, created_by_ref, deleted_at) \
                     VALUES ('agent', 'test-agent', 'row content', ?, ?, 5.0, ?, \
                             ?, 0, 0, 'agent', '', ?)",
                    rusqlite::params![mt, tier, status, created_at, deleted_at],
                )?;
                Ok(c.last_insert_rowid())
            })
            .await
            .expect("seed_memory_full failed")
    }

    async fn link(store: &MemoryStore, memory_id: i64, source_id: i64) {
        store
            .conn
            .call(move |c| {
                c.execute(
                    "INSERT INTO memory_derivation (memory_id, source_id, created_at) VALUES (?1, ?2, 0)",
                    [memory_id, source_id],
                )
            })
            .await
            .unwrap();
    }

    async fn mark_deleted(store: &MemoryStore, id: i64, at: i64) {
        store
            .conn
            .call(move |c| c.execute("UPDATE memory SET deleted_at = ?2 WHERE id = ?1", [id, at]))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn deleted_rows_are_listed_only_when_asked() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let live = seed_memory_full(&store, "semantic_fact", 3, "active", false, 1).await;
        let gone = seed_memory_full(&store, "semantic_fact", 3, "active", false, 2).await;
        mark_deleted(&store, gone, 50).await;

        let listed = |rows: Vec<MemoryAdminRow>| rows.iter().map(|r| r.id).collect::<Vec<_>>();
        let default = list_memories_core(&store, ListParams::default()).await.unwrap();
        let deleted = list_memories_core(&store, ListParams { deleted: true, ..Default::default() }).await.unwrap();

        assert_eq!(listed(default), vec![live]);
        assert_eq!(listed(deleted), vec![gone]);
    }

    #[tokio::test]
    async fn deleted_rows_list_newest_deletion_first() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let old_save = seed_memory_full(&store, "semantic_fact", 3, "active", false, 1).await;
        let new_save = seed_memory_full(&store, "semantic_fact", 3, "active", false, 2).await;
        let middle = seed_memory_full(&store, "user_preference", 1, "active", false, 3).await;
        mark_deleted(&store, new_save, 10).await;
        mark_deleted(&store, middle, 20).await;
        mark_deleted(&store, old_save, 30).await;

        let rows = list_memories_core(&store, ListParams { deleted: true, ..Default::default() }).await.unwrap();
        let only_memory = list_memories_core(
            &store,
            ListParams { deleted: true, memory_type: Some("semantic_fact".into()), ..Default::default() },
        )
        .await
        .unwrap();

        assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), vec![old_save, middle, new_save]);
        assert_eq!(rows[0].deleted_at, Some(30));
        assert_eq!(only_memory.iter().map(|r| r.id).collect::<Vec<_>>(), vec![old_save, new_save]);
    }

    #[tokio::test]
    async fn the_listing_names_the_sources_of_a_conclusion() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let a = seed_memory_full(&store, "semantic_fact", 3, "active", false, 1).await;
        let b = seed_memory_full(&store, "semantic_fact", 3, "active", false, 2).await;
        let c = seed_memory_full(&store, "user_preference", 1, "active", false, 3).await;
        link(&store, c, b).await;
        link(&store, c, a).await;

        let rows = list_memories_core(&store, ListParams::default()).await.unwrap();
        let sources = |id: i64| rows.iter().find(|r| r.id == id).unwrap().derived_from.clone();
        assert_eq!(sources(c), Some(vec![a, b]));
        assert_eq!(sources(a), None);

        let one = get_memory_core(&store, c).await.unwrap().unwrap();
        assert_eq!(one.derived_from, Some(vec![a, b]));
        let json = serde_json::to_value(&rows).unwrap();
        let plain = json.as_array().unwrap().iter().find(|r| r["id"] == a).unwrap();
        assert!(plain.get("derived_from").is_none(), "a row with no sources omits the field: {plain}");
    }

    // Tier filter (WHERE).
    #[tokio::test]
    async fn list_memories_tier_filter_applies() {
        let store = MemoryStore::new_for_test().await.unwrap();
        seed_memory_full(&store, "chat_message", 2, "active", false, 100).await;
        seed_memory_full(&store, "chat_message", 3, "active", false, 200).await;

        let rows = list_memories_core(
            &store,
            ListParams { tier: Some(2), ..Default::default() },
        )
        .await
        .unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].tier, 2);
    }

    // Limit=500 clamps to <=100.
    #[tokio::test]
    async fn list_memories_limit_clamps_to_100() {
        let store = MemoryStore::new_for_test().await.unwrap();
        for i in 0..120i64 {
            seed_memory_full(&store, "chat_message", 3, "active", false, i).await;
        }

        let rows = list_memories_core(
            &store,
            ListParams { limit: Some(500), ..Default::default() },
        )
        .await
        .unwrap();

        assert_eq!(rows.len(), 100, "limit must clamp to the hard cap of 100");
    }

    // Offset paginates.
    #[tokio::test]
    async fn list_memories_offset_paginates_merged_results() {
        let store = MemoryStore::new_for_test().await.unwrap();
        for i in 0..5i64 {
            seed_memory_full(&store, "chat_message", 3, "active", false, i * 10).await;
        }

        let page1 = list_memories_core(
            &store,
            ListParams { limit: Some(2), offset: Some(0), ..Default::default() },
        )
        .await
        .unwrap();
        let page2 = list_memories_core(
            &store,
            ListParams { limit: Some(2), offset: Some(2), ..Default::default() },
        )
        .await
        .unwrap();

        assert_eq!(page1.len(), 2);
        assert_eq!(page2.len(), 2);
        assert_ne!(page1[0].id, page2[0].id, "offset must shift the page window");
    }

    // Fragments of one save share `created_at`; without a tiebreak the page
    // boundaries between equal timestamps are undefined.
    #[tokio::test]
    async fn list_memories_ties_on_created_at_order_by_id_desc() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let mut ids = Vec::new();
        for _ in 0..5 {
            ids.push(seed_memory_full(&store, "chat_message", 3, "active", false, 100).await);
        }
        let expected: Vec<i64> = ids.iter().rev().copied().collect();

        let all = list_memories_core(&store, ListParams::default()).await.unwrap();
        assert_eq!(all.iter().map(|r| r.id).collect::<Vec<_>>(), expected);

        let mut paged = Vec::new();
        for offset in [0u32, 2, 4] {
            let page = list_memories_core(&store, ListParams { limit: Some(2), offset: Some(offset), ..Default::default() })
                .await
                .unwrap();
            paged.extend(page.iter().map(|r| r.id));
        }
        assert_eq!(paged, expected, "pages must neither overlap nor skip");
    }

    #[tokio::test]
    async fn list_memories_memory_row_carries_provenance() {
        let store = MemoryStore::new_for_test().await.unwrap();
        store
            .conn
            .call(|c| {
                c.execute(
                    "INSERT INTO memory \
                     (scope, agent_id, content, memory_type, source, source_session_id, \
                      created_at, created_by_type, created_by_ref, last_modified_at, \
                      update_count, last_accessed_at, dispute_count, disputed_against, \
                      last_disputed_at, status) \
                     VALUES ('agent', 'a1', 'body', 'observation', 'explicit', 'sess-9', \
                             10, 'user', 'someone', 20, 3, 30, 2, '[7,8]', 40, 'disputed')",
                    [],
                )
            })
            .await
            .unwrap();

        let rows = list_memories_core(&store, ListParams::default()).await.unwrap();

        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.source.as_deref(), Some("explicit"));
        assert_eq!(row.source_session_id.as_deref(), Some("sess-9"));
        assert_eq!(row.created_by_type.as_deref(), Some("user"));
        assert_eq!(row.created_by_ref.as_deref(), Some("someone"));
        assert_eq!(row.last_modified_at, Some(20));
        assert_eq!(row.update_count, Some(3));
        assert_eq!(row.dispute_count, Some(2));
        assert_eq!(row.disputed_against.as_deref(), Some("[7,8]"));
        assert_eq!(row.last_disputed_at, Some(40));
    }

    // ── list_memories_core filters ──────────────────

    async fn seed_memory_scoped(
        store: &MemoryStore,
        content: &str,
        agent: &str,
        project: Option<&str>,
        status: &str,
    ) -> i64 {
        let (content, agent, project, status) = (
            content.to_string(),
            agent.to_string(),
            project.map(str::to_string),
            status.to_string(),
        );
        store
            .conn
            .call(move |c| -> rusqlite::Result<i64> {
                c.execute(
                    "INSERT INTO memory \
                     (scope, agent_id, project_id, content, memory_type, status, \
                      created_at, last_modified_at, last_accessed_at, \
                      created_by_type, created_by_ref) \
                     VALUES ('agent', ?, ?, ?, 'observation', ?, 0, 0, 0, 'agent', '')",
                    rusqlite::params![agent, project, content, status],
                )?;
                Ok(c.last_insert_rowid())
            })
            .await
            .expect("seed_memory_scoped failed")
    }

    fn ids_of(rows: &[MemoryAdminRow]) -> Vec<i64> {
        let mut ids: Vec<i64> = rows.iter().map(|r| r.id).collect();
        ids.sort_unstable();
        ids
    }

    #[tokio::test]
    async fn list_filters_by_agent() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let a = seed_memory_scoped(&store, "one", "agent-a", None, "active").await;
        seed_memory_scoped(&store, "two", "agent-b", None, "active").await;

        let rows = list_memories_core(
            &store,
            ListParams { agent_id: Some("agent-a".into()), ..Default::default() },
        )
        .await
        .unwrap();

        assert_eq!(ids_of(&rows), vec![a]);
    }

    #[tokio::test]
    async fn a_list_reaches_what_its_reader_reaches() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let mine = seed_memory_scoped(&store, "one", "agent-a", None, "active").await;
        let theirs = seed_memory_scoped(&store, "two", "agent-b", None, "active").await;
        let hidden = seed_memory_scoped(&store, "three", "agent-c", None, "active").await;
        let list = |reader: Reader, agent: Option<&str>| {
            let params = ListParams { reader: Some(reader), agent_id: agent.map(Into::into), ..Default::default() };
            list_memories_core(&store, params)
        };

        let rows = list(hiding("agent-a", "agent-c"), None).await.unwrap();
        assert_eq!(ids_of(&rows), vec![mine, theirs], "every author it reaches, not the hidden one");
        let rows = list(hiding("agent-a", "agent-c"), Some("agent-b")).await.unwrap();
        assert_eq!(ids_of(&rows), vec![theirs], "`agent_id` filters by author within the reach");
        let rows = list(shared("agent-a"), None).await.unwrap();
        assert_eq!(ids_of(&rows), vec![mine, theirs, hidden]);
    }

    #[tokio::test]
    async fn list_filters_by_project() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let exact = seed_memory_scoped(&store, "one", "a", Some("proj"), "active").await;
        seed_memory_scoped(&store, "two", "a", Some("proj-2"), "active").await;
        seed_memory_scoped(&store, "three", "a", None, "active").await;

        let rows = list_memories_core(
            &store,
            ListParams { project_id: Some("proj".into()), ..Default::default() },
        )
        .await
        .unwrap();

        assert_eq!(ids_of(&rows), vec![exact], "project filter is equality, not prefix");
    }

    #[tokio::test]
    async fn a_listing_of_rows_without_a_project() {
        let store = MemoryStore::new_for_test().await.unwrap();
        seed_memory_scoped(&store, "one", "a", Some("proj"), "active").await;
        let loose = seed_memory_scoped(&store, "two", "a", None, "active").await;

        let rows = list_memories_core(&store, ListParams { no_project: true, ..Default::default() }).await.unwrap();

        assert_eq!(ids_of(&rows), vec![loose]);
    }

    #[tokio::test]
    async fn a_listing_filters_by_origin_project() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let from_a = seed_memory_scoped(&store, "one", "agent-a", None, "active").await;
        let from_a_other = seed_memory_scoped(&store, "two", "agent-b", None, "active").await;
        let from_b = seed_memory_scoped(&store, "three", "agent-a", None, "active").await;
        seed_memory_scoped(&store, "four", "agent-a", None, "active").await;
        store
            .conn
            .call(move |c| -> rusqlite::Result<()> {
                for (id, origin) in [(from_a, "a"), (from_a_other, "a"), (from_b, "b")] {
                    c.execute("UPDATE memory SET origin_project = ?1 WHERE id = ?2", rusqlite::params![origin, id])?;
                }
                Ok(())
            })
            .await
            .unwrap();

        let rows = list_memories_core(&store, ListParams { origin_project: Some("a".into()), ..Default::default() })
            .await
            .unwrap();
        assert_eq!(ids_of(&rows), vec![from_a, from_a_other], "{rows:?}");

        let rows = list_memories_core(
            &store,
            ListParams { origin_project: Some("a".into()), agent_id: Some("agent-a".into()), ..Default::default() },
        )
        .await
        .unwrap();
        assert_eq!(ids_of(&rows), vec![from_a]);

        let rows = list_memories_core(&store, ListParams { origin_project: Some("A".into()), ..Default::default() })
            .await
            .unwrap();
        assert!(rows.is_empty(), "the name is compared whole: {rows:?}");
    }

    #[tokio::test]
    async fn list_filters_by_status() {
        let store = MemoryStore::new_for_test().await.unwrap();
        seed_memory_scoped(&store, "one", "a", None, "active").await;
        let disputed = seed_memory_scoped(&store, "two", "a", None, "disputed").await;
        seed_memory_scoped(&store, "three", "a", None, "superseded").await;

        let rows = list_memories_core(
            &store,
            ListParams { status: Some("disputed".into()), ..Default::default() },
        )
        .await
        .unwrap();

        assert_eq!(ids_of(&rows), vec![disputed]);
    }

    #[tokio::test]
    async fn viewer_match_is_prefix_and_folds_accents() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let id = seed_memory_scoped(&store, "la configuración del núcleo", "a", None, "active").await;
        seed_memory_scoped(&store, "otra cosa distinta", "a", None, "active").await;

        for query in ["configuracion", "config", "CONFIGURACIÓN nucleo"] {
            let rows = list_memories_core(
                &store,
                ListParams { query: Some(query.into()), ..Default::default() },
            )
            .await
            .unwrap();
            assert_eq!(ids_of(&rows), vec![id], "query {query:?}");
        }
    }

    #[tokio::test]
    async fn text_query_requires_every_token_across_rows() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let both = seed_memory_scoped(&store, "alpha and beta together", "a", None, "active").await;
        seed_memory_scoped(&store, "only alpha here", "a", None, "active").await;

        let rows = list_memories_core(
            &store,
            ListParams { query: Some("alpha beta".into()), ..Default::default() },
        )
        .await
        .unwrap();

        assert_eq!(ids_of(&rows), vec![both]);
    }

    // ── get_memory_core ─────────────────────────────

    #[tokio::test]
    async fn get_memory_by_id() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let mem = seed_memory_scoped(&store, "full body text", "a1", Some("p1"), "active").await;

        let row = get_memory_core(&store, mem).await.unwrap().unwrap();
        assert_eq!(row.summary, "full body text");
        assert_eq!(row.agent_id.as_deref(), Some("a1"));
        assert_eq!(row.project_id.as_deref(), Some("p1"));
        assert_eq!(row.source.as_deref(), Some("agent"));
    }

    #[tokio::test]
    async fn get_missing_is_none() {
        let store = MemoryStore::new_for_test().await.unwrap();
        seed_memory_scoped(&store, "x", "a", None, "active").await;

        assert!(get_memory_core(&store, 999).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn get_returns_soft_deleted() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let id = seed_memory_scoped(&store, "gone", "a", None, "active").await;
        delete_memory_core(&store, id, None).await.unwrap();

        let row = get_memory_core(&store, id).await.unwrap().unwrap();

        assert!(row.deleted_at.is_some());
        assert_eq!(row.summary, "gone");
    }

    #[tokio::test]
    async fn get_does_not_touch_access_count() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let id = seed_memory_scoped(&store, "x", "a", None, "active").await;
        store
            .conn
            .call(move |c| {
                c.execute(
                    "UPDATE memory SET access_count = 7, last_accessed_at = 55 WHERE id = ?1",
                    [id],
                )
            })
            .await
            .unwrap();

        for _ in 0..2 {
            let row = get_memory_core(&store, id).await.unwrap().unwrap();
            assert_eq!((row.access_count, row.last_accessed_at), (7, 55));
        }
    }

    // ── memory_stats_core ───────────────────────────

    async fn seed_stat_row(
        store: &MemoryStore,
        (memory_type, tier, status): (&str, i64, &str),
        (agent, project): (&str, Option<&str>),
        created_at: i64,
        deleted: bool,
    ) {
        let (memory_type, status, agent, project) = (
            memory_type.to_string(),
            status.to_string(),
            agent.to_string(),
            project.map(str::to_string),
        );
        let deleted_at: Option<i64> = deleted.then_some(1);
        store
            .conn
            .call(move |c| {
                c.execute(
                    "INSERT INTO memory \
                     (scope, agent_id, project_id, content, memory_type, tier, status, \
                      created_at, last_modified_at, last_accessed_at, \
                      created_by_type, created_by_ref, deleted_at) \
                     VALUES ('agent', ?, ?, 'c', ?, ?, ?, ?, 0, 0, 'agent', '', ?)",
                    rusqlite::params![agent, project, memory_type, tier, status, created_at, deleted_at],
                )
            })
            .await
            .expect("seed_stat_row failed");
    }

    fn count(key: Option<&str>, n: u64) -> Count {
        Count { key: key.map(str::to_string), count: n }
    }

    #[tokio::test]
    async fn stats_counts_live_and_deleted_apart() {
        let store = MemoryStore::new_for_test().await.unwrap();
        seed_memory_full(&store, "observation", 3, "active", false, 10).await;
        seed_memory_full(&store, "observation", 3, "active", false, 20).await;
        seed_memory_full(&store, "observation", 3, "active", true, 30).await;

        let stats = memory_stats_core(&store, StatsParams::default()).await.unwrap();

        assert_eq!(stats.totals, StatsTotals { memory: 2, deleted: 1 });
        assert_eq!(stats.newest_created_at, Some(20), "newest live row, deleted ones ignored");
    }

    #[tokio::test]
    async fn stats_groups_by_type_tier_status_agent_project() {
        let store = MemoryStore::new_for_test().await.unwrap();
        seed_stat_row(&store, ("fact", 1, "active"), ("a2", Some("p1")), 1, false).await;
        seed_stat_row(&store, ("fact", 3, "active"), ("a1", Some("p1")), 2, false).await;
        seed_stat_row(&store, ("note", 3, "disputed"), ("a1", Some("p2")), 3, false).await;
        // Deleted rows must not reach any grouping.
        seed_stat_row(&store, ("ghost", 2, "superseded"), ("a9", Some("p9")), 4, true).await;

        let stats = memory_stats_core(&store, StatsParams::default()).await.unwrap();

        assert_eq!(stats.by_type, vec![count(Some("fact"), 2), count(Some("note"), 1)]);
        assert_eq!(stats.by_tier, vec![count(Some("3"), 2), count(Some("1"), 1)]);
        assert_eq!(stats.by_status, vec![count(Some("active"), 2), count(Some("disputed"), 1)]);
        assert_eq!(stats.by_agent, vec![count(Some("a1"), 2), count(Some("a2"), 1)]);
        assert_eq!(stats.by_project, vec![count(Some("p1"), 2), count(Some("p2"), 1)]);
    }

    #[tokio::test]
    async fn stats_filters_by_agent_and_project() {
        let store = MemoryStore::new_for_test().await.unwrap();
        seed_stat_row(&store, ("fact", 3, "active"), ("a1", Some("p1")), 1, false).await;
        seed_stat_row(&store, ("fact", 3, "active"), ("a1", Some("p2")), 2, false).await;
        seed_stat_row(&store, ("fact", 3, "active"), ("a2", Some("p1")), 3, false).await;
        seed_stat_row(&store, ("fact", 3, "active"), ("a1", Some("p1")), 4, true).await;

        let stats = memory_stats_core(
            &store,
            StatsParams { agent_id: Some("a1".into()), project_id: Some("p1".into()) },
        )
        .await
        .unwrap();

        assert_eq!(stats.totals.memory, 1);
        assert_eq!(stats.totals.deleted, 1);
        assert_eq!(stats.by_agent, vec![count(Some("a1"), 1)]);
        assert_eq!(stats.by_project, vec![count(Some("p1"), 1)]);

        let by_agent = memory_stats_core(
            &store,
            StatsParams { agent_id: Some("a1".into()), project_id: None },
        )
        .await
        .unwrap();
        assert_eq!(by_agent.totals.memory, 2);
    }

    #[tokio::test]
    async fn stats_empty_store_is_zero() {
        let store = MemoryStore::new_for_test().await.unwrap();

        let stats = memory_stats_core(&store, StatsParams::default()).await.unwrap();

        assert_eq!(stats.totals, StatsTotals { memory: 0, deleted: 0 });
        assert!(stats.by_type.is_empty() && stats.by_tier.is_empty());
        assert!(stats.by_status.is_empty() && stats.by_agent.is_empty());
        assert!(stats.by_project.is_empty());
        assert_eq!(stats.newest_created_at, None);
    }

    #[tokio::test]
    async fn stats_null_project_is_own_bucket() {
        let store = MemoryStore::new_for_test().await.unwrap();
        seed_stat_row(&store, ("fact", 3, "active"), ("a", None), 1, false).await;
        seed_stat_row(&store, ("fact", 3, "active"), ("a", None), 2, false).await;
        seed_stat_row(&store, ("fact", 3, "active"), ("a", Some("p")), 3, false).await;

        let stats = memory_stats_core(&store, StatsParams::default()).await.unwrap();

        assert_eq!(stats.by_project, vec![count(None, 2), count(Some("p"), 1)]);
    }

    // Deleted_at excluded; status='superseded' visible.
    #[tokio::test]
    async fn list_memories_excludes_deleted_keeps_superseded() {
        let store = MemoryStore::new_for_test().await.unwrap();
        seed_memory_full(&store, "chat_message", 3, "active", true, 100).await; // deleted
        let superseded_id =
            seed_memory_full(&store, "chat_message", 3, "superseded", false, 200).await;

        let rows = list_memories_core(
            &store,
            ListParams { limit: Some(500), ..Default::default() },
        )
        .await
        .unwrap();

        assert_eq!(rows.len(), 1, "deleted row must be excluded");
        assert_eq!(rows[0].id, superseded_id);
        assert_eq!(rows[0].status, "superseded", "superseded rows must remain visible");
    }
}
