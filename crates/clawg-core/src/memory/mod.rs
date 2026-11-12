//! Persistent-memory substrate: a SQLite-backed store with vector search.
//!
//! # Folders
//!
//! Each folder is one concern, and its code names only the folders listed
//! before it here, except that `upkeep` and `ingest` do not name each other,
//! `revision` names no `embedding`, and only `projects` and `upkeep` name
//! `project_docs`:
//!
//! - `domain`: what a memory is, apart from where it is kept.
//! - `store`: the SQLite store, its lock, its snapshots and its errors.
//! - `embedding`: from text to vectors.
//! - `retrieval`: reading memories back.
//! - `revision`: how a save stands to the ones before it.
//! - `project_docs`: the index of a project's documentation.
//! - `projects`: the projects memories belong to.
//! - `ingest`: saving.
//! - `upkeep`: what keeps a store right over time.
//! - `portability`: export, import and their undo.
//!
//! Code of this crate names a module by its folder
//! (`crate::memory::store::db`). The other crates name the modules this root
//! re-exports (`clawg_core::memory::db`).
//!
//! # Architecture
//!
//! A memory is a row of `memory`, with its embedding in the `vec0` virtual
//! table `memory_vectors` for search. The rest of the schema (projects, the
//! docs index, the ingest outbox, conflicts) is built around it by the
//! refinery migrations in
//! `crates/clawg-core/migrations/`, which [`store::migrations`] embeds; opening a store
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

pub(crate) mod domain;
pub(crate) mod embedding;
pub(crate) mod ingest;
pub mod portability;
pub mod project_docs;
pub(crate) mod projects;
pub(crate) mod retrieval;
pub(crate) mod revision;
pub(crate) mod store;
pub(crate) mod upkeep;

pub use domain::clock;
pub use domain::events;
pub use domain::provenance;
pub use domain::redact;
pub use domain::sharing;
pub use domain::vector;

pub use store::backup;
pub use store::db;
pub use store::store_lock;

pub use embedding::chunker;
pub use embedding::embedder;
pub use embedding::idle_embedder;

pub use retrieval::language;
pub use retrieval::prompt_builder;
pub use retrieval::workspace;

pub use revision::dispute;
pub use revision::topic;

pub use projects::identity;
pub use projects::inspect;
pub use projects::merge;
pub use projects::project;
pub use projects::project_delete;
pub use projects::registry;
pub use projects::relocate;
pub use projects::subfolder;

pub use ingest::outbox;
pub use ingest::spill;
pub use ingest::store_insert;
pub use ingest::worker;

pub use upkeep::admin;
pub use upkeep::bootstrap;
pub use upkeep::secrets;
pub use upkeep::synthesis;

/// `project_docs` names these two by the path of this root.
pub(crate) use retrieval::search;
#[cfg(test)]
pub(crate) use embedding::model_download;

pub use domain::record::{MemoryRecord, MemoryRow};
pub use projects::identity::{derive_project_id, ProjectIdentity};
/// Re-exported at the `memory` root because structured retrieval is now an
/// entry point in its own right: the `clawg serve` daemon and the `clawg mcp`
/// bridge call it directly, without the chat pipeline that used to be the only
/// caller. Keeping the import path one level shorter than `memory::search::…`
/// mirrors how [`MemoryStore`] is already re-exported out of `db`.
pub use retrieval::pieces::{read_save_core, SaveText};
pub use retrieval::search::{search_all_memories, PiecePosition, RankedHit, SearchScope};
/// No other crate names it: `rag-eval` reads the pool through `eval_api`. It
/// stays public so that its fields count as read in a build without `eval`.
/// Private to the crate, that build would have to stop filling it, and a
/// search would count its pool under test and not in a release.
pub use retrieval::search::MemoryPool;
pub use revision::contradiction::Candidate;
pub use store::db::MemoryStore;
pub use store::error::{MemoryError, StorageError};
