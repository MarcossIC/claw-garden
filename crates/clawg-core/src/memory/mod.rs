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

pub mod bootstrap;
pub mod portability;
pub mod project_docs;
pub mod secrets;

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

pub(crate) mod maintenance;
pub mod admin;
pub mod synthesis;

mod domain;
pub use domain::clock;
pub use domain::events;
pub use domain::provenance;
pub(crate) use domain::record;
pub use domain::redact;
pub use domain::sharing;
pub use domain::vector;

mod store;
pub use store::backup;
pub use store::db;
pub(crate) use store::dedup;
pub(crate) use store::error;
pub(crate) use store::latency;
pub(crate) use store::migrations;
pub use store::store_lock;
pub(crate) use store::tier;

mod embedding;
pub use embedding::chunker;
pub use embedding::embedder;
pub use embedding::idle_embedder;
pub(crate) use embedding::model_download;

mod retrieval;
pub(crate) use retrieval::access;
pub use retrieval::language;
pub(crate) use retrieval::listing;
pub(crate) use retrieval::pieces;
pub use retrieval::prompt_builder;
#[cfg(any(test, feature = "eval"))]
pub(crate) use retrieval::query_context;
pub(crate) use retrieval::score;
pub(crate) use retrieval::search;
pub(crate) use retrieval::search_log;
pub use retrieval::workspace;

mod revision;
pub(crate) use revision::contradiction;
pub use revision::dispute;
pub(crate) use revision::revival;
pub use revision::topic;

mod projects;
pub use projects::identity;
pub use projects::inspect;
pub use projects::merge;
pub use projects::project;
pub use projects::project_delete;
pub use projects::registry;
pub use projects::relocate;
pub use projects::subfolder;

mod ingest;
pub use ingest::outbox;
pub use ingest::spill;
pub use ingest::store_insert;
pub use ingest::worker;
