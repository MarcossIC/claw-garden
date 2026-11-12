//! Persistent-memory substrate: a SQLite-backed store with vector search.
//!
//! # Architecture
//!
//! A memory is a row of `memory`, with its embedding in the `vec0` virtual
//! table `memory_vectors` for search. The rest of the schema (projects, the
//! docs index, the ingest outbox, conflicts) is built around it by the
//! refinery migrations in
//! `crates/clawg-core/migrations/`, which [`migrations`] embeds; opening a store
//! applies the ones it has not run yet.
//!
//! # `MemoryStore` is `Clone` — no `Mutex` needed
//!
//! [`MemoryStore`] wraps a [`tokio_rusqlite::Connection`], which is itself `Clone`
//! (it is a channel handle — cloning hands you another handle to the same background
//! thread). Therefore, callers may `.clone()` the store freely, and wrapping it in an
//! `Arc` is only a cheap reference-count, not exclusion.
//!
//! # `disputed_against`
//!
//! The `disputed_against` column stores a JSON array of `i64` IDs as raw TEXT.
//! [`MemoryRecord::disputed_against`] hands back that raw string;
//! [`MemoryRecord::disputed_against_ids`] parses it into `Vec<i64>`, returning
//! `None` for NULL or malformed JSON rather than panicking.

pub mod backup;
pub mod bootstrap;
pub mod chunker;
pub(crate) mod conn_probe;
pub mod db;
pub mod embedder;
pub mod events;
pub mod idle_embedder;
pub mod store_insert;
pub mod store_lock;
pub mod worker;
pub(crate) mod error;
pub mod identity;
pub mod language;
pub mod merge;
pub(crate) mod migrations;
pub mod outbox;
pub mod portability;
pub(crate) mod pieces;
pub mod project;
pub mod project_delete;
pub mod project_docs;
pub mod provenance;
pub(crate) mod model_download;
pub(crate) mod record;
pub mod redact;
pub mod registry;
pub mod relocate;
pub mod inspect;
pub mod secrets;
pub mod sharing;
pub mod subfolder;

pub use contradiction::Candidate;
pub use db::MemoryStore;
pub use error::{MemoryError, StorageError};
pub use identity::{derive_project_id, ProjectIdentity};
pub use record::{MemoryRecord, MemoryRow};
/// Re-exported at the `memory` root because structured retrieval is now an
/// entry point in its own right: the `clawg serve` daemon and the `clawg mcp`
/// bridge call it directly, without the chat pipeline that used to be the only
/// caller. Keeping the import path one level shorter than `memory::search::…`
/// mirrors how [`MemoryStore`] is already re-exported out of `db`.
pub use pieces::{read_save_core, SaveText};
pub use search::{search_all_memories, PiecePosition, RankedHit, SearchScope};
/// No other crate names it: `rag-eval` reads the pool through `eval_api`. It
/// stays public so that its fields count as read in a build without `eval`.
/// Private to the crate, that build would have to stop filling it, and a
/// search would count its pool under test and not in a release.
pub use search::MemoryPool;

pub mod clock;
pub(crate) mod latency;
pub(crate) mod access;
pub(crate) mod score;
pub(crate) mod tier;
pub(crate) mod dedup;
pub(crate) mod search;
pub(crate) mod search_log;
pub mod workspace;
pub mod prompt_builder;
pub(crate) mod contradiction;
pub mod spill;
pub(crate) mod maintenance;
#[cfg(any(test, feature = "eval"))]
pub(crate) mod query_context;
pub mod vector;
pub mod admin;
pub mod dispute;
pub mod synthesis;
pub mod topic;
pub(crate) mod revival;
#[cfg(test)]
mod crowding;
