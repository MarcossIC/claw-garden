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

// Re-export so callers can write `crate::memory::store::migrations::runner()`.
pub use migrations::runner;

use refinery_core::traits::sync::{Query, Transaction};
use rusqlite::types::Type;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

/// The connection refinery migrates through.
///
/// refinery's own rusqlite driver accepts a closed range of rusqlite versions,
/// which would decide the SQLite this crate bundles. The driver is the three
/// traits below, so it lives here and refinery is built without it.
pub struct Driver<'c>(pub &'c mut rusqlite::Connection);

impl Transaction for Driver<'_> {
    type Error = rusqlite::Error;

    fn execute<'a, T: Iterator<Item = &'a str>>(&mut self, queries: T) -> Result<usize, Self::Error> {
        let transaction = self.0.transaction()?;
        let mut count = 0;
        for query in queries {
            transaction.execute_batch(query)?;
            count += 1;
        }
        transaction.commit()?;
        Ok(count)
    }
}

impl Query<Vec<refinery::Migration>> for Driver<'_> {
    fn query(&mut self, query: &str) -> Result<Vec<refinery::Migration>, Self::Error> {
        let transaction = self.0.transaction()?;
        let applied = {
            let mut stmt = transaction.prepare(query)?;
            let rows = stmt.query_map([], applied_migration)?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        transaction.commit()?;
        Ok(applied)
    }
}

impl refinery::Migrate for Driver<'_> {}

/// One row of the history table: version, name, applied_on, checksum.
fn applied_migration(row: &rusqlite::Row<'_>) -> rusqlite::Result<refinery::Migration> {
    let applied_on: String = row.get(2)?;
    let applied_on = OffsetDateTime::parse(&applied_on, &Rfc3339)
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(2, Type::Text, Box::new(e)))?;
    let checksum: String = row.get(3)?;
    let checksum = checksum
        .parse::<u64>()
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(3, Type::Text, Box::new(e)))?;
    Ok(refinery::Migration::applied(row.get(0)?, row.get(1)?, applied_on, checksum))
}

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
            (20, "project_scoped_indexes", 11833462148711770144),
            (21, "memory_move", 7771689620137773022),
            (22, "memory_lang", 10609892886635042350),
            (23, "origin_project", 6076322649061703623),
            (24, "project_remote", 579166074213349048),
            (25, "dedup_index_all_rows", 10903318606938253393),
            (26, "import_journal", 15286911047364848905),
            (27, "project_registry", 14271020849722783756),
            (28, "project_merge", 13822382867550515604),
            (29, "project_delete", 13889377801103386605),
            (30, "project_subfolder", 596964247716918935),
            (31, "import_registry", 7644682682507967915),
            (32, "retire_legacy_kinds", 10641941920249750761),
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

    fn two_migrations() -> Vec<refinery::Migration> {
        vec![
            refinery::Migration::unapplied("V1__first", "CREATE TABLE first (x INTEGER);").unwrap(),
            refinery::Migration::unapplied("V2__second", "CREATE TABLE second (x INTEGER);").unwrap(),
        ]
    }

    #[test]
    fn the_driver_applies_migrations_and_reads_them_back() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        let runner = refinery::Runner::new(&two_migrations());

        let first = runner.run(&mut Driver(&mut conn)).unwrap();
        assert_eq!(first.applied_migrations().len(), 2);

        let applied = runner.get_applied_migrations(&mut Driver(&mut conn)).unwrap();
        let read: Vec<(i32, &str, u64)> = applied.iter().map(|m| (m.version(), m.name(), m.checksum())).collect();
        let embedded = two_migrations();
        let expected: Vec<(i32, &str, u64)> = embedded.iter().map(|m| (m.version(), m.name(), m.checksum())).collect();
        assert_eq!(read, expected);
        assert!(applied.iter().all(|m| m.applied_on().is_some()));

        let second = runner.run(&mut Driver(&mut conn)).unwrap();
        assert!(second.applied_migrations().is_empty());
    }

    #[test]
    fn the_driver_rolls_back_a_group_that_fails() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        let mut migrations = two_migrations();
        migrations.push(refinery::Migration::unapplied("V3__broken", "THIS IS NOT SQL;").unwrap());

        let outcome = refinery::Runner::new(&migrations).set_grouped(true).run(&mut Driver(&mut conn));
        assert!(outcome.is_err());

        let tables: i64 = conn
            .query_row("SELECT COUNT(*) FROM sqlite_master WHERE name IN ('first', 'second')", [], |r| r.get(0))
            .unwrap();
        assert_eq!(tables, 0);
    }

    #[test]
    fn a_history_row_with_a_bad_checksum_is_an_error_not_a_panic() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        let runner = refinery::Runner::new(&two_migrations());
        runner.run(&mut Driver(&mut conn)).unwrap();
        conn.execute("UPDATE refinery_schema_history SET checksum = 'not a number' WHERE version = 2", []).unwrap();

        assert!(runner.get_applied_migrations(&mut Driver(&mut conn)).is_err());
    }

    // ── the rows of episodic_trajectory and procedural_skill ────────────

    use std::path::Path;

    use crate::memory::upkeep::admin::restore_memory_core;
    use crate::memory::store::db::{ensure_vec0_registered, seed_on_disk, MemoryStore};
    use crate::memory::embedding::embedder::{EmbedRole, Embedder, FakeEmbedder};
    use crate::memory::retrieval::search::{search_all_memories, SearchScope};
    use crate::memory::store::store_lock::StoreHolder;
    use crate::test_dir::{test_dir, TestDir};

    /// The last schema with the two tables. Fixed, not "the latest minus
    /// one": the rows are written by SQL against this schema.
    const BEFORE: i32 = 31;
    const REPO: &str = "repo-1a2b";
    const GONE: &str = "gone-3c4d";

    async fn unit_bytes(text: &str) -> Vec<u8> {
        let v = FakeEmbedder::default().embed_with_role(text, EmbedRole::Passage).await.unwrap();
        crate::memory::domain::vector::to_unit(&v).unwrap().iter().flat_map(|f| f.to_le_bytes()).collect()
    }

    /// A store at [`BEFORE`] with `seed` written into it, closed. It holds
    /// the row `seed_on_disk` writes, id 1, and `memory` has given ids up to
    /// 7, so the rows that move start at 8.
    async fn store_before(seed: &str, vectors: &[(&str, i64, &str)]) -> TestDir {
        let dir = test_dir();
        seed_on_disk(dir.path(), Some(BEFORE)).await;
        ensure_vec0_registered();
        let c = rusqlite::Connection::open(dir.path().join("memory.db")).unwrap();
        c.execute_batch(
            "INSERT INTO memory (id, scope, agent_id, content, created_at, last_modified_at, last_accessed_at) \
               VALUES (7, 'agent', 'agent-seed', 'a row purged before', 0, 0, 0);
             DELETE FROM memory WHERE id = 7;",
        )
        .unwrap();
        c.execute_batch(seed).unwrap();
        for &(table, id, text) in vectors {
            c.execute(&format!("INSERT INTO {table} (rowid, embedding) VALUES (?1, ?2)"), rusqlite::params![id, unit_bytes(text).await])
                .unwrap();
        }
        drop(c);
        dir
    }

    fn query<T>(dir: &Path, sql: &str, row: impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>) -> Vec<T> {
        ensure_vec0_registered();
        let c = rusqlite::Connection::open(dir.join("memory.db")).unwrap();
        let mut stmt = c.prepare(sql).unwrap();
        stmt.query_map([], row).unwrap().collect::<rusqlite::Result<_>>().unwrap()
    }

    async fn close(store: MemoryStore) {
        store.conn.clone().close().await.unwrap();
        drop(store);
    }

    /// The ids of the hits a search by `text`, as text and as a vector, finds
    /// for `agent-a` in [`REPO`].
    async fn found(store: &MemoryStore, text: &str) -> Vec<i64> {
        let v = FakeEmbedder::default().embed_with_role(text, EmbedRole::Query).await.unwrap();
        search_all_memories(store, &v, Some(text), "agent-a", SearchScope::all_default().with_project(Some(REPO)), 10)
            .await
            .unwrap()
            .into_iter()
            .map(|h| h.id)
            .collect()
    }

    const TRAJECTORY: &str = "depurar el timeout de red";
    const OLD_VERSION: &str = "migrar la base a sqlite";
    const NEW_VERSION: &str = "migrar la base a postgres";
    const DELETED: &str = "limpiar el cache del proyecto borrado";
    const SKILL: &str = "reparar-test-roto";
    const RIVAL: &str = "correr-la-suite";

    /// Four trajectories: one with every column, one replaced by another, and
    /// one deleted with the project it was in; two skills, the first
    /// disputed against the second and drawn from the first trajectory and
    /// from one that is not there; and a row the registration of the project
    /// rekeyed.
    async fn legacy_rows() -> TestDir {
        let seed = format!(
            "INSERT INTO project (id, name, born, reviewed, created_at) VALUES ('{REPO}', 'repo', 'root', 1, 1), ('{GONE}', 'gone', 'root', 1, 1);
             UPDATE project SET deleted_at = 50 WHERE id = '{GONE}';
             INSERT INTO episodic_trajectory (id, scope, agent_id, project_id, session_id, task, tool_calls, errors_encountered, \
               resolution, success_score, root_cause, created_at, created_by_type, created_by_ref, last_modified_at, last_modified_by, \
               update_count, last_accessed_at, access_count, importance, tier, status, superseded_by, deleted_at) VALUES
               (1, 'agent', 'agent-a', '{REPO}', 's-1', '{TRAJECTORY}', '[\"read\",\"edit\"]', '[\"timeout\"]', \
                'subir el timeout a 30 s', 0.75, 'config', 10, 'agent', 'agent-a', 11, 'agent-a', 1, 12, 3, 6.0, 2, 'active', NULL, NULL),
               (2, 'agent', 'agent-a', '{REPO}', 's-2', '{OLD_VERSION}', '[]', '[]', NULL, 0.0, NULL, 13, 'agent', '', 13, NULL, 0, 13, 0, \
                5.0, 3, 'superseded', 3, NULL),
               (3, 'agent', 'agent-a', '{REPO}', 's-3', '{NEW_VERSION}', '[]', '[]', NULL, 1.0, NULL, 14, 'agent', '', 14, NULL, 0, 14, 0, \
                5.0, 3, 'active', NULL, NULL),
               (4, 'agent', 'agent-a', '{GONE}', 's-4', '{DELETED}', '[]', '[]', NULL, 0.0, NULL, 15, 'agent', '', 15, NULL, 0, 15, 0, \
                5.0, 3, 'active', NULL, 50);
             INSERT INTO procedural_skill (id, scope, agent_id, project_id, skill_name, trigger, tool_sequence, success_count, \
               promoted_at, source_trajectory_ids, created_at, created_by_type, created_by_ref, last_modified_at, last_accessed_at, \
               access_count, importance, tier, status, dispute_count, disputed_against, last_disputed_at) VALUES
               (1, 'agent', 'agent-a', '{REPO}', '{SKILL}', 'un test falla', '[\"run\",\"fix\"]', 2, 20, '[1,99]', 20, 'agent', \
                'agent-a', 20, 22, 4, 8.0, 1, 'disputed', 1, '[2]', 25),
               (2, 'agent', 'agent-a', '{REPO}', '{RIVAL}', 'antes de un commit', '[\"run\"]', 1, 21, '[3]', 21, 'agent', \
                'agent-a', 21, 21, 0, 8.0, 1, 'active', 0, NULL, NULL);
             INSERT INTO project_rekey (tbl, row_id, legacy_id, project_id, rekeyed_at) VALUES ('episodic_trajectory', 1, 'repo-0000', '{REPO}', 5);"
        );
        let vectors = [
            ("episodic_vectors", 1, TRAJECTORY),
            ("episodic_vectors", 2, OLD_VERSION),
            ("episodic_vectors", 3, NEW_VERSION),
            ("episodic_vectors", 4, DELETED),
            ("procedural_vectors", 1, SKILL),
            ("procedural_vectors", 2, RIVAL),
        ];
        store_before(&seed, &vectors).await
    }

    /// What a moved row holds of its own, the rest of `memory` aside.
    #[derive(Debug, Clone, PartialEq)]
    struct Moved {
        id: i64,
        memory_type: String,
        source: String,
        content: String,
        session: Option<String>,
        superseded_by: Option<i64>,
        status: String,
        cause: Option<String>,
        disputed_against: Option<String>,
        deleted_at: Option<i64>,
    }

    /// What a moved row carries over from the row it was, besides its text.
    #[derive(Debug, Clone, PartialEq)]
    struct Carried {
        id: i64,
        project_id: Option<String>,
        created_by_ref: String,
        scope: String,
        last_modified_by: Option<String>,
        created_at: i64,
        last_modified_at: i64,
        update_count: i64,
        access_count: i64,
        importance: f64,
        tier: i64,
        last_disputed_at: Option<i64>,
    }

    fn moved_rows(dir: &Path) -> Vec<Moved> {
        query(
            dir,
            "SELECT id, memory_type, source, content, source_session_id, superseded_by, status, supersede_cause, disputed_against, \
                    deleted_at FROM memory WHERE id >= 8 ORDER BY id",
            |r| {
                Ok(Moved {
                    id: r.get(0)?,
                    memory_type: r.get(1)?,
                    source: r.get(2)?,
                    content: r.get(3)?,
                    session: r.get(4)?,
                    superseded_by: r.get(5)?,
                    status: r.get(6)?,
                    cause: r.get(7)?,
                    disputed_against: r.get(8)?,
                    deleted_at: r.get(9)?,
                })
            },
        )
    }

    fn fact(id: i64, source: &str, content: &str, session: Option<&str>) -> Moved {
        Moved {
            id,
            memory_type: "semantic_fact".into(),
            source: source.into(),
            content: content.into(),
            session: session.map(str::to_string),
            superseded_by: None,
            status: "active".into(),
            cause: None,
            disputed_against: None,
            deleted_at: None,
        }
    }

    #[tokio::test]
    async fn trajectories_and_skills_move_to_memory_and_their_tables_go() {
        let dir = legacy_rows().await;
        let store = MemoryStore::open(dir.path(), StoreHolder::Eval).await.unwrap();

        for (text, id) in [(TRAJECTORY, 8), (NEW_VERSION, 10), (SKILL, 12), (RIVAL, 13)] {
            assert!(found(&store, text).await.contains(&id), "#{id} {text:?} is found");
        }
        assert!(!found(&store, DELETED).await.contains(&11), "a deleted row is not found");
        close(store).await;

        let moved: Vec<(i64, String, i64)> =
            query(dir.path(), "SELECT new_id, tbl, old_id FROM legacy_kind_move ORDER BY new_id", |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)));
        let trajectory = "episodic_trajectory".to_string();
        let skill = "procedural_skill".to_string();
        assert_eq!(
            moved,
            [(8, trajectory.clone(), 1), (9, trajectory.clone(), 2), (10, trajectory.clone(), 3), (11, trajectory, 4), (12, skill.clone(), 1), (13, skill, 2)]
        );

        assert_eq!(
            moved_rows(dir.path()),
            [
                fact(
                    8,
                    "episodic_trajectory",
                    "depurar el timeout de red\nresolution: subir el timeout a 30 s\nroot_cause: config\n\
                     tool_calls: [\"read\",\"edit\"]\nerrors_encountered: [\"timeout\"]\nsuccess_score: 0.75\nsession_id: s-1",
                    Some("s-1"),
                ),
                Moved {
                    superseded_by: Some(10),
                    status: "superseded".into(),
                    cause: Some("verdict".into()),
                    ..fact(
                        9,
                        "episodic_trajectory",
                        "migrar la base a sqlite\ntool_calls: []\nerrors_encountered: []\nsuccess_score: 0.0\nsession_id: s-2",
                        Some("s-2"),
                    )
                },
                fact(
                    10,
                    "episodic_trajectory",
                    "migrar la base a postgres\ntool_calls: []\nerrors_encountered: []\nsuccess_score: 1.0\nsession_id: s-3",
                    Some("s-3"),
                ),
                Moved {
                    deleted_at: Some(50),
                    ..fact(
                        11,
                        "episodic_trajectory",
                        "limpiar el cache del proyecto borrado\ntool_calls: []\nerrors_encountered: []\nsuccess_score: 0.0\nsession_id: s-4",
                        Some("s-4"),
                    )
                },
                Moved {
                    status: "disputed".into(),
                    disputed_against: Some("[13]".into()),
                    ..fact(
                        12,
                        "procedural_skill",
                        "reparar-test-roto\ntrigger: un test falla\ntool_sequence: [\"run\",\"fix\"]\nsuccess_count: 2\n\
                         promoted_at: 20\nsource_trajectory_ids: [8]",
                        None,
                    )
                },
                fact(
                    13,
                    "procedural_skill",
                    "correr-la-suite\ntrigger: antes de un commit\ntool_sequence: [\"run\"]\nsuccess_count: 1\n\
                     promoted_at: 21\nsource_trajectory_ids: [10]",
                    None,
                ),
            ]
        );

        let carried = query(
            dir.path(),
            "SELECT id, project_id, created_by_ref, scope, last_modified_by, created_at, last_modified_at, update_count, \
                    access_count, importance, tier, last_disputed_at FROM memory WHERE id IN (8, 12) ORDER BY id",
            |r| {
                Ok(Carried {
                    id: r.get(0)?,
                    project_id: r.get(1)?,
                    created_by_ref: r.get(2)?,
                    scope: r.get(3)?,
                    last_modified_by: r.get(4)?,
                    created_at: r.get(5)?,
                    last_modified_at: r.get(6)?,
                    update_count: r.get(7)?,
                    access_count: r.get(8)?,
                    importance: r.get(9)?,
                    tier: r.get(10)?,
                    last_disputed_at: r.get(11)?,
                })
            },
        );
        let by_agent = |id, created_at, last_modified_at, update_count, access_count, importance, tier| Carried {
            id,
            project_id: Some(REPO.to_string()),
            created_by_ref: "agent-a".into(),
            scope: "agent".into(),
            last_modified_by: None,
            created_at,
            last_modified_at,
            update_count,
            access_count,
            importance,
            tier,
            last_disputed_at: None,
        };
        assert_eq!(
            carried,
            [
                Carried { last_modified_by: Some("agent-a".into()), ..by_agent(8, 10, 11, 1, 3, 6.0, 2) },
                Carried { last_disputed_at: Some(25), ..by_agent(12, 20, 20, 0, 4, 8.0, 1) },
            ]
        );
        let hashes: Vec<(String, Option<String>)> =
            query(dir.path(), "SELECT content, content_hash FROM memory WHERE id >= 8 ORDER BY id", |r| Ok((r.get(0)?, r.get(1)?)));
        for (content, hash) in hashes {
            assert_eq!(hash, Some(crate::memory::store::dedup::content_hash(&content)), "{content:?}");
        }

        let vectors: Vec<(i64, Vec<u8>)> =
            query(dir.path(), "SELECT rowid, embedding FROM memory_vectors ORDER BY rowid", |r| Ok((r.get(0)?, r.get(1)?)));
        let mut expected = Vec::new();
        for (id, text) in [(8, TRAJECTORY), (9, OLD_VERSION), (10, NEW_VERSION), (12, SKILL), (13, RIVAL)] {
            expected.push((id, unit_bytes(text).await));
        }
        assert_eq!(vectors, expected, "each live row keeps its vector, the deleted one has none");

        let indexed: Vec<i64> = query(dir.path(), "SELECT rowid FROM memory_fts WHERE memory_fts MATCH 'limpiar OR depurar'", |r| r.get(0));
        assert_eq!(indexed, [8], "a deleted row is not in the text index");
        let pairs: Vec<(i64, i64, String, i64)> =
            query(dir.path(), "SELECT new_id, old_id, state, created_at FROM memory_conflict", |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)));
        assert_eq!(pairs, [(13, 12, "escalated".to_string(), 25)]);
        let rekeyed: Vec<(String, i64)> = query(dir.path(), "SELECT tbl, row_id FROM project_rekey", |r| Ok((r.get(0)?, r.get(1)?)));
        assert_eq!(rekeyed, [("memory".to_string(), 8)]);

        let projects: Vec<(String, Option<i64>, bool)> =
            query(dir.path(), "SELECT id, deleted_at, reviewed FROM project ORDER BY id", |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)));
        assert_eq!(projects, [(GONE.to_string(), Some(50), true), (REPO.to_string(), None, true)], "no project changed");
        let seeded: Vec<(i64, String, String)> =
            query(dir.path(), "SELECT id, content, memory_type FROM memory WHERE id < 8", |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)));
        assert_eq!(seeded, [(1, "seeded on disk".to_string(), "chat_message".to_string())]);
        let left: Vec<String> = query(
            dir.path(),
            "SELECT name FROM sqlite_master WHERE name LIKE 'episodic%' OR name LIKE 'procedural%' OR name LIKE 'idx_epi%' OR name LIKE 'idx_proc%'",
            |r| r.get(0),
        );
        assert!(left.is_empty(), "{left:?}");
    }

    #[tokio::test]
    async fn a_store_moved_from_trajectories_and_skills_breaks_no_invariant() {
        let dir = legacy_rows().await;
        close(MemoryStore::open(dir.path(), StoreHolder::Eval).await.unwrap()).await;

        assert_eq!(moved_rows(dir.path()).len(), 6, "every row moved");
        let found = crate::memory::projects::inspect::inspect(dir.path());
        let facts = found.facts.expect("a migrated store is read");
        assert!(facts.invariants.is_empty(), "{:?}", facts.invariants);
    }

    #[tokio::test]
    async fn a_project_deleted_before_the_move_comes_back_with_its_trajectories_and_skills() {
        let dir = store_before(
            &format!(
                "INSERT INTO project (id, name, born, reviewed, created_at, deleted_at) VALUES ('{REPO}', 'repo', 'root', 1, 1, 100);
                 INSERT INTO project_delete (delete_id, project_id, with_memories, code, snapshot, deleted_at) VALUES (1, '{REPO}', 1, 'abcd1234', NULL, 100);
                 INSERT INTO episodic_trajectory (id, scope, agent_id, project_id, session_id, task, created_at, last_modified_at, last_accessed_at, deleted_at) \
                   VALUES (1, 'agent', 'agent-a', '{REPO}', 's-1', '{TRAJECTORY}', 1, 1, 1, 100);
                 INSERT INTO procedural_skill (id, scope, agent_id, project_id, skill_name, trigger, promoted_at, source_trajectory_ids, created_at, \
                   last_modified_at, last_accessed_at, deleted_at) VALUES (1, 'agent', 'agent-a', '{REPO}', '{SKILL}', 'un test falla', 1, '[1]', 1, 1, 1, 100);
                 INSERT INTO project_delete_row (delete_id, tbl, row_id, deleted_at) VALUES (1, 'episodic_trajectory', 1, NULL), (1, 'procedural_skill', 1, NULL);"
            ),
            &[],
        )
        .await;
        let store = MemoryStore::open(dir.path(), StoreHolder::Eval).await.unwrap();
        let backups = dir.path().join("backups");

        let report = crate::memory::projects::project_delete::undo_delete_core(&store, Some(&FakeEmbedder::default()), &backups, 1, false)
            .await
            .unwrap();

        assert_eq!(report.restored.memories, 2, "{report:?}");
        assert!(report.kept.is_empty(), "{report:?}");
        assert!(found(&store, TRAJECTORY).await.contains(&8));
        assert!(found(&store, SKILL).await.contains(&9));
        close(store).await;
        let rows: Vec<(i64, String, Option<i64>, bool)> = query(
            dir.path(),
            "SELECT id, memory_type, deleted_at, EXISTS (SELECT 1 FROM memory_vectors WHERE rowid = memory.id) FROM memory WHERE id >= 8 ORDER BY id",
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        );
        assert_eq!(rows, [(8, "semantic_fact".to_string(), None, true), (9, "semantic_fact".to_string(), None, true)]);
        let project: Vec<Option<i64>> = query(dir.path(), &format!("SELECT deleted_at FROM project WHERE id = '{REPO}'"), |r| r.get(0));
        assert_eq!(project, [None]);
    }

    #[tokio::test]
    async fn a_merge_from_before_the_move_takes_its_trajectories_and_skills_back() {
        const APP: &str = "app-aaaa";
        let dir = store_before(
            &format!(
                "INSERT INTO project (id, name, born, reviewed, created_at, merged_into, merged_at) VALUES \
                   ('{APP}', 'app', 'root', 1, 1, '{REPO}', 100), ('{REPO}', 'repo', 'root', 1, 1, NULL, NULL);
                 INSERT INTO episodic_trajectory (id, scope, agent_id, project_id, session_id, task, created_at, last_modified_at, last_accessed_at) \
                   VALUES (1, 'agent', 'agent-a', '{REPO}', 's-1', '{TRAJECTORY}', 1, 1, 1);
                 INSERT INTO procedural_skill (id, scope, agent_id, project_id, skill_name, trigger, promoted_at, source_trajectory_ids, created_at, \
                   last_modified_at, last_accessed_at) VALUES (1, 'agent', 'agent-a', '{REPO}', '{SKILL}', 'un test falla', 1, '[1]', 1, 1, 1);
                 INSERT INTO project_merge (merge_id, source, target, move_id, snapshot, merged_at) VALUES (1, '{APP}', '{REPO}', NULL, NULL, 100);
                 INSERT INTO project_merge_change (merge_id, kind, a, b) VALUES (1, 'trajectory', '1', NULL), (1, 'skill', '1', NULL), \
                   (1, 'tombstoned', '{APP}', NULL);"
            ),
            &[("episodic_vectors", 1, TRAJECTORY), ("procedural_vectors", 1, SKILL)],
        )
        .await;
        let store = MemoryStore::open(dir.path(), StoreHolder::Eval).await.unwrap();
        let workspaces = crate::memory::retrieval::workspace::WorkspaceManager::new(dir.path().join("workspaces"));

        let report = crate::memory::projects::merge::undo_merge_core(&store, &workspaces, &dir.path().join("backups"), 1, false).await.unwrap();

        assert_eq!(report.memories, 2, "{report:?}");
        close(store).await;
        let rows: Vec<(i64, Option<String>)> = query(dir.path(), "SELECT id, project_id FROM memory WHERE id >= 8 ORDER BY id", |r| Ok((r.get(0)?, r.get(1)?)));
        assert_eq!(rows, [(8, Some(APP.to_string())), (9, Some(APP.to_string()))]);
        let app: Vec<Option<String>> = query(dir.path(), &format!("SELECT merged_into FROM project WHERE id = '{APP}'"), |r| r.get(0));
        assert_eq!(app, [None]);
    }

    #[tokio::test]
    async fn a_trajectory_and_a_skill_deleted_before_the_move_are_restored_and_purged() {
        let dir = store_before(
            &format!(
                "INSERT INTO project (id, name, born, reviewed, created_at) VALUES ('{REPO}', 'repo', 'root', 1, 1);
                 INSERT INTO episodic_trajectory (id, scope, agent_id, project_id, session_id, task, created_at, last_modified_at, last_accessed_at, deleted_at) \
                   VALUES (1, 'agent', 'agent-a', '{REPO}', 's-1', '{TRAJECTORY}', 1, 1, 1, 100);
                 INSERT INTO procedural_skill (id, scope, agent_id, project_id, skill_name, trigger, promoted_at, source_trajectory_ids, created_at, \
                   last_modified_at, last_accessed_at, deleted_at) VALUES (1, 'agent', 'agent-a', '{REPO}', '{SKILL}', 'un test falla', 1, '[]', 1, 1, 1, 100);"
            ),
            &[],
        )
        .await;
        let store = MemoryStore::open(dir.path(), StoreHolder::Eval).await.unwrap();

        let restored = restore_memory_core(&store, &FakeEmbedder::default(), 8, None).await.unwrap();

        assert_eq!((restored.id, restored.status.as_str()), (8, "active"));
        assert!(found(&store, TRAJECTORY).await.contains(&8));
        close(store).await;
        crate::memory::upkeep::secrets::purge(dir.path(), 9).unwrap();
        let left: Vec<i64> = query(
            dir.path(),
            "SELECT id FROM memory WHERE id >= 8 UNION ALL SELECT rowid FROM memory_vectors WHERE rowid >= 8 ORDER BY 1",
            |r| r.get(0),
        );
        assert_eq!(left, [8, 8], "the restored row and its vector stay, the purged one is gone");
    }
}
