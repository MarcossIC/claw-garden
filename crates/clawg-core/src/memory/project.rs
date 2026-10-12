//! Registers a project's canonical root and moves the rows saved under its
//! legacy ids onto the id that root derives.
//!
//! Every move is recorded row by row in `project_rekey`, inside the same
//! transaction, so it can be undone exactly (the SQL is in the migration that
//! creates the table). `memory_fts` needs nothing: it is an external-content
//! table and reads `project_id` from `memory` at query time.

use std::path::Path;

use crate::memory::identity::derive_project_id;
use crate::memory::workspace::WorkspaceManager;
use crate::memory::{MemoryError, MemoryStore};

/// The tables whose rows carry a `project_id`. A literal list rather than
/// interpolated input, so no caller value ever reaches a table name.
const PROJECT_TABLES: [&str; 3] = ["memory", "episodic_trajectory", "procedural_skill"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterOutcome {
    pub project_id: String,
    /// Rows moved onto `project_id` by this call, across every table.
    pub rekeyed: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum RegisterError {
    #[error("{0:?} is not a project id")]
    InvalidLegacyId(String),
    /// Two roots hashing to one id. Moving rows into it would merge two
    /// projects, so nothing moves and the caller keeps the id it had.
    #[error("project id {id} already belongs to {existing}")]
    RootCollision { id: String, existing: String },
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

/// Records `root` as a project and moves every row of each legacy id onto the
/// id `root` derives, soft-deleted rows included, then hands each legacy
/// `workspace.md` to the project (see [`WorkspaceManager::adopt_legacy`]).
///
/// Idempotent: once a legacy id has no rows left, a repeat moves nothing.
pub async fn register_project(
    store: &MemoryStore,
    workspaces: &WorkspaceManager,
    root: &Path,
    legacy_ids: &[String],
) -> Result<RegisterOutcome, RegisterError> {
    if let Some(bad) = legacy_ids.iter().find(|id| !is_project_id(id)) {
        return Err(RegisterError::InvalidLegacyId(bad.clone()));
    }
    let project_id = derive_project_id(root);
    let root_path = root.to_string_lossy().into_owned();
    let legacy: Vec<String> = legacy_ids.iter().filter(|id| **id != project_id).cloned().collect();
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;

    let id = project_id.clone();
    let moves = legacy.clone();
    let outcome = store
        .conn
        .call(move |c| -> rusqlite::Result<Result<usize, String>> {
            let tx = c.transaction()?;
            tx.execute(
                "INSERT OR IGNORE INTO project (id, root_path, created_at) VALUES (?1, ?2, ?3)",
                rusqlite::params![id, root_path, now_ms],
            )?;
            let existing: String =
                tx.query_row("SELECT root_path FROM project WHERE id = ?1", [&id], |r| r.get(0))?;
            if existing != root_path {
                return Ok(Err(existing));
            }
            let mut moved = 0;
            for legacy_id in &moves {
                for table in PROJECT_TABLES {
                    tx.execute(
                        &format!(
                            "INSERT INTO project_rekey (tbl, row_id, legacy_id, project_id, rekeyed_at) \
                             SELECT '{table}', id, project_id, ?2, ?3 FROM {table} WHERE project_id = ?1"
                        ),
                        rusqlite::params![legacy_id, id, now_ms],
                    )?;
                    moved += tx.execute(
                        &format!("UPDATE {table} SET project_id = ?2 WHERE project_id = ?1"),
                        rusqlite::params![legacy_id, id],
                    )?;
                }
            }
            tx.commit()?;
            Ok(Ok(moved))
        })
        .await
        .map_err(|e| match e {
            tokio_rusqlite::Error::Error(re) => MemoryError::Sqlite(re),
            other => MemoryError::Backend(other.to_string()),
        })?;
    let rekeyed = outcome
        .map_err(|existing| RegisterError::RootCollision { id: project_id.clone(), existing })?;

    // After the commit and outside it: a file copy that fails must not undo
    // rows already moved, and the legacy file is still there for the next try.
    for legacy_id in &legacy {
        if let Err(e) = workspaces.adopt_legacy(legacy_id, &project_id).await {
            log::warn!("could not carry the workspace of {legacy_id} over to {project_id}: {e}");
        }
    }

    Ok(RegisterOutcome { project_id, rekeyed })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_ids_are_a_name_a_dash_and_four_hex_digits() {
        for ok in ["repo-1a2b", "claw-garden-00ff", "project-0000"] {
            assert!(is_project_id(ok), "{ok}");
        }
        for bad in ["repo", "-1a2b", "repo-1A2B", "repo-1a2", "repo-1a2bc", "repo-zzzz", "", "repo 1a2b"] {
            assert!(!is_project_id(bad), "{bad}");
        }
    }

    async fn seeded() -> (MemoryStore, tempfile::TempDir) {
        let store = MemoryStore::new_for_test().await.unwrap();
        store
            .conn
            .call(|c| {
                for (project, content, deleted) in [
                    ("src-1a2b", "zanzibar parser decision", None),
                    ("src-1a2b", "second fact", Some(5_i64)),
                    ("other-9f9f", "unrelated", None),
                ] {
                    c.execute(
                        "INSERT INTO memory \
                         (scope, agent_id, project_id, content, memory_type, source, \
                          created_at, created_by_type, created_by_ref, \
                          last_modified_at, last_accessed_at, importance, tier, status, deleted_at) \
                         VALUES ('agent', 'agent-a', ?1, ?2, 'observation', 'agent', \
                          0, 'agent', 'agent-a', 0, 0, 5.0, 3, 'active', ?3)",
                        rusqlite::params![project, content, deleted],
                    )?;
                }
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
        (store, tempfile::tempdir().unwrap())
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

    async fn count(store: &MemoryStore, sql: &'static str) -> i64 {
        store.conn.call(move |c| c.query_row(sql, [], |r| r.get(0))).await.unwrap()
    }

    #[tokio::test]
    async fn rows_of_a_legacy_id_move_to_the_resolved_root() {
        let (store, ws) = seeded().await;
        let root = Path::new("/work/repo");
        let id = derive_project_id(root);

        let outcome = register_project(&store, &WorkspaceManager::new(ws.path().into()), root, &["src-1a2b".into()])
            .await
            .unwrap();

        assert_eq!(outcome, RegisterOutcome { project_id: id.clone(), rekeyed: 4 });
        let ids = project_ids(&store).await;
        let moved = ids.iter().filter(|(_, _, p)| p.as_deref() == Some(id.as_str())).count();
        assert_eq!(moved, 4, "{ids:?}");
        assert!(ids.iter().any(|(_, _, p)| p.as_deref() == Some("other-9f9f")), "{ids:?}");
        assert_eq!(count(&store, "SELECT COUNT(*) FROM project_rekey").await, 4);
        assert_eq!(count(&store, "SELECT COUNT(*) FROM project").await, 1);
    }

    #[tokio::test]
    async fn registering_again_moves_nothing() {
        let (store, ws) = seeded().await;
        let ws = WorkspaceManager::new(ws.path().into());
        let root = Path::new("/work/repo");
        register_project(&store, &ws, root, &["src-1a2b".into()]).await.unwrap();

        let again = register_project(&store, &ws, root, &["src-1a2b".into()]).await.unwrap();

        assert_eq!(again.rekeyed, 0);
        assert_eq!(count(&store, "SELECT COUNT(*) FROM project_rekey").await, 4);
    }

    #[tokio::test]
    async fn the_rekey_log_restores_every_row_it_moved() {
        let (store, ws) = seeded().await;
        let before = project_ids(&store).await;
        register_project(&store, &WorkspaceManager::new(ws.path().into()), Path::new("/work/repo"), &["src-1a2b".into()])
            .await
            .unwrap();

        store
            .conn
            .call(|c| {
                for table in PROJECT_TABLES {
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

        assert_eq!(project_ids(&store).await, before);
    }

    #[tokio::test]
    async fn a_moved_row_is_found_lexically_under_its_new_project() {
        let (store, ws) = seeded().await;
        let outcome = register_project(&store, &WorkspaceManager::new(ws.path().into()), Path::new("/work/repo"), &["src-1a2b".into()])
            .await
            .unwrap();

        let id = outcome.project_id;
        let found: i64 = store
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
    async fn a_root_whose_id_belongs_to_another_root_moves_nothing() {
        let (store, ws) = seeded().await;
        let ws = WorkspaceManager::new(ws.path().into());
        let root = Path::new("/work/repo");
        let id = derive_project_id(root);
        let taken = id.clone();
        store
            .conn
            .call(move |c| {
                c.execute(
                    "INSERT INTO project (id, root_path, created_at) VALUES (?1, '/elsewhere/repo', 0)",
                    [taken],
                )
            })
            .await
            .unwrap();
        let before = project_ids(&store).await;

        let err = register_project(&store, &ws, root, &["src-1a2b".into()]).await.unwrap_err();

        assert!(matches!(err, RegisterError::RootCollision { ref existing, .. } if existing == "/elsewhere/repo"), "{err:?}");
        assert_eq!(project_ids(&store).await, before);
        assert_eq!(count(&store, "SELECT COUNT(*) FROM project_rekey").await, 0);
    }

    #[tokio::test]
    async fn a_malformed_legacy_id_is_refused_before_anything_moves() {
        let (store, ws) = seeded().await;
        let before = project_ids(&store).await;

        let err = register_project(&store, &WorkspaceManager::new(ws.path().into()), Path::new("/work/repo"), &["src-1a2b".into(), "x' OR 1".into()])
            .await
            .unwrap_err();

        assert!(matches!(err, RegisterError::InvalidLegacyId(_)), "{err:?}");
        assert_eq!(project_ids(&store).await, before);
        assert_eq!(count(&store, "SELECT COUNT(*) FROM project").await, 0);
    }

    #[tokio::test]
    async fn registering_carries_the_legacy_workspace_over() {
        let (store, ws_dir) = seeded().await;
        let ws = WorkspaceManager::new(ws_dir.path().into());
        let legacy = ws.workspace_path("agent-a", Some("src-1a2b"));
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        std::fs::write(&legacy, "notes").unwrap();

        let outcome = register_project(&store, &ws, Path::new("/work/repo"), &["src-1a2b".into()]).await.unwrap();

        let adopted = ws.workspace_path("agent-a", Some(&outcome.project_id));
        assert_eq!(std::fs::read_to_string(adopted).unwrap(), "notes");
        assert!(legacy.exists());
    }
}
