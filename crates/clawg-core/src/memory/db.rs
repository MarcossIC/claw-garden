use std::path::{Path, PathBuf};
use std::sync::{Arc, Once};

use rusqlite::OptionalExtension;
use tokio_rusqlite::Connection;

use crate::memory::backup::{self, SchemaPlan};
use crate::memory::embedder::Embedder;
use crate::memory::error::MemoryError;
use crate::memory::latency::KnnLatency;
use crate::memory::store_lock::{StoreHolder, StoreLock};

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
pub struct EmbeddingIdentity {
    pub model_id: String,
    pub dim: usize,
    pub query_prefix: String,
    pub passage_prefix: String,
}

impl EmbeddingIdentity {
    /// Reads all four fields off ONE embedder instance — a mismatch cannot be
    /// assembled by pairing metadata from two different sources.
    pub fn of(e: &dyn Embedder) -> Self {
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
pub struct StoredContract {
    pub model_id: String,
    pub dim: i64,
    pub query_prefix: String,
    pub passage_prefix: String,
}

/// Outcome of comparing the active embedder's identity against the
/// persisted `embedding_contract` row.
///
/// `Match` and `Initialized` are kept distinct (rather than collapsed into a
/// single "ok" case) because "row absent -> written" and "row present ->
/// left alone" are two different assertions callers and tests care about
/// separately.
#[derive(Debug, Clone, PartialEq)]
pub enum ContractStatus {
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
pub fn differing_fields(stored: &StoredContract, actual: &EmbeddingIdentity) -> Vec<String> {
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
pub(crate) fn ensure_vec0_registered() {
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
    /// Exclusive ownership of `<base_dir>/memory.db`, shared with every clone.
    ///
    /// Never read: the guard is the value. Behind an `Arc` so the store stays
    /// owned until the LAST clone is dropped — the runtime hands clones to the
    /// ingest worker and the maintenance loop, and releasing the lock when the
    /// first one goes would let a second process in under still-live writers.
    /// `None` for the in-memory test store, which has nothing to exclude.
    _lock: Option<Arc<StoreLock>>,
    /// The data directory holding `memory.db`, where a second connection can
    /// open it. `None` for the in-memory test store, which has no file.
    dir: Option<PathBuf>,
}

impl MemoryStore {
    /// Open (or create) the memory database at `<base_dir>/memory.db`.
    ///
    /// Registers the `sqlite-vec` extension (process-global, idempotent) and
    /// runs every refinery migration before returning.
    ///
    /// Takes exclusive ownership of the data directory FIRST — before SQLite is
    /// touched — and keeps it for as long as any clone of the returned store
    /// lives. A process that loses gets [`MemoryError::StoreInUse`] and has not
    /// created, opened or migrated `memory.db`: those are the holder's alone.
    /// `holder` says who is asking, so the loser's message can name it.
    pub async fn open(base_dir: &Path, holder: StoreHolder) -> Result<Self, MemoryError> {
        ensure_vec0_registered();
        // Best-effort directory creation — we'll get a clean error on open if it fails.
        let _ = std::fs::create_dir_all(base_dir);
        let lock = StoreLock::acquire(base_dir, holder)?;
        let path = base_dir.join("memory.db");
        // Connection::open returns Result<_, rusqlite::Error>
        let conn = Connection::open(&path).await
            .map_err(MemoryError::Sqlite)?;
        // First, before any pragma: switching the journal mode rewrites the
        // file header, and a database this binary refuses must stay untouched.
        let plan = check_schema(&conn).await?;
        // WAL plus synchronous=NORMAL: a commit appends without an fsync,
        // which the checkpoint pays instead, and a snapshot reading on its
        // own connection does not stop this one from writing. Set before the
        // migrations so the schema build runs under it too.
        //
        // Read back rather than assumed, and never fatal: journal_mode
        // returns the mode it actually landed on, and a filesystem that
        // refuses WAL (a network-mounted home does exactly this) must leave
        // the app slower, not unable to start.
        match conn
            .call(|c| -> rusqlite::Result<String> {
                c.pragma_update(None, "synchronous", "NORMAL")?;
                c.query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))
            })
            .await
        {
            Ok(mode) if mode.eq_ignore_ascii_case("wal") => {}
            Ok(mode) => log::warn!("memory.db journal_mode is {mode}, not wal; falling back to the default rollback journal"),
            Err(e) => log::warn!("could not set memory.db journal mode: {e}; falling back to the default rollback journal"),
        }
        register_udfs(&conn).await?;
        if let SchemaPlan::Pending { .. } = plan {
            // Migrations only ever move forward, and a binary that applied
            // them cannot be swapped back for the one before: this copy is
            // the way back.
            let backups = crate::paths::backups_dir(base_dir);
            conn.call(move |c| -> rusqlite::Result<_> {
                Ok(backup::snapshot_into(c, &backups, backup::Reason::PreMigration))
            })
            .await
            .map_err(MemoryError::Connection)??;
        }
        run_migrations(&conn).await?;
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
            _lock: Some(Arc::new(lock)),
            dir: Some(base_dir.to_path_buf()),
        })
    }

    /// In-memory database for unit tests.
    ///
    /// Registers vec0 (process-global) and runs all migrations. Every call in
    /// the same process shares the process-global `Once` guard — registration
    /// happens at most once regardless of how many test stores are created.
    #[cfg(any(test, feature = "testing"))]
    pub async fn new_for_test() -> Result<Self, MemoryError> {
        ensure_vec0_registered();
        // Connection::open_in_memory returns Result<_, rusqlite::Error>
        let conn = Connection::open_in_memory().await
            .map_err(MemoryError::Sqlite)?;
        register_udfs(&conn).await?;
        run_migrations(&conn).await?;
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
            _lock: None,
            dir: None,
        })
    }

    /// Writes a snapshot of the store into `into` and returns its path.
    ///
    /// `VACUUM INTO` reads every page, so it runs on a short-lived connection
    /// of its own: on this one, every call queued behind it would wait for the
    /// whole copy. WAL lets that connection read one consistent state while
    /// this one keeps writing. The in-memory test store has no file for a
    /// second connection and copies through its own.
    pub async fn snapshot(&self, into: PathBuf, reason: backup::Reason) -> Result<PathBuf, MemoryError> {
        let Some(dir) = self.dir.clone() else {
            return self
                .conn
                .call(move |c| -> rusqlite::Result<_> { Ok(backup::snapshot_into(c, &into, reason)) })
                .await
                .map_err(MemoryError::Connection)?
                .map_err(MemoryError::from);
        };
        tokio::task::spawn_blocking(move || {
            let c = backup::open_existing(&dir)?;
            Ok(backup::snapshot_into(&c, &into, reason)?)
        })
        .await
        .map_err(|e| MemoryError::Internal(format!("snapshot task: {e}")))?
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
    pub async fn ensure_embedding_contract(
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
// UDF registration
// ---------------------------------------------------------------------------

/// Registers every Rust scalar UDF this database's SQL may call.
///
/// Runs BEFORE the migration runner, not after: a migration body can call
/// these (the content-hash backfill does), and an unregistered UDF is a
/// "no such function" error that aborts migration on cold start. Nothing
/// here touches the schema, so running it against an unmigrated connection
/// is safe by construction. New UDFs go inside this function — never as a
/// separate call beside run_migrations.
async fn register_udfs(conn: &Connection) -> Result<(), MemoryError> {
    crate::memory::dedup::register_content_hash_udf(conn).await?;
    crate::memory::tier::register_tier_udf(conn).await?;
    crate::memory::tier::register_demote_udf(conn).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Migration helper
// ---------------------------------------------------------------------------

/// The runner settings production migrates with.
///
/// Grouped: every pending migration and its history row commit in one
/// transaction. Ungrouped, a kill between a migration's body and its history
/// row re-runs the body on the next start, and for an `ADD COLUMN` that is a
/// "duplicate column" error on every start from then on.
fn configured(runner: refinery::Runner) -> refinery::Runner {
    configured_to(runner, crate::build_info::migrations_up_to())
}

/// [`configured`] with the newest migration to apply, `None` for all.
fn configured_to(runner: refinery::Runner, up_to: Option<i32>) -> refinery::Runner {
    let runner = runner.set_grouped(true);
    match up_to {
        Some(newest) => runner.set_target(refinery::Target::Version(newest)),
        None => runner,
    }
}

/// Refuses a database this binary must not migrate, and says what opening it
/// would do otherwise.
async fn check_schema(conn: &Connection) -> Result<SchemaPlan, MemoryError> {
    let applied = conn
        .call(|c| backup::read_history(c))
        .await
        .map_err(MemoryError::Connection)?;
    match backup::schema_plan(&applied, &backup::known_migrations()) {
        SchemaPlan::Newer { db, binary } => Err(MemoryError::SchemaNewer { db, binary }),
        SchemaPlan::Diverged { version } => Err(MemoryError::SchemaDiverged { version }),
        plan => Ok(plan),
    }
}

async fn run_migrations(conn: &Connection) -> Result<(), MemoryError> {
    conn.call(|c: &mut rusqlite::Connection| -> Result<refinery::Report, refinery::Error> {
        configured(crate::memory::migrations::runner()).run(c)
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

/// Builds `<dir>/memory.db` migrated up to `target` (all when `None`),
/// with one row, and closes it so the file stands alone.
#[cfg(test)]
pub(crate) async fn seed_on_disk(dir: &Path, target: Option<i32>) {
    ensure_vec0_registered();
    let conn = Connection::open(dir.join("memory.db")).await.unwrap();
    register_udfs(&conn).await.unwrap();
    conn.call(move |c| {
        let runner = crate::memory::migrations::runner();
        let runner = match target {
            Some(v) => runner.set_target(refinery::Target::Version(v)),
            None => runner,
        };
        runner.run(c)
    })
    .await
    .unwrap();
    conn.call(|c| {
        c.execute(
            "INSERT INTO memory \
             (scope, agent_id, content, memory_type, source, \
              created_at, created_by_type, created_by_ref, \
              last_modified_at, last_accessed_at, importance, tier, status) \
             VALUES ('agent', 'agent-seed', 'seeded on disk', 'chat_message', 'chat', \
              0, 'user', 'agent-seed', 0, 0, 5.0, 3, 'active')",
            [],
        )
    })
    .await
    .unwrap();
    conn.close().await.unwrap();
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
    // Vec0 registration + schema tests
    // -----------------------------------------------------------------------

    /// Vec_version() returns a non-empty string.
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

    /// V6's eager backfill, exercised through the migration runner itself.
    ///
    /// `dedup.rs::backfilled_row_becomes_lookup_visible` covers the same
    /// property by executing its own transcription of the UPDATE, so it says
    /// nothing about the migration file: deleting the backfill statement from
    /// `V6__content_hash.sql` outright left all 801 tests green
    /// (mutation-verified). A shipped build with no backfill leaves every row
    /// written before the upgrade at `content_hash IS NULL` — permanently
    /// invisible to the dedup gate, which is the entire point of doing the
    /// backfill eagerly instead of lazily.
    ///
    /// This is the only test that migrates in two steps: rows must exist
    /// BEFORE V6 runs for a backfill to have anything to do, and
    /// `new_for_test()` runs the whole chain against an empty database.
    #[tokio::test]
    async fn v6_migration_backfills_rows_written_before_it() {
        ensure_vec0_registered();
        let conn = Connection::open_in_memory().await.unwrap();
        register_udfs(&conn).await.unwrap();

        // Stop at V5: the schema has no content_hash column yet.
        conn.call(|c| {
            crate::memory::migrations::runner()
                .set_target(refinery::Target::Version(5))
                .run(c)
        })
        .await
        .unwrap();

        conn.call(|c| {
            c.execute(
                "INSERT INTO memory \
                 (scope, agent_id, content, memory_type, source, \
                  created_at, created_by_type, created_by_ref, \
                  last_modified_at, last_accessed_at, importance, tier, status) \
                 VALUES ('agent', 'agent-v6', 'pre-v6 content', 'chat_message', 'chat', \
                  0, 'user', 'agent-v6', 0, 0, 5.0, 3, 'active')",
                [],
            )
        })
        .await
        .unwrap();

        // Now run V6 — the row above is exactly the legacy case it targets.
        conn.call(|c| crate::memory::migrations::runner().run(c))
            .await
            .unwrap();

        let hash: Option<String> = conn
            .call(|c| {
                c.query_row(
                    "SELECT content_hash FROM memory WHERE agent_id = 'agent-v6'",
                    [],
                    |r| r.get(0),
                )
            })
            .await
            .unwrap();

        assert_eq!(
            hash.as_deref(),
            Some(crate::memory::dedup::content_hash("pre-v6 content").as_str()),
            "V6 must backfill content_hash for rows that predate the column, in the same hash \
             space the runtime write path uses"
        );
    }

    fn table_exists(c: &rusqlite::Connection, name: &str) -> bool {
        c.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name = ?1",
            [name],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
            > 0
    }

    /// A kill between a migration's body and its history row would re-run it
    /// on the next start, and an `ADD COLUMN` re-run is "duplicate column" on
    /// every start after that. Applying a run's migrations in one transaction
    /// closes that window; a failure anywhere in the run must leave nothing.
    #[tokio::test]
    async fn a_failing_migration_rolls_back_every_migration_of_its_run() {
        ensure_vec0_registered();
        let conn = Connection::open_in_memory().await.unwrap();
        register_udfs(&conn).await.unwrap();

        let result = conn
            .call(|c| {
                let mut migrations = crate::memory::migrations::runner().get_migrations().clone();
                migrations.push(
                    refinery::Migration::unapplied("V9__broken", "THIS IS NOT SQL;").unwrap(),
                );
                configured(refinery::Runner::new(&migrations)).run(c)
            })
            .await;
        assert!(result.is_err(), "the broken V9 must fail the run");

        let (memory, vectors, history_rows) = conn
            .call(|c| -> Result<(bool, bool, i64), rusqlite::Error> {
                let history_rows = if table_exists(c, "refinery_schema_history") {
                    c.query_row("SELECT COUNT(*) FROM refinery_schema_history", [], |r| r.get(0))?
                } else {
                    0
                };
                Ok((table_exists(c, "memory"), table_exists(c, "memory_vectors"), history_rows))
            })
            .await
            .unwrap();
        assert!(!memory, "V1 must be rolled back");
        assert!(!vectors, "the vec0 table from V2 must be rolled back");
        assert_eq!(history_rows, 0, "no history row may survive a failed run");
    }

    /// Existing installs were migrated one migration per transaction. Under the
    /// grouped runner they have nothing pending, so opening them must change
    /// nothing.
    #[tokio::test]
    async fn a_v8_database_opens_unchanged_under_grouped_migrations() {
        ensure_vec0_registered();
        let conn = Connection::open_in_memory().await.unwrap();
        register_udfs(&conn).await.unwrap();
        conn.call(|c| crate::memory::migrations::runner().run(c))
            .await
            .unwrap();
        conn.call(|c| {
            c.execute(
                "INSERT INTO memory \
                 (scope, agent_id, content, memory_type, source, \
                  created_at, created_by_type, created_by_ref, \
                  last_modified_at, last_accessed_at, importance, tier, status) \
                 VALUES ('agent', 'agent-v8', 'seeded before grouping', 'chat_message', 'chat', \
                  0, 'user', 'agent-v8', 0, 0, 5.0, 3, 'active')",
                [],
            )
        })
        .await
        .unwrap();

        let snapshot = |c: &mut rusqlite::Connection| -> Result<(i64, String), rusqlite::Error> {
            let history: i64 =
                c.query_row("SELECT COUNT(*) FROM refinery_schema_history", [], |r| r.get(0))?;
            let content: String = c.query_row(
                "SELECT group_concat(content, '|') FROM memory WHERE agent_id = 'agent-v8'",
                [],
                |r| r.get(0),
            )?;
            Ok((history, content))
        };
        let before = conn.call(snapshot).await.unwrap();

        run_migrations(&conn).await.unwrap();

        let after = conn.call(snapshot).await.unwrap();
        assert_eq!(before, after);
        assert_eq!(after.0, 17);
    }

    /// Rows written before `project` existed keep their project ids when the
    /// migration that adds it runs: moving them is the bridge's call, made per
    /// path, never a blanket rewrite at open.
    #[tokio::test]
    async fn a_database_from_the_previous_version_opens_with_its_project_ids_untouched() {
        ensure_vec0_registered();
        let conn = Connection::open_in_memory().await.unwrap();
        register_udfs(&conn).await.unwrap();
        conn.call(|c| {
            configured(crate::memory::migrations::runner())
                .set_target(refinery::Target::Version(8))
                .run(c)
        })
        .await
        .unwrap();
        conn.call(|c| {
            for (agent, project) in [("a", Some("repo-1a2b")), ("a", Some("src-3c4d")), ("b", None)] {
                c.execute(
                    "INSERT INTO memory \
                     (scope, agent_id, project_id, content, memory_type, source, \
                      created_at, created_by_type, created_by_ref, \
                      last_modified_at, last_accessed_at, importance, tier, status) \
                     VALUES ('agent', ?1, ?2, 'seeded at v8', 'observation', 'agent', \
                      0, 'agent', ?1, 0, 0, 5.0, 3, 'active')",
                    rusqlite::params![agent, project],
                )?;
            }
            c.execute(
                "INSERT INTO episodic_trajectory \
                 (scope, agent_id, project_id, session_id, task, created_at, \
                  last_modified_at, last_accessed_at) \
                 VALUES ('agent', 'a', 'repo-1a2b', 's', 't', 0, 0, 0)",
                [],
            )?;
            c.execute(
                "INSERT INTO procedural_skill \
                 (scope, agent_id, project_id, skill_name, trigger, promoted_at, \
                  source_trajectory_ids, created_at, last_modified_at, last_accessed_at) \
                 VALUES ('agent', 'a', 'src-3c4d', 'k', 't', 0, '[]', 0, 0, 0)",
                [],
            )?;
            Ok::<_, rusqlite::Error>(())
        })
        .await
        .unwrap();

        let project_ids = |c: &mut rusqlite::Connection| -> Result<Vec<Option<String>>, rusqlite::Error> {
            let mut stmt = c.prepare(
                "SELECT project_id FROM memory UNION ALL SELECT project_id FROM episodic_trajectory \
                 UNION ALL SELECT project_id FROM procedural_skill",
            )?;
            let ids = stmt.query_map([], |r| r.get(0))?.collect();
            ids
        };
        let before = conn.call(project_ids).await.unwrap();

        run_migrations(&conn).await.unwrap();

        assert_eq!(conn.call(project_ids).await.unwrap(), before);
        let (projects, rekeyed): (i64, i64) = conn
            .call(|c| {
                c.query_row(
                    "SELECT (SELECT COUNT(*) FROM project), (SELECT COUNT(*) FROM project_rekey)",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
            })
            .await
            .unwrap();
        assert_eq!((projects, rekeyed), (0, 0));
    }

    /// The tables that record where a memory came from and when a scope was
    /// last reviewed arrive empty: every row a store already holds is a plain
    /// memory nobody has drawn a conclusion from, and nothing it held moves.
    #[tokio::test]
    async fn a_store_from_the_previous_version_opens_with_every_row_untouched() {
        ensure_vec0_registered();
        let conn = Connection::open_in_memory().await.unwrap();
        register_udfs(&conn).await.unwrap();
        conn.call(|c| {
            configured(crate::memory::migrations::runner())
                .set_target(refinery::Target::Version(9))
                .run(c)
        })
        .await
        .unwrap();
        conn.call(|c| {
            c.execute(
                "INSERT INTO memory \
                 (scope, agent_id, project_id, content, memory_type, source, \
                  created_at, created_by_type, created_by_ref, \
                  last_modified_at, last_accessed_at, importance, tier, status) \
                 VALUES ('agent', 'a', 'p', 'seeded at v9', 'user_preference', 'explicit', \
                  1, 'user', 'a', 1, 1, 7.0, 1, 'active')",
                [],
            )?;
            c.execute(
                "INSERT INTO episodic_trajectory \
                 (scope, agent_id, project_id, session_id, task, created_at, \
                  last_modified_at, last_accessed_at) \
                 VALUES ('agent', 'a', 'p', 's', 'fix the bug', 2, 2, 2)",
                [],
            )?;
            c.execute(
                "INSERT INTO procedural_skill \
                 (scope, agent_id, project_id, skill_name, trigger, promoted_at, \
                  source_trajectory_ids, created_at, last_modified_at, last_accessed_at) \
                 VALUES ('agent', 'a', 'p', 'Fix login flow', 'when login fails', 3, '[1]', 3, 3, 3)",
                [],
            )?;
            Ok::<_, rusqlite::Error>(())
        })
        .await
        .unwrap();

        let rows = |c: &mut rusqlite::Connection| -> Result<Vec<String>, rusqlite::Error> {
            let mut stmt = c.prepare(
                "SELECT json_array(id, content, memory_type, tier, importance, status) FROM memory \
                 UNION ALL SELECT json_array(id, task, tier, status) FROM episodic_trajectory \
                 UNION ALL SELECT json_array(id, skill_name, trigger, tier, status) FROM procedural_skill",
            )?;
            let rows = stmt.query_map([], |r| r.get(0))?.collect();
            rows
        };
        let before = conn.call(rows).await.unwrap();

        run_migrations(&conn).await.unwrap();

        assert_eq!(conn.call(rows).await.unwrap(), before);
        let (derivations, reviews): (i64, i64) = conn
            .call(|c| {
                c.query_row(
                    "SELECT (SELECT COUNT(*) FROM memory_derivation), \
                            (SELECT COUNT(*) FROM synthesis_review)",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
            })
            .await
            .unwrap();
        assert_eq!((derivations, reviews), (0, 0));
    }

    /// The documentation index is derived from files on disk, so a store that
    /// predates it opens with the index empty and every memory, conclusion and
    /// project exactly as it was.
    #[tokio::test]
    async fn a_database_from_the_previous_version_opens_with_an_empty_doc_index() {
        ensure_vec0_registered();
        let conn = Connection::open_in_memory().await.unwrap();
        register_udfs(&conn).await.unwrap();
        conn.call(|c| {
            configured(crate::memory::migrations::runner())
                .set_target(refinery::Target::Version(11))
                .run(c)
        })
        .await
        .unwrap();
        conn.call(|c| {
            for (id, content) in [(1, "the daemon listens on loopback"), (2, "saves are confirmed")] {
                c.execute(
                    "INSERT INTO memory \
                     (id, scope, agent_id, project_id, content, memory_type, source, \
                      created_at, created_by_type, created_by_ref, \
                      last_modified_at, last_accessed_at, importance, tier, status) \
                     VALUES (?1, 'agent', 'a', 'repo-1a2b', ?2, 'semantic_fact', 'explicit', \
                      1, 'user', 'a', 1, 1, 5.0, 2, 'active')",
                    rusqlite::params![id, content],
                )?;
            }
            c.execute("INSERT INTO memory_derivation VALUES (2, 1, 1)", [])?;
            c.execute("INSERT INTO synthesis_review VALUES ('a', 'repo-1a2b', 1)", [])?;
            c.execute("INSERT INTO project VALUES ('repo-1a2b', '/work/repo', 1)", [])?;
            Ok::<_, rusqlite::Error>(())
        })
        .await
        .unwrap();

        let rows = |c: &mut rusqlite::Connection| -> Result<Vec<String>, rusqlite::Error> {
            let mut stmt = c.prepare(
                "SELECT json_array(id, project_id, content, tier, status) FROM memory \
                 UNION ALL SELECT json_array(memory_id, source_id) FROM memory_derivation \
                 UNION ALL SELECT json_array(agent_id, scope_key, reviewed_at) FROM synthesis_review \
                 UNION ALL SELECT json_array(id, root_path) FROM project",
            )?;
            let rows = stmt.query_map([], |r| r.get(0))?.collect();
            rows
        };
        let before = conn.call(rows).await.unwrap();

        run_migrations(&conn).await.unwrap();

        assert_eq!(conn.call(rows).await.unwrap(), before);
        let index: (i64, i64, i64, i64) = conn
            .call(|c| {
                c.query_row(
                    "SELECT (SELECT COUNT(*) FROM project_doc_file), \
                            (SELECT COUNT(*) FROM project_doc_chunk), \
                            (SELECT COUNT(*) FROM project_doc_vectors), \
                            (SELECT COUNT(*) FROM project_doc_fts)",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
            })
            .await
            .unwrap();
        assert_eq!(index, (0, 0, 0, 0));
    }

    /// Every root registered before the daemon asked whether it may read a
    /// project's files was one the user chose by working in it, so it stays
    /// indexable, and its index is kept as it was.
    #[tokio::test]
    async fn a_database_from_the_previous_version_keeps_its_projects_indexed() {
        ensure_vec0_registered();
        let conn = Connection::open_in_memory().await.unwrap();
        register_udfs(&conn).await.unwrap();
        conn.call(|c| {
            configured(crate::memory::migrations::runner())
                .set_target(refinery::Target::Version(12))
                .run(c)
        })
        .await
        .unwrap();
        conn.call(|c| {
            for (id, content) in [(1, "the daemon listens on loopback"), (2, "saves are confirmed")] {
                c.execute(
                    "INSERT INTO memory \
                     (id, scope, agent_id, project_id, content, memory_type, source, \
                      created_at, created_by_type, created_by_ref, \
                      last_modified_at, last_accessed_at, importance, tier, status) \
                     VALUES (?1, 'agent', 'a', 'repo-1a2b', ?2, 'semantic_fact', 'explicit', \
                      1, 'user', 'a', 1, 1, 5.0, 2, 'active')",
                    rusqlite::params![id, content],
                )?;
            }
            c.execute("INSERT INTO project VALUES ('repo-1a2b', '/work/repo', 1)", [])?;
            c.execute("INSERT INTO project VALUES ('web-3c4d', '/work/web', 2)", [])?;
            c.execute(
                "INSERT INTO project_doc_file VALUES ('repo-1a2b', 'README.md', 'abc', 'model', 1, 1)",
                [],
            )?;
            c.execute(
                "INSERT INTO project_doc_chunk (id, project_id, path, ordinal, breadcrumb, body) \
                 VALUES (1, 'repo-1a2b', 'README.md', 0, 'README.md', 'How the repo is laid out.')",
                [],
            )?;
            Ok::<_, rusqlite::Error>(())
        })
        .await
        .unwrap();

        let rows = |c: &mut rusqlite::Connection| -> Result<Vec<String>, rusqlite::Error> {
            let mut stmt = c.prepare(
                "SELECT json_array(id, project_id, content, tier, status) FROM memory \
                 UNION ALL SELECT json_array(id, root_path, created_at) FROM project \
                 UNION ALL SELECT json_array(project_id, path, content_hash, chunks) FROM project_doc_file \
                 UNION ALL SELECT json_array(id, project_id, breadcrumb, body) FROM project_doc_chunk",
            )?;
            let rows = stmt.query_map([], |r| r.get(0))?.collect();
            rows
        };
        let before = conn.call(rows).await.unwrap();

        run_migrations(&conn).await.unwrap();

        assert_eq!(conn.call(rows).await.unwrap(), before);
        let marks: Vec<(String, i64)> = conn
            .call(|c| {
                let mut stmt = c.prepare("SELECT id, index_docs FROM project ORDER BY id")?;
                let marks = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect();
                marks
            })
            .await
            .unwrap();
        assert_eq!(marks, vec![("repo-1a2b".to_string(), 1), ("web-3c4d".to_string(), 1)]);
    }

    /// Saves accepted but not yet stored lived only in memory before this
    /// table, so a store from before it opens with the table empty and every
    /// memory, piece link and project exactly as it was.
    #[tokio::test]
    async fn a_database_from_the_previous_version_opens_with_an_empty_outbox() {
        ensure_vec0_registered();
        let conn = Connection::open_in_memory().await.unwrap();
        register_udfs(&conn).await.unwrap();
        conn.call(|c| {
            configured(crate::memory::migrations::runner())
                .set_target(refinery::Target::Version(13))
                .run(c)
        })
        .await
        .unwrap();
        conn.call(|c| {
            for (id, content) in [(1, "the opening of a long save"), (2, "and the rest of it")] {
                c.execute(
                    "INSERT INTO memory \
                     (id, scope, agent_id, project_id, content, memory_type, source, \
                      created_at, created_by_type, created_by_ref, \
                      last_modified_at, last_accessed_at, importance, tier, status) \
                     VALUES (?1, 'agent', 'a', 'repo-1a2b', ?2, 'semantic_fact', 'explicit', \
                      1, 'user', 'a', 1, 1, 5.0, 2, 'active')",
                    rusqlite::params![id, content],
                )?;
            }
            c.execute("INSERT INTO memory_piece VALUES (1, 1, 1, ''), (1, 2, 2, 'the opening')", [])?;
            c.execute("INSERT INTO project VALUES ('repo-1a2b', '/work/repo', 1, 0)", [])?;
            Ok::<_, rusqlite::Error>(())
        })
        .await
        .unwrap();

        let rows = |c: &mut rusqlite::Connection| -> Result<Vec<String>, rusqlite::Error> {
            let mut stmt = c.prepare(
                "SELECT json_array(id, project_id, content, tier, status) FROM memory \
                 UNION ALL SELECT json_array(save_id, piece, memory_id, context) FROM memory_piece \
                 UNION ALL SELECT json_array(id, root_path, index_docs) FROM project",
            )?;
            let rows = stmt.query_map([], |r| r.get(0))?.collect();
            rows
        };
        let before = conn.call(rows).await.unwrap();

        run_migrations(&conn).await.unwrap();

        assert_eq!(conn.call(rows).await.unwrap(), before);
        let outbox: i64 = conn
            .call(|c| c.query_row("SELECT COUNT(*) FROM ingest_outbox", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(outbox, 0);
    }

    /// Disputes lived only in `disputed_against` before the pair table, so
    /// each id it names becomes an escalated pair with no distance, and every
    /// row keeps its status and its list: what was in ACTIVE CONFLICTS stays.
    #[tokio::test]
    async fn a_database_with_disputes_opens_with_escalated_pairs() {
        ensure_vec0_registered();
        let conn = Connection::open_in_memory().await.unwrap();
        register_udfs(&conn).await.unwrap();
        conn.call(|c| {
            configured(crate::memory::migrations::runner())
                .set_target(refinery::Target::Version(14))
                .run(c)
        })
        .await
        .unwrap();
        conn.call(|c| {
            let rows: [(i64, &str, Option<&str>, Option<i64>, Option<i64>); 6] = [
                (1, "active", None, None, None),
                (2, "disputed", Some("[1]"), None, Some(20)),
                (3, "active", None, None, None),
                (4, "disputed", Some("[3,1]"), None, Some(40)),
                (5, "disputed", Some("not json"), None, Some(50)),
                (6, "superseded", None, Some(2), None),
            ];
            for (id, status, against, by, disputed_at) in rows {
                c.execute(
                    "INSERT INTO memory \
                     (id, scope, agent_id, content, memory_type, source, \
                      created_at, created_by_type, created_by_ref, \
                      last_modified_at, last_accessed_at, importance, tier, status, \
                      disputed_against, superseded_by, last_disputed_at) \
                     VALUES (?1, 'agent', 'a', 'claim ' || ?1, 'semantic_fact', 'explicit', \
                      ?1, 'user', 'a', 1, 1, 5.0, 3, ?2, ?3, ?4, ?5)",
                    rusqlite::params![id, status, against, by, disputed_at],
                )?;
            }
            c.execute("UPDATE memory SET deleted_at = 99 WHERE id = 3", [])?;
            Ok::<_, rusqlite::Error>(())
        })
        .await
        .unwrap();

        let rows = |c: &mut rusqlite::Connection| -> Result<Vec<String>, rusqlite::Error> {
            let mut stmt = c.prepare(
                "SELECT json_array(id, content, status, disputed_against, superseded_by, \
                 last_disputed_at, deleted_at) FROM memory ORDER BY id",
            )?;
            let rows = stmt.query_map([], |r| r.get(0))?.collect();
            rows
        };
        let before = conn.call(rows).await.unwrap();

        run_migrations(&conn).await.unwrap();

        assert_eq!(conn.call(rows).await.unwrap(), before);
        let pairs: Vec<(i64, i64, Option<f64>, String, Option<String>, i64)> = conn
            .call(|c| {
                let mut stmt = c.prepare(
                    "SELECT new_id, old_id, distance, state, verdict, created_at FROM memory_conflict \
                     ORDER BY new_id, old_id",
                )?;
                let pairs = stmt
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))?
                    .collect();
                pairs
            })
            .await
            .unwrap();
        let escalated = |new, old, at| (new, old, None, "escalated".to_string(), None, at);
        assert_eq!(pairs, vec![escalated(2, 1, 20), escalated(4, 1, 40), escalated(4, 3, 40)]);
    }

    #[tokio::test]
    async fn a_database_from_before_topic_keys_opens_unchanged() {
        ensure_vec0_registered();
        let conn = Connection::open_in_memory().await.unwrap();
        register_udfs(&conn).await.unwrap();
        conn.call(|c| {
            configured(crate::memory::migrations::runner())
                .set_target(refinery::Target::Version(15))
                .run(c)
        })
        .await
        .unwrap();
        conn.call(|c| {
            for (id, status, by) in [(1, "active", None), (2, "superseded", Some(3)), (3, "active", None)] {
                c.execute(
                    "INSERT INTO memory \
                     (id, scope, agent_id, content, memory_type, source, \
                      created_at, created_by_type, created_by_ref, \
                      last_modified_at, last_accessed_at, importance, tier, status, superseded_by) \
                     VALUES (?1, 'agent', 'a', 'storage engine claim ' || ?1, 'architectural_decision', 'explicit', \
                      ?1, 'user', 'a', 1, 1, 5.0, 3, ?2, ?3)",
                    rusqlite::params![id, status, by],
                )?;
            }
            c.execute_batch(
                "INSERT INTO memory_piece (save_id, piece, memory_id, context) VALUES (1, 1, 1, ''), (1, 2, 3, 'head');
                 INSERT INTO memory_conflict (new_id, old_id, distance, state, created_at) VALUES (3, 1, 0.2, 'pending', 3);",
            )?;
            Ok::<_, rusqlite::Error>(())
        })
        .await
        .unwrap();

        let snapshot = |c: &mut rusqlite::Connection| -> Result<Vec<String>, rusqlite::Error> {
            let mut out = Vec::new();
            for sql in [
                "SELECT json_array(id, content, status, superseded_by, tier, access_count) FROM memory ORDER BY id",
                "SELECT json_array(save_id, piece, memory_id, context) FROM memory_piece ORDER BY save_id, piece",
                "SELECT json_array(new_id, old_id, distance, state) FROM memory_conflict ORDER BY new_id",
                "SELECT json_array(rowid) FROM memory_fts WHERE memory_fts MATCH 'engine' ORDER BY rowid",
            ] {
                let mut stmt = c.prepare(sql)?;
                let rows: Vec<String> = stmt.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
                out.extend(rows);
            }
            Ok(out)
        };
        let before = conn.call(snapshot).await.unwrap();

        run_migrations(&conn).await.unwrap();

        assert_eq!(conn.call(snapshot).await.unwrap(), before);
        let keys: Vec<Option<String>> = conn
            .call(|c| {
                let mut stmt = c.prepare("SELECT topic_key FROM memory ORDER BY id")?;
                let keys = stmt.query_map([], |r| r.get(0))?.collect();
                keys
            })
            .await
            .unwrap();
        assert_eq!(keys, vec![None, None, None]);
    }

    #[tokio::test]
    async fn a_database_from_before_revivals_opens_with_its_supersede_causes() {
        ensure_vec0_registered();
        let conn = Connection::open_in_memory().await.unwrap();
        register_udfs(&conn).await.unwrap();
        conn.call(|c| {
            configured(crate::memory::migrations::runner())
                .set_target(refinery::Target::Version(16))
                .run(c)
        })
        .await
        .unwrap();
        conn.call(|c| {
            // 2 is a revision of 1; 4 lost a verdict to 3; 6 was a revision of
            // 5 until a verdict undid it; 7 is another agent's row under 1's key.
            let rows: [(i64, &str, &str, Option<i64>, Option<&str>); 7] = [
                (1, "a", "active", None, Some("storage/engine")),
                (2, "a", "superseded", Some(1), Some("storage/engine")),
                (3, "a", "active", None, None),
                (4, "a", "superseded", Some(3), None),
                (5, "a", "active", None, Some("api/port")),
                (6, "a", "superseded", Some(5), Some("api/port")),
                (7, "b", "superseded", Some(1), Some("storage/engine")),
            ];
            for (id, agent, status, by, key) in rows {
                c.execute(
                    "INSERT INTO memory \
                     (id, scope, agent_id, project_id, content, memory_type, source, \
                      created_at, created_by_type, created_by_ref, \
                      last_modified_at, last_accessed_at, importance, tier, status, superseded_by, topic_key) \
                     VALUES (?1, 'agent', ?2, 'p', 'storage engine claim ' || ?1, 'architectural_decision', 'explicit', \
                      ?1, 'agent', ?2, 1, 1, 5.0, 3, ?3, ?4, ?5)",
                    rusqlite::params![id, agent, status, by, key],
                )?;
            }
            c.execute_batch(
                "INSERT INTO memory_piece (save_id, piece, memory_id, context) VALUES (1, 1, 1, ''), (1, 2, 3, 'head');
                 INSERT INTO memory_conflict (new_id, old_id, distance, state, verdict, created_at, judged_at)
                 VALUES (6, 5, 0.2, 'judged', 'supersedes', 6, 6), (3, 1, 0.3, 'pending', NULL, 3, NULL);",
            )?;
            Ok::<_, rusqlite::Error>(())
        })
        .await
        .unwrap();

        let snapshot = |c: &mut rusqlite::Connection| -> Result<Vec<String>, rusqlite::Error> {
            let mut out = Vec::new();
            for sql in [
                "SELECT json_array(id, content, status, superseded_by, topic_key, tier, access_count, last_modified_at) \
                 FROM memory ORDER BY id",
                "SELECT json_array(save_id, piece, memory_id, context) FROM memory_piece ORDER BY save_id, piece",
                "SELECT json_array(new_id, old_id, distance, state, verdict) FROM memory_conflict ORDER BY new_id",
                "SELECT json_array(rowid) FROM memory_fts WHERE memory_fts MATCH 'engine' ORDER BY rowid",
            ] {
                let mut stmt = c.prepare(sql)?;
                let rows: Vec<String> = stmt.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
                out.extend(rows);
            }
            Ok(out)
        };
        let before = conn.call(snapshot).await.unwrap();

        run_migrations(&conn).await.unwrap();

        assert_eq!(conn.call(snapshot).await.unwrap(), before);
        let added: Vec<(Option<String>, Option<i64>)> = conn
            .call(|c| {
                let mut stmt = c.prepare("SELECT supersede_cause, revived_from FROM memory ORDER BY id")?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect();
                rows
            })
            .await
            .unwrap();
        let causes: Vec<Option<&str>> = added.iter().map(|(cause, _)| cause.as_deref()).collect();
        assert_eq!(
            causes,
            vec![None, Some("revision"), None, Some("verdict"), None, Some("verdict"), Some("verdict")]
        );
        assert!(added.iter().all(|(_, from)| from.is_none()));
    }

    fn tamper(dir: &Path, sql: &str) {
        let c = rusqlite::Connection::open(dir.join("memory.db")).unwrap();
        c.execute_batch(sql).unwrap();
    }

    #[tokio::test]
    async fn a_newer_database_is_refused_untouched() {
        let dir = tempfile::tempdir().expect("tempdir");
        seed_on_disk(dir.path(), None).await;
        tamper(
            dir.path(),
            &format!(
                "INSERT INTO refinery_schema_history (version, name, applied_on, checksum) \
                 VALUES ({}, 'from_the_future', '2026-10-12T00:00:00Z', '1')",
                backup::latest_known() + 1
            ),
        );
        let before = std::fs::read(dir.path().join("memory.db")).unwrap();

        let err = MemoryStore::open(dir.path(), StoreHolder::Eval)
            .await
            .err()
            .expect("a newer database must not open");

        let latest = backup::latest_known();
        assert!(
            matches!(err, MemoryError::SchemaNewer { db, binary } if db == latest + 1 && binary == latest),
            "got {err:?}"
        );
        assert!(err.to_string().contains("clawg restore"), "{err}");
        assert_eq!(std::fs::read(dir.path().join("memory.db")).unwrap(), before);
    }

    #[tokio::test]
    async fn a_diverged_database_is_refused_untouched() {
        let dir = tempfile::tempdir().expect("tempdir");
        seed_on_disk(dir.path(), None).await;
        tamper(dir.path(), "UPDATE refinery_schema_history SET checksum = '1' WHERE version = 3");
        let before = std::fs::read(dir.path().join("memory.db")).unwrap();

        let err = MemoryStore::open(dir.path(), StoreHolder::Eval)
            .await
            .err()
            .expect("a diverged database must not open");

        assert!(matches!(err, MemoryError::SchemaDiverged { version: 3 }), "got {err:?}");
        assert!(err.to_string().contains("clawg restore"), "{err}");
        assert_eq!(std::fs::read(dir.path().join("memory.db")).unwrap(), before);
    }

    fn history_top(db: &Path) -> i32 {
        let c = rusqlite::Connection::open(db).unwrap();
        c.query_row("SELECT MAX(version) FROM refinery_schema_history", [], |r| r.get(0))
            .unwrap()
    }

    fn files_in(dir: &Path) -> Vec<std::path::PathBuf> {
        std::fs::read_dir(dir)
            .map(|rd| rd.map(|e| e.unwrap().path()).collect())
            .unwrap_or_default()
    }

    #[tokio::test]
    async fn opening_an_older_database_snapshots_it_before_migrating() {
        let dir = tempfile::tempdir().expect("tempdir");
        seed_on_disk(dir.path(), Some(7)).await;

        let store = MemoryStore::open(dir.path(), StoreHolder::Eval).await.unwrap();

        let snapshots = files_in(&dir.path().join("backups"));
        assert_eq!(snapshots.len(), 1, "{snapshots:?}");
        let name = snapshots[0].file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(backup::parse_snapshot_name(&name), Some(backup::Reason::PreMigration));
        assert_eq!(history_top(&snapshots[0]), 7);
        let c = rusqlite::Connection::open(&snapshots[0]).unwrap();
        let content: String =
            c.query_row("SELECT content FROM memory", [], |r| r.get(0)).unwrap();
        assert_eq!(content, "seeded on disk");

        let top: i32 = store
            .conn
            .call(|c| c.query_row("SELECT MAX(version) FROM refinery_schema_history", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(top, backup::latest_known());
    }

    #[tokio::test]
    async fn a_runner_with_a_cap_migrates_no_further_than_it() {
        ensure_vec0_registered();
        for (cap, expected) in [(Some(7), 7), (None, backup::latest_known())] {
            let conn = Connection::open_in_memory().await.unwrap();
            register_udfs(&conn).await.unwrap();

            conn.call(move |c| configured_to(crate::memory::migrations::runner(), cap).run(c))
                .await
                .unwrap();

            let top: i32 = conn
                .call(|c| c.query_row("SELECT MAX(version) FROM refinery_schema_history", [], |r| r.get(0)))
                .await
                .unwrap();
            assert_eq!(top, expected, "{cap:?}");
        }
    }

    #[tokio::test]
    async fn opening_a_current_database_takes_no_snapshot() {
        let dir = tempfile::tempdir().expect("tempdir");
        seed_on_disk(dir.path(), None).await;

        let _store = MemoryStore::open(dir.path(), StoreHolder::Eval).await.unwrap();

        assert!(files_in(&dir.path().join("backups")).is_empty());
    }

    #[tokio::test]
    async fn a_new_database_takes_no_snapshot() {
        let dir = tempfile::tempdir().expect("tempdir");
        let _store = MemoryStore::open(dir.path(), StoreHolder::Eval).await.unwrap();
        assert!(files_in(&dir.path().join("backups")).is_empty());
    }

    #[tokio::test]
    async fn a_failed_snapshot_stops_the_migration() {
        let dir = tempfile::tempdir().expect("tempdir");
        seed_on_disk(dir.path(), Some(7)).await;
        std::fs::write(dir.path().join("backups"), b"in the way").unwrap();

        let err = MemoryStore::open(dir.path(), StoreHolder::Eval)
            .await
            .err()
            .expect("no snapshot, no migration");

        assert!(matches!(err, MemoryError::Backup(_)), "got {err:?}");
        assert_eq!(history_top(&dir.path().join("memory.db")), 7);
    }

    /// Ensure_vec0_registered is idempotent (no panic on second call).
    #[tokio::test]
    async fn ensure_vec0_idempotent() {
        // Call registration twice in the same process — Once guard prevents double-init.
        ensure_vec0_registered();
        ensure_vec0_registered();
        // If we reach here without panic, the test passes.
    }

    /// Tables present after open.
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

    /// Virtual tables (vec0) present after open.
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

    /// The schema carries no views at all, and `all_memories` in particular is
    /// gone.
    ///
    /// V3 created it to scan the three memory kinds in one query; nothing ever
    /// selected from it. That was structural: flattening three tables onto one
    /// column list drops `memory_type` and every per-kind column, so callers
    /// read the tables directly instead. V7 drops it, and asserting the view
    /// list is EMPTY — rather than just that this one name is absent — is what
    /// keeps a re-created view from slipping back in unnoticed.
    #[tokio::test]
    async fn schema_carries_no_views_after_open() {
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
            views.is_empty(),
            "no view should survive the migration chain, found: {views:?}"
        );
    }

    /// An existing database that already has the view loses it on upgrade —
    /// the drop has to reach installed schemas, not just freshly built ones.
    /// `new_for_test()` runs the whole chain at once and so can never show
    /// this; the two-step run is the only shape that can.
    #[tokio::test]
    async fn v7_drops_the_view_from_an_already_migrated_database() {
        ensure_vec0_registered();
        let conn = Connection::open_in_memory().await.unwrap();
        register_udfs(&conn).await.unwrap();

        conn.call(|c| {
            crate::memory::migrations::runner()
                .set_target(refinery::Target::Version(3))
                .run(c)
        })
        .await
        .unwrap();

        let before: i64 = conn
            .call(|c| {
                c.query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='view' AND name='all_memories'",
                    [],
                    |r| r.get(0),
                )
            })
            .await
            .unwrap();
        assert_eq!(before, 1, "V3 must have created the view for this test to mean anything");

        conn.call(|c| crate::memory::migrations::runner().run(c))
            .await
            .unwrap();

        let after: i64 = conn
            .call(|c| {
                c.query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='view' AND name='all_memories'",
                    [],
                    |r| r.get(0),
                )
            })
            .await
            .unwrap();
        assert_eq!(after, 0, "V7 must drop the view on an upgraded database");
    }

    /// Indices present after open.
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

    /// Total object count.
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
        // A floor, not an exact count: vec0 and FTS5 each register their own shadow
        // tables in sqlite_master alongside the ones this schema declares directly,
        // and that shadow-table count is an extension implementation detail this
        // test does not want to pin. No view is included — V7 dropped the one this
        // schema used to carry, and nothing since has added one back.
        assert!(
            count >= 18,
            "Expected at least 18 schema objects, got {}",
            count
        );
    }

    /// MemoryStore is Clone, both handles work.
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

    /// Production open() path sets SQLITE_LIMIT_VARIABLE_NUMBER = 32766.
    ///
    /// Uses `sqlite3_limit(db, SQLITE_LIMIT_VARIABLE_NUMBER, -1)` to query the
    /// current value without changing it (SQLite contract: any negative new_val
    /// is a no-op that returns the current limit).
    #[tokio::test]
    async fn limit_variable_number_set_in_open_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = MemoryStore::open(dir.path(), StoreHolder::Eval).await.unwrap();
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

    /// Test new_for_test() path sets SQLITE_LIMIT_VARIABLE_NUMBER = 32766.
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

    // -----------------------------------------------------------------------
    // WAL durability — production open() enables journal_mode=WAL and
    // synchronous=NORMAL; the in-memory test path is deliberately untouched.
    // -----------------------------------------------------------------------

    /// WAL-1 — production open() path reports journal_mode "wal" on read-back.
    #[tokio::test]
    async fn open_sets_journal_mode_wal() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = MemoryStore::open(dir.path(), StoreHolder::Eval).await.unwrap();
        let mode: String = store
            .conn
            .call(|c| -> rusqlite::Result<String> { c.query_row("PRAGMA journal_mode", [], |r| r.get(0)) })
            .await
            .unwrap();
        assert!(mode.eq_ignore_ascii_case("wal"), "expected journal_mode wal, got {mode}");
    }

    /// WAL-2 — production open() path sets synchronous=NORMAL (value 1).
    #[tokio::test]
    async fn open_sets_synchronous_normal() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = MemoryStore::open(dir.path(), StoreHolder::Eval).await.unwrap();
        let synchronous: i64 = store
            .conn
            .call(|c| -> rusqlite::Result<i64> { c.query_row("PRAGMA synchronous", [], |r| r.get(0)) })
            .await
            .unwrap();
        assert_eq!(synchronous, 1, "expected synchronous=NORMAL (1), got {synchronous}");
    }

    /// WAL-3 — new_for_test()'s in-memory connection is deliberately left
    /// alone: no WAL pragma is issued, so journal_mode never becomes "wal".
    #[tokio::test]
    async fn new_for_test_journal_mode_is_not_wal() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let mode: String = store
            .conn
            .call(|c| -> rusqlite::Result<String> { c.query_row("PRAGMA journal_mode", [], |r| r.get(0)) })
            .await
            .unwrap();
        assert!(!mode.eq_ignore_ascii_case("wal"), "in-memory path must not be wal, got {mode}");
    }

    /// Second new_for_test() in same process succeeds.
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
        assert_eq!(count, 17, "Expected exactly 17 applied migrations, got {}", count);
    }

    /// V8 — a fresh store records V8 as applied and carries the FTS5 lexical
    /// index alongside it, not one without the other.
    #[tokio::test]
    async fn v8_migration_records_itself_and_creates_memory_fts() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let count: i64 = store
            .conn
            .call(|c| -> Result<i64, rusqlite::Error> {
                c.query_row(
                    "SELECT COUNT(*) FROM refinery_schema_history WHERE version = 8",
                    [],
                    |r| r.get(0),
                )
            })
            .await
            .unwrap();
        assert_eq!(count, 1, "a fresh store must record V8 as applied");

        let fts_exists: i64 = store
            .conn
            .call(|c| -> Result<i64, rusqlite::Error> {
                c.query_row(
                    "SELECT count(*) FROM sqlite_master WHERE name = 'memory_fts'",
                    [],
                    |r| r.get(0),
                )
            })
            .await
            .unwrap();
        assert_eq!(fts_exists, 1, "V8 must create the memory_fts virtual table");
    }

    /// A store that stopped at V7 upgrades to V8 cleanly, and a second run of
    /// the same migration set afterward is a no-op — refinery's own
    /// idempotency, exercised against the new migration specifically rather
    /// than assumed from the V6/V7 cases already covered above.
    #[tokio::test]
    async fn v7_store_upgrades_to_v8_and_a_second_run_is_a_noop() {
        ensure_vec0_registered();
        let conn = Connection::open_in_memory().await.unwrap();
        register_udfs(&conn).await.unwrap();

        conn.call(|c| {
            crate::memory::migrations::runner()
                .set_target(refinery::Target::Version(7))
                .run(c)
        })
        .await
        .unwrap();

        let before: i64 = conn
            .call(|c| c.query_row("SELECT COUNT(*) FROM refinery_schema_history", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(before, 7, "test setup: must actually stop at V7");

        let to_v8 = |c: &mut rusqlite::Connection| {
            crate::memory::migrations::runner()
                .set_target(refinery::Target::Version(8))
                .run(c)
        };
        conn.call(to_v8).await.unwrap();
        let after: i64 = conn
            .call(|c| c.query_row("SELECT COUNT(*) FROM refinery_schema_history", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(after, 8, "V8 must apply on top of an existing V7 database");

        // Re-run again: nothing left to apply, count must not move.
        conn.call(to_v8).await.unwrap();
        let after_second: i64 = conn
            .call(|c| c.query_row("SELECT COUNT(*) FROM refinery_schema_history", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(after_second, 8, "a second run with nothing pending must be a no-op");
    }

    /// The migration's backfill is what makes a memory written before V8
    /// existed lexically reachable — the three triggers only fire on writes
    /// that happen after V8 creates them, so without the backfill this row
    /// would be indexed nowhere.
    #[tokio::test]
    async fn v8_backfill_makes_a_pre_existing_row_lexically_searchable() {
        ensure_vec0_registered();
        let conn = Connection::open_in_memory().await.unwrap();
        register_udfs(&conn).await.unwrap();

        // Stop at V7: memory_fts does not exist yet, so this insert cannot be
        // caught by any trigger — it is exactly the row the backfill exists for.
        conn.call(|c| {
            crate::memory::migrations::runner()
                .set_target(refinery::Target::Version(7))
                .run(c)
        })
        .await
        .unwrap();

        conn.call(|c| {
            c.execute(
                "INSERT INTO memory \
                 (scope, agent_id, content, memory_type, source, \
                  created_at, created_by_type, created_by_ref, \
                  last_modified_at, last_accessed_at, importance, tier, status) \
                 VALUES ('agent', 'agent-v8-backfill', 'pre-v8 content mentions ZQXJ77PREEXIST', \
                  'chat_message', 'chat', 0, 'user', 'agent-v8-backfill', 0, 0, 5.0, 3, 'active')",
                [],
            )
        })
        .await
        .unwrap();

        // Now run V8 — the row above predates the FTS table and its triggers.
        conn.call(|c| crate::memory::migrations::runner().run(c))
            .await
            .unwrap();

        let found_rowid: Option<i64> = conn
            .call(|c| {
                c.query_row(
                    "SELECT rowid FROM memory_fts WHERE memory_fts MATCH '\"ZQXJ77PREEXIST\"'",
                    [],
                    |r| r.get(0),
                )
                .optional()
            })
            .await
            .unwrap();
        assert!(
            found_rowid.is_some(),
            "the backfill must make a pre-V8 row reachable through the lexical index"
        );
    }

    /// `agent_id`/`project_id`/`memory_type` on `memory_fts` are `UNINDEXED`:
    /// they are filter inputs for the candidate query's scope predicates, not
    /// search terms. If any of them were indexed instead, the literal text of
    /// a project id would become matchable by an unrelated query, and a row
    /// could surface for a search that never mentioned its content at all.
    #[tokio::test]
    async fn v8_scope_columns_do_not_contribute_searchable_tokens() {
        let store = MemoryStore::new_for_test().await.unwrap();

        // Content carries no special token; only `project_id` does. An
        // indexed `project_id` would make this row match a MATCH query for
        // its own value even though the content never mentions it.
        store
            .conn
            .call(|c| {
                c.execute(
                    "INSERT INTO memory \
                     (scope, agent_id, project_id, content, memory_type, source, \
                      created_at, created_by_type, created_by_ref, \
                      last_modified_at, last_accessed_at, importance, tier, status) \
                     VALUES ('agent', 'agent-v8-unindexed', 'PROJIDQXJ99UNIQUE', \
                      'ordinary content, no special markers here', 'chat_message', 'chat', \
                      0, 'user', 'agent-v8-unindexed', 0, 0, 5.0, 3, 'active')",
                    [],
                )
            })
            .await
            .unwrap();

        let found_rowid: Option<i64> = store
            .conn
            .call(|c| {
                c.query_row(
                    "SELECT rowid FROM memory_fts WHERE memory_fts MATCH '\"PROJIDQXJ99UNIQUE\"'",
                    [],
                    |r| r.get(0),
                )
                .optional()
            })
            .await
            .unwrap();
        assert!(
            found_rowid.is_none(),
            "a query for a project id's literal text must not match a row whose content \
             never mentions it — UNINDEXED columns must not contribute searchable tokens"
        );
    }

    /// The backfill is filtered to live rows (`WHERE deleted_at IS NULL`) —
    /// the same invariant the three triggers maintain on every write after
    /// V8. A row already soft-deleted before V8 ever ran must not become
    /// lexically reachable, or the migration's index and the triggers'
    /// index would disagree about what "live" means from the moment V8 applies.
    #[tokio::test]
    async fn v8_backfill_excludes_rows_already_soft_deleted_before_migration() {
        ensure_vec0_registered();
        let conn = Connection::open_in_memory().await.unwrap();
        register_udfs(&conn).await.unwrap();

        // Stop at V7: memory_fts does not exist yet.
        conn.call(|c| {
            crate::memory::migrations::runner()
                .set_target(refinery::Target::Version(7))
                .run(c)
        })
        .await
        .unwrap();

        // Insert a row that is ALREADY soft-deleted, predating V8 entirely —
        // the backfill is the only mechanism that could ever index it.
        conn.call(|c| {
            c.execute(
                "INSERT INTO memory \
                 (scope, agent_id, content, memory_type, source, \
                  created_at, created_by_type, created_by_ref, \
                  last_modified_at, last_accessed_at, importance, tier, status, deleted_at) \
                 VALUES ('agent', 'agent-v8-softdel', 'pre-v8 deleted content mentions ZQXJ88DELETED', \
                  'chat_message', 'chat', 0, 'user', 'agent-v8-softdel', 0, 0, 5.0, 3, 'active', 12345)",
                [],
            )
        })
        .await
        .unwrap();

        conn.call(|c| crate::memory::migrations::runner().run(c))
            .await
            .unwrap();

        let found_rowid: Option<i64> = conn
            .call(|c| {
                c.query_row(
                    "SELECT rowid FROM memory_fts WHERE memory_fts MATCH '\"ZQXJ88DELETED\"'",
                    [],
                    |r| r.get(0),
                )
                .optional()
            })
            .await
            .unwrap();
        assert!(
            found_rowid.is_none(),
            "the backfill must not index a row that was already soft-deleted before V8 ran"
        );
    }

    // -----------------------------------------------------------------------
    // Vec0 KNN
    // -----------------------------------------------------------------------

    /// INSERT vector + KNN returns rowid=1.
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
    // CHECK constraints
    // -----------------------------------------------------------------------

    /// Scope='agent' with agent_id IS NULL rejected.
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

    /// Scope='project' with project_id IS NULL rejected.
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

    /// Scope='global' with agent_id IS NOT NULL rejected.
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
    // End-to-end integration smoke test
    // -----------------------------------------------------------------------

    /// Covers tables, vec_version, KNN, clone, CHECK constraints and migration
    /// count in one run.
    #[tokio::test]
    async fn e2e_store_smoke() {
        let store = MemoryStore::new_for_test().await.unwrap();

        // Tables
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

        // The schema defines no views — V3's `all_memories` was dropped by V7
        // after shipping unused for its whole life.
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
        assert!(views.is_empty(), "found unexpected views: {views:?}");

        // Vec_version
        let version: String = store
            .conn
            .call(|c| -> Result<String, rusqlite::Error> {
                c.query_row("SELECT vec_version()", [], |r| r.get(0))
            })
            .await
            .unwrap();
        assert!(!version.is_empty());

        // KNN insert + search
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

        // Clone
        let s2 = store.clone();
        let v2: String = s2
            .conn
            .call(|c| -> Result<String, rusqlite::Error> {
                c.query_row("SELECT vec_version()", [], |r| r.get(0))
            })
            .await
            .unwrap();
        assert!(!v2.is_empty());

        // CHECK constraints
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

        // Migration count
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
        assert_eq!(migration_count, 17);
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

    // -----------------------------------------------------------------------
    // Exclusive ownership — the store lock is taken before SQLite is touched
    // and lives as long as any clone of the store.
    // -----------------------------------------------------------------------

    /// A store whose lock is held elsewhere must refuse to open, and must not
    /// have touched `memory.db` on the way out: creating or migrating the file
    /// is exactly what the holder is entitled to do alone.
    #[tokio::test]
    async fn open_refuses_a_held_store_and_never_creates_memory_db() {
        let dir = tempfile::tempdir().expect("tempdir");
        let _holder = StoreLock::acquire(dir.path(), StoreHolder::Daemon).expect("holder");

        let err = MemoryStore::open(dir.path(), StoreHolder::Eval)
            .await
            .err()
            .expect("a held store must not open");
        match err {
            MemoryError::StoreInUse { owner: Some(o) } => assert_eq!(o.holder, StoreHolder::Daemon),
            other => panic!("expected StoreInUse naming the daemon, got {other:?}"),
        }
        assert!(
            !dir.path().join("memory.db").exists(),
            "the loser must never create or migrate the database"
        );
    }

    /// `MemoryStore` is `Clone`, and the runtime hands clones to the ingest
    /// worker and the maintenance loop: the store is only free once the LAST
    /// clone is gone, not the first.
    #[tokio::test]
    async fn the_lock_lives_as_long_as_any_clone_of_the_store() {
        let dir = tempfile::tempdir().expect("tempdir");
        let a = MemoryStore::open(dir.path(), StoreHolder::Daemon).await.expect("open a");
        let b = a.clone();

        drop(a);
        assert!(
            matches!(
                MemoryStore::open(dir.path(), StoreHolder::Eval).await.err(),
                Some(MemoryError::StoreInUse { .. })
            ),
            "a clone is still alive, so the store must still be held"
        );

        drop(b);
        MemoryStore::open(dir.path(), StoreHolder::Eval)
            .await
            .map(|_| ())
            .expect("the last clone is gone, so the store is free");
    }

    /// The in-memory test store has nothing to exclude anybody from.
    #[tokio::test]
    async fn new_for_test_takes_no_lock() {
        let dir = tempfile::tempdir().expect("tempdir");
        let _t = MemoryStore::new_for_test().await.expect("test store");
        StoreLock::acquire(dir.path(), StoreHolder::Daemon).expect("nothing to collide with");
    }
}
