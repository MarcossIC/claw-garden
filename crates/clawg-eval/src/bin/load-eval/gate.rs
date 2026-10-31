//! The directory a gate run downloads the model into, and the file it writes.

use std::path::{Path, PathBuf};

const PREFIX: &str = "load-eval-";

/// A directory `load-eval-<uuid>` made directly under the system temp dir,
/// removed when this is dropped: the only place a gate run reads or downloads
/// the model, so it can never touch a data profile.
pub struct ModelDir(PathBuf);

fn is_gate_dir_name(name: &str) -> bool {
    name.strip_prefix(PREFIX).is_some_and(|id| id.len() == 36 && uuid::Uuid::parse_str(id).is_ok())
}

impl ModelDir {
    /// `dir` when it is a `load-eval-<uuid>` directly under `temp` (made if
    /// missing), else a new one there.
    pub fn claim(dir: Option<PathBuf>, temp: &Path) -> Result<Self, String> {
        let Some(dir) = dir else {
            let made = temp.join(format!("{PREFIX}{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&made).map_err(|e| format!("cannot create {}: {e}", made.display()))?;
            return Ok(ModelDir(made));
        };
        let refused = || {
            format!(
                "--model-dir must be a {PREFIX}<uuid> directory directly under {}; {} is not, and a data profile is never read",
                temp.display(),
                dir.display()
            )
        };
        let named = dir.file_name().and_then(|n| n.to_str()).is_some_and(is_gate_dir_name);
        let parent = dir.parent().and_then(|p| p.canonicalize().ok());
        if !named || parent.is_none() || parent != temp.canonicalize().ok() {
            return Err(refused());
        }
        if std::fs::symlink_metadata(&dir).is_ok_and(|m| !m.is_dir()) {
            return Err(refused());
        }
        std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        Ok(ModelDir(dir))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for ModelDir {
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_dir_all(&self.0) {
            eprintln!("warning: could not remove {}: {e}", self.0.display());
        }
    }
}

/// Runs `run` over the model directory and removes it afterwards, whether
/// `run` fails, panics or finishes.
pub fn with_model_dir<T>(
    dir: Option<PathBuf>,
    temp: &Path,
    run: impl FnOnce(&Path) -> Result<T, String>,
) -> Result<T, String> {
    let dir = ModelDir::claim(dir, temp)?;
    run(dir.path())
}

pub fn is_slug(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

pub fn results_filename(date: &str, tag: Option<&str>) -> String {
    match tag {
        Some(tag) => format!("{date}-real-load-eval-{tag}.json"),
        None => format!("{date}-real-load-eval.json"),
    }
}

/// The directory the dated results go to, outside git.
pub fn results_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/eval/results")
}

/// Writes `text` at `path` unless a file is already there: a rerun must not
/// replace the baseline it is compared with.
pub fn write_new(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(path)?;
    writeln!(file, "{text}")
}

/// Howard Hinnant's `civil_from_days`: this binary has no date crate for one stamp.
fn civil_from_days(days_since_epoch: i64) -> (i64, u32, u32) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// The current UTC date as `YYYY-MM-DD`.
pub fn current_date() -> String {
    let seconds = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
    let (y, m, d) = civil_from_days((seconds / 86_400) as i64);
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name() -> String {
        format!("load-eval-{}", uuid::Uuid::new_v4())
    }

    #[test]
    fn a_dir_that_is_not_a_load_eval_dir_of_the_temp_is_refused_and_left_alone() {
        let temp = tempfile::tempdir().unwrap();
        let profile = temp.path().join(".clawg-dev");
        std::fs::create_dir(&profile).unwrap();
        std::fs::write(profile.join("memory.db"), b"rows").unwrap();
        let nested = temp.path().join("other");
        std::fs::create_dir(&nested).unwrap();
        let elsewhere = nested.join(name());
        let bad_uuid = temp.path().join("load-eval-not-a-uuid");

        for dir in [profile.clone(), elsewhere, bad_uuid, temp.path().to_path_buf()] {
            let refused = ModelDir::claim(Some(dir.clone()), temp.path());
            assert!(refused.is_err(), "{} must be refused", dir.display());
        }
        assert!(profile.join("memory.db").exists(), "a refused dir is never touched");
    }

    #[test]
    fn a_load_eval_dir_of_the_temp_is_accepted_and_one_is_made_without_a_dir() {
        let temp = tempfile::tempdir().unwrap();
        let given = temp.path().join(name());
        let claimed = ModelDir::claim(Some(given.clone()), temp.path()).unwrap();
        assert_eq!(claimed.path(), given);
        assert!(given.is_dir());
        drop(claimed);

        let made = ModelDir::claim(None, temp.path()).unwrap();
        assert!(made.path().is_dir());
        assert_eq!(made.path().parent(), Some(temp.path()));
    }

    #[test]
    fn the_dir_is_removed_when_the_run_fails() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join(name());
        let outcome: Result<(), String> = with_model_dir(Some(dir.clone()), temp.path(), |at| {
            std::fs::write(at.join("partial"), b"half a model").unwrap();
            Err("the mirror does not answer".into())
        });
        assert_eq!(outcome.unwrap_err(), "the mirror does not answer");
        assert!(!dir.exists());
    }

    #[test]
    fn the_dir_is_removed_when_the_run_succeeds_and_when_it_panics() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join(name());
        let value = with_model_dir(Some(dir.clone()), temp.path(), |_| Ok::<_, String>(7)).unwrap();
        assert_eq!(value, 7);
        assert!(!dir.exists());

        let dir = temp.path().join(name());
        let unwound = std::panic::catch_unwind(|| {
            let _ = with_model_dir::<()>(Some(dir.clone()), temp.path(), |_| panic!("a load blew up"));
        });
        assert!(unwound.is_err());
        assert!(!dir.exists());
    }

    #[test]
    fn a_refused_dir_never_reaches_the_run() {
        let temp = tempfile::tempdir().unwrap();
        let mut ran = false;
        let outcome: Result<(), String> = with_model_dir(Some(temp.path().join("profile")), temp.path(), |_| {
            ran = true;
            Ok(())
        });
        assert!(outcome.is_err());
        assert!(!ran);
    }

    #[test]
    fn a_tag_is_a_lowercase_slug() {
        assert!(is_slug("again"));
        assert!(is_slug("run-2"));
        assert!(!is_slug(""));
        assert!(!is_slug("Again"));
        assert!(!is_slug("../x"));
    }

    #[test]
    fn the_result_never_replaces_another_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(results_filename("2026-10-29", None));
        assert!(path.ends_with("2026-10-29-real-load-eval.json"));
        assert!(results_filename("2026-10-29", Some("b")).ends_with("2026-10-29-real-load-eval-b.json"));
        write_new(&path, "{}").unwrap();
        assert!(write_new(&path, "{\"other\":1}").is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap().trim(), "{}");
    }
}
