//! The `clawg serve` daemon: its HTTP surface, its on-disk lifecycle, and the
//! process wiring that starts both.

pub mod api;
pub mod lifecycle;
pub mod serve;
