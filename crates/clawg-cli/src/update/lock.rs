//! Keeps two `clawg update` of one profile from swapping at the same time.

use std::fs::File;
use std::path::{Path, PathBuf};

use clawg_core::paths::update_lock_file;
use clawg_core::private_dir::ensure_private_dir;

#[derive(Debug, thiserror::Error)]
pub enum LockError {
    #[error("another clawg update is running for this profile")]
    Busy,
    #[error("cannot take the update lock {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// An exclusive OS lock on `<data>/update.lock`, held while the value lives
/// and released by the OS if the process dies.
#[derive(Debug)]
pub struct UpdateLock {
    // Never read: dropping the file is what releases the lock.
    _file: File,
}

impl UpdateLock {
    pub fn acquire(base_dir: &Path) -> Result<Self, LockError> {
        let path = update_lock_file(base_dir);
        let io = |source| LockError::Io { path: path.clone(), source };
        ensure_private_dir(base_dir).map_err(io)?;
        let file = File::options().create(true).read(true).write(true).truncate(false).open(&path).map_err(io)?;
        // Fully qualified: `File` has its own `try_lock*` methods with other
        // semantics, as `StoreLock` notes.
        match fs2::FileExt::try_lock_exclusive(&file) {
            Ok(()) => Ok(Self { _file: file }),
            // Compared by OS code, the portable spelling; `ErrorKind` of a
            // contended lock on Windows is `Uncategorized`.
            Err(e) if e.raw_os_error() == fs2::lock_contended_error().raw_os_error() => Err(LockError::Busy),
            Err(source) => Err(io(source)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_updates_do_not_swap_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let first = UpdateLock::acquire(dir.path()).unwrap();

        let second = UpdateLock::acquire(dir.path()).unwrap_err();

        assert!(matches!(second, LockError::Busy), "{second:?}");
        assert!(second.to_string().contains("another clawg update is running"));
        drop(first);
        UpdateLock::acquire(dir.path()).expect("the lock is free once the first update ends");
    }

    #[test]
    fn the_lock_is_per_data_directory() {
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let _first = UpdateLock::acquire(a.path()).unwrap();

        UpdateLock::acquire(b.path()).expect("another profile's update is not blocked");
    }

    #[test]
    fn the_data_directory_is_created_when_missing() {
        let dir = tempfile::tempdir().unwrap();

        UpdateLock::acquire(&dir.path().join("never-created")).expect("created on the way");
    }

    #[cfg(unix)]
    #[test]
    fn the_update_lock_makes_the_data_directory_private() {
        use std::os::unix::fs::PermissionsExt;
        let home = tempfile::tempdir().unwrap();
        let (fresh, open) = (home.path().join(".clawg-dev"), home.path().join(".clawg"));
        std::fs::create_dir(&open).unwrap();
        std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o755)).unwrap();

        for base in [&fresh, &open] {
            drop(UpdateLock::acquire(base).unwrap());
            let mode = std::fs::metadata(base).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700, "{}", base.display());
        }
    }
}
