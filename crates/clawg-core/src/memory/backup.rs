//! Snapshots of `memory.db`, and the schema check that decides when one is due.

use std::path::{Path, PathBuf};

use crate::memory::store_lock::{StoreHolder, StoreLock};
use crate::memory::MemoryError;

/// One migration as the schema plan compares it: applied to a database
/// (`refinery_schema_history`) or embedded in this binary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationId {
    pub version: i32,
    pub name: String,
    pub checksum: u64,
}

/// Reads `refinery_schema_history`, empty when the table does not exist yet.
///
/// Refinery stores the checksum as the decimal text of a `u64`.
pub fn read_history(c: &rusqlite::Connection) -> rusqlite::Result<Vec<MigrationId>> {
    let exists: bool = c.query_row(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master \
         WHERE type = 'table' AND name = 'refinery_schema_history')",
        [],
        |r| r.get(0),
    )?;
    if !exists {
        return Ok(Vec::new());
    }
    let mut stmt = c.prepare("SELECT version, name, checksum FROM refinery_schema_history")?;
    let rows = stmt.query_map([], |r| {
        let checksum: String = r.get(2)?;
        let checksum = checksum.parse::<u64>().map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, Box::new(e))
        })?;
        Ok(MigrationId { version: r.get(0)?, name: r.get(1)?, checksum })
    })?;
    rows.collect()
}

/// The migrations embedded in this binary.
pub fn known_migrations() -> Vec<MigrationId> {
    crate::memory::migrations::runner()
        .get_migrations()
        .iter()
        .map(|m| MigrationId {
            version: m.version(),
            name: m.name().to_string(),
            checksum: m.checksum(),
        })
        .collect()
}

/// What opening a database with this binary would do to its schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaPlan {
    /// No migration applied yet: a new database.
    Fresh,
    /// Exactly this binary's migrations.
    UpToDate,
    /// A prefix of this binary's migrations; `from + 1 ..= to` are pending.
    Pending { from: i32, to: i32 },
    /// Written by a newer binary: version `db` is past the last one this
    /// binary knows, `binary`.
    Newer { db: i32, binary: i32 },
    /// Applied version `version` differs from this binary's (name or
    /// checksum), or is a hole this binary would never fill.
    Diverged { version: i32 },
}

/// Compares a database's applied migrations with this binary's.
///
/// A divergence inside the range this binary knows wins over `Newer`: a
/// binary that disagrees about its own migrations has a different problem
/// than one that is merely older.
pub fn schema_plan(applied: &[MigrationId], known: &[MigrationId]) -> SchemaPlan {
    let Some(db) = applied.iter().map(|m| m.version).max() else {
        return SchemaPlan::Fresh;
    };
    let binary = known.iter().map(|m| m.version).max().unwrap_or(0);

    let mut in_range: Vec<&MigrationId> = applied.iter().filter(|m| m.version <= binary).collect();
    in_range.sort_by_key(|m| m.version);
    let mut expected: Vec<&MigrationId> = known.iter().collect();
    expected.sort_by_key(|m| m.version);

    for (i, a) in in_range.iter().enumerate() {
        match expected.get(i) {
            Some(k) if k.version == a.version => {
                if k.name != a.name || k.checksum != a.checksum {
                    return SchemaPlan::Diverged { version: a.version };
                }
            }
            // The binary has a migration the database skipped, or the
            // database has one the binary never had.
            Some(k) => return SchemaPlan::Diverged { version: k.version.min(a.version) },
            None => return SchemaPlan::Diverged { version: a.version },
        }
    }

    if db > binary {
        SchemaPlan::Newer { db, binary }
    } else if db < binary {
        SchemaPlan::Pending { from: db, to: binary }
    } else {
        SchemaPlan::UpToDate
    }
}

/// Why a snapshot was taken. Part of its file name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// Before this binary applied pending migrations.
    PreMigration,
    /// Before `clawg restore` replaced the store.
    PreRestore,
    /// `clawg backup`.
    Manual,
}

impl Reason {
    const ALL: [Reason; 3] = [Reason::PreMigration, Reason::PreRestore, Reason::Manual];

    pub fn as_str(self) -> &'static str {
        match self {
            Reason::PreMigration => "pre-migration",
            Reason::PreRestore => "pre-restore",
            Reason::Manual => "manual",
        }
    }

    fn is_automatic(self) -> bool {
        !matches!(self, Reason::Manual)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    #[error("could not create the backup directory {path}: {source}")]
    Dir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("there is no memory store at {path}")]
    NoStore { path: PathBuf },
    #[error("cannot snapshot to {path}: SQLite needs the path as UTF-8")]
    NonUtf8Path { path: PathBuf },
    #[error("could not write the snapshot {path}: {source}")]
    Snapshot {
        path: PathBuf,
        #[source]
        source: rusqlite::Error,
    },
    #[error("could not finish the snapshot {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// `memory-<UTC date>T<hh-mm-ss.mmm>Z-<reason>.db`, so that alphabetical order
/// is time order. Dashes instead of colons: Windows refuses `:` in a file name.
pub fn snapshot_name(ms: i64, reason: Reason) -> String {
    let (days, ms_of_day) = (ms.div_euclid(86_400_000), ms.rem_euclid(86_400_000));
    let (y, mo, d) = civil_from_days(days);
    let (h, mi, s, milli) = (
        ms_of_day / 3_600_000,
        ms_of_day / 60_000 % 60,
        ms_of_day / 1000 % 60,
        ms_of_day % 1000,
    );
    format!(
        "memory-{y:04}-{mo:02}-{d:02}T{h:02}-{mi:02}-{s:02}.{milli:03}Z-{}.db",
        reason.as_str()
    )
}

/// The reason in a name [`snapshot_name`] produced; `None` for anything else.
pub fn parse_snapshot_name(name: &str) -> Option<Reason> {
    let stem = name.strip_prefix("memory-")?.strip_suffix(".db")?;
    let (stamp, reason) = stem.split_once("Z-")?;
    let well_formed = stamp.len() == 23
        && stamp.bytes().enumerate().all(|(i, b)| match i {
            4 | 7 | 13 | 16 => b == b'-',
            10 => b == b'T',
            19 => b == b'.',
            _ => b.is_ascii_digit(),
        });
    if !well_formed {
        return None;
    }
    Reason::ALL.into_iter().find(|r| r.as_str() == reason)
}

/// Days since 1970-01-01 to a proleptic Gregorian (year, month, day).
/// Howard Hinnant's `civil_from_days`.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// How many automatic snapshots (`pre-migration`, `pre-restore`) are kept.
/// One is taken per upgrade that migrates or per restore, so five cover
/// several in a row. A starting value, not measured.
pub const AUTO_KEEP: usize = 5;

/// The automatic snapshots among `names` that fall outside the newest
/// [`AUTO_KEEP`]. Manual snapshots and names this module did not produce are
/// never chosen.
pub fn to_prune(names: &[String]) -> Vec<String> {
    let mut automatic: Vec<&String> = names
        .iter()
        .filter(|n| parse_snapshot_name(n).is_some_and(Reason::is_automatic))
        .collect();
    automatic.sort();
    let excess = automatic.len().saturating_sub(AUTO_KEEP);
    automatic.into_iter().take(excess).cloned().collect()
}

/// Best effort: a snapshot that cannot be deleted is one too many, never an
/// error.
fn prune(dir: &Path) {
    let names: Vec<String> = match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect(),
        Err(e) => {
            log::warn!("could not list {} to prune old snapshots: {e}", dir.display());
            return;
        }
    };
    for name in to_prune(&names) {
        if let Err(e) = std::fs::remove_file(dir.join(&name)) {
            log::warn!("could not delete the old snapshot {name}: {e}");
        }
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Writes a full copy of the database behind `c` into `dir` and returns its
/// path.
///
/// `VACUUM INTO` reads inside one transaction, so the copy is consistent and
/// includes what is still in the WAL; the result has no WAL of its own. It is
/// written under a `.tmp` name and renamed when complete, so an interrupted
/// snapshot never passes for a finished one.
pub fn snapshot_into(
    c: &rusqlite::Connection,
    dir: &Path,
    reason: Reason,
) -> Result<PathBuf, BackupError> {
    std::fs::create_dir_all(dir)
        .map_err(|source| BackupError::Dir { path: dir.to_path_buf(), source })?;
    let path = dir.join(snapshot_name(now_ms(), reason));
    let tmp = path.with_extension("db.tmp");
    let target = tmp.to_str().ok_or_else(|| BackupError::NonUtf8Path { path: tmp.clone() })?;

    let _ = std::fs::remove_file(&tmp);
    if let Err(source) = c.execute("VACUUM INTO ?1", [target]) {
        let _ = std::fs::remove_file(&tmp);
        return Err(BackupError::Snapshot { path, source });
    }
    if let Err(source) = std::fs::rename(&tmp, &path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(BackupError::Io { path, source });
    }
    if reason.is_automatic() {
        prune(dir);
    }
    Ok(path)
}

/// Snapshots a store no process is serving, without migrating it: a backup
/// copies what is there, not what a newer binary would make of it.
///
/// Holds the store lock for the duration, so neither a daemon nor another
/// backup can start in between.
pub fn backup_offline(base_dir: &Path) -> Result<PathBuf, MemoryError> {
    let _lock = StoreLock::acquire(base_dir, StoreHolder::Maintenance)?;
    let c = open_existing(base_dir)?;
    Ok(snapshot_into(&c, &crate::paths::backups_dir(base_dir), Reason::Manual)?)
}

fn open_existing(base_dir: &Path) -> Result<rusqlite::Connection, MemoryError> {
    let path = base_dir.join("memory.db");
    if !path.exists() {
        return Err(BackupError::NoStore { path }.into());
    }
    crate::memory::db::ensure_vec0_registered();
    Ok(rusqlite::Connection::open_with_flags(
        &path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::db::seed_on_disk;
    use crate::memory::MemoryStore;

    const INSERT_ROW: &str = "INSERT INTO memory \
        (scope, agent_id, content, memory_type, source, \
         created_at, created_by_type, created_by_ref, \
         last_modified_at, last_accessed_at, importance, tier, status) \
        VALUES ('agent', 'agent-b', ?1, 'chat_message', 'chat', \
         0, 'user', 'agent-b', 0, 0, 5.0, 3, 'active')";

    async fn insert(store: &MemoryStore, content: &'static str) -> i64 {
        store
            .conn
            .call(move |c| -> rusqlite::Result<i64> {
                c.execute(INSERT_ROW, [content])?;
                Ok(c.last_insert_rowid())
            })
            .await
            .unwrap()
    }

    fn contents(db: &Path) -> Vec<String> {
        let c = rusqlite::Connection::open(db).unwrap();
        let mut stmt = c.prepare("SELECT content FROM memory ORDER BY id").unwrap();
        let rows = stmt.query_map([], |r| r.get(0)).unwrap();
        rows.map(Result::unwrap).collect()
    }

    fn files_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .map(|rd| rd.map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect())
            .unwrap_or_default();
        names.sort();
        names
    }

    #[test]
    fn snapshot_names_sort_in_time_order() {
        assert_eq!(
            snapshot_name(1_791_827_476_123, Reason::Manual),
            "memory-2026-10-12T17-51-16.123Z-manual.db"
        );
        assert_eq!(
            snapshot_name(0, Reason::PreMigration),
            "memory-1970-01-01T00-00-00.000Z-pre-migration.db"
        );
        assert_eq!(
            snapshot_name(1_709_164_800_000, Reason::PreRestore),
            "memory-2024-02-29T00-00-00.000Z-pre-restore.db"
        );

        let stamps = [0, 999, 86_399_999, 1_709_164_800_000, 1_791_827_476_123, 4_102_444_800_000];
        let names: Vec<String> = stamps.iter().map(|&ms| snapshot_name(ms, Reason::Manual)).collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
    }

    #[test]
    fn snapshot_names_parse_back_to_their_reason() {
        for reason in Reason::ALL {
            assert_eq!(parse_snapshot_name(&snapshot_name(1_791_827_476_123, reason)), Some(reason));
        }
        for other in [
            "notes.txt",
            "memory.db",
            "memory-2026-10-12T17-51-16.123Z-manual.db.tmp",
            "memory-2026-10-12T17-51-16.123Z-other.db",
            "memory-2026-10-12T17:51:16.123Z-manual.db",
        ] {
            assert_eq!(parse_snapshot_name(other), None, "{other}");
        }
    }

    #[tokio::test]
    async fn a_snapshot_includes_writes_still_in_the_wal() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::open(dir.path(), StoreHolder::Eval).await.unwrap();
        insert(&store, "only in the wal").await;
        let wal = dir.path().join("memory.db-wal");
        assert!(wal.metadata().map(|m| m.len() > 0).unwrap_or(false), "the write is in the WAL");

        let backups = dir.path().join("backups");
        let path = store
            .conn
            .call(move |c| -> rusqlite::Result<_> { Ok(snapshot_into(c, &backups, Reason::Manual)) })
            .await
            .unwrap()
            .unwrap();

        assert_eq!(parse_snapshot_name(&path.file_name().unwrap().to_string_lossy()), Some(Reason::Manual));
        assert_eq!(contents(&path), vec!["only in the wal".to_string()]);
    }

    #[tokio::test]
    async fn a_snapshot_keeps_vectors_and_full_text_searchable() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::open(dir.path(), StoreHolder::Eval).await.unwrap();
        let id = insert(&store, "a searchable sentence about lighthouses").await;
        store
            .conn
            .call(move |c| -> rusqlite::Result<()> {
                let mut v = vec![0f32; 384];
                v[0] = 1.0;
                let blob: Vec<u8> = v.iter().flat_map(|f| f.to_le_bytes()).collect();
                c.execute(
                    "INSERT INTO memory_vectors (rowid, embedding) VALUES (?1, ?2)",
                    rusqlite::params![id, blob],
                )?;
                Ok(())
            })
            .await
            .unwrap();

        let backups = dir.path().join("backups");
        let path = store
            .conn
            .call(move |c| -> rusqlite::Result<_> { Ok(snapshot_into(c, &backups, Reason::Manual)) })
            .await
            .unwrap()
            .unwrap();

        let c = rusqlite::Connection::open(&path).unwrap();
        let lexical: i64 = c
            .query_row("SELECT rowid FROM memory_fts WHERE memory_fts MATCH 'lighthouses'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(lexical, id);
        let mut q = vec![0f32; 384];
        q[0] = 1.0;
        let q: Vec<u8> = q.iter().flat_map(|f| f.to_le_bytes()).collect();
        let nearest: i64 = c
            .query_row(
                "SELECT rowid FROM memory_vectors WHERE embedding MATCH ?1 AND k = 1",
                [q],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(nearest, id);
    }

    fn auto_and_manual_names() -> (Vec<String>, Vec<String>) {
        let auto: Vec<String> = (0..7)
            .map(|i| {
                let reason = if i % 2 == 0 { Reason::PreMigration } else { Reason::PreRestore };
                snapshot_name(1_700_000_000_000 + i * 1000, reason)
            })
            .collect();
        let manual: Vec<String> =
            (0..2).map(|i| snapshot_name(1_600_000_000_000 + i, Reason::Manual)).collect();
        (auto, manual)
    }

    #[test]
    fn pruning_keeps_the_newest_automatic_snapshots_and_every_manual_one() {
        let (auto, manual) = auto_and_manual_names();
        let mut names: Vec<String> = manual.clone();
        names.extend(auto.iter().rev().cloned());
        names.push("notes.txt".into());

        assert_eq!(to_prune(&names), auto[..2].to_vec());
        assert_eq!(to_prune(&auto[..AUTO_KEEP]), Vec::<String>::new());
    }

    #[tokio::test]
    async fn an_automatic_snapshot_prunes_older_automatic_ones() {
        let dir = tempfile::tempdir().unwrap();
        let backups = dir.path().join("backups");
        std::fs::create_dir_all(&backups).unwrap();
        let (auto, manual) = auto_and_manual_names();
        for name in auto.iter().chain(&manual).chain(std::iter::once(&"notes.txt".to_string())) {
            std::fs::write(backups.join(name), b"").unwrap();
        }
        let store = MemoryStore::open(dir.path(), StoreHolder::Eval).await.unwrap();

        let target = backups.clone();
        let new = store
            .conn
            .call(move |c| -> rusqlite::Result<_> { Ok(snapshot_into(c, &target, Reason::PreMigration)) })
            .await
            .unwrap()
            .unwrap();

        let mut expected: Vec<String> = auto[3..].to_vec();
        expected.push(new.file_name().unwrap().to_string_lossy().into_owned());
        expected.extend(manual);
        expected.push("notes.txt".into());
        expected.sort();
        assert_eq!(files_in(&backups), expected);
    }

    fn history_top(db: &Path) -> i32 {
        let c = rusqlite::Connection::open(db).unwrap();
        c.query_row("SELECT MAX(version) FROM refinery_schema_history", [], |r| r.get(0))
            .unwrap()
    }

    #[tokio::test]
    async fn an_offline_backup_copies_without_migrating() {
        let dir = tempfile::tempdir().unwrap();
        seed_on_disk(dir.path(), Some(7)).await;

        let path = backup_offline(dir.path()).unwrap();

        assert_eq!(parse_snapshot_name(&path.file_name().unwrap().to_string_lossy()), Some(Reason::Manual));
        assert_eq!(contents(&path), vec!["seeded on disk".to_string()]);
        assert_eq!(history_top(&path), 7);
        assert_eq!(history_top(&dir.path().join("memory.db")), 7, "the store was not migrated");
        StoreLock::acquire(dir.path(), StoreHolder::Daemon).expect("the lock was released");
    }

    #[tokio::test]
    async fn an_offline_backup_refuses_a_held_store() {
        let dir = tempfile::tempdir().unwrap();
        seed_on_disk(dir.path(), None).await;
        let _daemon = StoreLock::acquire(dir.path(), StoreHolder::Daemon).unwrap();

        let err = backup_offline(dir.path()).unwrap_err();

        match err {
            MemoryError::StoreInUse { owner: Some(o) } => assert_eq!(o.holder, StoreHolder::Daemon),
            other => panic!("expected StoreInUse naming the daemon, got {other:?}"),
        }
        assert!(files_in(&dir.path().join("backups")).is_empty());
    }

    #[test]
    fn an_offline_backup_of_nothing_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let err = backup_offline(dir.path()).unwrap_err();
        assert!(matches!(err, MemoryError::Backup(BackupError::NoStore { .. })), "got {err:?}");
    }

    #[tokio::test]
    async fn a_failed_snapshot_leaves_no_file_behind() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::open(dir.path(), StoreHolder::Eval).await.unwrap();
        let backups = dir.path().join("backups");
        let target = backups.clone();

        // VACUUM refuses to run inside an open transaction.
        let result = store
            .conn
            .call(move |c| -> rusqlite::Result<_> {
                c.execute_batch("BEGIN")?;
                let result = snapshot_into(c, &target, Reason::Manual);
                c.execute_batch("ROLLBACK")?;
                Ok(result)
            })
            .await
            .unwrap();

        assert!(matches!(result, Err(BackupError::Snapshot { .. })), "got {result:?}");
        assert_eq!(files_in(&backups), Vec::<String>::new());
    }

    fn m(version: i32, checksum: u64) -> MigrationId {
        MigrationId { version, name: format!("m{version}"), checksum }
    }

    fn range(to: i32) -> Vec<MigrationId> {
        (1..=to).map(|v| m(v, v as u64 * 10)).collect()
    }

    #[test]
    fn schema_plan_covers_every_history_shape() {
        let known = range(8);

        assert_eq!(schema_plan(&[], &known), SchemaPlan::Fresh);
        assert_eq!(schema_plan(&range(8), &known), SchemaPlan::UpToDate);
        assert_eq!(schema_plan(&range(5), &known), SchemaPlan::Pending { from: 5, to: 8 });
        assert_eq!(schema_plan(&range(9), &known), SchemaPlan::Newer { db: 9, binary: 8 });

        let mut altered = range(8);
        altered[2].checksum = 1;
        assert_eq!(schema_plan(&altered, &known), SchemaPlan::Diverged { version: 3 });

        let mut renamed = range(8);
        renamed[3].name = "other".into();
        assert_eq!(schema_plan(&renamed, &known), SchemaPlan::Diverged { version: 4 });

        let holed: Vec<MigrationId> = range(8).into_iter().filter(|m| m.version != 6).collect();
        assert_eq!(schema_plan(&holed, &known), SchemaPlan::Diverged { version: 6 });
    }

    #[test]
    fn a_divergence_the_binary_knows_about_wins_over_newer() {
        let mut applied = range(9);
        applied[2].checksum = 1;
        assert_eq!(schema_plan(&applied, &range(8)), SchemaPlan::Diverged { version: 3 });
    }

    #[test]
    fn history_order_does_not_matter() {
        let mut applied = range(5);
        applied.reverse();
        assert_eq!(schema_plan(&applied, &range(8)), SchemaPlan::Pending { from: 5, to: 8 });
    }
}
