//! Moving memories to another project, or out of every project.
//!
//! A move changes `project_id`, and `scope` where the CHECK of `memory` needs
//! it, and nothing else: no vector, no text index (`memory_fts` reads
//! `project_id` from `memory` at query time), no detector, no
//! `last_modified_at`. What moves is a unit, every row tied to the one named:
//! the pieces of its saves, its topic's history and what its delete brought
//! back, so a save is never split between two projects and deleting the
//! current version of a key still revives the one before it. A unit that would
//! leave two live rows of one text or one key in the target is refused whole,
//! so a move changes nothing but where rows are and can be undone exactly.
//! Every moved row is logged in `memory_move` (V21).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::path::Path;

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::memory::backup::Reason;
use crate::memory::clock::TimestampMs;
use crate::memory::merge::canonical_in;
use crate::memory::project_delete::deleted_in;
use crate::memory::search::{reader_predicate, viewer_match_expression};
use crate::memory::store_insert::MemoryType;
use crate::memory::{MemoryError, MemoryStore};

/// Why a unit stays where it is. Nothing of it is written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum Refusal {
    #[error("#{row} holds the same text as #{twin}, live in the target; delete one of them and move again")]
    LiveTwin { row: i64, twin: i64 },
    #[error("#{row} is under topic {topic_key}, which #{holder} holds in the target; delete one of them and move again")]
    TopicTaken { row: i64, topic_key: String, holder: i64 },
    #[error("its save or topic history is in more than one project")]
    SpansProjects,
    #[error("its save or topic history holds memories out of your reach")]
    OutOfReach,
}

/// A unit left where it was: its first selected row, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("memory #{id} cannot move: {refusal}")]
pub struct Skipped {
    pub id: i64,
    #[serde(flatten)]
    pub refusal: Refusal,
}

/// A row a move changes. What it skips on the wire reads back empty.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MovedRow {
    pub id: i64,
    pub from_project: Option<String>,
    pub to_project: Option<String>,
    #[serde(skip)]
    pub(crate) from_scope: String,
    #[serde(skip)]
    pub(crate) to_scope: String,
    /// The type of a live preference or decision, which only the user's word
    /// takes out of its project through an agent's door.
    #[serde(skip)]
    pub(crate) protected: Option<String>,
}

/// What a move does, or would do on a dry run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoveReport {
    pub dry_run: bool,
    /// The move's number in `memory_move`; `None` when nothing was written.
    pub move_id: Option<i64>,
    /// Every row that changes project, selected or pulled in.
    pub moved: Vec<MovedRow>,
    /// The rows of `moved` nobody named: they belong to a moved unit.
    pub pulled_in: Vec<i64>,
    /// Selected rows already in their target.
    pub unchanged: Vec<i64>,
    pub skipped: Vec<Skipped>,
    /// Open pairs, newer row first, between a moved row and one left in
    /// another project. They stay: a pair is a pair of ids.
    pub split_pairs: Vec<[i64; 2]>,
    /// How many live preferences and decisions changed project.
    pub protected: usize,
    /// The `pre-move` snapshot taken before writing.
    pub snapshot: Option<String>,
    /// What joining two projects superseded, each row with the one that
    /// stays live in its place.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub superseded: Vec<Supersede>,
    /// The first row of each unit a merge took only part of: its rows in a
    /// project the merge does not join stayed there.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub split: Vec<i64>,
}

impl MoveReport {
    fn absorb(&mut self, other: MoveReport) {
        self.moved.extend(other.moved);
        self.pulled_in.extend(other.pulled_in);
        self.unchanged.extend(other.unchanged);
        self.skipped.extend(other.skipped);
        self.split_pairs.extend(other.split_pairs);
        self.protected += other.protected;
    }
}

/// Why a move was refused before deciding anything. Nothing is written.
#[derive(Debug, thiserror::Error)]
pub enum MoveError {
    #[error("nothing selected: name ids or at least one filter")]
    NoSelection,
    #[error("the text filter cannot search deleted rows: deleting a memory takes it out of the text index")]
    TextOnDeleted,
    #[error("the text filter must contain at least one letter or digit to search for")]
    EmptyText,
    #[error(
        "no project {0} is registered: register its root first (`clawg projects add`, or a bridge started in it)"
    )]
    UnknownProject(String),
    #[error("no move {0}")]
    UnknownMove(i64),
    #[error("project {id} was merged into {into}: its memories are there now")]
    Merged { id: String, into: String },
    /// `delete_id` is the open delete that took it.
    #[error("project {id} is deleted")]
    Deleted { id: String, delete_id: Option<i64> },
    #[error("move {move_id} is part of merge {merge_id}: undo it with `clawg projects merge --undo {merge_id}`")]
    PartOfMerge { move_id: i64, merge_id: i64 },
    #[error(transparent)]
    Refused(#[from] Skipped),
    #[error(transparent)]
    Store(#[from] MemoryError),
}

impl From<rusqlite::Error> for MoveError {
    fn from(e: rusqlite::Error) -> Self {
        MoveError::Store(MemoryError::Sqlite(e))
    }
}

/// The `scope` a row takes in `target`, as the CHECK of `memory` wants it:
/// a row an agent saved keeps its own; one no agent saved is `project` in a
/// project and `global` in none.
pub(crate) fn scope_after(agent_id: Option<&str>, scope: &str, target: Option<&str>) -> String {
    match (agent_id, target) {
        (Some(_), _) => scope.to_string(),
        (None, Some(_)) => "project".to_string(),
        (None, None) => "global".to_string(),
    }
}

/// One row of a unit, as [`decide`] reads it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct UnitRow {
    pub id: i64,
    pub project_id: Option<String>,
    pub agent_id: Option<String>,
    pub scope: String,
    /// Not deleted, and `active` or `disputed`.
    pub live: bool,
    pub memory_type: String,
    pub content_hash: Option<String>,
    pub topic_key: Option<String>,
    /// Within the reach of the caller.
    pub admitted: bool,
}

/// A unit, the selected rows in it, where they go and what the target holds.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Unit {
    pub rows: Vec<UnitRow>,
    pub selected: Vec<i64>,
    pub target: Option<String>,
    /// Its selected rows were sent to more than one target.
    pub split_target: bool,
    /// A live row of the unit and the live row of the target with its text.
    pub twin: Option<(i64, i64)>,
    /// A live row of the unit, its key and the live row of the target that
    /// holds it.
    pub holder: Option<(i64, String, i64)>,
}

/// Decides what happens to each unit, in order: refused, already in place, or
/// moved. A unit accepted earlier holds its texts and keys in its target for
/// the units after it, which then clash with it as they would with a stored
/// row: of two that cannot both be live in one project, the first moves.
pub(crate) fn decide(units: &[Unit]) -> MoveReport {
    let mut report = MoveReport::default();
    let mut texts: HashMap<(Option<String>, String), i64> = HashMap::new();
    let mut keys: HashMap<(Option<String>, String), i64> = HashMap::new();
    for unit in units {
        let first = unit.selected.first().copied().unwrap_or_default();
        let skip = |refusal| Skipped { id: first, refusal };
        if unit.rows.iter().any(|r| !r.admitted) {
            report.skipped.push(skip(Refusal::OutOfReach));
            continue;
        }
        let projects: BTreeSet<&Option<String>> = unit.rows.iter().map(|r| &r.project_id).collect();
        if projects.len() > 1 || unit.split_target {
            report.skipped.push(skip(Refusal::SpansProjects));
            continue;
        }
        if projects.first().is_some_and(|p| **p == unit.target) {
            report.unchanged.extend(&unit.selected);
            continue;
        }
        if let Some((row, twin)) = unit.twin {
            report.skipped.push(skip(Refusal::LiveTwin { row, twin }));
            continue;
        }
        if let Some((row, topic_key, holder)) = unit.holder.clone() {
            report.skipped.push(skip(Refusal::TopicTaken { row, topic_key, holder }));
            continue;
        }
        let live = || unit.rows.iter().filter(|r| r.live);
        let in_target = |value: &String| (unit.target.clone(), value.clone());
        let text_clash = live().find_map(|r| {
            let twin = texts.get(&in_target(r.content_hash.as_ref()?))?;
            Some(Refusal::LiveTwin { row: r.id, twin: *twin })
        });
        let key_clash = || {
            live().find_map(|r| {
                let key = r.topic_key.as_ref()?;
                let holder = keys.get(&in_target(key))?;
                Some(Refusal::TopicTaken { row: r.id, topic_key: key.clone(), holder: *holder })
            })
        };
        if let Some(refusal) = text_clash.or_else(key_clash) {
            report.skipped.push(skip(refusal));
            continue;
        }
        for r in live() {
            if let Some(hash) = &r.content_hash {
                texts.entry(in_target(hash)).or_insert(r.id);
            }
            if let Some(key) = &r.topic_key {
                keys.entry(in_target(key)).or_insert(r.id);
            }
        }
        for r in &unit.rows {
            let protected = (r.live && MemoryType::is_protected_name(&r.memory_type)).then(|| r.memory_type.clone());
            report.protected += usize::from(protected.is_some());
            report.moved.push(MovedRow {
                id: r.id,
                from_project: r.project_id.clone(),
                to_project: unit.target.clone(),
                from_scope: r.scope.clone(),
                to_scope: scope_after(r.agent_id.as_deref(), &r.scope, unit.target.as_deref()),
                protected,
            });
            if !unit.selected.contains(&r.id) {
                report.pulled_in.push(r.id);
            }
        }
    }
    report
}

/// The rows tied to `?1`: the pieces of every save it is a piece of, the rows
/// it replaced by a revision and the one that replaced it so, and the rows its
/// delete brought back and the one whose delete brought it back.
const NEIGHBOURS_SQL: &str = "\
    SELECT b.memory_id FROM memory_piece a JOIN memory_piece b ON b.save_id = a.save_id WHERE a.memory_id = ?1 \
    UNION SELECT id FROM memory WHERE superseded_by = ?1 AND supersede_cause = 'revision' \
    UNION SELECT superseded_by FROM memory WHERE id = ?1 AND supersede_cause = 'revision' AND superseded_by IS NOT NULL \
    UNION SELECT id FROM memory WHERE revived_from = ?1 \
    UNION SELECT revived_from FROM memory WHERE id = ?1 AND revived_from IS NOT NULL";

/// Row `?1` as [`decide`] reads it, for the caller whose reach is bound to `?2`.
const UNIT_ROW_SQL: &str = concat!(
    "SELECT id, project_id, agent_id, scope, deleted_at IS NULL AND status IN ('active', 'disputed'), \
     memory_type, content_hash, topic_key, ",
    reader_predicate!("agent_id", "?2"),
    " FROM memory WHERE id = ?1"
);

/// The live row of project `?1` (none when NULL) with text `?2`, of an author
/// the caller (`?3`) reaches: the row a save of that text would reinforce.
const TARGET_TWIN_SQL: &str = concat!(
    "SELECT id FROM memory WHERE COALESCE(project_id, '') = COALESCE(?1, '') AND content_hash = ?2 \
     AND deleted_at IS NULL AND status IN ('active', 'disputed') AND ",
    reader_predicate!("agent_id", "?3"),
    " ORDER BY created_at ASC LIMIT 1"
);

/// The live row of project `?1` that holds key `?2`, of an author the caller
/// (`?3`) reaches.
const TARGET_HOLDER_SQL: &str = concat!(
    "SELECT id FROM memory WHERE COALESCE(project_id, '') = COALESCE(?1, '') AND topic_key = ?2 \
     AND deleted_at IS NULL AND status IN ('active', 'disputed') AND ",
    reader_predicate!("agent_id", "?3"),
    " ORDER BY id LIMIT 1"
);

/// Every row reachable from `seed` through [`NEIGHBOURS_SQL`], ascending.
fn closure(c: &Connection, seed: i64) -> rusqlite::Result<Vec<i64>> {
    let mut stmt = c.prepare_cached(NEIGHBOURS_SQL)?;
    let mut seen = BTreeSet::from([seed]);
    let mut queue = VecDeque::from([seed]);
    while let Some(id) = queue.pop_front() {
        for next in stmt.query_map([id], |r| r.get::<_, i64>(0))? {
            let next = next?;
            if seen.insert(next) {
                queue.push_back(next);
            }
        }
    }
    Ok(seen.into_iter().collect())
}

/// Reads, for each seed and the target it goes to, the unit it belongs to and
/// what the target holds against it, for the caller whose reach is `reach`
/// (`reader_bind`). A seed that is not a row, or that a unit read before
/// already holds, opens none.
pub(crate) fn read_units_in(
    c: &Connection,
    seeds: &[(i64, Option<String>)],
    reach: Option<&str>,
) -> rusqlite::Result<Vec<Unit>> {
    let mut seeds = seeds.to_vec();
    seeds.sort_by_key(|(id, _)| *id);
    seeds.dedup_by_key(|(id, _)| *id);
    let mut units: Vec<Unit> = Vec::new();
    let mut unit_of: HashMap<i64, usize> = HashMap::new();
    let mut row_stmt = c.prepare_cached(UNIT_ROW_SQL)?;
    for (seed, target) in seeds {
        if let Some(&i) = unit_of.get(&seed) {
            units[i].selected.push(seed);
            units[i].split_target |= units[i].target != target;
            continue;
        }
        let mut rows = Vec::new();
        for id in closure(c, seed)? {
            let row = row_stmt
                .query_row(rusqlite::params![id, reach], |r| {
                    Ok(UnitRow {
                        id: r.get(0)?,
                        project_id: r.get(1)?,
                        agent_id: r.get(2)?,
                        scope: r.get(3)?,
                        live: r.get(4)?,
                        memory_type: r.get(5)?,
                        content_hash: r.get(6)?,
                        topic_key: r.get(7)?,
                        admitted: r.get(8)?,
                    })
                })
                .optional()?;
            rows.extend(row);
        }
        if !rows.iter().any(|r| r.id == seed) {
            continue;
        }
        let (twin, holder) = clashes_in(c, &rows, target.as_deref(), reach)?;
        for r in &rows {
            unit_of.insert(r.id, units.len());
        }
        units.push(Unit { rows, selected: vec![seed], target, split_target: false, twin, holder });
    }
    Ok(units)
}

/// The first live row of the target holding the text, and the first holding
/// the key, of a live row of `rows`.
fn clashes_in(
    c: &Connection,
    rows: &[UnitRow],
    target: Option<&str>,
    reach: Option<&str>,
) -> rusqlite::Result<(Option<(i64, i64)>, Option<(i64, String, i64)>)> {
    let ids: HashSet<i64> = rows.iter().map(|r| r.id).collect();
    let mut twin = None;
    let mut holder = None;
    let mut twins = c.prepare_cached(TARGET_TWIN_SQL)?;
    let mut holders = c.prepare_cached(TARGET_HOLDER_SQL)?;
    for r in rows.iter().filter(|r| r.live) {
        if let (None, Some(hash)) = (twin, &r.content_hash) {
            let found: Option<i64> = twins.query_row(rusqlite::params![target, hash, reach], |x| x.get(0)).optional()?;
            twin = found.filter(|t| !ids.contains(t)).map(|t| (r.id, t));
        }
        if let (None, Some(key)) = (&holder, &r.topic_key) {
            let found: Option<i64> = holders.query_row(rusqlite::params![target, key, reach], |x| x.get(0)).optional()?;
            holder = found.filter(|t| !ids.contains(t)).map(|t| (r.id, key.clone(), t));
        }
    }
    Ok((twin, holder))
}

/// The open pairs between a row of `report.moved` and a row that stays in
/// another project than the moved one's target, both within `reach`.
fn split_pairs_in(c: &Connection, moved: &[MovedRow], reach: Option<&str>) -> rusqlite::Result<Vec<[i64; 2]>> {
    let targets: HashMap<i64, &Option<String>> = moved.iter().map(|m| (m.id, &m.to_project)).collect();
    let mut pairs = c.prepare_cached(
        "SELECT new_id, old_id FROM memory_conflict \
         WHERE (new_id = ?1 OR old_id = ?1) AND state IN ('pending', 'escalated')",
    )?;
    let mut other = c.prepare_cached(concat!(
        "SELECT project_id FROM memory WHERE id = ?1 AND ",
        reader_predicate!("agent_id", "?2")
    ))?;
    let mut split = BTreeSet::new();
    for m in moved {
        for pair in pairs.query_map([m.id], |r| Ok([r.get::<_, i64>(0)?, r.get::<_, i64>(1)?]))? {
            let pair = pair?;
            let side = if pair[0] == m.id { pair[1] } else { pair[0] };
            if targets.contains_key(&side) {
                continue;
            }
            let project: Option<Option<String>> = other.query_row(rusqlite::params![side, reach], |r| r.get(0)).optional()?;
            if project.is_some_and(|p| p != m.to_project) {
                split.insert(pair);
            }
        }
    }
    Ok(split.into_iter().collect())
}

/// Logs and writes `report.moved` as one move, numbered after the last one,
/// and returns its number; `None` when there is nothing to move.
fn apply_in(c: &Connection, report: &MoveReport, now: TimestampMs) -> rusqlite::Result<Option<i64>> {
    if report.moved.is_empty() {
        return Ok(None);
    }
    let move_id: i64 = c.query_row("SELECT COALESCE(MAX(move_id), 0) + 1 FROM memory_move", [], |r| r.get(0))?;
    let mut log = c.prepare_cached(
        "INSERT INTO memory_move (move_id, row_id, from_project, from_scope, to_project, to_scope, moved_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )?;
    let mut write = c.prepare_cached("UPDATE memory SET project_id = ?2, scope = ?3 WHERE id = ?1")?;
    for m in &report.moved {
        log.execute(rusqlite::params![move_id, m.id, m.from_project, m.from_scope, m.to_project, m.to_scope, now])?;
        write.execute(rusqlite::params![m.id, m.to_project, m.to_scope])?;
    }
    Ok(Some(move_id))
}

/// The live project a move to `target` goes to, read in the transaction that
/// moves, so a target merged meanwhile leaves no row under its tombstone.
/// Refuses a target `project` does not know, whether or not a folder leads
/// to it; no project is always a target.
fn target_in(c: &Connection, target: Option<&str>) -> Result<Option<String>, MoveError> {
    let Some(target) = target else {
        return Ok(None);
    };
    let known: bool = c.query_row("SELECT EXISTS (SELECT 1 FROM project WHERE id = ?1)", [target], |r| r.get(0))?;
    if known {
        let live = canonical_in(c, target)?;
        // Its rows would be out of reach of the undo of the delete.
        if let Some(deleted) = deleted_in(c, &live)? {
            return Err(MoveError::Deleted { id: live, delete_id: deleted.delete_id });
        }
        Ok(Some(live))
    } else {
        Err(MoveError::UnknownProject(target.to_string()))
    }
}

/// What a move of the live row `id` to `target` does, decided inside the
/// caller's transaction, for the caller whose reach is `reach`.
pub(crate) fn plan_one_in(
    c: &Connection,
    id: i64,
    target: Option<&str>,
    reach: Option<&str>,
) -> Result<MoveReport, MoveError> {
    let target = target_in(c, target)?;
    let units = read_units_in(c, &[(id, target)], reach)?;
    let mut report = decide(&units);
    report.split_pairs = split_pairs_in(c, &report.moved, reach)?;
    Ok(report)
}

/// Writes what [`plan_one_in`] decided, inside the same transaction.
pub(crate) fn apply_one_in(c: &Connection, report: &mut MoveReport, now: TimestampMs) -> rusqlite::Result<()> {
    report.move_id = apply_in(c, report, now)?;
    Ok(())
}

/// Which project the selected rows are in now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectFilter {
    Project(String),
    None,
}

/// The rows a batch move starts from. Every set field narrows the selection;
/// at least one besides `include_deleted` is needed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MoveFilter {
    pub ids: Vec<i64>,
    pub from: Option<ProjectFilter>,
    pub agent_id: Option<String>,
    pub memory_type: Option<String>,
    pub source: Option<String>,
    pub session_id: Option<String>,
    /// Free text over `memory.content`, as the viewer searches it.
    pub query: Option<String>,
    /// The project the row had in the tool it came from, compared whole.
    pub origin_project: Option<String>,
    /// Selects deleted rows too; a unit's deleted rows move with it anyway.
    pub include_deleted: bool,
}

impl MoveFilter {
    fn is_empty(&self) -> bool {
        self.ids.is_empty()
            && self.from.is_none()
            && self.agent_id.is_none()
            && self.memory_type.is_none()
            && self.source.is_none()
            && self.session_id.is_none()
            && self.query.is_none()
            && self.origin_project.is_none()
    }

    /// The SQL that selects the rows and its parameters, every value bound.
    fn to_sql(&self) -> Result<(String, Vec<rusqlite::types::Value>), MoveError> {
        use rusqlite::types::Value;
        if self.is_empty() {
            return Err(MoveError::NoSelection);
        }
        let mut sql = String::from("SELECT id FROM memory WHERE 1 = 1");
        let mut params = Vec::new();
        if !self.include_deleted {
            sql.push_str(" AND deleted_at IS NULL");
        }
        if !self.ids.is_empty() {
            sql.push_str(" AND id IN (SELECT value FROM json_each(?))");
            params.push(Value::Text(serde_json::to_string(&self.ids).unwrap_or_default()));
        }
        match &self.from {
            Some(ProjectFilter::Project(p)) => {
                sql.push_str(" AND project_id = ?");
                params.push(Value::Text(p.clone()));
            }
            Some(ProjectFilter::None) => sql.push_str(" AND project_id IS NULL"),
            None => {}
        }
        for (column, value) in [
            ("agent_id", &self.agent_id),
            ("memory_type", &self.memory_type),
            ("source", &self.source),
            ("source_session_id", &self.session_id),
            ("origin_project", &self.origin_project),
        ] {
            if let Some(v) = value {
                sql.push_str(&format!(" AND {column} = ?"));
                params.push(Value::Text(v.clone()));
            }
        }
        if let Some(text) = &self.query {
            if self.include_deleted {
                return Err(MoveError::TextOnDeleted);
            }
            let expr = viewer_match_expression(text).ok_or(MoveError::EmptyText)?;
            sql.push_str(" AND id IN (SELECT rowid FROM memory_fts WHERE memory_fts MATCH ?)");
            params.push(Value::Text(expr));
        }
        sql.push_str(" ORDER BY id");
        Ok((sql, params))
    }
}

/// Reads and decides a batch inside `c`, and writes it when `write`.
fn batch_in(
    c: &Connection,
    seeds: &[(i64, Option<String>)],
    write: bool,
    now: TimestampMs,
) -> rusqlite::Result<MoveReport> {
    let units = read_units_in(c, seeds, None)?;
    let mut report = decide(&units);
    report.split_pairs = split_pairs_in(c, &report.moved, None)?;
    if write {
        report.move_id = apply_in(c, &report, now)?;
    }
    Ok(report)
}

/// Runs `pass` twice when there is something to move: once in a transaction
/// that is dropped, to find out, and, after the `pre-move` snapshot, once in
/// one that commits, so what was saved in between is seen. A dry run, or a
/// move with nothing to move, stops after the first.
async fn run_batch<F>(store: &MemoryStore, backups: &Path, dry_run: bool, pass: F) -> Result<MoveReport, MoveError>
where
    F: Fn(&Connection, bool) -> Result<MoveReport, MoveError> + Clone + Send + 'static,
{
    let find_out = pass.clone();
    let mut report = store
        .conn
        .call(move |c| -> Result<MoveReport, MoveError> {
            let tx = c.transaction()?;
            find_out(&tx, false)
        })
        .await
        .map_err(connection_error)?;
    report.dry_run = dry_run;
    if dry_run || report.moved.is_empty() {
        return Ok(report);
    }
    let snapshot = store.snapshot(backups.to_path_buf(), Reason::PreMove).await?;
    let mut report = store
        .conn
        .call(move |c| -> Result<MoveReport, MoveError> {
            let tx = c.transaction()?;
            let report = pass(&tx, true)?;
            tx.commit()?;
            Ok(report)
        })
        .await
        .map_err(connection_error)?;
    report.snapshot = Some(snapshot.display().to_string());
    Ok(report)
}

fn connection_error(e: tokio_rusqlite::Error<MoveError>) -> MoveError {
    match e {
        tokio_rusqlite::Error::Error(e) => e,
        other => MoveError::Store(MemoryError::Backend(other.to_string())),
    }
}

/// Moves the rows `filter` selects, with their units, to `target` (`None`:
/// out of every project). The batch is the user's: it clashes with every row
/// of the target and needs no word for preferences and decisions. Before it
/// writes it snapshots the store as `pre-move` into `backups`.
pub async fn move_memories_core(
    store: &MemoryStore,
    backups: &Path,
    filter: &MoveFilter,
    target: Option<String>,
    dry_run: bool,
) -> Result<MoveReport, MoveError> {
    let (sql, params) = filter.to_sql()?;
    let from = filter.from.clone();
    run_batch(store, backups, dry_run, move |c, write| {
        // The rows of a project merged since are all in the one it went into,
        // with that project's own: following it would move those too.
        if let Some(ProjectFilter::Project(id)) = &from {
            let into = canonical_in(c, id)?;
            if into != *id {
                return Err(MoveError::Merged { id: id.clone(), into });
            }
        }
        let live = target_in(c, target.as_deref())?;
        let ids: Vec<i64> = {
            let mut stmt = c.prepare(&sql)?;
            let rows = stmt.query_map(rusqlite::params_from_iter(params.iter()), |r| r.get(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let seeds: Vec<(i64, Option<String>)> = ids.into_iter().map(|id| (id, live.clone())).collect();
        Ok(batch_in(c, &seeds, write, TimestampMs::now())?)
    })
    .await
}

/// Moves back the rows of move `move_id` that are still where it left them,
/// each to the project it came from, under the same rules as any move and as
/// a move of its own. The project a row came from need not be registered: it
/// may be an id from before roots were, or none.
pub async fn undo_move_core(
    store: &MemoryStore,
    backups: &Path,
    move_id: i64,
    dry_run: bool,
) -> Result<MoveReport, MoveError> {
    run_batch(store, backups, dry_run, move |c, write| {
        // Its rows alone would leave the rest of the merge in its target, and
        // the rows under a tombstone.
        let merge: Option<i64> = c
            .query_row("SELECT merge_id FROM project_merge WHERE ?1 IN (move_id, undo_move_id)", [move_id], |r| r.get(0))
            .optional()?;
        if let Some(merge_id) = merge {
            return Err(MoveError::PartOfMerge { move_id, merge_id });
        }
        undo_move_in(c, move_id, write)
    })
    .await
}

/// [`undo_move_core`] inside the caller's transaction, which a merge's undo
/// shares with the rest it takes back. The rows move only when `write`.
pub(crate) fn undo_move_in(c: &Connection, move_id: i64, write: bool) -> Result<MoveReport, MoveError> {
    let known: bool = c.query_row("SELECT EXISTS (SELECT 1 FROM memory_move WHERE move_id = ?1)", [move_id], |r| r.get(0))?;
    if !known {
        return Err(MoveError::UnknownMove(move_id));
    }
    c.execute(
        "UPDATE memory SET status = s.from_status, superseded_by = s.from_superseded_by, \
         supersede_cause = s.from_cause FROM memory_move_supersede AS s \
         WHERE s.move_id = ?1 AND s.row_id = memory.id \
           AND memory.status = 'superseded' AND memory.superseded_by = s.superseded_by",
        [move_id],
    )?;
    let back: Vec<(i64, Option<String>)> = {
        let mut stmt = c.prepare(
            "SELECT m.row_id, m.from_project FROM memory_move m JOIN memory r ON r.id = m.row_id \
             WHERE m.move_id = ?1 AND r.project_id IS m.to_project ORDER BY m.row_id",
        )?;
        let rows = stmt.query_map([move_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let mut by_origin: BTreeMap<Option<String>, Vec<(i64, Option<String>)>> = BTreeMap::new();
    for (id, from) in back {
        // A project the rows came from that was merged since is the one it went into.
        let from = from.map(|p| canonical_in(c, &p)).transpose()?;
        by_origin.entry(from.clone()).or_default().push((id, from));
    }
    let mut report = MoveReport::default();
    for seeds in by_origin.values() {
        report.absorb(batch_in(c, seeds, false, TimestampMs::now())?);
    }
    if write {
        report.move_id = apply_in(c, &report, TimestampMs::now())?;
    }
    Ok(report)
}

/// A row joining two projects leaves superseded by `by`, which holds its key
/// or its text in the joined project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Supersede {
    pub row: i64,
    pub by: i64,
}

/// What joining does with a unit that also has rows in a project it does not
/// join.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Span {
    /// Leaves the unit where it is: the legacy ids a registration joins stay
    /// live projects, where their rows are still found.
    Keep,
    /// Takes the unit's rows in the projects joined and leaves the others: a
    /// project merged is a tombstone, and a row left under it would be in no
    /// project anyone lists or searches.
    Carry,
}

/// [`decide`] for joining the projects `origins` into `target`: a clash is
/// no refusal, since [`reconcile`] settles it after the move, and a unit may
/// already have rows in the target, which stay. A unit with a row anywhere
/// else is left where it is or split, as `span` says.
pub(crate) fn decide_merge(units: &[Unit], origins: &[String], target: &str, span: Span) -> MoveReport {
    let mut report = MoveReport::default();
    let joined = |r: &UnitRow| r.project_id.as_ref().is_some_and(|p| origins.contains(p));
    for unit in units {
        let first = unit.selected.first().copied().unwrap_or_default();
        let elsewhere = unit.rows.iter().any(|r| r.project_id.as_deref() != Some(target) && !joined(r));
        if elsewhere {
            if span == Span::Keep {
                report.skipped.push(Skipped { id: first, refusal: Refusal::SpansProjects });
                continue;
            }
            report.split.push(first);
        }
        let mut moved_any = false;
        for r in unit.rows.iter().filter(|r| joined(r)) {
            moved_any = true;
            let protected = (r.live && MemoryType::is_protected_name(&r.memory_type)).then(|| r.memory_type.clone());
            report.protected += usize::from(protected.is_some());
            report.moved.push(MovedRow {
                id: r.id,
                from_project: r.project_id.clone(),
                to_project: Some(target.to_string()),
                from_scope: r.scope.clone(),
                to_scope: scope_after(r.agent_id.as_deref(), &r.scope, Some(target)),
                protected,
            });
            if !unit.selected.contains(&r.id) {
                report.pulled_in.push(r.id);
            }
        }
        if !moved_any {
            report.unchanged.extend(&unit.selected);
        }
    }
    report
}

/// A live row of a joined project, as [`reconcile`] reads it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LiveRow {
    pub id: i64,
    pub created_at: i64,
    pub topic_key: Option<String>,
    pub content_hash: Option<String>,
    /// The save it is a piece of (the lowest when several), or none.
    pub save: Option<i64>,
    /// The first row of that save, or the row itself.
    pub head: i64,
    /// How many live rows that save has; 1 without a save.
    pub save_live: usize,
    /// A piece of a save whose first row is live without this row's key:
    /// superseding it would cut a hole in that save.
    pub foreign_piece: bool,
    /// Joining moved it here.
    pub moved: bool,
}

impl LiveRow {
    fn save_key(&self) -> i64 {
        self.save.unwrap_or(-self.id)
    }
}

/// What joining leaves live in a project, from its live rows in the groups a
/// moved row touches: of the saves under one key, the one with the newest
/// row, as a new save under the key would; then, of the rows of one text, the
/// oldest, as the dedup keeps it. A row of a save with several live rows is
/// replaced for its text only when all of them are, so no save is cut.
pub(crate) fn reconcile(rows: &[LiveRow]) -> Vec<Supersede> {
    fn age(r: &&&LiveRow) -> (i64, i64) {
        (r.created_at, r.id)
    }
    let mut out: Vec<Supersede> = Vec::new();
    let mut keys: BTreeMap<&str, Vec<&LiveRow>> = BTreeMap::new();
    for r in rows {
        if let Some(k) = &r.topic_key {
            keys.entry(k).or_default().push(r);
        }
    }
    for group in keys.values() {
        let saves: BTreeSet<i64> = group.iter().map(|r| r.save_key()).collect();
        if saves.len() < 2 || !group.iter().any(|r| r.moved) {
            continue;
        }
        let Some(winner) = group.iter().max_by_key(age) else {
            continue;
        };
        for r in group.iter().filter(|r| r.save_key() != winner.save_key() && !r.foreign_piece) {
            out.push(Supersede { row: r.id, by: winner.head });
        }
    }
    let gone: HashSet<i64> = out.iter().map(|s| s.row).collect();
    let mut texts: BTreeMap<&str, Vec<&LiveRow>> = BTreeMap::new();
    for r in rows.iter().filter(|r| !gone.contains(&r.id)) {
        if let Some(h) = &r.content_hash {
            texts.entry(h).or_default().push(r);
        }
    }
    let mut candidates: Vec<(&LiveRow, i64)> = Vec::new();
    for group in texts.values() {
        if group.len() < 2 || !group.iter().any(|r| r.moved) {
            continue;
        }
        let Some(keeper) = group.iter().min_by_key(age) else {
            continue;
        };
        candidates.extend(group.iter().filter(|r| r.save_key() != keeper.save_key()).map(|r| (*r, keeper.id)));
    }
    let mut per_save: HashMap<i64, usize> = HashMap::new();
    for (r, _) in &candidates {
        *per_save.entry(r.save_key()).or_default() += 1;
    }
    out.extend(
        candidates
            .into_iter()
            .filter(|(r, _)| per_save[&r.save_key()] >= r.save_live)
            .map(|(r, by)| Supersede { row: r.id, by }),
    );
    out.sort_by_key(|s| s.row);
    out
}

/// The live rows of `target` that share a key or a text with a live row of
/// `moved`, as [`reconcile`] reads them.
fn live_rows_in(c: &Connection, target: &str, moved: &HashSet<i64>) -> rusqlite::Result<Vec<LiveRow>> {
    const LIVE: &str = "deleted_at IS NULL AND status IN ('active', 'disputed')";
    let ids = serde_json::to_string(&moved.iter().collect::<Vec<_>>()).unwrap_or_default();
    let mut stmt = c.prepare(&format!(
        "WITH m AS (SELECT topic_key, content_hash FROM memory \
                    WHERE id IN (SELECT value FROM json_each(?2)) AND {LIVE}) \
         SELECT r.id, r.created_at, r.topic_key, r.content_hash, \
                (SELECT MIN(save_id) FROM memory_piece WHERE memory_id = r.id) \
         FROM memory r WHERE r.project_id = ?1 AND {LIVE} \
           AND (r.topic_key IN (SELECT topic_key FROM m) OR r.content_hash IN (SELECT content_hash FROM m)) \
         ORDER BY r.id"
    ))?;
    let found = stmt
        .query_map(rusqlite::params![target, ids], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, Option<String>>(2)?, r.get::<_, Option<String>>(3)?, r.get::<_, Option<i64>>(4)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut head = c.prepare_cached("SELECT memory_id FROM memory_piece WHERE save_id = ?1 AND piece = 1")?;
    let mut live_pieces = c.prepare_cached(
        "SELECT COUNT(*) FROM memory_piece p JOIN memory m ON m.id = p.memory_id \
         WHERE p.save_id = ?1 AND m.deleted_at IS NULL AND m.status IN ('active', 'disputed')",
    )?;
    let mut foreign = c.prepare_cached(&format!(
        "SELECT EXISTS (SELECT 1 FROM memory WHERE id = ?1 AND id != ?2 AND topic_key IS NOT ?3 AND {LIVE})"
    ))?;
    let mut rows = Vec::new();
    for (id, created_at, topic_key, content_hash, save) in found {
        let (head_id, save_live) = match save {
            Some(save) => (
                head.query_row([save], |r| r.get(0)).optional()?.unwrap_or(id),
                live_pieces.query_row([save], |r| r.get::<_, i64>(0))? as usize,
            ),
            None => (id, 1),
        };
        let foreign_piece: bool = topic_key.is_some()
            && foreign.query_row(rusqlite::params![head_id, id, topic_key], |r| r.get(0))?;
        rows.push(LiveRow {
            id,
            created_at,
            topic_key,
            content_hash,
            save,
            head: head_id,
            save_live: save_live.max(1),
            foreign_piece,
            moved: moved.contains(&id),
        });
    }
    Ok(rows)
}

/// Writes what [`reconcile`] decided as part of move `move_id`, each row with
/// the state it had, so [`undo_move_core`] can give it back.
fn supersede_in(c: &Connection, move_id: i64, supersedes: &[Supersede]) -> rusqlite::Result<()> {
    let mut log = c.prepare_cached(
        "INSERT INTO memory_move_supersede \
         (move_id, row_id, superseded_by, from_status, from_superseded_by, from_cause) \
         SELECT ?1, id, ?3, status, superseded_by, supersede_cause FROM memory WHERE id = ?2",
    )?;
    let mut write = c.prepare_cached(
        "UPDATE memory SET status = 'superseded', superseded_by = ?2, supersede_cause = 'revision' WHERE id = ?1",
    )?;
    for s in supersedes {
        log.execute(rusqlite::params![move_id, s.row, s.by])?;
        write.execute(rusqlite::params![s.row, s.by])?;
    }
    Ok(())
}

/// Joins the projects `origins` into `target` inside the caller's
/// transaction: every row of theirs, deleted ones too, moves with its unit as
/// one logged move (a unit that is also elsewhere as `span` says), and then
/// the target keeps one live row per key and per text that a moved row
/// touches (see [`reconcile`]). Nothing but `project_id`, `scope`, and the
/// state of the rows replaced, changes.
pub(crate) fn merge_in(
    c: &Connection,
    origins: &[String],
    target: &str,
    span: Span,
    now: TimestampMs,
) -> rusqlite::Result<MoveReport> {
    let seeds: Vec<(i64, Option<String>)> = {
        let ids = serde_json::to_string(origins).unwrap_or_default();
        let mut stmt = c.prepare(
            "SELECT id FROM memory WHERE project_id IN (SELECT value FROM json_each(?1)) ORDER BY id",
        )?;
        let rows = stmt.query_map([ids], |r| r.get::<_, i64>(0))?;
        rows.map(|id| id.map(|id| (id, Some(target.to_string())))).collect::<rusqlite::Result<_>>()?
    };
    let units = read_units_in(c, &seeds, None)?;
    let mut report = decide_merge(&units, origins, target, span);
    for skipped in &report.skipped {
        log::warn!("memory #{} stays where it is: {}", skipped.id, skipped.refusal);
    }
    report.move_id = apply_in(c, &report, now)?;
    if let Some(move_id) = report.move_id {
        let moved: HashSet<i64> = report.moved.iter().map(|m| m.id).collect();
        report.superseded = reconcile(&live_rows_in(c, target, &moved)?);
        supersede_in(c, move_id, &report.superseded)?;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::memory::admin::{delete_memory_core, restore_memory_core, MemoryKind};
    use crate::memory::embedder::{EmbedRole, Embedder, FakeEmbedder};
    use crate::memory::events::{MemoryEvent, MemorySource};
    use crate::memory::search::{search_all_memories, SearchScope};
    use crate::memory::sharing::{Reach, Reader};
    use crate::memory::worker::save_for_test;
    use crate::models::chat::Role;

    const A: &str = "repo-aaaa";
    const B: &str = "repo-bbbb";

    // ── decide, without a store ─────────────────────────────────────────

    fn row(id: i64, project: Option<&str>) -> UnitRow {
        UnitRow {
            id,
            project_id: project.map(str::to_string),
            agent_id: Some("agent-a".into()),
            scope: "agent".into(),
            live: true,
            memory_type: "semantic_fact".into(),
            content_hash: Some(format!("h{id}")),
            topic_key: None,
            admitted: true,
        }
    }

    fn unit(rows: Vec<UnitRow>, selected: &[i64], target: Option<&str>) -> Unit {
        Unit {
            rows,
            selected: selected.to_vec(),
            target: target.map(str::to_string),
            split_target: false,
            twin: None,
            holder: None,
        }
    }

    fn moved_ids(report: &MoveReport) -> Vec<i64> {
        report.moved.iter().map(|m| m.id).collect()
    }

    #[test]
    fn scope_after_a_move_follows_the_check() {
        assert_eq!(scope_after(Some("a"), "agent", None), "agent");
        assert_eq!(scope_after(Some("a"), "agent", Some(B)), "agent");
        assert_eq!(scope_after(None, "project", None), "global");
        assert_eq!(scope_after(None, "global", Some(B)), "project");
    }

    #[test]
    fn decide_moves_a_unit_and_names_what_it_pulled_in() {
        let report = decide(&[unit(vec![row(1, None), row(2, None)], &[2], Some(B))]);

        assert_eq!(moved_ids(&report), vec![1, 2]);
        assert_eq!(report.pulled_in, vec![1]);
        assert!(report.moved.iter().all(|m| m.from_project.is_none() && m.to_project.as_deref() == Some(B)));
    }

    #[test]
    fn decide_leaves_a_unit_already_in_its_target() {
        let report = decide(&[unit(vec![row(1, Some(B))], &[1], Some(B))]);

        assert_eq!(report.unchanged, vec![1]);
        assert!(report.moved.is_empty());
    }

    #[test]
    fn decide_refuses_a_unit_across_projects_or_out_of_reach() {
        let hidden = UnitRow { admitted: false, ..row(2, None) };
        let report = decide(&[
            unit(vec![row(1, Some(A)), row(3, None)], &[1], Some(B)),
            unit(vec![row(4, None), hidden], &[4], Some(B)),
            Unit { split_target: true, ..unit(vec![row(5, None)], &[5], Some(B)) },
        ]);

        assert!(report.moved.is_empty());
        assert_eq!(
            report.skipped,
            vec![
                Skipped { id: 1, refusal: Refusal::SpansProjects },
                Skipped { id: 4, refusal: Refusal::OutOfReach },
                Skipped { id: 5, refusal: Refusal::SpansProjects },
            ]
        );
        assert!(!report.skipped[1].to_string().contains("#2"), "a hidden row is not named");
    }

    #[test]
    fn decide_moves_the_first_of_two_units_that_clash_in_the_target() {
        let keyed = |id| UnitRow { topic_key: Some("storage/engine".into()), content_hash: Some(format!("k{id}")), ..row(id, None) };
        let same_text = |id| UnitRow { content_hash: Some("same".into()), ..row(id, None) };
        let report = decide(&[
            unit(vec![same_text(1)], &[1], Some(B)),
            unit(vec![same_text(2)], &[2], Some(B)),
            unit(vec![keyed(3)], &[3], Some(B)),
            unit(vec![keyed(4)], &[4], Some(B)),
            unit(vec![same_text(5)], &[5], Some(A)),
        ]);

        assert_eq!(moved_ids(&report), vec![1, 3, 5], "a clash is within one target");
        assert_eq!(
            report.skipped,
            vec![
                Skipped { id: 2, refusal: Refusal::LiveTwin { row: 2, twin: 1 } },
                Skipped { id: 4, refusal: Refusal::TopicTaken { row: 4, topic_key: "storage/engine".into(), holder: 3 } },
            ]
        );
    }

    #[test]
    fn decide_lets_a_deleted_row_through_and_counts_live_protected_rows() {
        let deleted = UnitRow { live: false, content_hash: Some("taken".into()), ..row(1, None) };
        let decision = UnitRow { memory_type: "architectural_decision".into(), ..row(2, None) };
        let old_decision = UnitRow { live: false, ..decision.clone() };
        let twin_of_deleted = UnitRow { content_hash: Some("taken".into()), ..row(9, None) };
        let report = decide(&[
            unit(vec![twin_of_deleted], &[9], Some(B)),
            unit(vec![deleted, decision, UnitRow { id: 3, ..old_decision }], &[1], Some(B)),
        ]);

        assert_eq!(moved_ids(&report), vec![9, 1, 2, 3]);
        assert_eq!(report.protected, 1);
        assert_eq!(report.moved[2].protected.as_deref(), Some("architectural_decision"));
    }

    #[test]
    fn decide_gives_a_row_no_agent_saved_the_scope_of_its_target() {
        let authorless = UnitRow { agent_id: None, scope: "project".into(), ..row(1, Some(A)) };
        let report = decide(&[unit(vec![authorless], &[1], None)]);

        assert_eq!((report.moved[0].from_scope.as_str(), report.moved[0].to_scope.as_str()), ("project", "global"));
    }

    // ── against a store ─────────────────────────────────────────────────

    async fn store() -> Arc<MemoryStore> {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        for id in [A, B] {
            register(&store, id).await;
        }
        store
    }

    async fn register(store: &MemoryStore, id: &str) {
        let id = id.to_string();
        store
            .conn
            .call(move |c| {
                c.execute(
                    "INSERT INTO project_path (id, root_path, created_at, index_docs) VALUES (?1, '/work/' || ?1, 0, 0)",
                    [&id],
                )?;
                c.execute("INSERT INTO project (id, name, born, created_at) VALUES (?1, ?1, 'root', 0)", [&id])
            })
            .await
            .unwrap();
    }

    fn fact(content: &str, project: Option<&str>) -> MemoryEvent {
        MemoryEvent {
            agent_id: "agent-a".into(),
            project_id: project.map(str::to_string),
            role: Role::User,
            content: content.into(),
            is_error: None,
            source: MemorySource::Explicit,
            timestamp_ms: 1,
            memory_type: Some("semantic_fact".into()),
            importance: None,
            derived_from: Vec::new(),
            topic_key: None,
            user_stated: false,
            session_id: None,
        }
    }

    fn keyed(content: &str, project: Option<&str>, key: &str) -> MemoryEvent {
        MemoryEvent { topic_key: Some(key.into()), ..fact(content, project) }
    }

    fn long_text(subject: &str) -> String {
        let words: Vec<String> = (0..1200).map(|i| format!("{subject}{i}")).collect();
        format!("Notes on {subject}. {}", words.join(" "))
    }

    async fn save(store: &Arc<MemoryStore>, evt: MemoryEvent) -> Vec<i64> {
        let report = save_for_test(store, &evt).await;
        assert_eq!(report.failure, None);
        report.memories.iter().map(|m| m.id).collect()
    }

    async fn save_one(store: &Arc<MemoryStore>, evt: MemoryEvent) -> i64 {
        let ids = save(store, evt).await;
        assert_eq!(ids.len(), 1, "{ids:?}");
        ids[0]
    }

    /// `(project_id, scope)` of row `id`.
    async fn place(store: &MemoryStore, id: i64) -> (Option<String>, String) {
        store
            .conn
            .call(move |c| c.query_row("SELECT project_id, scope FROM memory WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?))))
            .await
            .unwrap()
    }

    async fn projects_of(store: &MemoryStore, ids: &[i64]) -> Vec<Option<String>> {
        let mut out = Vec::new();
        for &id in ids {
            out.push(place(store, id).await.0);
        }
        out
    }

    /// The ids a search for `text` finds from `project`, by vector, by text
    /// or by identifier.
    async fn found_from(store: &MemoryStore, project: Option<&str>, text: &str) -> Vec<i64> {
        let vector = FakeEmbedder::default().embed_with_role(text, EmbedRole::Query).await.unwrap();
        let scope = SearchScope::all_default().with_project(project).with_reach(Reach::AllBut(Vec::new()));
        search_all_memories(store, &vector, Some(text), "agent-a", scope, 10)
            .await
            .unwrap()
            .into_iter()
            .map(|h| h.id)
            .collect()
    }

    fn ids(ids: &[i64]) -> MoveFilter {
        MoveFilter { ids: ids.to_vec(), ..MoveFilter::default() }
    }

    async fn move_to(store: &MemoryStore, filter: &MoveFilter, target: Option<&str>) -> MoveReport {
        let backups = tempfile::tempdir().unwrap();
        move_memories_core(store, backups.path(), filter, target.map(str::to_string), false).await.unwrap()
    }

    async fn moves_logged(store: &MemoryStore) -> Vec<(i64, i64, Option<String>, String, Option<String>, String)> {
        store
            .conn
            .call(|c| {
                let mut stmt = c.prepare(
                    "SELECT move_id, row_id, from_project, from_scope, to_project, to_scope FROM memory_move \
                     ORDER BY move_id, row_id",
                )?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))?;
                rows.collect()
            })
            .await
            .unwrap()
    }

    /// Every column of `memory` but `project_id` and `scope`, of every row.
    async fn everything_else(store: &MemoryStore) -> Vec<String> {
        store
            .conn
            .call(|c| {
                let mut stmt = c.prepare(
                    "SELECT json_array(id, agent_id, content, memory_type, status, superseded_by, topic_key, \
                     deleted_at, last_modified_at, update_count, tier, importance, access_count, content_hash) \
                     FROM memory ORDER BY id",
                )?;
                let rows = stmt.query_map([], |r| r.get(0))?;
                rows.collect()
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn a_row_without_a_project_moves_into_one() {
        let store = store().await;
        let text = "the zanzibar parser keeps a token cache";
        let id = save_one(&store, fact(text, None)).await;
        assert!(found_from(&store, Some(A), text).await.contains(&id));

        let report = move_to(&store, &ids(&[id]), Some(B)).await;

        assert_eq!(moved_ids(&report), vec![id]);
        assert_eq!(place(&store, id).await, (Some(B.to_string()), "agent".to_string()));
        assert!(found_from(&store, Some(B), text).await.contains(&id));
        assert!(found_from(&store, Some(B), "zanzibar").await.contains(&id), "by text alone");
        assert!(!found_from(&store, Some(A), text).await.contains(&id));
        assert!(found_from(&store, None, text).await.contains(&id));
    }

    #[tokio::test]
    async fn moving_to_no_project_shows_the_row_from_every_project() {
        let store = store().await;
        let text = "the release job signs SHA256SUMS";
        let id = save_one(&store, fact(text, Some(A))).await;
        assert!(!found_from(&store, Some(B), text).await.contains(&id));

        move_to(&store, &ids(&[id]), None).await;

        for project in [Some(A), Some(B), None] {
            assert!(found_from(&store, project, text).await.contains(&id), "from {project:?}");
        }
    }

    #[tokio::test]
    async fn a_row_without_an_agent_keeps_the_scope_check() {
        let store = store().await;
        let id = save_one(&store, fact("a note nobody saved", Some(A))).await;
        store
            .conn
            .call(move |c| c.execute("UPDATE memory SET agent_id = NULL, scope = 'project' WHERE id = ?1", [id]))
            .await
            .unwrap();

        move_to(&store, &ids(&[id]), None).await;
        assert_eq!(place(&store, id).await, (None, "global".to_string()));

        move_to(&store, &ids(&[id]), Some(B)).await;
        assert_eq!(place(&store, id).await, (Some(B.to_string()), "project".to_string()));
    }

    #[tokio::test]
    async fn the_pieces_of_a_save_move_together() {
        let store = store().await;
        let pieces = save(&store, fact(&long_text("ledger"), Some(A))).await;
        assert!(pieces.len() >= 3, "the fixture is long");

        let report = move_to(&store, &ids(&[pieces[1]]), Some(B)).await;

        assert_eq!(moved_ids(&report), pieces);
        let pulled: Vec<i64> = pieces.iter().copied().filter(|&p| p != pieces[1]).collect();
        assert_eq!(report.pulled_in, pulled);
        assert!(projects_of(&store, &pieces).await.iter().all(|p| p.as_deref() == Some(B)));
    }

    #[tokio::test]
    async fn two_saves_that_share_a_row_move_together() {
        let store = store().await;
        let mut rows = Vec::new();
        for text in ["first piece", "shared piece", "last piece"] {
            rows.push(save_one(&store, fact(text, Some(A))).await);
        }
        let (a, b, c) = (rows[0], rows[1], rows[2]);
        store
            .conn
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO memory_piece (save_id, piece, memory_id, context) \
                     VALUES (1, 1, ?1, ''), (1, 2, ?2, 'first'), (2, 1, ?2, ''), (2, 2, ?3, 'shared')",
                    [a, b, c],
                )
            })
            .await
            .unwrap();

        let report = move_to(&store, &ids(&[a]), Some(B)).await;

        assert_eq!(moved_ids(&report), vec![a, b, c]);
        assert_eq!(report.pulled_in, vec![b, c]);
    }

    #[tokio::test]
    async fn a_revision_chain_moves_with_its_head_and_deleting_the_head_still_revives() {
        let store = store().await;
        let old = save_one(&store, keyed("Storage engine is SQLite", Some(A), "storage/engine")).await;
        let head = save_one(&store, keyed("Storage engine is Postgres", Some(A), "storage/engine")).await;

        let report = move_to(&store, &ids(&[old]), Some(B)).await;

        assert_eq!(moved_ids(&report), vec![old, head]);
        let deleted = delete_memory_core(&store, head, MemoryKind::Memory, None).await.unwrap();
        assert_eq!(deleted.revived.iter().map(|r| r.id).collect::<Vec<_>>(), vec![old]);
        assert_eq!(place(&store, old).await.0.as_deref(), Some(B));
    }

    #[tokio::test]
    async fn after_a_move_a_save_of_the_same_text_reinforces_the_moved_row() {
        let store = store().await;
        let text = "the build uses cargo nextest";
        let id = save_one(&store, fact(text, None)).await;
        move_to(&store, &ids(&[id]), Some(B)).await;

        let report = save_for_test(&store, &fact(text, Some(B))).await;

        assert_eq!(report.memories[0].id, id, "{report:?}");
    }

    #[tokio::test]
    async fn after_a_move_a_save_under_its_key_revises_it() {
        let store = store().await;
        let id = save_one(&store, keyed("Logs go to serve.log", None, "logging/target")).await;
        move_to(&store, &ids(&[id]), Some(B)).await;

        let report = save_for_test(&store, &keyed("Logs go to serve.log, rotated past 10 MiB", Some(B), "logging/target")).await;

        assert_eq!(report.replaced.iter().map(|r| r.id).collect::<Vec<_>>(), vec![id]);
    }

    #[tokio::test]
    async fn a_live_row_of_the_same_text_in_the_target_refuses_the_move() {
        let store = store().await;
        let text = "the daemon listens on clawg.sock";
        let id = save_one(&store, fact(text, Some(A))).await;
        let twin = save_one(&store, fact(text, Some(B))).await;
        let before = (everything_else(&store).await, projects_of(&store, &[id, twin]).await);

        let report = move_to(&store, &ids(&[id]), Some(B)).await;

        assert_eq!(report.skipped, vec![Skipped { id, refusal: Refusal::LiveTwin { row: id, twin } }]);
        assert_eq!((everything_else(&store).await, projects_of(&store, &[id, twin]).await), before);
        assert!(moves_logged(&store).await.is_empty());
        assert_eq!(report.snapshot, None, "nothing to move, nothing snapshotted");
    }

    #[tokio::test]
    async fn a_live_holder_of_the_topic_key_in_the_target_refuses_the_move() {
        let store = store().await;
        let id = save_one(&store, keyed("Retries back off to 60 s", Some(A), "ingest/retry")).await;
        let holder = save_one(&store, keyed("Retries stop after 3 takes", Some(B), "ingest/retry")).await;

        let report = move_to(&store, &ids(&[id]), Some(B)).await;

        assert_eq!(
            report.skipped,
            vec![Skipped { id, refusal: Refusal::TopicTaken { row: id, topic_key: "ingest/retry".into(), holder } }]
        );
        assert_eq!(place(&store, id).await.0.as_deref(), Some(A));
        assert!(moves_logged(&store).await.is_empty());
    }

    #[tokio::test]
    async fn a_clash_is_judged_within_the_readers_reach() {
        let store = store().await;
        let text = "the hook answers within 3 s";
        let id = save_one(&store, fact(text, Some(A))).await;
        let theirs = save_one(&store, MemoryEvent { agent_id: "agent-b".into(), ..fact(text, Some(B)) }).await;

        let blind = Reader::new("agent-a", Reach::AllBut(vec!["agent-b".into()]));
        let sees = Reader::new("agent-a", Reach::AllBut(Vec::new()));
        let plan = |reader: Reader| {
            let reach = crate::memory::search::reader_bind(Some(&reader));
            store.conn.call(move |c| -> Result<MoveReport, MoveError> { plan_one_in(c, id, Some(B), reach.as_deref()) })
        };

        assert_eq!(moved_ids(&plan(blind).await.unwrap()), vec![id], "what the policy hides does not clash");
        let report = plan(sees).await.unwrap();
        assert_eq!(report.skipped, vec![Skipped { id, refusal: Refusal::LiveTwin { row: id, twin: theirs } }]);
    }

    #[tokio::test]
    async fn a_hidden_row_in_the_unit_refuses_it_without_naming_it() {
        let store = store().await;
        let pieces = save(&store, fact(&long_text("hidden"), Some(A))).await;
        let last = *pieces.last().unwrap();
        store
            .conn
            .call(move |c| c.execute("UPDATE memory SET agent_id = 'agent-b' WHERE id = ?1", [last]))
            .await
            .unwrap();
        let reader = Reader::new("agent-a", Reach::AllBut(vec!["agent-b".into()]));
        let reach = crate::memory::search::reader_bind(Some(&reader));
        let first = pieces[0];

        let report = store
            .conn
            .call(move |c| -> Result<MoveReport, MoveError> { plan_one_in(c, first, Some(B), reach.as_deref()) })
            .await
            .unwrap();

        assert_eq!(report.skipped, vec![Skipped { id: first, refusal: Refusal::OutOfReach }]);
        assert!(!report.skipped[0].to_string().contains(&format!("#{last}")));
    }

    #[tokio::test]
    async fn a_unit_across_projects_is_refused() {
        let store = store().await;
        let pieces = save(&store, fact(&long_text("split"), Some(A))).await;
        let last = *pieces.last().unwrap();
        store
            .conn
            .call(move |c| c.execute("UPDATE memory SET project_id = NULL WHERE id = ?1", [last]))
            .await
            .unwrap();

        let report = move_to(&store, &ids(&[pieces[0]]), Some(B)).await;

        assert_eq!(report.skipped, vec![Skipped { id: pieces[0], refusal: Refusal::SpansProjects }]);
        assert_eq!(place(&store, pieces[0]).await.0.as_deref(), Some(A));
    }

    #[tokio::test]
    async fn two_selected_saves_that_would_collide_in_the_target_move_only_the_first() {
        let store = store().await;
        let text = "the outbox row is written with synchronous FULL";
        let first = save_one(&store, fact(text, None)).await;
        let second = save_one(&store, fact(text, Some(A))).await;

        let filter = MoveFilter { query: Some("outbox synchronous".into()), ..MoveFilter::default() };
        let report = move_to(&store, &filter, Some(B)).await;

        assert_eq!(moved_ids(&report), vec![first]);
        assert_eq!(report.skipped, vec![Skipped { id: second, refusal: Refusal::LiveTwin { row: second, twin: first } }]);
    }

    #[tokio::test]
    async fn a_deleted_row_moves_with_its_save_and_is_restored_into_the_new_project() {
        let store = store().await;
        let pieces = save(&store, fact(&long_text("restore"), Some(A))).await;
        let gone = pieces[1];
        delete_memory_core(&store, gone, MemoryKind::Memory, None).await.unwrap();

        let report = move_to(&store, &ids(&[pieces[0]]), Some(B)).await;

        assert!(moved_ids(&report).contains(&gone));
        let restored = restore_memory_core(&store, &FakeEmbedder::default(), MemoryKind::Memory, gone, None).await.unwrap();
        assert_eq!(restored.status, "active");
        assert_eq!(place(&store, gone).await.0.as_deref(), Some(B));
    }

    #[tokio::test]
    async fn a_deleted_row_is_selected_only_when_asked() {
        let store = store().await;
        let id = save_one(&store, fact("a deleted note", None)).await;
        delete_memory_core(&store, id, MemoryKind::Memory, None).await.unwrap();

        assert!(move_to(&store, &ids(&[id]), Some(B)).await.moved.is_empty());
        let report = move_to(&store, &MoveFilter { include_deleted: true, ..ids(&[id]) }, Some(B)).await;
        assert_eq!(moved_ids(&report), vec![id]);
    }

    #[tokio::test]
    async fn a_pair_left_across_projects_is_reported() {
        let store = store().await;
        let a = save_one(&store, fact("the parser is hand written", Some(A))).await;
        let b = save_one(&store, fact("the parser uses nom", Some(A))).await;
        let c = save_one(&store, fact("the parser has no tests", Some(A))).await;
        store
            .conn
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO memory_conflict (new_id, old_id, distance, state, created_at) \
                     VALUES (?2, ?1, 0.2, 'pending', 1), (?3, ?1, 0.2, 'judged', 1)",
                    [a, b, c],
                )
            })
            .await
            .unwrap();

        let report = move_to(&store, &ids(&[a]), Some(B)).await;

        assert_eq!(report.split_pairs, vec![[b, a]], "a judged pair is closed");
    }

    #[tokio::test]
    async fn a_session_summary_moved_into_a_project_is_its_last_session() {
        use crate::memory::prompt_builder::{build_memory_prompt, QueryTexts, DEFAULT_BUDGET};
        use crate::memory::workspace::{EmittedIds, WorkspaceManager};
        let store = store().await;
        let id = save_one(
            &store,
            MemoryEvent { memory_type: Some("session_summary".into()), ..fact("Goal: wire the cache", None) },
        )
        .await;
        let workspaces = tempfile::tempdir().unwrap();
        let workspaces = WorkspaceManager::new(workspaces.path().into());
        let (reach, emitted) = (Reach::AllBut(Vec::new()), EmittedIds::default());
        let prompt = |project: &'static str| {
            build_memory_prompt(
                crate::preferences::Lang::En,
                &store,
                &workspaces,
                Some(Arc::new(FakeEmbedder::default()) as Arc<dyn Embedder>),
                "agent-a",
                Some(project),
                &reach,
                QueryTexts::single_shot(""),
                &emitted,
                None,
                DEFAULT_BUDGET,
            )
        };
        let shown = format!("#{id} session_summary] Goal: wire the cache");
        assert!(!prompt(B).await.unwrap().text.contains(&shown));

        move_to(&store, &ids(&[id]), Some(B)).await;

        assert!(prompt(B).await.unwrap().text.contains(&shown));
        assert!(!prompt(A).await.unwrap().text.contains(&shown));
    }

    #[tokio::test]
    async fn moving_a_superseded_source_does_not_make_its_conclusions_reappear_for_review() {
        use crate::memory::synthesis::conclusions_to_review_count;
        let store = store().await;
        let source = save_one(&store, fact("the cache is per process", Some(A))).await;
        let winner = save_one(&store, fact("the cache is shared", Some(A))).await;
        let conclusion = save_one(&store, fact("caching needs a shared store", Some(A))).await;
        store
            .conn
            .call(move |c| {
                c.execute(
                    "UPDATE memory SET status = 'superseded', superseded_by = ?2, supersede_cause = 'verdict', \
                     last_modified_at = 5 WHERE id = ?1",
                    [source, winner],
                )?;
                c.execute(
                    "INSERT INTO memory_derivation (memory_id, source_id, created_at) VALUES (?1, ?2, 0)",
                    [conclusion, source],
                )?;
                c.execute("INSERT INTO synthesis_review (agent_id, scope_key, reviewed_at) VALUES ('', ?1, 10)", [A])
            })
            .await
            .unwrap();
        let before = conclusions_to_review_count(&store, None, Some(A)).await.unwrap();

        move_to(&store, &ids(&[source]), Some(B)).await;

        assert_eq!(place(&store, source).await.0.as_deref(), Some(B));
        assert_eq!(conclusions_to_review_count(&store, None, Some(A)).await.unwrap(), before);
    }

    #[tokio::test]
    async fn a_move_leaves_vectors_and_the_text_index_alone() {
        let store = store().await;
        let id = save_one(&store, fact("the token file is 0600", None)).await;
        let vectors = |store: Arc<MemoryStore>| async move {
            store
                .conn
                .call(move |c| {
                    c.query_row(
                        "SELECT (SELECT embedding FROM memory_vectors WHERE rowid = ?1), \
                         (SELECT COUNT(*) FROM memory_fts WHERE memory_fts MATCH 'token')",
                        [id],
                        |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, i64>(1)?)),
                    )
                })
                .await
                .unwrap()
        };
        let before = (vectors(store.clone()).await, everything_else(&store).await);

        move_to(&store, &ids(&[id]), Some(B)).await;

        assert_eq!((vectors(store.clone()).await, everything_else(&store).await), before);
    }

    #[tokio::test]
    async fn a_dry_run_reports_what_a_move_does_and_writes_nothing() {
        let store = store().await;
        let pieces = save(&store, fact(&long_text("dry"), None)).await;
        let backups = tempfile::tempdir().unwrap();
        let filter = MoveFilter { from: Some(ProjectFilter::None), ..MoveFilter::default() };

        let dry = move_memories_core(&store, backups.path(), &filter, Some(B.into()), true).await.unwrap();

        assert!(dry.dry_run && dry.move_id.is_none() && dry.snapshot.is_none());
        assert_eq!(projects_of(&store, &pieces).await, vec![None; pieces.len()]);
        assert!(moves_logged(&store).await.is_empty());
        assert_eq!(std::fs::read_dir(backups.path()).unwrap().count(), 0);

        let done = move_memories_core(&store, backups.path(), &filter, Some(B.into()), false).await.unwrap();

        assert_eq!(MoveReport { dry_run: true, move_id: None, snapshot: None, ..done.clone() }, dry);
        assert_eq!(done.move_id, Some(1));
        let snapshot = done.snapshot.expect("a pre-move snapshot");
        assert!(snapshot.ends_with("-pre-move.db") && Path::new(&snapshot).exists(), "{snapshot}");
    }

    #[tokio::test]
    async fn every_moved_row_is_logged_and_the_log_undoes_it() {
        let store = store().await;
        let loose = save_one(&store, fact("an unfiled note", None)).await;
        let filed = save_one(&store, fact("a note filed in A", Some(A))).await;
        let backups = tempfile::tempdir().unwrap();

        let done = move_to(&store, &ids(&[loose, filed]), Some(B)).await;

        let to_b = |id, from: Option<&str>| (1, id, from.map(str::to_string), "agent".to_string(), Some(B.to_string()), "agent".to_string());
        assert_eq!(moves_logged(&store).await, vec![to_b(loose, None), to_b(filed, Some(A))]);

        let undone = undo_move_core(&store, backups.path(), done.move_id.unwrap(), false).await.unwrap();

        assert_eq!(undone.move_id, Some(2));
        assert_eq!(projects_of(&store, &[loose, filed]).await, vec![None, Some(A.to_string())]);
        assert!(undone.snapshot.is_some());
        let again = undo_move_core(&store, backups.path(), 1, false).await.unwrap();
        assert!(again.moved.is_empty() && again.move_id.is_none(), "{again:?}");
        assert!(matches!(undo_move_core(&store, backups.path(), 9, true).await, Err(MoveError::UnknownMove(9))));
    }

    #[tokio::test]
    async fn undoing_leaves_a_row_that_moved_again() {
        let store = store().await;
        let id = save_one(&store, fact("a note moved twice", None)).await;
        let backups = tempfile::tempdir().unwrap();
        let first = move_to(&store, &ids(&[id]), Some(B)).await.move_id.unwrap();
        move_to(&store, &ids(&[id]), Some(A)).await;

        let undone = undo_move_core(&store, backups.path(), first, false).await.unwrap();

        assert!(undone.moved.is_empty());
        assert_eq!(place(&store, id).await.0.as_deref(), Some(A));
    }

    #[tokio::test]
    async fn an_unregistered_target_is_refused() {
        let store = store().await;
        let id = save_one(&store, fact("a note", None)).await;
        let backups = tempfile::tempdir().unwrap();

        let refused = move_memories_core(&store, backups.path(), &ids(&[id]), Some("typo-0000".into()), false).await;

        assert!(matches!(refused, Err(MoveError::UnknownProject(ref p)) if p == "typo-0000"), "{refused:?}");
        assert_eq!(place(&store, id).await.0, None);
    }

    /// A project is a target because it is one, whether or not a folder of
    /// this machine leads to it.
    #[tokio::test]
    async fn a_project_without_a_folder_is_a_target() {
        let store = store().await;
        save_one(&store, fact("a note saved under an id no folder has", Some("idea-1a2b"))).await;
        let id = save_one(&store, fact("a note filed nowhere", None)).await;
        let folders: i64 = store
            .conn
            .call(|c| c.query_row("SELECT COUNT(*) FROM project_path WHERE id = 'idea-1a2b'", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(folders, 0);
        let backups = tempfile::tempdir().unwrap();

        let moved = move_memories_core(&store, backups.path(), &ids(&[id]), Some("idea-1a2b".into()), false).await.unwrap();

        assert_eq!(moved_ids(&moved), vec![id]);
        assert_eq!(place(&store, id).await.0.as_deref(), Some("idea-1a2b"));
    }

    #[tokio::test]
    async fn a_selection_needs_a_filter_and_text_only_over_live_rows() {
        let store = store().await;
        let backups = tempfile::tempdir().unwrap();
        let run = |filter: MoveFilter| {
            let (store, backups) = (store.clone(), backups.path().to_path_buf());
            async move { move_memories_core(&store, &backups, &filter, None, true).await }
        };

        assert!(matches!(run(MoveFilter { include_deleted: true, ..MoveFilter::default() }).await, Err(MoveError::NoSelection)));
        let text_on_deleted = MoveFilter { query: Some("cache".into()), include_deleted: true, ..MoveFilter::default() };
        assert!(matches!(run(text_on_deleted).await, Err(MoveError::TextOnDeleted)));
        assert!(matches!(run(MoveFilter { query: Some("--".into()), ..MoveFilter::default() }).await, Err(MoveError::EmptyText)));
    }

    #[tokio::test]
    async fn the_filters_narrow_the_selection() {
        let store = store().await;
        let mine = save_one(&store, MemoryEvent { session_id: Some("s-1".into()), ..fact("a note of s-1", None) }).await;
        save_one(&store, MemoryEvent { agent_id: "agent-b".into(), ..fact("a note of agent-b", None) }).await;
        save_one(&store, fact("a note in A", Some(A))).await;

        let filter = MoveFilter {
            from: Some(ProjectFilter::None),
            agent_id: Some("agent-a".into()),
            memory_type: Some("semantic_fact".into()),
            source: Some("explicit".into()),
            session_id: Some("s-1".into()),
            ..MoveFilter::default()
        };
        assert_eq!(moved_ids(&move_to(&store, &filter, Some(B)).await), vec![mine]);
    }

    #[tokio::test]
    async fn a_batch_move_selects_by_origin_project() {
        let store = store().await;
        let a_pieces = save(&store, fact(&long_text("alpha"), None)).await;
        let a_single = save_one(&store, fact("a note that came from a", None)).await;
        let b_note = save_one(&store, fact("a note that came from b", None)).await;
        let own = save_one(&store, fact("a note with no origin", None)).await;
        let seed = |ids: Vec<i64>, origin: &'static str| {
            let store = store.clone();
            async move {
                store
                    .conn
                    .call(move |c| {
                        for id in ids {
                            c.execute("UPDATE memory SET origin_project = ?1 WHERE id = ?2", rusqlite::params![origin, id])?;
                        }
                        Ok::<(), rusqlite::Error>(())
                    })
                    .await
                    .unwrap()
            }
        };
        seed(a_pieces.iter().copied().chain([a_single]).collect(), "a").await;
        seed(vec![b_note], "b").await;
        assert!(a_pieces.len() > 1, "{a_pieces:?}");

        let alone = MoveFilter { origin_project: Some("a".into()), ..MoveFilter::default() };
        let dry = move_memories_core(&store, tempfile::tempdir().unwrap().path(), &alone, Some(B.into()), true).await.unwrap();
        assert!(!dry.moved.is_empty(), "origin_project alone is a selection");

        let report = move_to(&store, &alone, Some(B)).await;

        let mut expected: Vec<i64> = a_pieces.iter().copied().chain([a_single]).collect();
        expected.sort();
        let mut moved = moved_ids(&report);
        moved.sort();
        assert_eq!(moved, expected);
        assert_eq!(projects_of(&store, &expected).await, vec![Some(B.to_string()); expected.len()]);
        assert_eq!(projects_of(&store, &[b_note, own]).await, vec![None, None]);

        let narrowed = MoveFilter { origin_project: Some("A".into()), ..MoveFilter::default() };
        let none = move_to(&store, &narrowed, Some(A)).await;
        assert!(none.moved.is_empty(), "the name is compared whole: {none:?}");
    }

    // ── joining two projects ────────────────────────────────────────────

    fn live(id: i64, created_at: i64, key: Option<&str>, hash: &str, moved: bool) -> LiveRow {
        LiveRow {
            id,
            created_at,
            topic_key: key.map(str::to_string),
            content_hash: Some(hash.into()),
            save: None,
            head: id,
            save_live: 1,
            foreign_piece: false,
            moved,
        }
    }

    #[test]
    fn reconcile_keeps_the_newest_save_of_a_key_whichever_side_moved() {
        let k = Some("storage/engine");
        for moved_first in [true, false] {
            let rows = [live(1, 10, k, "h1", moved_first), live(2, 20, k, "h2", !moved_first)];
            assert_eq!(reconcile(&rows), vec![Supersede { row: 1, by: 2 }]);
        }
        let untouched = [live(1, 10, k, "h1", false), live(2, 20, k, "h2", false)];
        assert_eq!(reconcile(&untouched), vec![], "a clash no moved row is part of stays");
    }

    #[test]
    fn reconcile_keeps_the_oldest_row_of_a_text() {
        let rows = [live(1, 10, None, "same", false), live(2, 5, None, "same", true), live(3, 1, None, "other", true)];
        assert_eq!(reconcile(&rows), vec![Supersede { row: 1, by: 2 }]);
    }

    #[test]
    fn reconcile_replaces_a_piece_only_with_its_whole_save() {
        let piece = |id, save, hash: &str, moved| LiveRow { save: Some(save), head: save * 10, save_live: 2, ..live(id, 50, None, hash, moved) };
        let half = [live(1, 1, None, "a", false), piece(70, 7, "a", true), piece(71, 7, "own", true)];
        assert_eq!(reconcile(&half), vec![], "a save with one piece of its own stays whole");

        let whole = [live(1, 1, None, "a", false), live(2, 1, None, "b", false), piece(70, 7, "a", true), piece(71, 7, "b", true)];
        assert_eq!(reconcile(&whole), vec![Supersede { row: 70, by: 1 }, Supersede { row: 71, by: 2 }]);
    }

    #[test]
    fn reconcile_leaves_a_piece_another_save_holds_and_one_key_per_save() {
        let k = Some("api/style");
        let foreign = LiveRow { foreign_piece: true, ..live(1, 1, k, "x", false) };
        let same_save = |id, created_at| LiveRow { save: Some(9), head: 30, save_live: 2, ..live(id, created_at, k, &format!("p{id}"), true) };
        let rows = [foreign, live(2, 5, k, "y", false), same_save(30, 10), same_save(31, 10)];
        assert_eq!(reconcile(&rows), vec![Supersede { row: 2, by: 30 }], "the winner's pieces stay, the foreign piece too");
    }

    #[test]
    fn decide_merge_moves_what_is_not_in_the_target_and_stops_at_other_projects() {
        let origins = vec![A.to_string()];
        let report = decide_merge(
            &[
                unit(vec![row(1, Some(A)), row(2, Some(B))], &[1], Some(B)),
                unit(vec![row(3, Some(A)), row(4, Some("repo-cccc"))], &[3], Some(B)),
                unit(vec![row(5, Some(B))], &[5], Some(B)),
            ],
            &origins,
            B,
            Span::Keep,
        );
        assert_eq!(moved_ids(&report), vec![1]);
        assert_eq!(report.skipped, vec![Skipped { id: 3, refusal: Refusal::SpansProjects }]);
        assert_eq!(report.unchanged, vec![5]);
        assert_eq!(report.split, Vec::<i64>::new());
    }

    #[test]
    fn decide_merge_carrying_takes_the_joined_part_of_a_unit_elsewhere() {
        let origins = vec![A.to_string()];
        let report = decide_merge(
            &[unit(vec![row(3, Some(A)), row(4, Some("repo-cccc")), row(6, None)], &[3], Some(B))],
            &origins,
            B,
            Span::Carry,
        );
        assert_eq!(moved_ids(&report), vec![3], "the rows elsewhere, or in no project, stay");
        assert_eq!(report.split, vec![3]);
        assert_eq!(report.skipped, Vec::<Skipped>::new());
    }

    /// Leaves `id` a tombstone of `into`, as a merge does.
    async fn merged_into(store: &MemoryStore, id: &str, into: &str) {
        let (id, into) = (id.to_string(), into.to_string());
        store
            .conn
            .call(move |c| {
                c.execute(
                    "INSERT INTO project (id, name, born, created_at) VALUES (?1, ?1, 'root', 0) \
                     ON CONFLICT (id) DO NOTHING",
                    [&id],
                )?;
                c.execute("UPDATE project SET merged_into = ?2, merged_at = 1 WHERE id = ?1", [&id, &into])
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn a_move_to_a_merged_project_lands_in_its_target() {
        let store = store().await;
        merged_into(&store, "gone-cccc", B).await;
        let id = save_one(&store, fact("the parser is written by hand", Some(A))).await;
        let backups = tempfile::tempdir().unwrap();

        let filter = MoveFilter { ids: vec![id], ..MoveFilter::default() };
        let report = move_memories_core(&store, backups.path(), &filter, Some("gone-cccc".into()), false).await.unwrap();

        assert_eq!(report.moved.iter().map(|m| m.to_project.clone()).collect::<Vec<_>>(), vec![Some(B.to_string())]);
        assert_eq!(place(&store, id).await.0.as_deref(), Some(B));
    }

    #[tokio::test]
    async fn undoing_a_move_whose_origin_was_merged_returns_to_its_target() {
        let store = store().await;
        register(&store, "web-cccc").await;
        let id = save_one(&store, fact("the parser is written by hand", Some(A))).await;
        let backups = tempfile::tempdir().unwrap();
        let filter = MoveFilter { ids: vec![id], ..MoveFilter::default() };
        let moved = move_memories_core(&store, backups.path(), &filter, Some(B.into()), false).await.unwrap();
        merged_into(&store, A, "web-cccc").await;

        undo_move_core(&store, backups.path(), moved.move_id.unwrap(), false).await.unwrap();

        assert_eq!(place(&store, id).await.0.as_deref(), Some("web-cccc"), "the project it came from is web-cccc now");
    }

    async fn merge(store: &MemoryStore, from: &str, to: &str) -> MoveReport {
        let (from, to) = (vec![from.to_string()], to.to_string());
        store
            .conn
            .call(move |c| {
                let tx = c.transaction()?;
                let report = merge_in(&tx, &from, &to, Span::Keep, TimestampMs::now())?;
                tx.commit()?;
                Ok::<_, rusqlite::Error>(report)
            })
            .await
            .unwrap()
    }

    /// `(project_id, status, superseded_by, supersede_cause)` of every row.
    async fn states(store: &MemoryStore) -> Vec<(i64, Option<String>, String, Option<i64>, Option<String>)> {
        store
            .conn
            .call(|c| {
                let mut stmt =
                    c.prepare("SELECT id, project_id, status, superseded_by, supersede_cause FROM memory ORDER BY id")?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?;
                rows.collect()
            })
            .await
            .unwrap()
    }

    fn at(evt: MemoryEvent, ms: u64) -> MemoryEvent {
        MemoryEvent { timestamp_ms: ms, ..evt }
    }

    #[tokio::test]
    async fn merging_keeps_the_newest_version_of_a_key() {
        let store = store().await;
        let theirs = save_one(&store, at(keyed("Storage engine is SQLite", Some(A), "storage/engine"), 20)).await;
        let ours = save_one(&store, at(keyed("Storage engine is SQLite with WAL", Some(B), "storage/engine"), 10)).await;

        let report = merge(&store, A, B).await;

        assert_eq!(moved_ids(&report), vec![theirs]);
        assert_eq!(report.superseded, vec![Supersede { row: ours, by: theirs }]);
        let after = states(&store).await;
        assert!(after.contains(&(ours, Some(B.into()), "superseded".into(), Some(theirs), Some("revision".into()))), "{after:?}");
        let deleted = delete_memory_core(&store, theirs, MemoryKind::Memory, None).await.unwrap();
        assert_eq!(deleted.revived.iter().map(|r| r.id).collect::<Vec<_>>(), vec![ours], "the older version comes back");
    }

    #[tokio::test]
    async fn merging_keeps_the_oldest_row_of_a_text() {
        let store = store().await;
        let text = "the daemon listens on clawg.sock";
        let theirs = save_one(&store, at(fact(text, Some(A)), 5)).await;
        let ours = save_one(&store, at(fact(text, Some(B)), 9)).await;
        let other = save_one(&store, at(fact("tests run with cargo nextest", Some(A)), 6)).await;

        let report = merge(&store, A, B).await;

        assert_eq!(moved_ids(&report), vec![theirs, other]);
        assert_eq!(report.superseded, vec![Supersede { row: ours, by: theirs }]);
        let found = found_from(&store, Some(B), text).await;
        assert!(found.contains(&theirs) && !found.contains(&ours), "{found:?}");
    }

    #[tokio::test]
    async fn merging_does_not_depend_on_which_clone_came_first() {
        let mut outcomes = Vec::new();
        for (from, to) in [(A, B), (B, A)] {
            let store = store().await;
            let old = save_one(&store, at(keyed("Logs go to serve.log", Some(A), "logging/target"), 10)).await;
            let new = save_one(&store, at(keyed("Logs go to serve.log, rotated", Some(B), "logging/target"), 20)).await;
            let first = save_one(&store, at(fact("the socket is clawg.sock", Some(B)), 1)).await;
            let second = save_one(&store, at(fact("the socket is clawg.sock", Some(A)), 2)).await;
            merge(&store, from, to).await;
            let live: Vec<i64> = states(&store).await.into_iter().filter(|r| r.2 == "active").map(|r| r.0).collect();
            outcomes.push((live, vec![new, first], old, second));
        }
        for (live, expected, _, _) in &outcomes {
            assert_eq!(live, expected);
        }
    }

    #[tokio::test]
    async fn merging_a_long_save_twice_keeps_one_whole() {
        let store = store().await;
        let text = long_text("tokio");
        let theirs = save(&store, at(fact(&text, Some(A)), 1)).await;
        let ours = save(&store, at(fact(&text, Some(B)), 2)).await;
        assert!(theirs.len() > 1, "{theirs:?}");

        let report = merge(&store, A, B).await;

        let replaced: Vec<i64> = report.superseded.iter().map(|s| s.row).collect();
        assert_eq!(replaced, ours, "every piece of the newer copy, none of the older");
    }

    #[tokio::test]
    async fn undoing_a_merge_restores_every_row_and_state() {
        let store = store().await;
        let a_key = save_one(&store, at(keyed("Storage engine is SQLite", Some(A), "storage/engine"), 20)).await;
        save_one(&store, at(keyed("Storage engine is SQLite with WAL", Some(B), "storage/engine"), 10)).await;
        save_one(&store, at(fact("the daemon listens on clawg.sock", Some(A)), 5)).await;
        save_one(&store, at(fact("the daemon listens on clawg.sock", Some(B)), 9)).await;
        let deleted = save_one(&store, at(fact("an old note", Some(A)), 3)).await;
        delete_memory_core(&store, deleted, MemoryKind::Memory, None).await.unwrap();
        let before = (states(&store).await, everything_else(&store).await);

        let report = merge(&store, A, B).await;
        assert_eq!(report.superseded.len(), 2);
        assert!(moved_ids(&report).contains(&a_key) && moved_ids(&report).contains(&deleted));
        let backups = tempfile::tempdir().unwrap();
        undo_move_core(&store, backups.path(), report.move_id.unwrap(), false).await.unwrap();

        assert_eq!((states(&store).await, everything_else(&store).await), before);
    }

    #[tokio::test]
    async fn a_deleted_project_is_refused_by_what_manages_projects() {
        let store = store().await;
        let id = save_one(&store, fact("a note", Some(A))).await;
        let theirs = save_one(&store, fact("a note of the project to delete", Some(B))).await;
        let backups = tempfile::tempdir().unwrap();
        let away = move_to(&store, &ids(&[theirs]), Some(A)).await.move_id.unwrap();
        let gone = save_one(&store, fact("a note deleted with the project", Some(B))).await;
        store.conn.call(move |c| c.execute("UPDATE memory SET deleted_at = 4 WHERE id = ?1", [gone])).await.unwrap();
        store
            .conn
            .call(|c| {
                c.execute_batch(
                    "UPDATE project SET deleted_at = 5 WHERE id = 'repo-bbbb';
                     INSERT INTO project_delete (delete_id, project_id, with_memories, deleted_at) VALUES (1, 'repo-bbbb', 0, 5);",
                )
            })
            .await
            .unwrap();

        let moved = move_memories_core(&store, backups.path(), &ids(&[id]), Some(B.into()), false).await;
        assert!(matches!(&moved, Err(MoveError::Deleted { id, delete_id }) if id == B && *delete_id == Some(1)), "{moved:?}");
        let patched = store.conn.call(move |c| -> Result<MoveReport, MoveError> { plan_one_in(c, id, Some(B), None) }).await;
        assert!(matches!(&patched, Err(tokio_rusqlite::Error::Error(MoveError::Deleted { id, .. })) if id == B), "{patched:?}");
        assert_eq!(place(&store, id).await.0.as_deref(), Some(A));

        // Its deleted rows can still be taken out before a purge.
        let from = MoveFilter { from: Some(ProjectFilter::Project(B.into())), include_deleted: true, ..MoveFilter::default() };
        let rescued = move_memories_core(&store, backups.path(), &from, Some(A.into()), false).await.unwrap();
        assert_eq!(moved_ids(&rescued), vec![gone]);

        // The undo of a move that gives a live row back brings it back, as any live row does.
        undo_move_core(&store, backups.path(), away, false).await.unwrap();
        assert_eq!(place(&store, theirs).await.0.as_deref(), Some(B));
        let back: Option<i64> =
            store.conn.call(|c| c.query_row("SELECT deleted_at FROM project WHERE id = 'repo-bbbb'", [], |r| r.get(0))).await.unwrap();
        assert_eq!(back, None);
    }
}
