//! Exclusive ownership of a profile's `memory.db`.
//!
//! # Why the store, and not the daemon
//!
//! The ingest pipeline's dedup gate is only correct with ONE sequential
//! consumer of the ingest channel (see `MemoryRuntime::ingest_worker`). Every
//! process that opens the database brings its own consumer, its own
//! maintenance loop and its own embedder bootstrap — so "who may open
//! `memory.db`" is the invariant, and it has to be enforced where the file is
//! opened. An earlier lock lived in the daemon's startup, which excluded a
//! daemon from a daemon and left every other host as a convention; taking it in
//! [`MemoryStore::open`](crate::memory::MemoryStore::open) covers every host
//! (the daemon, an evaluation run) with one mechanism.
//!
//! # First holder wins
//!
//! Nobody evicts anybody. The loser gets [`MemoryError::StoreInUse`] naming the
//! holder, and it is the loser's host that decides how to say so (a stderr line
//! for the daemon).
//!
//! # Two files, on purpose
//!
//! * `memory.lock` — only ever locked, never read or written.
//! * `memory.owner` — a small JSON record the holder writes after acquiring and
//!   that losers read to build their message.
//!
//! They cannot be one file: on Windows an exclusive `fs2` lock is a
//! `LockFileEx` byte-range lock over the whole file, so no other process can
//! READ a locked file — the loser would be unable to read the very record that
//! explains why it lost.
//!
//! # Crashes
//!
//! The OS drops the lock with the process, so a killed holder never blocks the
//! next start. What it leaves behind is a stale `memory.owner`; that is why
//! [`StoreLock::current_owner`] confirms the record against the lock instead of
//! trusting it.

use std::fs::File;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::memory::error::MemoryError;
use crate::paths::{store_lock_file, store_owner_file};

/// Which kind of host holds (or is asking for) the store.
///
/// A closed set rather than a free-form string: losers branch on it (the bridge
/// fails fast for a non-daemon holder), and a typo in a string would silently take the wrong branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoreHolder {
    /// `clawg serve`.
    Daemon,
    /// An evaluation run (`rag-eval`).
    Eval,
    /// `clawg backup` or `clawg restore` on a stopped store.
    Maintenance,
}

impl std::fmt::Display for StoreHolder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            StoreHolder::Daemon => "the clawg daemon",
            StoreHolder::Eval => "an evaluation run",
            StoreHolder::Maintenance => "a clawg backup or restore",
        })
    }
}

/// What a holder publishes about itself in `memory.owner`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreOwner {
    pub holder: StoreHolder,
    pub pid: u32,
    pub started_at_ms: u64,
}

/// Exclusive ownership of `<base_dir>/memory.db`, held for as long as the value
/// lives.
#[derive(Debug)]
pub struct StoreLock {
    // Never read: dropping it is what releases the OS lock. It is a field, not
    // a local, so that `Drop::drop` below runs first (record removed) and the
    // field drop after it (lock released) — see the `Drop` impl.
    _file: File,
    owner_path: PathBuf,
}

/// Milliseconds since the Unix epoch, saturating at 0 on a clock before it.
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Whether `err` from a try-lock means "someone else has it", as opposed to a
/// real I/O failure (permissions, a read-only file system).
///
/// Compared against `fs2`'s own contention error by raw OS code: that is the
/// portable spelling — `ErrorKind` for `ERROR_LOCK_VIOLATION` on Windows is
/// `Uncategorized`, so a kind comparison would never match there.
fn is_contended(err: &std::io::Error) -> bool {
    err.raw_os_error() == fs2::lock_contended_error().raw_os_error()
}

fn read_owner(base_dir: &Path) -> Option<StoreOwner> {
    let raw = std::fs::read_to_string(store_owner_file(base_dir)).ok()?;
    serde_json::from_str(&raw).ok()
}

impl StoreLock {
    /// Takes the lock and writes `memory.owner`, or reports who has it.
    ///
    /// The owner write is best-effort: the lock is the guarantee, the record is
    /// only there to make a refusal readable. A holder that cannot write it
    /// still holds the store, and its losers just say "another process".
    pub fn acquire(base_dir: &Path, holder: StoreHolder) -> Result<Self, MemoryError> {
        let path = store_lock_file(base_dir);
        let file = File::options()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .map_err(|source| MemoryError::StoreLockIo { path: path.clone(), source })?;

        // Fully qualified: `std::fs::File` grew inherent `try_lock*` methods,
        // and an unqualified call would silently resolve to whichever the
        // compiler prefers — with a different return type and different
        // semantics than the one this code was written against.
        match fs2::FileExt::try_lock_exclusive(&file) {
            Ok(()) => {}
            Err(e) if is_contended(&e) => {
                return Err(MemoryError::StoreInUse { owner: read_owner(base_dir) });
            }
            Err(source) => return Err(MemoryError::StoreLockIo { path, source }),
        }

        let owner_path = store_owner_file(base_dir);
        let owner = StoreOwner { holder, pid: std::process::id(), started_at_ms: now_ms() };
        match serde_json::to_string(&owner) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&owner_path, json) {
                    log::warn!(
                        "holding the memory store but could not write {}: {e}; \
                         other processes will only be told it is in use",
                        owner_path.display()
                    );
                }
            }
            Err(e) => log::warn!("could not serialize the memory store owner record: {e}"),
        }

        Ok(Self { _file: file, owner_path })
    }

    /// Who holds the store right now, if anyone.
    ///
    /// Reads `memory.owner`, then CONFIRMS it with a try-lock on `memory.lock`:
    /// if the try-lock succeeds, nobody holds the store and the record is a
    /// leftover from a crashed holder, so the answer is `None`.
    ///
    /// # The window this probe opens
    ///
    /// While the probe holds the lock (microseconds), a holder trying to
    /// acquire would be refused, and would report the stale record's owner. That
    /// needs a stale record — which only exists after a crash — AND a holder
    /// starting in the same instant as a probe. The cost is one spurious
    /// refusal that a retry clears, against the alternative of never being able
    /// to tell a live holder from a dead one's leftovers. Acceptable.
    ///
    /// Anything the probe cannot establish (no record, an unreadable record, a
    /// lock file that cannot be opened, a non-contention lock error) is `None`:
    /// callers use this to fail fast, and failing fast on a guess would be worse
    /// than the slow path they fall back to.
    pub fn current_owner(base_dir: &Path) -> Option<StoreOwner> {
        let owner = read_owner(base_dir)?;

        // No `create`: a record without a lock file is stale by definition,
        // and a read-only probe should not conjure files.
        let file = File::options().read(true).write(true).open(store_lock_file(base_dir)).ok()?;
        match fs2::FileExt::try_lock_exclusive(&file) {
            // We got it, so nobody had it: the record is stale. Released by the
            // drop of `file` at the end of this function.
            Ok(()) => None,
            Err(e) if is_contended(&e) => Some(owner),
            Err(_) => None,
        }
    }
}

impl Drop for StoreLock {
    /// Removes the owner record BEFORE the lock is released.
    ///
    /// In this order a loser can never read a record and then find the store
    /// free-but-advertised, and a NEW holder's record can never be deleted by
    /// the old one: the old holder still holds the lock while it removes its
    /// own file, so no successor has written anything yet.
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.owner_path);
        // `_file` drops after this body, releasing the OS lock.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::{store_lock_file, store_owner_file};

    fn tmp() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    #[test]
    fn acquire_creates_the_lock_file_and_writes_the_owner_record() {
        let dir = tmp();
        let _lock = StoreLock::acquire(dir.path(), StoreHolder::Eval).expect("acquire");

        assert!(store_lock_file(dir.path()).exists());
        let raw = std::fs::read_to_string(store_owner_file(dir.path())).expect("owner file");
        let owner: StoreOwner = serde_json::from_str(&raw).expect("owner json");
        assert_eq!(owner.holder, StoreHolder::Eval);
        assert_eq!(owner.pid, std::process::id());
        assert!(owner.started_at_ms > 0, "the record must carry a real timestamp");
    }

    #[test]
    fn a_second_acquire_is_refused_and_names_the_first_holder() {
        let dir = tmp();
        let _first = StoreLock::acquire(dir.path(), StoreHolder::Daemon).expect("first");

        let err = StoreLock::acquire(dir.path(), StoreHolder::Eval)
            .expect_err("a second acquire must be refused while the first is held");
        match err {
            MemoryError::StoreInUse { owner: Some(owner) } => {
                assert_eq!(owner.holder, StoreHolder::Daemon);
                assert_eq!(owner.pid, std::process::id());
            }
            other => panic!("expected StoreInUse naming the daemon, got {other:?}"),
        }
    }

    #[test]
    fn dropping_the_lock_removes_the_owner_record_and_frees_the_store() {
        let dir = tmp();
        let first = StoreLock::acquire(dir.path(), StoreHolder::Daemon).expect("first");
        drop(first);

        assert!(
            !store_owner_file(dir.path()).exists(),
            "a released store must not keep advertising an owner"
        );
        StoreLock::acquire(dir.path(), StoreHolder::Eval).expect("acquire after release");
    }

    #[test]
    fn a_refused_acquire_does_not_disturb_the_holders_record() {
        let dir = tmp();
        let _first = StoreLock::acquire(dir.path(), StoreHolder::Daemon).expect("first");
        let _ = StoreLock::acquire(dir.path(), StoreHolder::Eval);

        let owner = StoreLock::current_owner(dir.path()).expect("still held");
        assert_eq!(owner.holder, StoreHolder::Daemon);
    }

    #[test]
    fn current_owner_is_none_when_nobody_holds_the_store() {
        assert_eq!(StoreLock::current_owner(tmp().path()), None);
    }

    #[test]
    fn current_owner_reports_the_holder_while_the_lock_is_held() {
        let dir = tmp();
        let _lock = StoreLock::acquire(dir.path(), StoreHolder::Eval).expect("acquire");

        let owner = StoreLock::current_owner(dir.path()).expect("held");
        assert_eq!(owner.holder, StoreHolder::Eval);
        assert_eq!(owner.pid, std::process::id());
    }

    /// The crash case: a holder killed before `Drop` leaves its record behind,
    /// but the OS released the lock. Trusting the record would tell every
    /// client the store is busy forever.
    #[test]
    fn a_stale_owner_record_without_a_lock_reads_as_no_owner() {
        let dir = tmp();
        let stale = StoreOwner { holder: StoreHolder::Eval, pid: 4242, started_at_ms: 1 };
        std::fs::write(
            store_owner_file(dir.path()),
            serde_json::to_string(&stale).expect("serialize"),
        )
        .expect("write");
        std::fs::write(store_lock_file(dir.path()), b"").expect("lock file exists, unlocked");

        assert_eq!(StoreLock::current_owner(dir.path()), None);
        // And the probe must not have left the store locked behind it.
        StoreLock::acquire(dir.path(), StoreHolder::Daemon).expect("probe released its lock");
    }

    #[test]
    fn an_unparseable_owner_record_reads_as_no_owner() {
        let dir = tmp();
        let _lock = StoreLock::acquire(dir.path(), StoreHolder::Daemon).expect("acquire");
        std::fs::write(store_owner_file(dir.path()), b"{half writ").expect("write");

        assert_eq!(StoreLock::current_owner(dir.path()), None);
    }

    /// A loser that races the holder's owner write reads a missing record and
    /// must still get a usable, non-panicking error.
    #[test]
    fn a_refused_acquire_with_no_readable_record_still_reports_in_use() {
        let dir = tmp();
        let _first = StoreLock::acquire(dir.path(), StoreHolder::Daemon).expect("first");
        std::fs::remove_file(store_owner_file(dir.path())).expect("remove record");

        let err = StoreLock::acquire(dir.path(), StoreHolder::Eval)
            .expect_err("still refused");
        assert!(matches!(err, MemoryError::StoreInUse { owner: None }), "{err:?}");
    }

    #[test]
    fn holders_have_human_names() {
        assert_eq!(StoreHolder::Daemon.to_string(), "the clawg daemon");
        assert_eq!(StoreHolder::Eval.to_string(), "an evaluation run");
        assert_eq!(StoreHolder::Maintenance.to_string(), "a clawg backup or restore");
    }

    #[test]
    fn the_owner_record_uses_snake_case_holder_names_on_disk() {
        let owner = StoreOwner { holder: StoreHolder::Daemon, pid: 7, started_at_ms: 9 };
        let json = serde_json::to_string(&owner).expect("serialize");
        assert!(json.contains("\"holder\":\"daemon\""), "{json}");
    }
}
