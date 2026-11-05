//! The record of `clawg hook session-start`: one JSON line per run in
//! `<data>/hook-runs.jsonl`, so that a session that starts without memory
//! leaves evidence of whether the hook ran, what the client sent and why
//! nothing was injected.
//!
//! The client runs the hook and reads its stdout, so nothing here may
//! disturb it: every failure is swallowed. A line holds the names of the
//! input's fields and never their values, and the data directory is never
//! created for it.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio::time::Instant;

/// The record, in the data directory.
pub const FILE: &str = "hook-runs.jsonl";
/// What the record becomes when it reaches [`ROLL_AT`].
pub const ROLLED: &str = "hook-runs.1.jsonl";
/// The size, in bytes, at which the record is renamed and a new one starts.
pub const ROLL_AT: u64 = 64 * 1024;

// What a line keeps of each thing the client or the command line says, in
// characters: a line stays small however large the input is.
pub const SOURCE_CHARS: usize = 32;
pub const FOLDER_CHARS: usize = 1024;
pub const VERSION_CHARS: usize = 32;
pub const AGENT_CHARS: usize = 64;
pub const KEYS_KEPT: usize = 32;
pub const KEY_CHARS: usize = 64;

/// The first `chars` characters of `text`, never splitting one.
pub fn cut(text: &str, chars: usize) -> String {
    text.chars().take(chars).collect()
}

/// What happened to the memory in a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// A context with text was printed.
    Injected,
    /// The daemon answered and the context has no text; the hook may still
    /// have printed a language line or a nudge.
    EmptyContext,
    /// Cursor sent more than one workspace root.
    SeveralFolders,
    NoFolder,
    NoDaemon,
    /// The daemon refused the folder's project.
    ProjectRefused,
    DaemonError,
    TimedOut,
    BadInput,
    /// A hook installed for Claude Code that Cursor runs.
    OtherClient,
}

/// One run of `session-start`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    pub at_ms: i64,
    pub client: String,
    pub agent: String,
    pub source: Option<String>,
    pub outcome: Outcome,
    /// What was printed to stdout.
    pub bytes: usize,
    pub elapsed_ms: u64,
    /// How many workspace roots the client sent, when it sent them.
    pub roots: Option<usize>,
    /// The folder whose project was read, exactly as it arrived.
    pub folder: Option<String>,
    pub cursor_version: Option<String>,
    /// The names of the input's top-level fields.
    pub input_keys: Vec<String>,
}

/// The line a run is written as, with its newline, built whole so that it is
/// appended in one write.
pub fn line(run: &Run) -> String {
    let mut text = serde_json::to_string(run).expect("a run serializes");
    text.push('\n');
    text
}

/// The names of the top-level fields of `input`; none when it is not a JSON
/// object.
pub fn input_keys(input: &[u8]) -> Vec<String> {
    match serde_json::from_slice::<serde_json::Value>(input) {
        Ok(serde_json::Value::Object(fields)) => fields.keys().take(KEYS_KEPT).map(|k| cut(k, KEY_CHARS)).collect(),
        _ => Vec::new(),
    }
}

/// The runs in the bytes of a record, oldest first. A line that is not a run is
/// skipped, and so is one with a byte that is not UTF-8: read leniently it could
/// still be a run, with a folder other than the one written.
pub fn parse_runs(bytes: &[u8]) -> Vec<Run> {
    bytes
        .split(|byte| *byte == b'\n')
        .filter_map(|line| std::str::from_utf8(line).ok())
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

/// The runs of a data directory, oldest first: the rolled file, then the
/// current one. Missing or unreadable files read as empty, and a damaged byte
/// costs the line it is in, not the file.
pub fn read_runs(dir: &Path) -> Vec<Run> {
    [ROLLED, FILE].iter().filter_map(|name| std::fs::read(dir.join(name)).ok()).flat_map(|bytes| parse_runs(&bytes)).collect()
}

/// Appends `run` to the record of `dir`. Any failure is swallowed, and a
/// directory that does not exist is left that way.
pub fn record(dir: &Path, run: &Run) {
    let _ = append(dir, &line(run));
}

/// [`record`], waited for until `deadline` and no later. The write runs on a
/// thread of its own, so a disk that holds it up cannot hold up the answer, and
/// what is left of it ends with the process.
pub async fn record_by(deadline: Instant, dir: PathBuf, run: Run) {
    wait_until(deadline, move || record(&dir, &run)).await;
}

async fn wait_until(deadline: Instant, write: impl FnOnce() + Send + 'static) {
    let _ = tokio::time::timeout_at(deadline, tokio::task::spawn_blocking(write)).await;
}

fn append(dir: &Path, line: &str) -> std::io::Result<()> {
    let path = dir.join(FILE);
    // Renaming instead of rewriting: two hooks that start together never
    // lose a line to a copy of the file. A roll that fails costs nothing: the
    // run is still appended and the next one tries again.
    if std::fs::metadata(&path).is_ok_and(|m| m.len() >= ROLL_AT) {
        let _ = std::fs::rename(&path, dir.join(ROLLED));
    }
    std::fs::OpenOptions::new().append(true).create(true).open(path)?.write_all(line.as_bytes())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn run(outcome: Outcome) -> Run {
        Run {
            at_ms: 1_762_250_000_000,
            client: "cursor".into(),
            agent: "cursor".into(),
            source: Some("startup".into()),
            outcome,
            bytes: 3_200,
            elapsed_ms: 41,
            roots: Some(1),
            folder: Some("/c:/work/repo".into()),
            cursor_version: Some("3.28.0".into()),
            input_keys: vec!["conversation_id".into(), "workspace_roots".into()],
        }
    }

    #[test]
    fn a_run_is_one_json_line_with_every_field() {
        let text = line(&run(Outcome::Injected));
        assert!(text.ends_with('\n') && text.matches('\n').count() == 1, "{text:?}");
        let value: serde_json::Value = serde_json::from_str(text.trim_end()).unwrap();
        let keys: Vec<&str> = value.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            [
                "at_ms", "client", "agent", "source", "outcome", "bytes", "elapsed_ms", "roots", "folder",
                "cursor_version", "input_keys"
            ]
        );
        assert_eq!(value["outcome"], "injected");
        assert_eq!(value["folder"], "/c:/work/repo", "the folder is written as it arrived");
    }

    #[test]
    fn every_outcome_is_written_in_snake_case() {
        let named = [
            (Outcome::Injected, "injected"),
            (Outcome::EmptyContext, "empty_context"),
            (Outcome::SeveralFolders, "several_folders"),
            (Outcome::NoFolder, "no_folder"),
            (Outcome::NoDaemon, "no_daemon"),
            (Outcome::ProjectRefused, "project_refused"),
            (Outcome::DaemonError, "daemon_error"),
            (Outcome::TimedOut, "timed_out"),
            (Outcome::BadInput, "bad_input"),
            (Outcome::OtherClient, "other_client"),
        ];
        for (outcome, name) in named {
            assert_eq!(serde_json::to_value(outcome).unwrap(), name);
        }
    }

    #[test]
    fn input_keys_are_the_names_of_the_top_level_fields() {
        let keys = input_keys(br#"{"user_email":"a@b.c","workspace_roots":["/x"],"model":{"id":"m"}}"#);
        assert_eq!(keys, ["user_email", "workspace_roots", "model"]);
        assert_eq!(input_keys(b"[1,2]"), Vec::<String>::new());
        assert_eq!(input_keys(b"not json"), Vec::<String>::new());
    }

    #[test]
    fn runs_are_read_back_and_a_line_that_is_not_a_run_is_skipped() {
        let (a, b) = (run(Outcome::Injected), run(Outcome::NoDaemon));
        let text = format!("{}garbage\n\n{{\"outcome\":\"unheard_of\"}}\n{}", line(&a), line(&b));
        assert_eq!(parse_runs(text.as_bytes()), [a, b]);
    }

    #[test]
    fn a_line_is_appended() {
        let dir = tempfile::tempdir().unwrap();
        record(dir.path(), &run(Outcome::Injected));
        record(dir.path(), &run(Outcome::NoDaemon));
        let runs = read_runs(dir.path());
        assert_eq!(runs.iter().map(|r| r.outcome).collect::<Vec<_>>(), [Outcome::Injected, Outcome::NoDaemon]);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1, "only the record");
    }

    #[test]
    fn the_file_rolls_over_at_64_kib_and_replaces_the_old_one() {
        let dir = tempfile::tempdir().unwrap();
        let current = dir.path().join(FILE);
        let rolled = dir.path().join(ROLLED);

        std::fs::write(&current, vec![b'a'; ROLL_AT as usize - 1]).unwrap();
        record(dir.path(), &run(Outcome::Injected));
        assert!(!rolled.exists(), "one byte under the limit keeps the file");

        std::fs::write(&current, vec![b'b'; ROLL_AT as usize]).unwrap();
        std::fs::write(&rolled, b"older").unwrap();
        record(dir.path(), &run(Outcome::NoDaemon));
        assert_eq!(std::fs::read(&rolled).unwrap(), vec![b'b'; ROLL_AT as usize], "the old rolled file is replaced");
        assert_eq!(parse_runs(&std::fs::read(&current).unwrap()).len(), 1, "a new file starts");
    }

    #[test]
    fn concurrent_appends_keep_every_line() {
        let dir = tempfile::tempdir().unwrap();
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| (0..20).for_each(|_| record(dir.path(), &run(Outcome::Injected))));
            }
        });
        assert_eq!(read_runs(dir.path()).len(), 160);
    }

    #[test]
    fn a_missing_data_directory_is_not_created() {
        let parent = tempfile::tempdir().unwrap();
        let missing = parent.path().join(".clawg");
        record(&missing, &run(Outcome::Injected));
        assert!(!missing.exists());
        assert_eq!(std::fs::read_dir(parent.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn a_write_in_time_is_done_when_the_wait_ends() {
        let dir = tempfile::tempdir().unwrap();
        let started = Instant::now();
        record_by(started + Duration::from_secs(30), dir.path().to_path_buf(), run(Outcome::Injected)).await;
        assert_eq!(read_runs(dir.path()).len(), 1, "written by the time the hook prints");
        assert!(started.elapsed() < Duration::from_secs(20), "the rest of the time is not spent");
    }

    #[tokio::test]
    async fn a_write_held_up_is_not_waited_for_past_the_deadline() {
        let (release, held) = std::sync::mpsc::channel::<()>();
        let deadline = Duration::from_millis(100);
        let started = Instant::now();
        // Bounded so that a wait with no limit fails this test and does not hang it.
        wait_until(started + deadline, move || {
            let _ = held.recv_timeout(Duration::from_secs(10));
        })
        .await;
        let took = started.elapsed();
        drop(release);
        assert!(took >= deadline && took < Duration::from_secs(2), "the deadline ends the wait, after {took:?}");
    }

    #[test]
    fn cutting_stops_on_a_character_boundary() {
        assert_eq!(cut("abcdef", 3), "abc");
        assert_eq!(cut("abc", 3), "abc");
        assert_eq!(cut("ééé", 2), "éé", "a character is never split");
        assert_eq!(cut("日本語", 1), "日");
        assert_eq!(cut("", 4), "");
    }

    #[test]
    fn input_keys_keep_the_first_32_names_cut_at_64_characters() {
        let fields: Vec<String> = (0..100).map(|i| format!("\"{}{i}\":1", "k".repeat(200))).collect();
        let keys = input_keys(format!("{{{}}}", fields.join(",")).as_bytes());
        assert_eq!(keys.len(), KEYS_KEPT);
        assert!(keys.iter().all(|k| k.chars().count() == KEY_CHARS), "{keys:?}");
        let kept: Vec<String> = (0..40).map(|i| format!("\"f{i}\":1")).collect();
        assert_eq!(input_keys(format!("{{{}}}", kept.join(",")).as_bytes())[..2], ["f0", "f1"], "the first ones, in order");
    }

    #[test]
    fn a_damaged_file_costs_only_its_damaged_lines() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (run(Outcome::Injected), run(Outcome::NoDaemon));
        let mut bytes = line(&a).into_bytes();
        bytes.extend_from_slice(b"half a li\xc3\n");
        // Still JSON once a bad byte reads as U+FFFD: the folder it would show
        // is not the one that was written.
        let mut altered = line(&run(Outcome::DaemonError)).replace("/work/", "/w@rk/").into_bytes();
        *altered.iter_mut().find(|byte| **byte == b'@').unwrap() = 0xff;
        bytes.extend_from_slice(&altered);
        bytes.extend_from_slice(line(&b).as_bytes());
        std::fs::write(dir.path().join(FILE), bytes).unwrap();
        assert_eq!(read_runs(dir.path()), [a, b]);
    }

    #[test]
    fn a_failed_roll_still_records_the_run() {
        let dir = tempfile::tempdir().unwrap();
        let current = dir.path().join(FILE);
        std::fs::write(&current, format!("{}\n", "x".repeat(1023)).repeat(64)).unwrap();
        // A directory with something in it cannot be replaced by a file.
        let rolled = dir.path().join(ROLLED);
        std::fs::create_dir(&rolled).unwrap();
        std::fs::write(rolled.join("keep"), b"1").unwrap();

        record(dir.path(), &run(Outcome::Injected));

        let text = std::fs::read(&current).unwrap();
        assert!(text.len() as u64 > ROLL_AT, "the file was not rolled");
        assert_eq!(parse_runs(&text).len(), 1, "and the run was appended");
    }

    #[test]
    fn a_failed_append_is_swallowed() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(FILE)).unwrap();
        assert!(append(dir.path(), &line(&run(Outcome::Injected))).is_err());
        record(dir.path(), &run(Outcome::Injected));
        assert_eq!(read_runs(dir.path()), []);
    }
}
