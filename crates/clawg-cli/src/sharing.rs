//! `clawg sharing`: show or change which agents read which agents' memories.
//!
//! Works on `<data>/sharing.json` alone, never the store, so it runs while the
//! daemon holds the store lock; the daemon reads the file on every search.

use std::path::Path;

use clawg_core::memory::sharing;

use crate::cli::SharingAction;

/// Applies `action` (none shows the policy) and returns the policy as it now
/// stands, one grant per line.
pub fn run(base_dir: &Path, action: Option<&SharingAction>) -> Result<String, String> {
    if let Some(SharingAction::Grant(g) | SharingAction::Revoke(g)) = action {
        g.check()?;
    }
    // An unreadable file is never overwritten: it may be a hand edit halfway
    // done, and replacing it would throw away every grant it holds.
    let mut policy = sharing::load(base_dir).map_err(|e| match action {
        None => e.to_string(),
        Some(_) => format!("{e}; nothing was changed"),
    })?;
    match action {
        None => return Ok(policy.render()),
        Some(SharingAction::Grant(g)) => policy.grant(&g.reader, &g.author),
        Some(SharingAction::Revoke(g)) => policy.revoke(&g.reader, &g.author),
    }
    sharing::save(base_dir, &policy).map_err(|e| e.to_string())?;
    Ok(policy.render())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Grant, ProfileArg};
    use clawg_core::paths::sharing_file;

    fn grant(reader: &str, author: &str) -> Grant {
        Grant { reader: reader.into(), author: author.into(), profile: ProfileArg::default() }
    }

    #[test]
    fn grant_and_revoke_print_the_policy_they_leave() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(run(dir.path(), None).unwrap(), "nothing is shared");
        assert_eq!(run(dir.path(), Some(&SharingAction::Grant(grant("cursor", "claude-code")))).unwrap(), "cursor reads claude-code");
        assert_eq!(run(dir.path(), None).unwrap(), "cursor reads claude-code");
        assert_eq!(run(dir.path(), Some(&SharingAction::Revoke(grant("cursor", "claude-code")))).unwrap(), "nothing is shared");
    }

    #[test]
    fn a_change_on_an_invalid_policy_leaves_it_alone() {
        let dir = tempfile::tempdir().unwrap();
        let hand_edit = r#"{"version":1,"reads":{"cursor":["claude-code",]}}"#;
        std::fs::write(sharing_file(dir.path()), hand_edit).unwrap();

        let shown = run(dir.path(), None).unwrap_err();
        assert!(shown.contains("sharing.json is not valid"), "{shown}");
        let refused = run(dir.path(), Some(&SharingAction::Grant(grant("codex", "cursor")))).unwrap_err();
        assert!(refused.ends_with("nothing was changed"), "{refused}");
        assert_eq!(std::fs::read_to_string(sharing_file(dir.path())).unwrap(), hand_edit);
    }

    #[test]
    fn a_self_grant_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(run(dir.path(), Some(&SharingAction::Grant(grant("cursor", "cursor")))).is_err());
        assert!(!sharing_file(dir.path()).exists());
    }
}
