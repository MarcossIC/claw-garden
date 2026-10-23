//! `clawg doctor` — what is wrong with this install, its data directory and
//! its store, with the evidence and the next step that is safe to take.
//!
//! First, which `clawg` a terminal actually runs: two installs can sit on the
//! PATH and both answer `--version` identically, so nothing flags that the
//! earlier one shadows the later. Then the data directory, the store read
//! without the lock (`clawg_core::memory::inspect`) and Claude Code's
//! registration. Every decision is a pure function over facts ([`analyze`],
//! [`checks`], [`mcp`]); reading the environment and the filesystem stays in
//! [`run`], which never writes.

pub mod checks;
pub mod finding;
pub mod mcp;

use std::path::{Path, PathBuf};

use clawg_core::memory::inspect::inspect;
use clawg_core::memory::sharing;
use clawg_core::memory::spill::{REJECTED_FILE, SPILL_FILE};
use clawg_core::memory::store_lock::{StoreHolder, StoreLock};
use clawg_core::paths::{PathsError, Profile};
use clawg_core::preferences::{self, Lang};
use serde::Serialize;

use finding::{Finding, Summary};

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
pub(crate) fn identity(path: &Path) -> String {
    let text = path.to_string_lossy();
    if !cfg!(windows) {
        return text.into_owned();
    }
    let text = text.replace('/', "\\");
    text.strip_prefix(r"\\?\").unwrap_or(&text).to_lowercase()
}

/// The JSON document `clawg doctor` prints. The PATH fields stay as they
/// were before the checks, for whoever reads them.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Report {
    pub exe: PathBuf,
    pub version: String,
    pub profile: String,
    #[serde(flatten)]
    pub analysis: Analysis,
    pub data_dir: PathBuf,
    pub checks: Vec<Finding>,
    pub summary: Summary,
}

/// Failure gathering what `doctor` needs from the process.
#[derive(Debug, thiserror::Error)]
pub enum DoctorError {
    #[error("cannot locate the running clawg binary: {0}")]
    CurrentExe(#[from] std::io::Error),
    #[error(transparent)]
    DataDir(#[from] PathsError),
}

/// Reads the process PATH, the running executable, the profile's data
/// directory, its store and Claude Code's configuration, and checks them.
///
/// Writes nothing. The store is read without its lock, as `clawg secrets`
/// reads it, so it works beside a running daemon and leaves a stopped store
/// without a new byte or file; only confirming who holds the store takes the
/// lock for an instant, when `memory.owner` names a holder, as `clawg status`
/// does.
pub fn run(profile: Profile) -> Result<Report, DoctorError> {
    let exe = std::env::current_exe()?;
    let entries: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|raw| std::env::split_paths(&raw).collect())
        .unwrap_or_default();
    let name = if cfg!(windows) { "clawg.exe" } else { "clawg" };
    let analysis = analyze(&exe, &entries, name, |p| p.is_file());
    let data_dir = clawg_core::paths::data_dir(profile)?;

    // Read first: every message is written in the language it names.
    let preferences_path = clawg_core::paths::preferences_file(&data_dir);
    let (state, language) = if !preferences_path.exists() {
        (checks::PreferencesState::Missing, None)
    } else {
        match preferences::load(&data_dir) {
            Ok(p) => (checks::PreferencesState::Readable, p.language),
            Err(e) => (checks::PreferencesState::Invalid(e.to_string()), None),
        }
    };
    let lang = language.unwrap_or(Lang::En);

    let mut found = checks::path(lang, &analysis);
    found.extend(checks::data_dir(lang, &data_dir, dir_state(&data_dir)));
    let socket = clawg_core::paths::socket_file(&data_dir);
    found.extend(checks::socket(lang, &socket, &crate::daemon::socket::check_socket_path(&socket)));
    let owner = StoreLock::current_owner(&data_dir);
    found.extend(checks::store_owner(lang, owner.as_ref()));

    let inspection = inspect(&data_dir);
    found.extend(checks::schema(lang, &inspection.schema, profile));
    if let Some(facts) = &inspection.facts {
        found.extend(checks::contract(lang, facts.contract.as_ref(), &clawg_core::memory::bootstrap::serving_contract()));
        let has_rows = facts.agents.iter().any(|a| a.total > 0);
        let backups = names_in(&clawg_core::paths::backups_dir(&data_dir));
        found.extend(checks::backups(lang, &backups, has_rows, now_ms(), profile));
    }
    let rejected = present(&data_dir, REJECTED_FILE)
        .map(|path| (lines_in(&path), path));
    found.extend(checks::spill(
        lang,
        present(&data_dir, SPILL_FILE).as_deref(),
        rejected.as_ref().map(|(lines, path)| (path.as_path(), *lines)),
    ));
    let sharing_error = sharing::load(&data_dir).err().map(|e| e.to_string());
    found.extend(checks::sharing(lang, sharing_error.as_deref(), profile));
    found.extend(checks::preferences(lang, &state, &preferences_path));
    if let Some(language) = language {
        found.extend(protocol_block(language, profile));
    }
    if let Some(facts) = &inspection.facts {
        let daemon_running = owner.as_ref().is_some_and(|o| o.holder == StoreHolder::Daemon);
        found.extend(checks::outbox(lang, &facts.outbox, daemon_running, profile));
        found.extend(checks::agents(lang, &facts.agents));
        found.extend(checks::projects(lang, &facts.projects));
        found.extend(checks::invariants(lang, &facts.invariants));
    }
    found.extend(claude_code(lang, profile, &exe));

    let summary = Summary::of(&found);
    Ok(Report {
        exe,
        version: clawg_core::build_info::version().to_string(),
        profile: profile.as_str().to_string(),
        analysis,
        data_dir,
        checks: found,
        summary,
    })
}

#[cfg(unix)]
fn dir_state(dir: &Path) -> checks::DirState {
    use std::os::unix::fs::PermissionsExt;
    match std::fs::metadata(dir) {
        Ok(m) if m.is_dir() => checks::DirState::Present { mode: Some(m.permissions().mode()) },
        _ => checks::DirState::Missing,
    }
}

#[cfg(not(unix))]
fn dir_state(dir: &Path) -> checks::DirState {
    if dir.is_dir() {
        checks::DirState::Present { mode: None }
    } else {
        checks::DirState::Missing
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// The files of a directory, by name; none when it cannot be read.
fn names_in(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .map(|rd| rd.filter_map(Result::ok).map(|e| e.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default()
}

/// A file of the data directory, when it is there.
fn present(dir: &Path, name: &str) -> Option<PathBuf> {
    let path = dir.join(name);
    path.is_file().then_some(path)
}

/// The lines of a file, counted and never kept: they are someone's saves.
fn lines_in(path: &Path) -> usize {
    std::fs::read(path).map(|b| b.split(|&c| c == b'\n').filter(|l| !l.is_empty()).count()).unwrap_or(0)
}

/// The protocol block of the user `CLAUDE.md` `clawg setup claude-code`
/// writes, against the one it would write for `language`.
fn protocol_block(language: Lang, profile: Profile) -> Vec<Finding> {
    let home = clawg_core::paths::home_dir();
    let Ok((_, _, path)) = crate::setup::locations(home.as_deref(), std::env::var_os("CLAUDE_CONFIG_DIR")) else {
        return Vec::new();
    };
    let state = match std::fs::read_to_string(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => checks::BlockState::Missing,
        Err(e) => checks::BlockState::Unreadable(e.to_string()),
        Ok(text) => {
            let file = crate::setup::File { path: path.clone(), text: Some(text) };
            match crate::setup::find_block(&file) {
                Ok(Some((begin, end))) => checks::BlockState::Installed(file.text.unwrap_or_default()[begin..end].to_string()),
                Ok(None) => checks::BlockState::Missing,
                Err(e) => checks::BlockState::Unreadable(e.to_string()),
            }
        }
    };
    checks::protocol_block(language, &state, &crate::setup::block("\n", Some(language)), &path, profile)
}

/// Claude Code's registration of this profile's server, from the file
/// `clawg setup claude-code` reads.
fn claude_code(lang: Lang, profile: Profile, exe: &Path) -> Vec<Finding> {
    let home = clawg_core::paths::home_dir();
    let Ok((path, _, _)) = crate::setup::locations(home.as_deref(), std::env::var_os("CLAUDE_CONFIG_DIR")) else {
        return Vec::new();
    };
    let config = match std::fs::read_to_string(&path) {
        Ok(text) => mcp::Config::Text(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => mcp::Config::Missing,
        Err(e) => mcp::Config::Unreadable(e.to_string()),
    };
    mcp::mcp(lang, &path, &config, profile, exe, Path::is_file)
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
        let analysis = analyze(&exe("new"), &dirs(&["old", "new"]), NAME, |_| true);
        let found = checks::path(Lang::En, &analysis);
        let report = Report {
            exe: exe("new"),
            version: "0.1.0".into(),
            profile: "dev".into(),
            summary: Summary::of(&found),
            analysis,
            data_dir: dir(".clawg-dev"),
            checks: found,
        };
        let json = serde_json::to_value(&report).expect("serialise");
        for key in ["exe", "version", "profile", "installs", "winner", "shadowed", "duplicates", "data_dir", "checks", "summary"] {
            assert!(json.get(key).is_some(), "missing {key}: {json}");
        }
        assert_eq!(json["installs"][1]["is_current"], true);
        assert_eq!(json["shadowed"], true);
        assert_eq!(json["duplicates"], true);
        assert_eq!(json["checks"][0]["check_id"], "path");
        assert_eq!(json["summary"]["error"], 2);
    }

    #[test]
    fn path_findings() {
        let kinds = |a: &Analysis| -> Vec<(finding::Severity, &'static str)> {
            checks::path(Lang::En, a).iter().map(|f| (f.severity, f.reason_code)).collect()
        };
        let healthy = analyze(&exe("new"), &dirs(&["new"]), NAME, present(&["new"]));
        assert_eq!(kinds(&healthy), [(finding::Severity::Ok, "healthy")]);
        let none = analyze(&exe("build"), &dirs(&["tools"]), NAME, |_| false);
        assert_eq!(kinds(&none), [(finding::Severity::Ok, "healthy")]);
        let shadowed = analyze(&exe("new"), &dirs(&["old", "new"]), NAME, present(&["old", "new"]));
        assert_eq!(
            kinds(&shadowed),
            [(finding::Severity::Error, "shadowed"), (finding::Severity::Error, "duplicate")]
        );
        assert!(Summary::of(&checks::path(Lang::En, &shadowed)).fails(), "an unhealthy PATH still exits 1");
        let later = analyze(&exe("new"), &dirs(&["new", "old"]), NAME, present(&["old", "new"]));
        assert_eq!(kinds(&later), [(finding::Severity::Error, "duplicate")]);
    }
}
