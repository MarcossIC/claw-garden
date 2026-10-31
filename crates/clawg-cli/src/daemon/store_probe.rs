//! The store under load, as the daemon samples it: how long a call waits on
//! its one connection, every [`PROBE_INTERVAL`], and once a minute its page
//! cache, the size of its files and the outbox.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use clawg_core::memory::db::MemoryStore;
use clawg_core::metrics::{Recorder, Unit};
use tokio::task::JoinHandle;

/// How often a probe is queued on the store's connection.
pub const PROBE_INTERVAL: Duration = Duration::from_secs(10);

/// One probe in this many also reads the minute's cache, files and outbox.
const MINUTE_EVERY: u64 = 6;

/// The sizes of the store's files, `None` for one that is not there.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct StoreFiles {
    pub db: Option<u64>,
    pub wal: Option<u64>,
    /// The files directly in `backups/`, not those of its subdirectories.
    pub backups: Option<u64>,
}

/// Reads the sizes of `memory.db`, its WAL and the snapshots of `base_dir`.
pub fn store_files(base_dir: &Path) -> StoreFiles {
    let size = |path: &Path| std::fs::metadata(path).ok().filter(|m| m.is_file()).map(|m| m.len());
    let backups = std::fs::read_dir(clawg_core::paths::backups_dir(base_dir)).ok().map(|entries| {
        entries.flatten().filter_map(|entry| entry.metadata().ok().filter(|m| m.is_file())).map(|m| m.len()).sum()
    });
    StoreFiles { db: size(&base_dir.join("memory.db")), wal: size(&base_dir.join("memory.db-wal")), backups }
}

/// Records how long a call waited on the connection; a closed one records
/// nothing.
pub async fn probe(store: &MemoryStore, recorder: &Recorder) {
    match store.probe_wait().await {
        Ok(waited) => {
            recorder.observe("db.probe_wait", Unit::Micros, &[], u64::try_from(waited.as_micros()).unwrap_or(u64::MAX))
        }
        Err(e) => log::debug!("the store probe did not run: {e}"),
    }
}

/// Records the page cache of the connection, the sizes of the store's files
/// and the outbox. A part that cannot be read leaves its series out.
pub async fn read_minute(store: &MemoryStore, base_dir: &Path, recorder: &Recorder) {
    match store.cache_status().await {
        Ok(cache) => {
            let gauges = [
                ("db.cache_hit", cache.hit),
                ("db.cache_miss", cache.miss),
                ("db.cache_write", cache.write),
                ("db.cache_spill", cache.spill),
                ("db.cache_used_bytes", cache.used_bytes),
                ("db.stmt_used_bytes", cache.stmt_used_bytes),
            ];
            for (name, value) in gauges {
                if let Some(value) = value {
                    recorder.gauge(name, &[], value as f64);
                }
            }
        }
        Err(e) => log::debug!("the store's cache was not read: {e}"),
    }

    let dir = base_dir.to_path_buf();
    match tokio::task::spawn_blocking(move || store_files(&dir)).await {
        Ok(files) => {
            for (file, bytes) in [("db", files.db), ("wal", files.wal), ("backups", files.backups)] {
                if let Some(bytes) = bytes {
                    recorder.gauge("db.file_bytes", &[("file", file)], bytes as f64);
                }
            }
        }
        Err(e) => log::debug!("the store's files were not measured: {e}"),
    }

    match store.outbox_summary().await {
        Ok(outbox) => {
            recorder.gauge("outbox.pending", &[], outbox.pending as f64);
            recorder.gauge("outbox.failed", &[], outbox.failed as f64);
            if let Some(oldest) = outbox.oldest_pending_s {
                recorder.gauge("outbox.oldest_pending_s", &[], oldest as f64);
            }
        }
        Err(e) => log::debug!("the outbox was not read: {e}"),
    }
}

/// Probes at once and then every [`PROBE_INTERVAL`] until aborted. A probe
/// that waited past its turn delays the next instead of firing a burst.
pub fn spawn(store: Arc<MemoryStore>, base_dir: std::path::PathBuf, recorder: Arc<Recorder>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticks = tokio::time::interval(PROBE_INTERVAL);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        for n in 0u64.. {
            ticks.tick().await;
            probe(&store, &recorder).await;
            if n % MINUTE_EVERY == 0 {
                read_minute(&store, &base_dir, &recorder).await;
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use clawg_core::metrics::SeriesKey;

    #[test]
    fn the_store_files_are_measured_and_a_missing_one_is_left_out() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(store_files(dir.path()), StoreFiles::default(), "nothing there is nothing measured");

        std::fs::write(dir.path().join("memory.db"), [0u8; 4096]).unwrap();
        let backups = clawg_core::paths::backups_dir(dir.path());
        std::fs::create_dir_all(backups.join("setup")).unwrap();
        std::fs::write(backups.join("memory-a.db"), [0u8; 100]).unwrap();
        std::fs::write(backups.join("memory-b.db"), [0u8; 50]).unwrap();
        std::fs::write(backups.join("setup").join("settings.json"), [0u8; 7]).unwrap();

        let files = store_files(dir.path());
        assert_eq!(files, StoreFiles { db: Some(4096), wal: None, backups: Some(150) });
    }

    #[tokio::test]
    async fn a_probe_records_its_wait() {
        let store = MemoryStore::new_for_test().await.unwrap();
        let recorder = Recorder::default();
        probe(&store, &recorder).await;
        probe(&store, &recorder).await;
        let run = recorder.run();
        assert_eq!(run[&SeriesKey::new("db.probe_wait", &[])].count(), 2);

        store.conn.clone().close().await.unwrap();
        probe(&store, &recorder).await;
        assert_eq!(recorder.run()[&SeriesKey::new("db.probe_wait", &[])].count(), 2, "a probe that did not run is not a wait");
    }

    #[tokio::test]
    async fn a_minute_reading_records_the_cache_the_files_and_the_outbox() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("memory.db"), [0u8; 10]).unwrap();
        let store = MemoryStore::new_for_test().await.unwrap();
        let recorder = Recorder::default();

        read_minute(&store, dir.path(), &recorder).await;

        let gauges = recorder.take_minute().gauges;
        let last = |name: &str, labels: &[(&str, &str)]| gauges.get(&SeriesKey::new(name, labels)).map(|g| g.last);
        assert!(last("db.cache_hit", &[]).is_some(), "{gauges:?}");
        assert!(last("db.cache_used_bytes", &[]).is_some_and(|b| b > 0.0), "{gauges:?}");
        assert_eq!(last("db.file_bytes", &[("file", "db")]), Some(10.0));
        assert_eq!(last("db.file_bytes", &[("file", "wal")]), None, "a missing file is left out, never 0");
        assert_eq!(last("db.file_bytes", &[("file", "backups")]), None);
        assert_eq!(last("outbox.pending", &[]), Some(0.0));
        assert_eq!(last("outbox.failed", &[]), Some(0.0));
        assert_eq!(last("outbox.oldest_pending_s", &[]), None, "no row waits, so there is no age");
    }
}
