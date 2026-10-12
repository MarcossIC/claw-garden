//! Single resolver for the on-disk data directory, with dev/prod profile
//! isolation.
//!
//! This module MUST NOT depend on anything else in this crate and MUST NOT
//! depend on any GUI/runtime crate — only `std`, the `dirs` crate and
//! `thiserror`. It is designed to be
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

    /// Loopback TCP port the `clawg serve` daemon binds for this profile.
    ///
    /// Two ports rather than one because the profiles are not alternatives at
    /// runtime: a developer routinely runs a release build against `~/.clawg`
    /// while a debug build works on `~/.clawg-dev`, and those are two live
    /// daemons. Sharing a port would make the second one fail to bind — or,
    /// worse, make an MCP client talk to the daemon of the *other* data
    /// directory, which is exactly the isolation [`Profile::dir_name`] exists
    /// to guarantee. Picked from the IANA dynamic/private range (49152+ is
    /// reserved for ephemeral allocation, so 27380/27381 sits below it where a
    /// fixed service port belongs) and adjacent so the pair reads as one
    /// allocation.
    pub const fn default_port(self) -> u16 {
        match self {
            Profile::Prod => 27380,
            Profile::Dev => 27381,
        }
    }
}

/// Failure resolving the data directory.
#[derive(Debug, thiserror::Error)]
pub enum PathsError {
    /// Falling back to the current directory would start against an empty
    /// store there, and the user's memory would look gone.
    #[error("cannot determine the home directory, which holds the clawg data directory; set HOME (USERPROFILE on Windows) and try again")]
    NoHome,
    #[error(transparent)]
    Profile(#[from] ProfileError),
}

/// Pure core of [`data_dir`]: `<home>/<profile.dir_name()>`.
pub fn data_dir_in(home: Option<PathBuf>, profile: Profile) -> Result<PathBuf, PathsError> {
    Ok(home.ok_or(PathsError::NoHome)?.join(profile.dir_name()))
}

/// Data directory for `profile` under the user's home directory.
pub fn data_dir(profile: Profile) -> Result<PathBuf, PathsError> {
    data_dir_in(dirs::home_dir(), profile)
}

/// [`data_dir`] for the profile resolved from the environment via
/// [`Profile::from_env`].
pub fn data_dir_from_env() -> Result<PathBuf, PathsError> {
    data_dir(Profile::from_env()?)
}

/// Where the `clawg serve` daemon publishes its liveness record
/// (`<base_dir>/daemon.json`: pid, port, version).
///
/// Lives beside the data it serves rather than in a system run directory so
/// the record is isolated per profile for free — a dev daemon can never
/// overwrite the prod daemon's entry, because they resolve different
/// `base_dir`s.
pub fn daemon_state_file(base_dir: &std::path::Path) -> PathBuf {
    base_dir.join("daemon.json")
}

/// Where the daemon's loopback bearer token is stored (`<base_dir>/token`).
///
/// Separate from [`daemon_state_file`] on purpose: the state file is meant to
/// be read by anything that wants to know whether a daemon is up, while this
/// one is a secret whose file permissions are the access control. Merging them
/// would leak the token to every liveness check.
pub fn token_file(base_dir: &std::path::Path) -> PathBuf {
    base_dir.join("token")
}

/// The file whose exclusive OS lock means "a process owns this profile's
/// memory store" (`<base_dir>/memory.lock`). Only ever locked, never read or
/// written — see [`store_owner_file`] for why the two are separate.
pub fn store_lock_file(base_dir: &std::path::Path) -> PathBuf {
    base_dir.join("memory.lock")
}

/// Where the store's holder says who it is (`<base_dir>/memory.owner`).
///
/// Separate from [`store_lock_file`] because on Windows a locked file cannot be
/// read by other processes, and the whole point of this record is to be read by
/// the process that just lost the lock.
pub fn store_owner_file(base_dir: &std::path::Path) -> PathBuf {
    base_dir.join("memory.owner")
}

#[cfg(test)]
mod tests {
    use super::*;

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

    // ── default_port ─────────────────────────────────────────────────────

    #[test]
    fn default_port_matches_profile() {
        assert_eq!(Profile::Prod.default_port(), 27380);
        assert_eq!(Profile::Dev.default_port(), 27381);
    }

    /// The whole point of a per-profile port: a dev daemon and a prod daemon
    /// must be able to listen at the same time without one refusing to bind.
    #[test]
    fn default_port_differs_between_profiles() {
        assert_ne!(Profile::Dev.default_port(), Profile::Prod.default_port());
    }

    // ── daemon_state_file / token_file ───────────────────────────────────

    #[test]
    fn daemon_state_file_is_daemon_json_under_base_dir() {
        let base = PathBuf::from("/base");
        assert_eq!(daemon_state_file(&base), base.join("daemon.json"));
    }

    #[test]
    fn token_file_is_token_under_base_dir() {
        let base = PathBuf::from("/base");
        assert_eq!(token_file(&base), base.join("token"));
    }

    #[test]
    fn store_lock_file_is_memory_lock_under_base_dir() {
        let base = PathBuf::from("/base");
        assert_eq!(store_lock_file(&base), base.join("memory.lock"));
    }

    #[test]
    fn store_owner_file_is_memory_owner_under_base_dir() {
        let base = PathBuf::from("/base");
        assert_eq!(store_owner_file(&base), base.join("memory.owner"));
    }

    /// The lock is never read and the record is read by losers; on Windows a
    /// locked file cannot be read by other processes, so merging them would
    /// make the "who holds it" message unreadable exactly when it is needed.
    #[test]
    fn store_lock_file_and_store_owner_file_are_distinct() {
        let base = PathBuf::from("/base");
        assert_ne!(store_lock_file(&base), store_owner_file(&base));
    }

    #[test]
    fn daemon_state_file_and_token_file_are_distinct() {
        let base = PathBuf::from("/base");
        assert_ne!(daemon_state_file(&base), token_file(&base));
    }

    // ── data_dir ─────────────────────────────────────────────────────────

    #[test]
    fn data_dir_ends_with_dev_dir_name() {
        let dir = data_dir(Profile::Dev).unwrap();
        assert_eq!(dir.file_name().unwrap(), Profile::Dev.dir_name());
    }

    #[test]
    fn data_dir_ends_with_prod_dir_name() {
        let dir = data_dir(Profile::Prod).unwrap();
        assert_eq!(dir.file_name().unwrap(), Profile::Prod.dir_name());
    }

    #[test]
    fn data_dir_without_a_home_is_an_error() {
        let err = data_dir_in(None, Profile::Dev).expect_err("no home must not resolve");
        assert!(err.to_string().contains("home directory"), "{err}");
    }

    #[test]
    fn data_dir_in_a_home_is_the_profile_directory_under_it() {
        let home = PathBuf::from("/home/someone");
        assert_eq!(
            data_dir_in(Some(home.clone()), Profile::Prod).unwrap(),
            home.join(".clawg")
        );
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
