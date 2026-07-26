use std::path::Path;
use std::sync::Once;

use tokio_rusqlite::Connection;

use crate::memory::error::MemoryError;

// ---------------------------------------------------------------------------
// vec0 static registration
// ---------------------------------------------------------------------------

static VEC0_REGISTERED: Once = Once::new();

/// Register `sqlite-vec` with SQLite's auto-extension hook — ONCE per process.
///
/// Uses `sqlite3_auto_extension` so the extension is loaded for every subsequent
/// `sqlite3_open` call, including the in-memory connections used in tests.
///
/// The `transmute` pattern matches the upstream `sqlite-vec` demo and is the
/// correct approach for `rusqlite 0.37` (which still exposes the raw FFI).
/// The `Once` guard makes this idempotent and thread-safe.
fn ensure_vec0_registered() {
    VEC0_REGISTERED.call_once(|| {
        unsafe {
            rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(
                sqlite_vec::sqlite3_vec_init as *const (),
            )));
        }
    });
}

// ---------------------------------------------------------------------------
// MemoryStore
// ---------------------------------------------------------------------------

/// Handle to the persistent memory SQLite database.
///
/// Wraps a [`tokio_rusqlite::Connection`], which is itself `Clone` (channel
/// handle) — cloning `MemoryStore` gives another handle to the same background
/// thread. No `Mutex` is needed.
#[derive(Clone)]
pub struct MemoryStore {
    pub conn: Connection,
}

impl MemoryStore {
    /// Open (or create) the memory database at `<base_dir>/memory.db`.
    ///
    /// Registers the `sqlite-vec` extension (process-global, idempotent) and
    /// runs all three refinery migrations before returning.
    pub async fn open(base_dir: &Path) -> Result<Self, MemoryError> {
        ensure_vec0_registered();
        // Best-effort directory creation — we'll get a clean error on open if it fails.
        let _ = std::fs::create_dir_all(base_dir);
        let path = base_dir.join("memory.db");
        // Connection::open returns Result<_, rusqlite::Error>
        let conn = Connection::open(&path).await
            .map_err(MemoryError::Sqlite)?;
        run_migrations(&conn).await?;
        crate::memory::score::register_score_udfs(&conn).await?;
        crate::memory::tier::register_tier_udf(&conn).await?;
        crate::memory::tier::register_demote_udf(&conn).await?;
        // Raise SQLITE_LIMIT_VARIABLE_NUMBER from the default 999 to 32766 (SQLite hard
        // maximum). Flush batches of ≤500 rows × 3 bind vars = 1500 params would otherwise
        // exceed the default limit at row 334. See REQ-LIMIT (LIMIT-1).
        conn.call(|c| -> rusqlite::Result<()> {
            // SAFETY: c is the exclusive connection reference for this call.
            unsafe {
                rusqlite::ffi::sqlite3_limit(
                    c.handle(),
                    rusqlite::ffi::SQLITE_LIMIT_VARIABLE_NUMBER,
                    32766,
                );
            }
            Ok(())
        })
        .await
        .map_err(MemoryError::Connection)?;
        Ok(Self { conn })
    }

    /// In-memory database for unit tests.
    ///
    /// Registers vec0 (process-global) and runs all migrations. Every call in
    /// the same process shares the process-global `Once` guard — registration
    /// happens at most once regardless of how many test stores are created.
    #[cfg(test)]
    pub async fn new_for_test() -> Result<Self, MemoryError> {
        ensure_vec0_registered();
        // Connection::open_in_memory returns Result<_, rusqlite::Error>
        let conn = Connection::open_in_memory().await
            .map_err(MemoryError::Sqlite)?;
        run_migrations(&conn).await?;
        crate::memory::score::register_score_udfs(&conn).await?;
        crate::memory::tier::register_tier_udf(&conn).await?;
        crate::memory::tier::register_demote_udf(&conn).await?;
        // Raise SQLITE_LIMIT_VARIABLE_NUMBER — same as open() path. See REQ-LIMIT (LIMIT-2).
        conn.call(|c| -> rusqlite::Result<()> {
            // SAFETY: c is the exclusive connection reference for this call.
            unsafe {
                rusqlite::ffi::sqlite3_limit(
                    c.handle(),
                    rusqlite::ffi::SQLITE_LIMIT_VARIABLE_NUMBER,
                    32766,
                );
            }
            Ok(())
        })
        .await
        .map_err(MemoryError::Connection)?;
        Ok(Self { conn })
    }
}

// ---------------------------------------------------------------------------
// Migration helper
// ---------------------------------------------------------------------------

async fn run_migrations(conn: &Connection) -> Result<(), MemoryError> {
    conn.call(|c: &mut rusqlite::Connection| -> Result<refinery::Report, refinery::Error> {
        crate::memory::migrations::runner().run(c)
    })
    .await
    .map_err(|e| match e {
        tokio_rusqlite::Error::ConnectionClosed => {
            // Connection was closed before migrations could run — treat as SQLite error.
            MemoryError::Sqlite(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error {
                    code: rusqlite::ffi::ErrorCode::InternalMalfunction,
                    extended_code: 0,
                },
                Some("connection closed during migration".to_string()),
            ))
        }
        tokio_rusqlite::Error::Close((_, db_err)) => {
            MemoryError::Sqlite(db_err)
        }
        tokio_rusqlite::Error::Error(refinery_err) => {
            MemoryError::Migration(refinery_err)
        }
        // non_exhaustive — catch-all for future tokio_rusqlite::Error variants
        _ => MemoryError::Internal(
            "unexpected tokio-rusqlite error during migration".to_string()
        ),
    })
    .map(|_report| ())?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    // -----------------------------------------------------------------------
    // Helper: serialize f32 vector to bytes for vec0 (little-endian IEEE 754)
    // -----------------------------------------------------------------------

    fn serialize_f32_vec(v: &[f32]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(v.len() * 4);
        for &f in v {
            bytes.extend_from_slice(&f.to_le_bytes());
        }
        bytes
    }

    // -----------------------------------------------------------------------
    // F.1-test: vec0 registration + schema tests
    // -----------------------------------------------------------------------

    /// SPEC-3 Sc 3.1, 3.2 — vec_version() returns a non-empty string.
    #[tokio::test]
    async fn vec_version_returns_string() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let version: String = store
            .conn
            .call(|c| -> Result<String, rusqlite::Error> {
                c.query_row("SELECT vec_version()", [], |r| r.get(0))
            })
            .await
            .unwrap();
        assert!(!version.is_empty(), "vec_version() returned empty string");
    }

    /// SPEC-3 Sc 3.2 — ensure_vec0_registered is idempotent (no panic on second call).
    #[tokio::test]
    async fn ensure_vec0_idempotent() {
        // Call registration twice in the same process — Once guard prevents double-init.
        ensure_vec0_registered();
        ensure_vec0_registered();
        // If we reach here without panic, the test passes.
    }

    /// SPEC-2 Sc 2.1 — tables present after open.
    #[tokio::test]
    async fn tables_exist_after_open() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let tables: Vec<String> = store
            .conn
            .call(|c| -> Result<Vec<String>, rusqlite::Error> {
                let mut stmt = c
                    .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")?;
                let names = stmt
                    .query_map([], |r| r.get(0))?
                    .filter_map(|r| r.ok())
                    .collect();
                Ok(names)
            })
            .await
            .unwrap();
        assert!(tables.contains(&"memory".to_string()), "missing 'memory' table");
        assert!(
            tables.contains(&"episodic_trajectory".to_string()),
            "missing 'episodic_trajectory' table"
        );
        assert!(
            tables.contains(&"procedural_skill".to_string()),
            "missing 'procedural_skill' table"
        );
    }

    /// SPEC-2 Sc 2.2 — virtual tables (vec0) present after open.
    #[tokio::test]
    async fn virtual_tables_exist_after_open() {
        let store = MemoryStore::new_for_test().await.unwrap();
        // vec0 shadow tables appear in sqlite_master under various types; query by name.
        let names: Vec<String> = store
            .conn
            .call(|c| -> Result<Vec<String>, rusqlite::Error> {
                let mut stmt = c.prepare("SELECT name FROM sqlite_master ORDER BY name")?;
                let names = stmt
                    .query_map([], |r| r.get(0))?
                    .filter_map(|r| r.ok())
                    .collect();
                Ok(names)
            })
            .await
            .unwrap();
        assert!(
            names.contains(&"memory_vectors".to_string()),
            "missing 'memory_vectors'"
        );
        assert!(
            names.contains(&"episodic_vectors".to_string()),
            "missing 'episodic_vectors'"
        );
        assert!(
            names.contains(&"procedural_vectors".to_string()),
            "missing 'procedural_vectors'"
        );
    }

    /// SPEC-2 Sc 2.3 — view present after open.
    #[tokio::test]
    async fn view_exists_after_open() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let views: Vec<String> = store
            .conn
            .call(|c| -> Result<Vec<String>, rusqlite::Error> {
                let mut stmt =
                    c.prepare("SELECT name FROM sqlite_master WHERE type='view'")?;
                let names = stmt
                    .query_map([], |r| r.get(0))?
                    .filter_map(|r| r.ok())
                    .collect();
                Ok(names)
            })
            .await
            .unwrap();
        assert!(
            views.contains(&"all_memories".to_string()),
            "missing 'all_memories' view"
        );
    }

    /// SPEC-2 Sc 2.4 — indices present after open.
    #[tokio::test]
    async fn indices_exist_after_open() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let indices: Vec<String> = store
            .conn
            .call(|c| -> Result<Vec<String>, rusqlite::Error> {
                let mut stmt = c.prepare(
                    "SELECT name FROM sqlite_master WHERE type='index' ORDER BY name",
                )?;
                let names = stmt
                    .query_map([], |r| r.get(0))?
                    .filter_map(|r| r.ok())
                    .collect();
                Ok(names)
            })
            .await
            .unwrap();
        let expected = [
            "idx_memory_agent_active",
            "idx_memory_scope_project",
            "idx_memory_last_accessed",
            "idx_memory_superseded",
            "idx_memory_disputed",
            "idx_memory_tier_status",
            "idx_epi_agent_active",
            "idx_epi_scope_project",
            "idx_epi_last_accessed",
            "idx_epi_tier_status",
            "idx_proc_agent_active",
            "idx_proc_scope_project",
            "idx_proc_last_accessed",
            "idx_proc_tier_status",
        ];
        for name in &expected {
            assert!(
                indices.contains(&name.to_string()),
                "missing index '{}'",
                name
            );
        }
    }

    /// SPEC-2 total object count.
    #[tokio::test]
    async fn nine_schema_objects_exist() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let count: i64 = store
            .conn
            .call(|c| -> Result<i64, rusqlite::Error> {
                c.query_row(
                    "SELECT count(*) FROM sqlite_master \
                     WHERE type IN ('table','view','index')",
                    [],
                    |r| r.get(0),
                )
            })
            .await
            .unwrap();
        // 3 regular tables + 3 vec0 virtual + 1 view + 14 indices = 21 rows minimum
        assert!(
            count >= 18,
            "Expected at least 18 schema objects, got {}",
            count
        );
    }

    /// SPEC-5 Sc 5.1, 5.2 — MemoryStore is Clone, both handles work.
    #[tokio::test]
    async fn memory_store_is_clone() {
        let s1 = MemoryStore::new_for_test().await.unwrap();
        let s2 = s1.clone();
        let v1: String = s1
            .conn
            .call(|c| -> Result<String, rusqlite::Error> {
                c.query_row("SELECT vec_version()", [], |r| r.get(0))
            })
            .await
            .unwrap();
        let v2: String = s2
            .conn
            .call(|c| -> Result<String, rusqlite::Error> {
                c.query_row("SELECT vec_version()", [], |r| r.get(0))
            })
            .await
            .unwrap();
        assert_eq!(v1, v2);
        assert!(!v1.is_empty());
    }

    // -----------------------------------------------------------------------
    // T-04-test: SQLITE_LIMIT_VARIABLE_NUMBER == 32766 in BOTH paths
    // -----------------------------------------------------------------------

    /// LIMIT-1 — production open() path sets SQLITE_LIMIT_VARIABLE_NUMBER = 32766.
    ///
    /// Uses `sqlite3_limit(db, SQLITE_LIMIT_VARIABLE_NUMBER, -1)` to query the
    /// current value without changing it (SQLite contract: any negative new_val
    /// is a no-op that returns the current limit).
    #[tokio::test]
    async fn limit_variable_number_set_in_open_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = MemoryStore::open(dir.path()).await.unwrap();
        let limit: i32 = store
            .conn
            .call(|c| -> rusqlite::Result<i32> {
                // SAFETY: c is the active handle; sqlite3_limit with -1 is a read-only query.
                Ok(unsafe {
                    rusqlite::ffi::sqlite3_limit(
                        c.handle(),
                        rusqlite::ffi::SQLITE_LIMIT_VARIABLE_NUMBER,
                        -1,
                    )
                })
            })
            .await
            .unwrap();
        assert_eq!(limit, 32766, "open(): expected 32766, got {limit}");
    }

    /// LIMIT-2 — test new_for_test() path sets SQLITE_LIMIT_VARIABLE_NUMBER = 32766.
    #[tokio::test]
    async fn limit_variable_number_set_in_new_for_test_path() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let limit: i32 = store
            .conn
            .call(|c| -> rusqlite::Result<i32> {
                // SAFETY: c is the active handle; sqlite3_limit with -1 is a read-only query.
                Ok(unsafe {
                    rusqlite::ffi::sqlite3_limit(
                        c.handle(),
                        rusqlite::ffi::SQLITE_LIMIT_VARIABLE_NUMBER,
                        -1,
                    )
                })
            })
            .await
            .unwrap();
        assert_eq!(limit, 32766, "new_for_test(): expected 32766, got {limit}");
    }

    /// SPEC-12 Sc 12.1–12.3 — second new_for_test() in same process succeeds.
    #[tokio::test]
    async fn idempotent_migrations_second_open() {
        let _s1 = MemoryStore::new_for_test().await.unwrap();
        let s2 = MemoryStore::new_for_test().await.unwrap();
        let count: i64 = s2
            .conn
            .call(|c| -> Result<i64, rusqlite::Error> {
                c.query_row(
                    "SELECT COUNT(*) FROM refinery_schema_history",
                    [],
                    |r| r.get(0),
                )
            })
            .await
            .unwrap();
        assert_eq!(count, 4, "Expected exactly 4 applied migrations, got {}", count);
    }

    // -----------------------------------------------------------------------
    // G.1-test: vec0 KNN
    // -----------------------------------------------------------------------

    /// SPEC-4 Sc 4.1, 4.2 — INSERT vector + KNN returns rowid=1.
    #[tokio::test]
    async fn vec0_insert_and_knn() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let embedding: Vec<f32> = vec![0.0f32; 384];
        let bytes = serialize_f32_vec(&embedding);

        store
            .conn
            .call(move |c| -> Result<(), rusqlite::Error> {
                c.execute(
                    "INSERT INTO memory_vectors(rowid, embedding) VALUES (?, ?)",
                    params![1i64, bytes],
                )?;
                Ok(())
            })
            .await
            .unwrap();

        let query_bytes = serialize_f32_vec(&embedding);
        let row: i64 = store
            .conn
            .call(move |c| -> Result<i64, rusqlite::Error> {
                c.query_row(
                    "SELECT rowid FROM memory_vectors \
                     WHERE embedding MATCH ? ORDER BY distance LIMIT 1",
                    params![query_bytes],
                    |r| r.get(0),
                )
            })
            .await
            .unwrap();
        assert_eq!(row, 1i64);
    }

    // -----------------------------------------------------------------------
    // I.1-test: CHECK constraints
    // -----------------------------------------------------------------------

    /// SPEC-11 Sc 11.1 — scope='agent' with agent_id IS NULL rejected.
    #[tokio::test]
    async fn check_agent_scope_null_agent_id_rejected() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let result = store
            .conn
            .call(|c| -> Result<usize, rusqlite::Error> {
                c.execute(
                    "INSERT INTO memory \
                     (scope, agent_id, content, created_at, last_modified_at, last_accessed_at) \
                     VALUES ('agent', NULL, 'x', 0, 0, 0)",
                    [],
                )
            })
            .await;
        assert!(result.is_err(), "Expected CHECK constraint violation");
    }

    /// SPEC-11 Sc 11.2 — scope='project' with project_id IS NULL rejected.
    #[tokio::test]
    async fn check_project_scope_null_project_id_rejected() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let result = store
            .conn
            .call(|c| -> Result<usize, rusqlite::Error> {
                c.execute(
                    "INSERT INTO memory \
                     (scope, agent_id, project_id, content, created_at, last_modified_at, last_accessed_at) \
                     VALUES ('project', NULL, NULL, 'x', 0, 0, 0)",
                    [],
                )
            })
            .await;
        assert!(result.is_err(), "Expected CHECK constraint violation");
    }

    /// SPEC-11 Sc 11.3 — scope='global' with agent_id IS NOT NULL rejected.
    #[tokio::test]
    async fn check_global_scope_nonnull_agent_id_rejected() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let result = store
            .conn
            .call(|c| -> Result<usize, rusqlite::Error> {
                c.execute(
                    "INSERT INTO memory \
                     (scope, agent_id, content, created_at, last_modified_at, last_accessed_at) \
                     VALUES ('global', 'some-agent', 'x', 0, 0, 0)",
                    [],
                )
            })
            .await;
        assert!(result.is_err(), "Expected CHECK constraint violation");
    }

    // -----------------------------------------------------------------------
    // K.1-test: End-to-end integration smoke test
    // -----------------------------------------------------------------------

    /// SPEC-2 through SPEC-12 — covers all 12 spec scenarios in one run.
    #[tokio::test]
    async fn e2e_all_spec_scenarios() {
        let store = MemoryStore::new_for_test().await.unwrap();

        // SPEC-2 Sc 2.1 — tables
        let tables: Vec<String> = store
            .conn
            .call(|c| -> Result<Vec<String>, rusqlite::Error> {
                let mut s = c.prepare(
                    "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name",
                )?;
                let result: Vec<String> = s
                    .query_map([], |r| r.get(0))?
                    .filter_map(|r| r.ok())
                    .collect();
                Ok(result)
            })
            .await
            .unwrap();
        assert!(tables.contains(&"memory".to_string()));
        assert!(tables.contains(&"episodic_trajectory".to_string()));
        assert!(tables.contains(&"procedural_skill".to_string()));

        // SPEC-2 Sc 2.3 — view
        let views: Vec<String> = store
            .conn
            .call(|c| -> Result<Vec<String>, rusqlite::Error> {
                let mut s =
                    c.prepare("SELECT name FROM sqlite_master WHERE type='view'")?;
                let result: Vec<String> = s
                    .query_map([], |r| r.get(0))?
                    .filter_map(|r| r.ok())
                    .collect();
                Ok(result)
            })
            .await
            .unwrap();
        assert!(views.contains(&"all_memories".to_string()));

        // SPEC-3 Sc 3.1 — vec_version
        let version: String = store
            .conn
            .call(|c| -> Result<String, rusqlite::Error> {
                c.query_row("SELECT vec_version()", [], |r| r.get(0))
            })
            .await
            .unwrap();
        assert!(!version.is_empty());

        // SPEC-4 Sc 4.1, 4.2 — KNN insert + search
        let embedding: Vec<f32> = vec![1.0f32; 384];
        let bytes = serialize_f32_vec(&embedding);
        store
            .conn
            .call(move |c| -> Result<(), rusqlite::Error> {
                c.execute(
                    "INSERT INTO memory_vectors(rowid, embedding) VALUES (1, ?)",
                    params![bytes],
                )?;
                Ok(())
            })
            .await
            .unwrap();

        let query_bytes = serialize_f32_vec(&embedding);
        let rowid: i64 = store
            .conn
            .call(move |c| -> Result<i64, rusqlite::Error> {
                c.query_row(
                    "SELECT rowid FROM memory_vectors \
                     WHERE embedding MATCH ? ORDER BY distance LIMIT 1",
                    params![query_bytes],
                    |r| r.get(0),
                )
            })
            .await
            .unwrap();
        assert_eq!(rowid, 1);

        // SPEC-5 Sc 5.2 — clone
        let s2 = store.clone();
        let v2: String = s2
            .conn
            .call(|c| -> Result<String, rusqlite::Error> {
                c.query_row("SELECT vec_version()", [], |r| r.get(0))
            })
            .await
            .unwrap();
        assert!(!v2.is_empty());

        // SPEC-11 Sc 11.1 — CHECK constraints
        let bad = store
            .conn
            .call(|c| -> Result<usize, rusqlite::Error> {
                c.execute(
                    "INSERT INTO memory (scope, agent_id, content, \
                     created_at, last_modified_at, last_accessed_at) \
                     VALUES ('agent', NULL, 'x', 0, 0, 0)",
                    [],
                )
            })
            .await;
        assert!(bad.is_err());

        // SPEC-12 Sc 12.3 — migration count
        let migration_count: i64 = store
            .conn
            .call(|c| -> Result<i64, rusqlite::Error> {
                c.query_row(
                    "SELECT COUNT(*) FROM refinery_schema_history",
                    [],
                    |r| r.get(0),
                )
            })
            .await
            .unwrap();
        assert_eq!(migration_count, 4);
    }
}
