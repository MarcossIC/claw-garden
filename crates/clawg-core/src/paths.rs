//! Single resolver for the on-disk data directory, with dev/prod profile
//! isolation.
//!
//! This module MUST NOT depend on anything else in this crate and MUST NOT
//! depend on any GUI/runtime crate — only `std` and the `dirs` crate. It is designed to be
//! lifted, unchanged, into this GUI-free crate.
//!
//! The profile is selected by the `CLAWG_PROFILE` env var (`dev` | `prod`,
//! case-insensitive, trimmed). Unset (or empty) falls back to the compiled
//! default: `dev` in a debug build, `prod` in a release build.
//! `CLAWG_PROFILE=prod` is refused in a debug build — a debug binary must
//! never write into the production data directory.

use std::path::PathBuf;

/// Which on-disk data directory (and identity) the app resolves to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    Dev,
    Prod,
}

/// Failure resolving [`Profile::ENV_VAR`] into a [`Profile`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileError {
    /// `CLAWG_PROFILE` was set to something other than `dev`/`prod`.
    InvalidValue(String),
    /// `CLAWG_PROFILE=prod` was set while running a debug build.
    ProdForbiddenInDebugBuild,
}

impl std::fmt::Display for ProfileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProfileError::InvalidValue(raw) => write!(
                f,
                "invalid {} value {raw:?}: accepted values are \"dev\" and \"prod\"",
                Profile::ENV_VAR
            ),
            ProfileError::ProdForbiddenInDebugBuild => write!(
                f,
                "{}=prod is forbidden in a debug build; unset {} (or set it to \"dev\") \
                 to use the dev profile, or run a release build to use prod",
                Profile::ENV_VAR,
                Profile::ENV_VAR
            ),
        }
    }
}

impl std::error::Error for ProfileError {}

impl Profile {
    /// Env var name that selects the profile.
    pub const ENV_VAR: &'static str = "CLAWG_PROFILE";

    /// The profile a build uses when `CLAWG_PROFILE` is unset: `Dev` for a
    /// debug build, `Prod` for a release build.
    pub const fn compiled_default() -> Profile {
        if cfg!(debug_assertions) {
            Profile::Dev
        } else {
            Profile::Prod
        }
    }

    /// Pure, testable core of profile resolution.
    ///
    /// `raw` is the env var value (`None` = unset). `debug_build` is passed
    /// explicitly (rather than read via `cfg!`) so both branches are testable
    /// regardless of how the test binary itself was compiled.
    pub fn resolve(raw: Option<&str>, debug_build: bool) -> Result<Profile, ProfileError> {
        let default = if debug_build { Profile::Dev } else { Profile::Prod };

        let trimmed = match raw {
            None => return Ok(default),
            Some(s) => s.trim(),
        };
        if trimmed.is_empty() {
            return Ok(default);
        }

        match trimmed.to_ascii_lowercase().as_str() {
            "dev" => Ok(Profile::Dev),
            "prod" => {
                if debug_build {
                    Err(ProfileError::ProdForbiddenInDebugBuild)
                } else {
                    Ok(Profile::Prod)
                }
            }
            _ => Err(ProfileError::InvalidValue(trimmed.to_string())),
        }
    }

    /// Thin wrapper: reads [`Profile::ENV_VAR`] from the process environment
    /// and resolves it against the build's actual `cfg!(debug_assertions)`.
    pub fn from_env() -> Result<Profile, ProfileError> {
        let raw = std::env::var(Self::ENV_VAR).ok();
        Self::resolve(raw.as_deref(), cfg!(debug_assertions))
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Profile::Dev => "dev",
            Profile::Prod => "prod",
        }
    }

    /// Name of the data directory under the user's home directory.
    pub const fn dir_name(self) -> &'static str {
        match self {
            Profile::Dev => ".clawg-dev",
            Profile::Prod => ".clawg",
        }
    }

    /// Identity this profile registers itself under with an MCP host.
    pub const fn mcp_server_name(self) -> &'static str {
        match self {
            Profile::Dev => "clawg-dev",
            Profile::Prod => "clawg",
        }
    }
}

/// Name of the project-relative configuration directory
/// (`<project>/.clawg/config.json`, `<project>/.clawg/agents/*.json`).
///
/// Deliberately NOT profile-dependent: profile isolation protects the user's
/// home data directory ([`data_dir`]); a folder that lives next to a project
/// is shared by whoever opens that project, regardless of build type.
pub const PROJECT_CONFIG_DIR: &str = ".clawg";

/// Data directory for `profile`: `<home>/<profile.dir_name()>`, falling back
/// to the current directory when the home directory cannot be determined.
pub fn data_dir(profile: Profile) -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(profile.dir_name())
}

/// [`data_dir`] for the profile resolved from the environment via
/// [`Profile::from_env`].
pub fn data_dir_from_env() -> Result<PathBuf, ProfileError> {
    Ok(data_dir(Profile::from_env()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_config_dir_is_fixed_and_not_profile_dependent() {
        assert_eq!(PROJECT_CONFIG_DIR, ".clawg");
        assert_ne!(PROJECT_CONFIG_DIR, Profile::Dev.dir_name());
    }

    // ── resolve: unset / empty ──────────────────────────────────────────

    #[test]
    fn resolve_none_with_debug_build_true_is_dev() {
        assert_eq!(Profile::resolve(None, true), Ok(Profile::Dev));
    }

    #[test]
    fn resolve_none_with_debug_build_false_is_prod() {
        assert_eq!(Profile::resolve(None, false), Ok(Profile::Prod));
    }

    #[test]
    fn resolve_empty_string_is_treated_as_unset_debug_true() {
        assert_eq!(Profile::resolve(Some(""), true), Ok(Profile::Dev));
    }

    #[test]
    fn resolve_empty_string_is_treated_as_unset_debug_false() {
        assert_eq!(Profile::resolve(Some(""), false), Ok(Profile::Prod));
    }

    #[test]
    fn resolve_whitespace_only_is_treated_as_unset() {
        assert_eq!(Profile::resolve(Some("   "), true), Ok(Profile::Dev));
        assert_eq!(Profile::resolve(Some("   "), false), Ok(Profile::Prod));
    }

    // ── resolve: dev ─────────────────────────────────────────────────────

    #[test]
    fn resolve_dev_lowercase() {
        assert_eq!(Profile::resolve(Some("dev"), true), Ok(Profile::Dev));
        assert_eq!(Profile::resolve(Some("dev"), false), Ok(Profile::Dev));
    }

    #[test]
    fn resolve_dev_uppercase() {
        assert_eq!(Profile::resolve(Some("DEV"), true), Ok(Profile::Dev));
    }

    #[test]
    fn resolve_dev_with_surrounding_whitespace() {
        assert_eq!(Profile::resolve(Some(" dev "), true), Ok(Profile::Dev));
    }

    // ── resolve: prod ────────────────────────────────────────────────────

    #[test]
    fn resolve_prod_lowercase_in_release_build_is_prod() {
        assert_eq!(Profile::resolve(Some("prod"), false), Ok(Profile::Prod));
    }

    #[test]
    fn resolve_prod_uppercase_in_release_build_is_prod() {
        assert_eq!(Profile::resolve(Some("PROD"), false), Ok(Profile::Prod));
    }

    #[test]
    fn resolve_prod_with_surrounding_whitespace_in_release_build_is_prod() {
        assert_eq!(Profile::resolve(Some(" prod "), false), Ok(Profile::Prod));
    }

    #[test]
    fn resolve_prod_in_debug_build_is_forbidden() {
        assert_eq!(
            Profile::resolve(Some("prod"), true),
            Err(ProfileError::ProdForbiddenInDebugBuild)
        );
    }

    // ── resolve: invalid ─────────────────────────────────────────────────

    #[test]
    fn resolve_invalid_value_is_err_regardless_of_debug_build() {
        assert_eq!(
            Profile::resolve(Some("staging"), true),
            Err(ProfileError::InvalidValue("staging".to_string()))
        );
        assert_eq!(
            Profile::resolve(Some("staging"), false),
            Err(ProfileError::InvalidValue("staging".to_string()))
        );
    }

    #[test]
    fn invalid_value_error_trims_before_reporting() {
        assert_eq!(
            Profile::resolve(Some("  bogus  "), true),
            Err(ProfileError::InvalidValue("bogus".to_string()))
        );
    }

    // ── Display ──────────────────────────────────────────────────────────

    #[test]
    fn invalid_value_display_mentions_env_var_and_accepted_values() {
        let err = ProfileError::InvalidValue("bogus".to_string());
        let msg = err.to_string();
        assert!(msg.contains("CLAWG_PROFILE"), "message was: {msg}");
        assert!(msg.contains("dev"), "message was: {msg}");
        assert!(msg.contains("prod"), "message was: {msg}");
    }

    #[test]
    fn prod_forbidden_display_mentions_env_var() {
        let err = ProfileError::ProdForbiddenInDebugBuild;
        let msg = err.to_string();
        assert!(msg.contains("CLAWG_PROFILE"), "message was: {msg}");
    }

    // ── as_str / dir_name / mcp_server_name ─────────────────────────────

    #[test]
    fn as_str_matches_profile() {
        assert_eq!(Profile::Dev.as_str(), "dev");
        assert_eq!(Profile::Prod.as_str(), "prod");
    }

    #[test]
    fn dir_name_matches_profile() {
        assert_eq!(Profile::Dev.dir_name(), ".clawg-dev");
        assert_eq!(Profile::Prod.dir_name(), ".clawg");
    }

    #[test]
    fn mcp_server_name_matches_profile() {
        assert_eq!(Profile::Dev.mcp_server_name(), "clawg-dev");
        assert_eq!(Profile::Prod.mcp_server_name(), "clawg");
    }

    // ── data_dir ─────────────────────────────────────────────────────────

    #[test]
    fn data_dir_ends_with_dev_dir_name() {
        let dir = data_dir(Profile::Dev);
        assert_eq!(dir.file_name().unwrap(), Profile::Dev.dir_name());
    }

    #[test]
    fn data_dir_ends_with_prod_dir_name() {
        let dir = data_dir(Profile::Prod);
        assert_eq!(dir.file_name().unwrap(), Profile::Prod.dir_name());
    }

    // ── compiled_default ─────────────────────────────────────────────────

    #[test]
    fn compiled_default_is_dev_iff_debug_assertions() {
        let expected = if cfg!(debug_assertions) {
            Profile::Dev
        } else {
            Profile::Prod
        };
        assert_eq!(Profile::compiled_default(), expected);
    }
}
