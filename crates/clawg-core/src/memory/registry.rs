//! The projects of the store. `project` is the reference every table keys
//! its rows by, whether a folder of this machine leads to it
//! (`project_path`), a remote does (`project_remote`), or nothing does.
//!
//! A row of `memory` written under an id `project` does not know adopts it in
//! the same write, through the two triggers of V27, which name the project as
//! [`project_name`] does.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::memory::identity::Ground;
use crate::memory::project::is_project_id;
use crate::memory::{MemoryError, MemoryStore};

/// The name a project has until someone gives it one: its id without the
/// `-` and four hex digits `derive_project_id` appends, or the whole id when
/// it does not have that shape. V27's backfill and triggers say the same in
/// SQL.
pub(crate) fn project_name(id: &str) -> &str {
    match id.rsplit_once('-') {
        Some((name, _)) if is_project_id(id) => name,
        _ => id,
    }
}

/// The longest name a project may be given, in characters.
pub const MAX_NAME_CHARS: usize = 80;

/// A project as `project` holds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    /// How it started: `migrated`, `root`, `adopted` or `import`.
    pub born: String,
    /// Whether the user has looked at it: named it, even with the name it had.
    pub reviewed: bool,
    pub created_at: i64,
}

fn project_from(r: &rusqlite::Row<'_>) -> rusqlite::Result<Project> {
    Ok(Project { id: r.get(0)?, name: r.get(1)?, born: r.get(2)?, reviewed: r.get(3)?, created_at: r.get(4)? })
}

/// A folder of this machine that leads to a project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListedFolder {
    pub root_id: String,
    pub root_path: String,
    pub remote: Option<String>,
    pub index_docs: bool,
    /// Whether the checkout recorded there is still there.
    pub present: bool,
}

/// A suggestion nobody answered yet that a project may be the same as another.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListedSuggestion {
    pub other_id: String,
    /// The other's name, when it is still a project.
    pub other_name: Option<String>,
    pub reason: String,
    pub created_at: i64,
}

/// A project with what leads to it and what it holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListedProject {
    #[serde(flatten)]
    pub project: Project,
    pub folders: Vec<ListedFolder>,
    /// The remotes whose clones share it.
    pub remotes: Vec<String>,
    /// Its rows of `memory` that are not deleted.
    pub memories: i64,
    pub suggestions: Vec<ListedSuggestion>,
}

/// A folder as `project_path` records it.
struct StoredFolder {
    id: String,
    root_path: String,
    project_id: Option<String>,
    remote: Option<String>,
    index_docs: bool,
    git_stamp: Option<String>,
}

/// What [`list_projects`] reads of the store.
struct Stored {
    projects: Vec<Project>,
    /// Oldest first.
    folders: Vec<StoredFolder>,
    /// Remote and project, by remote.
    remotes: Vec<(String, String)>,
    memories: HashMap<String, i64>,
    /// Project, other, reason and date of each suggestion not answered, oldest first.
    suggestions: Vec<(String, String, String, i64)>,
}

fn stored_in(c: &Connection) -> rusqlite::Result<Stored> {
    let mut stmt = c.prepare("SELECT id, name, born, reviewed, created_at FROM project")?;
    let projects = stmt.query_map([], project_from)?.collect::<rusqlite::Result<_>>()?;
    let mut stmt =
        c.prepare("SELECT id, root_path, project_id, remote, index_docs, git_stamp FROM project_path ORDER BY created_at, id")?;
    let folders = stmt
        .query_map([], |r| {
            Ok(StoredFolder {
                id: r.get(0)?,
                root_path: r.get(1)?,
                project_id: r.get(2)?,
                remote: r.get(3)?,
                index_docs: r.get(4)?,
                git_stamp: r.get(5)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    let mut stmt = c.prepare("SELECT remote, project_id FROM project_remote ORDER BY remote")?;
    let remotes = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
    let mut stmt = c.prepare(
        "SELECT project_id, COUNT(*) FROM memory WHERE deleted_at IS NULL AND project_id IS NOT NULL GROUP BY project_id",
    )?;
    let memories = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
    let mut stmt = c.prepare(
        "SELECT project_id, other_id, reason, created_at FROM project_suggestion WHERE answer IS NULL ORDER BY created_at, rowid",
    )?;
    let suggestions = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<rusqlite::Result<_>>()?;
    Ok(Stored { projects, folders, remotes, memories, suggestions })
}

/// What the disk says of a folder: whether its checkout is still there, and
/// the key of its remote when no registration recorded its project.
struct Found {
    present: bool,
    key: Option<String>,
}

/// Every project, with its folders, its remotes, how many memories it holds
/// and the suggestions about it nobody answered, by name. Whether each
/// recorded checkout is still there, and the remote of a folder no
/// registration recorded a project in, are read from `ground` on a blocking
/// thread.
pub async fn list_projects(store: &MemoryStore, ground: Arc<dyn Ground>) -> Result<Vec<ListedProject>, MemoryError> {
    let stored = store.conn.call(|c| stored_in(c)).await.map_err(MemoryError::Connection)?;
    let asked: Vec<(PathBuf, Option<String>, bool)> = stored
        .folders
        .iter()
        .map(|f| (PathBuf::from(&f.root_path), f.git_stamp.clone(), f.project_id.is_none()))
        .collect();
    let found = tokio::task::spawn_blocking(move || {
        asked
            .into_iter()
            .map(|(path, stamp, keyless)| Found {
                present: ground.recorded_checkout_present(&path, stamp.as_deref()),
                key: if keyless { ground.checkout(&path).key } else { None },
            })
            .collect::<Vec<_>>()
    })
    .await
    .map_err(|e| MemoryError::Internal(format!("could not look at the project folders: {e}")))?;
    Ok(listed(&stored, &found))
}

/// The listing of `stored`, with `found` for each of its folders in order.
fn listed(stored: &Stored, found: &[Found]) -> Vec<ListedProject> {
    let mut listed: Vec<ListedProject> = stored
        .projects
        .iter()
        .map(|p| ListedProject {
            project: p.clone(),
            folders: Vec::new(),
            remotes: Vec::new(),
            memories: stored.memories.get(&p.id).copied().unwrap_or(0),
            suggestions: Vec::new(),
        })
        .collect();
    let at: HashMap<&str, usize> = stored.projects.iter().enumerate().map(|(i, p)| (p.id.as_str(), i)).collect();
    let bound: HashMap<&str, &str> = stored.remotes.iter().map(|(r, p)| (r.as_str(), p.as_str())).collect();
    for (folder, found) in stored.folders.iter().zip(found) {
        let remote = folder.remote.as_deref().or(found.key.as_deref());
        // The one a registration recorded, else the one the clones of its
        // remote share, else its own id. A folder that leads to no project
        // (nothing was saved in it before V27) waits for a registration to
        // make it one.
        let project = folder.project_id.as_deref().or_else(|| remote.and_then(|r| bound.get(r).copied())).unwrap_or(&folder.id);
        if let Some(&i) = at.get(project) {
            listed[i].folders.push(ListedFolder {
                root_id: folder.id.clone(),
                root_path: folder.root_path.clone(),
                remote: remote.map(str::to_string),
                index_docs: folder.index_docs,
                present: found.present,
            });
        }
    }
    for (remote, project) in &stored.remotes {
        if let Some(&i) = at.get(project.as_str()) {
            listed[i].remotes.push(remote.clone());
        }
    }
    for (a, b, reason, created_at) in &stored.suggestions {
        for (this, other) in [(a, b), (b, a)] {
            if let Some(&i) = at.get(this.as_str()) {
                let other_name = at.get(other.as_str()).map(|&o| stored.projects[o].name.clone());
                listed[i].suggestions.push(ListedSuggestion {
                    other_id: other.clone(),
                    other_name,
                    reason: reason.clone(),
                    created_at: *created_at,
                });
            }
        }
    }
    listed.sort_by_cached_key(|l| (l.project.name.to_lowercase(), l.project.id.clone()));
    listed
}

/// Why a name may not be a project's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum NameError {
    #[error("a project name must not be empty")]
    Empty,
    #[error("a project name has at most {MAX_NAME_CHARS} characters")]
    TooLong,
    #[error("a project name must not hold control characters")]
    Control,
}

#[derive(Debug, thiserror::Error)]
pub enum RenameError {
    #[error("no project has the id {0}")]
    NotFound(String),
    #[error(transparent)]
    Name(#[from] NameError),
    #[error("{name:?} is already the name or the id of project {other}")]
    Taken { name: String, other: String },
    #[error(transparent)]
    Store(#[from] MemoryError),
}

/// `name` without the spaces around it, when it may name a project.
fn checked_name(name: &str) -> Result<&str, NameError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(NameError::Empty);
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(NameError::TooLong);
    }
    if name.chars().any(char::is_control) {
        return Err(NameError::Control);
    }
    Ok(name)
}

/// Gives project `id` the name `name`, without the spaces around it, and
/// marks it reviewed, even when the name is the one it had. Nothing else
/// changes, its id least of all, so what was saved under it stays there.
/// Two projects may share a name a registration gave them, but not one the
/// user chose: the name or the id of another project, in any case, is
/// refused.
pub async fn rename_project(store: &MemoryStore, id: &str, name: &str) -> Result<Project, RenameError> {
    let (id, name) = (id.to_string(), checked_name(name)?.to_string());
    store
        .conn
        .call(move |c| {
            let sql = |e: rusqlite::Error| RenameError::Store(MemoryError::Sqlite(e));
            let tx = c.transaction().map_err(sql)?;
            let project = tx
                .query_row("SELECT id, name, born, reviewed, created_at FROM project WHERE id = ?1", [&id], project_from)
                .optional()
                .map_err(sql)?
                .ok_or_else(|| RenameError::NotFound(id.clone()))?;
            let others: Vec<(String, String)> = {
                let mut stmt = tx.prepare("SELECT id, name FROM project WHERE id <> ?1").map_err(sql)?;
                let rows = stmt.query_map([&id], |r| Ok((r.get(0)?, r.get(1)?))).map_err(sql)?;
                rows.collect::<rusqlite::Result<_>>().map_err(sql)?
            };
            // In Rust, since SQLite's lower() folds ASCII alone.
            let wanted = name.to_lowercase();
            if let Some((other, _)) = others.into_iter().find(|(o, n)| o.to_lowercase() == wanted || n.to_lowercase() == wanted) {
                return Err(RenameError::Taken { name, other });
            }
            tx.execute("UPDATE project SET name = ?2, reviewed = 1 WHERE id = ?1", rusqlite::params![id, name]).map_err(sql)?;
            tx.commit().map_err(sql)?;
            Ok(Project { name, reviewed: true, ..project })
        })
        .await
        .map_err(|e| match e {
            tokio_rusqlite::Error::Error(e) => e,
            other => RenameError::Store(MemoryError::Backend(other.to_string())),
        })
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;

    use super::*;
    use crate::memory::events::{MemoryEvent, MemorySource};
    use crate::memory::identity::{FakeGround, RootCommit};
    use crate::memory::relocate::{move_memories_core, undo_move_core, MoveFilter};
    use crate::memory::worker::save_for_test;
    use crate::memory::MemoryStore;
    use crate::models::chat::Role;

    /// `(name, born, reviewed, created_at)` of project `id`.
    type Project = (String, String, i64, i64);

    async fn project(store: &MemoryStore, id: &str) -> Option<Project> {
        use rusqlite::OptionalExtension;
        let id = id.to_string();
        store
            .conn
            .call(move |c| {
                c.query_row("SELECT name, born, reviewed, created_at FROM project WHERE id = ?1", [id], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                })
                .optional()
            })
            .await
            .unwrap()
    }

    async fn row(store: &MemoryStore, id: i64) -> (Option<String>, i64) {
        store
            .conn
            .call(move |c| c.query_row("SELECT project_id, created_at FROM memory WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?))))
            .await
            .unwrap()
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

    async fn save(store: &Arc<MemoryStore>, evt: MemoryEvent) -> i64 {
        let report = save_for_test(store, &evt).await;
        assert_eq!(report.failure, None, "a row under an unknown project is not refused");
        assert_eq!(report.memories.len(), 1);
        report.memories[0].id
    }

    #[tokio::test]
    async fn a_row_saved_under_an_unknown_project_adopts_it() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());

        let id = save(&store, fact("a note from a bridge whose registration failed", Some("app-1a2b"))).await;

        let (project_id, created_at) = row(&store, id).await;
        assert_eq!(project_id.as_deref(), Some("app-1a2b"));
        assert_eq!(project(&store, "app-1a2b").await, Some(("app".into(), "adopted".into(), 0, created_at)));
    }

    #[tokio::test]
    async fn a_row_moved_to_an_unknown_project_adopts_it() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        save(&store, fact("a note that makes the target a project", Some("target-2b3c"))).await;
        let id = save(&store, fact("a note of an id from before projects", Some("old-1a2b"))).await;
        let backups = tempfile::tempdir().unwrap();
        let filter = MoveFilter { ids: vec![id], ..MoveFilter::default() };
        let moved = move_memories_core(&store, backups.path(), &filter, Some("target-2b3c".into()), false).await.unwrap();
        // As an id a store from before V27 only names in its history.
        store.conn.call(|c| c.execute("DELETE FROM project WHERE id = 'old-1a2b'", [])).await.unwrap();

        undo_move_core(&store, backups.path(), moved.move_id.expect("a move"), false).await.unwrap();

        let (project_id, created_at) = row(&store, id).await;
        assert_eq!(project_id.as_deref(), Some("old-1a2b"));
        assert_eq!(project(&store, "old-1a2b").await, Some(("old".into(), "adopted".into(), 0, created_at)));
    }

    #[tokio::test]
    async fn the_name_of_an_adopted_id_is_the_one_rust_gives() {
        assert_eq!(project_name("claw-garden-00ff"), "claw-garden");
        assert_eq!(project_name("release-2024"), "release");
        assert_eq!(project_name("repo-1A2B"), "repo-1A2B");
        assert_eq!(project_name("p"), "p");
        let ids = [
            "repo-1a2b", "claw-garden-00ff", "release-2024", "a--1a2b", "ñandú-0f0f", "repo", "p", "-1a2b", "repo-1A2B",
            "repo-1a2", "repo-1a2bc", "repo-zzzz", "repo 1a2b", "repo-1a2b-",
        ];
        let store = MemoryStore::new_for_test().await.unwrap();
        let named: Vec<(String, String)> = store
            .conn
            .call(move |c| {
                // Every other id arrives by a change of project, the other trigger.
                for (n, id) in ids.iter().enumerate() {
                    let inserted = if n % 2 == 0 { Some(*id) } else { None };
                    c.execute(
                        "INSERT INTO memory \
                         (scope, agent_id, project_id, content, memory_type, source, \
                          created_at, created_by_type, created_by_ref, \
                          last_modified_at, last_accessed_at, importance, tier, status) \
                         VALUES ('agent', 'a', ?1, 'note ' || ?2, 'semantic_fact', 'explicit', 1, 'agent', 'a', \
                          1, 1, 5.0, 3, 'active')",
                        rusqlite::params![inserted, n],
                    )?;
                    if inserted.is_none() {
                        c.execute("UPDATE memory SET project_id = ?1 WHERE id = ?2", rusqlite::params![id, c.last_insert_rowid()])?;
                    }
                }
                let mut stmt = c.prepare("SELECT id, name FROM project")?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
                rows.collect::<rusqlite::Result<_>>()
            })
            .await
            .unwrap();

        assert_eq!(named.len(), ids.len());
        for (id, name) in named {
            assert_eq!(name, project_name(&id), "{id}");
        }
    }

    /// Inserts each `(id, name, reviewed)` into `project`.
    async fn projects(store: &MemoryStore, rows: &'static [(&'static str, &'static str, i64)]) {
        store
            .conn
            .call(move |c| {
                for (id, name, reviewed) in rows {
                    c.execute(
                        "INSERT INTO project (id, name, born, reviewed, created_at) VALUES (?1, ?2, 'root', ?3, 7)",
                        rusqlite::params![id, name, reviewed],
                    )?;
                }
                Ok::<_, rusqlite::Error>(())
            })
            .await
            .unwrap();
    }

    async fn changes(store: &MemoryStore) -> i64 {
        store.conn.call(|c| c.query_row("SELECT total_changes()", [], |r| r.get(0))).await.unwrap()
    }

    #[tokio::test]
    async fn renaming_keeps_the_id_and_marks_the_project_reviewed() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        let id = save(&store, fact("a note of the web app", Some("app-1a2b"))).await;
        let (_, created_at) = row(&store, id).await;
        let before = changes(&store).await;

        let renamed = rename_project(&store, "app-1a2b", "App web").await.unwrap();

        assert_eq!(changes(&store).await - before, 1, "only the project's row changes");
        assert_eq!((renamed.id.as_str(), renamed.name.as_str(), renamed.reviewed), ("app-1a2b", "App web", true));
        assert_eq!(project(&store, "app-1a2b").await, Some(("App web".into(), "adopted".into(), 1, created_at)));
        assert_eq!(row(&store, id).await.0.as_deref(), Some("app-1a2b"), "what was saved under it stays");

        projects(&store, &[("web-3c4d", "web", 0)]).await;
        rename_project(&store, "web-3c4d", "web").await.expect("the same name only marks it reviewed");
        assert_eq!(project(&store, "web-3c4d").await, Some(("web".into(), "root".into(), 1, 7)));
    }

    #[tokio::test]
    async fn a_name_another_project_holds_is_refused() {
        let store = MemoryStore::new_for_test().await.unwrap();
        projects(&store, &[("api-1a2b", "api", 0), ("web-3c4d", "web", 0), ("ñandú-0f0f", "Ñandú", 0)]).await;

        for (id, name) in [("web-3c4d", "API"), ("web-3c4d", "Api-1A2B"), ("api-1a2b", "ÑANDÚ")] {
            let err = rename_project(&store, id, name).await.unwrap_err();
            assert!(matches!(&err, RenameError::Taken { .. }), "{name}: {err}");
        }
        assert_eq!(project(&store, "web-3c4d").await, Some(("web".into(), "root".into(), 0, 7)), "a refusal changes nothing");
        rename_project(&store, "api-1a2b", "Api").await.expect("its own name in another case");
        rename_project(&store, "api-1a2b", "API-1A2B").await.expect("its own id");
        let err = rename_project(&store, "nobody-0000", "anything").await.unwrap_err();
        assert!(matches!(err, RenameError::NotFound(ref id) if id == "nobody-0000"), "{err}");
    }

    #[tokio::test]
    async fn a_name_is_trimmed_and_bounded() {
        let store = MemoryStore::new_for_test().await.unwrap();
        projects(&store, &[("app-1a2b", "app", 0)]).await;

        assert_eq!(rename_project(&store, "app-1a2b", "  App web\t ").await.unwrap().name, "App web");
        let longest = "ñ".repeat(MAX_NAME_CHARS);
        assert_eq!(rename_project(&store, "app-1a2b", &longest).await.unwrap().name, longest, "counted in characters");
        let too_long = "x".repeat(MAX_NAME_CHARS + 1);
        let refused = [("", "empty"), ("   ", "blank"), ("a\nb", "a line break"), ("a\u{7f}b", "a control"), (too_long.as_str(), "too long")];
        for (name, why) in refused {
            let err = rename_project(&store, "app-1a2b", name).await.unwrap_err();
            assert!(matches!(err, RenameError::Name(_)), "{why}: {err}");
        }
        assert_eq!(project(&store, "app-1a2b").await.unwrap().0, longest, "a refused name changes nothing");
    }

    #[tokio::test]
    async fn a_folder_no_registration_recorded_is_listed_in_the_project_of_its_remote() {
        let store = MemoryStore::new_for_test().await.unwrap();
        store
            .conn
            .call(|c| {
                c.execute_batch(
                    "INSERT INTO project (id, name, born, reviewed, created_at) VALUES ('app-1a2b', 'app', 'migrated', 1, 1);
                     INSERT INTO project_path (id, root_path, created_at, project_id, remote) VALUES ('app-1a2b', '/work/app', 1, 'app-1a2b', 'github.com/acme/app');
                     INSERT INTO project_remote (remote, project_id, created_at) VALUES ('github.com/acme/app', 'app-1a2b', 1);
                     INSERT INTO project_path (id, root_path, created_at) VALUES ('clone-9f9f', '/work/clone', 2);",
                )
            })
            .await
            .unwrap();
        let ground = Arc::new(FakeGround::default());
        ground.put(Path::new("/work/clone"), Some("github.com/acme/app"), Some("s1"), RootCommit::Later);

        let listed = list_projects(&store, ground).await.unwrap();

        assert_eq!(listed.len(), 1, "a folder's id alone is no project: {listed:?}");
        let folders: Vec<(&str, Option<&str>, bool)> =
            listed[0].folders.iter().map(|f| (f.root_path.as_str(), f.remote.as_deref(), f.present)).collect();
        assert_eq!(
            folders,
            vec![("/work/app", Some("github.com/acme/app"), false), ("/work/clone", Some("github.com/acme/app"), true)],
            "the clone's folder goes by the remote its checkout has"
        );
        assert_eq!(listed[0].remotes, vec!["github.com/acme/app".to_string()]);
    }
}
