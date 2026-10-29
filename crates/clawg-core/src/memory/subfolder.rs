//! The subfolders of a repository declared as projects of their own (V30).
//!
//! A declaration says that, in the project of a repository, what is worked on
//! inside a subpath goes to another project. It keys on that project and not
//! on a remote, so whatever already leads to the repository's project (a
//! clone, a moved checkout without `origin`, a worktree, the same store on
//! another machine) leads to its subprojects too. A merge rewrites no
//! declaration: both of its ids are read through [`canonical_in`].

use std::path::Path;

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::memory::clock::TimestampMs;
use crate::memory::merge::{canonical_in, merge_core, MergeError, MergeReport};
use crate::memory::project_delete::{deleted_in, deleted_message};
use crate::memory::workspace::WorkspaceManager;
use crate::memory::{MemoryError, MemoryStore};

/// A row of `project_subfolder`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    /// The project of the repository, or a tombstone that leads to it.
    pub parent_id: String,
    /// Relative to the top of the checkout, its components joined by `/`.
    pub subpath: String,
    /// Where what is worked on inside it goes, read through merges.
    pub project_id: String,
    pub created_at: i64,
}

/// Whether the subpath `outer` holds `inner`, by whole components:
/// `apps/web` holds `apps/web/src` but not `apps/webapp`.
pub fn contains(outer: &str, inner: &str) -> bool {
    inner == outer || inner.strip_prefix(outer).is_some_and(|rest| rest.starts_with('/'))
}

/// The declaration that decides `subpath` among `declared`, read under the
/// repository's project `parent` and its tombstones: the deepest that holds
/// it, then the one of `parent` before a tombstone's, then the oldest.
pub fn deepest<'a>(declared: &'a [Declaration], parent: &str, subpath: &str) -> Option<&'a Declaration> {
    declared
        .iter()
        .filter(|d| !d.subpath.is_empty() && contains(&d.subpath, subpath))
        .min_by(|a, b| {
            let depth = |d: &Declaration| std::cmp::Reverse(d.subpath.split('/').count());
            depth(a)
                .cmp(&depth(b))
                .then_with(|| (a.parent_id != parent).cmp(&(b.parent_id != parent)))
                .then_with(|| a.created_at.cmp(&b.created_at))
                .then_with(|| a.parent_id.cmp(&b.parent_id))
        })
}

/// The declarations of `parent` and of the tombstones that lead to it, which
/// are one hop away since a merge repoints the tombstones of its source.
pub(crate) fn declarations_in(c: &Connection, parent: &str) -> rusqlite::Result<Vec<Declaration>> {
    let mut stmt = c.prepare_cached(
        "SELECT parent_id, subpath, project_id, created_at FROM project_subfolder \
         WHERE parent_id = ?1 OR parent_id IN (SELECT id FROM project WHERE merged_into = ?1) \
         ORDER BY parent_id, subpath",
    )?;
    let rows = stmt.query_map([parent], |r| {
        Ok(Declaration { parent_id: r.get(0)?, subpath: r.get(1)?, project_id: r.get(2)?, created_at: r.get(3)? })
    })?;
    rows.collect()
}

/// The project a path at `subpath` of a checkout of `parent` goes to when a
/// declaration decides it, through merges; `None` without one, or when the
/// one that decides leads back to `parent`. Declarations of the subproject
/// are not read, so they never chain.
pub(crate) fn declared_in(c: &Connection, parent: &str, subpath: &str) -> rusqlite::Result<Option<String>> {
    if subpath.is_empty() {
        return Ok(None);
    }
    let declared = declarations_in(c, parent)?;
    let Some(chosen) = deepest(&declared, parent, subpath) else {
        return Ok(None);
    };
    let project = canonical_in(c, &chosen.project_id)?;
    Ok(Some(project).filter(|p| p != parent))
}

/// A subfolder declared as a project, by the repository it is in.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SubfolderOf {
    /// The live project of the repository.
    pub repository_id: String,
    pub subpath: String,
}

/// The subfolders whose declaration leads to `project`, through merges on
/// both sides, by repository and subpath.
pub(crate) fn leading_to_in(c: &Connection, project: &str) -> rusqlite::Result<Vec<SubfolderOf>> {
    let mut stmt = c.prepare(
        "SELECT parent_id, subpath FROM project_subfolder \
         WHERE project_id = ?1 OR project_id IN (SELECT id FROM project WHERE merged_into = ?1)",
    )?;
    let rows = stmt.query_map([project], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    let mut out = Vec::new();
    for row in rows {
        let (parent, subpath) = row?;
        out.push(SubfolderOf { repository_id: canonical_in(c, &parent)?, subpath });
    }
    out.sort();
    out.dedup();
    Ok(out)
}

/// What [`declare_subfolder`] did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Declared {
    /// The project of the repository the subfolder is in.
    pub repository_id: String,
    pub subpath: String,
    /// Where the subfolder leads now.
    pub project_id: String,
    /// It led there already, and nothing was written.
    pub already: bool,
    /// The project named was the repository's: the declaration went.
    pub removed: bool,
    /// Where the subfolder led before a removal, which keeps what was saved there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<String>,
    /// The merge of the project the subfolder led to before, which only it led to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge: Option<MergeReport>,
}

/// Why a declaration was refused. Nothing is written.
#[derive(Debug, thiserror::Error)]
pub enum SubfolderError {
    #[error("no project {0}")]
    UnknownProject(String),
    #[error("project {id} was merged into {into}")]
    Merged { id: String, into: String },
    #[error("{}", deleted_message(.id, .delete_id))]
    Deleted { id: String, delete_id: Option<i64> },
    /// The subfolder leads to `other`, which something else leads to as well.
    #[error(
        "{subpath} leads to project {other}, which other folders or subfolders lead to as well; \
         `clawg projects merge {other} --into {target}` merges it whole"
    )]
    Shared { subpath: String, other: String, target: String },
    #[error(transparent)]
    Merge(#[from] MergeError),
    #[error(transparent)]
    Store(#[from] MemoryError),
}

impl From<rusqlite::Error> for SubfolderError {
    fn from(e: rusqlite::Error) -> Self {
        SubfolderError::Store(MemoryError::Sqlite(e))
    }
}

/// What a declaration of `subpath` would do, decided on the connection.
enum Step {
    Done(Declared),
    /// `other` holds nothing but this subfolder: it is merged into the target.
    Merge { other: String },
}

/// Makes `subpath` of the repository whose project is `parent` lead to
/// `target`: naming `parent` removes its declaration, naming the project it
/// leads to already writes nothing, and a project it led to that nothing else
/// reaches (no folder, no remote, no other declaration) is merged into
/// `target` as `projects merge` does. Moves no row: what was saved from the
/// subfolder before stays in the project it went to.
pub async fn declare_subfolder(
    store: &MemoryStore,
    workspaces: &WorkspaceManager,
    backups: &Path,
    parent: &str,
    subpath: &str,
    target: &str,
) -> Result<Declared, SubfolderError> {
    let (parent, subpath, target) = (parent.to_string(), subpath.to_string(), target.to_string());
    let decide = {
        let (parent, subpath, target) = (parent.clone(), subpath.clone(), target.clone());
        move |c: &mut Connection| -> Result<Step, SubfolderError> {
            let tx = c.transaction()?;
            let step = declare_in(&tx, &parent, &subpath, &target, TimestampMs::now())?;
            tx.commit()?;
            Ok(step)
        }
    };
    let step = store.conn.call(decide).await.map_err(|e| match e {
        tokio_rusqlite::Error::Error(e) => e,
        other => SubfolderError::Store(MemoryError::Backend(other.to_string())),
    })?;
    match step {
        Step::Done(declared) => Ok(declared),
        Step::Merge { other } => {
            let merge = merge_core(store, workspaces, backups, &other, &target, false).await?;
            Ok(Declared {
                repository_id: parent,
                subpath,
                project_id: target,
                already: false,
                removed: false,
                previous: None,
                merge: Some(merge),
            })
        }
    }
}

fn declare_in(c: &Connection, parent: &str, subpath: &str, target: &str, now: TimestampMs) -> Result<Step, SubfolderError> {
    let into: Option<String> = c
        .query_row("SELECT merged_into FROM project WHERE id = ?1", [target], |r| r.get(0))
        .optional()?
        .ok_or_else(|| SubfolderError::UnknownProject(target.to_string()))?;
    if into.is_some() {
        return Err(SubfolderError::Merged { id: target.to_string(), into: canonical_in(c, target)? });
    }
    if let Some(deleted) = deleted_in(c, target)? {
        return Err(SubfolderError::Deleted { id: target.to_string(), delete_id: deleted.delete_id });
    }
    let done = |already: bool, previous: Option<String>| {
        Step::Done(Declared {
            repository_id: parent.to_string(),
            subpath: subpath.to_string(),
            project_id: target.to_string(),
            already,
            removed: previous.is_some(),
            previous,
            merge: None,
        })
    };
    let mut exact = declarations_in(c, parent)?;
    exact.retain(|d| d.subpath == subpath);
    let leads = match deepest(&exact, parent, subpath) {
        Some(d) => Some(canonical_in(c, &d.project_id)?),
        None => None,
    };
    if target == parent {
        let removed = c.execute(
            "DELETE FROM project_subfolder WHERE subpath = ?2 \
             AND (parent_id = ?1 OR parent_id IN (SELECT id FROM project WHERE merged_into = ?1))",
            rusqlite::params![parent, subpath],
        )?;
        return Ok(if removed == 0 { done(true, None) } else { done(false, leads) });
    }
    match leads {
        Some(other) if other == target => return Ok(done(true, None)),
        Some(other) if other != parent => {
            return if reached_otherwise_in(c, &other, parent, subpath)? {
                Err(SubfolderError::Shared { subpath: subpath.to_string(), other, target: target.to_string() })
            } else {
                Ok(Step::Merge { other })
            };
        }
        _ => {}
    }
    c.execute(
        "INSERT INTO project_subfolder (parent_id, subpath, project_id, created_at) VALUES (?1, ?2, ?3, ?4) \
         ON CONFLICT (parent_id, subpath) DO UPDATE SET project_id = excluded.project_id, created_at = excluded.created_at",
        rusqlite::params![parent, subpath, target, now],
    )?;
    Ok(done(false, None))
}

/// Whether anything but the declarations of `subpath` under `parent` leads
/// to `project`: a folder, a remote, or another declaration.
fn reached_otherwise_in(c: &Connection, project: &str, parent: &str, subpath: &str) -> rusqlite::Result<bool> {
    c.query_row(
        "WITH ids AS (SELECT ?1 AS id UNION SELECT id FROM project WHERE merged_into = ?1), \
              family AS (SELECT ?2 AS id UNION SELECT id FROM project WHERE merged_into = ?2) \
         SELECT EXISTS (SELECT 1 FROM project_path WHERE project_id IN ids OR (project_id IS NULL AND id IN ids)) \
             OR EXISTS (SELECT 1 FROM project_remote WHERE project_id IN ids) \
             OR EXISTS (SELECT 1 FROM project_subfolder WHERE project_id IN ids \
                         AND NOT (subpath = ?3 AND parent_id IN family))",
        rusqlite::params![project, parent, subpath],
        |r| r.get(0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn declared(parent: &str, subpath: &str, project: &str, created_at: i64) -> Declaration {
        Declaration { parent_id: parent.into(), subpath: subpath.into(), project_id: project.into(), created_at }
    }

    #[test]
    fn the_deepest_declared_subfolder_wins() {
        let all = [
            declared("mono", "apps", "apps", 1),
            declared("mono", "apps/web", "web", 2),
            declared("old", "apps/web", "web-old", 0),
            declared("older", "apps/web/src", "src-older", 1),
            declared("old", "apps/web/src", "src-old", 3),
            declared("mono", "libs/ui", "ui", 4),
        ];
        let chosen = |subpath: &str| deepest(&all, "mono", subpath).map(|d| d.project_id.as_str());

        assert_eq!(chosen("apps/web"), Some("web"), "the repository's own before a tombstone's");
        assert_eq!(chosen("apps/web/lib"), Some("web"));
        assert_eq!(chosen("apps/web/src/main"), Some("src-older"), "the deepest, and the oldest of the tombstones'");
        assert_eq!(chosen("apps/webapp"), Some("apps"), "by whole components");
        assert_eq!(chosen("apps"), Some("apps"));
        assert_eq!(chosen("libs"), None, "a declaration never reaches above itself");
        assert_eq!(chosen("docs"), None);
        assert_eq!(chosen(""), None, "the top of the checkout is the repository");
        assert!(contains("apps/web", "apps/web/src") && !contains("apps/web", "apps/webapp") && !contains("apps/web", "apps"));
    }

    struct Fixture {
        store: MemoryStore,
        ws: WorkspaceManager,
        backups: std::path::PathBuf,
        _dir: crate::test_dir::TestDir,
    }

    impl Fixture {
        async fn new() -> Self {
            let store = MemoryStore::new_for_test().await.unwrap();
            let dir = crate::test_dir::test_dir();
            let ws = WorkspaceManager::new(dir.path().join("workspaces"));
            let backups = dir.path().join("backups");
            let f = Fixture { store, ws, backups, _dir: dir };
            f.sql(
                "INSERT INTO project (id, name, born, reviewed, created_at) VALUES \
                   ('mono-1a2b', 'mono', 'root', 1, 0), ('web', 'web', 'user', 1, 0), ('site', 'site', 'user', 1, 0);
                 INSERT INTO project_path (id, root_path, created_at, index_docs, project_id) \
                   VALUES ('mono-1a2b', '/work/mono', 0, 1, 'mono-1a2b');
                 INSERT INTO memory \
                  (scope, agent_id, project_id, content, memory_type, source, created_at, created_by_type, created_by_ref, \
                   last_modified_at, last_accessed_at, importance, tier, status) \
                  VALUES ('agent', 'a', 'mono-1a2b', 'saved from apps/web', 'semantic_fact', 'explicit', 1, 'agent', 'a', \
                   1, 1, 5.0, 3, 'active');",
            )
            .await;
            f
        }

        async fn sql(&self, sql: &'static str) {
            self.store.conn.call(move |c| c.execute_batch(sql)).await.unwrap();
        }

        async fn declare(&self, subpath: &str, target: &str) -> Result<Declared, SubfolderError> {
            declare_subfolder(&self.store, &self.ws, &self.backups, "mono-1a2b", subpath, target).await
        }

        async fn rows(&self) -> Vec<(String, String, String)> {
            self.store
                .conn
                .call(|c| {
                    let mut stmt = c.prepare("SELECT parent_id, subpath, project_id FROM project_subfolder ORDER BY 1, 2")?;
                    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect();
                    rows
                })
                .await
                .unwrap()
        }

        async fn leads(&self, subpath: &'static str) -> Option<String> {
            self.store.conn.call(move |c| declared_in(c, "mono-1a2b", subpath)).await.unwrap()
        }

        async fn memories(&self) -> Vec<Option<String>> {
            self.store
                .conn
                .call(|c| {
                    let mut stmt = c.prepare("SELECT project_id FROM memory ORDER BY id")?;
                    let rows = stmt.query_map([], |r| r.get(0))?.collect();
                    rows
                })
                .await
                .unwrap()
        }
    }

    fn row(subpath: &str, project: &str) -> (String, String, String) {
        ("mono-1a2b".into(), subpath.into(), project.into())
    }

    #[tokio::test]
    async fn declaring_a_subfolder_writes_one_row_and_moves_nothing() {
        let f = Fixture::new().await;

        let declared = f.declare("apps/web", "web").await.unwrap();

        assert_eq!((declared.already, declared.removed, declared.merge.is_none()), (false, false, true));
        assert_eq!((declared.repository_id.as_str(), declared.project_id.as_str()), ("mono-1a2b", "web"));
        assert_eq!(f.rows().await, vec![row("apps/web", "web")]);
        assert_eq!(f.memories().await, vec![Some("mono-1a2b".to_string())], "what was saved from it before stays");
        assert_eq!(f.leads("apps/web/src").await.as_deref(), Some("web"));
        assert!(f.declare("apps/web", "web").await.unwrap().already, "the same declaration again writes nothing");
        assert_eq!(f.rows().await, vec![row("apps/web", "web")]);
    }

    #[tokio::test]
    async fn naming_the_repository_project_removes_the_declaration() {
        let f = Fixture::new().await;
        f.declare("apps/web", "web").await.unwrap();
        f.sql(
            "INSERT INTO project (id, name, born, reviewed, created_at, merged_into, merged_at) \
               VALUES ('mono-old', 'mono', 'root', 1, 0, 'mono-1a2b', 1);
             INSERT INTO project_subfolder VALUES ('mono-old', 'apps/web', 'site', 0);",
        )
        .await;

        let removed = f.declare("apps/web", "mono-1a2b").await.unwrap();

        assert_eq!((removed.removed, removed.already), (true, false));
        assert_eq!(removed.previous.as_deref(), Some("web"), "what was saved there stays in it");
        assert!(f.rows().await.is_empty(), "the tombstone's went too");
        assert_eq!(f.leads("apps/web").await, None, "it leads to the repository again");
        assert!(f.declare("apps/web", "mono-1a2b").await.unwrap().already, "nothing left to remove");
    }

    #[tokio::test]
    async fn declaring_again_merges_a_project_only_that_subfolder_led_to() {
        let f = Fixture::new().await;
        f.declare("apps/web", "web").await.unwrap();

        let again = f.declare("apps/web", "site").await.unwrap();

        let merge = again.merge.expect("web is merged into site");
        assert_eq!((merge.source.as_str(), merge.target.as_str()), ("web", "site"));
        assert!(merge.merge_id.is_some(), "with its number, which --undo takes");
        assert_eq!(f.rows().await, vec![row("apps/web", "web")], "the row still names web");
        assert_eq!(f.leads("apps/web").await.as_deref(), Some("site"));
    }

    #[tokio::test]
    async fn a_project_reached_otherwise_is_not_merged_by_a_declaration() {
        let f = Fixture::new().await;
        f.declare("apps/web", "web").await.unwrap();
        f.declare("libs/ui", "web").await.unwrap();

        let shared = f.declare("apps/web", "site").await.unwrap_err();
        assert!(matches!(&shared, SubfolderError::Shared { other, .. } if other == "web"), "{shared:?}");
        assert!(shared.to_string().contains("clawg projects merge web --into site"), "{shared}");

        f.declare("libs/ui", "mono-1a2b").await.unwrap();
        f.sql("INSERT INTO project_path (id, root_path, created_at, index_docs, project_id) VALUES ('web-9f9f', '/work/web', 0, 0, 'web')")
            .await;
        assert!(matches!(f.declare("apps/web", "site").await, Err(SubfolderError::Shared { .. })), "a folder of its own");
        assert_eq!(f.rows().await, vec![row("apps/web", "web")]);
    }

    #[tokio::test]
    async fn a_declaration_into_an_unknown_merged_or_deleted_project_is_refused() {
        let f = Fixture::new().await;
        f.sql(
            "INSERT INTO project (id, name, born, reviewed, created_at, merged_into, merged_at) \
               VALUES ('old-site', 'old', 'user', 1, 0, 'site', 1);
             UPDATE project SET deleted_at = 5 WHERE id = 'web';",
        )
        .await;

        let merged = f.declare("apps/web", "old-site").await.unwrap_err();
        assert!(matches!(&merged, SubfolderError::Merged { into, .. } if into == "site"), "{merged:?}");
        let deleted = f.declare("apps/web", "web").await.unwrap_err();
        assert!(matches!(deleted, SubfolderError::Deleted { .. }), "{deleted:?}");
        let unknown = f.declare("apps/web", "nowhere-9f9f").await.unwrap_err();
        assert!(matches!(&unknown, SubfolderError::UnknownProject(id) if id == "nowhere-9f9f"), "{unknown:?}");
        assert!(f.rows().await.is_empty());
    }
}
