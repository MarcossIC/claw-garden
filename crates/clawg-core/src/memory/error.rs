use crate::memory::store_lock::StoreOwner;

/// Memory subsystem errors.
///
/// This enum consolidates all failure modes of the persistent-memory substrate:
/// migration failures, connection errors, raw SQLite errors, vec0 registration
/// failures, and row-decode errors.
#[derive(Debug, thiserror::Error)]
pub enum MemoryError {
    #[error("Migration failed: {0}")]
    Migration(#[from] refinery::Error),

    #[error("Memory DB connection error: {0}")]
    Connection(#[from] tokio_rusqlite::Error),

    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("Failed to register sqlite-vec extension: {0}")]
    VecExtensionRegistration(String),

    #[error("Failed to decode row from {table}: {source}")]
    RowDecode {
        table: &'static str,
        #[source]
        source: rusqlite::Error,
    },

    #[error("internal memory error: {0}")]
    Internal(String),

    /// Workspace.md file I/O failure (read/write/create_dir_all).
    #[error("workspace I/O failed at {path}: {source}")]
    WorkspaceIo {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// tokio_rusqlite channel error not tied to a SQLite error code.
    #[error("memory backend channel error: {0}")]
    Backend(String),

    /// Another process already owns this data directory's store. `owner` is
    /// whatever `memory.owner` said — best-effort, because the holder may not
    /// have written it yet (or could not).
    #[error("the memory store is in use by {}", match .owner {
        Some(o) => format!("{} (pid {})", o.holder, o.pid),
        None => "another process".to_string(),
    })]
    StoreInUse { owner: Option<StoreOwner> },

    /// `memory.db` was migrated by a newer clawg. Refused before anything is
    /// written, so going back to that version still works.
    #[error("memory.db is at schema V{db}, newer than the V{binary} this clawg knows: \
             it was written by a newer clawg. Nothing was changed. Install that version \
             again, or restore a backup taken before the upgrade with `clawg restore`")]
    SchemaNewer { db: i32, binary: i32 },

    /// A migration recorded in `memory.db` differs from this binary's.
    #[error("memory.db's migration V{version} does not match this clawg build. Nothing was \
             changed. Use the clawg build that migrated it, or restore a backup with \
             `clawg restore`")]
    SchemaDiverged { version: i32 },

    /// A snapshot of the store could not be written.
    #[error(transparent)]
    Backup(#[from] crate::memory::backup::BackupError),

    /// The store's lock file could not be opened or locked for a reason other
    /// than contention (permissions, a read-only file system).
    #[error("memory store lock error at {path}: {source}")]
    StoreLockIo {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn workspace_io_error_displays_path_and_source() {
        let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        let e = MemoryError::WorkspaceIo {
            path: PathBuf::from("/x"),
            source: io_err,
        };
        let msg = format!("{e}");
        assert!(msg.contains("/x"), "Expected path '/x' in: {msg}");
    }

    #[test]
    fn backend_error_displays_message() {
        let e = MemoryError::Backend("closed".into());
        let msg = format!("{e}");
        assert!(msg.contains("closed"), "Expected 'closed' in: {msg}");
    }

    #[test]
    fn workspace_io_error_is_send_sync() {
        fn _s<T: Send + Sync>() {}
        _s::<MemoryError>();
    }

    #[test]
    fn store_in_use_names_the_holder_and_its_pid() {
        use crate::memory::store_lock::{StoreHolder, StoreOwner};
        let e = MemoryError::StoreInUse {
            owner: Some(StoreOwner { holder: StoreHolder::Eval, pid: 4242, started_at_ms: 1 }),
        };
        assert_eq!(
            e.to_string(),
            "the memory store is in use by an evaluation run (pid 4242)"
        );
    }

    #[test]
    fn store_in_use_without_an_owner_still_explains_itself() {
        let e = MemoryError::StoreInUse { owner: None };
        assert_eq!(e.to_string(), "the memory store is in use by another process");
    }

    #[test]
    fn store_lock_io_displays_path_and_source() {
        let e = MemoryError::StoreLockIo {
            path: PathBuf::from("/x/memory.lock"),
            source: std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied"),
        };
        let msg = e.to_string();
        assert!(msg.contains("memory.lock") && msg.contains("denied"), "{msg}");
    }

    /// Internal variant carries message and displays it.
    #[test]
    fn internal_variant_displays_message() {
        let e = MemoryError::Internal("boom".into());
        let msg = format!("{e}");
        assert!(msg.contains("boom"), "Expected 'boom' in: {msg}");
    }

    /// From<rusqlite::Error> allows ? propagation.
    #[test]
    fn from_rusqlite_error() {
        // Create a rusqlite error via the documented FromSql trait violation path.
        let rusqlite_err = rusqlite::Error::InvalidQuery;
        let memory_err: MemoryError = rusqlite_err.into();
        assert!(
            matches!(memory_err, MemoryError::Sqlite(_)),
            "Expected MemoryError::Sqlite, got {:?}",
            memory_err
        );
    }

    /// RowDecode Display contains table name.
    #[test]
    fn row_decode_displays_table_name() {
        let err = MemoryError::RowDecode {
            table: "memory",
            source: rusqlite::Error::InvalidQuery,
        };
        let display = format!("{}", err);
        assert!(
            display.contains("memory"),
            "Expected 'memory' in display: {}",
            display
        );
    }
}
