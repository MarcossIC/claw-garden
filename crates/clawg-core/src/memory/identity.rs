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

/// How many ids a root may be given: [`candidate_id`] for `n` below this.
pub const CANDIDATES: u32 = 64;

/// The `n`th id a root may be given when the ones before belong to other
/// roots: [`derive_project_id`] for 0, and the same format with `#n` after
/// the path's bytes for the rest.
pub fn candidate_id(root: &Path, n: u32) -> String {
    if n == 0 {
        return derive_project_id(root);
    }
    let mut bytes = root.as_os_str().as_encoded_bytes().to_vec();
    bytes.extend_from_slice(format!("#{n}").as_bytes());
    format!("{}-{}", basename(root), hash4(&bytes))
}

/// Whether `id` is one of the ids [`candidate_id`] gives `root`.
pub fn is_candidate_for(root: &Path, id: &str) -> bool {
    (0..CANDIDATES).any(|n| candidate_id(root, n) == id)
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

/// What makes two checkouts one project: the `origin` remote of the root's
/// git directory, normalized by [`normalize_remote`]. `None` outside git,
/// without an `origin`, or with one that is not a server.
pub fn origin_key(root: &Path) -> Option<String> {
    let marker = read_marker(root)?;
    let config = std::fs::read_to_string(git_config_dir(root, &marker).join("config")).ok()?;
    normalize_remote(&origin_url(&config)?)
}

/// The directory whose `config` holds the remotes of the checkout at `dir`:
/// the common git directory of a worktree, the git directory of a submodule.
pub fn git_config_dir(dir: &Path, marker: &GitMarker) -> PathBuf {
    match marker {
        GitMarker::Dir => dir.join(".git"),
        GitMarker::Link { gitdir, commondir: Some(common) } => normalize_lexical(&gitdir.join(common.trim())),
        GitMarker::Link { gitdir, commondir: None } => gitdir.clone(),
    }
}

/// The first `url` of the `origin` remote in the text of a git `config`.
/// Section names are matched without case and the subsection exactly, as git
/// does; `include` and `insteadOf` are not followed.
pub fn origin_url(config: &str) -> Option<String> {
    let mut in_origin = false;
    for line in config.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(header) = line.strip_prefix('[') {
            let header = header.split(']').next().unwrap_or_default().trim();
            in_origin = match header.split_once(char::is_whitespace) {
                Some((name, sub)) => {
                    name.eq_ignore_ascii_case("remote") && sub.trim().trim_matches('"') == "origin"
                }
                None => header.eq_ignore_ascii_case("remote.origin"),
            };
            continue;
        }
        if !in_origin {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if !key.trim().eq_ignore_ascii_case("url") {
            continue;
        }
        let value = value.trim();
        let value = match value.strip_prefix('"') {
            Some(quoted) => quoted.split('"').next().unwrap_or_default(),
            None => value.split(['#', ';']).next().unwrap_or_default().trim(),
        };
        return Some(value.to_string());
    }
    None
}

/// `host/owner/repo` for a remote URL, in lower case, without scheme, user,
/// password, port, trailing slash or `.git`. `None` for anything that is not
/// a server two clones would name alike: a path, `file://`, an empty value.
pub fn normalize_remote(url: &str) -> Option<String> {
    let url = url.trim();
    let (authority, path) = match url.split_once("://") {
        Some((scheme, rest)) => {
            let scheme = scheme.to_ascii_lowercase();
            if !matches!(scheme.as_str(), "https" | "http" | "ssh" | "git" | "git+ssh" | "ssh+git") {
                return None;
            }
            rest.split_once('/').unwrap_or((rest, ""))
        }
        None => {
            let drive = url.as_bytes().get(1) == Some(&b':') && url.as_bytes()[0].is_ascii_alphabetic();
            if drive || url.starts_with(['/', '.', '~', '\\']) {
                return None;
            }
            let (host, path) = url.split_once(':')?;
            if host.contains('/') {
                return None;
            }
            (host, path)
        }
    };
    let host = authority.rsplit('@').next().unwrap_or_default();
    let (host, path) = match host.split_once(':') {
        Some((h, port)) if port.bytes().all(|b| b.is_ascii_digit()) => (h, path.to_string()),
        Some((h, prefix)) => (h, format!("{prefix}/{path}")),
        None => (host, path.to_string()),
    };
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path).trim_matches('/');
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if host.is_empty() || segments.is_empty() {
        return None;
    }
    Some(format!("{host}/{}", segments.join("/")).to_lowercase())
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

    #[test]
    fn a_remote_normalizes_to_host_owner_and_repo() {
        for url in [
            "https://github.com/Acme/App.git",
            "git@github.com:acme/app.git",
            "ssh://git@github.com:22/Acme/App",
            "https://u:tok@github.com/Acme/App/",
            "git+ssh://git@GitHub.com/acme/app",
            "  https://github.com/acme/app  ",
        ] {
            let key = normalize_remote(url);
            assert_eq!(key.as_deref(), Some("github.com/acme/app"), "{url}");
            assert!(!key.unwrap().contains("tok"));
        }
        assert_eq!(normalize_remote("https://gitlab.com/group/sub/repo.git").as_deref(), Some("gitlab.com/group/sub/repo"));
        assert_ne!(normalize_remote("https://github.com/acme/app"), normalize_remote("https://github.com/me/app"));
    }

    #[test]
    fn a_local_remote_has_no_key() {
        for url in [
            "/srv/git/app.git",
            "../app",
            "./app",
            "~/repos/app",
            "file:///srv/app",
            r"C:\repos\app",
            "C:/repos/app",
            r"\\server\share\app",
            "",
            "https://github.com",
            "https://github.com/",
            "app",
            "ftp://host/app",
        ] {
            assert_eq!(normalize_remote(url), None, "{url}");
        }
    }

    #[test]
    fn the_origin_url_is_read_from_its_section() {
        let config = "[core]\n\tbare = false\n[remote \"upstream\"]\n\turl = https://github.com/up/app\n\
                      ; a comment\n[Remote \"origin\"]\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n\
                      \tURL = git@github.com:acme/app.git # mine\n\turl = https://second/url\n[branch \"main\"]\n\tremote = origin\n";
        assert_eq!(origin_url(config).as_deref(), Some("git@github.com:acme/app.git"));
        assert_eq!(origin_url("[remote \"origin\"]\n  url = \"https://h/o/r\"\n").as_deref(), Some("https://h/o/r"));
        assert_eq!(origin_url("[remote \"Origin\"]\n  url = https://h/o/r\n"), None, "the subsection is exact");
        assert_eq!(origin_url("[remote \"upstream\"]\n  url = https://h/o/r\n"), None);
        assert_eq!(origin_url(""), None);
    }

    #[test]
    fn the_config_of_a_worktree_is_in_its_common_directory() {
        let dir = Path::new("/w/wt");
        assert_eq!(git_config_dir(Path::new("/w/repo"), &GitMarker::Dir), PathBuf::from("/w/repo/.git"));
        let worktree = GitMarker::Link {
            gitdir: PathBuf::from("/w/repo/.git/worktrees/wt"),
            commondir: Some("../..\n".into()),
        };
        assert_eq!(git_config_dir(dir, &worktree), PathBuf::from("/w/repo/.git"));
        let submodule = GitMarker::Link { gitdir: PathBuf::from("/w/repo/.git/modules/sub"), commondir: None };
        assert_eq!(git_config_dir(dir, &submodule), PathBuf::from("/w/repo/.git/modules/sub"));
    }

    #[test]
    fn the_candidates_of_a_root_keep_its_name_and_start_with_its_id() {
        let root = Path::new("/work/my-app");
        assert_eq!(candidate_id(root, 0), derive_project_id(root));
        let ids: std::collections::HashSet<String> = (0..CANDIDATES).map(|n| candidate_id(root, n)).collect();
        assert!(ids.len() > 60, "{ids:?}");
        assert!(ids.iter().all(|id| id.starts_with("my-app-") && id.len() == "my-app-".len() + 4));
        assert!(is_candidate_for(root, &candidate_id(root, 7)));
        assert!(!is_candidate_for(root, "my-app-zzzz"));
    }
}
