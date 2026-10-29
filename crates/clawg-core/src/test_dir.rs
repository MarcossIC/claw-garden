//! A temporary directory for a test that opens a store in it.

use std::path::{Path, PathBuf};

/// Set to keep every test directory on disk, each named on stderr as it is kept.
const KEEP_VAR: &str = "CLAWG_KEEP_TEST_FILES";

/// A temporary directory removed when the test that took it ends, passed or
/// failed.
pub(crate) struct TestDir {
    dir: Option<tempfile::TempDir>,
    keep: bool,
    /// Removals tried, 50 ms apart, before the test is failed.
    attempts: u32,
}

/// A fresh, empty directory under the system temp dir.
pub(crate) fn test_dir() -> TestDir {
    TestDir::new(std::env::var_os(KEEP_VAR).is_some())
}

impl TestDir {
    fn new(keep: bool) -> Self {
        TestDir { dir: Some(tempfile::tempdir().expect("a temp dir")), keep, attempts: 100 }
    }

    pub(crate) fn path(&self) -> &Path {
        self.dir.as_ref().expect("a test dir lives until it is dropped").path()
    }
}

impl AsRef<Path> for TestDir {
    fn as_ref(&self) -> &Path {
        self.path()
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let Some(dir) = self.dir.take() else { return };
        let path: PathBuf = dir.keep();
        if self.keep {
            eprintln!("kept {}", path.display());
            return;
        }
        // A store dropped at the end of a test only signals its connection's
        // thread, which closes the database a moment later, and on Windows an
        // open file cannot be deleted: `TempDir` gives up at once, in silence.
        let mut last = None;
        for _ in 0..self.attempts {
            match std::fs::remove_dir_all(&path) {
                Ok(()) => return,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
                Err(e) => last = Some(e),
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let Some(e) = last else { return };
        let held = format!(
            "the test directory {} is still held: close the store before the test ends ({e})",
            path.display()
        );
        // A second panic while a failed test unwinds would abort the run.
        if std::thread::panicking() {
            eprintln!("warning: {held}");
        } else {
            panic!("{held}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::db::MemoryStore;
    use crate::memory::store_lock::StoreHolder;

    #[tokio::test]
    async fn a_test_dir_waits_for_its_store_to_let_the_files_go() {
        let dir = TestDir::new(false);
        let path = dir.path().to_path_buf();
        let store = MemoryStore::open(dir.path(), StoreHolder::Eval).await.unwrap();

        drop(store);
        drop(dir);

        assert!(!path.exists(), "the store's files are removed once its connection lets them go");
    }

    #[cfg(windows)]
    #[test]
    fn a_test_dir_still_held_fails_its_test() {
        let mut dir = TestDir::new(false);
        dir.attempts = 2;
        let path = dir.path().to_path_buf();

        // SQLite opens its files without letting them be deleted while open, as
        // a store does; a file std opens can be.
        let held = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let conn = rusqlite::Connection::open(dir.path().join("memory.db")).unwrap();
            conn.execute_batch("CREATE TABLE t (x INTEGER)").unwrap();
            drop(dir);
            drop(conn);
        }));

        assert!(held.is_err(), "a store still open when its test ends fails the test");
        std::fs::remove_dir_all(&path).unwrap();
    }

    #[test]
    fn a_kept_test_dir_stays() {
        let dir = TestDir::new(true);
        let path = dir.path().to_path_buf();

        drop(dir);

        assert!(path.exists(), "asked for, the directory stays to be read");
        std::fs::remove_dir_all(&path).unwrap();
    }
}
