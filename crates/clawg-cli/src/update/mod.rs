//! `clawg update`: what to download, whether to trust it, and how.

pub mod client;
pub mod eligibility;
pub mod plan;
pub mod release;
pub mod report;
pub mod signature;
pub mod version;

#[cfg(test)]
mod testkit;

use clawg_core::build_info;
use clawg_core::paths::Profile;

use client::{FetchError, ReleaseClient};
use eligibility::{eligibility, in_cargo_target, Refusal};
use release::{choose, Release, ReleaseError};
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
    #[error("installing a release is not available yet: `clawg update --check` tells whether there is one")]
    InstallNotAvailable,
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
    eligibility(
        debug,
        build_info::dist_target(),
        in_cargo_target(&exe),
        profile,
        Profile::compiled_default(),
    )?;
    let installed = Version::parse(build_info::version())
        .map_err(|e| UpdateError::OwnVersion(build_info::version().to_string(), e))?;

    let listing = ReleaseClient::for_this_build().list_releases().await?;
    let found = outcome(installed, choose(&listing)?.as_ref());
    if !check {
        return Err(UpdateError::InstallNotAvailable);
    }
    Ok((report(installed, &found), exit_code(&found)))
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
