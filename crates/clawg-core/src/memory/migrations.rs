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

    /// E.1-test: Smoke test — runner() is callable without a DB connection.
    /// This is a compile-time + API-availability test.
    #[test]
    fn runner_is_accessible() {
        // Just call runner() — if the embed_migrations! macro and SQL files are
        // wired correctly, this returns a Runner without panic.
        let _runner = runner();
    }
}
