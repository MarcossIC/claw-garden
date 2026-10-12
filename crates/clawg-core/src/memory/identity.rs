use std::path::{Component, Path, PathBuf};
use std::hash::{Hash, Hasher};
use siphasher::sip::SipHasher13;

/// Derive a stable, human-readable project_id from a project path.
///
/// Format: `{basename}-{hash4}` where `hash4` is 4 lowercase hex characters
/// of SipHash-1-3 (zero key) over the full path bytes (OS-string representation).
/// The result is deterministic on a given path, and the basename component
/// makes it human-readable without embedding parent directory information.
pub fn derive_project_id(project_path: &Path) -> String {
    let name = basename(project_path);
    let h = hash4(project_path.as_os_str().as_encoded_bytes());
    format!("{}-{}", name, h)
}

/// A project as the store keys it: the root every path inside it resolves to,
/// the id derived from that root, and the id the same path had before paths
/// were resolved, which rows saved back then still carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectIdentity {
    pub id: String,
    pub root: PathBuf,
    pub legacy_id: String,
}

impl ProjectIdentity {
    /// Resolves `path` to its project. Reads the filesystem (see [`resolve_project_root`]).
    pub fn resolve(path: &Path) -> Self {
        let root = resolve_project_root(path);
        Self { id: derive_project_id(&root), legacy_id: derive_project_id(path), root }
    }
}

/// The root of the project `path` belongs to: the nearest ancestor of its
/// canonical form holding a `.git` (with worktrees resolved by
/// [`root_from_marker`]), or the canonical path itself outside git.
///
/// A path that does not exist cannot be canonicalized or searched for `.git`,
/// so it is only made absolute and normalized textually.
pub fn resolve_project_root(path: &Path) -> PathBuf {
    let canonical = match std::fs::canonicalize(path) {
        Ok(p) => strip_verbatim(&p),
        Err(_) => {
            let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
            return normalize_lexical(&absolute);
        }
    };
    canonical
        .ancestors()
        .find_map(|dir| read_marker(dir).map(|marker| root_from_marker(dir, &marker)))
        .unwrap_or(canonical)
}

fn read_marker(dir: &Path) -> Option<GitMarker> {
    let dot_git = dir.join(".git");
    let meta = std::fs::metadata(&dot_git).ok()?;
    if meta.is_dir() {
        return Some(GitMarker::Dir);
    }
    // A `.git` file git cannot follow still marks where the user put the
    // checkout, so it ends the search at this directory.
    let Some(gitdir) = std::fs::read_to_string(&dot_git).ok().and_then(|c| parse_gitdir(&c, dir)) else {
        return Some(GitMarker::Link { gitdir: dot_git, commondir: None });
    };
    let commondir = std::fs::read_to_string(gitdir.join("commondir")).ok();
    Some(GitMarker::Link { gitdir, commondir })
}

/// What a directory's `.git` entry says about where its project's root is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitMarker {
    /// `.git` is a directory: an ordinary checkout.
    Dir,
    /// `.git` is a file naming the real git directory, as worktrees and
    /// submodules write it. `commondir` is the content of `<gitdir>/commondir`,
    /// which only a worktree's git directory has.
    Link { gitdir: PathBuf, commondir: Option<String> },
}

/// The project root for `dir`, the directory holding a `.git` described by `marker`.
///
/// A worktree resolves to the repository it was added from, so every branch
/// checked out beside it shares one memory. That only holds when the common
/// git directory is a `.git` inside a work tree; for a bare repository there
/// is no such work tree, and the worktree stays its own root.
pub fn root_from_marker(dir: &Path, marker: &GitMarker) -> PathBuf {
    let GitMarker::Link { gitdir, commondir: Some(common) } = marker else {
        return dir.to_path_buf();
    };
    let common = normalize_lexical(&gitdir.join(common.trim()));
    match common.file_name() {
        Some(name) if name == ".git" => common.parent().map_or_else(|| dir.to_path_buf(), Path::to_path_buf),
        _ => dir.to_path_buf(),
    }
}

/// The git directory a `.git` file in `dir` points at, or `None` when the
/// content is not a `gitdir:` line. A relative target is relative to `dir`,
/// as git writes it with `worktree.useRelativePaths`.
pub fn parse_gitdir(contents: &str, dir: &Path) -> Option<PathBuf> {
    let target = contents.lines().next()?.strip_prefix("gitdir:")?.trim();
    if target.is_empty() {
        return None;
    }
    Some(normalize_lexical(&dir.join(target)))
}

/// `path` without `.` components, trailing separators or `..` that a
/// preceding name cancels. Purely textual: it never asks the filesystem
/// whether a name is a link.
pub fn normalize_lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => match out.components().next_back() {
                Some(Component::Normal(_)) => {
                    out.pop();
                }
                Some(Component::RootDir | Component::Prefix(_)) => {}
                _ => out.push(".."),
            },
            other => out.push(other),
        }
    }
    out
}

/// `path` without the `\\?\` verbatim prefix that `canonicalize` adds on
/// Windows. Ids are hashed from the path's bytes, and the working directory a
/// bridge starts in never carries that prefix.
pub fn strip_verbatim(path: &Path) -> PathBuf {
    let Some(s) = path.to_str() else {
        return path.to_path_buf();
    };
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    match s.strip_prefix(r"\\?\") {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
        _ => path.to_path_buf(),
    }
}

/// Return the 4-hex-character SipHash-1-3 fingerprint of `s`.
fn hash4(s: &[u8]) -> String {
    let mut hasher = SipHasher13::new_with_keys(0, 0);
    s.hash(&mut hasher);
    let h: u64 = hasher.finish();
    // Take the low 16 bits → 4 hex chars
    format!("{:04x}", h & 0xFFFF)
}

/// Return the file-name component of `p` as a lowercase string, falling back to
/// `"project"` if the path has no file-name or the name is not valid UTF-8.
fn basename(p: &Path) -> String {
    p.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("project")
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Same path → same id.
    #[test]
    fn same_path_same_id() {
        let path = PathBuf::from("/home/user/projects/my-app");
        assert_eq!(derive_project_id(&path), derive_project_id(&path));
    }

    /// Format matches `{basename}-[0-9a-f]{4}`.
    #[test]
    fn format_matches_basename_hash4() {
        let path = PathBuf::from("/home/user/projects/my-app");
        let id = derive_project_id(&path);
        // Must start with the basename (lowercased)
        assert!(
            id.starts_with("my-app-"),
            "Expected id to start with 'my-app-', got: {}",
            id
        );
        // Must end with exactly 4 hex chars
        let hash_part = id.split('-').last().unwrap();
        assert_eq!(hash_part.len(), 4, "Hash part must be 4 chars: {}", id);
        assert!(
            hash_part.chars().all(|c| c.is_ascii_hexdigit()),
            "Hash part must be hex: {}",
            id
        );
    }

    /// Different paths → different ids.
    #[test]
    fn different_paths_different_ids() {
        let p1 = PathBuf::from("/home/user/projects/my-app");
        let p2 = PathBuf::from("/home/user/projects/my-app-v2");
        assert_ne!(derive_project_id(&p1), derive_project_id(&p2));
    }

    #[test]
    fn normalize_lexical_drops_dots_and_trailing_separators() {
        assert_eq!(normalize_lexical(Path::new("/w/repo/")), PathBuf::from("/w/repo"));
        assert_eq!(normalize_lexical(Path::new("/w/./repo/src/..")), PathBuf::from("/w/repo"));
        assert_eq!(normalize_lexical(Path::new("/..")), PathBuf::from("/"));
        assert_eq!(normalize_lexical(Path::new("../x/../y")), PathBuf::from("../y"));
    }

    #[test]
    fn strip_verbatim_removes_only_the_prefix_canonicalize_adds() {
        assert_eq!(strip_verbatim(Path::new(r"\\?\C:\Users\u\Repo")), PathBuf::from(r"C:\Users\u\Repo"));
        assert_eq!(strip_verbatim(Path::new(r"\\?\UNC\srv\share\repo")), PathBuf::from(r"\\srv\share\repo"));
        assert_eq!(strip_verbatim(Path::new(r"\\?\Volume{x}\repo")), PathBuf::from(r"\\?\Volume{x}\repo"));
        assert_eq!(strip_verbatim(Path::new("/home/u/repo")), PathBuf::from("/home/u/repo"));
    }

    #[test]
    fn parse_gitdir_reads_absolute_and_relative_targets() {
        let dir = Path::new("/w/wt");
        assert_eq!(
            parse_gitdir("gitdir: /w/repo/.git/worktrees/wt\n", dir),
            Some(PathBuf::from("/w/repo/.git/worktrees/wt"))
        );
        assert_eq!(
            parse_gitdir("gitdir: ../repo/.git/worktrees/wt\r\n", dir),
            Some(PathBuf::from("/w/repo/.git/worktrees/wt"))
        );
        assert_eq!(
            parse_gitdir("gitdir: /w/my repo/.git/modules/sub", dir),
            Some(PathBuf::from("/w/my repo/.git/modules/sub"))
        );
        assert_eq!(parse_gitdir("ref: refs/heads/main", dir), None);
        assert_eq!(parse_gitdir("gitdir:   ", dir), None);
    }

    #[test]
    fn a_git_directory_makes_its_parent_the_root() {
        assert_eq!(root_from_marker(Path::new("/w/repo"), &GitMarker::Dir), PathBuf::from("/w/repo"));
    }

    #[test]
    fn a_worktree_resolves_to_the_repository_it_was_added_from() {
        let relative = GitMarker::Link {
            gitdir: PathBuf::from("/w/repo/.git/worktrees/wt"),
            commondir: Some("../..\n".into()),
        };
        assert_eq!(root_from_marker(Path::new("/w/wt"), &relative), PathBuf::from("/w/repo"));

        let absolute = GitMarker::Link {
            gitdir: PathBuf::from("/w/repo/.git/worktrees/wt"),
            commondir: Some("/w/repo/.git".into()),
        };
        assert_eq!(root_from_marker(Path::new("/w/wt"), &absolute), PathBuf::from("/w/repo"));
    }

    #[test]
    fn a_worktree_of_a_bare_repository_is_its_own_root() {
        let marker = GitMarker::Link {
            gitdir: PathBuf::from("/w/repo.git/worktrees/wt"),
            commondir: Some("../..".into()),
        };
        assert_eq!(root_from_marker(Path::new("/w/wt"), &marker), PathBuf::from("/w/wt"));
    }

    #[test]
    fn a_submodule_is_its_own_root() {
        let marker = GitMarker::Link {
            gitdir: PathBuf::from("/w/repo/.git/modules/sub"),
            commondir: None,
        };
        assert_eq!(root_from_marker(Path::new("/w/repo/sub"), &marker), PathBuf::from("/w/repo/sub"));
    }
}
