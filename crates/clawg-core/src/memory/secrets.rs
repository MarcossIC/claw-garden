//! Secrets already in the store: finding the rows that hold one.
//!
//! New saves are masked on the way in (see [`crate::memory::redact`]); this
//! is for what was stored before, which nothing rewrites on its own.

use std::path::Path;

use crate::memory::backup::{open_for_probe, BackupError};
use crate::memory::redact::{redact, SecretKind};
use crate::memory::MemoryError;

/// A `memory` row whose content holds at least one secret. Carries what the
/// row is and what it holds, never the content itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Flagged {
    pub id: i64,
    pub kinds: Vec<SecretKind>,
    pub deleted: bool,
}

/// Every row of `memory`, live or deleted, that holds a secret, by id.
///
/// Reads without the lock and without writing, so it works beside a running
/// daemon: WAL lets a reader in while the daemon writes.
pub fn scan(base_dir: &Path) -> Result<Vec<Flagged>, MemoryError> {
    let path = base_dir.join("memory.db");
    if !path.exists() {
        return Err(BackupError::NoStore { path }.into());
    }
    crate::memory::db::ensure_vec0_registered();
    let c = open_for_probe(&path)?;
    let mut stmt = c.prepare("SELECT id, content, deleted_at IS NOT NULL FROM memory ORDER BY id")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, bool>(2)?)))?;
    let mut flagged = Vec::new();
    for row in rows {
        let (id, content, deleted) = row?;
        let kinds = redact(&content).kinds;
        if !kinds.is_empty() {
            flagged.push(Flagged { id, kinds, deleted });
        }
    }
    Ok(flagged)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::store_lock::{StoreHolder, StoreLock};
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
            scan(dir.path()).unwrap(),
            vec![
                Flagged { id: aws, kinds: vec![SecretKind::AwsAccessKey], deleted: false },
                Flagged { id: jwt, kinds: vec![SecretKind::Jwt], deleted: true },
            ]
        );
        assert_eq!(files_in(dir.path()), before, "the scan wrote nothing beside the store");
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

        assert_eq!(found, vec![Flagged { id, kinds: vec![SecretKind::GithubToken], deleted: false }]);
        drop(store);
    }

    #[test]
    fn the_scan_of_a_missing_store_creates_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let err = scan(dir.path()).unwrap_err();
        assert!(matches!(err, MemoryError::Backup(BackupError::NoStore { .. })), "{err}");
        assert!(files_in(dir.path()).is_empty(), "{:?}", files_in(dir.path()));
    }
}
