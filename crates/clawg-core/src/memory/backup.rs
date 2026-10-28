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

/// The migrations embedded in this binary; a `testing` build stamped with a
/// cap knows only up to it.
pub fn known_migrations() -> Vec<MigrationId> {
    let all = crate::memory::migrations::runner()
        .get_migrations()
        .iter()
        .map(|m| MigrationId {
            version: m.version(),
            name: m.name().to_string(),
            checksum: m.checksum(),
        })
        .collect();
    capped(all, crate::build_info::migrations_up_to())
}

/// `migrations` without those past `up_to`.
fn capped(migrations: Vec<MigrationId>, up_to: Option<i32>) -> Vec<MigrationId> {
    match up_to {
        Some(newest) => migrations.into_iter().filter(|m| m.version <= newest).collect(),
        None => migrations,
    }
}

/// The newest migration this binary knows, so tests do not assume a count
/// that the next migration changes.
#[cfg(test)]
pub(crate) fn latest_known() -> i32 {
    known_migrations().iter().map(|m| m.version).max().unwrap_or(0)
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
    /// Before the daemon re-embedded every stored vector with its model.
    PreReembed,
    /// Before `clawg import` wrote rows from an export.
    PreImport,
    /// Before `clawg projects move` moved rows to another project.
    PreMove,
    /// Before registering a root joined rows of other ids into its project.
    PreRekey,
    /// Before `clawg import --undo` took an import back.
    PreUndo,
    /// Before `clawg projects merge` joined one project into another.
    PreMerge,
    /// Before `clawg projects delete` deleted a project with its memories.
    PreDelete,
}

impl Reason {
    const ALL: [Reason; 10] = [
        Reason::PreMigration,
        Reason::PreRestore,
        Reason::Manual,
        Reason::PreReembed,
        Reason::PreImport,
        Reason::PreMove,
        Reason::PreRekey,
        Reason::PreUndo,
        Reason::PreMerge,
        Reason::PreDelete,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Reason::PreMigration => "pre-migration",
            Reason::PreRestore => "pre-restore",
            Reason::Manual => "manual",
            Reason::PreReembed => "pre-reembed",
            Reason::PreImport => "pre-import",
            Reason::PreMove => "pre-move",
            Reason::PreRekey => "pre-rekey",
            Reason::PreUndo => "pre-undo",
            Reason::PreMerge => "pre-merge",
            Reason::PreDelete => "pre-delete",
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
    #[error("cannot restore {path}: {reason}. Nothing was changed")]
    Unusable { path: PathBuf, reason: String },
    #[error("another connection kept {path} from being checkpointed. Nothing was changed")]
    Busy { path: PathBuf },
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

/// When the snapshot `name` was taken, in milliseconds since the epoch, read
/// off the name [`snapshot_name`] gave it: a copy or a restore of the file
/// changes its modification time, never its name. `None` for anything else.
pub fn parse_snapshot_time(name: &str) -> Option<i64> {
    parse_snapshot_name(name)?;
    let stamp = name.strip_prefix("memory-")?.get(..23)?;
    let field = |range: std::ops::Range<usize>| stamp.get(range)?.parse::<i64>().ok();
    let (y, mo, d) = (field(0..4)?, field(5..7)?, field(8..10)?);
    let (h, mi, s, milli) = (field(11..13)?, field(14..16)?, field(17..19)?, field(20..23)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || s > 59 {
        return None;
    }
    let days = days_from_civil(y, mo as u32, d as u32);
    Some(days * 86_400_000 + h * 3_600_000 + mi * 60_000 + s * 1000 + milli)
}

/// A proleptic Gregorian (year, month, day) to days since 1970-01-01, the
/// inverse of [`civil_from_days`]. Howard Hinnant's `days_from_civil`.
pub(crate) fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (i64::from(m) + 9) % 12;
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Days since 1970-01-01 to a proleptic Gregorian (year, month, day).
/// Howard Hinnant's `civil_from_days`.
pub(crate) fn civil_from_days(z: i64) -> (i64, u32, u32) {
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

/// How many automatic snapshots (`pre-migration`, `pre-restore`, `pre-import`,
/// `pre-reembed`, `pre-move`) are kept. One is taken per upgrade that migrates,
/// per restore, per import, per re-embed or per batch move, so five cover
/// several in a row. A starting
/// value, not measured.
pub const AUTO_KEEP: usize = 5;

/// The automatic snapshots among `names` that fall outside the newest
/// [`AUTO_KEEP`], except the newest of each reason: a burst of restores or
/// re-embeds must not push out the `pre-migration` snapshot that is the way
/// back from an upgrade. Manual snapshots and names this module did not
/// produce are never chosen.
pub fn to_prune(names: &[String]) -> Vec<String> {
    let mut automatic: Vec<(&String, Reason)> = names
        .iter()
        .filter_map(|n| parse_snapshot_name(n).filter(|r| r.is_automatic()).map(|r| (n, r)))
        .collect();
    automatic.sort_by(|a, b| a.0.cmp(b.0));
    let excess = automatic.len().saturating_sub(AUTO_KEEP);
    let mut newest_of: Vec<Reason> = Vec::new();
    let mut kept_newest: Vec<&String> = Vec::new();
    for (name, reason) in automatic.iter().rev() {
        if !newest_of.contains(reason) {
            newest_of.push(*reason);
            kept_newest.push(name);
        }
    }
    automatic
        .into_iter()
        .take(excess)
        .filter(|(name, _)| !kept_newest.contains(name))
        .map(|(name, _)| name.clone())
        .collect()
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
    crate::private_dir::ensure_private_dir(dir)
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
    if let Err(source) = sync_dir(dir) {
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
    let path = base_dir.join("memory.db");
    if !path.exists() {
        return Err(BackupError::NoStore { path }.into());
    }
    let _lock = StoreLock::acquire(base_dir, StoreHolder::Maintenance)?;
    let c = open_existing(base_dir)?;
    Ok(snapshot_into(&c, &crate::paths::backups_dir(base_dir), Reason::Manual)?)
}

/// What [`restore`] did.
#[derive(Debug)]
pub struct Restored {
    /// The snapshot of the store it replaced; `None` when there was none.
    pub previous: Option<PathBuf>,
}

/// Replaces `<base_dir>/memory.db` with the database at `from`.
///
/// Only on a stopped store: the lock is taken first, so a running daemon makes
/// this fail before anything is read. `from` is checked before anything is
/// touched, and the store it replaces is snapshotted first; either failing
/// leaves the store as it was.
pub fn restore(base_dir: &Path, from: &Path) -> Result<Restored, MemoryError> {
    // A machine that never ran clawg has no data directory yet, and restoring
    // onto one is how a store moves to it.
    crate::private_dir::ensure_private_dir(base_dir)
        .map_err(|source| BackupError::Dir { path: base_dir.to_path_buf(), source })?;
    let _lock = StoreLock::acquire(base_dir, StoreHolder::Maintenance)?;
    check_restorable(from)?;

    let db = base_dir.join("memory.db");
    let tmp = base_dir.join("memory.db.restore-tmp");
    let restored = stage(from, &tmp)
        .map_err(|source| MemoryError::from(BackupError::Io { path: tmp.clone(), source }))
        .and_then(|()| keep_and_replace(base_dir, &db, &tmp));
    if restored.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    restored
}

/// Copied before the snapshot of the current store is taken: that snapshot
/// prunes old automatic ones, and `from` may be one of them.
fn stage(from: &Path, tmp: &Path) -> std::io::Result<()> {
    std::fs::copy(from, tmp)?;
    // Windows flushes only through a handle opened for writing; a read-only one
    // fails with "Access is denied".
    std::fs::OpenOptions::new().write(true).open(tmp)?.sync_all()
}

fn keep_and_replace(base_dir: &Path, db: &Path, tmp: &Path) -> Result<Restored, MemoryError> {
    let previous = if db.exists() {
        let c = open_existing(base_dir)?;
        let snapshot = snapshot_into(&c, &crate::paths::backups_dir(base_dir), Reason::PreRestore)?;
        // An empty WAL is what makes deleting it below lose nothing.
        let busy: i64 = c.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0))?;
        if busy != 0 {
            return Err(BackupError::Busy { path: db.to_path_buf() }.into());
        }
        drop(c);
        Some(snapshot)
    } else {
        None
    };
    swap_in(tmp, db).map_err(|source| BackupError::Io { path: db.to_path_buf(), source })?;
    Ok(Restored { previous })
}

/// The WAL goes BEFORE the rename: the other way round, a crash in between
/// would leave the old store's WAL beside the new file, and SQLite would apply
/// it on the next open.
fn swap_in(tmp: &Path, db: &Path) -> std::io::Result<()> {
    for suffix in ["-wal", "-shm"] {
        let mut side = db.as_os_str().to_owned();
        side.push(suffix);
        match std::fs::remove_file(PathBuf::from(side)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
    }
    std::fs::rename(tmp, db)?;
    match db.parent() {
        Some(dir) => sync_dir(dir),
        None => Ok(()),
    }
}

/// Makes a rename in `dir` survive a power cut. On Unix the rename lives in
/// the directory entry, which reaches the disk only when the directory itself
/// is synced: without it a `pre-migration` snapshot could be gone after a
/// crash although the migration it guards was applied. NTFS journals the
/// rename with the file system, and Windows cannot open a directory this way.
#[cfg(unix)]
pub fn sync_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::File::open(dir)?.sync_all()
}

#[cfg(not(unix))]
pub fn sync_dir(_dir: &Path) -> std::io::Result<()> {
    Ok(())
}

/// The `-wal` beside `path` when it holds commits, read from its size alone:
/// the file it sits beside lacks them, so a copy of that file alone is not the
/// store that wrote them.
pub(crate) fn wal_with_commits(path: &Path) -> Option<PathBuf> {
    let mut wal = path.as_os_str().to_owned();
    wal.push("-wal");
    let wal = PathBuf::from(wal);
    std::fs::metadata(&wal).is_ok_and(|m| m.len() > 0).then_some(wal)
}

/// Whether `path` is a whole clawg store this binary can open, read without
/// writing to it, and what opening it would do to its schema.
pub(crate) fn check_restorable(path: &Path) -> Result<SchemaPlan, BackupError> {
    let unusable = |reason: String| BackupError::Unusable { path: path.to_path_buf(), reason };
    if !path.is_file() {
        return Err(unusable("there is no such file".into()));
    }
    // Opened here, the file reads with its WAL folded in, but only the file is
    // copied in: what the WAL holds would pass every check and then be gone.
    if let Some(wal) = wal_with_commits(path) {
        return Err(unusable(format!(
            "{} beside it holds commits the file alone does not have, and only the file is restored; \
             restore a snapshot `clawg backup` wrote, or copy the store again with nothing using it",
            wal.display()
        )));
    }
    crate::memory::db::ensure_vec0_registered();
    let c = rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| unusable(e.to_string()))?;
    let check: String = c
        .query_row("PRAGMA quick_check", [], |r| r.get(0))
        .map_err(|e| unusable(e.to_string()))?;
    if check != "ok" {
        return Err(unusable(format!("it is damaged ({check})")));
    }
    let history = read_history(&c).map_err(|e| unusable(e.to_string()))?;
    match schema_plan(&history, &known_migrations()) {
        SchemaPlan::Fresh => Err(unusable("it has no clawg migration history".into())),
        SchemaPlan::Newer { db, binary } => Err(unusable(format!(
            "it is at schema V{db}, newer than the V{binary} this clawg knows"
        ))),
        SchemaPlan::Diverged { version } => Err(unusable(format!(
            "its migration V{version} does not match this clawg build"
        ))),
        plan @ (SchemaPlan::UpToDate | SchemaPlan::Pending { .. }) => Ok(plan),
    }
}

/// What a stopped store's file says about its schema, for `clawg status`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreProbe {
    pub state: ProbeState,
    /// The newest migration applied to the file, when it could be read.
    pub db_version: Option<i32>,
    /// The newest migration this binary knows.
    pub binary_version: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeState {
    Missing,
    Unreadable(String),
    Schema(SchemaPlan),
}

/// Reads the schema history of `<base_dir>/memory.db` without the lock and
/// without writing: a probe must never stand in the way of a daemon starting
/// at the same moment, nor create a store where there is none.
pub fn probe(base_dir: &Path) -> StoreProbe {
    let known = known_migrations();
    let binary_version = known.iter().map(|m| m.version).max().unwrap_or(0);
    let path = base_dir.join("memory.db");
    if !path.exists() {
        return StoreProbe { state: ProbeState::Missing, db_version: None, binary_version };
    }
    let read = open_for_probe(&path).and_then(|c| read_history(&c));
    match read {
        Ok(applied) => StoreProbe {
            db_version: applied.iter().map(|m| m.version).max(),
            state: ProbeState::Schema(schema_plan(&applied, &known)),
            binary_version,
        },
        Err(e) => StoreProbe { state: ProbeState::Unreadable(e.to_string()), db_version: None, binary_version },
    }
}

/// A read-only connection still creates `-wal` and `-shm` beside a WAL-mode
/// file, and cannot delete them on close. A stopped store has neither, so it
/// is opened `immutable` and nothing appears. With a `-wal` left by a crash,
/// the commits in it matter, so it is read through a plain read-only open.
pub(crate) fn open_for_probe(path: &Path) -> rusqlite::Result<rusqlite::Connection> {
    let mut wal = path.as_os_str().to_owned();
    wal.push("-wal");
    let read_only = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
    match immutable_uri(path) {
        Some(uri) if !Path::new(&wal).exists() => {
            rusqlite::Connection::open_with_flags(uri, read_only | rusqlite::OpenFlags::SQLITE_OPEN_URI)
        }
        _ => rusqlite::Connection::open_with_flags(path, read_only),
    }
}

/// `file:` URI for `path` with `immutable=1`; `None` when the path is not
/// UTF-8. Backslashes become slashes and a drive letter gets the leading slash
/// SQLite expects on Windows.
fn immutable_uri(path: &Path) -> Option<String> {
    let raw = path.to_str()?.replace('\\', "/");
    let mut uri = String::from(if raw.starts_with('/') { "file://" } else { "file:///" });
    for b in raw.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/:".contains(&b) {
            uri.push(b as char);
        } else {
            uri.push_str(&format!("%{b:02X}"));
        }
    }
    uri.push_str("?immutable=1");
    Some(uri)
}

pub(crate) fn open_existing(base_dir: &Path) -> Result<rusqlite::Connection, MemoryError> {
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
        assert_eq!(
            snapshot_name(0, Reason::PreReembed),
            "memory-1970-01-01T00-00-00.000Z-pre-reembed.db"
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

    #[test]
    fn snapshot_times_come_from_their_names() {
        let stamps = [0, 999, 86_399_999, 951_782_400_000, 1_709_164_800_000, 1_791_827_476_123, 4_102_444_800_000];
        for ms in stamps {
            assert_eq!(parse_snapshot_time(&snapshot_name(ms, Reason::PreImport)), Some(ms), "{ms}");
        }
        for other in ["notes.txt", "memory-2026-10-12T17-51-16.123Z-other.db", "memory-2026-13-12T17-51-16.123Z-manual.db"] {
            assert_eq!(parse_snapshot_time(other), None, "{other}");
        }
    }

    #[test]
    fn a_cap_hides_the_migrations_past_it() {
        let all: Vec<MigrationId> = (1..=5).map(|v| MigrationId { version: v, name: format!("m{v}"), checksum: v as u64 }).collect();

        assert_eq!(capped(all.clone(), None), all);
        assert_eq!(capped(all.clone(), Some(5)), all);
        let versions: Vec<i32> = capped(all.clone(), Some(3)).iter().map(|m| m.version).collect();
        assert_eq!(versions, [1, 2, 3]);
        assert!(capped(all, Some(0)).is_empty());
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

    #[test]
    fn pruning_keeps_the_newest_snapshot_of_every_automatic_reason() {
        let migration = snapshot_name(1_700_000_000_000, Reason::PreMigration);
        let reembed = snapshot_name(1_700_000_001_000, Reason::PreReembed);
        let restores: Vec<String> = (0..AUTO_KEEP as i64 + 2)
            .map(|i| snapshot_name(1_700_000_010_000 + i * 1000, Reason::PreRestore))
            .collect();
        let mut names = vec![migration.clone(), reembed.clone()];
        names.extend(restores.iter().cloned());

        // Only restores outside the newest AUTO_KEEP go; the lone migration and
        // re-embed snapshots are the way back from their operations.
        assert_eq!(to_prune(&names), restores[..2].to_vec());
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
        for base in [dir.path().to_path_buf(), dir.path().join("never-created")] {
            let err = backup_offline(&base).unwrap_err();
            assert!(matches!(err, MemoryError::Backup(BackupError::NoStore { .. })), "got {err:?}");
        }
        assert!(!dir.path().join("never-created").exists());
    }

    #[tokio::test]
    async fn restore_onto_a_machine_without_a_data_directory() {
        let source = tempfile::tempdir().unwrap();
        seed_on_disk(source.path(), None).await;
        let snapshot = backup_offline(source.path()).unwrap();
        let home = tempfile::tempdir().unwrap();
        let base = home.path().join(".clawg-dev");

        let restored = restore(&base, &snapshot).unwrap();

        assert!(restored.previous.is_none());
        assert_eq!(contents(&base.join("memory.db")), vec!["seeded on disk".to_string()]);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn restore_creates_a_private_data_directory() {
        let source = tempfile::tempdir().unwrap();
        seed_on_disk(source.path(), None).await;
        let snapshot = backup_offline(source.path()).unwrap();
        let home = tempfile::tempdir().unwrap();
        let base = home.path().join(".clawg-dev");

        restore(&base, &snapshot).unwrap();

        assert_eq!(crate::private_dir::mode_of(&base), 0o700);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_snapshot_directory_is_created_private() {
        let dir = tempfile::tempdir().unwrap();
        seed_on_disk(dir.path(), None).await;

        let path = backup_offline(dir.path()).unwrap();

        assert_eq!(crate::private_dir::mode_of(path.parent().unwrap()), 0o700);
    }

    fn add_row(dir: &Path, content: &str) {
        let c = rusqlite::Connection::open(dir.join("memory.db")).unwrap();
        c.execute(INSERT_ROW, [content]).unwrap();
    }

    #[tokio::test]
    async fn restore_replaces_the_store_and_keeps_the_previous_one() {
        let dir = tempfile::tempdir().unwrap();
        seed_on_disk(dir.path(), None).await;
        let only_a = backup_offline(dir.path()).unwrap();
        add_row(dir.path(), "added after the backup");

        let restored = restore(dir.path(), &only_a).unwrap();

        assert_eq!(contents(&dir.path().join("memory.db")), vec!["seeded on disk".to_string()]);
        let previous = restored.previous.expect("the replaced store was kept");
        assert_eq!(parse_snapshot_name(&previous.file_name().unwrap().to_string_lossy()), Some(Reason::PreRestore));
        assert_eq!(
            contents(&previous),
            vec!["seeded on disk".to_string(), "added after the backup".to_string()]
        );
        let leftovers: Vec<String> =
            files_in(dir.path()).into_iter().filter(|n| n.contains("restore-tmp")).collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
        StoreLock::acquire(dir.path(), StoreHolder::Daemon).expect("the lock was released");
    }

    #[tokio::test]
    async fn restore_refuses_files_it_cannot_open_safely() {
        let dir = tempfile::tempdir().unwrap();
        seed_on_disk(dir.path(), None).await;
        let elsewhere = tempfile::tempdir().unwrap();

        let text = elsewhere.path().join("notes.db");
        std::fs::write(&text, "not a database at all, just some words").unwrap();
        let foreign = elsewhere.path().join("foreign.db");
        rusqlite::Connection::open(&foreign).unwrap().execute_batch("CREATE TABLE x (y)").unwrap();
        let newer_dir = tempfile::tempdir().unwrap();
        seed_on_disk(newer_dir.path(), None).await;
        rusqlite::Connection::open(newer_dir.path().join("memory.db"))
            .unwrap()
            .execute_batch(
                &format!(
                    "INSERT INTO refinery_schema_history (version, name, applied_on, checksum) \
                     VALUES ({}, 'from_the_future', '2026-10-12T00:00:00Z', '1')",
                    latest_known() + 1
                ),
            )
            .unwrap();
        let newer = newer_dir.path().join("memory.db");
        let missing = elsewhere.path().join("missing.db");

        let before = std::fs::read(dir.path().join("memory.db")).unwrap();
        for (file, why) in [
            (&text, "not a database"),
            (&foreign, "no clawg migration history"),
            (&newer, "newer than"),
            (&missing, "no such file"),
        ] {
            let err = restore(dir.path(), file).unwrap_err();
            assert!(
                matches!(err, MemoryError::Backup(BackupError::Unusable { .. })),
                "{}: got {err:?}",
                file.display()
            );
            let msg = err.to_string();
            assert!(msg.contains(&file.display().to_string()) && msg.contains(why), "{msg}");
        }
        assert_eq!(std::fs::read(dir.path().join("memory.db")).unwrap(), before);
        assert!(files_in(&dir.path().join("backups")).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn a_directory_syncs_and_a_missing_one_says_so() {
        let dir = tempfile::tempdir().unwrap();
        sync_dir(dir.path()).expect("a directory syncs");
        assert!(sync_dir(&dir.path().join("missing")).is_err());
    }

    /// A copy of a store something still had open carries commits in its
    /// `-wal` that the file alone lacks, and a restore installs only the file.
    #[tokio::test]
    async fn restore_refuses_a_copy_with_commits_in_its_wal() {
        let live = tempfile::tempdir().unwrap();
        let store = MemoryStore::open(live.path(), StoreHolder::Eval).await.unwrap();
        insert(&store, "only in the WAL").await;
        let elsewhere = tempfile::tempdir().unwrap();
        let copy = elsewhere.path().join("copy.db");
        std::fs::copy(live.path().join("memory.db"), &copy).unwrap();
        std::fs::copy(live.path().join("memory.db-wal"), elsewhere.path().join("copy.db-wal")).unwrap();
        drop(store);
        let target = tempfile::tempdir().unwrap();
        seed_on_disk(target.path(), None).await;
        let before = std::fs::read(target.path().join("memory.db")).unwrap();

        let err = restore(target.path(), &copy).unwrap_err();

        assert!(matches!(err, MemoryError::Backup(BackupError::Unusable { .. })), "{err:?}");
        assert!(err.to_string().contains("copy.db-wal"), "{err}");
        assert_eq!(std::fs::read(target.path().join("memory.db")).unwrap(), before);
        assert!(files_in(&target.path().join("backups")).is_empty());
    }

    #[tokio::test]
    async fn restoring_the_oldest_kept_automatic_snapshot_survives_the_pruning() {
        let dir = tempfile::tempdir().unwrap();
        seed_on_disk(dir.path(), None).await;
        let manual = backup_offline(dir.path()).unwrap();
        let backups = dir.path().join("backups");
        let automatic: Vec<PathBuf> = (0..AUTO_KEEP as i64)
            .map(|i| backups.join(snapshot_name(1_600_000_000_000 + i, Reason::PreMigration)))
            .collect();
        for path in &automatic {
            std::fs::copy(&manual, path).unwrap();
        }
        add_row(dir.path(), "added after the backup");

        restore(dir.path(), &automatic[0]).unwrap();

        assert_eq!(contents(&dir.path().join("memory.db")), vec!["seeded on disk".to_string()]);
    }

    #[tokio::test]
    async fn restore_refuses_while_the_daemon_holds_the_store() {
        let dir = tempfile::tempdir().unwrap();
        seed_on_disk(dir.path(), None).await;
        let snapshot = backup_offline(dir.path()).unwrap();
        let moved = dir.path().join("kept.db");
        std::fs::rename(&snapshot, &moved).unwrap();
        let _daemon = StoreLock::acquire(dir.path(), StoreHolder::Daemon).unwrap();

        let err = restore(dir.path(), &moved).unwrap_err();

        match err {
            MemoryError::StoreInUse { owner: Some(o) } => assert_eq!(o.holder, StoreHolder::Daemon),
            other => panic!("expected StoreInUse naming the daemon, got {other:?}"),
        }
        assert!(files_in(&dir.path().join("backups")).is_empty());
    }

    #[tokio::test]
    async fn a_restored_older_snapshot_migrates_on_the_next_open() {
        let older = tempfile::tempdir().unwrap();
        seed_on_disk(older.path(), Some(7)).await;
        let v7 = backup_offline(older.path()).unwrap();
        let dir = tempfile::tempdir().unwrap();
        seed_on_disk(dir.path(), None).await;

        restore(dir.path(), &v7).unwrap();
        assert_eq!(history_top(&dir.path().join("memory.db")), 7);
        let store = MemoryStore::open(dir.path(), StoreHolder::Eval).await.unwrap();

        let top: i32 = store
            .conn
            .call(|c| c.query_row("SELECT MAX(version) FROM refinery_schema_history", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(top, latest_known());
        let reasons: Vec<Option<Reason>> = files_in(&dir.path().join("backups"))
            .iter()
            .map(|n| parse_snapshot_name(n))
            .collect();
        assert!(reasons.contains(&Some(Reason::PreMigration)), "{reasons:?}");
        assert!(reasons.contains(&Some(Reason::PreRestore)), "{reasons:?}");
    }

    #[tokio::test]
    async fn restoring_over_a_store_migrated_by_a_newer_version_keeps_it_as_pre_restore() {
        let older = tempfile::tempdir().unwrap();
        seed_on_disk(older.path(), None).await;
        let snapshot = backup_offline(older.path()).unwrap();
        let dir = tempfile::tempdir().unwrap();
        seed_on_disk(dir.path(), None).await;
        add_row(dir.path(), "written by the newer version");
        rusqlite::Connection::open(dir.path().join("memory.db"))
            .unwrap()
            .execute_batch(&format!(
                "INSERT INTO refinery_schema_history (version, name, applied_on, checksum) \
                 VALUES ({}, 'from_the_future', '2026-10-12T00:00:00Z', '1')",
                latest_known() + 1
            ))
            .unwrap();
        assert!(matches!(probe(dir.path()).state, ProbeState::Schema(SchemaPlan::Newer { .. })));

        let restored = restore(dir.path(), &snapshot).unwrap();

        let kept = restored.previous.expect("the migrated store was kept");
        assert_eq!(parse_snapshot_name(&kept.file_name().unwrap().to_string_lossy()), Some(Reason::PreRestore));
        assert_eq!(history_top(&kept), latest_known() + 1);
        assert_eq!(
            contents(&kept),
            vec!["seeded on disk".to_string(), "written by the newer version".to_string()]
        );
        assert_eq!(history_top(&dir.path().join("memory.db")), latest_known());
        assert_eq!(contents(&dir.path().join("memory.db")), vec!["seeded on disk".to_string()]);
        assert_eq!(probe(dir.path()).state, ProbeState::Schema(SchemaPlan::UpToDate));
    }

    #[tokio::test]
    async fn probing_a_newer_store_reads_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        // Through the production open, so the file is in WAL mode and was
        // closed the way a stopped daemon leaves it.
        let store = MemoryStore::open(dir.path(), StoreHolder::Eval).await.unwrap();
        store.conn.clone().close().await.unwrap();
        drop(store);
        rusqlite::Connection::open(dir.path().join("memory.db"))
            .unwrap()
            .execute_batch(
                &format!(
                    "INSERT INTO refinery_schema_history (version, name, applied_on, checksum) \
                     VALUES ({}, 'from_the_future', '2026-10-12T00:00:00Z', '1')",
                    latest_known() + 1
                ),
            )
            .unwrap();
        let files = files_in(dir.path());
        let before = std::fs::read(dir.path().join("memory.db")).unwrap();

        let probe = probe(dir.path());

        assert_eq!(
            probe,
            StoreProbe {
                state: ProbeState::Schema(SchemaPlan::Newer {
                    db: latest_known() + 1,
                    binary: latest_known(),
                }),
                db_version: Some(latest_known() + 1),
                binary_version: latest_known(),
            }
        );
        assert_eq!(std::fs::read(dir.path().join("memory.db")).unwrap(), before);
        assert_eq!(files_in(dir.path()), files);
    }

    #[test]
    fn immutable_uris_escape_what_a_uri_cannot_hold() {
        assert_eq!(
            immutable_uri(Path::new("/home/u/.clawg-dev/memory.db")).unwrap(),
            "file:///home/u/.clawg-dev/memory.db?immutable=1"
        );
        assert_eq!(
            immutable_uri(Path::new("/tmp/a b?#%/memory.db")).unwrap(),
            "file:///tmp/a%20b%3F%23%25/memory.db?immutable=1"
        );
        assert_eq!(
            immutable_uri(Path::new("C:\\Users\\José\\.clawg\\memory.db")).unwrap(),
            "file:///C:/Users/Jos%C3%A9/.clawg/memory.db?immutable=1"
        );
    }

    #[test]
    fn probing_without_a_store_creates_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(probe(dir.path()).state, ProbeState::Missing);
        assert!(files_in(dir.path()).is_empty());
    }

    #[test]
    fn probing_a_file_that_is_not_a_store_says_unreadable() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("memory.db"), "not a database at all, just some words").unwrap();
        assert!(matches!(probe(dir.path()).state, ProbeState::Unreadable(_)));
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
