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
    /// The newest migration the binary knows and applies, as if it were an
    /// older release.
    pub migrations_up_to: Option<i32>,
    pub serve_fault: Option<ServeFault>,
    /// The only transport the binary speaks, as if it were an older release.
    pub transport: Option<Transport>,
}

/// A transport a stamp can restrict the binary to.
#[cfg(any(test, feature = "testing"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    /// Loopback TCP alone: no socket is bound or dialled, and `clawg update`
    /// passes the old daemon's port on.
    Tcp,
}

/// Where `clawg serve` fails on purpose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServeFault {
    /// Before the store is opened: the daemon does not start and touches nothing.
    BeforeOpen,
    /// After the store is opened, and so migrated, and before the daemon answers.
    AfterOpen,
}

/// The newest migration a `testing` build applies and knows, from its stamp;
/// `None` is all of them.
#[cfg(feature = "testing")]
pub fn migrations_up_to() -> Option<i32> {
    stamp().and_then(|s| s.migrations_up_to)
}

/// Always `None`: a build without `testing` knows every migration it has.
#[cfg(not(feature = "testing"))]
pub fn migrations_up_to() -> Option<i32> {
    None
}

/// Whether a `testing` build was stamped to speak TCP alone.
#[cfg(feature = "testing")]
pub fn tcp_only() -> bool {
    stamp().and_then(|s| s.transport) == Some(Transport::Tcp)
}

/// Always `false`: a build without `testing` speaks every transport it has.
#[cfg(not(feature = "testing"))]
pub fn tcp_only() -> bool {
    false
}

/// Env var that replaces the profile's port of a `clawg serve` without
/// `--port` in a `testing` build: `off`, or a port.
#[cfg(any(test, feature = "testing"))]
pub const TEST_DEFAULT_PORT_VAR: &str = "CLAWG_TEST_DEFAULT_PORT";

/// What stands in for the profile's port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefaultPort {
    /// No TCP unless `--port` asks for it.
    Off,
    Port(u16),
}

/// The stand-in for the profile's port in a `testing` build.
#[cfg(feature = "testing")]
pub fn default_port_override() -> Option<DefaultPort> {
    parse_default_port(&std::env::var(TEST_DEFAULT_PORT_VAR).ok()?)
}

/// Always `None`: a build without `testing` binds the profile's port.
#[cfg(not(feature = "testing"))]
pub fn default_port_override() -> Option<DefaultPort> {
    None
}

#[cfg(any(test, feature = "testing"))]
fn parse_default_port(raw: &str) -> Option<DefaultPort> {
    match raw {
        "off" => Some(DefaultPort::Off),
        port => port.parse().ok().map(DefaultPort::Port),
    }
}

/// Fails with a message when a `testing` build was stamped to fail at `at`.
#[cfg(feature = "testing")]
pub fn inject_serve_fault(at: ServeFault) -> Result<(), String> {
    fault_at(stamp().and_then(|s| s.serve_fault), at)
}

/// Always succeeds: a build without `testing` fails only for real.
#[cfg(not(feature = "testing"))]
pub fn inject_serve_fault(_at: ServeFault) -> Result<(), String> {
    Ok(())
}

#[cfg(any(test, feature = "testing"))]
fn fault_at(configured: Option<ServeFault>, at: ServeFault) -> Result<(), String> {
    match configured {
        Some(fault) if fault == at => Err(format!("fault injected by the test stamp: {fault:?}")),
        _ => Ok(()),
    }
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
                ..Stamp::default()
            })
        );
    }

    #[test]
    fn a_stamp_can_cap_the_migrations_and_ask_the_daemon_to_fail() {
        let stamp = parse_stamp(&stamped(r#"{"migrations_up_to":7,"serve_fault":"after_open"}"#)).unwrap();

        assert_eq!(stamp.migrations_up_to, Some(7));
        assert_eq!(stamp.serve_fault, Some(ServeFault::AfterOpen));
        let stamp = parse_stamp(&stamped(r#"{"serve_fault":"before_open"}"#)).unwrap();
        assert_eq!(stamp.serve_fault, Some(ServeFault::BeforeOpen));
        assert_eq!(stamp.migrations_up_to, None);
        assert_eq!(parse_stamp(&stamped(r#"{"serve_fault":"somewhere_else"}"#)), None);
    }

    #[test]
    fn a_stamp_can_restrict_the_binary_to_tcp() {
        let stamp = parse_stamp(&stamped(r#"{"transport":"tcp"}"#)).unwrap();

        assert_eq!(stamp.transport, Some(Transport::Tcp));
        assert_eq!(parse_stamp(&stamped("{}")).unwrap().transport, None);
        assert_eq!(parse_stamp(&stamped(r#"{"transport":"pigeon"}"#)), None);
    }

    #[test]
    fn a_default_port_override_is_off_or_a_port() {
        assert_eq!(parse_default_port("off"), Some(DefaultPort::Off));
        assert_eq!(parse_default_port("40123"), Some(DefaultPort::Port(40123)));
        assert_eq!(parse_default_port(""), None);
        assert_eq!(parse_default_port("70000"), None);
        assert_eq!(parse_default_port("OFF"), None);
    }

    #[test]
    fn a_fault_fires_only_at_the_point_it_names() {
        let configured = Some(ServeFault::AfterOpen);

        assert_eq!(fault_at(configured, ServeFault::BeforeOpen), Ok(()));
        assert_eq!(fault_at(None, ServeFault::AfterOpen), Ok(()));
        let fired = fault_at(configured, ServeFault::AfterOpen).unwrap_err();
        assert!(fired.contains("AfterOpen"), "{fired}");
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
        assert_eq!(migrations_up_to(), None);
        assert_eq!(inject_serve_fault(ServeFault::AfterOpen), Ok(()));
        assert_eq!(version(), env!("CARGO_PKG_VERSION"));
        std::env::set_var(TEST_DEFAULT_PORT_VAR, "off");
        assert_eq!(default_port_override(), None);
        assert!(!tcp_only());
    }
}
