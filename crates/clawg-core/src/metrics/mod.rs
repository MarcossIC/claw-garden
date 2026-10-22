//! Usage metrics of the daemon: what it records while it serves and what it
//! keeps of it in `metrics.db`, beside the memory store and never in it.

pub mod histogram;
pub mod recorder;
pub mod report;
pub mod store;

pub use histogram::{Histogram, Unit};
pub use recorder::{Gauge, Recorder, SeriesKey, Window};
