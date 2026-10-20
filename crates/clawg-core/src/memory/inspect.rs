//! What `clawg doctor` reads from a store: facts, never row content.
//!
//! The schema imposes none of the rules `memory` lives by (no foreign key, no
//! `CHECK` on `status`, `superseded_by`, `revived_from` or `disputed_against`,
//! and neither the topic nor the dedup index is unique), so each rule is read
//! back here as a query and every row that breaks it is named by id. Deciding
//! how serious each fact is belongs to the caller.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension};

use crate::memory::backup::{open_for_probe, probe, ProbeState, SchemaPlan, StoreProbe};
use crate::memory::db::StoredContract;

/// What [`inspect`] found.
#[derive(Debug, Clone, PartialEq)]
pub struct Inspection {
    /// The schema of the file, as `clawg status` reads it. When the rows
    /// could not be read after the schema was, this says why.
    pub schema: StoreProbe,
    /// Read only when the schema is this binary's or an older one: the
    /// columns of a newer or diverged store may mean something else.
    pub facts: Option<StoreFacts>,
}

/// The facts of a store, each list in id order. A table or column the store
/// does not have yet leaves its fact empty.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StoreFacts {
    /// The `embedding_contract` row; `None` until a model was loaded once.
    pub contract: Option<StoredContract>,
    pub agents: Vec<AgentRows>,
    pub projects: Vec<ProjectRow>,
    pub outbox: OutboxFacts,
    pub invariants: Invariants,
}

/// The rows of one agent; `agent_id` is `None` for the rows no agent saved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRows {
    pub agent_id: Option<String>,
    pub live: i64,
    pub total: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRow {
    pub id: String,
    pub root_path: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OutboxFacts {
    pub pending: i64,
    pub failed: Vec<FailedSave>,
}

/// A `failed` outbox row, by what names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailedSave {
    pub operation_id: String,
    pub agent_id: String,
    pub attempts: i64,
}

/// The rows that break each rule of `memory`. A row is live when it is
/// neither deleted nor superseded; a save is the rows of one `save_id` of
/// `memory_piece`, or a row alone when it has no link.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Invariants {
    /// More than one live save under one topic key of an agent and project.
    pub topic_versions: Vec<TopicVersions>,
    /// A superseded row whose `superseded_by` is NULL or names no row.
    pub dangling_superseded: Vec<DanglingSuperseded>,
    /// A row whose `revived_from` names no row.
    pub dangling_revived: Vec<DanglingRevived>,
    /// Live rows of one agent and project with one `content_hash`.
    pub hash_duplicates: Vec<HashDuplicates>,
    /// A save whose first piece is gone while a link still keeps its opening.
    pub purged_headers: Vec<PurgedHeader>,
    /// A live `disputed` row whose `disputed_against` names no live row.
    pub lonely_disputes: Vec<i64>,
}

impl Invariants {
    pub fn is_empty(&self) -> bool {
        *self == Invariants::default()
    }
}

/// The saves live under one key, each by its first row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopicVersions {
    pub agent_id: Option<String>,
    pub project_id: Option<String>,
    pub saves: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DanglingSuperseded {
    pub id: i64,
    pub superseded_by: Option<i64>,
    /// `revision`, `verdict`, or `None` before the cause was recorded.
    pub cause: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DanglingRevived {
    pub id: i64,
    pub revived_from: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HashDuplicates {
    pub agent_id: Option<String>,
    pub project_id: Option<String>,
    pub ids: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PurgedHeader {
    pub save_id: i64,
    /// The id the first piece had.
    pub first_piece: i64,
}

/// Reads `<base_dir>/memory.db` without the lock and without writing, so it
/// works beside a running daemon (WAL lets a reader in) and leaves a stopped
/// store as it was.
pub fn inspect(base_dir: &Path) -> Inspection {
    let mut schema = probe(base_dir);
    let readable = matches!(
        schema.state,
        ProbeState::Schema(SchemaPlan::UpToDate | SchemaPlan::Pending { .. })
    );
    if !readable {
        return Inspection { schema, facts: None };
    }
    crate::memory::db::ensure_vec0_registered();
    match open_for_probe(&base_dir.join("memory.db")).and_then(|c| read_facts(&c)) {
        Ok(facts) => Inspection { schema, facts: Some(facts) },
        Err(e) => {
            schema.state = ProbeState::Unreadable(e.to_string());
            Inspection { schema, facts: None }
        }
    }
}

fn read_facts(c: &Connection) -> rusqlite::Result<StoreFacts> {
    let has = Has::of(c)?;
    Ok(StoreFacts {
        contract: if has.table("embedding_contract") { contract(c)? } else { None },
        agents: agents(c)?,
        projects: if has.table("project") { projects(c)? } else { Vec::new() },
        outbox: if has.table("ingest_outbox") { outbox(c)? } else { OutboxFacts::default() },
        invariants: Invariants {
            topic_versions: if has.memory_column("topic_key") { topic_versions(c, &has)? } else { Vec::new() },
            dangling_superseded: dangling_superseded(c, &has)?,
            dangling_revived: if has.memory_column("revived_from") { dangling_revived(c)? } else { Vec::new() },
            hash_duplicates: if has.memory_column("content_hash") { hash_duplicates(c)? } else { Vec::new() },
            purged_headers: if has.table("memory_piece") { purged_headers(c)? } else { Vec::new() },
            lonely_disputes: lonely_disputes(c)?,
        },
    })
}

/// The tables and the columns of `memory` the store has.
struct Has {
    tables: Vec<String>,
    memory_columns: Vec<String>,
}

impl Has {
    fn of(c: &Connection) -> rusqlite::Result<Has> {
        let names = |sql: &str| -> rusqlite::Result<Vec<String>> {
            let mut stmt = c.prepare(sql)?;
            let rows = stmt.query_map([], |r| r.get(0))?;
            rows.collect()
        };
        Ok(Has {
            tables: names("SELECT name FROM sqlite_master WHERE type = 'table'")?,
            memory_columns: names("SELECT name FROM pragma_table_info('memory')")?,
        })
    }

    fn table(&self, name: &str) -> bool {
        self.tables.iter().any(|t| t == name)
    }

    fn memory_column(&self, name: &str) -> bool {
        self.memory_columns.iter().any(|col| col == name)
    }
}

const LIVE: &str = "deleted_at IS NULL AND status <> 'superseded'";

fn contract(c: &Connection) -> rusqlite::Result<Option<StoredContract>> {
    c.query_row(
        "SELECT model_id, dim, query_prefix, passage_prefix FROM embedding_contract WHERE id = 1",
        [],
        |r| Ok(StoredContract { model_id: r.get(0)?, dim: r.get(1)?, query_prefix: r.get(2)?, passage_prefix: r.get(3)? }),
    )
    .optional()
}

fn agents(c: &Connection) -> rusqlite::Result<Vec<AgentRows>> {
    let mut stmt = c.prepare(&format!(
        "SELECT agent_id, SUM({LIVE}), COUNT(*) FROM memory GROUP BY agent_id ORDER BY agent_id"
    ))?;
    let rows = stmt.query_map([], |r| Ok(AgentRows { agent_id: r.get(0)?, live: r.get(1)?, total: r.get(2)? }))?;
    rows.collect()
}

fn projects(c: &Connection) -> rusqlite::Result<Vec<ProjectRow>> {
    let mut stmt = c.prepare("SELECT id, root_path FROM project ORDER BY id")?;
    let rows = stmt.query_map([], |r| Ok(ProjectRow { id: r.get(0)?, root_path: r.get(1)? }))?;
    rows.collect()
}

fn outbox(c: &Connection) -> rusqlite::Result<OutboxFacts> {
    let pending = c.query_row("SELECT COUNT(*) FROM ingest_outbox WHERE state = 'pending'", [], |r| r.get(0))?;
    let mut stmt =
        c.prepare("SELECT operation_id, agent_id, attempts FROM ingest_outbox WHERE state = 'failed' ORDER BY id")?;
    let rows = stmt.query_map([], |r| {
        Ok(FailedSave { operation_id: r.get(0)?, agent_id: r.get(1)?, attempts: r.get(2)? })
    })?;
    Ok(OutboxFacts { pending, failed: rows.collect::<rusqlite::Result<_>>()? })
}

/// One live save per key: the pieces of a long save share the key and are
/// one version, so rows are grouped by save before they are counted.
fn topic_versions(c: &Connection, has: &Has) -> rusqlite::Result<Vec<TopicVersions>> {
    let save = if has.table("memory_piece") {
        "COALESCE((SELECT 's' || MIN(p.save_id) FROM memory_piece p WHERE p.memory_id = m.id), 'r' || m.id)"
    } else {
        "'r' || m.id"
    };
    let mut stmt = c.prepare(&format!(
        "SELECT agent_id, project_id, topic_key, MIN(id) FROM ( \
             SELECT m.id, m.agent_id, m.project_id, m.topic_key, {save} AS save FROM memory m \
             WHERE m.{LIVE} AND m.topic_key IS NOT NULL) \
         GROUP BY agent_id, COALESCE(project_id, ''), topic_key, save \
         ORDER BY agent_id, COALESCE(project_id, ''), topic_key, MIN(id)"
    ))?;
    let rows = stmt.query_map([], |r| {
        Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, String>(2)?, r.get::<_, i64>(3)?))
    })?;
    let mut found: Vec<(TopicVersions, String)> = Vec::new();
    for row in rows {
        let (agent_id, project_id, key, first) = row?;
        match found.last_mut() {
            Some((group, k)) if *k == key && same_scope(&group.agent_id, &group.project_id, &agent_id, &project_id) => {
                group.saves.push(first)
            }
            _ => found.push((TopicVersions { agent_id, project_id, saves: vec![first] }, key)),
        }
    }
    Ok(found.into_iter().map(|(group, _)| group).filter(|g| g.saves.len() > 1).collect())
}

/// The scope the topic and the dedup indexes key on: a NULL project is the
/// empty one, as their `COALESCE` reads it.
fn same_scope(agent: &Option<String>, project: &Option<String>, other_agent: &Option<String>, other_project: &Option<String>) -> bool {
    agent == other_agent && project.as_deref().unwrap_or("") == other_project.as_deref().unwrap_or("")
}

fn dangling_superseded(c: &Connection, has: &Has) -> rusqlite::Result<Vec<DanglingSuperseded>> {
    let cause = if has.memory_column("supersede_cause") { "m.supersede_cause" } else { "NULL" };
    let mut stmt = c.prepare(&format!(
        "SELECT m.id, m.superseded_by, {cause} FROM memory m \
         WHERE m.deleted_at IS NULL AND m.status = 'superseded' \
           AND (m.superseded_by IS NULL OR NOT EXISTS (SELECT 1 FROM memory s WHERE s.id = m.superseded_by)) \
         ORDER BY m.id"
    ))?;
    let rows = stmt.query_map([], |r| Ok(DanglingSuperseded { id: r.get(0)?, superseded_by: r.get(1)?, cause: r.get(2)? }))?;
    rows.collect()
}

fn dangling_revived(c: &Connection) -> rusqlite::Result<Vec<DanglingRevived>> {
    let mut stmt = c.prepare(
        "SELECT m.id, m.revived_from FROM memory m \
         WHERE m.deleted_at IS NULL AND m.revived_from IS NOT NULL \
           AND NOT EXISTS (SELECT 1 FROM memory r WHERE r.id = m.revived_from) \
         ORDER BY m.id",
    )?;
    let rows = stmt.query_map([], |r| Ok(DanglingRevived { id: r.get(0)?, revived_from: r.get(1)? }))?;
    rows.collect()
}

fn hash_duplicates(c: &Connection) -> rusqlite::Result<Vec<HashDuplicates>> {
    let mut stmt = c.prepare(&format!(
        "SELECT agent_id, project_id, content_hash, id FROM ( \
             SELECT agent_id, project_id, content_hash, id, \
                    COUNT(*) OVER (PARTITION BY agent_id, COALESCE(project_id, ''), content_hash) AS n \
             FROM memory WHERE {LIVE} AND content_hash IS NOT NULL) \
         WHERE n > 1 \
         ORDER BY agent_id, COALESCE(project_id, ''), content_hash, id"
    ))?;
    let rows = stmt.query_map([], |r| {
        Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, String>(2)?, r.get::<_, i64>(3)?))
    })?;
    let mut found: Vec<(HashDuplicates, String)> = Vec::new();
    for row in rows {
        let (agent_id, project_id, hash, id) = row?;
        match found.last_mut() {
            Some((group, h)) if *h == hash && same_scope(&group.agent_id, &group.project_id, &agent_id, &project_id) => {
                group.ids.push(id)
            }
            _ => found.push((HashDuplicates { agent_id, project_id, ids: vec![id] }, hash)),
        }
    }
    Ok(found.into_iter().map(|(group, _)| group).collect())
}

/// A link to a row that is gone is not a fault by itself: `purge` keeps it,
/// with its id, to say which piece is missing. What it must not keep is the
/// opening of the save, which is the purged row's text.
fn purged_headers(c: &Connection) -> rusqlite::Result<Vec<PurgedHeader>> {
    let mut stmt = c.prepare(
        "SELECT p.save_id, p.memory_id FROM memory_piece p \
         WHERE p.piece = 1 \
           AND NOT EXISTS (SELECT 1 FROM memory m WHERE m.id = p.memory_id) \
           AND EXISTS (SELECT 1 FROM memory_piece q WHERE q.save_id = p.save_id AND q.context <> '') \
         ORDER BY p.save_id",
    )?;
    let rows = stmt.query_map([], |r| Ok(PurgedHeader { save_id: r.get(0)?, first_piece: r.get(1)? }))?;
    rows.collect()
}

/// The same reading of `disputed_against` the writers use: a value that is
/// not valid JSON names nothing.
fn lonely_disputes(c: &Connection) -> rusqlite::Result<Vec<i64>> {
    let mut stmt = c.prepare(
        "SELECT m.id FROM memory m \
         WHERE m.deleted_at IS NULL AND m.status = 'disputed' \
           AND NOT EXISTS ( \
               SELECT 1 FROM json_each(CASE WHEN json_valid(m.disputed_against) THEN m.disputed_against END) j \
               JOIN memory p ON p.id = j.value \
               WHERE p.deleted_at IS NULL AND p.status <> 'superseded') \
         ORDER BY m.id",
    )?;
    let rows = stmt.query_map([], |r| r.get(0))?;
    rows.collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::db::seed_on_disk;
    use crate::memory::store_lock::StoreHolder;
    use crate::memory::MemoryStore;

    /// A row with what each rule reads; every other column takes a default.
    struct Row {
        agent: Option<&'static str>,
        project: Option<&'static str>,
        content: &'static str,
        status: &'static str,
        topic: Option<&'static str>,
        superseded_by: Option<i64>,
        cause: Option<&'static str>,
        revived_from: Option<i64>,
        disputed_against: Option<String>,
        deleted: bool,
    }

    fn row(content: &'static str) -> Row {
        Row {
            agent: Some("agent-a"),
            project: Some("proj-1a2b"),
            content,
            status: "active",
            topic: None,
            superseded_by: None,
            cause: None,
            revived_from: None,
            disputed_against: None,
            deleted: false,
        }
    }

    fn insert(c: &rusqlite::Connection, r: &Row) -> i64 {
        c.execute(
            "INSERT INTO memory \
             (scope, agent_id, project_id, content, content_hash, memory_type, source, \
              created_at, created_by_type, created_by_ref, last_modified_at, last_accessed_at, \
              importance, tier, status, topic_key, superseded_by, supersede_cause, revived_from, \
              disputed_against, deleted_at) \
             VALUES (?11, ?1, ?2, ?3, content_hash(?3), 'semantic_fact', 'explicit', \
              0, 'agent', 'x', 0, 0, 5.0, 3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            rusqlite::params![
                r.agent,
                r.project,
                r.content,
                r.status,
                r.topic,
                r.superseded_by,
                r.cause,
                r.revived_from,
                r.disputed_against.as_deref(),
                r.deleted.then_some(1_i64),
                match (r.agent, r.project) {
                    (Some(_), _) => "agent",
                    (None, Some(_)) => "project",
                    (None, None) => "global",
                }
            ],
        )
        .unwrap();
        c.last_insert_rowid()
    }

    fn link(c: &rusqlite::Connection, save_id: i64, piece: i64, memory_id: i64, context: &str) {
        c.execute(
            "INSERT INTO memory_piece (save_id, piece, memory_id, context) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![save_id, piece, memory_id, context],
        )
        .unwrap();
    }

    /// A store at this binary's schema, written by `fill` and closed, so the
    /// inspection reads the file alone.
    async fn store_with(fill: impl FnOnce(&rusqlite::Connection) + Send + 'static) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::open(dir.path(), StoreHolder::Eval).await.unwrap();
        store
            .conn
            .call(move |c| -> rusqlite::Result<()> {
                fill(c);
                Ok(())
            })
            .await
            .unwrap();
        store.conn.clone().close().await.unwrap();
        drop(store);
        dir
    }

    async fn invariants_of(fill: impl FnOnce(&rusqlite::Connection) + Send + 'static) -> Invariants {
        let dir = store_with(fill).await;
        let found = inspect(dir.path());
        assert_eq!(found.schema.state, ProbeState::Schema(SchemaPlan::UpToDate));
        found.facts.expect("an up-to-date store is read").invariants
    }

    fn files_in(dir: &Path) -> Vec<(String, Vec<u8>)> {
        let mut files: Vec<(String, Vec<u8>)> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.is_file())
            .map(|p| (p.file_name().unwrap().to_string_lossy().into_owned(), std::fs::read(&p).unwrap()))
            .collect();
        files.sort();
        files
    }

    /// A store where every rule holds, with something under each one: a
    /// topic with a superseded revision, a long save, a judged verdict, a
    /// revived row and a live dispute.
    fn healthy(c: &rusqlite::Connection) {
        let old = insert(c, &Row { topic: Some("auth/tokens"), ..row("tokens last a day") });
        let new = insert(c, &Row { topic: Some("auth/tokens"), ..row("tokens last an hour") });
        c.execute(
            "UPDATE memory SET status = 'superseded', superseded_by = ?2, supersede_cause = 'revision' WHERE id = ?1",
            [old, new],
        )
        .unwrap();
        let first = insert(c, &Row { topic: Some("guide/setup"), ..row("setup, part one") });
        let second = insert(c, &Row { topic: Some("guide/setup"), ..row("setup, part two") });
        link(c, 1, 1, first, "");
        link(c, 1, 2, second, "setup,");
        let deleted = insert(c, &Row { deleted: true, ..row("a deleted current version") });
        insert(c, &Row { revived_from: Some(deleted), ..row("a revived version") });
        let rival = insert(c, &row("the rival"));
        insert(c, &Row { status: "disputed", disputed_against: Some(format!("[{rival}]")), ..row("the challenger") });
        insert(c, &Row { agent: Some("agent-b"), ..row("tokens last an hour") });
        insert(c, &Row { project: None, ..row("tokens last an hour") });
    }

    #[tokio::test]
    async fn a_healthy_store_breaks_no_invariant() {
        let found = invariants_of(healthy).await;
        assert!(found.is_empty(), "{found:?}");
    }

    #[tokio::test]
    async fn two_live_saves_of_one_topic_are_found() {
        let found = invariants_of(|c| {
            insert(c, &Row { topic: Some("auth/tokens"), ..row("tokens last a day") });
            insert(c, &Row { topic: Some("auth/tokens"), ..row("tokens last an hour") });
            insert(c, &Row { topic: Some("auth/tokens"), agent: Some("agent-b"), ..row("another agent's") });
        })
        .await;
        assert_eq!(
            found.topic_versions,
            [TopicVersions { agent_id: Some("agent-a".into()), project_id: Some("proj-1a2b".into()), saves: vec![1, 2] }]
        );
        assert_eq!(Invariants { topic_versions: Vec::new(), ..found }, Invariants::default());
    }

    #[tokio::test]
    async fn the_pieces_of_one_save_are_one_version() {
        let found = invariants_of(|c| {
            let ids: Vec<i64> =
                ["one", "two", "three"].iter().map(|t| insert(c, &Row { topic: Some("guide/setup"), ..row(*t) })).collect();
            for (n, id) in ids.iter().enumerate() {
                link(c, 7, n as i64 + 1, *id, if n == 0 { "" } else { "one" });
            }
            let other = insert(c, &Row { topic: Some("guide/setup"), ..row("a second save") });
            insert(c, &Row { topic: Some("guide/setup"), project: None, ..row("another project") });
            assert_eq!(other, 4);
        })
        .await;
        assert_eq!(found.topic_versions.len(), 1);
        assert_eq!(found.topic_versions[0].saves, [1, 4], "the three pieces count once, by their first row");
    }

    #[tokio::test]
    async fn a_superseded_row_whose_winner_is_gone_is_found() {
        let found = invariants_of(|c| {
            let revision = insert(c, &Row { status: "superseded", superseded_by: Some(90), cause: Some("revision"), ..row("a") });
            let verdict = insert(c, &Row { status: "superseded", superseded_by: Some(91), cause: Some("verdict"), ..row("b") });
            let unlinked = insert(c, &Row { status: "superseded", ..row("c") });
            insert(c, &Row { status: "superseded", superseded_by: Some(92), deleted: true, ..row("d") });
            assert_eq!((revision, verdict, unlinked), (1, 2, 3));
        })
        .await;
        assert_eq!(
            found.dangling_superseded,
            [
                DanglingSuperseded { id: 1, superseded_by: Some(90), cause: Some("revision".into()) },
                DanglingSuperseded { id: 2, superseded_by: Some(91), cause: Some("verdict".into()) },
                DanglingSuperseded { id: 3, superseded_by: None, cause: None },
            ],
            "a deleted row is not read"
        );
    }

    #[tokio::test]
    async fn a_dangling_revived_from_is_found() {
        let found = invariants_of(|c| {
            insert(c, &Row { revived_from: Some(77), ..row("brought back by a purged row") });
        })
        .await;
        assert_eq!(found.dangling_revived, [DanglingRevived { id: 1, revived_from: 77 }]);
    }

    #[tokio::test]
    async fn live_rows_with_one_hash_are_found() {
        let found = invariants_of(|c| {
            insert(c, &row("the same text"));
            insert(c, &row("  the same text "));
            insert(c, &Row { deleted: true, ..row("the same text") });
            insert(c, &Row { status: "superseded", superseded_by: Some(1), ..row("the same text") });
        })
        .await;
        assert_eq!(
            found.hash_duplicates,
            [HashDuplicates { agent_id: Some("agent-a".into()), project_id: Some("proj-1a2b".into()), ids: vec![1, 2] }],
            "trimmed text hashes the same; deleted and superseded rows are not live"
        );
    }

    #[tokio::test]
    async fn a_purged_header_left_in_the_links_is_found() {
        let found = invariants_of(|c| {
            let second = insert(c, &row("part two"));
            link(c, 3, 1, 40, "");
            link(c, 3, 2, second, "the purged opening");
            let kept = insert(c, &row("part two of a purge that cleaned up"));
            link(c, 4, 1, 41, "");
            link(c, 4, 2, kept, "");
        })
        .await;
        assert_eq!(found.purged_headers, [PurgedHeader { save_id: 3, first_piece: 40 }]);
    }

    #[tokio::test]
    async fn a_dispute_without_a_live_rival_is_found() {
        let found = invariants_of(|c| {
            let gone = insert(c, &Row { deleted: true, ..row("deleted rival") });
            assert_eq!(gone, 1);
            insert(c, &Row { status: "disputed", disputed_against: Some("[1, 99]".into()), ..row("challenger") });
            insert(c, &Row { status: "disputed", disputed_against: Some("not json".into()), ..row("malformed") });
        })
        .await;
        assert_eq!(found.lonely_disputes, [2, 3]);
    }

    #[tokio::test]
    async fn the_facts_name_agents_projects_the_outbox_and_the_contract() {
        let dir = store_with(|c| {
            insert(c, &row("one"));
            insert(c, &Row { deleted: true, ..row("two") });
            insert(c, &Row { agent: None, project: None, ..row("global") });
            c.execute("INSERT INTO project (id, root_path, created_at) VALUES ('proj-1a2b', '/work/proj', 0)", []).unwrap();
            for (op, state) in [("op-1", "pending"), ("op-2", "failed")] {
                c.execute(
                    "INSERT INTO ingest_outbox (operation_id, agent_id, content, event, state, attempts, created_at, updated_at) \
                     VALUES (?1, 'agent-a', 'text', '{}', ?2, 3, 0, 0)",
                    [op, state],
                )
                .unwrap();
            }
            c.execute(
                "INSERT INTO embedding_contract (id, model_id, dim, metric, normalized, query_prefix, passage_prefix, created_at) \
                 VALUES (1, 'some-model', 384, 'l2_unit', 1, 'query: ', 'query: ', 0)",
                [],
            )
            .unwrap();
        })
        .await;
        let facts = inspect(dir.path()).facts.unwrap();
        assert_eq!(
            facts.agents,
            [
                AgentRows { agent_id: None, live: 1, total: 1 },
                AgentRows { agent_id: Some("agent-a".into()), live: 1, total: 2 },
            ]
        );
        assert_eq!(facts.projects, [ProjectRow { id: "proj-1a2b".into(), root_path: "/work/proj".into() }]);
        assert_eq!(
            facts.outbox,
            OutboxFacts {
                pending: 1,
                failed: vec![FailedSave { operation_id: "op-2".into(), agent_id: "agent-a".into(), attempts: 3 }]
            }
        );
        assert_eq!(facts.contract.map(|c| (c.model_id, c.dim)), Some(("some-model".into(), 384)));
    }

    #[tokio::test]
    async fn inspecting_leaves_the_store_unchanged() {
        let dir = store_with(healthy).await;
        let before = files_in(dir.path());
        assert!(inspect(dir.path()).facts.is_some());
        assert_eq!(files_in(dir.path()), before, "a stopped store gains no file and no byte");

        let live = tempfile::tempdir().unwrap();
        let store = MemoryStore::open(live.path(), StoreHolder::Daemon).await.unwrap();
        store
            .conn
            .call(|c| -> rusqlite::Result<()> {
                healthy(c);
                Ok(())
            })
            .await
            .unwrap();
        let wal = live.path().join("memory.db-wal");
        assert!(wal.exists(), "the daemon's writes sit in the WAL");
        let (db_before, wal_before) = (std::fs::read(live.path().join("memory.db")).unwrap(), std::fs::read(&wal).unwrap());
        let found = inspect(live.path());
        assert!(found.facts.expect("read beside the daemon").invariants.is_empty());
        assert_eq!(std::fs::read(live.path().join("memory.db")).unwrap(), db_before);
        assert_eq!(std::fs::read(&wal).unwrap(), wal_before);
        drop(store);
    }

    #[tokio::test]
    async fn a_store_at_an_older_schema_is_read_with_what_it_has() {
        let dir = tempfile::tempdir().unwrap();
        seed_on_disk(dir.path(), Some(10)).await;
        let before = files_in(dir.path());
        let found = inspect(dir.path());
        assert!(matches!(found.schema.state, ProbeState::Schema(SchemaPlan::Pending { from: 10, .. })));
        let facts = found.facts.expect("an older store is read");
        assert_eq!(facts.agents, [AgentRows { agent_id: Some("agent-seed".into()), live: 1, total: 1 }]);
        assert!(facts.invariants.is_empty());
        assert_eq!(facts.outbox, OutboxFacts::default(), "V14 is not applied");
        assert_eq!(files_in(dir.path()), before);
    }

    #[tokio::test]
    async fn a_newer_store_is_not_read_past_its_history() {
        let dir = tempfile::tempdir().unwrap();
        seed_on_disk(dir.path(), None).await;
        let c = rusqlite::Connection::open(dir.path().join("memory.db")).unwrap();
        c.execute(
            "INSERT INTO refinery_schema_history (version, name, applied_on, checksum) VALUES (999, 'future', '', '1')",
            [],
        )
        .unwrap();
        drop(c);
        let found = inspect(dir.path());
        assert!(matches!(found.schema.state, ProbeState::Schema(SchemaPlan::Newer { db: 999, .. })));
        assert_eq!(found.facts, None);
    }

    #[test]
    fn a_missing_store_has_no_facts() {
        let dir = tempfile::tempdir().unwrap();
        let found = inspect(dir.path());
        assert_eq!(found.schema.state, ProbeState::Missing);
        assert_eq!(found.facts, None);
        assert!(!dir.path().join("memory.db").exists(), "no store is created");
    }
}
