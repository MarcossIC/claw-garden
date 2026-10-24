//! Usage metrics of the daemon: what it records while it serves and what it
//! keeps of it in `metrics.db`, beside the memory store and never in it.

pub mod histogram;
pub mod recorder;
pub mod report;
pub mod store;

pub use histogram::{Histogram, Unit};
pub use recorder::{Gauge, Recorder, SeriesKey, Window};

/// Reads the private memory of this process in bytes, or `None` where the
/// system does not say. Only the daemon knows the calls, so it hands one to
/// what logs memory.
pub type MemoryProbe = fn() -> Option<u64>;

/// `bytes` in MiB with one decimal, as the log lines print memory.
pub fn mib(bytes: u64) -> String {
    format!("{:.1} MiB", bytes as f64 / 1_048_576.0)
}

#[cfg(test)]
mod tests {
    #[test]
    fn memory_reads_in_mib() {
        assert_eq!(super::mib(297_795_584), "284.0 MiB");
        assert_eq!(super::mib(0), "0.0 MiB");
    }
}
