use crate::memory::store::store_lock::StoreOwner;

/// A failure of the store's backend, shown as the backend gave it: the error
/// that wraps it puts the heading.
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct StorageError(Cause);

#[derive(Debug, thiserror::Error)]
enum Cause {
    #[error("{0}")]
    Sqlite(#[source] rusqlite::Error),
    #[error("{0}")]
    Connection(#[source] tokio_rusqlite::Error),
    #[error("{0}")]
    Migration(#[source] refinery::Error),
    /// A call whose closure did not run or did not answer.
    #[error("{0}")]
    Channel(String),
}

impl From<rusqlite::Error> for StorageError {
    fn from(e: rusqlite::Error) -> Self {
        StorageError(Cause::Sqlite(e))
    }
}

impl From<tokio_rusqlite::Error> for StorageError {
    fn from(e: tokio_rusqlite::Error) -> Self {
        StorageError(Cause::Connection(e))
    }
}

impl From<refinery::Error> for StorageError {
    fn from(e: refinery::Error) -> Self {
        StorageError(Cause::Migration(e))
    }
}

/// What a [`MemoryError`] says before a storage failure, by its cause.
fn heading(e: &StorageError) -> &'static str {
    match e.0 {
        Cause::Sqlite(_) => "SQLite error: ",
        Cause::Connection(_) => "Memory DB connection error: ",
        Cause::Migration(_) => "Migration failed: ",
        Cause::Channel(_) => "memory backend channel error: ",
    }
}

/// How a call that failed because the connection is closed words it. The
/// three wordings reach the daemon's answers, so each call keeps its own.
pub(crate) enum ClosedAs {
    /// `connection closed during <op>`; any other failure as the connection
    /// gives it.
    During(&'static str),
    /// `<op>: <failure>`.
    Named(&'static str),
    /// The failure as the connection gives it.
    AsIs,
}

/// Translates a failed `call` on the connection: the closure's own error
/// becomes `Out`, and a failure of the connection a storage failure worded
/// as `closed` says.
pub(crate) fn failed_call<E, Out>(closed: ClosedAs) -> impl Fn(tokio_rusqlite::Error<E>) -> Out
where
    E: std::fmt::Display,
    Out: From<E> + From<MemoryError>,
{
    move |e| {
        let text = match (e, &closed) {
            (tokio_rusqlite::Error::Error(e), _) => return Out::from(e),
            (tokio_rusqlite::Error::ConnectionClosed, ClosedAs::During(op)) => format!("connection closed during {op}"),
            (other, ClosedAs::Named(op)) => format!("{op}: {other}"),
            (other, _) => other.to_string(),
        };
        Out::from(MemoryError::channel(text))
    }
}

/// Translates a failed run of the migrations.
pub(crate) fn failed_migration(e: tokio_rusqlite::Error<refinery::Error>) -> MemoryError {
    match e {
        tokio_rusqlite::Error::ConnectionClosed => MemoryError::from(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error { code: rusqlite::ffi::ErrorCode::InternalMalfunction, extended_code: 0 },
            Some("connection closed during migration".to_string()),
        )),
        tokio_rusqlite::Error::Close((_, db_err)) => MemoryError::from(db_err),
        tokio_rusqlite::Error::Error(refinery_err) => MemoryError::from(refinery_err),
        // `tokio_rusqlite::Error` is non_exhaustive.
        _ => MemoryError::Internal("unexpected tokio-rusqlite error during migration".to_string()),
    }
}

/// Why a purge did not run.
#[derive(Debug, thiserror::Error)]
pub enum PurgeError {
    #[error("there is no memory #{0}")]
    NotFound(i64),
    #[error("there is no delete #{0}")]
    UnknownDelete(i64),
    #[error("delete #{0} was undone: there is nothing of it to purge")]
    DeleteUndone(i64),
    #[error("delete #{0} was already purged")]
    DeletePurged(i64),
    #[error("memory.db has migrations pending (V{from} to V{to}); start the daemon once so it \
             migrates them, stop it, and run this again")]
    Pending { from: i32, to: i32 },
    #[error("memory.db stayed busy after the rewrite, so its WAL could not be emptied; \
             make sure no other clawg is running and run this again")]
    Busy,
}

/// Memory subsystem errors.
#[derive(Debug, thiserror::Error)]
pub enum MemoryError {
    /// A failure of SQLite, of the connection or of a migration, under the
    /// heading of its cause.
    #[error("{}{}", heading(.0), .0)]
    Storage(#[from] StorageError),

    #[error("internal memory error: {0}")]
    Internal(String),

    /// Workspace.md file I/O failure (read/write/create_dir_all).
    #[error("workspace I/O failed at {path}: {source}")]
    WorkspaceIo {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },

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
    Backup(#[from] crate::memory::store::backup::BackupError),

    /// A purge was refused, or could not finish.
    #[error(transparent)]
    Purge(#[from] PurgeError),

    /// The data directory could not be created.
    #[error("cannot create the data directory {path}: {source}")]
    DataDir {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// The store's lock file could not be opened or locked for a reason other
    /// than contention (permissions, a read-only file system).
    #[error("memory store lock error at {path}: {source}")]
    StoreLockIo {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl MemoryError {
    /// A storage failure that is not an error of SQLite's, worded by the
    /// call that met it.
    pub(crate) fn channel(text: String) -> Self {
        MemoryError::Storage(StorageError(Cause::Channel(text)))
    }

    /// The error of SQLite's this is, for a caller that answers with one;
    /// anything else comes back as it was.
    pub(crate) fn into_sqlite(self) -> Result<rusqlite::Error, Self> {
        match self {
            MemoryError::Storage(StorageError(Cause::Sqlite(e))) => Ok(e),
            other => Err(other),
        }
    }
}

impl From<rusqlite::Error> for MemoryError {
    fn from(e: rusqlite::Error) -> Self {
        MemoryError::Storage(e.into())
    }
}

impl From<tokio_rusqlite::Error> for MemoryError {
    fn from(e: tokio_rusqlite::Error) -> Self {
        MemoryError::Storage(e.into())
    }
}

impl From<refinery::Error> for MemoryError {
    fn from(e: refinery::Error) -> Self {
        MemoryError::Storage(e.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    async fn closed_store() -> crate::memory::store::db::MemoryStore {
        let store = crate::memory::store::db::MemoryStore::new_for_test().await.unwrap();
        store.conn.clone().close().await.unwrap();
        store
    }

    #[tokio::test]
    async fn each_storage_failure_keeps_its_text() {
        use crate::preferences::Lang;
        use crate::memory::domain::sharing::{Reach, Reader};

        let sqlite = MemoryError::from(rusqlite::Error::InvalidQuery);
        assert_eq!(sqlite.to_string(), "SQLite error: Query is not read-only");

        let store = closed_store().await;
        let probe = store.probe_wait().await.unwrap_err();
        assert_eq!(probe.to_string(), "Memory DB connection error: ConnectionClosed");

        let during = crate::memory::upkeep::admin::get_memory_core(&store, 1).await.unwrap_err();
        assert_eq!(during.to_string(), "memory backend channel error: connection closed during get_memory_core");

        let reader = Reader::new("agent", Reach::Own);
        let named = crate::memory::retrieval::language::count_saves_in(&store, Lang::Es, None, &reader).await.unwrap_err();
        assert_eq!(named.to_string(), "memory backend channel error: language count: ConnectionClosed");

        let as_is = crate::memory::projects::registry::rename_project(&store, "p", "name").await.unwrap_err();
        assert_eq!(as_is.to_string(), "memory backend channel error: ConnectionClosed");
    }

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
        let e = MemoryError::channel("closed".into());
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
        use crate::memory::store::store_lock::{StoreHolder, StoreOwner};
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
            matches!(memory_err, MemoryError::Storage(_)),
            "Expected MemoryError::Storage, got {:?}",
            memory_err
        );
    }
}
