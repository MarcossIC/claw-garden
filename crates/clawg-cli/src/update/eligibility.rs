//! Whether this binary can replace itself with a release.

use std::path::Path;

use clawg_core::paths::Profile;

/// The targets `release.yml` publishes a binary for.
pub const PUBLISHED_TARGETS: [&str; 2] = ["x86_64-pc-windows-msvc", "x86_64-unknown-linux-gnu"];

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Refusal {
    #[error("this is a debug build, built locally: update it with git and cargo, or install a release")]
    DebugBuild,
    #[error("this binary was built locally, not downloaded from a release: update it with git and cargo, or install a release")]
    NotDistributed,
    #[error("no release is published for {0}")]
    NoReleaseForTarget(String),
    #[error("this binary runs from a cargo target directory: update it with git and cargo, or install a release")]
    InCargoTarget,
    #[error(
        "this binary updates the {} profile and is running the {} one",
        built.as_str(),
        running.as_str()
    )]
    OtherProfile { running: Profile, built: Profile },
}

/// Whether this binary may update itself, and for which published target.
///
/// `default_profile` is the profile the build uses when nothing says
/// otherwise; a run on another one is refused, because the update would swap
/// the executable of a profile the user did not ask about.
pub fn eligibility(
    debug: bool,
    dist_target: Option<&str>,
    in_cargo_target: bool,
    profile: Profile,
    default_profile: Profile,
) -> Result<&'static str, Refusal> {
    if debug {
        return Err(Refusal::DebugBuild);
    }
    let target = dist_target.ok_or(Refusal::NotDistributed)?;
    let published = PUBLISHED_TARGETS
        .iter()
        .find(|published| **published == target)
        .ok_or_else(|| Refusal::NoReleaseForTarget(target.to_string()))?;
    if in_cargo_target {
        return Err(Refusal::InCargoTarget);
    }
    if profile != default_profile {
        return Err(Refusal::OtherProfile { running: profile, built: default_profile });
    }
    Ok(published)
}

/// Whether `exe` sits in a cargo target directory: cargo writes a
/// `.cargo-lock` in every profile directory it builds into, whatever the
/// directory is called.
pub fn in_cargo_target(exe: &Path) -> bool {
    exe.parent().is_some_and(|dir| dir.join(".cargo-lock").exists())
}

#[cfg(test)]
mod update_eligibility_cases {
    use super::*;

    const LINUX: &str = "x86_64-unknown-linux-gnu";

    fn decide(
        debug: bool,
        dist_target: Option<&str>,
        in_cargo_target: bool,
        profile: Profile,
    ) -> Result<&'static str, Refusal> {
        eligibility(debug, dist_target, in_cargo_target, profile, Profile::Prod)
    }

    #[test]
    fn a_marked_release_outside_a_cargo_target_on_prod_passes() {
        assert_eq!(decide(false, Some(LINUX), false, Profile::Prod), Ok(LINUX));
        assert_eq!(
            decide(false, Some("x86_64-pc-windows-msvc"), false, Profile::Prod),
            Ok("x86_64-pc-windows-msvc")
        );
    }

    #[test]
    fn every_refusal_has_its_own_reason() {
        let debug = decide(true, Some(LINUX), false, Profile::Prod).unwrap_err();
        let unmarked = decide(false, None, false, Profile::Prod).unwrap_err();
        let unpublished = decide(false, Some("aarch64-apple-darwin"), false, Profile::Prod).unwrap_err();
        let in_target = decide(false, Some(LINUX), true, Profile::Prod).unwrap_err();
        let other_profile = decide(false, Some(LINUX), false, Profile::Dev).unwrap_err();

        assert_eq!(debug, Refusal::DebugBuild);
        assert_eq!(unmarked, Refusal::NotDistributed);
        assert_eq!(unpublished, Refusal::NoReleaseForTarget("aarch64-apple-darwin".into()));
        assert_eq!(in_target, Refusal::InCargoTarget);
        assert_eq!(other_profile, Refusal::OtherProfile { running: Profile::Dev, built: Profile::Prod });
        let reasons: Vec<String> = [debug, unmarked, unpublished, in_target, other_profile]
            .iter()
            .map(ToString::to_string)
            .collect();
        for (i, a) in reasons.iter().enumerate() {
            for b in &reasons[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }

    #[test]
    fn the_refusals_are_checked_in_order() {
        assert_eq!(decide(true, None, true, Profile::Dev), Err(Refusal::DebugBuild));
        assert_eq!(decide(false, None, true, Profile::Dev), Err(Refusal::NotDistributed));
        assert_eq!(
            decide(false, Some("riscv64"), true, Profile::Dev),
            Err(Refusal::NoReleaseForTarget("riscv64".into()))
        );
        assert_eq!(decide(false, Some(LINUX), true, Profile::Dev), Err(Refusal::InCargoTarget));
    }

    #[test]
    fn the_local_build_reasons_say_how_to_update_it() {
        for refusal in [Refusal::DebugBuild, Refusal::NotDistributed] {
            let text = refusal.to_string();
            assert!(text.contains("built locally"), "{text}");
            assert!(text.contains("git") && text.contains("cargo"), "{text}");
        }
    }

    #[test]
    fn a_cargo_lock_beside_the_executable_means_a_target_directory() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("clawg");
        assert!(!in_cargo_target(&exe));

        std::fs::write(dir.path().join(".cargo-lock"), "").unwrap();

        assert!(in_cargo_target(&exe));
    }
}
