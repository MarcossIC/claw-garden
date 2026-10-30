//! What the daemon reads of the store's one connection to see it under load:
//! how long a call waits in its queue, and what SQLite's page cache did.

use std::time::{Duration, Instant};

use rusqlite::ffi;

use crate::memory::db::MemoryStore;
use crate::memory::error::MemoryError;

/// What `sqlite3_db_status` says of the connection. The four counters count
/// from the reading before, since each reading restarts them; the two sizes
/// are what is held now. A value SQLite refused to give is `None`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CacheStatus {
    pub hit: Option<u64>,
    pub miss: Option<u64>,
    pub write: Option<u64>,
    pub spill: Option<u64>,
    pub used_bytes: Option<u64>,
    pub stmt_used_bytes: Option<u64>,
}

impl MemoryStore {
    /// How long a call waits behind the ones queued before it, from the
    /// moment it is queued until it starts on the connection's thread. It
    /// runs no SQL: the wait is the queue, not the statement.
    pub async fn probe_wait(&self) -> Result<Duration, MemoryError> {
        let queued = Instant::now();
        self.conn
            .call(move |_| -> rusqlite::Result<Duration> { Ok(queued.elapsed()) })
            .await
            .map_err(MemoryError::Connection)
    }

    /// The page cache and statement memory of the connection, restarting
    /// the counters so the next reading counts from here.
    pub async fn cache_status(&self) -> Result<CacheStatus, MemoryError> {
        self.conn
            .call(|c| -> rusqlite::Result<CacheStatus> {
                // SAFETY: the handle is the open connection of this thread,
                // and sqlite3_db_status only reads (or zeroes) its counters.
                let read = |op: i32, reset: bool| unsafe {
                    let (mut current, mut highwater) = (0, 0);
                    let rc = ffi::sqlite3_db_status(c.handle(), op, &mut current, &mut highwater, i32::from(reset));
                    (rc == ffi::SQLITE_OK).then(|| current.max(0) as u64)
                };
                Ok(CacheStatus {
                    hit: read(ffi::SQLITE_DBSTATUS_CACHE_HIT, true),
                    miss: read(ffi::SQLITE_DBSTATUS_CACHE_MISS, true),
                    write: read(ffi::SQLITE_DBSTATUS_CACHE_WRITE, true),
                    spill: read(ffi::SQLITE_DBSTATUS_CACHE_SPILL, true),
                    used_bytes: read(ffi::SQLITE_DBSTATUS_CACHE_USED, false),
                    stmt_used_bytes: read(ffi::SQLITE_DBSTATUS_STMT_USED, false),
                })
            })
            .await
            .map_err(MemoryError::Connection)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_probe_waits_behind_a_call_in_progress() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let busy = {
            let store = store.clone();
            tokio::spawn(async move {
                store
                    .conn
                    .call(move |_| -> rusqlite::Result<()> {
                        let _ = started_tx.send(());
                        std::thread::sleep(Duration::from_millis(200));
                        Ok(())
                    })
                    .await
            })
        };
        started_rx.await.unwrap();

        let waited = store.probe_wait().await.unwrap();
        busy.await.unwrap().unwrap();
        assert!(waited >= Duration::from_millis(150), "the probe queued behind a 200 ms call waited {waited:?}");
        assert!(store.probe_wait().await.unwrap() < Duration::from_millis(150), "an idle connection answers at once");
    }

    #[tokio::test]
    async fn the_cache_counters_restart_at_each_reading() {
        let store = MemoryStore::new_for_test().await.unwrap();
        store.cache_status().await.unwrap();
        store
            .conn
            .call(|c| -> rusqlite::Result<()> {
                for _ in 0..5 {
                    c.query_row("SELECT COUNT(*) FROM memory", [], |r| r.get::<_, i64>(0))?;
                }
                Ok(())
            })
            .await
            .unwrap();

        let first = store.cache_status().await.unwrap();
        let second = store.cache_status().await.unwrap();
        assert!(first.hit.is_some_and(|hits| hits > 0), "the queries read cached pages: {first:?}");
        assert_eq!(second.hit, Some(0), "a reading restarts the counter: {second:?}");
        assert_eq!(second.miss, Some(0));
        assert!(second.used_bytes.is_some_and(|bytes| bytes > 0), "the size is what is held, not restarted: {second:?}");
    }

    #[tokio::test]
    async fn a_closed_connection_is_an_error_not_a_reading() {
        let store = MemoryStore::new_for_test().await.unwrap();
        store.conn.clone().close().await.unwrap();
        assert!(store.probe_wait().await.is_err());
        assert!(store.cache_status().await.is_err());
    }
}
