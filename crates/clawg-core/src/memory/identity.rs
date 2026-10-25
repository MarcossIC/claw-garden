use std::path::{Component, Path, PathBuf};
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant, UNIX_EPOCH};
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

    /// What a registration of this project names as its legacy ids: the id of
    /// the path it was reached through, when that is not the root's.
    pub fn legacy_ids(&self) -> Vec<String> {
        Some(self.legacy_id.clone()).filter(|l| *l != self.id).into_iter().collect()
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
    key_in(&git_config_dir(root, &marker))
}

fn key_in(git_dir: &Path) -> Option<String> {
    let config = std::fs::read_to_string(git_dir.join("config")).ok()?;
    normalize_remote(&origin_url(&config)?)
}

/// What tells the git directory of `root` ([`git_config_dir`]) from another
/// one at the same path: when it was created and, on Unix, its device and
/// inode. A new clone, `git init` or copy changes it, a move within one
/// filesystem does not. `None` without a git directory.
pub fn git_stamp(root: &Path) -> Option<String> {
    let marker = read_marker(root)?;
    stamp_of(&git_config_dir(root, &marker))
}

fn stamp_of(git_dir: &Path) -> Option<String> {
    let meta = std::fs::metadata(git_dir).ok()?;
    let created = meta.created().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_nanos());
    stamp_from(&meta, created)
}

#[cfg(unix)]
fn stamp_from(meta: &std::fs::Metadata, created: Option<u128>) -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    // Some filesystems keep no creation date: device and inode still tell
    // two directories apart while both exist.
    let created = created.map(|c| c.to_string()).unwrap_or_default();
    Some(format!("{}:{}:{created}", meta.dev(), meta.ino()))
}

#[cfg(not(unix))]
fn stamp_from(_meta: &std::fs::Metadata, created: Option<u128>) -> Option<String> {
    created.map(|c| c.to_string())
}

/// What a registration reads of a root every time: the key of its `origin`
/// ([`origin_key`]) and the stamp of its git directory ([`git_stamp`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Checkout {
    pub key: Option<String>,
    pub stamp: Option<String>,
}

/// What reading the first commit of a checkout found ([`root_commit`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RootCommit {
    /// The commit without parents that the first-parent chain of `HEAD` ends in.
    Known(String),
    /// Not to be known for this git directory: it is shallow, so its chain
    /// ends before the first commit, or `git` did not answer within
    /// [`ROOT_COMMIT_WAIT`]. Recorded as none beside the stamp, and not read
    /// again until the git directory is another.
    Unknowable,
    /// Not now: there is no git directory, `git` could not be run, or it
    /// ended in an error, as in a repository without commits. Nothing is
    /// recorded, so the next registration reads it again.
    Later,
}

/// How long `git` may take to give the first commit before it is stopped.
pub const ROOT_COMMIT_WAIT: Duration = Duration::from_secs(1);

/// The first commit of the checkout at `root`, from `git rev-list
/// --max-parents=0 --first-parent HEAD`: a history merged in later comes in
/// through a second parent, so it adds no root. Blocks for up to
/// [`ROOT_COMMIT_WAIT`].
pub fn root_commit(root: &Path) -> RootCommit {
    let Some(marker) = read_marker(root) else {
        return RootCommit::Later;
    };
    if git_config_dir(root, &marker).join("shallow").exists() {
        return RootCommit::Unknowable;
    }
    let mut command = git_in(root);
    command.args(["rev-list", "--max-parents=0", "--first-parent", "HEAD"]);
    let Ok(mut child) = command.spawn() else {
        return RootCommit::Later;
    };
    let Some(status) = wait_or_kill(&mut child, ROOT_COMMIT_WAIT) else {
        return RootCommit::Unknowable;
    };
    let mut out = String::new();
    let read = child.stdout.take().is_some_and(|mut stdout| stdout.read_to_string(&mut out).is_ok());
    match first_commit_in(&out) {
        Some(hash) if status.success() && read => RootCommit::Known(hash),
        _ => RootCommit::Later,
    }
}

/// `git` in `root` as the daemon runs it: without the `GIT_*` of whoever
/// started it, which can point it at another repository or history, without
/// prompts, optional locks or a filesystem monitor, with no input, and on
/// Windows without a console, since the daemon is started detached from one
/// and each `git` would open its own.
fn git_in(root: &Path) -> Command {
    let mut command = Command::new("git");
    for (key, _) in std::env::vars_os() {
        if key.as_encoded_bytes().get(..4).is_some_and(|p| p.eq_ignore_ascii_case(b"GIT_")) {
            command.env_remove(key);
        }
    }
    command
        .current_dir(root)
        .args(["-c", "core.fsmonitor=false"])
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

/// Waits for `child` until `wait` passes, then kills it. `None` when it had
/// to be killed.
fn wait_or_kill(child: &mut Child, wait: Duration) -> Option<ExitStatus> {
    let deadline = Instant::now() + wait;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

/// The hash in what `rev-list` printed, when that is one line of a SHA-1 or
/// SHA-256 hash and nothing else.
fn first_commit_in(stdout: &str) -> Option<String> {
    let mut lines = stdout.lines();
    let line = lines.next()?;
    let hash = lines.next().is_none() && matches!(line.len(), 40 | 64) && line.bytes().all(|b| b.is_ascii_hexdigit());
    hash.then(|| line.to_ascii_lowercase())
}

/// What a registration reads from the disk, kept apart from the decisions it
/// feeds so those are tested without `git`. [`Disk`] reads the real one.
pub trait Ground: Send + Sync {
    /// The key and the stamp of what is at `root` today.
    fn checkout(&self, root: &Path) -> Checkout;
    /// The first commit of the checkout at `root`.
    fn root_commit(&self, root: &Path) -> RootCommit;
    /// Whether the checkout recorded at `path` with `stamp` is still there:
    /// the path exists and what it holds has that stamp.
    fn recorded_checkout_present(&self, path: &Path, stamp: Option<&str>) -> bool;
}

/// The filesystem and `git` of this machine.
pub struct Disk;

impl Ground for Disk {
    fn checkout(&self, root: &Path) -> Checkout {
        let Some(marker) = read_marker(root) else {
            return Checkout::default();
        };
        let git_dir = git_config_dir(root, &marker);
        Checkout { key: key_in(&git_dir), stamp: stamp_of(&git_dir) }
    }

    fn root_commit(&self, root: &Path) -> RootCommit {
        root_commit(root)
    }

    fn recorded_checkout_present(&self, path: &Path, stamp: Option<&str>) -> bool {
        path.exists() && git_stamp(path).as_deref() == stamp
    }
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

    #[test]
    fn a_command_that_does_not_end_in_time_is_stopped() {
        let mut command = if cfg!(windows) {
            let mut c = Command::new("ping");
            c.args(["-n", "30", "127.0.0.1"]);
            c
        } else {
            let mut c = Command::new("sleep");
            c.arg("30");
            c
        };
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn a long-lived process");

        let started = std::time::Instant::now();
        assert_eq!(wait_or_kill(&mut child, Duration::from_millis(300)), None);
        assert!(started.elapsed() >= Duration::from_millis(250), "it waited");
        assert!(started.elapsed() < Duration::from_secs(10), "it did not wait for the command to end");
        assert!(matches!(child.try_wait(), Ok(Some(_))), "the command was ended");
    }

    #[test]
    fn only_one_line_of_a_hash_is_a_first_commit() {
        let sha1 = "3f786850e387550fdab836ed7e6dc881de23001b";
        let sha256 = "a".repeat(64);
        assert_eq!(first_commit_in(&format!("{sha1}\n")).as_deref(), Some(sha1));
        assert_eq!(first_commit_in(&format!("{}\r\n", sha1.to_uppercase())).as_deref(), Some(sha1));
        assert_eq!(first_commit_in(&sha256).as_deref(), Some(sha256.as_str()));
        let (longer, twice, not_hex) = (format!("{sha1}0"), format!("{sha1}\n{sha1}\n"), sha1.replace('3', "g"));
        for answer in ["", "\n", &sha1[..39], longer.as_str(), twice.as_str(), not_hex.as_str()] {
            assert_eq!(first_commit_in(answer), None, "{answer:?}");
        }
    }
}
