//! `logs/serve.log`, where a daemon clawg starts (a bridge's autostart,
//! `update`) writes its stdout and stderr.
//!
//! It is rotated when it is opened, never while a daemon writes to it: the
//! daemon only inherits the file, so its panics and whatever ONNX Runtime
//! prints land there too, and no writer of its own could rotate those. A
//! daemon that runs for weeks may go past the limit; the next start cuts it.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Past this size, the next start moves `serve.log` aside.
pub const SERVE_LOG_LIMIT: u64 = 10 * 1024 * 1024;

/// `serve.log`, `serve.log.1` and `serve.log.2`.
const FILES: usize = 3;

/// Opens `log` to append to, rotated first when it is over
/// [`SERVE_LOG_LIMIT`].
pub fn open(log: &Path) -> std::io::Result<File> {
    open_with(log, SERVE_LOG_LIMIT, FILES)
}

/// A rotation that fails does not keep the daemon from starting: the file is
/// appended to as it is, and its first new line says why.
fn open_with(log: &Path, limit: u64, files: usize) -> std::io::Result<File> {
    let rotated = rotate(log, limit, files);
    let mut file = OpenOptions::new().create(true).append(true).open(log)?;
    if let Err(e) = rotated {
        let _ = writeln!(file, "clawg: {} was not rotated: {e}", log.display());
    }
    Ok(file)
}

/// Renaming, not copying, so a daemon still writing to the file (one that
/// lost the race for the store) goes on in the renamed one: on Unix it keeps
/// the inode, and on Windows std opens files with `FILE_SHARE_DELETE`.
fn rotate(log: &Path, limit: u64, files: usize) -> std::io::Result<()> {
    let len = match std::fs::metadata(log) {
        Ok(meta) => Some(meta.len()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e),
    };
    for (from, to) in rotation(log, len, limit, files) {
        match std::fs::rename(&from, &to) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
    }
    Ok(())
}

/// The renames, in order, that make room for a new `log` of `len` bytes
/// (`None` when there is none): none up to `limit`, else each file moves one
/// place and the last one is replaced.
fn rotation(log: &Path, len: Option<u64>, limit: u64, files: usize) -> Vec<(PathBuf, PathBuf)> {
    if len.is_none_or(|len| len <= limit) {
        return Vec::new();
    }
    let mut renames: Vec<_> = (1..files.saturating_sub(1)).rev().map(|n| (numbered(log, n), numbered(log, n + 1))).collect();
    renames.push((log.to_path_buf(), numbered(log, 1)));
    renames
}

/// `serve.log.N`.
fn numbered(log: &Path, n: usize) -> PathBuf {
    let mut name = log.as_os_str().to_owned();
    name.push(format!(".{n}"));
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_rotates_up_to_the_limit() {
        let log = Path::new("logs/serve.log");
        assert!(rotation(log, None, 10, 3).is_empty());
        assert!(rotation(log, Some(0), 10, 3).is_empty());
        assert!(rotation(log, Some(10), 10, 3).is_empty());
    }

    #[test]
    fn over_the_limit_each_file_moves_one_place() {
        let log = Path::new("logs/serve.log");
        assert_eq!(
            rotation(log, Some(11), 10, 3),
            vec![(numbered(log, 1), numbered(log, 2)), (log.to_path_buf(), numbered(log, 1))]
        );
        assert_eq!(rotation(log, Some(11), 10, 2), vec![(log.to_path_buf(), numbered(log, 1))]);
    }

    #[test]
    fn rotating_keeps_three_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let log = dir.path().join("serve.log");
        std::fs::write(&log, "newest run").expect("log");
        std::fs::write(numbered(&log, 1), "older run").expect("log.1");
        std::fs::write(numbered(&log, 2), "oldest run").expect("log.2");

        let mut file = open_with(&log, 4, 3).expect("open");
        writeln!(file, "next run").expect("write");

        assert_eq!(std::fs::read_to_string(&log).expect("log"), "next run\n");
        assert_eq!(std::fs::read_to_string(numbered(&log, 1)).expect("log.1"), "newest run");
        assert_eq!(std::fs::read_to_string(numbered(&log, 2)).expect("log.2"), "older run");
        assert!(!numbered(&log, 3).exists());
    }

    #[test]
    fn a_log_under_the_limit_is_appended_to() {
        let dir = tempfile::tempdir().expect("tempdir");
        let log = dir.path().join("serve.log");
        std::fs::write(&log, "run\n").expect("log");

        let mut file = open_with(&log, 4, 3).expect("open");
        writeln!(file, "next run").expect("write");

        assert_eq!(std::fs::read_to_string(&log).expect("log"), "run\nnext run\n");
        assert!(!numbered(&log, 1).exists());
    }

    /// Two bridges that autostart at once: the one that loses the store lock
    /// may still be writing when the other rotates.
    #[test]
    fn a_file_open_for_writing_still_rotates() {
        let dir = tempfile::tempdir().expect("tempdir");
        let log = dir.path().join("serve.log");
        let mut writer = OpenOptions::new().create(true).append(true).open(&log).expect("writer");
        writeln!(writer, "first daemon").expect("write");

        let _second = open_with(&log, 4, 3).expect("open");
        writeln!(writer, "first daemon, later").expect("write");

        assert_eq!(
            std::fs::read_to_string(numbered(&log, 1)).expect("log.1"),
            "first daemon\nfirst daemon, later\n"
        );
        assert_eq!(std::fs::read_to_string(&log).expect("log"), "");
    }

    #[test]
    fn a_failed_rotation_is_written_to_the_log() {
        let dir = tempfile::tempdir().expect("tempdir");
        let log = dir.path().join("serve.log");
        std::fs::write(&log, "full run\n").expect("log");
        std::fs::write(numbered(&log, 1), "older run").expect("log.1");
        // A directory with something in it cannot be replaced by a file, on
        // Unix or on Windows.
        std::fs::create_dir(numbered(&log, 2)).expect("log.2 dir");
        std::fs::write(numbered(&log, 2).join("keep"), "").expect("keep");

        drop(open_with(&log, 4, 3).expect("the log opens anyway"));

        let text = std::fs::read_to_string(&log).expect("log");
        assert!(text.starts_with("full run\n"), "{text}");
        assert!(text.contains("was not rotated"), "{text}");
        assert_eq!(std::fs::read_to_string(numbered(&log, 1)).expect("log.1"), "older run");
    }
}
