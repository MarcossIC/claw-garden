//! Which host session each agent is working in, per project, as the client's
//! hooks report it. Only the hooks know the session id: the bridge is started
//! once and the model is never asked for it. A save is stamped with the id
//! registered for its agent and project when the daemon accepts it.
//!
//! Kept in memory only: after a restart a save carries no session until the
//! next hook registers one, which beats carrying a stale one.

use std::collections::HashMap;
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

/// The last session registered for each (agent, project). Two sessions of
/// one agent in one project cannot be told apart: the last to register wins,
/// and since every prompt registers, that is the one the user spoke to last.
#[derive(Debug, Default)]
pub struct SessionRegistry {
    current: Mutex<HashMap<(String, Option<String>), String>>,
}

impl SessionRegistry {
    pub fn register(&self, agent_id: &str, project_id: Option<&str>, session_id: &str) {
        let key = (agent_id.to_string(), project_id.map(str::to_string));
        self.current.lock().unwrap_or_else(|e| e.into_inner()).insert(key, session_id.to_string());
    }

    pub fn current(&self, agent_id: &str, project_id: Option<&str>) -> Option<String> {
        let key = (agent_id.to_string(), project_id.map(str::to_string));
        self.current.lock().unwrap_or_else(|e| e.into_inner()).get(&key).cloned()
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
    fn a_session_id_is_checked() {
        assert_eq!(check_session_id("2f1c9e9a-4c1b-4d7e-9f5a-0b1c2d3e4f50"), Ok(()));
        assert_eq!(check_session_id(&"x".repeat(MAX_SESSION_ID_LEN)), Ok(()));
        for bad in ["", "has space", "tab\t", "ñ"] {
            assert_eq!(check_session_id(bad), Err(InvalidSessionId), "{bad:?}");
        }
        assert_eq!(check_session_id(&"x".repeat(MAX_SESSION_ID_LEN + 1)), Err(InvalidSessionId));
    }
}
