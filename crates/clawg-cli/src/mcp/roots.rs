//! The folders a client has open, read from its MCP roots.
//!
//! A host does not say where the user works by the directory it starts the
//! bridge in: Cursor starts it in the home directory whatever its windows
//! hold. What it does answer is `roots/list`, with the folders of every
//! window open at that moment, and it never notifies a change. So a call that
//! names no project reads the roots the client gives in that call, and only a
//! client that gives none falls back on the bridge's own directory.

use std::path::{Path, PathBuf};
use std::time::Duration;

use clawg_core::memory::ProjectIdentity;
use rmcp::service::RequestContext;
use rmcp::RoleServer;

/// How long a call waits for the client's `roots/list`. A client answers it
/// from what it holds, so this only bounds one that never answers; past it
/// the call reads as a client without roots.
pub const ROOTS_TIMEOUT: Duration = Duration::from_secs(2);

/// The folders the client of `ctx` has open, asked in this call: empty when
/// it declares no `roots`, answers with an error or not in time, or names no
/// folder this machine has.
pub async fn asked(ctx: &RequestContext<RoleServer>) -> Vec<PathBuf> {
    if ctx.client_capabilities().and_then(|c| c.roots).is_none() {
        return Vec::new();
    }
    match tokio::time::timeout(ROOTS_TIMEOUT, uris(ctx)).await {
        Ok(Ok(uris)) => uris.iter().filter_map(|uri| folder_of(uri)).collect(),
        Ok(Err(e)) => {
            log::debug!("the client did not list its roots: {e}");
            Vec::new()
        }
        Err(_) => {
            log::warn!("the client did not list its roots within {} s", ROOTS_TIMEOUT.as_secs());
            Vec::new()
        }
    }
}

/// The `uri` of each root the client answers `roots/list` with.
#[expect(deprecated, reason = "rmcp deprecates roots (SEP-2577), yet roots/list is the one place Cursor names the folders it has open")]
async fn uris(ctx: &RequestContext<RoleServer>) -> Result<Vec<String>, rmcp::service::ServiceError> {
    Ok(ctx.peer.list_roots().await?.roots.into_iter().map(|root| root.uri).collect())
}

/// The folder a root's `uri` names on this machine, or `None` when it names
/// none. A `file:` URI is read as one; anything else as the path itself,
/// which is what Cursor sends (`c:\...`), when it is absolute here.
pub fn folder_of(uri: &str) -> Option<PathBuf> {
    let text = match uri.get(..5) {
        Some(scheme) if scheme.eq_ignore_ascii_case("file:") => uri_path(uri, cfg!(windows))?,
        _ => uri.to_string(),
    };
    let path = PathBuf::from(text);
    path.is_absolute().then_some(path)
}

/// The path of a `file:` URI, decoded, when its host is this machine (empty
/// or `localhost`). On Windows the drive follows a `/` that is not part of
/// the path (`file:///C:/x` is `C:/x`).
fn uri_path(uri: &str, windows: bool) -> Option<String> {
    let rest = uri.get(..5).filter(|s| s.eq_ignore_ascii_case("file:")).and(uri.get(5..))?;
    let rest = rest.split(['?', '#']).next().unwrap_or_default();
    let path = match rest.strip_prefix("//") {
        Some(authority_and_path) => {
            let at = authority_and_path.find('/')?;
            let (host, path) = authority_and_path.split_at(at);
            (host.is_empty() || host.eq_ignore_ascii_case("localhost")).then_some(path)?
        }
        None => rest.starts_with('/').then_some(rest)?,
    };
    let path = percent_decode(path)?;
    let bytes = path.as_bytes();
    let drive = bytes.len() >= 3 && bytes[1].is_ascii_alphabetic() && bytes[2] == b':';
    Some(if windows && drive { path[1..].to_string() } else { path })
}

/// `text` with each `%XX` turned into its byte, as long as the result is
/// UTF-8 and every `%` starts an escape.
fn percent_decode(text: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(text.len());
    let mut rest = text.as_bytes();
    while let Some((&first, tail)) = rest.split_first() {
        if first == b'%' {
            let hex = std::str::from_utf8(tail.get(..2)?).ok()?;
            bytes.push(u8::from_str_radix(hex, 16).ok()?);
            rest = &tail[2..];
        } else {
            bytes.push(first);
            rest = tail;
        }
    }
    String::from_utf8(bytes).ok()
}

/// The folders a call that names no project reads: the client's `roots`,
/// resolved as a `project_path` is, in their order and once each root; when
/// none is left, the project of the directory the bridge started in. A
/// folder whose root is `home` is never one of them, from either side.
pub fn open_folders(
    roots: &[PathBuf],
    start: Option<&ProjectIdentity>,
    home: Option<&Path>,
    resolve: impl Fn(&Path) -> ProjectIdentity,
) -> Vec<ProjectIdentity> {
    let not_home = |project: &ProjectIdentity| home != Some(project.root.as_path());
    let mut folders: Vec<ProjectIdentity> = Vec::new();
    for project in roots.iter().map(|root| resolve(root)).filter(not_home) {
        if !folders.iter().any(|f| f.root == project.root) {
            folders.push(project);
        }
    }
    if folders.is_empty() {
        folders.extend(start.filter(|p| not_home(p)).cloned());
    }
    folders
}

#[cfg(test)]
mod tests {
    use super::*;

    use clawg_core::memory::derive_project_id;

    #[test]
    fn a_root_is_read_as_a_uri_or_a_path() {
        assert_eq!(uri_path("file:///work/my%20repo", false).as_deref(), Some("/work/my repo"));
        assert_eq!(uri_path("file://localhost/work/repo", false).as_deref(), Some("/work/repo"));
        assert_eq!(uri_path("FILE:///work/repo", false).as_deref(), Some("/work/repo"), "the scheme ignores case");
        assert_eq!(uri_path("file:/work/repo", false).as_deref(), Some("/work/repo"), "no authority at all");
        assert_eq!(uri_path("file:///work/a%23b#frag", false).as_deref(), Some("/work/a#b"), "a fragment is not the path");
        assert_eq!(uri_path("file:///C:/work/repo", true).as_deref(), Some("C:/work/repo"));
        assert_eq!(uri_path("file:///c%3A/work/repo", true).as_deref(), Some("c:/work/repo"), "VS Code encodes the colon");
        assert_eq!(uri_path("file:///C:/work/repo", false).as_deref(), Some("/C:/work/repo"), "only Windows has drives");
        assert_eq!(uri_path("file://server/share/repo", false), None, "a folder of another host");
        assert_eq!(uri_path("file:///work/%ff", false), None, "not UTF-8");
        assert_eq!(uri_path("file:///work/%zz", false), None, "not an escape");
        assert_eq!(uri_path("https://example.com/repo", false), None);

        let here = std::env::temp_dir();
        assert_eq!(folder_of(&here.display().to_string()), Some(here.clone()), "Cursor sends the path itself");
        assert_eq!(folder_of("work/repo"), None, "a relative path names no folder");
        assert_eq!(folder_of(""), None);
    }

    #[test]
    fn a_root_as_a_path_and_as_a_uri_is_one_project() {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = dir.path().join("my repo");
        std::fs::create_dir(&repo).expect("folder");
        let text = repo.display().to_string();
        let encoded = text.replace('\\', "/").replace(' ', "%20");
        let uri = if encoded.starts_with('/') { format!("file://{encoded}") } else { format!("file:///{encoded}") };
        // Cursor sends a Windows path with the drive in lower case.
        let as_cursor_sends = match text.split_at_checked(1) {
            Some((drive, rest)) if rest.starts_with(':') => format!("{}{rest}", drive.to_lowercase()),
            _ => text.clone(),
        };

        let from_uri = folder_of(&uri).expect("the uri names a folder");
        let from_path = folder_of(&as_cursor_sends).expect("the path names a folder");

        assert_eq!(ProjectIdentity::resolve(&from_uri).id, ProjectIdentity::resolve(&from_path).id, "{uri} and {as_cursor_sends}");
        assert_eq!(ProjectIdentity::resolve(&from_uri).id, ProjectIdentity::resolve(&repo).id);
    }

    /// A project whose root is `path`, as a resolver that reads nothing.
    fn at(path: &str) -> ProjectIdentity {
        let root = PathBuf::from(path);
        ProjectIdentity { id: derive_project_id(&root), legacy_id: derive_project_id(&root), root: root.clone(), path: root }
    }

    /// A resolver that climbs from `/work/a/src` to `/work/a`, as a checkout's root would.
    fn resolve(path: &Path) -> ProjectIdentity {
        match path.to_str() {
            Some("/work/a/src") => ProjectIdentity { path: path.to_path_buf(), ..at("/work/a") },
            Some(other) => at(other),
            None => at("/"),
        }
    }

    fn roots(folders: &[ProjectIdentity]) -> Vec<&str> {
        folders.iter().map(|f| f.root.to_str().expect("utf-8")).collect()
    }

    #[test]
    fn the_open_folders_are_the_roots_else_the_directory() {
        let start = at("/start");
        let declared = [PathBuf::from("/work/b"), PathBuf::from("/work/a"), PathBuf::from("/work/a/src")];

        assert_eq!(roots(&open_folders(&declared, Some(&start), None, resolve)), ["/work/b", "/work/a"], "in their order, once each root");
        assert_eq!(roots(&open_folders(&[], Some(&start), None, resolve)), ["/start"]);
        assert!(open_folders(&[], None, None, resolve).is_empty());
    }

    #[test]
    fn the_home_directory_is_never_a_project() {
        let home = Path::new("/home/u");
        let declared = [PathBuf::from("/home/u"), PathBuf::from("/work/a")];

        assert_eq!(roots(&open_folders(&declared, Some(&at("/start")), Some(home), resolve)), ["/work/a"]);
        assert!(open_folders(&[], Some(&at("/home/u")), Some(home), resolve).is_empty(), "a bridge started at home");
        assert!(
            open_folders(&[PathBuf::from("/home/u")], Some(&at("/home/u")), Some(home), resolve).is_empty(),
            "a client whose only root is home, in a bridge started there"
        );
        assert_eq!(roots(&open_folders(&[PathBuf::from("/home/u")], Some(&at("/start")), Some(home), resolve)), ["/start"]);
    }
}
