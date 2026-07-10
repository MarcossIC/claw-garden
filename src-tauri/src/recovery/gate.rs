use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Per-server recovery semaphore. Serializes recoveries for the same server,
/// while allowing concurrent recoveries for different servers.
///
/// Stored in `AppState` — lives for the duration of the app.
///
/// # Why this exists
///
/// Without `RecoveryGate`, N agents that all encounter the same downed MCP server
/// would each attempt a reconnect serially (write-locked manager). The second and
/// third agents would reconnect an already-working server — wasted I/O.
///
/// The gate serializes reconnects per server, and the fast-path `is_connected_for_path`
/// check lets waiting agents skip the reconnect when the first agent already fixed it.
pub struct RecoveryGate {
    // std::sync::Mutex: protects the map only (brief, synchronous acquisition)
    // Arc<Semaphore(1)> per server: async mutex for the recovery critical section
    servers: std::sync::Mutex<HashMap<PathBuf, Arc<Semaphore>>>,
}

impl RecoveryGate {
    pub fn new() -> Self {
        Self {
            servers: std::sync::Mutex::new(HashMap::new()),
        }
    }

    /// Acquire the recovery permit for the given server path.
    ///
    /// If another agent is in recovery for this server, waits until they finish.
    /// The permit is released automatically when it is dropped.
    pub async fn acquire(&self, path: &Path) -> OwnedSemaphorePermit {
        let sem = {
            // unwrap_or_else: poison recovery — the map data is non-critical, continue
            let mut map = self.servers.lock().unwrap_or_else(|e| e.into_inner());
            map.entry(path.to_path_buf())
                .or_insert_with(|| Arc::new(Semaphore::new(1)))
                .clone()
        };
        // unwrap(): Semaphore only fails if closed — we never close it
        sem.acquire_owned().await.unwrap()
    }

    /// Remove the semaphore entry for a server.
    ///
    /// Call ONLY on permanent server removal (user deletes the server from their config).
    /// Do NOT call during reconnect/rebuild — those are temporary disconnections.
    ///
    /// Safety: any agent that already holds an `OwnedSemaphorePermit` retains its
    /// `Arc<Semaphore>` clone and completes normally. New `acquire(path)` calls
    /// after removal create a fresh Arc.
    pub fn remove_server(&self, path: &Path) {
        let mut map = self.servers.lock().unwrap_or_else(|e| e.into_inner());
        map.remove(path);
    }
}

impl Default for RecoveryGate {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::Barrier;

    #[tokio::test]
    async fn gate_serializes_concurrent_recoveries_for_same_server() {
        let gate = Arc::new(RecoveryGate::new());
        let path = PathBuf::from("/srv/A");
        let call_count = Arc::new(AtomicUsize::new(0));
        let barrier = Arc::new(Barrier::new(3));

        let mut handles = vec![];
        for _ in 0..3 {
            let gate = gate.clone();
            let path = path.clone();
            let counter = call_count.clone();
            let barrier = barrier.clone();
            handles.push(tokio::spawn(async move {
                barrier.wait().await;
                let _permit = gate.acquire(&path).await;
                counter.fetch_add(1, Ordering::SeqCst);
            }));
        }

        for h in handles {
            h.await.unwrap();
        }

        assert_eq!(call_count.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn gate_allows_concurrent_recoveries_for_different_servers() {
        let gate = Arc::new(RecoveryGate::new());
        let path_a = PathBuf::from("/srv/A");
        let path_b = PathBuf::from("/srv/B");

        // Both acquire simultaneously — should not block each other
        let gate_a = gate.clone();
        let gate_b = gate.clone();
        let pa = path_a.clone();
        let pb = path_b.clone();

        let (ha, hb) = tokio::join!(
            tokio::spawn(async move { gate_a.acquire(&pa).await }),
            tokio::spawn(async move { gate_b.acquire(&pb).await }),
        );

        // Both succeed — permits acquired without deadlock
        let _pa = ha.unwrap();
        let _pb = hb.unwrap();
    }

    #[tokio::test]
    async fn gate_reused_across_acquire_calls() {
        let gate = Arc::new(RecoveryGate::new());
        let path = PathBuf::from("/srv/A");

        let _p1 = gate.acquire(&path).await; // drops immediately
        drop(_p1);
        let _p2 = gate.acquire(&path).await; // same Arc reused
        drop(_p2);
        // No deadlock, no panic
    }

    #[tokio::test]
    async fn remove_server_cleans_up_map_entry() {
        let gate = RecoveryGate::new();
        let path = PathBuf::from("/srv/A");

        let _p = gate.acquire(&path).await;
        drop(_p);

        gate.remove_server(&path);

        // Entry removed — next acquire creates a fresh Arc (no panic)
        let _p2 = gate.acquire(&path).await;
    }

    #[tokio::test]
    async fn remove_server_does_not_affect_different_server() {
        let gate = RecoveryGate::new();
        let path_a = PathBuf::from("/srv/A");
        let path_b = PathBuf::from("/srv/B");

        let _pb = gate.acquire(&path_b).await;
        drop(_pb);

        gate.remove_server(&path_a); // removes A, not B

        // B still works
        let _pb2 = gate.acquire(&path_b).await;
    }

    #[tokio::test]
    async fn second_agent_can_acquire_after_first_releases() {
        let gate = Arc::new(RecoveryGate::new());
        let path = PathBuf::from("/srv/A");

        let permit = gate.acquire(&path).await;

        let gate2 = gate.clone();
        let path2 = path.clone();
        let handle = tokio::spawn(async move {
            gate2.acquire(&path2).await
        });

        // Release the first permit — second agent can now acquire
        drop(permit);
        let _p2 = handle.await.unwrap();
    }
}
