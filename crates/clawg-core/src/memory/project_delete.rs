//! A deleted project comes back when a live row is written under its id.

#[cfg(test)]
mod tests {
    use crate::memory::MemoryStore;

    const GONE: &str = "app-aaaa";

    async fn store_with_a_deleted_project() -> MemoryStore {
        let store = MemoryStore::new_for_test().await.unwrap();
        run(
            &store,
            &format!(
                "INSERT INTO project (id, name, born, reviewed, created_at, deleted_at) \
                   VALUES ('{GONE}', 'app', 'root', 1, 1, 5), ('other-bbbb', 'other', 'root', 1, 1, NULL);"
            ),
        )
        .await;
        store
    }

    async fn run(store: &MemoryStore, batch: &str) {
        let batch = batch.to_string();
        store.conn.call(move |c| c.execute_batch(&batch)).await.unwrap();
    }

    fn memory(id: i64, project: &str, deleted_at: &str) -> String {
        format!(
            "INSERT INTO memory \
             (id, scope, agent_id, project_id, content, memory_type, source, created_at, created_by_type, \
              created_by_ref, last_modified_at, last_accessed_at, importance, tier, status, deleted_at) \
             VALUES ({id}, 'agent', 'a', '{project}', 'note {id}', 'semantic_fact', 'explicit', {id}, 'agent', \
              'x', {id}, {id}, 5.0, 3, 'active', {deleted_at});"
        )
    }

    /// `(deleted_at, reviewed)` of a project.
    async fn state(store: &MemoryStore, id: &str) -> (Option<i64>, i64) {
        let id = id.to_string();
        store
            .conn
            .call(move |c| {
                c.query_row("SELECT deleted_at, reviewed FROM project WHERE id = ?1", [&id], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn a_live_row_brings_a_deleted_project_back_unreviewed() {
        // A save, an import or a worker replay: a live row inserted under the id.
        let store = store_with_a_deleted_project().await;
        run(&store, &memory(1, GONE, "NULL")).await;
        assert_eq!(state(&store, GONE).await, (None, 0), "an insert");

        // A move into the project: a live row changes project.
        let store = store_with_a_deleted_project().await;
        run(&store, &memory(2, "other-bbbb", "NULL")).await;
        assert_eq!(state(&store, GONE).await, (Some(5), 1), "nothing wrote under it yet");
        run(&store, &format!("UPDATE memory SET project_id = '{GONE}' WHERE id = 2;")).await;
        assert_eq!(state(&store, GONE).await, (None, 0), "a change of project");

        // A restore: a deleted row of the project becomes live.
        let store = store_with_a_deleted_project().await;
        run(&store, &memory(3, GONE, "9")).await;
        assert_eq!(state(&store, GONE).await, (Some(5), 1), "a deleted row insert leaves it");
        run(&store, "UPDATE memory SET deleted_at = NULL WHERE id = 3;").await;
        assert_eq!(state(&store, GONE).await, (None, 0), "a restore");
    }

    #[tokio::test]
    async fn a_deleted_row_or_a_registration_leaves_it_deleted() {
        let store = store_with_a_deleted_project().await;
        run(&store, &memory(1, GONE, "9")).await;
        run(&store, &memory(2, "other-bbbb", "9")).await;
        run(&store, &format!("UPDATE memory SET project_id = '{GONE}' WHERE id = 2;")).await;
        assert_eq!(state(&store, GONE).await, (Some(5), 1), "deleted rows");

        run(
            &store,
            &format!(
                "INSERT OR IGNORE INTO project (id, name, born, reviewed, created_at) VALUES ('{GONE}', 'app', 'root', 0, 7);
                 INSERT INTO project_path (id, root_path, created_at, index_docs, project_id) \
                   VALUES ('{GONE}', '/work/app', 7, 0, '{GONE}');"
            ),
        )
        .await;
        assert_eq!(state(&store, GONE).await, (Some(5), 1), "a registration");

        run(&store, &format!("UPDATE memory SET importance = 6.0 WHERE id = 1; {}", memory(3, "other-bbbb", "NULL"))).await;
        assert_eq!(state(&store, GONE).await, (Some(5), 1), "another project's live row");
        assert_eq!(state(&store, "other-bbbb").await, (None, 1), "a live project keeps its review");
    }
}
