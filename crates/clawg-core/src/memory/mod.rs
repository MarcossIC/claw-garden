//! Persistent-memory substrate: a SQLite-backed store with vector search.
//!
//! # Architecture
//!
//! Provides a SQLite-backed store with three tables (`memory`, `episodic_trajectory`,
//! `procedural_skill`), three matching `vec0` virtual tables for embedding search, and a
//! `UNION ALL` view `all_memories` for cross-kind queries. Three refinery migrations
//! (`V1`–`V3`) create the full schema on cold start.
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
pub mod db;
pub mod embedder;
pub mod events;
pub mod idle_embedder;
pub mod store_insert;
pub mod store_lock;
pub mod worker;
pub mod error;
pub mod identity;
pub mod migrations;
pub mod outbox;
pub mod portability;
pub(crate) mod pieces;
pub mod project;
pub mod project_docs;
pub mod provenance;
pub mod model_download;
pub mod record;
pub mod redact;
pub mod secrets;
pub mod sharing;

pub use contradiction::Candidate;
pub use db::MemoryStore;
pub use error::MemoryError;
pub use identity::{derive_project_id, ProjectIdentity};
pub use record::{
    EpisodicTrajectoryRow, MemoryRecord, MemoryRow, ProceduralSkillRow, TableKind,
};
/// Re-exported at the `memory` root because structured retrieval is now an
/// entry point in its own right: the `clawg serve` daemon and the `clawg mcp`
/// bridge call it directly, without the chat pipeline that used to be the only
/// caller. Keeping the import path one level shorter than `memory::search::…`
/// mirrors how [`MemoryStore`] is already re-exported out of `db`.
pub use pieces::{read_save_core, SaveText};
pub use search::{search_all_memories, PiecePosition, RankedHit, SearchScope};

pub mod clock;
pub(crate) mod latency;
pub mod access;
pub(crate) mod score;
pub(crate) mod tier;
pub(crate) mod dedup;
pub mod search;
pub mod workspace;
pub mod prompt_builder;
pub(crate) mod contradiction;
pub mod spill;
pub mod maintenance;
pub mod query_context;
pub mod vector;
pub mod admin;
pub mod dispute;
pub mod synthesis;
pub mod topic;
pub(crate) mod revival;
