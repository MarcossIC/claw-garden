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
use crate::memory::merge::MAX_MERGE_HOPS;

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
    /// `project_remote`, by project and then remote.
    pub remotes: Vec<RemoteRow>,
    pub outbox: OutboxFacts,
    pub invariants: Invariants,
    /// The projects of `project`, tombstones too, by id; `None` before V27,
    /// when `project` held folders.
    pub registry: Option<Vec<RegistryRow>>,
    /// What each folder of `project_path` recorded (V27), by id.
    pub folder_prints: Vec<FolderPrint>,
    /// The pairs of `project_suggestion` someone answered, each lesser id first.
    pub answered_pairs: Vec<(String, String)>,
    /// The ids rows of the three tables are saved under, with how many.
    pub id_rows: Vec<IdRows>,
    /// The deletes nobody undid or purged (V29); `None` before the table
    /// exists, when no project can be deleted.
    pub open_deletes: Option<Vec<OpenDelete>>,
    /// The subfolders declared as projects (V30), by repository and subpath;
    /// `None` before the table exists, when none could be declared.
    pub subfolders: Option<Vec<DeclaredSubfolder>>,
}

/// A row of `project_subfolder`, as it is stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredSubfolder {
    pub parent_id: String,
    pub subpath: String,
    pub project_id: String,
}

/// A delete nobody undid or purged, by the project it took.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenDelete {
    pub project_id: String,
    pub delete_id: i64,
}

/// A project of `project`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryRow {
    pub id: String,
    pub name: String,
    pub created_at: i64,
    /// The project a merge took it into (V28); `None` for a live one.
    pub merged_into: Option<String>,
    /// When it was deleted (V29); `None` for one that is not.
    pub deleted_at: Option<i64>,
}

/// What a folder recorded: the project it leads to and its fingerprints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderPrint {
    pub root_id: String,
    pub project_id: Option<String>,
    pub remote: Option<String>,
    pub root_commit: Option<String>,
}

/// An id rows are saved under, deleted ones too, with how many of each table
/// and how many of them are live.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdRows {
    pub project_id: String,
    pub memories: i64,
    pub trajectories: i64,
    pub skills: i64,
    pub live_memories: i64,
    pub live_trajectories: i64,
    pub live_skills: i64,
}

/// Two live projects that may be one, the older first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Duplicate {
    /// `same_name` or `same_first_commit`.
    pub reason: &'static str,
    pub older: RegistryRow,
    pub newer: RegistryRow,
    /// The first commit their folders share, for `same_first_commit`.
    pub root_commit: Option<String>,
}

/// The project `id` leads to through the merges of `registry`, deleted or
/// not; `None` when `id` is no project or its chain ends in none (a cycle,
/// one too long, an id `project` does not have).
pub fn live_of<'a>(registry: &'a [RegistryRow], id: &str) -> Option<&'a RegistryRow> {
    let mut at = registry.iter().find(|p| p.id == id)?;
    for _ in 0..=MAX_MERGE_HOPS {
        match &at.merged_into {
            None => return Some(at),
            Some(into) => at = registry.iter().find(|p| p.id == *into)?,
        }
    }
    None
}

/// Two projects, neither merged nor deleted, that may be one: with the same
/// name, in any case, or with folders of one first commit that no pair of
/// remotes tells apart (a fork has a remote of its own). A pair someone
/// answered is not one; the older of each pair, the one that usually has the
/// history, comes first.
pub fn duplicates(registry: &[RegistryRow], folders: &[FolderPrint], answered: &[(String, String)]) -> Vec<Duplicate> {
    let live: Vec<&RegistryRow> = registry.iter().filter(|p| p.merged_into.is_none() && p.deleted_at.is_none()).collect();
    let answered = |a: &str, b: &str| {
        let (a, b) = if a < b { (a, b) } else { (b, a) };
        answered.iter().any(|(x, y)| x == a && y == b)
    };
    let pair = |reason, a: &RegistryRow, b: &RegistryRow, root_commit: Option<&str>| {
        let (older, newer) = if (a.created_at, &a.id) <= (b.created_at, &b.id) { (a, b) } else { (b, a) };
        Duplicate { reason, older: older.clone(), newer: newer.clone(), root_commit: root_commit.map(str::to_string) }
    };
    let mut found = Vec::new();
    for (i, a) in live.iter().enumerate() {
        for b in &live[i + 1..] {
            if a.name.to_lowercase() == b.name.to_lowercase() && !answered(&a.id, &b.id) {
                found.push(pair("same_name", a, b, None));
            }
        }
    }
    let mut by_commit: std::collections::BTreeMap<&str, Vec<(&RegistryRow, Option<&str>)>> = std::collections::BTreeMap::new();
    for f in folders {
        let project = live_of(registry, f.project_id.as_deref().unwrap_or(&f.root_id)).filter(|p| p.deleted_at.is_none());
        let (Some(commit), Some(project)) = (f.root_commit.as_deref(), project) else {
            continue;
        };
        by_commit.entry(commit).or_default().push((project, f.remote.as_deref()));
    }
    for (commit, sharing) in by_commit {
        let mut seen: Vec<(String, String)> = Vec::new();
        for (i, (a, a_remote)) in sharing.iter().enumerate() {
            for (b, b_remote) in &sharing[i + 1..] {
                let told_apart = matches!((a_remote, b_remote), (Some(x), Some(y)) if x != y);
                if a.id == b.id || told_apart || answered(&a.id, &b.id) {
                    continue;
                }
                let found_pair = pair("same_first_commit", a, b, Some(commit));
                let ids = (found_pair.older.id.clone(), found_pair.newer.id.clone());
                if !seen.contains(&ids) {
                    seen.push(ids);
                    found.push(found_pair);
                }
            }
        }
    }
    found
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

/// A remote and the project its clones share.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteRow {
    pub remote: String,
    pub project_id: String,
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
    /// More than one live save under one topic key of a project.
    pub topic_versions: Vec<TopicVersions>,
    /// A superseded row whose `superseded_by` is NULL or names no row.
    pub dangling_superseded: Vec<DanglingSuperseded>,
    /// A row whose `revived_from` names no row.
    pub dangling_revived: Vec<DanglingRevived>,
    /// Live rows of one project with one `content_hash`.
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
    /// Who saved the saves, each once, `None` for a row no agent saved.
    pub agents: Vec<Option<String>>,
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
    /// Who saved the rows, each once, `None` for a row no agent saved.
    pub agents: Vec<Option<String>>,
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
        // The folders are in `project_path` from V27 and in `project` before.
        projects: match ["project_path", "project"].into_iter().find(|t| has.table(t)) {
            Some(folders) => projects(c, folders)?,
            None => Vec::new(),
        },
        remotes: if has.table("project_remote") { remotes(c)? } else { Vec::new() },
        outbox: if has.table("ingest_outbox") { outbox(c)? } else { OutboxFacts::default() },
        invariants: Invariants {
            topic_versions: if has.memory_column("topic_key") { topic_versions(c, &has)? } else { Vec::new() },
            dangling_superseded: dangling_superseded(c, &has)?,
            dangling_revived: if has.memory_column("revived_from") { dangling_revived(c)? } else { Vec::new() },
            hash_duplicates: if has.memory_column("content_hash") { hash_duplicates(c)? } else { Vec::new() },
            purged_headers: if has.table("memory_piece") { purged_headers(c)? } else { Vec::new() },
            lonely_disputes: lonely_disputes(c)?,
        },
        // `project` is the reference of the projects from V27, when the
        // folders moved to `project_path`.
        registry: if has.table("project_path") { Some(registry(c, &has)?) } else { None },
        folder_prints: if has.table("project_path") { folder_prints(c)? } else { Vec::new() },
        answered_pairs: if has.table("project_suggestion") { answered_pairs(c)? } else { Vec::new() },
        id_rows: id_rows(c)?,
        open_deletes: if has.table("project_delete") { Some(open_deletes(c)?) } else { None },
        subfolders: if has.table("project_subfolder") { Some(subfolders(c)?) } else { None },
    })
}

fn subfolders(c: &Connection) -> rusqlite::Result<Vec<DeclaredSubfolder>> {
    let mut stmt = c.prepare("SELECT parent_id, subpath, project_id FROM project_subfolder ORDER BY parent_id, subpath")?;
    let rows = stmt.query_map([], |r| Ok(DeclaredSubfolder { parent_id: r.get(0)?, subpath: r.get(1)?, project_id: r.get(2)? }))?;
    rows.collect()
}

/// The projects of `project`, tombstones and deleted ones too, with what the
/// schema of the store has of them.
pub(crate) fn registry(c: &Connection, has: &Has) -> rusqlite::Result<Vec<RegistryRow>> {
    let column = |name| if has.project_column(name) { name } else { "NULL" };
    let (merged_into, deleted_at) = (column("merged_into"), column("deleted_at"));
    let mut stmt =
        c.prepare(&format!("SELECT id, name, created_at, {merged_into}, {deleted_at} FROM project ORDER BY id"))?;
    let rows = stmt.query_map([], |r| {
        Ok(RegistryRow { id: r.get(0)?, name: r.get(1)?, created_at: r.get(2)?, merged_into: r.get(3)?, deleted_at: r.get(4)? })
    })?;
    rows.collect()
}

fn open_deletes(c: &Connection) -> rusqlite::Result<Vec<OpenDelete>> {
    let mut stmt = c.prepare(
        "SELECT project_id, delete_id FROM project_delete WHERE undone_at IS NULL AND purged_at IS NULL ORDER BY delete_id",
    )?;
    let rows = stmt.query_map([], |r| Ok(OpenDelete { project_id: r.get(0)?, delete_id: r.get(1)? }))?;
    rows.collect()
}

pub(crate) fn folder_prints(c: &Connection) -> rusqlite::Result<Vec<FolderPrint>> {
    let mut stmt = c.prepare("SELECT id, project_id, remote, root_commit FROM project_path ORDER BY id")?;
    let rows = stmt.query_map([], |r| {
        Ok(FolderPrint { root_id: r.get(0)?, project_id: r.get(1)?, remote: r.get(2)?, root_commit: r.get(3)? })
    })?;
    rows.collect()
}

pub(crate) fn answered_pairs(c: &Connection) -> rusqlite::Result<Vec<(String, String)>> {
    let mut stmt = c.prepare(
        "SELECT min(project_id, other_id), max(project_id, other_id) FROM project_suggestion \
         WHERE answer IS NOT NULL ORDER BY 1, 2",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    rows.collect()
}

fn id_rows(c: &Connection) -> rusqlite::Result<Vec<IdRows>> {
    let mut stmt = c.prepare(
        "SELECT project_id, SUM(t = 'm'), SUM(t = 't'), SUM(t = 's'), \
                SUM(t = 'm' AND live), SUM(t = 't' AND live), SUM(t = 's' AND live) FROM ( \
           SELECT project_id, 'm' AS t, deleted_at IS NULL AS live FROM memory \
           UNION ALL SELECT project_id, 't', deleted_at IS NULL FROM episodic_trajectory \
           UNION ALL SELECT project_id, 's', deleted_at IS NULL FROM procedural_skill) \
         WHERE project_id IS NOT NULL GROUP BY project_id ORDER BY project_id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(IdRows {
            project_id: r.get(0)?,
            memories: r.get(1)?,
            trajectories: r.get(2)?,
            skills: r.get(3)?,
            live_memories: r.get(4)?,
            live_trajectories: r.get(5)?,
            live_skills: r.get(6)?,
        })
    })?;
    rows.collect()
}

/// The tables, and the columns of `memory` and `project`, the store has.
pub(crate) struct Has {
    tables: Vec<String>,
    memory_columns: Vec<String>,
    project_columns: Vec<String>,
}

impl Has {
    pub(crate) fn of(c: &Connection) -> rusqlite::Result<Has> {
        let names = |sql: &str| -> rusqlite::Result<Vec<String>> {
            let mut stmt = c.prepare(sql)?;
            let rows = stmt.query_map([], |r| r.get(0))?;
            rows.collect()
        };
        Ok(Has {
            tables: names("SELECT name FROM sqlite_master WHERE type = 'table'")?,
            memory_columns: names("SELECT name FROM pragma_table_info('memory')")?,
            project_columns: names("SELECT name FROM pragma_table_info('project')")?,
        })
    }

    fn project_column(&self, name: &str) -> bool {
        self.project_columns.iter().any(|col| col == name)
    }

    fn table(&self, name: &str) -> bool {
        self.tables.iter().any(|t| t == name)
    }

    fn memory_column(&self, name: &str) -> bool {
        self.memory_columns.iter().any(|col| col == name)
    }
}

const LIVE: &str = "deleted_at IS NULL AND status <> 'superseded'";

pub(crate) fn contract(c: &Connection) -> rusqlite::Result<Option<StoredContract>> {
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

fn projects(c: &Connection, folders: &str) -> rusqlite::Result<Vec<ProjectRow>> {
    let mut stmt = c.prepare(&format!("SELECT id, root_path FROM {folders} ORDER BY id"))?;
    let rows = stmt.query_map([], |r| Ok(ProjectRow { id: r.get(0)?, root_path: r.get(1)? }))?;
    rows.collect()
}

fn remotes(c: &Connection) -> rusqlite::Result<Vec<RemoteRow>> {
    let mut stmt = c.prepare("SELECT remote, project_id FROM project_remote ORDER BY project_id, remote")?;
    let rows = stmt.query_map([], |r| Ok(RemoteRow { remote: r.get(0)?, project_id: r.get(1)? }))?;
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

/// One live save per key in a project, whoever saved it: the pieces of a long
/// save share the key and are one version, so rows are grouped by save before
/// they are counted. Two agents the sharing policy hides from each other may
/// hold one key each, which this cannot tell from a fault without the policy.
fn topic_versions(c: &Connection, has: &Has) -> rusqlite::Result<Vec<TopicVersions>> {
    let save = if has.table("memory_piece") {
        "COALESCE((SELECT 's' || MIN(p.save_id) FROM memory_piece p WHERE p.memory_id = m.id), 'r' || m.id)"
    } else {
        "'r' || m.id"
    };
    let mut stmt = c.prepare(&format!(
        "SELECT MIN(agent_id), project_id, topic_key, MIN(id) FROM ( \
             SELECT m.id, m.agent_id, m.project_id, m.topic_key, {save} AS save FROM memory m \
             WHERE m.{LIVE} AND m.topic_key IS NOT NULL) \
         GROUP BY COALESCE(project_id, ''), topic_key, save \
         ORDER BY COALESCE(project_id, ''), topic_key, MIN(id)"
    ))?;
    let rows = stmt.query_map([], |r| {
        Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, String>(2)?, r.get::<_, i64>(3)?))
    })?;
    let mut found: Vec<(TopicVersions, String)> = Vec::new();
    for row in rows {
        let (agent_id, project_id, key, first) = row?;
        match found.last_mut() {
            Some((group, k)) if *k == key && same_project(&group.project_id, &project_id) => {
                group.saves.push(first);
                add_agent(&mut group.agents, agent_id);
            }
            _ => found.push((TopicVersions { agents: vec![agent_id], project_id, saves: vec![first] }, key)),
        }
    }
    Ok(found.into_iter().map(|(group, _)| group).filter(|g| g.saves.len() > 1).collect())
}

/// The scope the topic and the dedup indexes key on: a NULL project is the
/// empty one, as their `COALESCE` reads it.
fn same_project(project: &Option<String>, other: &Option<String>) -> bool {
    project.as_deref().unwrap_or("") == other.as_deref().unwrap_or("")
}

/// Names `agent` among `agents` once, in the order they were found.
fn add_agent(agents: &mut Vec<Option<String>>, agent: Option<String>) {
    if !agents.contains(&agent) {
        agents.push(agent);
    }
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
                    COUNT(*) OVER (PARTITION BY COALESCE(project_id, ''), content_hash) AS n \
             FROM memory WHERE {LIVE} AND content_hash IS NOT NULL) \
         WHERE n > 1 \
         ORDER BY COALESCE(project_id, ''), content_hash, id"
    ))?;
    let rows = stmt.query_map([], |r| {
        Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, String>(2)?, r.get::<_, i64>(3)?))
    })?;
    let mut found: Vec<(HashDuplicates, String)> = Vec::new();
    for row in rows {
        let (agent_id, project_id, hash, id) = row?;
        match found.last_mut() {
            Some((group, h)) if *h == hash && same_project(&group.project_id, &project_id) => {
                group.ids.push(id);
                add_agent(&mut group.agents, agent_id);
            }
            _ => found.push((HashDuplicates { agents: vec![agent_id], project_id, ids: vec![id] }, hash)),
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
        insert(c, &Row { agent: Some("agent-b"), project: Some("proj-3c4d"), ..row("tokens last an hour") });
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
            insert(c, &Row { topic: Some("auth/tokens"), project: Some("proj-3c4d"), ..row("another project's") });
        })
        .await;
        assert_eq!(
            found.topic_versions,
            [TopicVersions {
                agents: vec![Some("agent-a".into()), Some("agent-b".into())],
                project_id: Some("proj-1a2b".into()),
                saves: vec![1, 2, 3]
            }],
            "a key is the project's, whoever saved under it"
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
            insert(c, &Row { agent: None, ..row("the same text") });
            insert(c, &Row { agent: Some("agent-b"), project: Some("proj-3c4d"), ..row("the same text") });
        })
        .await;
        assert_eq!(
            found.hash_duplicates,
            [HashDuplicates { agents: vec![Some("agent-a".into()), None], project_id: Some("proj-1a2b".into()), ids: vec![1, 2, 5] }],
            "trimmed text hashes the same; deleted and superseded rows are not live; another project keeps its own"
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
            c.execute("INSERT INTO project_path (id, root_path, created_at) VALUES ('proj-1a2b', '/work/proj', 0)", []).unwrap();
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
    async fn the_facts_name_the_remotes_of_each_project() {
        let dir = store_with(|c| {
            for (remote, project) in
                [("github.com/acme/b", "app-1a2b"), ("github.com/acme/a", "app-1a2b"), ("github.com/acme/w", "w-3c4d")]
            {
                c.execute(
                    "INSERT INTO project_remote (remote, project_id, created_at) VALUES (?1, ?2, 0)",
                    [remote, project],
                )
                .unwrap();
            }
        })
        .await;
        let facts = inspect(dir.path()).facts.unwrap();
        let remote = |remote: &str, project_id: &str| RemoteRow { remote: remote.into(), project_id: project_id.into() };
        assert_eq!(
            facts.remotes,
            [remote("github.com/acme/a", "app-1a2b"), remote("github.com/acme/b", "app-1a2b"), remote("github.com/acme/w", "w-3c4d")]
        );
    }

    #[tokio::test]
    async fn the_facts_name_the_projects_their_folders_answers_and_rows() {
        let dir = store_with(|c| {
            c.execute_batch(
                "INSERT INTO project (id, name, born, reviewed, created_at) VALUES \
                   ('web-3c4d', 'web', 'root', 1, 2), ('api-5e6f', 'Web', 'root', 1, 3);
                 INSERT INTO project (id, name, born, reviewed, created_at, merged_into, merged_at) \
                   VALUES ('app-1a2b', 'app', 'root', 0, 1, 'web-3c4d', 9);
                 INSERT INTO project_path (id, root_path, created_at, project_id, root_commit) \
                   VALUES ('web-3c4d', '/work/web', 2, 'web-3c4d', 'c1');
                 INSERT INTO project_suggestion (project_id, other_id, reason, created_at, answer) VALUES \
                   ('web-3c4d', 'api-5e6f', 'user', 4, 'apart'), ('web-3c4d', 'app-1a2b', 'root_commit', 5, NULL);
                 INSERT INTO episodic_trajectory \
                   (scope, agent_id, project_id, session_id, task, created_at, last_modified_at, last_accessed_at) \
                   VALUES ('agent', 'a', 'orphan-9f9f', 's', 't', 1, 1, 1);",
            )
            .unwrap();
            for (content, project) in [("one", "web-3c4d"), ("two", "web-3c4d"), ("left", "app-1a2b")] {
                insert(c, &Row { project: Some(project), ..row(content) });
            }
        })
        .await;

        let facts = inspect(dir.path()).facts.unwrap();

        let project = |id: &str, name: &str, created_at, merged_into: Option<&str>| RegistryRow {
            id: id.into(),
            name: name.into(),
            created_at,
            merged_into: merged_into.map(str::to_string),
            deleted_at: None,
        };
        assert_eq!(
            facts.registry,
            Some(vec![project("api-5e6f", "Web", 3, None), project("app-1a2b", "app", 1, Some("web-3c4d")), project("web-3c4d", "web", 2, None)])
        );
        let print = FolderPrint { root_id: "web-3c4d".into(), project_id: Some("web-3c4d".into()), remote: None, root_commit: Some("c1".into()) };
        assert_eq!(facts.folder_prints, vec![print]);
        assert_eq!(facts.answered_pairs, vec![("api-5e6f".to_string(), "web-3c4d".to_string())], "a pending one is not answered");
        let rows = |id: &str, memories, trajectories| IdRows {
            project_id: id.into(),
            memories,
            trajectories,
            skills: 0,
            live_memories: memories,
            live_trajectories: trajectories,
            live_skills: 0,
        };
        assert_eq!(facts.id_rows, vec![rows("app-1a2b", 1, 0), rows("orphan-9f9f", 0, 1), rows("web-3c4d", 2, 0)]);

        let older = tempfile::tempdir().unwrap();
        seed_on_disk(older.path(), Some(27)).await;
        let c = rusqlite::Connection::open(older.path().join("memory.db")).unwrap();
        c.execute("INSERT INTO project (id, name, born, reviewed, created_at) VALUES ('web-3c4d', 'web', 'root', 1, 2)", [])
            .unwrap();
        drop(c);
        let facts = inspect(older.path()).facts.unwrap();
        assert_eq!(facts.registry, Some(vec![project("web-3c4d", "web", 2, None)]), "V27 merged nothing yet");
    }

    #[test]
    fn duplicates_pair_by_name_and_by_first_commit_and_skip_answered_pairs() {
        let project = |id: &str, name: &str, created_at, merged_into: Option<&str>| RegistryRow {
            id: id.into(),
            name: name.into(),
            created_at,
            merged_into: merged_into.map(str::to_string),
            deleted_at: None,
        };
        let print = |id: &str, remote: Option<&str>, commit: &str| FolderPrint {
            root_id: id.into(),
            project_id: Some(id.into()),
            remote: remote.map(str::to_string),
            root_commit: Some(commit.into()),
        };
        // Two that share a name, two whose folders share a first commit (one
        // through a tombstone), and two forks with remotes of their own.
        let registry = vec![
            project("api-1a2b", "App", 1, None),
            project("app-3c4d", "app", 5, None),
            project("lib-5e6f", "lib", 2, None),
            project("lib2-7a8b", "lib2", 6, None),
            project("forka-0001", "forka", 3, None),
            project("forkb-0002", "forkb", 4, None),
            project("old-0003", "old", 0, Some("lib-5e6f")),
        ];
        let folders = vec![
            print("lib-5e6f", Some("github.com/acme/lib"), "c1"),
            print("lib2-7a8b", None, "c1"),
            print("old-0003", None, "c1"),
            print("forka-0001", Some("github.com/acme/fork"), "c2"),
            print("forkb-0002", Some("github.com/other/fork"), "c2"),
        ];
        let pairs = |answered: &[(String, String)]| -> Vec<(&'static str, String, String, Option<String>)> {
            duplicates(&registry, &folders, answered)
                .into_iter()
                .map(|d| (d.reason, d.older.id, d.newer.id, d.root_commit))
                .collect()
        };

        assert_eq!(
            pairs(&[]),
            vec![
                ("same_name", "api-1a2b".to_string(), "app-3c4d".to_string(), None),
                ("same_first_commit", "lib-5e6f".to_string(), "lib2-7a8b".to_string(), Some("c1".to_string())),
            ],
            "the older first; a tombstone is its target, and a fork is no duplicate"
        );
        let answered = [("lib-5e6f".to_string(), "lib2-7a8b".to_string())];
        assert_eq!(pairs(&answered), vec![("same_name", "api-1a2b".to_string(), "app-3c4d".to_string(), None)]);
        assert_eq!(live_of(&registry, "old-0003").map(|p| p.id.as_str()), Some("lib-5e6f"));
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
        assert_eq!((facts.registry, facts.folder_prints, facts.answered_pairs), (None, vec![], vec![]), "V27 is not applied");
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

    #[tokio::test]
    async fn the_facts_read_a_deleted_project_where_the_schema_has_one() {
        let dir = store_with(|c| {
            c.execute_batch(
                "INSERT INTO project (id, name, born, reviewed, created_at, deleted_at) VALUES \
                   ('gone-1a2b', 'gone', 'root', 1, 1, NULL), ('web-3c4d', 'web', 'root', 1, 2, NULL);
                 INSERT INTO project_delete (delete_id, project_id, with_memories, deleted_at, undone_at) VALUES \
                   (1, 'gone-1a2b', 1, 3, 4), (2, 'gone-1a2b', 1, 5, NULL), (3, 'web-3c4d', 0, 6, 7);
                 INSERT INTO episodic_trajectory \
                   (scope, agent_id, project_id, session_id, task, created_at, last_modified_at, last_accessed_at) \
                   VALUES ('agent', 'a', 'gone-1a2b', 's', 't', 1, 1, 1);",
            )
            .unwrap();
            insert(c, &Row { project: Some("gone-1a2b"), deleted: true, ..row("one deleted") });
            insert(c, &Row { project: Some("gone-1a2b"), ..row("one live") });
            // After the rows: a live one under a deleted project brings it back.
            c.execute("UPDATE project SET deleted_at = 5 WHERE id = 'gone-1a2b'", []).unwrap();
        })
        .await;

        let facts = inspect(dir.path()).facts.unwrap();

        let deleted: Vec<(&str, Option<i64>)> =
            facts.registry.as_ref().unwrap().iter().map(|p| (p.id.as_str(), p.deleted_at)).collect();
        assert_eq!(deleted, vec![("gone-1a2b", Some(5)), ("web-3c4d", None)]);
        assert_eq!(
            facts.id_rows,
            vec![IdRows {
                project_id: "gone-1a2b".into(),
                memories: 2,
                trajectories: 1,
                skills: 0,
                live_memories: 1,
                live_trajectories: 1,
                live_skills: 0
            }]
        );
        assert_eq!(facts.open_deletes, Some(vec![OpenDelete { project_id: "gone-1a2b".into(), delete_id: 2 }]), "an undone one is not open");

        // At V28 there is no column and no table: nothing is deleted, and nothing is read.
        let older = tempfile::tempdir().unwrap();
        seed_on_disk(older.path(), Some(28)).await;
        let c = rusqlite::Connection::open(older.path().join("memory.db")).unwrap();
        c.execute("INSERT INTO project (id, name, born, reviewed, created_at) VALUES ('web-3c4d', 'web', 'root', 1, 2)", [])
            .unwrap();
        drop(c);
        let facts = inspect(older.path()).facts.unwrap();
        assert_eq!(facts.registry.unwrap()[0].deleted_at, None);
        assert_eq!(facts.open_deletes, None, "V29 is not applied");
    }

    #[tokio::test]
    async fn the_facts_read_the_declared_subfolders_where_the_schema_has_them() {
        let dir = store_with(|c| {
            c.execute_batch(
                "INSERT INTO project_subfolder (parent_id, subpath, project_id, created_at) VALUES \
                   ('mono-1a2b', 'libs/ui', 'ui', 2), ('mono-1a2b', 'apps/web', 'web', 1);",
            )
            .unwrap();
        })
        .await;

        let facts = inspect(dir.path()).facts.unwrap();

        let declared = |subpath: &str, project: &str| DeclaredSubfolder {
            parent_id: "mono-1a2b".into(),
            subpath: subpath.into(),
            project_id: project.into(),
        };
        assert_eq!(facts.subfolders, Some(vec![declared("apps/web", "web"), declared("libs/ui", "ui")]));

        let older = tempfile::tempdir().unwrap();
        seed_on_disk(older.path(), Some(29)).await;
        assert_eq!(inspect(older.path()).facts.unwrap().subfolders, None, "V30 is not applied");
    }

    #[test]
    fn a_deleted_project_is_no_duplicate_and_leads_nowhere_for_a_tombstone() {
        let project = |id: &str, name: &str, created_at, merged_into: Option<&str>, deleted_at| RegistryRow {
            id: id.into(),
            name: name.into(),
            created_at,
            merged_into: merged_into.map(str::to_string),
            deleted_at,
        };
        let registry = vec![
            project("web-3c4d", "web", 1, None, None),
            project("app-1a2b", "Web", 2, None, Some(9)),
            project("old-0003", "old", 0, Some("app-1a2b"), None),
            project("lib2-7a8b", "lib2", 3, None, None),
        ];
        let folders = vec![
            FolderPrint { root_id: "old-0003".into(), project_id: Some("old-0003".into()), remote: None, root_commit: Some("c1".into()) },
            FolderPrint { root_id: "lib2-7a8b".into(), project_id: Some("lib2-7a8b".into()), remote: None, root_commit: Some("c1".into()) },
        ];

        assert_eq!(duplicates(&registry, &folders, &[]), vec![]);
        assert_eq!(live_of(&registry, "old-0003").map(|p| p.id.as_str()), Some("app-1a2b"), "a chain ends in a project, deleted or not");
    }
}
