//! Reads a store's rows into the `clawg-memories` format.
//!
//! Through the read-only connection `clawg secrets` uses: no lock, nothing
//! written, so it works beside a running daemon. What it reads is what the
//! store committed; saves still waiting in the outbox are only counted.

use std::collections::BTreeSet;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension};

use crate::memory::backup::{known_migrations, open_for_probe, read_history, schema_plan, BackupError, SchemaPlan};
use crate::memory::portability::format::{
    ConflictLine, DerivationLine, Filter, Header, Line, MemoryLine, PieceLine, ProjectLine, FORMAT, VERSION,
};
use crate::memory::MemoryError;

/// Which rows an export takes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExportFilter {
    pub project_id: Option<String>,
    pub agent: Option<String>,
    pub include_deleted: bool,
}

/// What an export read, in the order the file lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct Export {
    pub header: Header,
    pub projects: Vec<ProjectLine>,
    pub memories: Vec<MemoryLine>,
    pub derivations: Vec<DerivationLine>,
    pub pieces: Vec<PieceLine>,
    pub conflicts: Vec<ConflictLine>,
    /// Saves of the exported agents still in the outbox, which do not travel.
    pub outbox_waiting: usize,
    /// Live rows of `episodic_trajectory` and `procedural_skill`, which
    /// nothing writes any more and which do not travel.
    pub legacy_rows: usize,
}

impl Export {
    /// The file: the header, then every line in order.
    pub fn to_jsonl(&self) -> String {
        let lines = std::iter::once(Line::Header(self.header.clone()))
            .chain(self.projects.iter().cloned().map(Line::Project))
            .chain(self.memories.iter().cloned().map(Line::Memory))
            .chain(self.derivations.iter().cloned().map(Line::Derivation))
            .chain(self.pieces.iter().cloned().map(Line::Piece))
            .chain(self.conflicts.iter().cloned().map(Line::Conflict));
        lines.map(|l| crate::memory::portability::format::render(&l)).collect()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error(transparent)]
    Store(#[from] MemoryError),
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("the store was written by a newer clawg (schema version {db}, this one knows up to {binary}); export it with that clawg")]
    Newer { db: i32, binary: i32 },
    #[error("the store's migration {0} differs from this clawg's, so it is not read")]
    Diverged(i32),
}

/// Exports the rows of `<base_dir>/memory.db` that `filter` takes.
pub fn export(base_dir: &Path, filter: &ExportFilter, now_ms: i64) -> Result<Export, ExportError> {
    let path = base_dir.join("memory.db");
    if !path.exists() {
        return Err(MemoryError::from(BackupError::NoStore { path }).into());
    }
    crate::memory::db::ensure_vec0_registered();
    let c = open_for_probe(&path)?;
    read(&c, filter, now_ms)
}

/// [`export`] over an open connection. A store from an older schema is read
/// with the columns and tables it has, and what it lacks is `null`.
pub(crate) fn read(c: &Connection, filter: &ExportFilter, now_ms: i64) -> Result<Export, ExportError> {
    match schema_plan(&read_history(c)?, &known_migrations()) {
        SchemaPlan::Newer { db, binary } => return Err(ExportError::Newer { db, binary }),
        SchemaPlan::Diverged { version } => return Err(ExportError::Diverged(version)),
        SchemaPlan::Fresh | SchemaPlan::UpToDate | SchemaPlan::Pending { .. } => {}
    }
    let memories = read_memories(c, filter)?;
    let ids: BTreeSet<i64> = memories.iter().map(|m| m.id).collect();
    let project_ids: BTreeSet<&str> = memories.iter().filter_map(|m| m.project_id.as_deref()).collect();

    let projects = if has_table(c, "project")? {
        collect(c, "SELECT id, root_path FROM project ORDER BY id", |r| {
            Ok(ProjectLine { id: r.get(0)?, root_path: r.get(1)? })
        })?
        .into_iter()
        .filter(|p| project_ids.contains(p.id.as_str()))
        .collect()
    } else {
        Vec::new()
    };
    let derivations = if has_table(c, "memory_derivation")? {
        collect(c, "SELECT memory_id, source_id, created_at FROM memory_derivation ORDER BY memory_id, source_id", |r| {
            Ok(DerivationLine { memory: r.get(0)?, source: r.get(1)?, created_at: r.get(2)? })
        })?
        .into_iter()
        .filter(|d| ids.contains(&d.memory))
        .collect()
    } else {
        Vec::new()
    };
    let pieces = if has_table(c, "memory_piece")? {
        collect(c, "SELECT save_id, piece, memory_id, context FROM memory_piece ORDER BY save_id, piece", |r| {
            Ok(PieceLine { save: r.get(0)?, piece: r.get(1)?, memory: r.get(2)?, context: r.get(3)? })
        })?
        .into_iter()
        .filter(|p| ids.contains(&p.memory))
        .collect()
    } else {
        Vec::new()
    };
    let conflicts = if has_table(c, "memory_conflict")? {
        let warned = if columns_of(c, "memory_conflict")?.contains("topic_key_warned") { "topic_key_warned" } else { "0" };
        collect(
            c,
            &format!(
                "SELECT new_id, old_id, distance, state, verdict, reason, created_at, judged_at, {warned} \
                 FROM memory_conflict ORDER BY new_id, old_id"
            ),
            |r| {
                Ok(ConflictLine {
                    new_id: r.get(0)?,
                    old_id: r.get(1)?,
                    distance: r.get(2)?,
                    state: r.get(3)?,
                    verdict: r.get(4)?,
                    reason: r.get(5)?,
                    created_at: r.get(6)?,
                    judged_at: r.get(7)?,
                    topic_key_warned: r.get(8)?,
                })
            },
        )?
        .into_iter()
        .filter(|p| ids.contains(&p.new_id) && ids.contains(&p.old_id))
        .collect()
    } else {
        Vec::new()
    };

    let embedding_model = if has_table(c, "embedding_contract")? {
        c.query_row("SELECT model_id FROM embedding_contract WHERE id = 1", [], |r| r.get(0)).optional()?
    } else {
        None
    };
    let outbox_waiting = if has_table(c, "ingest_outbox")? {
        c.query_row(
            "SELECT count(*) FROM ingest_outbox WHERE ?1 IS NULL OR agent_id = ?1",
            [&filter.agent],
            |r| r.get::<_, i64>(0),
        )? as usize
    } else {
        0
    };
    let legacy_rows = c.query_row(
        "SELECT (SELECT count(*) FROM episodic_trajectory WHERE deleted_at IS NULL) \
              + (SELECT count(*) FROM procedural_skill WHERE deleted_at IS NULL)",
        [],
        |r| r.get::<_, i64>(0),
    )? as usize;

    Ok(Export {
        header: Header {
            format: FORMAT.to_string(),
            version: VERSION,
            exported_at: now_ms,
            clawg: crate::build_info::version().to_string(),
            embedding_model,
            filter: Filter {
                project: filter.project_id.clone(),
                agent: filter.agent.clone(),
                include_deleted: filter.include_deleted,
            },
        },
        projects,
        memories,
        derivations,
        pieces,
        conflicts,
        outbox_waiting,
        legacy_rows,
    })
}

fn read_memories(c: &Connection, filter: &ExportFilter) -> rusqlite::Result<Vec<MemoryLine>> {
    let columns = columns_of(c, "memory")?;
    let or_null = |name: &str| if columns.contains(name) { name.to_string() } else { format!("NULL AS {name}") };
    let sql = format!(
        "SELECT id, scope, agent_id, project_id, content, memory_type, source, created_at, created_by_type, \
         created_by_ref, source_session_id, last_modified_at, last_modified_by, update_count, last_accessed_at, \
         access_count, importance, tier, status, superseded_by, {}, dispute_count, disputed_against, \
         last_disputed_at, deleted_at, {}, {}, {}, {} \
         FROM memory \
         WHERE (?1 IS NULL OR project_id = ?1) AND (?2 IS NULL OR agent_id = ?2) AND (?3 OR deleted_at IS NULL) \
         ORDER BY id",
        or_null("supersede_cause"),
        or_null("topic_key"),
        or_null("revived_from"),
        or_null("lang"),
        or_null("origin_project"),
    );
    let mut stmt = c.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params![filter.project_id, filter.agent, filter.include_deleted], |r| {
        let against: Option<String> = r.get(22)?;
        Ok(MemoryLine {
            id: r.get(0)?,
            scope: r.get(1)?,
            agent_id: r.get(2)?,
            project_id: r.get(3)?,
            content: r.get(4)?,
            memory_type: r.get(5)?,
            source: r.get(6)?,
            created_at: r.get(7)?,
            created_by_type: r.get(8)?,
            created_by_ref: r.get(9)?,
            source_session_id: r.get(10)?,
            last_modified_at: r.get(11)?,
            last_modified_by: r.get(12)?,
            update_count: r.get(13)?,
            last_accessed_at: r.get(14)?,
            access_count: r.get(15)?,
            importance: r.get(16)?,
            tier: r.get(17)?,
            status: r.get(18)?,
            superseded_by: r.get(19)?,
            supersede_cause: r.get(20)?,
            dispute_count: r.get(21)?,
            // A list that is not valid JSON names nothing, as for every
            // reader of the column.
            disputed_against: against.and_then(|a| serde_json::from_str(&a).ok()).unwrap_or_default(),
            last_disputed_at: r.get(23)?,
            deleted_at: r.get(24)?,
            topic_key: r.get(25)?,
            revived_from: r.get(26)?,
            lang: r.get(27)?,
            origin_project: r.get(28)?,
        })
    })?;
    rows.collect()
}

fn collect<T>(
    c: &Connection,
    sql: &str,
    f: impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
) -> rusqlite::Result<Vec<T>> {
    let mut stmt = c.prepare(sql)?;
    let rows = stmt.query_map([], f)?;
    rows.collect()
}

fn has_table(c: &Connection, name: &str) -> rusqlite::Result<bool> {
    c.query_row("SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)", [name], |r| r.get(0))
}

fn columns_of(c: &Connection, table: &str) -> rusqlite::Result<BTreeSet<String>> {
    let mut stmt = c.prepare("SELECT name FROM pragma_table_info(?1)")?;
    let names = stmt.query_map([table], |r| r.get(0))?;
    names.collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::db::seed_on_disk;
    use crate::memory::portability::fixtures::{other_project, project, seed};
    use crate::memory::portability::format::parse;
    use crate::memory::MemoryStore;

    async fn seeded_export(filter: ExportFilter) -> Export {
        let store = MemoryStore::new_for_test().await.unwrap();
        seed(&store).await;
        store.conn.call(move |c| Ok::<_, rusqlite::Error>(read(c, &filter, 42))).await.unwrap().unwrap()
    }

    fn ids(export: &Export) -> Vec<i64> {
        export.memories.iter().map(|m| m.id).collect()
    }

    #[tokio::test]
    async fn export_filters_by_project_agent_and_deleted() {
        let all = seeded_export(ExportFilter::default()).await;
        assert_eq!(ids(&all), vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 12, 13, 14, 15, 17], "deleted rows stay out");
        assert_eq!(all.projects.iter().map(|p| p.id.clone()).collect::<Vec<_>>(), {
            let mut both = vec![project(), other_project()];
            both.sort();
            both
        });
        assert_eq!((all.outbox_waiting, all.legacy_rows), (1, 0));
        assert_eq!(all.header.exported_at, 42);

        let with_deleted = seeded_export(ExportFilter { include_deleted: true, ..Default::default() }).await;
        assert_eq!(with_deleted.memories.len(), 17);
        let sixteen = with_deleted.memories.iter().find(|m| m.id == 16).unwrap();
        assert_eq!(sixteen.deleted_at, Some(1_757_000_000_000));

        let in_project = seeded_export(ExportFilter { project_id: Some(project()), ..Default::default() }).await;
        assert_eq!(ids(&in_project), vec![1, 2, 3, 4, 5, 6, 10, 12, 13, 15, 17]);
        assert_eq!(in_project.projects.len(), 1);
        assert_eq!(in_project.header.filter.project, Some(project()));

        let codex = seeded_export(ExportFilter { agent: Some("codex".into()), ..Default::default() }).await;
        assert_eq!(ids(&codex), vec![14]);
        assert_eq!(codex.projects, vec![ProjectLine { id: other_project(), root_path: "/work/other".into() }]);
        assert_eq!(codex.outbox_waiting, 0, "only the exported agent's saves are counted");
    }

    #[tokio::test]
    async fn every_column_travels() {
        let all = seeded_export(ExportFilter { include_deleted: true, ..Default::default() }).await;
        let row = |id: i64| all.memories.iter().find(|m| m.id == id).unwrap().clone();
        let one = row(1);
        assert_eq!(
            (one.status.as_str(), one.superseded_by, one.supersede_cause.as_deref(), one.topic_key.as_deref()),
            ("superseded", Some(2), Some("revision"), Some("storage/engine"))
        );
        assert_eq!((one.access_count, one.tier, one.update_count), (3, 2, 2));
        assert_eq!((one.created_at, one.last_accessed_at, one.last_modified_at), (1_750_000_001_000, 1_750_000_001_007, 1_750_000_001_005));
        assert_eq!(one.source_session_id.as_deref(), Some("session-9"));
        let four = row(4);
        assert_eq!((four.status.as_str(), four.disputed_against.clone(), four.dispute_count), ("disputed", vec![3], 1));
        assert_eq!(row(15).revived_from, Some(16));
        assert_eq!(row(10).source, "synthesis");
        let seventeen = row(17);
        assert_eq!((seventeen.scope.as_str(), seventeen.agent_id.clone()), ("project", None));
    }

    #[tokio::test]
    async fn an_export_carries_the_language_of_each_row() {
        let store = MemoryStore::new_for_test().await.unwrap();
        seed(&store).await;
        store.conn.call(|c| c.execute("UPDATE memory SET lang = 'es' WHERE id = 2", [])).await.unwrap();
        let exported = store
            .conn
            .call(|c| Ok::<_, rusqlite::Error>(read(c, &ExportFilter::default(), 0)))
            .await
            .unwrap()
            .unwrap();
        let lang = |id: i64| exported.memories.iter().find(|m| m.id == id).unwrap().lang.clone();
        assert_eq!((lang(2), lang(3)), (Some("es".to_string()), None), "a row not yet classified exports null");
        let (_, parsed) = parse(&exported.to_jsonl()).unwrap();
        assert_eq!(parsed.memories.into_iter().map(|(_, m)| m).collect::<Vec<_>>(), exported.memories);
    }

    #[tokio::test]
    async fn an_export_carries_the_origin_project() {
        let store = MemoryStore::new_for_test().await.unwrap();
        seed(&store).await;
        store
            .conn
            .call(|c| c.execute("UPDATE memory SET origin_project = 'notes/app' WHERE id = 2", []))
            .await
            .unwrap();
        let exported = store
            .conn
            .call(|c| Ok::<_, rusqlite::Error>(read(c, &ExportFilter::default(), 0)))
            .await
            .unwrap()
            .unwrap();
        let origin = |id: i64| exported.memories.iter().find(|m| m.id == id).unwrap().origin_project.clone();
        assert_eq!((origin(2), origin(3)), (Some("notes/app".to_string()), None));
        let text = exported.to_jsonl();
        let memory_lines: Vec<&str> = text.lines().filter(|l| l.contains("\"kind\":\"memory\"")).collect();
        assert!(!memory_lines.is_empty() && memory_lines.iter().all(|l| l.contains("\"origin_project\":")), "every line names it");
        let (_, parsed) = parse(&text).unwrap();
        assert_eq!(parsed.memories.into_iter().map(|(_, m)| m).collect::<Vec<_>>(), exported.memories);
    }

    #[tokio::test]
    async fn a_store_older_than_the_origin_column_exports_it_as_null() {
        let store = MemoryStore::new_for_test().await.unwrap();
        seed(&store).await;
        store
            .conn
            .call(|c| {
                c.execute_batch("DROP INDEX idx_memory_origin_project; ALTER TABLE memory DROP COLUMN origin_project;")
            })
            .await
            .unwrap();
        let exported = store
            .conn
            .call(|c| Ok::<_, rusqlite::Error>(read(c, &ExportFilter::default(), 0)))
            .await
            .unwrap()
            .unwrap();
        assert!(!exported.memories.is_empty() && exported.memories.iter().all(|m| m.origin_project.is_none()));
    }

    #[tokio::test]
    async fn links_leave_only_with_the_rows_that_hold_them() {
        let live = seeded_export(ExportFilter { project_id: Some(project()), ..Default::default() }).await;
        let pairs: Vec<(i64, i64)> = live.conflicts.iter().map(|c| (c.new_id, c.old_id)).collect();
        assert_eq!(pairs, vec![(4, 3), (13, 12)], "the pair with deleted row 11 stays out");
        assert_eq!(live.pieces.iter().map(|p| (p.save, p.piece, p.memory)).collect::<Vec<_>>(), vec![(4, 1, 5), (4, 2, 6)]);
        assert_eq!(live.pieces[1].context, "fn main()");
        assert_eq!(live.derivations.iter().map(|d| (d.memory, d.source)).collect::<Vec<_>>(), vec![(10, 2), (10, 3)]);

        let codex = seeded_export(ExportFilter { agent: Some("codex".into()), ..Default::default() }).await;
        assert!(codex.conflicts.is_empty() && codex.pieces.is_empty() && codex.derivations.is_empty());

        let (header, parsed) = parse(&live.to_jsonl()).unwrap();
        assert_eq!(header, live.header);
        assert!(parsed.invalid.is_empty(), "{:?}", parsed.invalid);
        assert_eq!(parsed.memories.into_iter().map(|(_, m)| m).collect::<Vec<_>>(), live.memories);
        assert_eq!(parsed.conflicts.len(), 2);
    }

    #[tokio::test]
    async fn a_store_from_v12_or_v16_exports_with_null_for_what_it_lacks() {
        for version in [12, 16] {
            let dir = tempfile::tempdir().unwrap();
            seed_on_disk(dir.path(), Some(version)).await;
            let exported = export(dir.path(), &ExportFilter::default(), 0).unwrap();
            assert_eq!(exported.memories.len(), 1, "V{version}");
            let row = &exported.memories[0];
            assert_eq!(row.content, "seeded on disk");
            assert_eq!((row.supersede_cause.clone(), row.revived_from, row.topic_key.clone()), (None, None, None));
            assert!(exported.conflicts.is_empty());
            assert_eq!(exported.outbox_waiting, 0);
        }
    }

    #[tokio::test]
    async fn a_store_from_before_the_warning_mark_exports_its_pairs_unmarked() {
        let dir = tempfile::tempdir().unwrap();
        seed_on_disk(dir.path(), Some(18)).await;
        let c = rusqlite::Connection::open(dir.path().join("memory.db")).unwrap();
        c.execute_batch(
            "INSERT INTO memory (scope, agent_id, content, memory_type, source, created_at, created_by_type, \
              created_by_ref, last_modified_at, last_accessed_at, importance, tier, status) \
             SELECT scope, agent_id, 'seeded again', memory_type, source, 1, created_by_type, created_by_ref, 1, 1, \
              importance, tier, status FROM memory;
             INSERT INTO memory_conflict (new_id, old_id, distance, state, created_at) VALUES (2, 1, 0.2, 'pending', 1);",
        )
        .unwrap();
        drop(c);

        let exported = export(dir.path(), &ExportFilter::default(), 0).unwrap();

        let pairs: Vec<(i64, i64, bool)> = exported.conflicts.iter().map(|p| (p.new_id, p.old_id, p.topic_key_warned)).collect();
        assert_eq!(pairs, vec![(2, 1, false)]);
    }

    #[tokio::test]
    async fn a_newer_store_is_not_exported() {
        let dir = tempfile::tempdir().unwrap();
        seed_on_disk(dir.path(), None).await;
        let c = rusqlite::Connection::open(dir.path().join("memory.db")).unwrap();
        c.execute(
            "INSERT INTO refinery_schema_history (version, name, applied_on, checksum) VALUES (999, 'future', '', '1')",
            [],
        )
        .unwrap();
        drop(c);
        assert!(matches!(export(dir.path(), &ExportFilter::default(), 0), Err(ExportError::Newer { db: 999, .. })));
    }

    #[test]
    fn a_missing_store_is_named() {
        let dir = tempfile::tempdir().unwrap();
        let err = export(dir.path(), &ExportFilter::default(), 0).unwrap_err();
        assert!(err.to_string().contains("no memory store"), "{err}");
    }
}
