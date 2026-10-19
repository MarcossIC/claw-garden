//! Secrets already in the store: finding the rows that hold one, and purging
//! a row for good.
//!
//! New saves are masked on the way in (see [`crate::memory::redact`]); this
//! is for what was stored before, which nothing rewrites on its own.

use std::path::{Path, PathBuf};

use rusqlite::OptionalExtension;

use crate::memory::backup::{
    known_migrations, open_existing, open_for_probe, parse_snapshot_name, read_history, schema_plan,
    BackupError, SchemaPlan,
};
use crate::memory::clock::TimestampMs;
use crate::memory::dispute::release_challengers;
use crate::memory::redact::{redact, SecretKind};
use crate::memory::store_lock::{StoreHolder, StoreLock};
use crate::memory::MemoryError;

/// A `memory` row whose content holds at least one secret. Carries what the
/// row is and what it holds, never the content itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Flagged {
    pub id: i64,
    pub kinds: Vec<SecretKind>,
    pub deleted: bool,
}

/// A pair of `memory_conflict` whose verdict reason holds a secret, by the
/// ids of its two rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlaggedPair {
    pub new_id: i64,
    pub old_id: i64,
    pub kinds: Vec<SecretKind>,
}

/// What [`scan`] found, each list in id order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scanned {
    pub memories: Vec<Flagged>,
    pub pairs: Vec<FlaggedPair>,
}

/// Every row of `memory`, live or deleted, and every verdict reason that
/// holds a secret.
///
/// Reads without the lock and without writing, so it works beside a running
/// daemon: WAL lets a reader in while the daemon writes.
pub fn scan(base_dir: &Path) -> Result<Scanned, MemoryError> {
    let path = base_dir.join("memory.db");
    if !path.exists() {
        return Err(BackupError::NoStore { path }.into());
    }
    crate::memory::db::ensure_vec0_registered();
    let c = open_for_probe(&path)?;
    let mut stmt = c.prepare("SELECT id, content, deleted_at IS NOT NULL FROM memory ORDER BY id")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, bool>(2)?)))?;
    let mut scanned = Scanned::default();
    for row in rows {
        let (id, content, deleted) = row?;
        let kinds = redact(&content).kinds;
        if !kinds.is_empty() {
            scanned.memories.push(Flagged { id, kinds, deleted });
        }
    }
    scanned.pairs = reasons_with_a_secret(&c)?.into_iter().map(|(pair, _)| pair).collect();
    Ok(scanned)
}

/// The verdict reasons that hold a secret, with each reason as stored. A
/// store from before the pairs has none.
fn reasons_with_a_secret(c: &rusqlite::Connection) -> rusqlite::Result<Vec<(FlaggedPair, String)>> {
    if !has_table(c, "memory_conflict")? {
        return Ok(Vec::new());
    }
    let mut stmt =
        c.prepare("SELECT new_id, old_id, reason FROM memory_conflict WHERE reason IS NOT NULL ORDER BY new_id, old_id")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?)))?;
    let mut flagged = Vec::new();
    for row in rows {
        let (new_id, old_id, reason) = row?;
        let kinds = redact(&reason).kinds;
        if !kinds.is_empty() {
            flagged.push((FlaggedPair { new_id, old_id, kinds }, reason));
        }
    }
    Ok(flagged)
}

fn has_table(c: &rusqlite::Connection, name: &str) -> rusqlite::Result<bool> {
    c.query_row("SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = ?1", [name], |r| {
        r.get::<_, i64>(0)
    })
    .map(|n| n > 0)
}

/// Why a purge did not run.
#[derive(Debug, thiserror::Error)]
pub enum PurgeError {
    #[error("there is no memory #{0}")]
    NotFound(i64),
    #[error("memory.db has migrations pending (V{from} to V{to}); start the daemon once so it \
             migrates them, stop it, and purge then")]
    Pending { from: i32, to: i32 },
    #[error("memory.db stayed busy after the purge, so its WAL could not be emptied; \
             make sure no other clawg is running and purge again")]
    Busy,
}

/// What [`purge`] did.
#[derive(Debug, PartialEq, Eq)]
pub struct Purged {
    /// Snapshots in `<data>/backups` that still hold the row, by name. None
    /// is changed: deleting them is the user's call.
    pub backups: Vec<PathBuf>,
}

/// Removes `memory` row `id` for good: the row, its vector, its terms in the
/// full-text index and the outbox rows holding its text, so that text is in
/// neither `memory.db` nor its WAL.
///
/// Only on a stopped store that is up to date: the lock is taken first, and
/// nothing is written before every check passes. No snapshot is taken first,
/// on purpose: it would be one more copy of what is being removed.
pub fn purge(base_dir: &Path, id: i64) -> Result<Purged, MemoryError> {
    let path = base_dir.join("memory.db");
    if !path.exists() {
        return Err(BackupError::NoStore { path }.into());
    }
    let _lock = StoreLock::acquire(base_dir, StoreHolder::Maintenance)?;
    let mut c = open_existing(base_dir)?;
    match schema_plan(&read_history(&c)?, &known_migrations()) {
        SchemaPlan::UpToDate => {}
        SchemaPlan::Fresh => return Err(PurgeError::NotFound(id).into()),
        SchemaPlan::Pending { from, to } => return Err(PurgeError::Pending { from, to }.into()),
        SchemaPlan::Newer { db, binary } => return Err(MemoryError::SchemaNewer { db, binary }),
        SchemaPlan::Diverged { version } => return Err(MemoryError::SchemaDiverged { version }),
    }
    let (content, live): (String, bool) = c
        .query_row("SELECT content, deleted_at IS NULL FROM memory WHERE id = ?1", [id], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .optional()?
        .ok_or(PurgeError::NotFound(id))?;

    let tx = c.transaction()?;
    tx.execute("DELETE FROM memory_vectors WHERE rowid = ?1", [id])?;
    tx.execute("DELETE FROM memory WHERE id = ?1", [id])?;
    // A purged conclusion rests on nothing. A link that names the row as a
    // source stays: it holds an id, never text, and it is the only way the
    // conclusion learns it lost a source. `AUTOINCREMENT` never hands the id
    // to a later row.
    tx.execute("DELETE FROM memory_derivation WHERE memory_id = ?1", [id])?;
    tx.execute("DELETE FROM memory_conflict WHERE new_id = ?1 OR old_id = ?1", [id])?;
    // A save not stored yet, or that failed after it was accepted, holds the
    // text in the outbox; a row stored from a long save is one of its pieces.
    tx.execute("DELETE FROM ingest_outbox WHERE instr(content, ?1) > 0", [&content])?;
    // After the delete, so the row no longer counts as live for them.
    if live {
        release_challengers(&tx, id, TimestampMs::now())?;
    }
    tx.commit()?;
    // FTS5 records a removal as a new entry that carries the row's terms, and
    // keeps the old segments until a merge: both still hold the text. A full
    // merge rewrites the index without them.
    c.execute("INSERT INTO memory_fts(memory_fts) VALUES ('optimize')", [])?;
    // Pages freed long before this purge (every merge of the index frees
    // some) still hold whatever they held, and `secure_delete` only zeroes
    // pages freed from now on. Rebuilding the file leaves no free page. It
    // keeps the rowids vec0 addresses its chunks by: a table with an index is
    // copied with its rowids, and every vec0 shadow table has one.
    c.execute_batch("VACUUM")?;
    // Older frames of the WAL can still hold the pages as they were.
    let busy: i64 = c.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0))?;
    if busy != 0 {
        return Err(PurgeError::Busy.into());
    }
    drop(c);
    Ok(Purged { backups: backups_holding(&crate::paths::backups_dir(base_dir), id, &content) })
}

/// The snapshots in `dir` with a row `id` whose content is `content`, or an
/// outbox row holding it, by name. One that cannot be read is listed too: it
/// may hold the row.
fn backups_holding(dir: &Path, id: i64, content: &str) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut holding: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.file_name().and_then(|n| n.to_str()).and_then(parse_snapshot_name).is_some())
        .filter(|p| {
            let Ok(c) = open_for_probe(p) else { return true };
            let in_memory = c
                .query_row(
                    "SELECT count(*) FROM memory WHERE id = ?1 AND content = ?2",
                    rusqlite::params![id, content],
                    |r| r.get::<_, i64>(0),
                )
                .map_or(true, |n| n > 0);
            in_memory || outbox_holds(&c, content)
        })
        .collect();
    holding.sort();
    holding
}

/// Whether a snapshot's outbox has a row with `content` in it. A snapshot
/// from before the outbox has no such table, and so cannot hold one.
fn outbox_holds(c: &rusqlite::Connection, content: &str) -> bool {
    let has_table = c
        .query_row("SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'ingest_outbox'", [], |r| {
            r.get::<_, i64>(0)
        })
        .map_or(true, |n| n > 0);
    has_table
        && c.query_row("SELECT count(*) FROM ingest_outbox WHERE instr(content, ?1) > 0", [content], |r| {
            r.get::<_, i64>(0)
        })
        .map_or(true, |n| n > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::MemoryStore;

    const INSERT_ROW: &str = "INSERT INTO memory \
        (scope, agent_id, content, memory_type, source, \
         created_at, created_by_type, created_by_ref, \
         last_modified_at, last_accessed_at, importance, tier, status) \
        VALUES ('agent', 'agent-s', ?1, 'semantic_fact', 'explicit', \
         0, 'user', 'agent-s', 0, 0, 5.0, 3, 'active')";

    // Built at run time so no literal in the repository looks like a live
    // credential to a secret scanner.
    fn fake(prefix: &str, unit: &str, len: usize) -> String {
        format!("{prefix}{}", unit.chars().cycle().take(len).collect::<String>())
    }

    async fn insert(store: &MemoryStore, content: String) -> i64 {
        store
            .conn
            .call(move |c| -> rusqlite::Result<i64> {
                c.execute(INSERT_ROW, [content])?;
                Ok(c.last_insert_rowid())
            })
            .await
            .unwrap()
    }

    async fn soft_delete(store: &MemoryStore, id: i64) {
        store
            .conn
            .call(move |c| c.execute("UPDATE memory SET deleted_at = 1 WHERE id = ?1", [id]))
            .await
            .unwrap();
    }

    fn files_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .map(|rd| rd.map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect())
            .unwrap_or_default();
        names.sort();
        names
    }

    #[tokio::test]
    async fn the_scan_finds_live_and_deleted_rows_without_their_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::open(dir.path(), StoreHolder::Eval).await.unwrap();
        let aws = insert(&store, format!("staging uses {}", fake("AKIA", "Q7ZX", 16))).await;
        insert(&store, "nothing secret here".to_string()).await;
        let jwt = insert(
            &store,
            format!("session {}", fake("eyJ", "hbGc", 20) + "." + &fake("eyJ", "zdWI", 20) + ".sig"),
        )
        .await;
        soft_delete(&store, jwt).await;
        store.conn.clone().close().await.unwrap();
        drop(store);
        let before = files_in(dir.path());

        assert_eq!(
            scan(dir.path()).unwrap().memories,
            vec![
                Flagged { id: aws, kinds: vec![SecretKind::AwsAccessKey], deleted: false },
                Flagged { id: jwt, kinds: vec![SecretKind::Jwt], deleted: true },
            ]
        );
        assert_eq!(files_in(dir.path()), before, "the scan wrote nothing beside the store");
    }

    #[tokio::test]
    async fn the_scan_flags_rows_saved_before_a_new_rule() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::open(dir.path(), StoreHolder::Eval).await.unwrap();
        let password = insert(&store, "db password=hunter2".to_string()).await;
        let marked = insert(&store, "card <private>4111 1111</private>".to_string()).await;
        store.conn.clone().close().await.unwrap();
        drop(store);

        assert_eq!(
            scan(dir.path()).unwrap().memories,
            vec![
                Flagged { id: password, kinds: vec![SecretKind::Password], deleted: false },
                Flagged { id: marked, kinds: vec![SecretKind::Private], deleted: false },
            ]
        );
    }

    async fn add_pair(store: &MemoryStore, new: i64, old: i64, reason: Option<String>) {
        store
            .conn
            .call(move |c| {
                c.execute(
                    "INSERT INTO memory_conflict (new_id, old_id, distance, state, verdict, reason, created_at) \
                     VALUES (?1, ?2, 0.3, 'judged', 'scoped', ?3, 0)",
                    rusqlite::params![new, old, reason],
                )
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn the_scan_finds_reasons_that_hold_a_secret() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::open(dir.path(), StoreHolder::Eval).await.unwrap();
        let a = insert(&store, "the deploy runs on fridays".to_string()).await;
        let b = insert(&store, "the deploy runs on mondays".to_string()).await;
        let c = insert(&store, "the deploy runs on sundays".to_string()).await;
        add_pair(&store, b, a, Some(format!("rotated {}", fake("ghp_", "a1B2", 36)))).await;
        add_pair(&store, c, a, Some("one is the dev profile".to_string())).await;
        add_pair(&store, c, b, None).await;
        store.conn.clone().close().await.unwrap();
        drop(store);

        let found = scan(dir.path()).unwrap();

        assert!(found.memories.is_empty(), "{:?}", found.memories);
        assert_eq!(found.pairs, vec![FlaggedPair { new_id: b, old_id: a, kinds: vec![SecretKind::GithubToken] }]);
    }

    #[tokio::test]
    async fn the_scan_of_a_store_from_before_the_pairs_lists_rows_only() {
        let dir = tempfile::tempdir().unwrap();
        crate::memory::db::seed_on_disk(dir.path(), Some(14)).await;
        rusqlite::Connection::open(dir.path().join("memory.db"))
            .unwrap()
            .execute(INSERT_ROW, [format!("ci token {}", fake("ghp_", "a1B2", 36))])
            .unwrap();

        let found = scan(dir.path()).unwrap();

        assert_eq!(found.memories.len(), 1);
        assert!(found.pairs.is_empty());
    }

    #[tokio::test]
    async fn the_scan_reads_while_another_process_holds_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::open(dir.path(), StoreHolder::Eval).await.unwrap();
        let id = insert(&store, format!("ci token {}", fake("ghp_", "a1B2", 36))).await;
        assert!(matches!(
            StoreLock::acquire(dir.path(), StoreHolder::Maintenance),
            Err(MemoryError::StoreInUse { .. })
        ));

        let found = scan(dir.path()).unwrap();

        assert_eq!(found.memories, vec![Flagged { id, kinds: vec![SecretKind::GithubToken], deleted: false }]);
        drop(store);
    }

    const SECRET: &str = "the vault passphrase is zebra-quokka-7741";

    /// A closed WAL-mode store with 300 ordinary rows and the secret one,
    /// which has a vector. Returns the secret row's id.
    async fn store_with_a_secret(dir: &Path, deleted: bool) -> i64 {
        let store = MemoryStore::open(dir, StoreHolder::Eval).await.unwrap();
        for i in 0..300 {
            let row = insert(&store, format!("ordinary row {i} about trains and buses")).await;
            add_vector(&store, row, i).await;
        }
        let id = insert(&store, SECRET.to_string()).await;
        add_vector(&store, id, 300).await;
        if deleted {
            soft_delete(&store, id).await;
        }
        store.conn.clone().close().await.unwrap();
        drop(store);
        id
    }

    /// A unit vector along axis `axis`, so a KNN probe along the same axis
    /// finds exactly this row.
    fn axis(axis: usize) -> Vec<u8> {
        let mut v = vec![0f32; 384];
        v[axis] = 1.0;
        v.iter().flat_map(|f| f.to_le_bytes()).collect()
    }

    async fn add_vector(store: &MemoryStore, id: i64, n: usize) {
        let blob = axis(n % 384);
        store
            .conn
            .call(move |c| {
                c.execute("INSERT INTO memory_vectors (rowid, embedding) VALUES (?1, ?2)", rusqlite::params![id, blob])
            })
            .await
            .unwrap();
    }

    fn nearest(dir: &Path, n: usize) -> i64 {
        crate::memory::db::ensure_vec0_registered();
        let c = rusqlite::Connection::open(dir.join("memory.db")).unwrap();
        c.query_row("SELECT rowid FROM memory_vectors WHERE embedding MATCH ?1 AND k = 1", [axis(n)], |r| r.get(0))
            .unwrap()
    }

    fn bytes_mention(dir: &Path, needle: &[u8]) -> bool {
        ["memory.db", "memory.db-wal"].iter().any(|name| {
            std::fs::read(dir.join(name))
                .map(|bytes| bytes.windows(needle.len()).any(|w| w == needle))
                .unwrap_or(false)
        })
    }

    fn count(dir: &Path, sql: &str, id: i64) -> i64 {
        crate::memory::db::ensure_vec0_registered();
        let c = rusqlite::Connection::open(dir.join("memory.db")).unwrap();
        c.query_row(sql, [id], |r| r.get(0)).unwrap()
    }

    fn assert_no_trace(dir: &Path, id: i64) {
        assert_eq!(count(dir, "SELECT count(*) FROM memory WHERE id = ?1", id), 0);
        assert_eq!(count(dir, "SELECT count(*) FROM memory_vectors WHERE rowid = ?1", id), 0);
        assert_eq!(
            count(dir, "SELECT count(*) FROM memory_fts WHERE memory_fts MATCH 'quokka' AND rowid >= ?1", 0),
            0
        );
        assert!(!bytes_mention(dir, b"quokka"), "the secret's bytes are still on disk");
    }

    #[tokio::test]
    async fn purging_a_live_row_leaves_no_trace_of_its_text() {
        let dir = tempfile::tempdir().unwrap();
        let id = store_with_a_secret(dir.path(), false).await;
        assert!(bytes_mention(dir.path(), b"quokka"), "the fixture holds the secret");

        purge(dir.path(), id).unwrap();

        assert_no_trace(dir.path(), id);
        assert_eq!(count(dir.path(), "SELECT count(*) FROM memory WHERE id <> ?1", id), 300);
        assert_eq!(nearest(dir.path(), 150), id - 150, "the other rows keep their vectors");
        StoreLock::acquire(dir.path(), StoreHolder::Daemon).expect("the lock was released");
    }

    #[tokio::test]
    async fn purging_a_deleted_row_leaves_no_trace_of_its_text() {
        let dir = tempfile::tempdir().unwrap();
        let id = store_with_a_secret(dir.path(), true).await;
        assert!(bytes_mention(dir.path(), b"quokka"), "a soft delete keeps the text");

        purge(dir.path(), id).unwrap();

        assert_no_trace(dir.path(), id);
    }

    #[tokio::test]
    async fn purging_releases_the_rows_that_disputed_it() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::open(dir.path(), StoreHolder::Eval).await.unwrap();
        let disputed = insert(&store, "the deploy runs on fridays".to_string()).await;
        let challenger = insert(&store, "the deploy runs on mondays".to_string()).await;
        store
            .conn
            .call(move |c| {
                c.execute(
                    "UPDATE memory SET status = 'disputed', disputed_against = ?2 WHERE id = ?1",
                    rusqlite::params![challenger, format!("[{disputed}]")],
                )
            })
            .await
            .unwrap();
        store.conn.clone().close().await.unwrap();
        drop(store);

        purge(dir.path(), disputed).unwrap();

        let c = rusqlite::Connection::open(dir.path().join("memory.db")).unwrap();
        let status: String =
            c.query_row("SELECT status FROM memory WHERE id = ?1", [challenger], |r| r.get(0)).unwrap();
        assert_eq!(status, "active");
    }

    #[tokio::test]
    async fn purging_drops_the_pairs_of_the_row() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::open(dir.path(), StoreHolder::Eval).await.unwrap();
        let old = insert(&store, "the deploy runs on fridays".to_string()).await;
        let purged = insert(&store, "the deploy runs on mondays".to_string()).await;
        let newer = insert(&store, "the deploy runs on sundays".to_string()).await;
        store
            .conn
            .call(move |c| {
                c.execute(
                    "INSERT INTO memory_conflict (new_id, old_id, distance, state, created_at) \
                     VALUES (?2, ?1, 0.3, 'pending', 0), (?3, ?2, 0.3, 'judged', 0), (?3, ?1, 0.3, 'pending', 0)",
                    [old, purged, newer],
                )
            })
            .await
            .unwrap();
        store.conn.clone().close().await.unwrap();
        drop(store);

        purge(dir.path(), purged).unwrap();

        let sql = "SELECT count(*) FROM memory_conflict WHERE new_id = ?1 OR old_id = ?1";
        assert_eq!(count(dir.path(), sql, purged), 0);
        assert_eq!(count(dir.path(), sql, old), 1, "the pair of two other rows stays");
    }

    #[tokio::test]
    async fn purging_keeps_the_links_that_name_the_row_as_a_source() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::open(dir.path(), StoreHolder::Eval).await.unwrap();
        let source = insert(&store, "the go service uses tabs".to_string()).await;
        let purged = insert(&store, "the user indents with tabs".to_string()).await;
        let other = insert(&store, "the rust crate uses tabs".to_string()).await;
        store
            .conn
            .call(move |c| {
                c.execute(
                    "INSERT INTO memory_derivation (memory_id, source_id, created_at) \
                     VALUES (?1, ?2, 0), (?3, ?1, 0)",
                    [purged, source, other],
                )
            })
            .await
            .unwrap();
        store.conn.clone().close().await.unwrap();
        drop(store);

        purge(dir.path(), purged).unwrap();

        let drawn = "SELECT count(*) FROM memory_derivation WHERE memory_id = ?1";
        assert_eq!(count(dir.path(), drawn, purged), 0, "a purged conclusion rests on nothing");
        let named = "SELECT count(*) FROM memory_derivation WHERE source_id = ?1";
        assert_eq!(count(dir.path(), named, purged), 1, "the conclusion drawn from it keeps the trace of its loss");
    }

    #[tokio::test]
    async fn purge_refuses_while_the_daemon_holds_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let id = store_with_a_secret(dir.path(), false).await;
        let before = std::fs::read(dir.path().join("memory.db")).unwrap();
        let _daemon = StoreLock::acquire(dir.path(), StoreHolder::Daemon).unwrap();

        let err = purge(dir.path(), id).unwrap_err();

        match err {
            MemoryError::StoreInUse { owner: Some(o) } => assert_eq!(o.holder, StoreHolder::Daemon),
            other => panic!("expected StoreInUse, got {other}"),
        }
        assert_eq!(std::fs::read(dir.path().join("memory.db")).unwrap(), before);
    }

    #[tokio::test]
    async fn purge_refuses_an_unknown_id_untouched() {
        let dir = tempfile::tempdir().unwrap();
        store_with_a_secret(dir.path(), false).await;
        let before = std::fs::read(dir.path().join("memory.db")).unwrap();

        let err = purge(dir.path(), 999_999).unwrap_err();

        assert!(matches!(err, MemoryError::Purge(PurgeError::NotFound(999_999))), "{err}");
        assert_eq!(err.to_string(), "there is no memory #999999");
        assert_eq!(std::fs::read(dir.path().join("memory.db")).unwrap(), before);
        StoreLock::acquire(dir.path(), StoreHolder::Daemon).expect("the lock was released");
    }

    #[tokio::test]
    async fn purge_refuses_a_store_with_pending_migrations_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let older = crate::memory::backup::latest_known() - 1;
        crate::memory::db::seed_on_disk(dir.path(), Some(older)).await;
        let before = std::fs::read(dir.path().join("memory.db")).unwrap();

        let err = purge(dir.path(), 1).unwrap_err();

        assert!(matches!(err, MemoryError::Purge(PurgeError::Pending { .. })), "{err}");
        assert!(err.to_string().contains("start the daemon once"), "{err}");
        assert_eq!(std::fs::read(dir.path().join("memory.db")).unwrap(), before);
    }

    #[tokio::test]
    async fn purge_names_the_backups_that_still_hold_the_row() {
        use crate::memory::backup::backup_offline;
        let pause = || std::thread::sleep(std::time::Duration::from_millis(5));
        let dir = tempfile::tempdir().unwrap();
        crate::memory::db::seed_on_disk(dir.path(), None).await;
        let without = backup_offline(dir.path()).unwrap();
        pause();
        let c = rusqlite::Connection::open(dir.path().join("memory.db")).unwrap();
        c.execute(INSERT_ROW, [SECRET]).unwrap();
        let id = c.last_insert_rowid();
        drop(c);
        let with = backup_offline(dir.path()).unwrap();
        pause();
        rusqlite::Connection::open(dir.path().join("memory.db"))
            .unwrap()
            .execute("UPDATE memory SET deleted_at = 1 WHERE id = ?1", [id])
            .unwrap();
        let after_delete = backup_offline(dir.path()).unwrap();
        let kept: Vec<Vec<u8>> =
            [&without, &with, &after_delete].iter().map(|p| std::fs::read(p).unwrap()).collect();

        let purged = purge(dir.path(), id).unwrap();

        assert_eq!(purged, Purged { backups: vec![with.clone(), after_delete.clone()] });
        let now: Vec<Vec<u8>> =
            [&without, &with, &after_delete].iter().map(|p| std::fs::read(p).unwrap()).collect();
        assert_eq!(now, kept, "no backup was changed");
    }

    fn add_outbox_row(c: &rusqlite::Connection, operation_id: &str, content: &str) {
        c.execute(
            "INSERT INTO ingest_outbox (operation_id, agent_id, content, event, state, created_at, updated_at) \
             VALUES (?1, 'agent-s', ?2, '{}', 'failed', 0, 0)",
            [operation_id, content],
        )
        .unwrap();
    }

    #[tokio::test]
    async fn purge_removes_the_text_from_the_outbox() {
        use crate::memory::backup::backup_offline;
        let dir = tempfile::tempdir().unwrap();
        crate::memory::db::seed_on_disk(dir.path(), None).await;
        let c = rusqlite::Connection::open(dir.path().join("memory.db")).unwrap();
        add_outbox_row(&c, "op-1", &format!("before it was stored: {SECRET}, and more"));
        add_outbox_row(&c, "op-2", "nothing to do with it");
        drop(c);
        let only_in_the_outbox = backup_offline(dir.path()).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        let c = rusqlite::Connection::open(dir.path().join("memory.db")).unwrap();
        c.execute(INSERT_ROW, [SECRET]).unwrap();
        let id = c.last_insert_rowid();
        drop(c);

        let purged = purge(dir.path(), id).unwrap();

        let c = rusqlite::Connection::open(dir.path().join("memory.db")).unwrap();
        let left: Vec<String> = c
            .prepare("SELECT operation_id FROM ingest_outbox ORDER BY id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        drop(c);
        assert_eq!(left, ["op-2"]);
        assert!(!bytes_mention(dir.path(), b"quokka"), "the secret's bytes are still on disk");
        assert_eq!(purged, Purged { backups: vec![only_in_the_outbox] });
    }

    #[tokio::test]
    async fn a_backup_from_before_the_outbox_is_not_named_for_it() {
        let dir = tempfile::tempdir().unwrap();
        let backups = crate::paths::backups_dir(dir.path());
        std::fs::create_dir_all(&backups).unwrap();
        let old = backups.join(crate::memory::backup::snapshot_name(0, crate::memory::backup::Reason::Manual));
        crate::memory::db::seed_on_disk(dir.path(), Some(crate::memory::backup::latest_known() - 1)).await;
        std::fs::copy(dir.path().join("memory.db"), &old).unwrap();

        assert!(backups_holding(&backups, 1, SECRET).is_empty());
    }

    #[test]
    fn the_scan_of_a_missing_store_creates_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let err = scan(dir.path()).unwrap_err();
        assert!(matches!(err, MemoryError::Backup(BackupError::NoStore { .. })), "{err}");
        assert!(files_in(dir.path()).is_empty(), "{:?}", files_in(dir.path()));
    }
}
