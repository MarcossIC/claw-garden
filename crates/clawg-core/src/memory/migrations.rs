//! Refinery migration runner for the memory subsystem.
//!
//! This module is intentionally thin — it exists to isolate the `embed_migrations!`
//! macro expansion from the connection-lifecycle code in `db.rs`.
//!
//! Migration files live at `crates/clawg-core/migrations/V{N}__{description}.sql`
//! (path is relative to the crate root, i.e. the directory containing `Cargo.toml`).

use refinery::embed_migrations;

// `embed_migrations!` generates an inner module named `migrations` with a `runner()`
// function that returns a `refinery::Runner`.
//
// `embed_migrations!` does not emit `cargo:rerun-if-changed` for the `migrations/`
// directory, so adding a new SQL file there does not by itself invalidate the
// build cache — `cargo test` can rerun a stale binary against a database the new
// migration was never embedded into. Touching THIS file — in the same commit
// as any new `V{N}__*.sql` migration — is what forces the recompile; the SQL
// file's own directory is not watched, so editing only that file is not enough.
embed_migrations!("migrations");

// Re-export so callers can write `crate::memory::migrations::runner()`.
pub use migrations::runner;

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke test — runner() is callable without a DB connection.
    /// This is a compile-time + API-availability test.
    #[test]
    fn runner_is_accessible() {
        // Just call runner() — if the embed_migrations! macro and SQL files are
        // wired correctly, this returns a Runner without panic.
        let _runner = runner();
    }

    /// Refinery refuses to open a database whose applied migrations no longer
    /// match their embedded checksums, so editing a published migration would
    /// leave every existing install without a daemon. A new migration adds its
    /// row here in the same commit.
    #[test]
    fn published_migration_checksums_are_pinned() {
        const PINNED: &[(i32, &str, u64)] = &[
            (1, "init", 5468576289350600607),
            (2, "vectors", 6383721682892048213),
            (3, "view", 5093982126784145775),
            (4, "root_cause", 11100773307424884808),
            (5, "embedding_contract", 2430145443532491464),
            (6, "content_hash", 13406404950456231160),
            (7, "drop_all_memories_view", 13752469436407078502),
            (8, "memory_fts", 11218029035149072530),
            (9, "project", 13889070755435575427),
            (10, "synthesis", 11323399618604098430),
            (11, "memory_piece", 17416414387852420399),
            (12, "project_docs", 4316694144498173558),
            (13, "project_index_docs", 13910146033917936477),
            (14, "ingest_outbox", 15167264139225320214),
            (15, "memory_conflict", 750788238593509675),
            (16, "topic_key", 2242188600417641488),
            (17, "revival", 10023441941658131595),
            (18, "session_indexes", 13764795356570932568),
            (19, "conflict_topic_key_warned", 10065829539537553357),
        ];
        let mut actual: Vec<(i32, String, u64)> = runner()
            .get_migrations()
            .iter()
            .map(|m| (m.version(), m.name().to_string(), m.checksum()))
            .collect();
        actual.sort_by_key(|&(v, _, _)| v);
        let pinned: Vec<(i32, String, u64)> = PINNED
            .iter()
            .map(|&(v, n, c)| (v, n.to_string(), c))
            .collect();
        assert_eq!(actual, pinned);
    }
}
