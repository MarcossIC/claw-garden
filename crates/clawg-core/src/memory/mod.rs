//! Persistent-memory substrate for the v1 RAG roadmap (Phase 1 of 7).
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
//! thread). Therefore, callers may `.clone()` the store freely and the outer `Arc` in
//! `AppState` is only a cheap reference-count, not exclusion.
//!
//! # `disputed_against`
//!
//! The `disputed_against` column stores a JSON array of `i64` IDs as raw TEXT.
//! [`MemoryRecord::disputed_against`] hands back that raw string;
//! [`MemoryRecord::disputed_against_ids`] parses it into `Vec<i64>`, returning
//! `None` for NULL or malformed JSON rather than panicking.

pub mod chunker;
pub mod db;
pub mod embedder;
pub mod events;
pub mod store_insert;
pub mod worker;
pub mod error;
pub mod identity;
pub mod migrations;
pub mod model_download;
pub mod record;

pub use db::MemoryStore;
pub use error::MemoryError;
pub use identity::derive_project_id;
pub use record::{
    EpisodicTrajectoryRow, MemoryRecord, MemoryRow, ProceduralSkillRow, Scope, TableKind,
};

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
pub mod resolver;
pub mod extractor;
pub(crate) mod episodic_writer;
pub mod miner;
pub mod maintenance;
pub mod query_context;
pub mod vector;
pub mod admin;
