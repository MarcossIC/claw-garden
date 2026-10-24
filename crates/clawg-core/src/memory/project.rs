//! Registers a project's canonical root, gives it an id, and joins into its
//! project the rows saved under the ids it had before.
//!
//! Each root keeps one id in `project` (its root id), which the docs index
//! uses. Its memories live under its project id: the id the root's remote was
//! first registered with (`project_remote`, V24), so the clones of one remote
//! share them, or the root id without a remote. Rows of `memory` join through
//! the move engine, which logs them in `memory_move` and leaves one live row
//! per key and per text (see [`merge_in`]); the other tables move row by row
//! with `project_rekey`, whose undo SQL is in the migration that creates it.
//! `memory_fts` needs nothing: it reads `project_id` from `memory`.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension};

use crate::memory::backup::Reason;
use crate::memory::clock::TimestampMs;
use crate::memory::identity::{candidate_id, derive_project_id, CANDIDATES};
use crate::memory::relocate::merge_in;
use crate::memory::workspace::WorkspaceManager;
use crate::memory::{MemoryError, MemoryStore};

/// The tables besides `memory` whose rows carry a `project_id`. A literal
/// list rather than interpolated input, so no caller value ever reaches a
/// table name.
const OTHER_PROJECT_TABLES: [&str; 2] = ["episodic_trajectory", "procedural_skill"];

/// What a caller asks of [`register_project`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registration<'a> {
    pub root: &'a Path,
    /// The key of the root's remote (`origin_key`), or none.
    pub remote: Option<&'a str>,
    /// Ids the same project was saved under before.
    pub legacy_ids: &'a [String],
    /// Whether the daemon may read the root's files.
    pub index_docs: bool,
    /// The caller uses the ids the answer gives. One that does not derives
    /// the id from the root itself, so it keeps the rule it was written for.
    pub assigned: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterOutcome {
    /// Where the root's memories are read and written.
    pub project_id: String,
    /// The root's own id, which its docs index uses.
    pub root_id: String,
    /// Rows moved onto `project_id` by this call, across every table.
    pub rekeyed: usize,
    /// Whether the daemon may read the root's files, as stored after this call.
    pub index_docs: bool,
    /// The move in `memory_move` that joined the rows of `memory`, which
    /// `clawg projects move --undo` takes.
    pub move_id: Option<i64>,
    /// The `pre-rekey` snapshot taken before rows moved.
    pub snapshot: Option<PathBuf>,
}

#[derive(Debug, thiserror::Error)]
pub enum RegisterError {
    #[error("{0:?} is not a project id")]
    InvalidLegacyId(String),
    /// A caller that derives its own id, for a root whose id belongs to
    /// another root. Moving rows into it would merge two projects, so nothing
    /// moves and the caller keeps the id it had.
    #[error("project id {id} already belongs to {existing}")]
    RootCollision { id: String, existing: String },
    #[error("every id {root} may take belongs to another root", root = .0.display())]
    NoFreeId(PathBuf),
    #[error(transparent)]
    Store(#[from] MemoryError),
}

/// Whether `s` has the shape [`derive_project_id`] produces:
/// a non-empty name, a dash and four lowercase hex digits.
pub fn is_project_id(s: &str) -> bool {
    match s.rsplit_once('-') {
        Some((name, hash)) => {
            !name.is_empty()
                && hash.len() == 4
                && hash.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        }
        None => false,
    }
}

/// The ids a registration settles on, before anything is written.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Plan {
    root_id: String,
    project_id: String,
    /// Whether `project` has no row for the root yet.
    new_root: bool,
    /// Ids whose rows join the project.
    legacy: Vec<String>,
    /// Whether any of them has a row.
    moves: bool,
}

/// The id `project` records for `root`, if any.
fn recorded_id_in(c: &Connection, root: &str) -> rusqlite::Result<Option<String>> {
    c.query_row("SELECT id FROM project WHERE root_path = ?1 ORDER BY created_at, id LIMIT 1", [root], |r| r.get(0))
        .optional()
}

/// The root `project` records for `id`, if any.
fn root_of_in(c: &Connection, id: &str) -> rusqlite::Result<Option<String>> {
    c.query_row("SELECT root_path FROM project WHERE id = ?1", [id], |r| r.get(0)).optional()
}

/// The project the clones of `remote` share, if one registered it.
fn remote_project_in(c: &Connection, remote: &str) -> rusqlite::Result<Option<String>> {
    c.query_row("SELECT project_id FROM project_remote WHERE remote = ?1", [remote], |r| r.get(0)).optional()
}

/// The id `root` has, or the first [`candidate_id`] no other root has.
pub(crate) fn root_id_in(c: &Connection, root: &Path) -> Result<(String, bool), RegisterError> {
    let root_path = root.to_string_lossy();
    if let Some(id) = recorded_id_in(c, &root_path).map_err(MemoryError::Sqlite)? {
        return Ok((id, false));
    }
    for n in 0..CANDIDATES {
        let id = candidate_id(root, n);
        if root_of_in(c, &id).map_err(MemoryError::Sqlite)?.is_none() {
            return Ok((id, true));
        }
    }
    Err(RegisterError::NoFreeId(root.to_path_buf()))
}

/// The project id `root` reads and writes under, as the store says now and
/// without writing: its remote's project, or its recorded id, or the one it
/// would be given.
pub fn project_id_in(c: &Connection, root: &Path, remote: Option<&str>) -> rusqlite::Result<String> {
    if let Some(project) = remote.map(|r| remote_project_in(c, r)).transpose()?.flatten() {
        return Ok(project);
    }
    match root_id_in(c, root) {
        Ok((id, _)) => Ok(id),
        Err(RegisterError::Store(MemoryError::Sqlite(e))) => Err(e),
        Err(_) => Ok(derive_project_id(root)),
    }
}

/// The ids each of `roots` (with the key of its remote) would get, as
/// [`register_project`] gives them, read without writing: a root later in the
/// list never takes an id or a remote an earlier one was given. Each answer is
/// `(root id, project id)`.
pub(crate) fn plan_ids_in(c: &Connection, roots: &[(PathBuf, Option<String>)]) -> rusqlite::Result<Vec<(String, String)>> {
    let mut claimed: Vec<String> = Vec::new();
    let mut remotes: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut out = Vec::new();
    for (root, remote) in roots {
        let root_id = match recorded_id_in(c, &root.to_string_lossy())? {
            Some(id) => id,
            None => {
                let mut free = None;
                for n in 0..CANDIDATES {
                    let id = candidate_id(root, n);
                    if !claimed.contains(&id) && root_of_in(c, &id)?.is_none() {
                        free = Some(id);
                        break;
                    }
                }
                free.unwrap_or_else(|| derive_project_id(root))
            }
        };
        claimed.push(root_id.clone());
        let project_id = match remote {
            Some(r) => match remotes.get(r) {
                Some(p) => p.clone(),
                None => {
                    let p = remote_project_in(c, r)?.unwrap_or_else(|| root_id.clone());
                    remotes.insert(r.clone(), p.clone());
                    p
                }
            },
            None => root_id.clone(),
        };
        out.push((root_id, project_id));
    }
    Ok(out)
}

/// Settles the ids of a registration inside the caller's transaction.
fn plan_in(c: &Connection, reg: &Registration<'_>) -> Result<Plan, RegisterError> {
    let root_path = reg.root.to_string_lossy().into_owned();
    let sql = |e: rusqlite::Error| RegisterError::Store(MemoryError::Sqlite(e));
    let (root_id, new_root, project_id) = if reg.assigned {
        let (root_id, new_root) = root_id_in(c, reg.root)?;
        let shared = reg.remote.map(|r| remote_project_in(c, r)).transpose().map_err(sql)?.flatten();
        let project_id = shared.unwrap_or_else(|| root_id.clone());
        (root_id, new_root, project_id)
    } else {
        let id = derive_project_id(reg.root);
        let owner = root_of_in(c, &id).map_err(sql)?;
        if let Some(other) = owner.as_ref().filter(|o| **o != root_path) {
            return Err(RegisterError::RootCollision { id, existing: other.clone() });
        }
        if recorded_id_in(c, &root_path).map_err(sql)?.is_some_and(|recorded| recorded != id) {
            return Err(RegisterError::RootCollision { id, existing: root_path });
        }
        (id.clone(), owner.is_none(), id)
    };
    let mut legacy = Vec::new();
    for id in reg.legacy_ids.iter().chain(std::iter::once(&root_id)) {
        if *id == project_id || legacy.contains(id) {
            continue;
        }
        // A legacy id that is another root's own id would take that
        // project's rows.
        if root_of_in(c, id).map_err(sql)?.is_some_and(|other| other != root_path) {
            log::warn!("{id} belongs to another registered project; its rows were not moved to {project_id}");
            continue;
        }
        legacy.push(id.clone());
    }
    let ids = serde_json::to_string(&legacy).unwrap_or_default();
    let moves: bool = c
        .query_row(
            "SELECT EXISTS (SELECT 1 FROM memory WHERE project_id IN (SELECT value FROM json_each(?1))) \
             OR EXISTS (SELECT 1 FROM episodic_trajectory WHERE project_id IN (SELECT value FROM json_each(?1))) \
             OR EXISTS (SELECT 1 FROM procedural_skill WHERE project_id IN (SELECT value FROM json_each(?1)))",
            [ids],
            |r| r.get(0),
        )
        .map_err(sql)?;
    Ok(Plan { root_id, project_id, new_root, legacy, moves })
}

/// Writes `plan` inside the caller's transaction and returns how many rows
/// moved, the move that joined the rows of `memory`, and the mark
/// `index_docs` keeps.
fn write_in(
    c: &Connection,
    reg: &Registration<'_>,
    plan: &Plan,
    now: TimestampMs,
) -> rusqlite::Result<(usize, Option<i64>, bool)> {
    let root_path = reg.root.to_string_lossy().into_owned();
    if plan.new_root {
        c.execute(
            "INSERT INTO project (id, root_path, created_at, index_docs) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![plan.root_id, root_path, now, reg.index_docs],
        )?;
    }
    let indexed: bool = c.query_row("SELECT index_docs FROM project WHERE id = ?1", [&plan.root_id], |r| r.get(0))?;
    if reg.index_docs && !indexed {
        c.execute("UPDATE project SET index_docs = 1 WHERE id = ?1", [&plan.root_id])?;
    }
    if let (true, Some(remote)) = (reg.assigned, reg.remote) {
        c.execute(
            "INSERT OR IGNORE INTO project_remote (remote, project_id, created_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![remote, plan.project_id, now],
        )?;
    }
    let (mut moved, mut move_id) = (0, None);
    if !plan.legacy.is_empty() {
        let merged = merge_in(c, &plan.legacy, &plan.project_id, now)?;
        (moved, move_id) = (merged.moved.len(), merged.move_id);
        for legacy_id in &plan.legacy {
            for table in OTHER_PROJECT_TABLES {
                c.execute(
                    &format!(
                        "INSERT INTO project_rekey (tbl, row_id, legacy_id, project_id, rekeyed_at) \
                         SELECT '{table}', id, project_id, ?2, ?3 FROM {table} WHERE project_id = ?1"
                    ),
                    rusqlite::params![legacy_id, plan.project_id, now],
                )?;
                moved += c.execute(
                    &format!("UPDATE {table} SET project_id = ?2 WHERE project_id = ?1"),
                    rusqlite::params![legacy_id, plan.project_id],
                )?;
            }
        }
    }
    Ok((moved, move_id, indexed || reg.index_docs))
}

/// The outcome of one pass of [`register_project`].
enum Pass {
    Done(RegisterOutcome, Vec<String>),
    /// Rows would move: the caller snapshots and runs the pass again.
    WouldMove,
}

/// Records `reg.root` as a project and joins into its project every row of
/// each legacy id, soft-deleted rows included, then hands each legacy
/// `workspace.md` to the project (see [`WorkspaceManager::adopt_legacy`]).
/// When rows are to move it first snapshots the store as `pre-rekey` into
/// `backups`, and decides again after.
///
/// `index_docs` only ever raises the stored mark: any tool that names a path
/// registers its root, and such a call must not take an index away from a
/// root the user accepted.
///
/// Idempotent: once a legacy id has no rows left, a repeat moves nothing.
pub async fn register_project(
    store: &MemoryStore,
    workspaces: &WorkspaceManager,
    backups: &Path,
    reg: Registration<'_>,
) -> Result<RegisterOutcome, RegisterError> {
    if let Some(bad) = reg.legacy_ids.iter().find(|id| !is_project_id(id)) {
        return Err(RegisterError::InvalidLegacyId(bad.clone()));
    }
    let owned = (reg.root.to_path_buf(), reg.remote.map(str::to_string), reg.legacy_ids.to_vec());
    let (index_docs, assigned) = (reg.index_docs, reg.assigned);
    let pass = move |snapshot: Option<PathBuf>| {
        let (root, remote, legacy_ids) = owned.clone();
        move |c: &mut Connection| -> Result<Pass, RegisterError> {
            let reg = Registration { root: &root, remote: remote.as_deref(), legacy_ids: &legacy_ids, index_docs, assigned };
            let tx = c.transaction().map_err(MemoryError::Sqlite)?;
            let plan = plan_in(&tx, &reg)?;
            if plan.moves && snapshot.is_none() {
                return Ok(Pass::WouldMove);
            }
            let (rekeyed, move_id, index_docs) =
                write_in(&tx, &reg, &plan, TimestampMs::now()).map_err(MemoryError::Sqlite)?;
            tx.commit().map_err(MemoryError::Sqlite)?;
            let outcome = RegisterOutcome {
                project_id: plan.project_id,
                root_id: plan.root_id,
                rekeyed,
                index_docs,
                move_id,
                snapshot,
            };
            Ok(Pass::Done(outcome, plan.legacy))
        }
    };
    let mut outcome = store.conn.call(pass(None)).await.map_err(connection_error)?;
    if let Pass::WouldMove = outcome {
        let snapshot = store.snapshot(backups.to_path_buf(), Reason::PreRekey).await?;
        outcome = store.conn.call(pass(Some(snapshot))).await.map_err(connection_error)?;
    }
    let Pass::Done(outcome, legacy) = outcome else {
        unreachable!("a pass after the snapshot always writes");
    };

    // After the commit and outside it: a file copy that fails must not undo
    // rows already moved, and the legacy file is still there for the next try.
    for legacy_id in &legacy {
        if let Err(e) = workspaces.adopt_legacy(legacy_id, &outcome.project_id).await {
            log::warn!("could not carry the workspace of {legacy_id} over to {}: {e}", outcome.project_id);
        }
    }
    Ok(outcome)
}

fn connection_error(e: tokio_rusqlite::Error<RegisterError>) -> RegisterError {
    match e {
        tokio_rusqlite::Error::Error(e) => e,
        other => RegisterError::Store(MemoryError::Backend(other.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::identity::candidate_id;
    use crate::memory::relocate::undo_move_core;

    #[test]
    fn project_ids_are_a_name_a_dash_and_four_hex_digits() {
        for ok in ["repo-1a2b", "claw-garden-00ff", "project-0000"] {
            assert!(is_project_id(ok), "{ok}");
        }
        for bad in ["repo", "-1a2b", "repo-1A2B", "repo-1a2", "repo-1a2bc", "repo-zzzz", "", "repo 1a2b"] {
            assert!(!is_project_id(bad), "{bad}");
        }
    }

    struct Fixture {
        store: MemoryStore,
        ws: WorkspaceManager,
        _dir: tempfile::TempDir,
        backups: PathBuf,
    }

    impl Fixture {
        async fn new() -> Self {
            let store = MemoryStore::new_for_test().await.unwrap();
            let dir = tempfile::tempdir().unwrap();
            let ws = WorkspaceManager::new(dir.path().join("workspaces"));
            let backups = dir.path().join("backups");
            Fixture { store, ws, _dir: dir, backups }
        }

        async fn seeded() -> Self {
            let f = Self::new().await;
            for (project, content, deleted) in
                [("src-1a2b", "zanzibar parser decision", None), ("src-1a2b", "second fact", Some(5_i64)), ("other-9f9f", "unrelated", None)]
            {
                f.memory(project, content, deleted, 0).await;
            }
            f.store
                .conn
                .call(|c| {
                    c.execute(
                        "INSERT INTO episodic_trajectory \
                         (scope, agent_id, project_id, session_id, task, created_at, \
                          last_modified_at, last_accessed_at) \
                         VALUES ('agent', 'agent-a', 'src-1a2b', 's', 't', 0, 0, 0)",
                        [],
                    )?;
                    c.execute(
                        "INSERT INTO procedural_skill \
                         (scope, agent_id, project_id, skill_name, trigger, promoted_at, \
                          source_trajectory_ids, created_at, last_modified_at, last_accessed_at) \
                         VALUES ('agent', 'agent-a', 'src-1a2b', 'k', 't', 0, '[]', 0, 0, 0)",
                        [],
                    )?;
                    Ok::<_, rusqlite::Error>(())
                })
                .await
                .unwrap();
            f
        }

        async fn memory(&self, project: &str, content: &str, deleted: Option<i64>, created_at: i64) -> i64 {
            let (project, content) = (project.to_string(), content.to_string());
            self.store
                .conn
                .call(move |c| {
                    c.execute(
                        "INSERT INTO memory \
                         (scope, agent_id, project_id, content, memory_type, source, \
                          created_at, created_by_type, created_by_ref, \
                          last_modified_at, last_accessed_at, importance, tier, status, deleted_at, content_hash) \
                         VALUES ('agent', 'agent-a', ?1, ?2, 'observation', 'agent', \
                          ?4, 'agent', 'agent-a', 0, 0, 5.0, 3, 'active', ?3, 'h:' || ?2)",
                        rusqlite::params![project, content, deleted, created_at],
                    )?;
                    Ok::<_, rusqlite::Error>(c.last_insert_rowid())
                })
                .await
                .unwrap()
        }

        async fn root(&self, id: &str, root: &str) {
            let (id, root) = (id.to_string(), root.to_string());
            self.store
                .conn
                .call(move |c| {
                    c.execute(
                        "INSERT INTO project (id, root_path, created_at, index_docs) VALUES (?1, ?2, 0, 0)",
                        rusqlite::params![id, root],
                    )
                })
                .await
                .unwrap();
        }

        async fn register(&self, reg: Registration<'_>) -> Result<RegisterOutcome, RegisterError> {
            register_project(&self.store, &self.ws, &self.backups, reg).await
        }

        async fn count(&self, sql: &'static str) -> i64 {
            self.store.conn.call(move |c| c.query_row(sql, [], |r| r.get(0))).await.unwrap()
        }

        async fn snapshots(&self) -> usize {
            std::fs::read_dir(&self.backups).map(|d| d.count()).unwrap_or(0)
        }
    }

    fn reg<'a>(root: &'a Path, legacy: &'a [String]) -> Registration<'a> {
        Registration { root, remote: None, legacy_ids: legacy, index_docs: true, assigned: true }
    }

    fn cloned<'a>(root: &'a Path, remote: &'a str) -> Registration<'a> {
        Registration { remote: Some(remote), ..reg(root, &[]) }
    }

    async fn project_ids(store: &MemoryStore) -> Vec<(String, i64, Option<String>)> {
        store
            .conn
            .call(|c| {
                let mut stmt = c.prepare(
                    "SELECT 'memory', id, project_id FROM memory \
                     UNION ALL SELECT 'episodic_trajectory', id, project_id FROM episodic_trajectory \
                     UNION ALL SELECT 'procedural_skill', id, project_id FROM procedural_skill \
                     ORDER BY 1, 2",
                )?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect();
                rows
            })
            .await
            .unwrap()
    }

    async fn index_docs_of(store: &MemoryStore, id: &str) -> Vec<i64> {
        let id = id.to_string();
        store
            .conn
            .call(move |c| {
                let mut stmt = c.prepare("SELECT index_docs FROM project WHERE id = ?1")?;
                let marks = stmt.query_map([id], |r| r.get(0))?.collect();
                marks
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn rows_of_a_legacy_id_move_to_the_resolved_root() {
        let f = Fixture::seeded().await;
        let root = Path::new("/work/repo");
        let id = derive_project_id(root);
        let legacy = ["src-1a2b".to_string()];

        let outcome = f.register(reg(root, &legacy)).await.unwrap();

        assert_eq!((outcome.project_id.as_str(), outcome.root_id.as_str(), outcome.rekeyed, outcome.index_docs), (id.as_str(), id.as_str(), 4, true));
        let ids = project_ids(&f.store).await;
        let moved = ids.iter().filter(|(_, _, p)| p.as_deref() == Some(id.as_str())).count();
        assert_eq!(moved, 4, "{ids:?}");
        assert!(ids.iter().any(|(_, _, p)| p.as_deref() == Some("other-9f9f")), "{ids:?}");
        assert_eq!(f.count("SELECT COUNT(*) FROM memory_move").await, 2, "memory rows are logged as a move");
        assert_eq!(f.count("SELECT COUNT(*) FROM project_rekey").await, 2, "the other tables in project_rekey");
        assert_eq!(f.count("SELECT COUNT(*) FROM project").await, 1);
    }

    #[tokio::test]
    async fn registering_again_moves_nothing() {
        let f = Fixture::seeded().await;
        let root = Path::new("/work/repo");
        let legacy = ["src-1a2b".to_string()];
        f.register(reg(root, &legacy)).await.unwrap();

        let again = f.register(reg(root, &legacy)).await.unwrap();

        assert_eq!((again.rekeyed, again.snapshot), (0, None));
        assert_eq!(f.count("SELECT COUNT(*) FROM memory_move").await, 2);
        assert_eq!(f.count("SELECT COUNT(*) FROM project_rekey").await, 2);
        assert_eq!(f.snapshots().await, 1, "only the registration that moved rows took one");
    }

    #[tokio::test]
    async fn a_registration_that_moves_rows_snapshots_first() {
        let f = Fixture::seeded().await;
        let nothing = f.register(reg(Path::new("/work/empty"), &[])).await.unwrap();
        assert_eq!((nothing.snapshot, f.snapshots().await), (None, 0));

        let legacy = ["src-1a2b".to_string()];
        let moved = f.register(reg(Path::new("/work/repo"), &legacy)).await.unwrap();

        let snapshot = moved.snapshot.expect("a snapshot");
        let name = snapshot.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.ends_with("-pre-rekey.db"), "{name}");
        let held: i64 = rusqlite::Connection::open(&snapshot)
            .unwrap()
            .query_row("SELECT COUNT(*) FROM memory WHERE project_id = 'src-1a2b'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(held, 2, "the snapshot is of the store before the move");
    }

    #[tokio::test]
    async fn the_logs_restore_every_row_a_registration_moved() {
        let f = Fixture::seeded().await;
        let before = project_ids(&f.store).await;
        let legacy = ["src-1a2b".to_string()];
        let moved = f.register(reg(Path::new("/work/repo"), &legacy)).await.unwrap();
        let again = f.register(reg(Path::new("/work/repo"), &legacy)).await.unwrap();
        assert_eq!((moved.move_id, again.move_id), (Some(1), None), "a registration that moves nothing makes no move");

        undo_move_core(&f.store, &f.backups, 1, false).await.unwrap();
        f.store
            .conn
            .call(|c| {
                for table in OTHER_PROJECT_TABLES {
                    c.execute(
                        &format!(
                            "UPDATE {table} SET project_id = r.legacy_id FROM project_rekey AS r \
                             WHERE r.tbl = '{table}' AND r.row_id = {table}.id \
                               AND r.project_id = {table}.project_id"
                        ),
                        [],
                    )?;
                }
                Ok::<_, rusqlite::Error>(())
            })
            .await
            .unwrap();

        assert_eq!(project_ids(&f.store).await, before);
    }

    #[tokio::test]
    async fn a_moved_row_is_found_lexically_under_its_new_project() {
        let f = Fixture::seeded().await;
        let legacy = ["src-1a2b".to_string()];
        let outcome = f.register(reg(Path::new("/work/repo"), &legacy)).await.unwrap();

        let id = outcome.project_id;
        let found: i64 = f
            .store
            .conn
            .call(move |c| {
                c.query_row(
                    "SELECT COUNT(*) FROM memory_fts WHERE memory_fts MATCH 'zanzibar' AND project_id = ?1",
                    [id],
                    |r| r.get(0),
                )
            })
            .await
            .unwrap();
        assert_eq!(found, 1);
    }

    #[tokio::test]
    async fn the_root_of_always_keeps_its_id() {
        let f = Fixture::new().await;
        let root = Path::new("/work/repo");
        let id = derive_project_id(root);
        let row = f.memory(&id, "saved before ids were assigned", None, 0).await;

        let outcome = f.register(cloned(root, "github.com/acme/repo")).await.unwrap();

        assert_eq!((outcome.project_id.as_str(), outcome.root_id.as_str(), outcome.rekeyed), (id.as_str(), id.as_str(), 0));
        assert_eq!(project_ids(&f.store).await, vec![("memory".into(), row, Some(id))]);
        assert_eq!(f.count("SELECT COUNT(*) FROM project_remote").await, 1);
    }

    #[tokio::test]
    async fn a_root_whose_id_is_taken_gets_the_next_free_one() {
        let f = Fixture::new().await;
        let root = Path::new("/work/repo");
        let taken = derive_project_id(root);
        f.root(&taken, "/elsewhere/repo").await;
        let theirs = f.memory(&taken, "the other project's note", None, 0).await;

        let outcome = f.register(reg(root, &[])).await.unwrap();

        assert_eq!(outcome.root_id, candidate_id(root, 1));
        assert_eq!(outcome.project_id, outcome.root_id);
        assert_eq!(outcome.rekeyed, 0, "the other root's id is never a legacy one");
        assert_eq!(project_ids(&f.store).await, vec![("memory".into(), theirs, Some(taken.clone()))]);
        assert_eq!(index_docs_of(&f.store, &taken).await, vec![0], "the other root's mark is left alone");
        assert_eq!(index_docs_of(&f.store, &outcome.root_id).await, vec![1]);
    }

    #[tokio::test]
    async fn a_root_keeps_the_id_it_was_given() {
        let f = Fixture::new().await;
        let root = Path::new("/work/repo");
        f.root(&derive_project_id(root), "/elsewhere/repo").await;
        let first = f.register(reg(root, &[])).await.unwrap();

        let again = f.register(reg(root, &[])).await.unwrap();

        assert_eq!(again.root_id, first.root_id);
        assert_eq!(f.count("SELECT COUNT(*) FROM project").await, 2);
    }

    #[tokio::test]
    async fn every_id_taken_is_an_error_that_writes_nothing() {
        let f = Fixture::new().await;
        let root = Path::new("/work/repo");
        for n in 0..CANDIDATES {
            f.root(&candidate_id(root, n), &format!("/elsewhere/{n}")).await;
        }

        let err = f.register(reg(root, &[])).await.unwrap_err();

        assert!(matches!(err, RegisterError::NoFreeId(_)), "{err:?}");
        assert_eq!(f.count("SELECT COUNT(*) FROM project").await, i64::from(CANDIDATES));
    }

    #[tokio::test]
    async fn a_second_clone_joins_the_project_of_the_first() {
        let f = Fixture::new().await;
        let (work, review) = (Path::new("/work/app"), Path::new("/review/app"));
        let ours = f.memory(&derive_project_id(work), "tests run with nextest", None, 1).await;
        let theirs = f.memory(&derive_project_id(review), "the socket is clawg.sock", None, 2).await;
        let first = f.register(cloned(work, "github.com/acme/app")).await.unwrap();

        let second = f.register(cloned(review, "github.com/acme/app")).await.unwrap();

        assert_eq!(second.project_id, first.project_id);
        assert_eq!(second.root_id, derive_project_id(review), "the clone keeps its own root id");
        assert_eq!(second.rekeyed, 1);
        assert!(second.snapshot.is_some());
        let p = Some(first.project_id.clone());
        assert_eq!(project_ids(&f.store).await, vec![("memory".into(), ours, p.clone()), ("memory".into(), theirs, p)]);
        assert_eq!(f.count("SELECT COUNT(*) FROM memory_move WHERE row_id = 2").await, 1);
        assert_eq!(f.count("SELECT COUNT(*) FROM project").await, 2);
    }

    #[tokio::test]
    async fn a_clone_joins_with_one_live_row_per_text() {
        let f = Fixture::new().await;
        let (work, review) = (Path::new("/work/app"), Path::new("/review/app"));
        let older = f.memory(&derive_project_id(work), "the socket is clawg.sock", None, 1).await;
        let newer = f.memory(&derive_project_id(review), "the socket is clawg.sock", None, 2).await;
        f.register(cloned(work, "github.com/acme/app")).await.unwrap();

        f.register(cloned(review, "github.com/acme/app")).await.unwrap();

        let live = f.count("SELECT COUNT(*) FROM memory WHERE status = 'active'").await;
        assert_eq!(live, 1);
        let by: i64 = f
            .store
            .conn
            .call(move |c| c.query_row("SELECT superseded_by FROM memory WHERE id = ?1", [newer], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(by, older);
    }

    #[tokio::test]
    async fn a_new_remote_keeps_the_project_of_its_root() {
        let f = Fixture::new().await;
        let root = Path::new("/work/app");
        let first = f.register(cloned(root, "github.com/acme/app")).await.unwrap();

        let moved = f.register(cloned(root, "gitlab.com/acme/app")).await.unwrap();

        assert_eq!((moved.project_id, moved.rekeyed), (first.project_id, 0));
    }

    #[tokio::test]
    async fn two_remotes_are_two_projects() {
        let f = Fixture::new().await;
        let ours = f.register(cloned(Path::new("/work/app"), "github.com/acme/app")).await.unwrap();
        let fork = f.register(cloned(Path::new("/fork/app"), "github.com/me/app")).await.unwrap();

        assert_ne!(ours.project_id, fork.project_id);
    }

    #[tokio::test]
    async fn a_caller_that_derives_its_id_keeps_the_conflict() {
        let f = Fixture::seeded().await;
        let root = Path::new("/work/repo");
        let id = derive_project_id(root);
        f.root(&id, "/elsewhere/repo").await;
        let before = project_ids(&f.store).await;
        let legacy = ["src-1a2b".to_string()];

        let err = f.register(Registration { assigned: false, ..reg(root, &legacy) }).await.unwrap_err();

        assert!(matches!(err, RegisterError::RootCollision { ref existing, .. } if existing == "/elsewhere/repo"), "{err:?}");
        assert_eq!(project_ids(&f.store).await, before);
        assert_eq!(index_docs_of(&f.store, &id).await, vec![0], "the other root's mark is left alone");
        assert_eq!(f.count("SELECT COUNT(*) FROM project_rekey").await, 0);

        // Given another id since, the root is still refused to such a caller.
        let given = f.register(reg(root, &[])).await.unwrap();
        assert_ne!(given.root_id, id);
        let err = f.register(Registration { assigned: false, ..reg(root, &[]) }).await.unwrap_err();
        assert!(matches!(err, RegisterError::RootCollision { .. }), "{err:?}");
    }

    #[tokio::test]
    async fn a_caller_that_derives_its_id_joins_no_remote() {
        let f = Fixture::new().await;
        let first = f.register(cloned(Path::new("/work/app"), "github.com/acme/app")).await.unwrap();
        let review = Path::new("/review/app");

        let old = f.register(Registration { assigned: false, ..cloned(review, "github.com/acme/app") }).await.unwrap();

        assert_eq!(old.project_id, derive_project_id(review));
        assert_ne!(old.project_id, first.project_id);
    }

    #[tokio::test]
    async fn registering_to_index_raises_the_mark_and_never_lowers_it() {
        let f = Fixture::seeded().await;
        let root = Path::new("/work/repo");
        let id = derive_project_id(root);

        let first = f.register(Registration { index_docs: false, ..reg(root, &[]) }).await.unwrap();
        assert!(!first.index_docs);
        assert_eq!(index_docs_of(&f.store, &id).await, vec![0]);

        let raised = f.register(reg(root, &[])).await.unwrap();
        assert!(raised.index_docs);
        assert_eq!(index_docs_of(&f.store, &id).await, vec![1]);

        let kept = f.register(Registration { index_docs: false, ..reg(root, &[]) }).await.unwrap();
        assert!(kept.index_docs, "a later registration that does not index keeps the mark");
        assert_eq!(index_docs_of(&f.store, &id).await, vec![1]);
    }

    #[tokio::test]
    async fn a_malformed_legacy_id_is_refused_before_anything_moves() {
        let f = Fixture::seeded().await;
        let before = project_ids(&f.store).await;
        let legacy = ["src-1a2b".to_string(), "x' OR 1".to_string()];

        let err = f.register(reg(Path::new("/work/repo"), &legacy)).await.unwrap_err();

        assert!(matches!(err, RegisterError::InvalidLegacyId(_)), "{err:?}");
        assert_eq!(project_ids(&f.store).await, before);
        assert_eq!(f.count("SELECT COUNT(*) FROM project").await, 0);
    }

    #[tokio::test]
    async fn a_legacy_id_of_another_registered_project_keeps_its_rows_and_workspace() {
        let f = Fixture::seeded().await;
        f.root("src-1a2b", "/work/other").await;
        let theirs = f.ws.workspace_path("agent-a", Some("src-1a2b"));
        std::fs::create_dir_all(theirs.parent().unwrap()).unwrap();
        std::fs::write(&theirs, "notes of the other project").unwrap();
        let before = project_ids(&f.store).await;
        let legacy = ["src-1a2b".to_string()];

        let outcome = f.register(reg(Path::new("/work/repo"), &legacy)).await.unwrap();

        assert_eq!(outcome.rekeyed, 0);
        assert_eq!(project_ids(&f.store).await, before);
        assert_eq!(f.count("SELECT COUNT(*) FROM project_rekey").await, 0);
        assert!(!f.ws.workspace_path("agent-a", Some(&outcome.project_id)).exists());
    }

    #[tokio::test]
    async fn registering_carries_the_legacy_workspace_over() {
        let f = Fixture::seeded().await;
        let legacy_file = f.ws.workspace_path("agent-a", Some("src-1a2b"));
        std::fs::create_dir_all(legacy_file.parent().unwrap()).unwrap();
        std::fs::write(&legacy_file, "notes").unwrap();
        let legacy = ["src-1a2b".to_string()];

        let outcome = f.register(reg(Path::new("/work/repo"), &legacy)).await.unwrap();

        let adopted = f.ws.workspace_path("agent-a", Some(&outcome.project_id));
        assert_eq!(std::fs::read_to_string(adopted).unwrap(), "notes");
        assert!(legacy_file.exists());
    }

    #[tokio::test]
    async fn a_clone_hands_its_workspace_to_the_shared_project() {
        let f = Fixture::new().await;
        let (work, review) = (Path::new("/work/app"), Path::new("/review/app"));
        let first = f.register(cloned(work, "github.com/acme/app")).await.unwrap();
        let mine = f.ws.workspace_path("agent-a", Some(&derive_project_id(review)));
        std::fs::create_dir_all(mine.parent().unwrap()).unwrap();
        std::fs::write(&mine, "review notes").unwrap();

        f.register(cloned(review, "github.com/acme/app")).await.unwrap();

        let shared = f.ws.workspace_path("agent-a", Some(&first.project_id));
        assert_eq!(std::fs::read_to_string(shared).unwrap(), "review notes");
    }

    #[tokio::test]
    async fn the_project_of_a_root_is_read_without_writing() {
        let f = Fixture::new().await;
        let (work, review, other) = (Path::new("/work/app"), Path::new("/review/app"), Path::new("/work/other"));
        let first = f.register(cloned(work, "github.com/acme/app")).await.unwrap();
        f.root(&derive_project_id(other), "/somewhere/else").await;

        let ids = f
            .store
            .conn
            .call(move |c| {
                Ok::<_, rusqlite::Error>((
                    project_id_in(c, review, Some("github.com/acme/app"))?,
                    project_id_in(c, review, None)?,
                    project_id_in(c, other, None)?,
                ))
            })
            .await
            .unwrap();

        assert_eq!(ids, (first.project_id, derive_project_id(review), candidate_id(other, 1)));
        assert_eq!(f.count("SELECT COUNT(*) FROM project").await, 2, "reading writes nothing");
    }
}
