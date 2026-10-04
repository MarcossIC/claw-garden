//! On-disk liveness record and bearer token.
//!
//! Two separate files under the profile's data directory, each with a reason
//! to be separate:
//!
//! * `daemon.json` — **public**. Anything that wants to know whether a daemon
//!   is up reads it, so it carries no secret.
//! * `token` — **secret**. Its file permissions are the access control for the
//!   whole loopback API; merging it into `daemon.json` would hand the token to
//!   every liveness check.
//!
//! Nothing here keeps two daemons apart. Two daemons on one data directory
//! would each run an ingest worker, and the pipeline's dedup gate is only
//! correct with a single sequential consumer of the channel, but that is a
//! property of the *store*, not of the daemon, so the exclusion lives where the
//! store is opened: `clawg_core::memory::store_lock`, taken inside
//! `MemoryStore::open`. It covers a second daemon and an evaluation run alike.
//! (Older builds kept a `daemon.lock` here; the file may still sit on disk and
//! is inert.)

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use clawg_core::paths::{daemon_state_file, token_file};
use serde::{Deserialize, Serialize};

/// Number of random bytes behind the bearer token; rendered as twice as many
/// hex characters. 32 bytes is 256 bits — far past anything guessable, and the
/// cost is one read from the OS CSPRNG once per daemon start.
const TOKEN_BYTES: usize = 32;

/// What a running `clawg serve` publishes about itself.
///
/// `port` is the real bound port, never the requested one: `--port 0` asks the
/// OS to choose, and a client that read the request would dial port 0.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonState {
    pub pid: u32,
    pub port: u16,
    pub profile: String,
    pub version: String,
    pub started_at_ms: u64,
}

/// Failure in the lifecycle files.
#[derive(Debug, thiserror::Error)]
pub enum LifecycleError {
    #[error("lifecycle file error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not serialize the daemon state: {0}")]
    Serialize(#[from] serde_json::Error),
}

fn io_err(path: &Path) -> impl Fn(std::io::Error) -> LifecycleError + '_ {
    move |source| LifecycleError::Io { path: path.to_path_buf(), source }
}

/// Milliseconds since the Unix epoch, saturating at 0 on a clock before it.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Fresh bearer token: [`TOKEN_BYTES`] CSPRNG bytes, lowercase hex.
pub fn generate_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; TOKEN_BYTES];
    rand::rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Tightens `path` to owner-only (`0600`) on Unix.
///
/// A no-op on Windows, and deliberately so rather than an error: the file lives
/// under the user's home directory, whose ACL already excludes other
/// non-administrator users by default, and NTFS has no mode bits to set. An
/// administrator can read it either way — on Unix too, where root ignores 0600
/// — so the platforms end up with the same practical guarantee by different
/// means.
#[cfg(unix)]
fn restrict_to_owner(path: &Path) -> Result<(), LifecycleError> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(io_err(path))
}

#[cfg(not(unix))]
fn restrict_to_owner(_path: &Path) -> Result<(), LifecycleError> {
    Ok(())
}

/// Writes `<base_dir>/daemon.json`.
pub fn write_state(base_dir: &Path, state: &DaemonState) -> Result<(), LifecycleError> {
    let path = daemon_state_file(base_dir);
    let json = serde_json::to_string_pretty(state)?;
    let mut f = File::create(&path).map_err(io_err(&path))?;
    f.write_all(json.as_bytes()).map_err(io_err(&path))?;
    // 0600 on the state file too, even though it holds no secret: it names a
    // pid and a port another local user has no business enumerating.
    restrict_to_owner(&path)
}

/// Reads `<base_dir>/daemon.json`, or `None` when it is missing or unparseable.
///
/// Unparseable collapses into `None` on purpose. Every caller's question is
/// "can I talk to a daemon?", and a corrupt record answers that with no just
/// as firmly as a missing one; the only difference an error could make is
/// noise on a path that must stay quiet.
pub fn read_state(base_dir: &Path) -> Option<DaemonState> {
    let raw = std::fs::read_to_string(daemon_state_file(base_dir)).ok()?;
    serde_json::from_str(&raw).ok()
}

/// Writes `<base_dir>/token` with owner-only permissions.
pub fn write_token(base_dir: &Path, token: &str) -> Result<(), LifecycleError> {
    let path = token_file(base_dir);
    let mut f = File::create(&path).map_err(io_err(&path))?;
    f.write_all(token.as_bytes()).map_err(io_err(&path))?;
    restrict_to_owner(&path)
}

/// Reads `<base_dir>/token`, trimmed. `None` when missing or empty.
///
/// Trimming matters: an editor or a shell redirect can leave a trailing
/// newline, and a token compared with one would never match while looking
/// byte-for-byte correct in a terminal.
pub fn read_token(base_dir: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(token_file(base_dir)).ok()?;
    let trimmed = raw.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// Removes `daemon.json` and `token`. Missing files are not an error — this
/// runs on the shutdown path, where the only wrong outcome is refusing to exit.
pub fn clear_state(base_dir: &Path) {
    let _ = std::fs::remove_file(daemon_state_file(base_dir));
    let _ = std::fs::remove_file(token_file(base_dir));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    fn sample_state() -> DaemonState {
        DaemonState {
            pid: 4242,
            port: 27381,
            profile: "dev".to_string(),
            version: "0.1.0".to_string(),
            started_at_ms: 1_700_000_000_000,
        }
    }

    #[test]
    fn state_round_trips_through_the_file() {
        let dir = tmp();
        let state = sample_state();
        write_state(dir.path(), &state).expect("write");
        assert_eq!(read_state(dir.path()), Some(state));
    }

    #[test]
    fn state_is_written_to_daemon_json_under_base_dir() {
        let dir = tmp();
        write_state(dir.path(), &sample_state()).expect("write");
        assert!(dir.path().join("daemon.json").exists());
    }

    #[test]
    fn reading_state_from_an_empty_dir_is_none() {
        let dir = tmp();
        assert_eq!(read_state(dir.path()), None);
    }

    /// A truncated or half-written record must read as "no daemon", not blow up
    /// a status command — the question being asked is only "can I connect?".
    #[test]
    fn corrupt_state_reads_as_none() {
        let dir = tmp();
        std::fs::write(dir.path().join("daemon.json"), b"{not json").expect("write");
        assert_eq!(read_state(dir.path()), None);
    }

    #[test]
    fn token_is_64_lowercase_hex_chars() {
        let token = generate_token();
        assert_eq!(token.len(), 64, "32 bytes must render as 64 hex chars");
        assert!(
            token.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
            "token was: {token}"
        );
    }

    #[test]
    fn two_generated_tokens_differ() {
        assert_ne!(generate_token(), generate_token());
    }

    #[test]
    fn token_round_trips_through_the_file() {
        let dir = tmp();
        let token = generate_token();
        write_token(dir.path(), &token).expect("write");
        assert_eq!(read_token(dir.path()), Some(token));
    }

    /// A trailing newline is what a human editing the file by hand leaves
    /// behind; comparing it verbatim would reject a token that looks right.
    #[test]
    fn token_read_trims_trailing_whitespace() {
        let dir = tmp();
        std::fs::write(dir.path().join("token"), b"abc123\n").expect("write");
        assert_eq!(read_token(dir.path()), Some("abc123".to_string()));
    }

    #[test]
    fn an_empty_token_file_reads_as_none() {
        let dir = tmp();
        std::fs::write(dir.path().join("token"), b"  \n").expect("write");
        assert_eq!(read_token(dir.path()), None);
    }

    #[test]
    fn missing_token_reads_as_none() {
        assert_eq!(read_token(tmp().path()), None);
    }

    #[test]
    fn clear_state_removes_both_files_and_tolerates_absence() {
        let dir = tmp();
        write_state(dir.path(), &sample_state()).expect("write state");
        write_token(dir.path(), "deadbeef").expect("write token");

        clear_state(dir.path());
        assert!(!dir.path().join("daemon.json").exists());
        assert!(!dir.path().join("token").exists());

        clear_state(dir.path()); // idempotent: shutdown must never fail here
    }

    #[cfg(unix)]
    #[test]
    fn token_file_is_owner_only_on_unix() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tmp();
        write_token(dir.path(), "deadbeef").expect("write");
        let mode = std::fs::metadata(dir.path().join("token"))
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "token must not be group/world readable");
    }
}
