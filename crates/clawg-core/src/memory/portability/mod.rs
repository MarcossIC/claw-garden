//! Memories by row, out of a store and into another: the `clawg-memories`
//! format, the export that writes it and the import that reads it.

pub mod control;
pub mod engram;
pub mod export;
pub mod foreign;
pub mod format;
pub mod import;
pub mod markdown;
pub mod projects;
pub mod timing;
pub mod translate;
pub mod undo;

pub use control::{Counters, ImportControl, Progress, StopCause};
pub use export::{export, Export, ExportError, ExportFilter};
pub use import::{
    import, import_foreign, import_parsed, map_project, ImportError, ImportInput, ImportOptions, ImportReport, Imported, ImportedProject,
    ImportedSubfolder, ImportedSuggestion, MapProject, MapTarget, Stopped, MAX_IMPORT_BYTES,
};
pub use projects::By;
pub use timing::ImportTimings;
pub use undo::{undo_import, KeepReason, KeptEntry, KeptRow, RegistryKeep, RegistryUndo, UndoError, UndoReport};

#[cfg(test)]
pub(crate) mod fixtures {
    use std::path::Path;

    use crate::memory::identity::derive_project_id;
    use crate::memory::MemoryStore;

    /// The project most rows belong to, and its root.
    pub(crate) const ROOT: &str = "/work/repo";
    pub(crate) const OTHER_ROOT: &str = "/work/other";

    pub(crate) fn project() -> String {
        derive_project_id(Path::new(ROOT))
    }

    pub(crate) fn other_project() -> String {
        derive_project_id(Path::new(OTHER_ROOT))
    }

    /// One row of the seeded store: id, scope, agent, project, type, source,
    /// status, superseded_by, supersede_cause, deleted_at, topic_key,
    /// revived_from, disputed_against, content.
    type Row = (
        i64,
        &'static str,
        Option<&'static str>,
        Option<String>,
        &'static str,
        &'static str,
        &'static str,
        Option<i64>,
        Option<&'static str>,
        Option<i64>,
        Option<&'static str>,
        Option<i64>,
        Option<&'static str>,
        &'static str,
    );

    /// A store with every type, a save in two pieces, a conclusion, a chain
    /// replaced by a revision and one by a verdict, a revived row, an
    /// escalated pair, a judged one, deleted rows, a project-scope row and a
    /// second agent in a second project. Rows carry their own access counts,
    /// tiers and dates, which the store would never derive from an insert.
    pub(crate) async fn seed(store: &MemoryStore) {
        let p = Some(project());
        let q = Some(other_project());
        let a = Some("claude-code");
        let rows: Vec<Row> = vec![
            (1, "agent", a, p.clone(), "semantic_fact", "explicit", "superseded", Some(2), Some("revision"), None, Some("storage/engine"), None, None, "Storage engine: Postgres"),
            (2, "agent", a, p.clone(), "semantic_fact", "explicit", "active", None, None, None, Some("storage/engine"), None, None, "Storage engine: SQLite with WAL"),
            (3, "agent", a, p.clone(), "user_preference", "explicit", "active", None, None, None, None, None, None, "Prefer tabs over spaces in Makefiles"),
            (4, "agent", a, p.clone(), "architectural_decision", "explicit", "disputed", None, None, None, None, None, Some("[3]"), "Indent Makefiles with spaces"),
            (5, "agent", a, p.clone(), "code_block", "explicit", "active", None, None, None, None, None, None, "fn main() { first half of a long save }"),
            (6, "agent", a, p.clone(), "code_block", "explicit", "active", None, None, None, None, None, None, "second half of a long save }"),
            (7, "agent", a, None, "session_summary", "explicit", "active", None, None, None, None, None, None, "Session: wired the exporter"),
            (8, "agent", a, None, "chat_message", "chat", "active", None, None, None, None, None, None, "hello there"),
            (9, "agent", a, None, "tool_error", "chat", "active", None, None, None, None, None, None, "cargo build failed: linker not found"),
            (10, "agent", a, p.clone(), "semantic_fact", "synthesis", "active", None, None, None, None, None, None, "The project favours embedded stores"),
            (11, "agent", a, p.clone(), "semantic_fact", "explicit", "active", None, None, Some(1_756_000_000_000), None, None, None, "Daemon port: 27390"),
            (12, "agent", a, p.clone(), "semantic_fact", "explicit", "superseded", Some(13), Some("verdict"), None, None, None, None, "Daemon port: 27381"),
            (13, "agent", a, p.clone(), "semantic_fact", "explicit", "active", None, None, None, None, None, None, "Daemon port: 27380"),
            (14, "agent", Some("codex"), q, "semantic_fact", "explicit", "active", None, None, None, None, None, None, "Codex keeps notes in English"),
            (15, "agent", a, p.clone(), "semantic_fact", "explicit", "active", None, None, None, Some("cache/policy"), Some(16), None, "Cache policy: none"),
            (16, "agent", a, p.clone(), "semantic_fact", "explicit", "superseded", Some(15), Some("revision"), Some(1_757_000_000_000), Some("cache/policy"), None, None, "Cache policy: redis"),
            (17, "project", None, p.clone(), "semantic_fact", "explicit", "active", None, None, None, None, None, None, "Every crate inherits the workspace version"),
        ];
        let (p, q) = (project(), other_project());
        store
            .conn
            .call(move |c| {
                let tx = c.transaction()?;
                for (id, scope, agent, project, ty, source, status, by, cause, deleted, key, revived, against, content) in rows {
                    let created = 1_750_000_000_000 + id * 1000;
                    tx.execute(
                        "INSERT INTO memory (id, scope, agent_id, project_id, content, memory_type, source, \
                         created_at, created_by_type, created_by_ref, source_session_id, last_modified_at, last_modified_by, \
                         update_count, last_accessed_at, access_count, importance, tier, superseded_by, supersede_cause, \
                         status, dispute_count, disputed_against, last_disputed_at, topic_key, revived_from, content_hash) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'agent', 'claude-code', 'session-9', ?8 + 5, 'claude-code', \
                         2, ?8 + 7, ?1 + 2, 6.5, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, content_hash(?5))",
                        rusqlite::params![
                            id, scope, agent, project, content, ty, source, created, if id % 2 == 0 { 1 } else { 2 },
                            by, cause, status, i64::from(against.is_some()), against,
                            against.map(|_| created), key, revived,
                        ],
                    )?;
                    if let Some(deleted) = deleted {
                        tx.execute("UPDATE memory SET deleted_at = ?2 WHERE id = ?1", rusqlite::params![id, deleted])?;
                    }
                }
                tx.execute(
                    "INSERT INTO project_path (id, root_path, created_at, index_docs) VALUES (?1, ?2, 1, 1), (?3, ?4, 1, 0)",
                    rusqlite::params![p, ROOT, q, OTHER_ROOT],
                )?;
                tx.execute_batch(
                    "INSERT INTO memory_piece (save_id, piece, memory_id, context) VALUES (4, 1, 5, ''), (4, 2, 6, 'fn main()');
                     INSERT INTO memory_derivation (memory_id, source_id, created_at) VALUES (10, 2, 1750000010000), (10, 3, 1750000010000);
                     INSERT INTO memory_conflict (new_id, old_id, distance, state, verdict, reason, created_at, judged_at)
                       VALUES (4, 3, 0.25, 'escalated', NULL, NULL, 1750000004000, NULL),
                              (13, 12, 0.2, 'judged', 'supersedes', 'the port moved', 1750000013000, 1750000014000),
                              (13, 11, 0.3, 'pending', NULL, NULL, 1750000013000, NULL);
                     INSERT INTO ingest_outbox (operation_id, agent_id, content, event, created_at, updated_at)
                       VALUES ('op-1', 'claude-code', 'waiting', '{}', 1, 1);",
                )?;
                tx.commit()
            })
            .await
            .unwrap();
    }
}
