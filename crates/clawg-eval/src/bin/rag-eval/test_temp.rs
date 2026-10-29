//! Paths the tests of `rag-eval` write to, removed when the test that took
//! them ends, passed or failed.

use std::path::{Path, PathBuf};
use std::time::Duration;

/// Set to keep every test path on disk, each named on stderr as it is kept.
const KEEP_VAR: &str = "CLAWG_KEEP_TEST_FILES";

/// A file or directory under the system temp dir that a test writes to. It
/// is removed when dropped, also while a failed test unwinds.
pub struct TempPath {
    path: PathBuf,
    keep: bool,
}

/// A fresh path for `tag`, not created.
pub fn temp_path(tag: &str) -> TempPath {
    TempPath { path: fresh(tag), keep: std::env::var_os(KEEP_VAR).is_some() }
}

fn fresh(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("rag-eval-test-{tag}-{}", uuid::Uuid::new_v4()))
}

impl Drop for TempPath {
    fn drop(&mut self) {
        if self.keep {
            if self.path.exists() {
                eprintln!("kept {}", self.path.display());
            }
            return;
        }
        // On Windows a store's connection thread can hold its files for a
        // moment after the store is dropped, and an open file cannot be deleted.
        for _ in 0..40 {
            let removed = match std::fs::symlink_metadata(&self.path) {
                Err(_) => return,
                Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(&self.path),
                Ok(_) => std::fs::remove_file(&self.path),
            };
            if removed.is_ok() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        eprintln!("warning: could not remove the test path {}", self.path.display());
    }
}

impl std::ops::Deref for TempPath {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.path
    }
}

impl AsRef<Path> for TempPath {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn removed_path(tag: &str) -> TempPath {
        TempPath { path: fresh(tag), keep: false }
    }

    #[test]
    fn a_temp_path_is_removed_when_its_test_passes() {
        let dir = removed_path("passes");
        std::fs::create_dir_all(dir.join("inner")).unwrap();
        std::fs::write(dir.join("inner").join("memory.db"), "rows").unwrap();
        let path = dir.to_path_buf();

        drop(dir);

        assert!(!path.exists());
    }

    #[test]
    fn a_temp_path_is_removed_when_its_test_panics() {
        let seen = std::sync::Mutex::new(None);

        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let file = removed_path("panics.json");
            std::fs::write(&file, "{}").unwrap();
            *seen.lock().unwrap() = Some(file.to_path_buf());
            panic!("the test fails before cleaning up");
        }));

        assert!(outcome.is_err());
        let path = seen.into_inner().unwrap().expect("the test wrote its file");
        assert!(!path.exists(), "removed while the failed test unwound");
    }

    #[test]
    fn a_kept_temp_path_stays() {
        let dir = TempPath { path: fresh("kept"), keep: true };
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.to_path_buf();

        drop(dir);

        assert!(path.exists(), "asked for, the path stays to be read");
        std::fs::remove_dir_all(&path).unwrap();
    }
}
