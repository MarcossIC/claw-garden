//! Where a call that names no project reads: the roots the MCP client
//! declares, read as folders, else the directory the bridge was started in,
//! and never the user's home directory.
//!
//! The roots are asked for on every call and never kept: Cursor gives every
//! window the folders of all the windows open at that moment, and says
//! nothing when they change (`roots.listChanged: false`).

use std::path::{Path, PathBuf};
use std::time::Duration;

use clawg_core::memory::identity::strip_verbatim;
use clawg_core::memory::project_docs::corpus::{auto_indexable, has_project_marker};
use clawg_core::memory::ProjectIdentity;
use rmcp::service::RequestContext;
use rmcp::RoleServer;

/// How long a call waits for the client to answer `roots/list`. A local
/// client answers in milliseconds; this only bounds one that never does,
/// whose call then goes on as a client's without roots.
pub const ROOTS_WAIT: Duration = Duration::from_secs(3);

/// What the bridge knows of the machine before any call: the directory its
/// process was started in and the user's home directory.
#[derive(Debug, Clone, Default)]
pub struct Start {
    pub dir: Option<PathBuf>,
    pub home: Option<PathBuf>,
}

impl Start {
    /// This process's.
    pub fn of_process() -> Self {
        Self { dir: std::env::current_dir().ok(), home: clawg_core::paths::home_dir() }
    }
}

/// A folder the person has open: a root the client declared, or the
/// directory the bridge was started in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folder {
    pub project: ProjectIdentity,
    /// Whether the daemon may read its files without the person adding it:
    /// it has a `.git` or a manifest and is not the home directory.
    pub indexable: bool,
}

impl Folder {
    /// The folder at `path`, or none when it is `home` or its project's root
    /// is: the home directory is never a project. `home` is canonical.
    pub fn at(path: &Path, home: Option<&Path>) -> Option<Self> {
        let project = ProjectIdentity::resolve(path);
        if home.is_some_and(|home| project.path == home || project.root == home) {
            return None;
        }
        let indexable = auto_indexable(&project.root, home, has_project_marker(&project.root));
        Some(Self { project, indexable })
    }
}

/// `home` as a resolved path reads it: canonical, without the `\\?\` prefix.
pub fn canonical_home(home: &Path) -> PathBuf {
    std::fs::canonicalize(home).map(|c| strip_verbatim(&c)).unwrap_or_else(|_| home.to_path_buf())
}

/// The folders the roots `uris` name, without the home directory and
/// without repeats, in the client's order.
pub fn folders(uris: &[String], home: Option<&Path>) -> Vec<Folder> {
    let mut folders: Vec<Folder> = Vec::new();
    for uri in uris {
        let Some(path) = root_path(uri, cfg!(windows)) else {
            log::warn!("a root the client declared is not a folder clawg can read: {uri}");
            continue;
        };
        if let Some(folder) = Folder::at(&path, home) {
            if !folders.iter().any(|f| f.project == folder.project) {
                folders.push(folder);
            }
        }
    }
    folders
}

/// The `uri` of each root the client of `ctx` declares now, or `None` when
/// it declares no `roots` capability, or does not answer `roots/list` in
/// [`ROOTS_WAIT`], which counts as the same.
#[expect(deprecated, reason = "roots are what Cursor answers; rmcp marks them for a protocol version no client in use speaks")]
pub async fn declared(ctx: &RequestContext<RoleServer>) -> Option<Vec<String>> {
    ctx.client_capabilities()?.roots?;
    match tokio::time::timeout(ROOTS_WAIT, ctx.peer.list_roots()).await {
        Ok(Ok(listed)) => Some(listed.roots.into_iter().map(|root| root.uri).collect()),
        Ok(Err(e)) => {
            log::warn!("the client declares roots, but roots/list failed: {e}");
            None
        }
        Err(_) => {
            log::warn!("the client declares roots, but did not answer roots/list within {ROOTS_WAIT:?}");
            None
        }
    }
}

/// The folder a root's `uri` names, read as on Windows when `windows`: a
/// `file:` URI, its host `localhost` or none and its `%XX` decoded (on
/// Windows `/c:/x` is `c:\x` and another host a UNC path), or an absolute
/// path as Cursor sends it. Anything else names no folder: a relative path,
/// another scheme, a bad escape, a path that is not UTF-8.
pub fn root_path(uri: &str, windows: bool) -> Option<PathBuf> {
    let text = match uri.strip_prefix("file:") {
        Some(rest) => from_file_uri(rest, windows)?,
        None if uri.contains("://") => return None,
        None => uri.to_string(),
    };
    let text = if windows { text.replace('/', "\\") } else { text };
    is_absolute(&text, windows).then(|| PathBuf::from(text))
}

/// The path of a `file:` URI after the scheme.
fn from_file_uri(rest: &str, windows: bool) -> Option<String> {
    let (host, path) = match rest.strip_prefix("//") {
        Some(authority) => match authority.find('/') {
            Some(at) => (&authority[..at], &authority[at..]),
            None => (authority, ""),
        },
        None => ("", rest),
    };
    let path = percent_decode(path)?;
    match (host, windows) {
        ("" | "localhost", true) => {
            let bytes = path.as_bytes();
            let drive = bytes.len() >= 3 && bytes[0] == b'/' && bytes[1].is_ascii_alphabetic() && bytes[2] == b':';
            Some(if drive { path[1..].to_string() } else { path })
        }
        ("" | "localhost", false) => Some(path),
        (host, true) if !path.is_empty() => Some(format!("//{host}{path}")),
        _ => None,
    }
}

/// `text` with every `%XX` replaced by its byte, or `None` when an escape is
/// not two hex digits or the bytes are not UTF-8.
fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Whether `text` is an absolute path on Windows (`c:\x` or `\\host\share`)
/// or, otherwise, on Unix.
fn is_absolute(text: &str, windows: bool) -> bool {
    let bytes = text.as_bytes();
    if !windows {
        return bytes.first() == Some(&b'/');
    }
    let drive = bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\';
    drive || text.starts_with(r"\\")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on_windows(uri: &str) -> Option<String> {
        root_path(uri, true).map(|p| p.display().to_string())
    }

    fn on_unix(uri: &str) -> Option<String> {
        root_path(uri, false).map(|p| p.display().to_string())
    }

    #[test]
    fn a_root_is_a_folder_as_a_uri_or_as_a_path() {
        let repo = Some(r"c:\Users\u\repo".to_string());
        assert_eq!(on_windows(r"c:\Users\u\repo"), repo, "as Cursor sends it");
        assert_eq!(on_windows("file:///c%3A/Users/u/repo"), repo, "as VS Code encodes it");
        assert_eq!(on_windows("file:///c:/Users/u/repo"), repo);
        assert_eq!(on_windows("file://localhost/c:/Users/u/repo"), repo);
        assert_eq!(on_windows("c:/Users/u/repo"), repo);
        assert_eq!(on_windows("file:///C:/Users/u/my%20repo"), Some(r"C:\Users\u\my repo".into()));
        assert_eq!(on_windows("file://server/share/repo"), Some(r"\\server\share\repo".into()));
        assert_eq!(on_windows(r"\\server\share\repo"), Some(r"\\server\share\repo".into()));

        assert_eq!(on_unix("/home/u/repo"), Some("/home/u/repo".into()));
        assert_eq!(on_unix("file:///home/u/my%20repo"), Some("/home/u/my repo".into()));
        assert_eq!(on_unix("file://localhost/home/u/repo"), Some("/home/u/repo".into()));
        assert_eq!(on_unix("file:/home/u/repo"), Some("/home/u/repo".into()));
        assert_eq!(on_unix("file:///home/u/ni%C3%B1o"), Some("/home/u/niño".into()));
    }

    #[test]
    fn a_root_that_names_no_folder_here_is_dropped() {
        for uri in ["repo", "./repo", "vscode-remote://ssh-remote+box/home/u/repo", "file:///home/u/%zz", "file:///home/u/%F", "file:///home/u/%FF", "file://server/share", "c:\\repo"] {
            assert_eq!(on_unix(uri), None, "{uri}");
        }
        for uri in ["repo", r"..\repo", "file:///home/u/repo", "file:///c%3A/%zz", "file://server", r"\repo"] {
            assert_eq!(on_windows(uri), None, "{uri}");
        }
    }

    #[test]
    fn the_home_directory_and_its_root_are_no_folder() {
        let dir = tempfile::tempdir().expect("tempdir");
        let home = canonical_home(dir.path());
        std::fs::create_dir_all(home.join("notes")).expect("notes");
        std::fs::create_dir_all(home.join("repo").join(".git")).expect("repo");

        assert_eq!(Folder::at(&home, Some(&home)), None);
        let notes = Folder::at(&home.join("notes"), Some(&home)).expect("a folder in home is a folder");
        assert!(!notes.indexable, "a folder of notes is not read on its own");
        assert!(Folder::at(&home.join("repo"), Some(&home)).expect("a repository").indexable);

        std::fs::create_dir_all(home.join(".git")).expect("home under git");
        assert_eq!(Folder::at(&home.join("notes"), Some(&home)), None, "a folder whose root is home is home");
    }
}
