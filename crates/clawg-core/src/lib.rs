//! Claw Garden memory substrate.
//!
//! Everything in this crate is free of the desktop runtime: it is the local-first memory,
//! provider and configuration layer that the desktop app (`clawg-desktop`) and
//! the evaluation harness (`clawg-eval`) both build on. Background jobs are
//! split into a pure, unit-testable core (here) and a thin `AppHandle` wrapper
//! that lives in the desktop crate — see `clawg_desktop_lib::memory_runtime`.

pub mod config;
pub mod hooks;
pub mod memory;
pub mod models;
pub mod paths;
pub mod providers;
pub mod services;

/// Narrow, feature-gated door into retrieval internals for the evaluation
/// binaries. See `eval_api.rs` for why an opaque wrapper is preferred over
/// widening the module tree.
#[cfg(feature = "eval")]
pub mod eval_api;
