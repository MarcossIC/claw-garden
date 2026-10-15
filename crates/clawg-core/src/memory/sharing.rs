//! Which agents may read which agents' memories.
//!
//! The policy is a set of directed grants — "cursor reads claude-code" — kept
//! in `<data>/sharing.json`. Without the file nothing is shared, so an upgrade
//! never changes what an agent sees. Parsing and resolving are pure; only
//! [`load`] and [`save`] touch the filesystem.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::paths::sharing_file;

/// The only file format this build reads and writes.
const VERSION: u32 = 1;

/// Why `sharing.json` could not be used. Every caller treats it as "nothing is
/// shared"; the message is what the user is shown so they can fix the file.
#[derive(Debug, thiserror::Error)]
pub enum SharingError {
    #[error("cannot read sharing.json: {0}")]
    Io(#[from] std::io::Error),
    #[error("cannot write sharing.json: {0}")]
    Write(std::io::Error),
    #[error("sharing.json is not valid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("sharing.json has version {0}; this clawg reads version {VERSION}")]
    Version(u32),
    #[error("sharing.json names a blank agent")]
    BlankName,
}

/// The file as written. `deny_unknown_fields` because a misspelled key
/// (`"read"`) would otherwise parse as an empty policy, and a user who meant to
/// grant something would be told nothing about why it does not work.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct OnDisk {
    version: u32,
    #[serde(default)]
    reads: BTreeMap<String, Vec<String>>,
}

/// Reader to the authors it may read. Ordered maps so the same policy always
/// renders and saves the same way.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SharingPolicy {
    reads: BTreeMap<String, BTreeSet<String>>,
}

impl SharingPolicy {
    /// Parses the file's text. Names are kept exactly as written, the way
    /// `agent_id` is stored, so `Cursor` and `cursor` stay two agents.
    pub fn parse(text: &str) -> Result<Self, SharingError> {
        let on_disk: OnDisk = serde_json::from_str(text)?;
        if on_disk.version != VERSION {
            return Err(SharingError::Version(on_disk.version));
        }
        let mut policy = Self::default();
        for (reader, authors) in on_disk.reads {
            if reader.trim().is_empty() {
                return Err(SharingError::BlankName);
            }
            for author in authors {
                if author.trim().is_empty() {
                    return Err(SharingError::BlankName);
                }
                policy.grant(&reader, &author);
            }
        }
        Ok(policy)
    }

    pub fn to_json(&self) -> String {
        let on_disk = OnDisk {
            version: VERSION,
            reads: self.reads.iter().map(|(r, a)| (r.clone(), a.iter().cloned().collect())).collect(),
        };
        // A map of strings to string lists always serializes.
        let mut json = serde_json::to_string_pretty(&on_disk).unwrap_or_default();
        json.push('\n');
        json
    }

    /// The other agents whose memories `reader` may read, in order. An agent
    /// always reads its own, so it is never in the list.
    pub fn authors_for(&self, reader: &str) -> Vec<String> {
        self.reads.get(reader).map(|a| a.iter().cloned().collect()).unwrap_or_default()
    }

    /// Lets `reader` read `author`. A grant to oneself means nothing and is
    /// dropped; granting twice is the same as once.
    pub fn grant(&mut self, reader: &str, author: &str) {
        if reader != author {
            self.reads.entry(reader.to_owned()).or_default().insert(author.to_owned());
        }
    }

    /// Takes the grant away, if there was one.
    pub fn revoke(&mut self, reader: &str, author: &str) {
        if let Some(authors) = self.reads.get_mut(reader) {
            authors.remove(author);
            if authors.is_empty() {
                self.reads.remove(reader);
            }
        }
    }

    /// One line per grant, `<reader> reads <author>`.
    pub fn render(&self) -> String {
        let lines: Vec<String> = self
            .reads
            .iter()
            .flat_map(|(r, authors)| authors.iter().map(move |a| format!("{r} reads {a}")))
            .collect();
        if lines.is_empty() {
            return "nothing is shared".to_owned();
        }
        lines.join("\n")
    }
}

/// The policy in `base_dir`. A missing file is the empty policy; anything else
/// that cannot be read is an error the caller decides how to show.
pub fn load(base_dir: &Path) -> Result<SharingPolicy, SharingError> {
    match std::fs::read_to_string(sharing_file(base_dir)) {
        Ok(text) => SharingPolicy::parse(&text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(SharingPolicy::default()),
        Err(e) => Err(e.into()),
    }
}

/// Writes the policy through a temporary file and a rename, so the daemon
/// reading it on every search sees the old policy or the new one, never half.
/// Creates the data directory when nothing has run in it yet, so a grant can
/// be the first thing a user does.
pub fn save(base_dir: &Path, policy: &SharingPolicy) -> Result<(), SharingError> {
    let path = sharing_file(base_dir);
    let tmp = path.with_extension("json.tmp");
    std::fs::create_dir_all(base_dir).map_err(SharingError::Write)?;
    std::fs::write(&tmp, policy.to_json()).map_err(SharingError::Write)?;
    std::fs::rename(&tmp, &path).map_err(SharingError::Write)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(text: &str) -> SharingPolicy {
        SharingPolicy::parse(text).expect("valid policy")
    }

    #[test]
    fn a_missing_policy_shares_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let p = load(dir.path()).unwrap();
        assert_eq!(p, SharingPolicy::default());
        assert!(p.authors_for("cursor").is_empty());
    }

    #[test]
    fn a_grant_has_a_direction() {
        let p = policy(r#"{"version":1,"reads":{"cursor":["claude-code"]}}"#);
        assert_eq!(p.authors_for("cursor"), ["claude-code"]);
        assert!(p.authors_for("claude-code").is_empty());
    }

    #[test]
    fn names_match_exactly() {
        let p = policy(r#"{"version":1,"reads":{"cursor":["claude-code"]}}"#);
        assert!(p.authors_for("Cursor").is_empty());
        assert!(p.authors_for(" cursor").is_empty());
    }

    #[test]
    fn self_grants_and_repeats_are_ignored() {
        let p = policy(r#"{"version":1,"reads":{"cursor":["cursor","codex","claude-code","codex"]}}"#);
        assert_eq!(p.authors_for("cursor"), ["claude-code", "codex"]);
    }

    #[test]
    fn an_unknown_version_is_invalid() {
        let e = SharingPolicy::parse(r#"{"version":2,"reads":{}}"#).unwrap_err();
        assert!(matches!(e, SharingError::Version(2)), "{e}");
        assert!(e.to_string().contains("version 2"), "{e}");
    }

    #[test]
    fn an_unknown_field_is_invalid() {
        let e = SharingPolicy::parse(r#"{"version":1,"read":{"cursor":["codex"]}}"#).unwrap_err();
        assert!(matches!(e, SharingError::Json(_)), "{e}");
        assert!(e.to_string().contains("read"), "the message must name the field: {e}");
    }

    #[test]
    fn a_blank_name_is_invalid() {
        for text in [r#"{"version":1,"reads":{" ":["codex"]}}"#, r#"{"version":1,"reads":{"cursor":[""]}}"#] {
            assert!(matches!(SharingPolicy::parse(text), Err(SharingError::BlankName)), "{text}");
        }
    }

    #[test]
    fn an_unreadable_policy_shares_nothing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(sharing_file(dir.path()), "{ not json").unwrap();
        assert!(matches!(load(dir.path()), Err(SharingError::Json(_))));
    }

    #[test]
    fn grant_and_revoke_rewrite_the_policy() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = load(dir.path()).unwrap();
        p.grant("cursor", "claude-code");
        save(dir.path(), &p).unwrap();
        assert_eq!(load(dir.path()).unwrap().authors_for("cursor"), ["claude-code"]);
        assert!(!sharing_file(dir.path()).with_extension("json.tmp").exists(), "no temporary file is left behind");

        let mut p = load(dir.path()).unwrap();
        p.revoke("cursor", "claude-code");
        save(dir.path(), &p).unwrap();
        assert_eq!(load(dir.path()).unwrap(), SharingPolicy::default());
    }

    #[test]
    fn a_grant_can_come_before_the_data_directory_exists() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join(".clawg-dev");
        let mut p = SharingPolicy::default();
        p.grant("cursor", "claude-code");
        save(&base, &p).unwrap();
        assert_eq!(load(&base).unwrap(), p);
    }

    #[test]
    fn granting_twice_and_revoking_nothing_are_fine() {
        let mut p = SharingPolicy::default();
        p.grant("cursor", "codex");
        p.grant("cursor", "codex");
        assert_eq!(p.authors_for("cursor"), ["codex"]);
        p.revoke("cursor", "claude-code");
        p.revoke("nobody", "codex");
        assert_eq!(p.authors_for("cursor"), ["codex"]);
    }

    #[test]
    fn an_invalid_policy_is_never_rewritten() {
        // The guard is the caller's: `load` fails, so there is no policy to
        // change and save. This pins that `load` does fail rather than
        // handing back an empty policy that a save would then write out.
        let dir = tempfile::tempdir().unwrap();
        let hand_edit = r#"{"version":1,"reads":{"cursor":["claude-code",]}}"#;
        std::fs::write(sharing_file(dir.path()), hand_edit).unwrap();
        assert!(load(dir.path()).is_err());
        assert_eq!(std::fs::read_to_string(sharing_file(dir.path())).unwrap(), hand_edit);
    }

    #[test]
    fn the_saved_file_parses_back_to_the_same_policy() {
        let mut p = SharingPolicy::default();
        p.grant("codex", "cursor");
        p.grant("codex", "claude-code");
        p.grant("cursor", "claude-code");
        assert_eq!(SharingPolicy::parse(&p.to_json()).unwrap(), p);
    }

    #[test]
    fn render_lists_one_grant_per_line() {
        let mut p = SharingPolicy::default();
        assert_eq!(p.render(), "nothing is shared");
        p.grant("cursor", "codex");
        p.grant("cursor", "claude-code");
        p.grant("codex", "claude-code");
        assert_eq!(p.render(), "codex reads claude-code\ncursor reads claude-code\ncursor reads codex");
    }
}
