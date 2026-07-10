use std::sync::Mutex;
use tokio::sync::Notify;

const DEFAULT_MAX_CONCURRENT_READS: usize = 8;

struct GateState {
    active_readers: usize,
    pending_writers: usize,
    active_writer: bool,
    max_concurrent_reads: usize,
}

impl GateState {
    fn can_start_read(&self) -> bool {
        !self.active_writer
            && self.pending_writers == 0
            && self.active_readers < self.max_concurrent_reads
    }

    fn can_start_write(&self) -> bool {
        !self.active_writer && self.active_readers == 0
    }
}

/// RW-lock for tool concurrency with writer priority.
/// is_concurrency_safe: true  → acquire_read (multiple simultaneous allowed)
/// is_concurrency_safe: false → acquire_write (serialization barrier)
pub struct ConcurrencyGate {
    state: Mutex<GateState>,
    notify: Notify,
}

impl ConcurrencyGate {
    pub fn new() -> Self {
        Self::with_max_reads(DEFAULT_MAX_CONCURRENT_READS)
    }

    pub fn with_max_reads(max: usize) -> Self {
        Self {
            state: Mutex::new(GateState {
                active_readers: 0,
                pending_writers: 0,
                active_writer: false,
                max_concurrent_reads: max,
            }),
            notify: Notify::new(),
        }
    }

    /// Acquire a read slot. Blocks if a writer is active or pending.
    pub async fn acquire_read(&self) -> ReadGuard<'_> {
        loop {
            {
                let mut s = self.state.lock().unwrap();
                if s.can_start_read() {
                    s.active_readers += 1;
                    return ReadGuard { gate: self };
                }
            }
            self.notify.notified().await;
        }
    }

    /// Acquire the write slot. Blocks until all readers finish.
    /// Sets pending_writers to signal writer priority to incoming readers.
    pub async fn acquire_write(&self) -> WriteGuard<'_> {
        {
            self.state.lock().unwrap().pending_writers += 1;
        }
        loop {
            {
                let mut s = self.state.lock().unwrap();
                if s.can_start_write() {
                    s.pending_writers -= 1;
                    s.active_writer = true;
                    return WriteGuard { gate: self };
                }
            }
            self.notify.notified().await;
        }
    }
}

impl Default for ConcurrencyGate {
    fn default() -> Self {
        Self::new()
    }
}

pub struct ReadGuard<'a> {
    gate: &'a ConcurrencyGate,
}

impl Drop for ReadGuard<'_> {
    fn drop(&mut self) {
        self.gate.state.lock().unwrap().active_readers -= 1;
        self.gate.notify.notify_waiters();
    }
}

pub struct WriteGuard<'a> {
    gate: &'a ConcurrencyGate,
}

impl Drop for WriteGuard<'_> {
    fn drop(&mut self) {
        self.gate.state.lock().unwrap().active_writer = false;
        self.gate.notify.notify_waiters();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[tokio::test]
    async fn multiple_readers_concurrent() {
        let gate = Arc::new(ConcurrencyGate::new());
        let g1 = gate.acquire_read().await;
        let g2 = gate.acquire_read().await;
        // Both acquired — no panic, no deadlock
        drop(g1);
        drop(g2);
    }

    #[tokio::test]
    async fn writer_excludes_readers() {
        let gate = ConcurrencyGate::new();
        let _write = gate.acquire_write().await;
        // State: active_writer = true, pending_writers = 0
        let s = gate.state.lock().unwrap();
        assert!(s.active_writer);
        assert_eq!(s.active_readers, 0);
    }
}
