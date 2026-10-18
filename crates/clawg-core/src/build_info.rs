//! What this binary says it is, read from one place.
//!
//! A build reports the version of the workspace and the target it was built
//! to be distributed for. A `testing` build can instead take both from a stamp
//! at the end of its own file, and take the release server and the trusted keys
//! of `clawg update` from the environment; a release build reads none of it
//! (`cfg(feature = "testing")`, the rule of `paths::TEST_HOME_VAR`).

#[cfg(any(test, feature = "testing"))]
use std::path::Path;

/// The version of the workspace, or the stamp's in a `testing` build.
#[cfg(feature = "testing")]
pub fn version() -> &'static str {
    stamp().and_then(|s| s.version.as_deref()).unwrap_or(env!("CARGO_PKG_VERSION"))
}

/// The version of the workspace.
#[cfg(not(feature = "testing"))]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// The target triple only `release.yml` builds are marked with
/// (`CLAWG_DIST_TARGET`), or the stamp's in a `testing` build.
#[cfg(feature = "testing")]
pub fn dist_target() -> Option<&'static str> {
    stamp().and_then(|s| s.dist_target.as_deref()).or_else(built_dist_target)
}

/// The target triple only `release.yml` builds are marked with
/// (`CLAWG_DIST_TARGET`).
#[cfg(not(feature = "testing"))]
pub fn dist_target() -> Option<&'static str> {
    built_dist_target()
}

fn built_dist_target() -> Option<&'static str> {
    option_env!("CLAWG_DIST_TARGET").filter(|target| !target.is_empty())
}

/// Env var that replaces the release listing's base URL in a `testing` build.
#[cfg(any(test, feature = "testing"))]
pub const TEST_RELEASE_URL_VAR: &str = "CLAWG_TEST_RELEASE_URL";

/// Env var that replaces the trusted public keys in a `testing` build: a JSON
/// array with the text of one `.pub` file per key.
#[cfg(any(test, feature = "testing"))]
pub const TEST_RELEASE_KEYS_VAR: &str = "CLAWG_TEST_RELEASE_KEYS";

/// The release server a `testing` build talks to instead of GitHub's.
#[cfg(feature = "testing")]
pub fn release_url_override() -> Option<String> {
    non_empty(std::env::var_os(TEST_RELEASE_URL_VAR))
}

/// Always `None`: a build without `testing` talks to GitHub's.
#[cfg(not(feature = "testing"))]
pub fn release_url_override() -> Option<String> {
    None
}

/// The keys a `testing` build trusts instead of the embedded ones.
#[cfg(feature = "testing")]
pub fn release_keys_override() -> Option<Vec<String>> {
    parse_keys(&std::env::var(TEST_RELEASE_KEYS_VAR).ok()?)
}

/// Always `None`: a build without `testing` trusts the embedded keys alone.
#[cfg(not(feature = "testing"))]
pub fn release_keys_override() -> Option<Vec<String>> {
    None
}

#[cfg(any(test, feature = "testing"))]
fn non_empty(raw: Option<std::ffi::OsString>) -> Option<String> {
    raw.and_then(|v| v.into_string().ok()).filter(|v| !v.is_empty())
}

#[cfg(any(test, feature = "testing"))]
fn parse_keys(raw: &str) -> Option<Vec<String>> {
    serde_json::from_str(raw).ok()
}

/// The text a test puts after a copy of the binary: a NUL, this marker and a
/// JSON object that runs to the end of the file.
#[cfg(any(test, feature = "testing"))]
pub const STAMP_MARKER: &[u8] = b"\0CLAWG-TEST-STAMP ";

/// How much of the end of the file is searched for [`STAMP_MARKER`].
#[cfg(any(test, feature = "testing"))]
pub const STAMP_WINDOW: u64 = 4096;

/// What a stamp can say about the binary it is on. A missing field leaves the
/// build's own value.
#[cfg(any(test, feature = "testing"))]
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct Stamp {
    pub version: Option<String>,
    pub dist_target: Option<String>,
}

/// The stamp at the end of `tail`: the last marker in it, followed by a JSON
/// object that goes on to the end.
#[cfg(any(test, feature = "testing"))]
pub fn parse_stamp(tail: &[u8]) -> Option<Stamp> {
    let at = tail.windows(STAMP_MARKER.len()).rposition(|w| w == STAMP_MARKER)?;
    serde_json::from_slice(&tail[at + STAMP_MARKER.len()..]).ok()
}

/// The last [`STAMP_WINDOW`] bytes of `path`.
#[cfg(any(test, feature = "testing"))]
fn read_tail(path: &Path) -> Option<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    file.seek(SeekFrom::Start(len.saturating_sub(STAMP_WINDOW))).ok()?;
    let mut tail = Vec::new();
    file.read_to_end(&mut tail).ok()?;
    Some(tail)
}

/// The stamp of the running executable, read once.
#[cfg(feature = "testing")]
fn stamp() -> Option<&'static Stamp> {
    static STAMP: std::sync::OnceLock<Option<Stamp>> = std::sync::OnceLock::new();
    STAMP
        .get_or_init(|| parse_stamp(&read_tail(&std::env::current_exe().ok()?)?))
        .as_ref()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stamped(json: &str) -> Vec<u8> {
        let mut bytes = b"MZ\x90\x00 an executable image".to_vec();
        bytes.extend_from_slice(STAMP_MARKER);
        bytes.extend_from_slice(json.as_bytes());
        bytes
    }

    #[test]
    fn a_stamp_at_the_end_is_read() {
        let stamp = parse_stamp(&stamped(r#"{"version":"0.18.0","dist_target":"x86_64-pc-windows-msvc"}"#));

        assert_eq!(
            stamp,
            Some(Stamp {
                version: Some("0.18.0".into()),
                dist_target: Some("x86_64-pc-windows-msvc".into()),
            })
        );
    }

    #[test]
    fn fields_the_stamp_leaves_out_stay_unset_and_unknown_ones_are_ignored() {
        assert_eq!(parse_stamp(&stamped("{}")), Some(Stamp::default()));
        let stamp = parse_stamp(&stamped(r#"{"version":"1.0.0","later":true}"#)).unwrap();
        assert_eq!(stamp.version.as_deref(), Some("1.0.0"));
        assert_eq!(stamp.dist_target, None);
    }

    #[test]
    fn the_last_marker_wins() {
        let mut bytes = stamped(r#"{"version":"1.0.0"}"#);
        bytes.extend_from_slice(STAMP_MARKER);
        bytes.extend_from_slice(br#"{"version":"2.0.0"}"#);

        assert_eq!(parse_stamp(&bytes).unwrap().version.as_deref(), Some("2.0.0"));
    }

    #[test]
    fn no_marker_or_a_json_that_does_not_reach_the_end_is_no_stamp() {
        assert_eq!(parse_stamp(b"MZ just an executable"), None);
        assert_eq!(parse_stamp(&stamped(r#"{"version":"1.0.0"} trailing"#)), None);
        assert_eq!(parse_stamp(&stamped(r#"{"version":"1.0"#)), None);
        assert_eq!(parse_stamp(&stamped("not json")), None);
        assert_eq!(parse_stamp(b"CLAWG-TEST-STAMP {}"), None, "the marker starts with a NUL");
    }

    #[test]
    fn only_the_end_of_a_big_file_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clawg");
        let mut file = vec![7u8; 100_000];
        file.append(&mut stamped(r#"{"version":"0.0.1"}"#));
        std::fs::write(&path, &file).unwrap();

        let tail = read_tail(&path).unwrap();

        assert_eq!(tail.len() as u64, STAMP_WINDOW);
        assert_eq!(parse_stamp(&tail).unwrap().version.as_deref(), Some("0.0.1"));
        assert_eq!(read_tail(&dir.path().join("missing")), None);
    }

    #[test]
    fn a_small_file_is_read_whole() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clawg");
        std::fs::write(&path, stamped("{}")).unwrap();

        assert_eq!(parse_stamp(&read_tail(&path).unwrap()), Some(Stamp::default()));
    }

    #[test]
    fn the_override_values_are_text_and_empty_means_unset() {
        assert_eq!(non_empty(Some("http://127.0.0.1:1".into())).as_deref(), Some("http://127.0.0.1:1"));
        assert_eq!(non_empty(Some("".into())), None);
        assert_eq!(non_empty(None), None);
        assert_eq!(parse_keys(r#"["a\nb\n","c"]"#), Some(vec!["a\nb\n".into(), "c".into()]));
        assert_eq!(parse_keys("not an array"), None);
    }

    #[cfg(feature = "testing")]
    #[test]
    fn update_overrides_are_read_only_in_a_test_build() {
        std::env::set_var(TEST_RELEASE_URL_VAR, "http://127.0.0.1:9");
        std::env::set_var(TEST_RELEASE_KEYS_VAR, r#"["key"]"#);

        assert_eq!(release_url_override().as_deref(), Some("http://127.0.0.1:9"));
        assert_eq!(release_keys_override(), Some(vec!["key".to_string()]));
    }

    #[cfg(not(feature = "testing"))]
    #[test]
    fn update_overrides_are_read_only_in_a_test_build() {
        std::env::set_var(TEST_RELEASE_URL_VAR, "http://127.0.0.1:9");
        std::env::set_var(TEST_RELEASE_KEYS_VAR, r#"["key"]"#);

        assert_eq!(release_url_override(), None);
        assert_eq!(release_keys_override(), None);
        assert_eq!(version(), env!("CARGO_PKG_VERSION"));
    }
}
