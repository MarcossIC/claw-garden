//! The subfolders of a repository declared as projects of their own (V30).
//!
//! A declaration says that, in the project of a repository, what is worked on
//! inside a subpath goes to another project. It keys on that project and not
//! on a remote, so whatever already leads to the repository's project (a
//! clone, a moved checkout without `origin`, a worktree, the same store on
//! another machine) leads to its subprojects too. A merge rewrites no
//! declaration: both of its ids are read through [`canonical_in`].

use rusqlite::Connection;

use crate::memory::merge::canonical_in;

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
}
