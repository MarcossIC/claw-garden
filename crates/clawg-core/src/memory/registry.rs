//! The projects of the store. `project` is the reference every table keys
//! its rows by, whether a folder of this machine leads to it
//! (`project_path`), a remote does (`project_remote`), or nothing does.
//!
//! A row of `memory` written under an id `project` does not know adopts it in
//! the same write, through the two triggers of V27, which name the project as
//! [`project_name`] does.

use crate::memory::project::is_project_id;

/// The name a project has until someone gives it one: its id without the
/// `-` and four hex digits `derive_project_id` appends, or the whole id when
/// it does not have that shape. V27's backfill and triggers say the same in
/// SQL.
pub(crate) fn project_name(id: &str) -> &str {
    match id.rsplit_once('-') {
        Some((name, _)) if is_project_id(id) => name,
        _ => id,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::memory::events::{MemoryEvent, MemorySource};
    use crate::memory::relocate::{move_memories_core, undo_move_core, MoveFilter};
    use crate::memory::worker::save_for_test;
    use crate::memory::MemoryStore;
    use crate::models::chat::Role;

    /// `(name, born, reviewed, created_at)` of project `id`.
    type Project = (String, String, i64, i64);

    async fn project(store: &MemoryStore, id: &str) -> Option<Project> {
        use rusqlite::OptionalExtension;
        let id = id.to_string();
        store
            .conn
            .call(move |c| {
                c.query_row("SELECT name, born, reviewed, created_at FROM project WHERE id = ?1", [id], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                })
                .optional()
            })
            .await
            .unwrap()
    }

    async fn row(store: &MemoryStore, id: i64) -> (Option<String>, i64) {
        store
            .conn
            .call(move |c| c.query_row("SELECT project_id, created_at FROM memory WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?))))
            .await
            .unwrap()
    }

    fn fact(content: &str, project: Option<&str>) -> MemoryEvent {
        MemoryEvent {
            agent_id: "agent-a".into(),
            project_id: project.map(str::to_string),
            role: Role::User,
            content: content.into(),
            is_error: None,
            source: MemorySource::Explicit,
            timestamp_ms: 1,
            memory_type: Some("semantic_fact".into()),
            importance: None,
            derived_from: Vec::new(),
            topic_key: None,
            user_stated: false,
            session_id: None,
        }
    }

    async fn save(store: &Arc<MemoryStore>, evt: MemoryEvent) -> i64 {
        let report = save_for_test(store, &evt).await;
        assert_eq!(report.failure, None, "a row under an unknown project is not refused");
        assert_eq!(report.memories.len(), 1);
        report.memories[0].id
    }

    #[tokio::test]
    async fn a_row_saved_under_an_unknown_project_adopts_it() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());

        let id = save(&store, fact("a note from a bridge whose registration failed", Some("app-1a2b"))).await;

        let (project_id, created_at) = row(&store, id).await;
        assert_eq!(project_id.as_deref(), Some("app-1a2b"));
        assert_eq!(project(&store, "app-1a2b").await, Some(("app".into(), "adopted".into(), 0, created_at)));
    }

    #[tokio::test]
    async fn a_row_moved_to_an_unknown_project_adopts_it() {
        let store = Arc::new(MemoryStore::new_for_test().await.unwrap());
        save(&store, fact("a note that makes the target a project", Some("target-2b3c"))).await;
        let id = save(&store, fact("a note of an id from before projects", Some("old-1a2b"))).await;
        let backups = tempfile::tempdir().unwrap();
        let filter = MoveFilter { ids: vec![id], ..MoveFilter::default() };
        let moved = move_memories_core(&store, backups.path(), &filter, Some("target-2b3c".into()), false).await.unwrap();
        // As an id a store from before V27 only names in its history.
        store.conn.call(|c| c.execute("DELETE FROM project WHERE id = 'old-1a2b'", [])).await.unwrap();

        undo_move_core(&store, backups.path(), moved.move_id.expect("a move"), false).await.unwrap();

        let (project_id, created_at) = row(&store, id).await;
        assert_eq!(project_id.as_deref(), Some("old-1a2b"));
        assert_eq!(project(&store, "old-1a2b").await, Some(("old".into(), "adopted".into(), 0, created_at)));
    }

    #[tokio::test]
    async fn the_name_of_an_adopted_id_is_the_one_rust_gives() {
        assert_eq!(project_name("claw-garden-00ff"), "claw-garden");
        assert_eq!(project_name("release-2024"), "release");
        assert_eq!(project_name("repo-1A2B"), "repo-1A2B");
        assert_eq!(project_name("p"), "p");
        let ids = [
            "repo-1a2b", "claw-garden-00ff", "release-2024", "a--1a2b", "ñandú-0f0f", "repo", "p", "-1a2b", "repo-1A2B",
            "repo-1a2", "repo-1a2bc", "repo-zzzz", "repo 1a2b", "repo-1a2b-",
        ];
        let store = MemoryStore::new_for_test().await.unwrap();
        let named: Vec<(String, String)> = store
            .conn
            .call(move |c| {
                // Every other id arrives by a change of project, the other trigger.
                for (n, id) in ids.iter().enumerate() {
                    let inserted = if n % 2 == 0 { Some(*id) } else { None };
                    c.execute(
                        "INSERT INTO memory \
                         (scope, agent_id, project_id, content, memory_type, source, \
                          created_at, created_by_type, created_by_ref, \
                          last_modified_at, last_accessed_at, importance, tier, status) \
                         VALUES ('agent', 'a', ?1, 'note ' || ?2, 'semantic_fact', 'explicit', 1, 'agent', 'a', \
                          1, 1, 5.0, 3, 'active')",
                        rusqlite::params![inserted, n],
                    )?;
                    if inserted.is_none() {
                        c.execute("UPDATE memory SET project_id = ?1 WHERE id = ?2", rusqlite::params![id, c.last_insert_rowid()])?;
                    }
                }
                let mut stmt = c.prepare("SELECT id, name FROM project")?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
                rows.collect::<rusqlite::Result<_>>()
            })
            .await
            .unwrap();

        assert_eq!(named.len(), ids.len());
        for (id, name) in named {
            assert_eq!(name, project_name(&id), "{id}");
        }
    }
}
