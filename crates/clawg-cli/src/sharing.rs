//! `clawg sharing`: show or change which agents do not read which agents'
//! memories.
//!
//! Works on `<data>/sharing.json` alone, never the store, so it runs while the
//! daemon holds the store lock; the daemon reads the file on every search.

use std::path::Path;

use clawg_core::memory::sharing;

use crate::cli::SharingAction;

/// Applies `action` (none shows the policy) and returns the policy as it now
/// stands, one exclusion per line.
pub fn run(base_dir: &Path, action: Option<&SharingAction>) -> Result<String, String> {
    if let Some(SharingAction::Hide(e) | SharingAction::Unhide(e)) = action {
        e.check()?;
    }
    // An unreadable file is never overwritten: it may be a hand edit halfway
    // done, and replacing it would throw away every exclusion it holds.
    let mut policy = sharing::load(base_dir).map_err(|e| match action {
        None => e.to_string(),
        Some(_) => format!("{e}; nothing was changed"),
    })?;
    match action {
        None => return Ok(policy.render()),
        Some(SharingAction::Hide(e)) => policy.hide(&e.reader, &e.author),
        Some(SharingAction::Unhide(e)) => policy.unhide(&e.reader, &e.author),
    }
    sharing::save(base_dir, &policy).map_err(|e| e.to_string())?;
    Ok(policy.render())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Exclusion, ProfileArg};
    use clawg_core::paths::sharing_file;

    fn exclusion(reader: &str, author: &str) -> Exclusion {
        Exclusion { reader: reader.into(), author: author.into(), profile: ProfileArg::default() }
    }

    #[test]
    fn hide_and_unhide_print_the_policy_they_leave() {
        let dir = tempfile::tempdir().unwrap();
        let everything = "every agent reads every other agent";
        assert_eq!(run(dir.path(), None).unwrap(), everything);
        assert_eq!(run(dir.path(), Some(&SharingAction::Hide(exclusion("cursor", "claude-code")))).unwrap(), "cursor does not read claude-code");
        assert_eq!(run(dir.path(), None).unwrap(), "cursor does not read claude-code");
        assert_eq!(run(dir.path(), Some(&SharingAction::Unhide(exclusion("cursor", "claude-code")))).unwrap(), everything);
    }

    #[test]
    fn a_change_on_an_invalid_policy_leaves_it_alone() {
        let dir = tempfile::tempdir().unwrap();
        let hand_edit = r#"{"version":1,"hides":{"cursor":["claude-code",]}}"#;
        std::fs::write(sharing_file(dir.path()), hand_edit).unwrap();

        let shown = run(dir.path(), None).unwrap_err();
        assert!(shown.contains("sharing.json is not valid"), "{shown}");
        let refused = run(dir.path(), Some(&SharingAction::Hide(exclusion("codex", "cursor")))).unwrap_err();
        assert!(refused.ends_with("nothing was changed"), "{refused}");
        assert_eq!(std::fs::read_to_string(sharing_file(dir.path())).unwrap(), hand_edit);
    }

    #[test]
    fn hiding_an_agent_from_itself_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(run(dir.path(), Some(&SharingAction::Hide(exclusion("cursor", "cursor")))).is_err());
        assert!(!sharing_file(dir.path()).exists());
    }

    #[test]
    fn every_agent_on_both_sides_stops_all_sharing() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(run(dir.path(), Some(&SharingAction::Hide(exclusion("*", "*")))).unwrap(), "no agent reads another");
    }
}
