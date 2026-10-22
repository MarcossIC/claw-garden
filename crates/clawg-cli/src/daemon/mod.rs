//! The `clawg serve` daemon: its HTTP surface, its on-disk lifecycle, and the
//! process wiring that starts both.

pub mod api;
pub mod guard;
pub mod lifecycle;
pub mod metrics;
pub mod serve;
pub mod sessions;
pub mod socket;
