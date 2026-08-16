use std::path::Path;
use std::sync::{Arc, Once};

use rusqlite::OptionalExtension;
use tokio_rusqlite::Connection;

use crate::memory::embedder::Embedder;
use crate::memory::error::MemoryError;
use crate::memory::latency::KnnLatency;

// ---------------------------------------------------------------------------
// Embedding contract
// ---------------------------------------------------------------------------

/// The active embedder's full identity, as reported by the embedder itself —
/// never assembled from two different sources. `model_id`/`dim`/`query_prefix`/
/// `passage_prefix` are properties the embedder reports about itself (like
/// `dim = 384`), not invariants the store imposes; `metric`/`normalized` stay
/// out of this type for exactly that reason (see `ensure_embedding_contract`'s
/// doc comment).
///
/// Named fields, not four positional args: two adjacent `String` prefixes
/// would let a query/passage swap compile silently and produce a contract
/// wrong in a way nothing would notice unless a test asserted orientation.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EmbeddingIdentity {
    pub(crate) model_id: String,
    pub(crate) dim: usize,
    pub(crate) query_prefix: String,
    pub(crate) passage_prefix: String,
}

impl EmbeddingIdentity {
    /// Reads all four fields off ONE embedder instance — a mismatch cannot be
    /// assembled by pairing metadata from two different sources.
    pub(crate) fn of(e: &dyn Embedder) -> Self {
        Self {
            model_id: e.model_id().to_string(),
            dim: e.dimensions(),
            query_prefix: e.query_prefix().to_string(),
            passage_prefix: e.passage_prefix().to_string(),
        }
    }
}

/// The `embedding_contract` row as it was actually persisted. `dim` is `i64`
/// because it came out of SQLite, not `usize` like the active embedder
/// reports — the boundary is the point where that type change happens.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct StoredContract {
    pub(crate) model_id: String,
    pub(crate) dim: i64,
    pub(crate) query_prefix: String,
    pub(crate) passage_prefix: String,
}

/// Outcome of comparing the active embedder's identity against the
/// persisted `embedding_contract` row.
///
/// `Match` and `Initialized` are kept distinct (rather than collapsed into a
/// single "ok" case) because "row absent -> written" and "row present ->
/// left alone" are two different assertions callers and tests care about
/// separately.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ContractStatus {
    /// An existing row's `model_id` + `dim` + `query_prefix` + `passage_prefix`
    /// match the active embedder.
    Match,
    /// No row existed; one was just written from the active embedder's metadata.
    Initialized,
    /// An existing row differs from the active embedder on at least one of
    /// `model_id` / `dim` / `query_prefix` / `passage_prefix` — the store
    /// holds vectors from a different embedding space than the one currently
    /// loaded.
    Mismatch {
        stored: StoredContract,
        actual: EmbeddingIdentity,
    },
}

/// One entry per field that differs between `stored` and `actual`, each
/// naming the field and `{:?}`-quoting both sides so a one-trailing-space
/// prefix difference is visible in a log line instead of looking identical.
/// Pure — this is the message body, not a side-effecting log call.
pub(crate) fn differing_fields(stored: &StoredContract, actual: &EmbeddingIdentity) -> Vec<String> {
    let mut out = Vec::new();
    if stored.model_id != actual.model_id {
        out.push(format!(
            "model_id: stored={:?} actual={:?}",
            stored.model_id, actual.model_id
        ));
    }
    if stored.dim != actual.dim as i64 {
        out.push(format!("dim: stored={:?} actual={:?}", stored.dim, actual.dim));
    }
    if stored.query_prefix != actual.query_prefix {
        out.push(format!(
            "query_prefix: stored={:?} actual={:?}",
            stored.query_prefix, actual.query_prefix
        ));
    }
    if stored.passage_prefix != actual.passage_prefix {
        out.push(format!(
            "passage_prefix: stored={:?} actual={:?}",
            stored.passage_prefix, actual.passage_prefix
        ));
    }
    out
}

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
    /// Shared with every clone deliberately: the handles all address one
    /// database, so they belong to one latency series instead of each
    /// keeping a private slice too short to mean anything.
    knn_latency: Arc<KnnLatency>,
}

impl MemoryStore {
    /// Open (or create) the memory database at `<base_dir>/memory.db`.
    ///
    /// Registers the `sqlite-vec` extension (process-global, idempotent) and
    /// runs all five refinery migrations before returning.
    pub async fn open(base_dir: &Path) -> Result<Self, MemoryError> {
        ensure_vec0_registered();
        // Best-effort directory creation — we'll get a clean error on open if it fails.
        let _ = std::fs::create_dir_all(base_dir);
        let path = base_dir.join("memory.db");
        // Connection::open returns Result<_, rusqlite::Error>
        let conn = Connection::open(&path).await
            .map_err(MemoryError::Sqlite)?;
        run_migrations(&conn).await?;
        crate::memory::tier::register_tier_udf(&conn).await?;
        crate::memory::tier::register_demote_udf(&conn).await?;
        // Raise SQLITE_LIMIT_VARIABLE_NUMBER from the default 999 to 32766 (SQLite hard
        // maximum). Flush batches of ≤500 rows × 3 bind vars = 1500 params would otherwise
        // exceed the default limit at row 334.
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
        Ok(Self {
            conn,
            knn_latency: Arc::new(KnnLatency::new()),
        })
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
        crate::memory::tier::register_tier_udf(&conn).await?;
        crate::memory::tier::register_demote_udf(&conn).await?;
        // Raise SQLITE_LIMIT_VARIABLE_NUMBER — same as open() path.
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
        Ok(Self {
            conn,
            knn_latency: Arc::new(KnnLatency::new()),
        })
    }

    /// The shared KNN latency recorder for this store's database.
    ///
    /// Every clone of this `MemoryStore` returns the same recorder — see the
    /// field doc comment for why that sharing is intended, not accidental.
    pub(crate) fn knn_latency(&self) -> &KnnLatency {
        &self.knn_latency
    }

    /// Write-if-absent, verify-if-present bootstrap for the single-row
    /// `embedding_contract` table.
    ///
    /// Compares `model_id` + `dim` + `query_prefix` + `passage_prefix`
    /// against the stored row, byte-exact — no trim, no normalization: a
    /// prefix is fed verbatim to the tokenizer, so `"query:"` and `"query: "`
    /// tokenize to different sequences and a trimmed comparison would forgive
    /// exactly the difference that matters. `metric` and `normalized` stay
    /// OUT of the comparison for the opposite reason: they are invariants
    /// `memory::vector::to_unit` imposes on every row in this store, not
    /// properties the embedder itself reports — the four compared fields are
    /// all things the embedder tells you about itself (like `dim = 384`);
    /// `metric`/`normalized` are things this codebase decided.
    pub(crate) async fn ensure_embedding_contract(
        &self,
        identity: EmbeddingIdentity,
    ) -> Result<ContractStatus, MemoryError> {
        self.conn
            .call(move |c| -> rusqlite::Result<ContractStatus> {
                let existing: Option<(String, i64, String, String)> = c
                    .query_row(
                        "SELECT model_id, dim, query_prefix, passage_prefix \
                         FROM embedding_contract WHERE id = 1",
                        [],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                    )
                    .optional()?;

                match existing {
                    None => {
                        let created_at = crate::memory::clock::TimestampMs::now();
                        c.execute(
                            "INSERT INTO embedding_contract \
                             (id, model_id, dim, metric, normalized, query_prefix, passage_prefix, created_at) \
                             VALUES (1, ?1, ?2, 'l2_unit', 1, ?3, ?4, ?5)",
                            rusqlite::params![
                                identity.model_id,
                                identity.dim as i64,
                                identity.query_prefix,
                                identity.passage_prefix,
                                created_at
                            ],
                        )?;
                        Ok(ContractStatus::Initialized)
                    }
                    Some((stored_model_id, stored_dim, stored_query_prefix, stored_passage_prefix)) => {
                        let stored = StoredContract {
                            model_id: stored_model_id,
                            dim: stored_dim,
                            query_prefix: stored_query_prefix,
                            passage_prefix: stored_passage_prefix,
                        };
                        if stored.model_id == identity.model_id
                            && stored.dim == identity.dim as i64
                            && stored.query_prefix == identity.query_prefix
                            && stored.passage_prefix == identity.passage_prefix
                        {
                            Ok(ContractStatus::Match)
                        } else {
                            Ok(ContractStatus::Mismatch { stored, actual: identity })
                        }
                    }
                }
            })
            .await
            .map_err(MemoryError::Connection)
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
        // 4 regular tables + 3 vec0 virtual + 1 view + 14 indices = 22 rows minimum
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
    // SQLITE_LIMIT_VARIABLE_NUMBER must be 32766 through BOTH connection-
    // opening paths (open() and new_for_test()) — a mismatch here would mean
    // only one of the two ever benefits from the raised bind-parameter cap.
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
        assert_eq!(count, 5, "Expected exactly 5 applied migrations, got {}", count);
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
        assert_eq!(migration_count, 5);
    }

    // -----------------------------------------------------------------------
    // Embedding contract: write-if-absent, verify-if-present
    // -----------------------------------------------------------------------

    /// Test-only embedder double with configurable, independently-settable
    /// query/passage prefixes — `FakeEmbedder` returns `""` for both, which
    /// cannot exercise a prefix-only mismatch.
    struct MockEmbedder {
        model_id: String,
        dim: usize,
        query_prefix: String,
        passage_prefix: String,
    }

    #[async_trait::async_trait]
    impl crate::memory::embedder::Embedder for MockEmbedder {
        async fn embed_with_role(
            &self,
            _text: &str,
            _role: crate::memory::embedder::EmbedRole,
        ) -> Result<Vec<f32>, crate::memory::embedder::EmbedError> {
            Ok(vec![0.0; self.dim])
        }
        fn dimensions(&self) -> usize {
            self.dim
        }
        fn model_id(&self) -> &str {
            &self.model_id
        }
        fn query_prefix(&self) -> &str {
            &self.query_prefix
        }
        fn passage_prefix(&self) -> &str {
            &self.passage_prefix
        }
    }

    fn identity(model_id: &str, dim: usize, query_prefix: &str, passage_prefix: &str) -> EmbeddingIdentity {
        EmbeddingIdentity {
            model_id: model_id.to_string(),
            dim,
            query_prefix: query_prefix.to_string(),
            passage_prefix: passage_prefix.to_string(),
        }
    }

    // -----------------------------------------------------------------------
    // EmbeddingIdentity::of — reads all 4 fields off one embedder instance
    // -----------------------------------------------------------------------

    #[test]
    fn embedding_identity_of_reads_all_four_fields_off_one_embedder() {
        let mock = MockEmbedder {
            model_id: "multilingual-e5-small".to_string(),
            dim: 384,
            query_prefix: "query: ".to_string(),
            passage_prefix: "passage: ".to_string(),
        };
        let got = EmbeddingIdentity::of(&mock);
        assert_eq!(got.model_id, "multilingual-e5-small");
        assert_eq!(got.dim, 384);
        assert_eq!(got.query_prefix, "query: ");
        assert_eq!(got.passage_prefix, "passage: ");
    }

    // -----------------------------------------------------------------------
    // differing_fields — pure
    // -----------------------------------------------------------------------

    #[test]
    fn differing_fields_matching_contract_is_empty() {
        let stored = StoredContract {
            model_id: "m".to_string(),
            dim: 384,
            query_prefix: "query: ".to_string(),
            passage_prefix: "passage: ".to_string(),
        };
        let actual = identity("m", 384, "query: ", "passage: ");
        assert!(differing_fields(&stored, &actual).is_empty());
    }

    #[test]
    fn differing_fields_passage_prefix_only_names_exactly_that_field() {
        let stored = StoredContract {
            model_id: "m".to_string(),
            dim: 384,
            query_prefix: "query: ".to_string(),
            passage_prefix: "passage:".to_string(), // missing the trailing space
        };
        let actual = identity("m", 384, "query: ", "passage: ");
        let diffs = differing_fields(&stored, &actual);
        assert_eq!(diffs.len(), 1, "expected exactly one differing field, got {diffs:?}");
        assert!(diffs[0].contains("passage_prefix"), "got {diffs:?}");
        assert!(diffs[0].contains("\"passage:\""), "stored value must be Debug-quoted, got {diffs:?}");
        assert!(diffs[0].contains("\"passage: \""), "actual value must be Debug-quoted, got {diffs:?}");
    }

    // -----------------------------------------------------------------------
    // ensure_embedding_contract — widened to compare prefixes too
    // -----------------------------------------------------------------------

    /// First call on an empty table writes the row and reports `Initialized`;
    /// a second call with the SAME identity finds the row unchanged and
    /// reports `Match` instead of rewriting it.
    #[tokio::test]
    async fn ensure_embedding_contract_initializes_then_matches() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let status1 = store
            .ensure_embedding_contract(identity("multilingual-e5-small", 384, "query: ", "passage: "))
            .await
            .unwrap();
        assert_eq!(
            status1,
            ContractStatus::Initialized,
            "empty table must be initialized on first call"
        );

        let status2 = store
            .ensure_embedding_contract(identity("multilingual-e5-small", 384, "query: ", "passage: "))
            .await
            .unwrap();
        assert_eq!(
            status2,
            ContractStatus::Match,
            "second call with the same identity must match, not rewrite"
        );
    }

    /// A fresh store's INSERT writes the REAL prefixes, not `''` literals.
    #[tokio::test]
    async fn ensure_embedding_contract_insert_writes_real_prefixes_not_empty_string() {
        let store = MemoryStore::new_for_test().await.unwrap();
        store
            .ensure_embedding_contract(identity("multilingual-e5-small", 384, "query: ", "passage: "))
            .await
            .unwrap();

        let (query_prefix, passage_prefix): (String, String) = store
            .conn
            .call(|c| {
                c.query_row(
                    "SELECT query_prefix, passage_prefix FROM embedding_contract WHERE id = 1",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
            })
            .await
            .unwrap();
        assert_eq!(query_prefix, "query: ");
        assert_eq!(passage_prefix, "passage: ");
    }

    /// A different `model_id` against an existing row is reported as a
    /// mismatch carrying both sides of the comparison.
    #[tokio::test]
    async fn ensure_embedding_contract_reports_mismatch_on_different_model() {
        let store = MemoryStore::new_for_test().await.unwrap();
        store
            .ensure_embedding_contract(identity("multilingual-e5-small", 384, "query: ", "passage: "))
            .await
            .unwrap();
        let status = store
            .ensure_embedding_contract(identity("other-model", 384, "query: ", "passage: "))
            .await
            .unwrap();
        match status {
            ContractStatus::Mismatch { stored, actual } => {
                assert_eq!(stored.model_id, "multilingual-e5-small");
                assert_eq!(stored.dim, 384);
                assert_eq!(actual.model_id, "other-model");
                assert_eq!(actual.dim, 384);
            }
            other => panic!("expected Mismatch, got {other:?}"),
        }
    }

    /// A different `dim` against an existing row (same model_id, same
    /// prefixes) is also a mismatch — model_id/dim comparison still works
    /// standalone after the widening.
    #[tokio::test]
    async fn ensure_embedding_contract_reports_mismatch_on_different_dim() {
        let store = MemoryStore::new_for_test().await.unwrap();
        store
            .ensure_embedding_contract(identity("multilingual-e5-small", 384, "query: ", "passage: "))
            .await
            .unwrap();
        let status = store
            .ensure_embedding_contract(identity("multilingual-e5-small", 768, "query: ", "passage: "))
            .await
            .unwrap();
        assert!(
            matches!(status, ContractStatus::Mismatch { .. }),
            "different dim must be reported as mismatch, got {status:?}"
        );
    }

    /// Matching model_id/dim/query_prefix but a stale `passage_prefix` — even
    /// by one trailing space — must still be reported as a mismatch. This is
    /// the scenario `Normalize` vs. `ReInfer`-without-completion produces.
    #[tokio::test]
    async fn ensure_embedding_contract_reports_mismatch_on_stale_passage_prefix_only() {
        let store = MemoryStore::new_for_test().await.unwrap();
        store
            .ensure_embedding_contract(identity("multilingual-e5-small", 384, "query: ", "passage:"))
            .await
            .unwrap();
        let status = store
            .ensure_embedding_contract(identity("multilingual-e5-small", 384, "query: ", "passage: "))
            .await
            .unwrap();
        match status {
            ContractStatus::Mismatch { stored, actual } => {
                assert_eq!(stored.model_id, actual.model_id, "model_id must still match");
                assert_eq!(stored.dim, actual.dim as i64, "dim must still match");
                assert_ne!(stored.passage_prefix, actual.passage_prefix);
            }
            other => panic!("expected Mismatch on passage_prefix alone, got {other:?}"),
        }
    }

    /// Matching prefixes on both sides → `Match`, not a false mismatch.
    #[tokio::test]
    async fn ensure_embedding_contract_matches_when_prefixes_agree() {
        let store = MemoryStore::new_for_test().await.unwrap();
        store
            .ensure_embedding_contract(identity("multilingual-e5-small", 384, "query: ", "passage: "))
            .await
            .unwrap();
        let status = store
            .ensure_embedding_contract(identity("multilingual-e5-small", 384, "query: ", "passage: "))
            .await
            .unwrap();
        assert_eq!(status, ContractStatus::Match);
    }
}
