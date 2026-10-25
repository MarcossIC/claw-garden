//! Merging one project into another. The project merged leaves a tombstone in
//! `project` (`merged_into`), since open bridges keep its id for their whole
//! session and old exports carry it, and every entry that takes a project
//! goes through [`canonical`] to reach the live one.

use std::collections::HashSet;

use rusqlite::{Connection, OptionalExtension};

use crate::memory::{MemoryError, MemoryStore};

/// How far [`canonical_in`] follows `merged_into`. A merge points every
/// tombstone at a live project, so a store it wrote needs one hop; the bound
/// is for one edited by hand or restored broken.
pub const MAX_MERGE_HOPS: usize = 8;

/// The live project `id` leads to: the one a tombstone was merged into, or
/// `id` itself for a live project or an id `project` does not know. A cycle,
/// a chain longer than [`MAX_MERGE_HOPS`] or one that ends in no project
/// answers `id` itself, logged, and `doctor` names it.
pub(crate) fn canonical_in(c: &Connection, id: &str) -> rusqlite::Result<String> {
    let mut stmt = c.prepare_cached("SELECT merged_into FROM project WHERE id = ?1")?;
    let mut at = id.to_string();
    let mut seen = HashSet::from([at.clone()]);
    for hops in 0..=MAX_MERGE_HOPS {
        let next = match stmt.query_row([&at], |r| r.get::<_, Option<String>>(0)).optional()? {
            Some(None) => return Ok(at),
            None if hops == 0 => return Ok(at),
            None => {
                log::warn!("project {id} was merged into {at}, which is no project");
                return Ok(id.to_string());
            }
            Some(Some(next)) => next,
        };
        if !seen.insert(next.clone()) {
            log::warn!("the merges of project {id} go round in a cycle through {next}");
            return Ok(id.to_string());
        }
        at = next;
    }
    log::warn!("project {id} is more than {MAX_MERGE_HOPS} merges away from a live project");
    Ok(id.to_string())
}

/// [`canonical_in`] on the store's connection.
pub async fn canonical(store: &MemoryStore, id: &str) -> Result<String, MemoryError> {
    let id = id.to_string();
    store.conn.call(move |c| canonical_in(c, &id)).await.map_err(MemoryError::Connection)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::MemoryStore;

    async fn store_with(projects: &[(&'static str, Option<&'static str>)]) -> MemoryStore {
        let store = MemoryStore::new_for_test().await.unwrap();
        let projects: Vec<_> = projects.to_vec();
        store
            .conn
            .call(move |c| {
                for (id, merged_into) in &projects {
                    c.execute(
                        "INSERT INTO project (id, name, born, reviewed, created_at, merged_into, merged_at) \
                         VALUES (?1, ?1, 'root', 0, 1, ?2, CASE WHEN ?2 IS NULL THEN NULL ELSE 2 END)",
                        rusqlite::params![id, merged_into],
                    )?;
                }
                Ok::<_, rusqlite::Error>(())
            })
            .await
            .unwrap();
        store
    }

    #[tokio::test]
    async fn canonical_follows_a_merge_and_leaves_any_other_id_as_it_is() {
        let store = store_with(&[("app-1a2b", Some("web-3c4d")), ("web-3c4d", None)]).await;
        assert_eq!(canonical(&store, "app-1a2b").await.unwrap(), "web-3c4d", "a tombstone leads to its project");
        assert_eq!(canonical(&store, "web-3c4d").await.unwrap(), "web-3c4d", "a live project is itself");
        assert_eq!(canonical(&store, "nowhere-9f9f").await.unwrap(), "nowhere-9f9f", "an unknown id is itself");
    }

    #[tokio::test]
    async fn a_cycle_or_a_chain_too_long_answers_the_id_itself() {
        let store = store_with(&[("a", Some("b")), ("b", Some("a")), ("lost", Some("gone"))]).await;
        assert_eq!(canonical(&store, "a").await.unwrap(), "a", "a cycle");
        assert_eq!(canonical(&store, "lost").await.unwrap(), "lost", "a chain that ends in no project");

        let names: Vec<String> = (0..=MAX_MERGE_HOPS + 1).map(|n| format!("p{n}")).collect();
        let starts: Vec<String> = names[..2].to_vec();
        let store = MemoryStore::new_for_test().await.unwrap();
        store
            .conn
            .call(move |c| {
                for (n, id) in names.iter().enumerate() {
                    c.execute(
                        "INSERT INTO project (id, name, born, reviewed, created_at, merged_into) VALUES (?1, ?1, 'root', 0, 1, ?2)",
                        rusqlite::params![id, names.get(n + 1)],
                    )?;
                }
                Ok::<_, rusqlite::Error>(())
            })
            .await
            .unwrap();
        assert_eq!(
            canonical(&store, &starts[1]).await.unwrap(),
            format!("p{}", MAX_MERGE_HOPS + 1),
            "a chain of MAX_MERGE_HOPS merges still ends in its project"
        );
        assert_eq!(canonical(&store, &starts[0]).await.unwrap(), "p0", "one merge more is too long");
    }
}
