//! Fixed-size rolling window of recent KNN retrieval latencies.
//!
//! The window is bounded so its memory cost is fixed and the numbers stay
//! about the recent past: a percentile over the whole lifetime of a process
//! that has been open for days answers a question nobody asked. Percentiles
//! are computed on demand from a sorted copy of the window rather than
//! maintained incrementally — the window is small enough (256 `u64` samples,
//! about 2KB) that sorting it costs microseconds, and an incremental
//! percentile structure would be real engineering for a metric that does not
//! need it.
//!
//! Samples are stored in **microseconds**, not milliseconds: a KNN query
//! that finishes in well under a millisecond would otherwise round down to
//! zero, and both percentiles would print as `0`.

use std::sync::Mutex;
use std::time::Duration;

// ---------------------------------------------------------------------------
// Tuning constants
// ---------------------------------------------------------------------------

/// Recent latencies retained. Roughly 2.5x the warm-up threshold below, so
/// the window is never dominated by the very samples that just crossed it.
const CAPACITY: usize = 256;

/// Below this many samples, a percentile is not reported.
///
/// The nearest-rank formula below picks `idx = ceil(p/100 * n) - 1`. At
/// `n=99`, `p99` picks index 98, which is `n-1` — i.e. the maximum sample
/// seen, not a percentile of anything. `n=100` is the smallest sample count
/// where index 98 stops being `n-1`, so it is the first point at which p99
/// means something other than "the slowest call so far". The threshold is
/// therefore derived from the formula, not chosen independently of it.
const WARM_UP_THRESHOLD: usize = 100;

// ---------------------------------------------------------------------------
// Ring
// ---------------------------------------------------------------------------

/// Backing storage: a fixed array plus a write cursor.
///
/// While `len < CAPACITY` the cursor and length coincide, so `samples[0..len]`
/// is exactly the set of values pushed so far, in push order. Once the array
/// is full, new pushes overwrite the oldest slot in place — `samples[0..len]`
/// (`len == CAPACITY` at that point) still holds every live sample, just no
/// longer in push order, which is fine because percentiles sort it anyway.
struct Ring {
    samples: [u64; CAPACITY],
    len: usize,
    next: usize,
}

impl Ring {
    fn new() -> Self {
        Self {
            samples: [0; CAPACITY],
            len: 0,
            next: 0,
        }
    }

    fn push(&mut self, sample_us: u64) {
        self.samples[self.next] = sample_us;
        self.next = (self.next + 1) % CAPACITY;
        if self.len < CAPACITY {
            self.len += 1;
        }
    }
}

// ---------------------------------------------------------------------------
// Percentile math
// ---------------------------------------------------------------------------

/// Nearest-rank percentile over an already-sorted-ascending slice.
///
/// `idx = ceil(p/100 * n) - 1`, clamped into `[0, n-1]`. This always selects
/// a value that was actually measured, never one interpolated between two
/// measurements — for an operator watching whether latency is creeping up,
/// a number that really happened is more trustworthy than a fiction between
/// two numbers that did.
///
/// Worked examples, because implementations disagree on this silently:
/// - `n=100, p=99` -> idx 98, the 99th smallest of 100 (never the maximum).
/// - `n=4, p=50` -> idx 1, the *lower* of the two middle values `[1,2,3,4]`
///   picks `2`, NOT the mean of the two middles (`2.5`).
fn nearest_rank(sorted: &[u64], p: u64) -> u64 {
    let n = sorted.len() as u64;
    debug_assert!(n > 0, "nearest_rank called on an empty window");
    // Integer ceiling of (p * n) / 100 without floating point.
    let rank = (p * n + 99) / 100;
    let idx = rank.saturating_sub(1).min(n.saturating_sub(1));
    sorted[idx as usize]
}

// ---------------------------------------------------------------------------
// KnnLatency
// ---------------------------------------------------------------------------

/// Recent per-call KNN retrieval latencies, oldest overwritten once the
/// window is full.
///
/// `std::sync::Mutex`, not an async-aware lock: every operation the guard
/// protects is a plain in-memory array write or a copy-then-sort, never an
/// `.await`. Held across an await point it would make the calling future
/// `!Send`; instead `record` and `snapshot` each take the lock, do their
/// work, and drop it before returning — a guard is never handed to a caller.
pub(crate) struct KnnLatency {
    ring: Mutex<Ring>,
}

impl KnnLatency {
    pub(crate) fn new() -> Self {
        Self {
            ring: Mutex::new(Ring::new()),
        }
    }

    /// Records one latency sample.
    ///
    /// Recovers from a poisoned lock via `into_inner()` rather than
    /// propagating the panic: losing retrieval telemetry because some other
    /// caller panicked while holding this lock would be worse than the
    /// inconsistency a recovered guard might carry, since the guarded state
    /// is just an array of past timings, not data whose corruption can
    /// spread.
    pub(crate) fn record(&self, sample: Duration) {
        let micros = u64::try_from(sample.as_micros()).unwrap_or(u64::MAX);
        let mut ring = self.ring.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        ring.push(micros);
    }

    /// Returns an owned snapshot of the current window.
    ///
    /// Below the warm-up threshold both percentiles are `None` — the field
    /// names stay constant so log greps keep working, but the value says why
    /// instead of printing something that would read as a measurement.
    pub(crate) fn snapshot(&self) -> LatencySnapshot {
        let (n, mut values) = {
            let ring = self.ring.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            (ring.len, ring.samples[..ring.len].to_vec())
        };
        if n < WARM_UP_THRESHOLD {
            return LatencySnapshot {
                n,
                p50: None,
                p99: None,
            };
        }
        values.sort_unstable();
        LatencySnapshot {
            n,
            p50: Some(Duration::from_micros(nearest_rank(&values, 50))),
            p99: Some(Duration::from_micros(nearest_rank(&values, 99))),
        }
    }
}

/// An owned, point-in-time read of the latency window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LatencySnapshot {
    pub(crate) n: usize,
    pub(crate) p50: Option<Duration>,
    pub(crate) p99: Option<Duration>,
}

impl std::fmt::Display for LatencySnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (self.p50, self.p99) {
            (Some(p50), Some(p99)) => write!(
                f,
                "knn_n={} knn_p50={:.1}ms knn_p99={:.1}ms",
                self.n,
                p50.as_secs_f64() * 1000.0,
                p99.as_secs_f64() * 1000.0
            ),
            _ => write!(f, "knn_n={} knn_p50=warming knn_p99=warming", self.n),
        }
    }
}

// A held guard across an await point would make the enclosing future
// `!Send`, which matters because retrieval runs on the desktop async runtime.
// Asserting both types stay `Send + Sync` here catches that class of
// regression at compile time instead of leaving it to be discovered by a
// runtime deadlock or a spawn error far from this file.
const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<KnnLatency>();
    assert_send_sync::<crate::memory::MemoryStore>();
};

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_rank_p99_of_hundred_is_the_ninety_ninth_smallest() {
        let latency = KnnLatency::new();
        for i in 1..=100u64 {
            latency.record(Duration::from_micros(i));
        }
        let snap = latency.snapshot();
        assert_eq!(snap.n, 100);
        // idx = ceil(99/100 * 100) - 1 = 98 (0-based) -> the 99th smallest
        // value, i.e. 99 — never the maximum, which is 100.
        assert_eq!(snap.p99, Some(Duration::from_micros(99)));
    }

    #[test]
    fn p50_of_even_count_is_the_lower_middle_not_the_mean() {
        // Below the warm-up threshold, so this exercises the pure formula
        // directly rather than going through the suppressed public API —
        // the divergence this pins is about the math, not about warm-up.
        let sorted = [1u64, 2, 3, 4];
        // idx = ceil(50/100 * 4) - 1 = 1 -> value 2, NOT the mean of the two
        // middle values (2 and 3), which would be 2.5.
        assert_eq!(nearest_rank(&sorted, 50), 2);
    }

    #[test]
    fn percentiles_absent_below_the_warm_threshold() {
        let latency = KnnLatency::new();
        for i in 1..=99u64 {
            latency.record(Duration::from_micros(i));
        }
        let snap = latency.snapshot();
        assert_eq!(snap.n, 99);
        assert_eq!(snap.p50, None);
        assert_eq!(snap.p99, None);
    }

    #[test]
    fn at_one_hundred_samples_p99_stops_being_the_maximum() {
        let latency = KnnLatency::new();
        for i in 1..=100u64 {
            latency.record(Duration::from_micros(i));
        }
        let snap = latency.snapshot();
        assert!(snap.p99.is_some(), "100 samples must cross the warm-up threshold");
        assert_ne!(
            snap.p99,
            Some(Duration::from_micros(100)),
            "at exactly 100 samples p99 must already differ from the maximum"
        );
    }

    #[test]
    fn oldest_sample_is_overwritten_past_capacity() {
        let latency = KnnLatency::new();
        for i in 1..=300u64 {
            latency.record(Duration::from_micros(i));
        }
        let snap = latency.snapshot();
        // 300 pushes into a 256-slot ring leaves exactly the newest 256:
        // values 45..=300 (300 - 256 + 1 = 45), the oldest 44 (1..=44) gone.
        assert_eq!(snap.n, 256);
        // Sorted ascending, the surviving values are 45..=300 (256 of them).
        // idx(p50) = ceil(50/100*256)-1 = 127 -> value 45+127 = 172.
        assert_eq!(snap.p50, Some(Duration::from_micros(172)));
        // idx(p99) = ceil(99/100*256)-1 = 253 -> value 45+253 = 298.
        // If eviction were broken and the window still held 1..=256 instead
        // (oldest samples never overwritten), the same index would select
        // 1+253 = 254 — a different number, so this assertion fails loudly
        // the day wrap-around eviction regresses.
        assert_eq!(snap.p99, Some(Duration::from_micros(298)));
    }

    #[test]
    fn snapshot_leaves_the_window_unchanged() {
        let latency = KnnLatency::new();
        for i in 1..=150u64 {
            latency.record(Duration::from_micros(i));
        }
        let first = latency.snapshot();
        let second = latency.snapshot();
        assert_eq!(first, second, "reading a snapshot must not mutate the window");
    }

    #[test]
    fn a_poisoned_lock_does_not_stop_recording() {
        let latency = std::sync::Arc::new(KnnLatency::new());
        let poisoning = latency.clone();
        let handle = std::thread::spawn(move || {
            let _guard = poisoning.ring.lock().unwrap();
            panic!("deliberately poisoning the lock to prove recovery");
        });
        assert!(handle.join().is_err(), "the spawned thread must have panicked");

        // Recording after the lock is poisoned must neither panic nor lose
        // the sample: losing retrieval telemetry because some unrelated
        // panic poisoned this lock would be absurd.
        latency.record(Duration::from_micros(7));
        let snap = latency.snapshot();
        assert_eq!(snap.n, 1);
    }
}
