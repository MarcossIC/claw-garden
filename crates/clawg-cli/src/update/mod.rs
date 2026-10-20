//! `clawg update`: what to download, whether to trust it, and how.

pub mod client;
pub mod effects;
pub mod eligibility;
pub mod lock;
pub mod plan;
pub mod release;
pub mod report;
pub mod signature;
pub mod version;

#[cfg(test)]
mod testkit;

use std::path::Path;

use clawg_core::build_info;
use clawg_core::paths::{data_dir, PathsError, Profile};

use crate::client::{probe_running, ClientError, Probe};
use crate::daemon::lifecycle::read_state;

use client::{FetchError, ReleaseClient};
use effects::{Candidate, Installation, Paths};
use eligibility::{eligibility, in_cargo_target, Refusal};
use lock::{LockError, UpdateLock};
use plan::{apply, aside_name, leftovers, plan, staged_name, Initial, Outcome, Running};
use release::{choose, Release, ReleaseError};
use report::{FailureContext, Updated};
use signature::trusted_keys;
use version::{Version, VersionError};

/// Exit codes of `clawg update --check`; `1` is any failure and `2` is clap's.
pub const EXIT_UP_TO_DATE: u8 = 0;
pub const EXIT_UPDATE_AVAILABLE: u8 = 100;

#[derive(Debug, PartialEq, Eq)]
pub enum CheckOutcome {
    UpToDate,
    Available(Version),
}

#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    #[error(transparent)]
    Refused(#[from] Refusal),
    #[error("this binary reports {0:?} as its version: {1}")]
    OwnVersion(String, VersionError),
    #[error("cannot tell where this binary is: {0}")]
    Executable(String),
    #[error(transparent)]
    Fetch(#[from] FetchError),
    #[error(transparent)]
    Release(#[from] ReleaseError),
    #[error(transparent)]
    Lock(#[from] LockError),
    #[error(transparent)]
    DataDir(#[from] PathsError),
    #[error("{0}")]
    Daemon(ClientError),
    /// The update could not be completed; the text says where it stopped.
    #[error("{0}")]
    Failed(String),
}

/// Only a release strictly newer than the installed version is an update.
pub fn outcome(installed: Version, newest: Option<&Release>) -> CheckOutcome {
    match newest {
        Some(release) if release.version > installed => CheckOutcome::Available(release.version),
        _ => CheckOutcome::UpToDate,
    }
}

pub fn exit_code(outcome: &CheckOutcome) -> u8 {
    match outcome {
        CheckOutcome::UpToDate => EXIT_UP_TO_DATE,
        CheckOutcome::Available(_) => EXIT_UPDATE_AVAILABLE,
    }
}

pub fn report(installed: Version, outcome: &CheckOutcome) -> String {
    match outcome {
        CheckOutcome::UpToDate => format!("clawg {installed} is up to date"),
        CheckOutcome::Available(newer) => format!("clawg {newer} is available (installed: {installed})"),
    }
}

/// `clawg update [--check]`: the text for stdout and the exit code.
pub async fn run(check: bool, profile: Profile) -> Result<(String, u8), UpdateError> {
    let exe = std::env::current_exe().map_err(|e| UpdateError::Executable(e.to_string()))?;
    // A test build whose stamp gives it a distribution mark stands for a
    // downloaded release, which is never a debug build.
    let debug = cfg!(debug_assertions) && build_info::dist_target().is_none();
    let triple = eligibility(
        debug,
        build_info::dist_target(),
        in_cargo_target(&exe),
        profile,
        Profile::compiled_default(),
    )?;
    let installed = Version::parse(build_info::version())
        .map_err(|e| UpdateError::OwnVersion(build_info::version().to_string(), e))?;

    // Held until the function returns. `--check` changes nothing, so it does
    // not wait for anyone and nobody waits for it.
    let base_dir = data_dir(profile)?;
    let _lock = if check { None } else { Some(UpdateLock::acquire(&base_dir)?) };

    let client = ReleaseClient::for_this_build();
    let listing = client.list_releases().await?;
    let newest = choose(&listing)?;
    let found = outcome(installed, newest.as_ref());
    if check {
        return Ok((report(installed, &found), exit_code(&found)));
    }
    let CheckOutcome::Available(_) = found else {
        return Ok((report(installed, &found), EXIT_UP_TO_DATE));
    };
    let release = newest.expect("an available update comes from a release");

    let old = running_daemon(&base_dir).await?;
    remove_leftovers(&exe);
    let installation = install(&exe, &base_dir, profile, installed, release, triple, client, old).await?;
    Ok((installation, EXIT_UP_TO_DATE))
}

/// The daemon answering for this profile, if one is.
async fn running_daemon(base_dir: &Path) -> Result<Option<Running>, UpdateError> {
    match probe_running(base_dir, false).await {
        Ok(Probe::Running(daemon)) => {
            let health = daemon.health().await.map_err(UpdateError::Daemon)?;
            let port = read_state(base_dir).and_then(|s| s.port).ok_or_else(|| {
                UpdateError::Daemon(ClientError::Http("the daemon answered but daemon.json is gone".to_string()))
            })?;
            Ok(Some(Running { pid: health.pid, port, version: health.version }))
        }
        Ok(Probe::NeedsStart) | Err(ClientError::NotRunning) => Ok(None),
        Err(other) => Err(UpdateError::Daemon(other)),
    }
}

/// Deletes what an earlier update left beside the binary. A file that is
/// still in use (an editor's bridge from before the update) stays for the next
/// run.
fn remove_leftovers(exe: &Path) {
    let (Some(dir), Some(name)) = (exe.parent(), exe.file_name().and_then(|n| n.to_str())) else {
        return;
    };
    let names: Vec<String> = std::fs::read_dir(dir)
        .map(|entries| entries.filter_map(Result::ok).map(|e| e.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    for old in leftovers(&names, name) {
        let _ = std::fs::remove_file(dir.join(old));
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

async fn install(
    exe: &Path,
    base_dir: &Path,
    profile: Profile,
    installed: Version,
    release: Release,
    triple: &'static str,
    client: ReleaseClient,
    old: Option<Running>,
) -> Result<String, UpdateError> {
    let (dir, name) = match (exe.parent(), exe.file_name().and_then(|n| n.to_str())) {
        (Some(dir), Some(name)) => (dir, name),
        _ => return Err(UpdateError::Executable(format!("{} has no directory or name", exe.display()))),
    };
    let stamp = now_ms();
    let paths = Paths {
        exe: exe.to_path_buf(),
        staged: dir.join(staged_name(name, stamp)),
        aside: dir.join(aside_name(name, stamp)),
        base_dir: base_dir.to_path_buf(),
    };
    let to = release.version;
    let had_daemon = old.is_some();
    let steps = plan(&Initial { daemon: old.clone() });
    let candidate = Candidate { release, triple, keys: trusted_keys(), client };
    let mut installation = Installation::new(paths, profile, candidate, installed, old);

    match apply(&steps, &mut installation).await {
        Outcome::Updated => Ok(report::success(&Updated {
            from: installed,
            to,
            aside: installation.set_aside_at().map(Path::to_path_buf).unwrap_or_default(),
            pre_migration: installation.pre_migration(),
            daemon: installation.new_daemon().cloned(),
        })),
        Outcome::Failed(failure) => {
            let context = FailureContext {
                from: installed,
                to,
                exe: exe.to_path_buf(),
                aside: installation.set_aside_at().map(Path::to_path_buf),
                data_dir: base_dir.to_path_buf(),
                pre_migration: installation.pre_migration(),
                had_daemon,
                answering: installation.answering().await,
            };
            Err(UpdateError::Failed(report::failure(&failure, &context)))
        }
    }
}

#[cfg(test)]
mod check_outcome_cases {
    use super::*;
    use release::choose;

    fn version(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    fn newest(tag: &str) -> Option<Release> {
        let listing = serde_json::json!([{
            "tag_name": tag, "draft": false, "prerelease": false, "assets": [],
        }]);
        choose(&listing.to_string()).unwrap()
    }

    #[test]
    fn only_a_strictly_greater_release_is_available() {
        assert_eq!(outcome(version("0.18.0"), newest("v0.19.0").as_ref()), CheckOutcome::Available(version("0.19.0")));
        assert_eq!(outcome(version("0.18.0"), newest("v0.18.0").as_ref()), CheckOutcome::UpToDate);
        assert_eq!(outcome(version("0.18.0"), newest("v0.17.9").as_ref()), CheckOutcome::UpToDate);
        assert_eq!(outcome(version("0.18.0"), None), CheckOutcome::UpToDate);
    }

    #[test]
    fn the_exit_codes_are_0_for_up_to_date_and_100_for_a_newer_release() {
        assert_eq!(exit_code(&CheckOutcome::UpToDate), 0);
        assert_eq!(exit_code(&CheckOutcome::Available(version("0.19.0"))), 100);
    }

    #[test]
    fn the_report_names_the_versions() {
        let installed = version("0.18.0");

        assert_eq!(report(installed, &CheckOutcome::UpToDate), "clawg 0.18.0 is up to date");
        assert_eq!(
            report(installed, &CheckOutcome::Available(version("0.19.0"))),
            "clawg 0.19.0 is available (installed: 0.18.0)"
        );
    }
}
