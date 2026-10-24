//! Claw Garden memory substrate.
//!
//! The local-first memory layer and the shared chat types, which the daemon
//! (`clawg-cli`) and the evaluation harness (`clawg-eval`) both build on.

pub mod build_info;
pub mod memory;
pub mod metrics;
pub mod models;
pub mod paths;
pub mod preferences;
pub mod private_dir;
/// Host-agnostic assembly of the memory substrate — the store, the ingest
/// channel and the background workers, without any UI runtime.
pub mod runtime;

/// Narrow, feature-gated door into retrieval internals for the evaluation
/// binaries. See `eval_api.rs` for why an opaque wrapper is preferred over
/// widening the module tree.
#[cfg(feature = "eval")]
pub mod eval_api;
