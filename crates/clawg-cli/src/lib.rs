//! `clawg` — one binary, four subcommands, two processes.
//!
//! # Why a library beside the binary
//!
//! Everything below is reachable from integration tests without running the
//! real executable. That matters more here than in a typical CLI: `clawg serve`
//! owns a 470 MB embedding model download, so a test that spawned the binary
//! would either download it or be quarantined. Instead `daemon::api::router`
//! takes an [`daemon::api::AppCtx`] a test can build from
//! `MemoryRuntime::new_for_test`, and `client::DaemonClient` talks to whatever
//! address it is pointed at — so the whole HTTP contract is exercised
//! in-process, and only `daemon::serve::run` (the thin wiring that opens the
//! real store and starts the real bootstrap) stays untested.
//!
//! # The process split
//!
//! `clawg serve` is the daemon and the ONLY owner of SQLite, the embedder and
//! the background workers. `clawg mcp` is a stdio↔HTTP bridge with no state of
//! its own; it starts a daemon if none answers. Keeping one owner of the
//! database is not a style choice — the ingest pipeline's dedup gate is only
//! correct while a single sequential consumer drains the channel (see
//! `MemoryRuntime::ingest_worker`), and two processes with the same `memory.db`
//! would each have one. So ownership is enforced, not agreed on: the store lock
//! taken inside `MemoryStore::open` (`clawg_core::memory::store_lock`) refuses
//! a second opener, whether it is another daemon or an evaluation run.

pub mod cli;
pub mod client;
pub mod daemon;
#[cfg(windows)]
pub mod detach;
pub mod doctor;
pub mod export;
pub mod hook;
pub mod import;
pub mod mcp;
pub mod memories;
pub mod metrics;
pub mod outbox;
pub mod process;
pub mod projects;
pub mod relevance;
pub mod restore;
pub mod secrets;
pub mod serve_log;
pub mod setup;
pub mod sharing;
pub mod status;
pub mod stop;
pub mod undelete;
pub mod update;
pub mod user_path;
