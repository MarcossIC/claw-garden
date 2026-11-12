//! Who stated a memory: how a save is sealed, and what a reader may say about it.
//!
//! Rows saved before the seal existed read `explicit` / `user` whether or not
//! the user said them, and a save the agent declares as the user's reads the
//! same. So a reader never claims a row is the user's: it marks only what was
//! never ambiguous, an inference, a conclusion or an import, and a row without
//! a mark may be the user's.

use crate::memory::domain::events::MemorySource;
use crate::memory::domain::record::CreatedByType;
use crate::models::chat::Role;

/// The name an import of Claude Code's native memory goes by: the source of
/// its notes, and what the `created_by_ref` of its rows starts with.
pub const CLAUDE_CODE: &str = "claude-code";

/// The `source` and `created_by_type` a row of this event is stored with.
/// `has_sources` is whether a deliberate save names the rows it was drawn from.
pub(crate) fn seal(source: MemorySource, role: &Role, has_sources: bool, user_stated: bool) -> (MemorySource, CreatedByType) {
    match source {
        // A conclusion generalizes several memories: the agent drew it, even
        // from what the user said.
        MemorySource::Explicit if has_sources => (MemorySource::Synthesis, CreatedByType::Agent),
        MemorySource::Explicit if user_stated => (MemorySource::Explicit, CreatedByType::User),
        // That the agent chose to save it is all the store knows.
        MemorySource::Explicit => (MemorySource::Explicit, CreatedByType::Agent),
        MemorySource::Synthesis => (MemorySource::Synthesis, CreatedByType::Agent),
        MemorySource::Compaction => (MemorySource::Compaction, CreatedByType::System),
        MemorySource::Chat | MemorySource::CodeFile => (
            source,
            match role {
                Role::User => CreatedByType::User,
                Role::Assistant => CreatedByType::Agent,
                Role::Tool | Role::System => CreatedByType::System,
            },
        ),
    }
}

/// Whether this is the seal [`seal`] gives a save the user stated.
pub(crate) fn is_stated(source: MemorySource, created_by_type: CreatedByType) -> bool {
    source == MemorySource::Explicit && created_by_type == CreatedByType::User
}

/// What a reader shows of a stored row's origin, if anything.
pub fn mark(source: &str, created_by_type: &str) -> Option<&'static str> {
    match (source, created_by_type) {
        ("synthesis", _) => Some("conclusion"),
        ("import", _) => Some("imported"),
        ("explicit", "agent") => Some("inferred"),
        _ => None,
    }
}

/// What a hit shows of its origin: the mark of a preference, since the
/// workspace prints only the ones the user stated and the model reads a
/// preference as a rule of the person. Every other row shows none.
pub(crate) fn preference_mark(memory_type: &str, source: Option<&str>, created_by_type: Option<&str>) -> Option<&'static str> {
    if memory_type != "user_preference" {
        return None;
    }
    mark(source?, created_by_type?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::upkeep::admin::get_memory_core;
    use crate::memory::store::db::MemoryStore;

    #[test]
    fn seal_cases() {
        use CreatedByType::{Agent, System, User};
        use MemorySource::{Chat, Compaction, Explicit, Synthesis};
        let cases = [
            (Explicit, Role::User, true, false, Synthesis, Agent),
            (Explicit, Role::User, true, true, Synthesis, Agent),
            (Explicit, Role::User, false, true, Explicit, User),
            (Explicit, Role::User, false, false, Explicit, Agent),
            (Compaction, Role::User, false, true, Compaction, System),
            (Chat, Role::User, false, false, Chat, User),
            (Chat, Role::Assistant, false, false, Chat, Agent),
            (Chat, Role::Tool, false, false, Chat, System),
        ];
        for (source, role, has_sources, user_stated, want_source, want_by) in cases {
            assert_eq!(
                seal(source, &role, has_sources, user_stated),
                (want_source, want_by),
                "{source:?} {role:?} sources={has_sources} user_stated={user_stated}"
            );
        }
    }

    #[test]
    fn mark_cases() {
        let cases = [
            ("synthesis", "agent", Some("conclusion")),
            ("synthesis", "user", Some("conclusion")),
            ("import", "user", Some("imported")),
            ("explicit", "agent", Some("inferred")),
            ("explicit", "user", None),
            ("chat", "agent", None),
            ("chat", "user", None),
            ("compaction", "system", None),
            ("", "", None),
        ];
        for (source, by, want) in cases {
            assert_eq!(mark(source, by), want, "{source}/{by}");
        }
    }

    #[tokio::test]
    async fn rows_saved_before_provenance_keep_their_origin_and_no_mark() {
        let store = MemoryStore::new_for_test().await.unwrap();
        // What every save by the previous version wrote, the user's own or not.
        let id = store
            .conn
            .call(|c| -> rusqlite::Result<i64> {
                c.execute(
                    "INSERT INTO memory (scope, agent_id, content, memory_type, source, created_at, \
                     created_by_type, created_by_ref, last_modified_at, last_accessed_at, importance, tier) \
                     VALUES ('agent', 'agent-p', 'the API listens on 8080', 'semantic_fact', 'explicit', \
                     1000, 'user', 'agent-p', 1000, 1000, 5.0, 3)",
                    [],
                )?;
                Ok(c.last_insert_rowid())
            })
            .await
            .unwrap();

        let row = get_memory_core(&store, id).await.unwrap().expect("the row opens");
        let (source, by) = (row.source.unwrap(), row.created_by_type.unwrap());
        assert_eq!((source.as_str(), by.as_str()), ("explicit", "user"));
        assert_eq!(mark(&source, &by), None);
    }
}
