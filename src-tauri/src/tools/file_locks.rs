use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};
use std::time::Duration;
use dashmap::DashMap;
use tokio::sync::{RwLock, OwnedRwLockReadGuard, OwnedRwLockWriteGuard};

/// Interval between periodic cleanup sweeps spawned by `spawn_cleanup_task`.
/// 60s is a conservative trade-off: short enough to keep the map size bounded
/// on long sessions, long enough to avoid cache-thrashing the retain() scan.
const CLEANUP_INTERVAL: Duration = Duration::from_secs(60);

/// Per-path RwLock registry. Prevents concurrent writes to the same file.
/// Allows concurrent reads to the same file.
/// Different files are independent — writes to a.rs don't block writes to b.rs.
pub struct FileLockRegistry {
    locks: DashMap<PathBuf, Arc<RwLock<()>>>,
}

impl FileLockRegistry {
    pub fn new() -> Self {
        Self {
            locks: DashMap::new(),
        }
    }

    /// Acquire a shared read lock for the given path.
    pub async fn acquire_read(&self, path: &Path) -> OwnedRwLockReadGuard<()> {
        let lock = self.lock_for(path);
        lock.read_owned().await
    }

    /// Acquire an exclusive write lock for the given path.
    pub async fn acquire_write(&self, path: &Path) -> OwnedRwLockWriteGuard<()> {
        let lock = self.lock_for(path);
        lock.write_owned().await
    }

    fn lock_for(&self, path: &Path) -> Arc<RwLock<()>> {
        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_owned());
        self.locks
            .entry(canonical)
            .or_insert_with(|| Arc::new(RwLock::new(())))
            .clone()
    }

    /// Remove entries that nothing holds (Arc strong_count == 1 → only registry owns it).
    /// Call periodically from a background task. NOT hot-path safe (O(n)).
    pub fn cleanup(&self) {
        self.locks.retain(|_, lock| Arc::strong_count(lock) > 1);
    }

    /// Current number of tracked paths — for observability and tests.
    pub fn len(&self) -> usize {
        self.locks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.locks.is_empty()
    }

    /// Spawn a background task that calls `cleanup()` every `CLEANUP_INTERVAL`.
    ///
    /// Uses a `Weak` pointer so the task terminates naturally when the registry
    /// is dropped — no explicit shutdown needed. Must be called from within a
    /// Tokio runtime (our ToolExecutor lives inside the Tauri async runtime).
    pub fn spawn_cleanup_task(registry: &Arc<Self>) {
        let weak = Arc::downgrade(registry);
        tokio::spawn(cleanup_loop(weak));
    }
}

async fn cleanup_loop(weak: Weak<FileLockRegistry>) {
    // interval() fires immediately on the first tick; skip it so we don't
    // run a no-op sweep at startup.
    let mut interval = tokio::time::interval(CLEANUP_INTERVAL);
    interval.tick().await;

    loop {
        interval.tick().await;
        match weak.upgrade() {
            Some(registry) => registry.cleanup(),
            None => return, // Registry dropped — exit quietly.
        }
    }
}

impl Default for FileLockRegistry {
    fn default() -> Self {
        Self::new()
    }
}
