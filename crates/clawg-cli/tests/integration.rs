//! The integration tests of this crate, one module per file of `tests/`, in a
//! single binary: each test binary links the whole dependency tree.

mod api;
mod binary;
mod conventions;
mod import_memory;
mod perf_bench;
mod transport_bar;
