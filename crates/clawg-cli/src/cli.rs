//! Argument parsing for the `clawg` binary.
//!
//! Parsing is deliberately separated from execution: every type here is inert
//! data, so the whole surface is testable by feeding argv vectors to clap
//! without touching a filesystem, a port or a database.

use clap::{Parser, Subcommand};
use clawg_core::paths::{Profile, ProfileError};

/// `clawg` — local-first memory for coding agents.
#[derive(Debug, Parser)]
#[command(name = "clawg", version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum Command {
    /// Run the memory daemon (owns the database, the embedder and the workers).
    Serve {
        #[command(flatten)]
        profile: ProfileArg,
        /// TCP port on 127.0.0.1. `0` binds an ephemeral port, which is what
        /// tests and side-by-side dev runs want; omitted uses the profile's
        /// default port.
        #[arg(long)]
        port: Option<u16>,
    },
    /// Bridge stdio MCP to a running daemon, starting one if needed.
    Mcp {
        #[command(flatten)]
        profile: ProfileArg,
        /// Fail instead of starting a daemon when none is running.
        #[arg(long)]
        no_autostart: bool,
    },
    /// Print the daemon's state file and health as JSON. Exits 1 if not running.
    Status {
        #[command(flatten)]
        profile: ProfileArg,
    },
    /// Ask a running daemon to shut down.
    Stop {
        #[command(flatten)]
        profile: ProfileArg,
    },
}

/// The `--profile` flag, shared by every subcommand.
///
/// Flattened rather than global so each subcommand's `--help` lists it where a
/// reader looks for it, and so the parsed value lives next to the other options
/// of the command it belongs to.
#[derive(Debug, Clone, Default, PartialEq, Eq, clap::Args)]
pub struct ProfileArg {
    /// Data directory profile: `dev` (~/.clawg-dev) or `prod` (~/.clawg).
    /// Defaults to `$CLAWG_PROFILE`, then to the build type.
    #[arg(long, value_name = "dev|prod")]
    pub profile: Option<String>,
}

impl ProfileArg {
    /// Resolves the flag into a [`Profile`].
    ///
    /// Routed through [`Profile::resolve`] with the build's real
    /// `debug_assertions` so the flag inherits the env var's guarantee
    /// *exactly*: a debug binary refuses `prod`. Without that, `--profile prod`
    /// would be a hole straight through the isolation `CLAWG_PROFILE` exists to
    /// enforce — a debug build writing into the production data directory.
    ///
    /// An absent flag is NOT `Profile::compiled_default()`: it falls through to
    /// [`Profile::from_env`], so `CLAWG_PROFILE` still decides when the user
    /// did not say.
    pub fn resolve(&self) -> Result<Profile, ProfileArgError> {
        match self.profile.as_deref() {
            Some(raw) => Profile::resolve(Some(raw), cfg!(debug_assertions))
                .map_err(|inner| ProfileArgError { inner, from_flag: true }),
            None => Profile::from_env()
                .map_err(|inner| ProfileArgError { inner, from_flag: false }),
        }
    }
}

/// Failure turning `--profile` (or its env fallback) into a [`Profile`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileArgError {
    inner: ProfileError,
    /// Whether the bad value came from the flag or from the environment —
    /// the message has to name the right one, or the user edits the wrong thing.
    from_flag: bool,
}

impl std::fmt::Display for ProfileArgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.from_flag {
            match &self.inner {
                ProfileError::InvalidValue(raw) => write!(
                    f,
                    "invalid --profile value {raw:?}: accepted values are \"dev\" and \"prod\""
                ),
                ProfileError::ProdForbiddenInDebugBuild => write!(
                    f,
                    "--profile prod is forbidden in a debug build; use --profile dev, \
                     or run a release build to use prod"
                ),
            }
        } else {
            write!(f, "{}", self.inner)
        }
    }
}

impl std::error::Error for ProfileArgError {}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(args).expect("parse")
    }

    #[test]
    fn serve_defaults_have_no_profile_and_no_port() {
        let cli = parse(&["clawg", "serve"]);
        assert_eq!(
            cli.command,
            Command::Serve {
                profile: ProfileArg { profile: None },
                port: None
            }
        );
    }

    #[test]
    fn serve_accepts_profile_and_port() {
        let cli = parse(&["clawg", "serve", "--profile", "dev", "--port", "0"]);
        assert_eq!(
            cli.command,
            Command::Serve {
                profile: ProfileArg { profile: Some("dev".into()) },
                port: Some(0)
            }
        );
    }

    #[test]
    fn mcp_defaults_to_autostart_enabled() {
        let cli = parse(&["clawg", "mcp"]);
        assert_eq!(
            cli.command,
            Command::Mcp {
                profile: ProfileArg { profile: None },
                no_autostart: false
            }
        );
    }

    #[test]
    fn mcp_no_autostart_is_a_flag() {
        let cli = parse(&["clawg", "mcp", "--no-autostart"]);
        assert_eq!(
            cli.command,
            Command::Mcp {
                profile: ProfileArg { profile: None },
                no_autostart: true
            }
        );
    }

    #[test]
    fn status_and_stop_take_only_a_profile() {
        assert_eq!(
            parse(&["clawg", "status", "--profile", "dev"]).command,
            Command::Status { profile: ProfileArg { profile: Some("dev".into()) } }
        );
        assert_eq!(
            parse(&["clawg", "stop"]).command,
            Command::Stop { profile: ProfileArg { profile: None } }
        );
    }

    #[test]
    fn a_port_above_u16_is_a_usage_error_not_a_wrap() {
        assert!(Cli::try_parse_from(["clawg", "serve", "--port", "70000"]).is_err());
    }

    #[test]
    fn an_unknown_subcommand_is_rejected() {
        assert!(Cli::try_parse_from(["clawg", "frobnicate"]).is_err());
    }

    /// `--version` is how a user and a bug report agree on which binary they
    /// are talking about, so both halves of the line have to be right.
    #[test]
    fn version_prints_the_binary_name_and_the_package_version() {
        let err = Cli::try_parse_from(["clawg", "--version"]).expect_err("--version exits");
        assert_eq!(err.kind(), clap::error::ErrorKind::DisplayVersion);
        let rendered = err.to_string();
        assert!(rendered.contains("clawg"), "rendered: {rendered}");
        assert!(
            rendered.contains(env!("CARGO_PKG_VERSION")),
            "rendered: {rendered}"
        );
    }

    /// A subcommand is mandatory: a bare `clawg` has nothing to do, and doing
    /// nothing silently would read as a hang.
    #[test]
    fn a_bare_invocation_is_a_usage_error() {
        assert!(Cli::try_parse_from(["clawg"]).is_err());
    }

    // ── ProfileArg::resolve ─────────────────────────────────────────────────

    #[test]
    fn resolve_dev_flag_is_dev_in_any_build() {
        let arg = ProfileArg { profile: Some("dev".into()) };
        assert_eq!(arg.resolve(), Ok(Profile::Dev));
    }

    #[test]
    fn resolve_is_case_and_whitespace_insensitive_like_the_env_var() {
        let arg = ProfileArg { profile: Some("  DEV ".into()) };
        assert_eq!(arg.resolve(), Ok(Profile::Dev));
    }

    /// The point of routing through `Profile::resolve`: the flag must inherit
    /// the env var's refusal, otherwise `--profile prod` on a debug build
    /// writes into the production data directory.
    #[test]
    fn resolve_prod_flag_matches_the_env_vars_debug_build_rule() {
        let arg = ProfileArg { profile: Some("prod".into()) };
        if cfg!(debug_assertions) {
            let err = arg.resolve().expect_err("debug build must refuse prod");
            let msg = err.to_string();
            assert!(msg.contains("--profile"), "message was: {msg}");
            assert!(msg.contains("debug build"), "message was: {msg}");
        } else {
            assert_eq!(arg.resolve(), Ok(Profile::Prod));
        }
    }

    #[test]
    fn resolve_invalid_flag_names_the_flag_and_the_accepted_values() {
        let arg = ProfileArg { profile: Some("staging".into()) };
        let err = arg.resolve().expect_err("staging is not a profile");
        let msg = err.to_string();
        assert!(msg.contains("--profile"), "message was: {msg}");
        assert!(msg.contains("dev"), "message was: {msg}");
        assert!(msg.contains("prod"), "message was: {msg}");
        assert!(
            !msg.contains("CLAWG_PROFILE"),
            "a flag error must not send the user to the env var: {msg}"
        );
    }

    /// An absent flag must defer to the environment, not hardcode the compiled
    /// default — otherwise `CLAWG_PROFILE` silently stops working under the CLI.
    #[test]
    fn resolve_absent_flag_matches_from_env() {
        let arg = ProfileArg { profile: None };
        assert_eq!(arg.resolve().ok(), Profile::from_env().ok());
    }
}
