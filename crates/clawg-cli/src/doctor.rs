//! `clawg doctor` — which `clawg` does a terminal actually run?
//!
//! Two installs can sit on the PATH and both answer `--version` identically, so
//! nothing flags that the earlier one shadows the later. The analysis here is a
//! pure function over a list of directories and an "exists" predicate; reading
//! the environment and the filesystem stays in [`run`].

use std::path::{Path, PathBuf};

use clawg_core::paths::Profile;
use serde::Serialize;

/// One `clawg` executable found on the PATH.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Install {
    pub path: PathBuf,
    /// Whether this is the binary running `doctor`.
    pub is_current: bool,
}

/// What the PATH says about the running binary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Analysis {
    /// Every distinct executable on the PATH, in resolution order.
    pub installs: Vec<Install>,
    /// The one a shell would run: the first hit.
    pub winner: Option<PathBuf>,
    /// The first hit is not the running binary.
    pub shadowed: bool,
    /// More than one distinct executable is reachable.
    pub duplicates: bool,
}

impl Analysis {
    /// A healthy setup resolves to the running binary and has no second copy.
    pub fn is_healthy(&self) -> bool {
        !self.shadowed && !self.duplicates
    }
}

/// Finds every `exe_name` reachable through `path_entries`, in order, and
/// compares the first one with `current_exe`.
///
/// A directory listed twice yields one install: the second hit is the same
/// file, not a competing copy.
pub fn analyze(
    current_exe: &Path,
    path_entries: &[PathBuf],
    exe_name: &str,
    exists: impl Fn(&Path) -> bool,
) -> Analysis {
    let current = identity(current_exe);
    let mut seen: Vec<String> = Vec::new();
    let mut installs: Vec<Install> = Vec::new();

    for dir in path_entries {
        let dir = unquoted(dir);
        if dir.as_os_str().is_empty() {
            continue;
        }
        let candidate = dir.join(exe_name);
        let key = identity(&candidate);
        if seen.contains(&key) || !exists(&candidate) {
            continue;
        }
        installs.push(Install { is_current: key == current, path: candidate });
        seen.push(key);
    }

    let winner = installs.first().map(|i| i.path.clone());
    let shadowed = installs.first().is_some_and(|i| !i.is_current);
    let duplicates = installs.len() > 1;
    Analysis { installs, winner, shadowed, duplicates }
}

/// A PATH entry may be wrapped in quotes when it contains spaces.
fn unquoted(entry: &Path) -> PathBuf {
    PathBuf::from(entry.to_string_lossy().trim().trim_matches('"'))
}

/// A comparable form of a path: Windows ignores case, treats `/` as `\` and
/// may report the `\\?\` form, so none of those may make two spellings of one
/// file look different.
fn identity(path: &Path) -> String {
    let text = path.to_string_lossy();
    if !cfg!(windows) {
        return text.into_owned();
    }
    let text = text.replace('/', "\\");
    text.strip_prefix(r"\\?\").unwrap_or(&text).to_lowercase()
}

/// The JSON document `clawg doctor` prints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Report {
    pub exe: PathBuf,
    pub version: String,
    pub profile: String,
    #[serde(flatten)]
    pub analysis: Analysis,
}

/// Failure gathering what `doctor` needs from the process.
#[derive(Debug, thiserror::Error)]
pub enum DoctorError {
    #[error("cannot locate the running clawg binary: {0}")]
    CurrentExe(#[from] std::io::Error),
}

/// Reads the process PATH and the running executable, then analyses them.
/// Touches neither the store nor anything under the data directory.
pub fn run(profile: Profile) -> Result<Report, DoctorError> {
    let exe = std::env::current_exe()?;
    let entries: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|raw| std::env::split_paths(&raw).collect())
        .unwrap_or_default();
    let name = if cfg!(windows) { "clawg.exe" } else { "clawg" };
    let analysis = analyze(&exe, &entries, name, |p| p.is_file());
    Ok(Report {
        exe,
        version: env!("CARGO_PKG_VERSION").to_string(),
        profile: profile.as_str().to_string(),
        analysis,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const NAME: &str = "clawg.exe";

    /// A directory under this platform's root, so `join` builds the same
    /// spelling `analyze` builds and the fixtures hold on every platform.
    fn dir(name: &str) -> PathBuf {
        let root = if cfg!(windows) { r"C:\" } else { "/" };
        PathBuf::from(root).join(name)
    }

    fn exe(name: &str) -> PathBuf {
        dir(name).join(NAME)
    }

    fn dirs(list: &[&str]) -> Vec<PathBuf> {
        list.iter().map(|d| dir(d)).collect()
    }

    fn present(names: &[&str]) -> impl Fn(&Path) -> bool {
        let files: Vec<PathBuf> = names.iter().map(|n| exe(n)).collect();
        move |p| files.iter().any(|f| f == p)
    }

    #[test]
    fn a_single_install_that_is_the_running_binary_is_healthy() {
        let a = analyze(&exe("new"), &dirs(&["tools", "new"]), NAME, present(&["new"]));
        assert_eq!(a.installs.len(), 1);
        assert!(a.installs[0].is_current);
        assert_eq!(a.winner, Some(exe("new")));
        assert!(a.is_healthy());
    }

    #[test]
    fn an_earlier_copy_shadows_the_running_binary() {
        let a = analyze(&exe("new"), &dirs(&["old", "new"]), NAME, present(&["old", "new"]));
        assert!(a.shadowed);
        assert!(a.duplicates);
        assert_eq!(a.winner, Some(exe("old")));
        let flags: Vec<bool> = a.installs.iter().map(|i| i.is_current).collect();
        assert_eq!(flags, [false, true], "order is resolution order");
        assert!(!a.is_healthy());
    }

    #[test]
    fn a_later_copy_is_a_duplicate_but_does_not_shadow() {
        let a = analyze(&exe("new"), &dirs(&["new", "old"]), NAME, present(&["old", "new"]));
        assert!(!a.shadowed);
        assert!(a.duplicates);
        assert!(!a.is_healthy());
    }

    #[test]
    fn a_running_binary_off_the_path_is_shadowed_by_the_one_on_it() {
        let a = analyze(&exe("build"), &dirs(&["old"]), NAME, present(&["old"]));
        assert!(a.shadowed);
        assert!(!a.duplicates);
    }

    #[test]
    fn nothing_on_the_path_means_no_winner_and_no_shadowing() {
        let a = analyze(&exe("build"), &dirs(&["tools"]), NAME, |_| false);
        assert!(a.installs.is_empty());
        assert_eq!(a.winner, None);
        assert!(!a.shadowed);
        assert!(!a.duplicates);
    }

    #[test]
    fn a_directory_listed_twice_is_one_install() {
        let a = analyze(&exe("new"), &dirs(&["new", "new"]), NAME, present(&["new"]));
        assert_eq!(a.installs.len(), 1);
        assert!(!a.duplicates);
    }

    #[test]
    fn blank_entries_are_skipped_and_quotes_are_stripped() {
        let quoted = PathBuf::from(format!("\"{}\"", dir("Program Files").display()));
        let a = analyze(
            &exe("Program Files"),
            &[PathBuf::from(""), PathBuf::from("  "), quoted],
            NAME,
            present(&["Program Files"]),
        );
        assert_eq!(a.installs.len(), 1);
        assert!(a.installs[0].is_current);
    }

    #[cfg(windows)]
    #[test]
    fn windows_spellings_of_one_file_are_the_same_install() {
        let a = analyze(
            Path::new(r"\\?\C:\New\CLAWG.exe"),
            &[PathBuf::from("c:/new")],
            NAME,
            |_| true,
        );
        assert_eq!(a.installs.len(), 1);
        assert!(a.installs[0].is_current, "case, slashes and the verbatim prefix must not matter");
        assert!(a.is_healthy());
    }

    #[test]
    fn the_report_serialises_the_documented_fields() {
        let report = Report {
            exe: exe("new"),
            version: "0.1.0".into(),
            profile: "dev".into(),
            analysis: analyze(&exe("new"), &dirs(&["old", "new"]), NAME, |_| true),
        };
        let json = serde_json::to_value(&report).expect("serialise");
        for key in ["exe", "version", "profile", "installs", "winner", "shadowed", "duplicates"] {
            assert!(json.get(key).is_some(), "missing {key}: {json}");
        }
        assert_eq!(json["installs"][1]["is_current"], true);
        assert_eq!(json["shadowed"], true);
    }
}
