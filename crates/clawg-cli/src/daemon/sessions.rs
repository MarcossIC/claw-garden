//! Which host session each agent is working in, per project, as the client's
//! hooks report it. Only the hooks know the session id: the bridge is started
//! once and the model is never asked for it. A save is stamped with the id
//! registered for its agent and project when the daemon accepts it.
//!
//! Kept in memory only: after a restart a save carries no session until the
//! next hook registers one, which beats carrying a stale one.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

/// The longest session id accepted, as for an `operation_id`.
pub const MAX_SESSION_ID_LEN: usize = 128;

/// Why a session id is refused.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error("session_id must be 1 to {MAX_SESSION_ID_LEN} visible ASCII characters")]
pub struct InvalidSessionId;

/// 1 to [`MAX_SESSION_ID_LEN`] visible ASCII characters, so it prints in the
/// viewer as it was sent.
pub fn check_session_id(id: &str) -> Result<(), InvalidSessionId> {
    if id.is_empty() || id.len() > MAX_SESSION_ID_LEN || !id.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(InvalidSessionId);
    }
    Ok(())
}

/// The tools a call is counted under: those of the bridge, by the name the
/// model calls them. A closed set, so what is stored of a call is one of
/// these and never the text a request carried.
pub const TOOLS: [&str; 13] = [
    "save_memory",
    "search_memory",
    "recall_context",
    "list_memories",
    "read_memory",
    "delete_memory",
    "restore_memory",
    "update_memory",
    "resolve_conflict",
    "answer_suggestion",
    "synthesize",
    "search_project",
    "memory_status",
];

/// `raw` as [`TOOLS`] names it, or `other`.
pub fn tool_label(raw: &str) -> &'static str {
    clawg_core::metrics::sessions::label(&TOOLS, raw)
}

/// The last session registered for each (agent, project). Two sessions of
/// one agent in one project cannot be told apart: the last to register wins,
/// and since every prompt registers, that is the one the user spoke to last.
#[derive(Debug, Default)]
pub struct SessionRegistry {
    current: Mutex<Sessions>,
}

/// Each session with the order it registered in, so that the entries of two
/// projects merged into one still tell which came last.
#[derive(Debug, Default)]
struct Sessions {
    by_key: HashMap<(String, Option<String>), (String, u64)>,
    registered: u64,
    /// The sessions already asked for their summary, by host session id.
    asked: HashSet<String>,
}

impl SessionRegistry {
    fn lock(&self) -> std::sync::MutexGuard<'_, Sessions> {
        self.current.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn register(&self, agent_id: &str, project_id: Option<&str>, session_id: &str) {
        let key = (agent_id.to_string(), project_id.map(str::to_string));
        let mut sessions = self.lock();
        sessions.registered += 1;
        let order = sessions.registered;
        sessions.by_key.insert(key, (session_id.to_string(), order));
    }

    pub fn current(&self, agent_id: &str, project_id: Option<&str>) -> Option<String> {
        let key = (agent_id.to_string(), project_id.map(str::to_string));
        self.lock().by_key.get(&key).map(|(session, _)| session.clone())
    }

    /// Whether `session_id` is asked for its summary now: true the first
    /// time, which marks it, and false from then on.
    pub fn ask_summary_once(&self, session_id: &str) -> bool {
        self.lock().asked.insert(session_id.to_string())
    }

    /// The sessions of `source`, merged into `target`, are `target`'s now: a
    /// bridge keeps the source's id, which leads to `target`. Where an agent
    /// has a session in both, the one registered last stays.
    pub fn merge_project(&self, source: &str, target: &str) {
        let mut sessions = self.lock();
        let moved: Vec<_> = sessions.by_key.keys().filter(|(_, p)| p.as_deref() == Some(source)).cloned().collect();
        for key in moved {
            let Some(entry) = sessions.by_key.remove(&key) else { continue };
            let into = (key.0, Some(target.to_string()));
            if sessions.by_key.get(&into).is_none_or(|(_, order)| *order < entry.1) {
                sessions.by_key.insert(into, entry);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_registry_keeps_the_latest_per_agent_and_project() {
        let registry = SessionRegistry::default();
        assert_eq!(registry.current("a", Some("p")), None);

        registry.register("a", Some("p"), "s1");
        registry.register("a", Some("p"), "s2");
        registry.register("a", None, "s3");

        assert_eq!(registry.current("a", Some("p")).as_deref(), Some("s2"));
        assert_eq!(registry.current("a", None).as_deref(), Some("s3"));
        assert_eq!(registry.current("b", Some("p")), None);
        assert_eq!(registry.current("a", Some("q")), None);
    }

    #[test]
    fn a_merge_moves_the_sessions_of_its_source_and_the_last_registered_stays() {
        let registry = SessionRegistry::default();
        registry.register("a", Some("target"), "a-older");
        registry.register("a", Some("source"), "a-newer");
        registry.register("b", Some("source"), "b-only");
        registry.register("c", Some("source"), "c-older");
        registry.register("c", Some("target"), "c-newer");
        registry.register("a", Some("other"), "a-other");

        registry.merge_project("source", "target");

        assert_eq!(registry.current("a", Some("target")).as_deref(), Some("a-newer"));
        assert_eq!(registry.current("b", Some("target")).as_deref(), Some("b-only"));
        assert_eq!(registry.current("c", Some("target")).as_deref(), Some("c-newer"));
        assert_eq!(registry.current("a", Some("other")).as_deref(), Some("a-other"));
        for agent in ["a", "b", "c"] {
            assert_eq!(registry.current(agent, Some("source")), None, "{agent}");
        }
    }

    #[test]
    fn a_session_is_asked_for_its_summary_once() {
        let registry = SessionRegistry::default();
        assert!(registry.ask_summary_once("s1"));
        assert!(!registry.ask_summary_once("s1"));
        assert!(registry.ask_summary_once("s2"), "each session once");
    }

    #[test]
    fn a_tool_outside_the_set_is_counted_as_other() {
        assert_eq!(tool_label("search_memory"), "search_memory");
        assert_eq!(tool_label("mcp__clawg__search_memory"), "other");
        assert_eq!(tool_label("where is the deploy script"), "other");
    }

    #[test]
    fn a_session_id_is_checked() {
        assert_eq!(check_session_id("2f1c9e9a-4c1b-4d7e-9f5a-0b1c2d3e4f50"), Ok(()));
        assert_eq!(check_session_id(&"x".repeat(MAX_SESSION_ID_LEN)), Ok(()));
        for bad in ["", "has space", "tab\t", "ñ"] {
            assert_eq!(check_session_id(bad), Err(InvalidSessionId), "{bad:?}");
        }
        assert_eq!(check_session_id(&"x".repeat(MAX_SESSION_ID_LEN + 1)), Err(InvalidSessionId));
    }
}
