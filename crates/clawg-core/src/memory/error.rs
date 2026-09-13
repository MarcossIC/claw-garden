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
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // T01-test: MemoryError new variants compile
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

    /// FIX-2 — Internal variant carries message and displays it.
    #[test]
    fn internal_variant_displays_message() {
        let e = MemoryError::Internal("boom".into());
        let msg = format!("{e}");
        assert!(msg.contains("boom"), "Expected 'boom' in: {msg}");
    }

    /// SPEC-7 Sc 7.2 — From<rusqlite::Error> allows ? propagation.
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

    /// SPEC-7 Sc 7.3 — RowDecode Display contains table name.
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
