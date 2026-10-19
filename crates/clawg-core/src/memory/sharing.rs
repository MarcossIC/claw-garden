//! Which agents read which agents' memories.
//!
//! Every agent reads every other agent's memories unless the user hides them:
//! a set of directed exclusions — "cursor does not read codex" — kept in
//! `<data>/sharing.json`, where `*` stands for every agent. A row no agent
//! saved is read by every agent: an exclusion names an author, and that row
//! has none. Parsing and resolving are pure; only [`load`] and [`save`] touch
//! the filesystem.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::paths::sharing_file;

/// The only file format this build reads and writes.
const VERSION: u32 = 1;

/// Every agent, on either side of an exclusion.
pub const EVERY_AGENT: &str = "*";

/// Why `sharing.json` could not be used. Every caller treats it as "nothing is
/// shared": an exclusion the user wrote must not be lost to a typo. The
/// message is what the user is shown so they can fix the file.
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

/// What one agent's search reaches besides its own rows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Reach {
    /// No other agent's rows: its own, and the rows no agent saved.
    #[default]
    Own,
    /// Every agent's rows except these authors', and the rows no agent saved.
    AllBut(Vec<String>),
    /// Its own rows alone, not even the ones no agent saved: what a save is
    /// compared against, since an agent can only judge a pair it owns.
    OwnOnly,
}

impl Reach {
    /// Whether `reader` may read a row `author` saved; `None` when no agent did.
    pub(crate) fn admits(&self, reader: &str, author: Option<&str>) -> bool {
        match (self, author) {
            (Reach::OwnOnly, author) => author == Some(reader),
            (_, None) => true,
            (Reach::Own, Some(author)) => author == reader,
            (Reach::AllBut(hidden), Some(author)) => author == reader || !hidden.iter().any(|h| h == author),
        }
    }
}

/// The file as written. `deny_unknown_fields` because a misspelled key
/// (`"hide"`) would otherwise parse as an empty policy, and a user who meant to
/// hide something would see it shared with no word about why.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct OnDisk {
    version: u32,
    #[serde(default)]
    hides: BTreeMap<String, Vec<String>>,
}

/// Reader to the authors hidden from it. Ordered maps so the same policy
/// always renders and saves the same way.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SharingPolicy {
    hides: BTreeMap<String, BTreeSet<String>>,
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
        for (reader, authors) in on_disk.hides {
            if reader.trim().is_empty() {
                return Err(SharingError::BlankName);
            }
            for author in authors {
                if author.trim().is_empty() {
                    return Err(SharingError::BlankName);
                }
                policy.hide(&reader, &author);
            }
        }
        Ok(policy)
    }

    pub fn to_json(&self) -> String {
        let on_disk = OnDisk {
            version: VERSION,
            hides: self.hides.iter().map(|(r, a)| (r.clone(), a.iter().cloned().collect())).collect(),
        };
        // A map of strings to string lists always serializes.
        let mut json = serde_json::to_string_pretty(&on_disk).unwrap_or_default();
        json.push('\n');
        json
    }

    /// What `reader`'s searches reach: every other agent but the ones hidden
    /// from it, by name or through `*`.
    pub fn reach_for(&self, reader: &str) -> Reach {
        let named = self.hides.get(reader).into_iter().flatten();
        let everyone = self.hides.get(EVERY_AGENT).into_iter().flatten();
        let hidden: BTreeSet<&String> = named.chain(everyone).collect();
        if hidden.iter().any(|a| *a == EVERY_AGENT) {
            return Reach::Own;
        }
        Reach::AllBut(hidden.into_iter().filter(|a| *a != reader).cloned().collect())
    }

    /// Stops `reader` from reading `author`. Hiding an agent from itself
    /// means nothing and is dropped; hiding twice is the same as once.
    pub fn hide(&mut self, reader: &str, author: &str) {
        if reader != author || reader == EVERY_AGENT {
            self.hides.entry(reader.to_owned()).or_default().insert(author.to_owned());
        }
    }

    /// Takes an exclusion away, if there was one.
    pub fn unhide(&mut self, reader: &str, author: &str) {
        if let Some(authors) = self.hides.get_mut(reader) {
            authors.remove(author);
            if authors.is_empty() {
                self.hides.remove(reader);
            }
        }
    }

    /// One line per exclusion, or that everything is shared.
    pub fn render(&self) -> String {
        let lines: Vec<String> = self
            .hides
            .iter()
            .flat_map(|(r, authors)| {
                authors.iter().map(move |a| match (r.as_str(), a.as_str()) {
                    (EVERY_AGENT, EVERY_AGENT) => "no agent reads another".to_owned(),
                    (EVERY_AGENT, a) => format!("no agent reads {a}"),
                    (r, EVERY_AGENT) => format!("{r} reads no other agent"),
                    (r, a) => format!("{r} does not read {a}"),
                })
            })
            .collect();
        if lines.is_empty() {
            return "every agent reads every other agent".to_owned();
        }
        lines.join("\n")
    }
}

/// The policy in `base_dir`. A missing file is the empty policy, which shares
/// everything; anything else that cannot be read is an error the caller
/// decides how to show.
pub fn load(base_dir: &Path) -> Result<SharingPolicy, SharingError> {
    match std::fs::read_to_string(sharing_file(base_dir)) {
        Ok(text) => SharingPolicy::parse(&text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(SharingPolicy::default()),
        Err(e) => Err(e.into()),
    }
}

/// Writes the policy through a temporary file and a rename, so the daemon
/// reading it on every search sees the old policy or the new one, never half.
/// Creates the data directory when nothing has run in it yet, so an exclusion
/// can be the first thing a user does.
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

    fn all_but(names: &[&str]) -> Reach {
        Reach::AllBut(names.iter().map(|n| n.to_string()).collect())
    }

    #[test]
    fn a_reach_admits_the_reader_and_whoever_is_not_hidden() {
        assert!(Reach::Own.admits("a", Some("a")));
        assert!(!Reach::Own.admits("a", Some("b")));
        let all_but_c = all_but(&["c"]);
        assert!(all_but_c.admits("a", Some("a")) && all_but_c.admits("a", Some("b")));
        assert!(!all_but_c.admits("a", Some("c")));
    }

    #[test]
    fn a_row_no_agent_saved_is_read_by_every_agent() {
        let nothing_shared = policy(r#"{"version":1,"hides":{"*":["*"]}}"#).reach_for("cursor");
        for reach in [Reach::Own, all_but(&[]), all_but(&["codex"]), nothing_shared] {
            assert!(reach.admits("cursor", None), "no exclusion names a row without an author: {reach:?}");
        }
        assert!(!Reach::OwnOnly.admits("cursor", None), "a save is compared only against its agent's rows");
        assert!(Reach::OwnOnly.admits("cursor", Some("cursor")) && !Reach::OwnOnly.admits("cursor", Some("codex")));
    }

    #[test]
    fn a_missing_policy_shares_everything() {
        let dir = tempfile::tempdir().unwrap();
        let p = load(dir.path()).unwrap();
        assert_eq!(p, SharingPolicy::default());
        assert_eq!(p.reach_for("cursor"), all_but(&[]));
    }

    #[test]
    fn an_exclusion_has_a_direction() {
        let p = policy(r#"{"version":1,"hides":{"cursor":["claude-code"]}}"#);
        assert_eq!(p.reach_for("cursor"), all_but(&["claude-code"]));
        assert_eq!(p.reach_for("claude-code"), all_but(&[]));
    }

    #[test]
    fn every_agent_stands_for_either_side() {
        let p = policy(r#"{"version":1,"hides":{"*":["codex"],"cursor":["claude-code"]}}"#);
        assert_eq!(p.reach_for("cursor"), all_but(&["claude-code", "codex"]), "nobody reads codex");
        assert_eq!(p.reach_for("codex"), all_but(&[]), "codex still reads the others");

        let p = policy(r#"{"version":1,"hides":{"cursor":["*"]}}"#);
        assert_eq!(p.reach_for("cursor"), Reach::Own);
        assert_eq!(p.reach_for("codex"), all_but(&[]));

        let p = policy(r#"{"version":1,"hides":{"*":["*"]}}"#);
        assert_eq!(p.reach_for("cursor"), Reach::Own, "nothing is shared");
    }

    #[test]
    fn names_match_exactly() {
        let p = policy(r#"{"version":1,"hides":{"cursor":["claude-code"]}}"#);
        assert_eq!(p.reach_for("Cursor"), all_but(&[]));
        assert_eq!(p.reach_for(" cursor"), all_but(&[]));
    }

    #[test]
    fn self_exclusions_and_repeats_are_ignored() {
        let p = policy(r#"{"version":1,"hides":{"cursor":["cursor","codex","claude-code","codex"]}}"#);
        assert_eq!(p.reach_for("cursor"), all_but(&["claude-code", "codex"]));
    }

    #[test]
    fn an_unknown_version_is_invalid() {
        let e = SharingPolicy::parse(r#"{"version":2,"hides":{}}"#).unwrap_err();
        assert!(matches!(e, SharingError::Version(2)), "{e}");
        assert!(e.to_string().contains("version 2"), "{e}");
    }

    #[test]
    fn an_unknown_field_is_invalid() {
        let e = SharingPolicy::parse(r#"{"version":1,"hide":{"cursor":["codex"]}}"#).unwrap_err();
        assert!(matches!(e, SharingError::Json(_)), "{e}");
        assert!(e.to_string().contains("hide"), "the message must name the field: {e}");
    }

    #[test]
    fn a_blank_name_is_invalid() {
        for text in [r#"{"version":1,"hides":{" ":["codex"]}}"#, r#"{"version":1,"hides":{"cursor":[""]}}"#] {
            assert!(matches!(SharingPolicy::parse(text), Err(SharingError::BlankName)), "{text}");
        }
    }

    #[test]
    fn an_unreadable_policy_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(sharing_file(dir.path()), "{ not json").unwrap();
        assert!(matches!(load(dir.path()), Err(SharingError::Json(_))));
    }

    #[test]
    fn hide_and_unhide_rewrite_the_policy() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = load(dir.path()).unwrap();
        p.hide("cursor", "claude-code");
        save(dir.path(), &p).unwrap();
        assert_eq!(load(dir.path()).unwrap().reach_for("cursor"), all_but(&["claude-code"]));
        assert!(!sharing_file(dir.path()).with_extension("json.tmp").exists(), "no temporary file is left behind");

        let mut p = load(dir.path()).unwrap();
        p.unhide("cursor", "claude-code");
        save(dir.path(), &p).unwrap();
        assert_eq!(load(dir.path()).unwrap(), SharingPolicy::default());
    }

    #[test]
    fn an_exclusion_can_come_before_the_data_directory_exists() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join(".clawg-dev");
        let mut p = SharingPolicy::default();
        p.hide("cursor", "claude-code");
        save(&base, &p).unwrap();
        assert_eq!(load(&base).unwrap(), p);
    }

    #[test]
    fn hiding_twice_and_unhiding_nothing_are_fine() {
        let mut p = SharingPolicy::default();
        p.hide("cursor", "codex");
        p.hide("cursor", "codex");
        assert_eq!(p.reach_for("cursor"), all_but(&["codex"]));
        p.unhide("cursor", "claude-code");
        p.unhide("nobody", "codex");
        assert_eq!(p.reach_for("cursor"), all_but(&["codex"]));
    }

    #[test]
    fn an_invalid_policy_is_never_rewritten() {
        // The guard is the caller's: `load` fails, so there is no policy to
        // change and save. This pins that `load` does fail rather than
        // handing back an empty policy that a save would then write out.
        let dir = tempfile::tempdir().unwrap();
        let hand_edit = r#"{"version":1,"hides":{"cursor":["claude-code",]}}"#;
        std::fs::write(sharing_file(dir.path()), hand_edit).unwrap();
        assert!(load(dir.path()).is_err());
        assert_eq!(std::fs::read_to_string(sharing_file(dir.path())).unwrap(), hand_edit);
    }

    #[test]
    fn the_saved_file_parses_back_to_the_same_policy() {
        let mut p = SharingPolicy::default();
        p.hide("codex", "cursor");
        p.hide("*", "claude-code");
        p.hide("cursor", "*");
        assert_eq!(SharingPolicy::parse(&p.to_json()).unwrap(), p);
    }

    #[test]
    fn render_lists_one_exclusion_per_line() {
        let mut p = SharingPolicy::default();
        assert_eq!(p.render(), "every agent reads every other agent");
        p.hide("cursor", "codex");
        p.hide("*", "claude-code");
        p.hide("codex", "*");
        assert_eq!(p.render(), "no agent reads claude-code\ncodex reads no other agent\ncursor does not read codex");
        let mut off = SharingPolicy::default();
        off.hide("*", "*");
        assert_eq!(off.render(), "no agent reads another");
    }
}
