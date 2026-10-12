//! Argument parsing for the `clawg` binary.
//!
//! Parsing is deliberately separated from execution: every type here is inert
//! data, so the whole surface is testable by feeding argv vectors to clap
//! without touching a filesystem, a port or a database.

use std::path::PathBuf;

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
    /// Stop the running daemon and wait until its process has exited.
    Stop {
        #[command(flatten)]
        profile: ProfileArg,
    },
    /// Report which `clawg` executables the PATH resolves to and whether this
    /// one is the one a terminal runs. Exits 1 if shadowed or duplicated.
    Doctor {
        #[command(flatten)]
        profile: ProfileArg,
    },
    /// Browse what the daemon has stored. Read-only; needs a running daemon.
    Memories {
        #[command(subcommand)]
        action: MemoriesAction,
    },
    /// Add or remove a directory on the user PATH (Windows). The installer
    /// runs this so `clawg` works from any new terminal.
    Path {
        #[command(subcommand)]
        action: PathAction,
    },
}

#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum MemoriesAction {
    /// List stored rows, newest first.
    List(ListOpts),
    /// Find memories whose content matches the words of QUERY.
    Search(SearchOpts),
    /// Show one row in full, with its metadata.
    Show(ShowOpts),
    /// Count what is stored, grouped by type, tier, status, agent and project.
    Stats(StatsOpts),
}

impl MemoriesAction {
    /// The `--profile` the action was given, whichever action it is.
    pub fn profile(&self) -> &ProfileArg {
        match self {
            MemoriesAction::List(o) => &o.profile,
            MemoriesAction::Search(o) => &o.profile,
            MemoriesAction::Show(o) => &o.profile,
            MemoriesAction::Stats(o) => &o.profile,
        }
    }
}

/// The three tables a row can live in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum KindArg {
    Memory,
    Episodic,
    Procedural,
}

impl KindArg {
    /// The spelling the daemon's API takes.
    pub fn as_str(self) -> &'static str {
        match self {
            KindArg::Memory => "memory",
            KindArg::Episodic => "episodic",
            KindArg::Procedural => "procedural",
        }
    }
}

/// The lifecycle states a memory can be in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum StatusArg {
    Active,
    Disputed,
    Superseded,
}

impl StatusArg {
    /// The spelling the daemon's API takes.
    pub fn as_str(self) -> &'static str {
        match self {
            StatusArg::Active => "active",
            StatusArg::Disputed => "disputed",
            StatusArg::Superseded => "superseded",
        }
    }
}

/// Who or what the rows belong to.
#[derive(Debug, Clone, Default, PartialEq, Eq, clap::Args)]
pub struct Scope {
    /// Only rows of this agent.
    #[arg(long, value_name = "ID")]
    pub agent: Option<String>,
    /// Only rows of this project id.
    #[arg(long, value_name = "ID")]
    pub project: Option<String>,
    /// Only rows of the project of the current directory, as `clawg mcp` would
    /// scope them.
    #[arg(long, conflicts_with = "project")]
    pub here: bool,
}

impl Scope {
    /// The project id to filter by: the one named, or for `--here` the id of
    /// the project the current directory resolves to. Takes that id rather
    /// than resolving it so the rule is testable without a filesystem.
    pub fn project_id(&self, cwd_project: Option<String>) -> Option<String> {
        if self.here {
            return cwd_project;
        }
        self.project.clone()
    }
}

/// The filters `list` and `search` share.
#[derive(Debug, Clone, Default, PartialEq, Eq, clap::Args)]
pub struct Filters {
    #[command(flatten)]
    pub scope: Scope,
    /// Only memories of this type (memory rows only).
    #[arg(long = "type", value_name = "T")]
    pub memory_type: Option<String>,
    /// Only rows in this tier: 1 (hot), 2 (warm) or 3 (cold).
    #[arg(long, value_parser = clap::value_parser!(i64).range(1..=3))]
    pub tier: Option<i64>,
    /// Only memories in this lifecycle state.
    #[arg(long, value_enum)]
    pub status: Option<StatusArg>,
}

#[derive(Debug, Clone, PartialEq, Eq, clap::Args)]
pub struct ListOpts {
    /// Which table to list. Lists all three when omitted.
    #[arg(long, value_enum)]
    pub kind: Option<KindArg>,
    #[command(flatten)]
    pub filters: Filters,
    /// How many rows to show (1 to 100).
    #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(1..=100))]
    pub limit: u32,
    /// How many rows to skip, for paging.
    #[arg(long, default_value_t = 0)]
    pub offset: u32,
    /// Print JSON instead of a table.
    #[arg(long)]
    pub json: bool,
    #[command(flatten)]
    pub profile: ProfileArg,
}

#[derive(Debug, Clone, PartialEq, Eq, clap::Args)]
pub struct SearchOpts {
    /// Words to look for. Every word must match, as a prefix, ignoring case
    /// and accents. Only `memory` rows are searched.
    pub query: String,
    #[command(flatten)]
    pub filters: Filters,
    /// How many rows to show (1 to 100).
    #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(1..=100))]
    pub limit: u32,
    /// How many rows to skip, for paging.
    #[arg(long, default_value_t = 0)]
    pub offset: u32,
    /// Print JSON instead of a table.
    #[arg(long)]
    pub json: bool,
    #[command(flatten)]
    pub profile: ProfileArg,
}

#[derive(Debug, Clone, PartialEq, Eq, clap::Args)]
pub struct ShowOpts {
    /// The row id, as printed by `list` and `search`. Ids are per table.
    pub id: i64,
    /// Which table the id belongs to.
    #[arg(long, value_enum, default_value_t = KindArg::Memory)]
    pub kind: KindArg,
    /// Print JSON instead of the detail view.
    #[arg(long)]
    pub json: bool,
    #[command(flatten)]
    pub profile: ProfileArg,
}

#[derive(Debug, Clone, PartialEq, Eq, clap::Args)]
pub struct StatsOpts {
    #[command(flatten)]
    pub scope: Scope,
    /// Print JSON instead of text.
    #[arg(long)]
    pub json: bool,
    #[command(flatten)]
    pub profile: ProfileArg,
}

#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum PathAction {
    /// Append the directory unless an equivalent entry is already there.
    Add {
        /// Directory to add. Defaults to the one holding this `clawg` binary.
        #[arg(long)]
        dir: Option<PathBuf>,
    },
    /// Remove every entry naming the directory.
    Remove {
        /// Directory to remove. Defaults to the one holding this `clawg` binary.
        #[arg(long)]
        dir: Option<PathBuf>,
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
    fn doctor_takes_only_a_profile() {
        assert_eq!(
            parse(&["clawg", "doctor"]).command,
            Command::Doctor { profile: ProfileArg { profile: None } }
        );
        assert_eq!(
            parse(&["clawg", "doctor", "--profile", "dev"]).command,
            Command::Doctor { profile: ProfileArg { profile: Some("dev".into()) } }
        );
    }

    #[test]
    fn path_add_and_remove_default_to_the_binarys_own_directory() {
        assert_eq!(
            parse(&["clawg", "path", "add"]).command,
            Command::Path { action: PathAction::Add { dir: None } }
        );
        assert_eq!(
            parse(&["clawg", "path", "remove"]).command,
            Command::Path { action: PathAction::Remove { dir: None } }
        );
    }

    #[test]
    fn path_accepts_an_explicit_directory() {
        assert_eq!(
            parse(&["clawg", "path", "add", "--dir", r"C:\Program Files\Claw Garden"]).command,
            Command::Path {
                action: PathAction::Add { dir: Some(r"C:\Program Files\Claw Garden".into()) }
            }
        );
    }

    #[test]
    fn path_without_an_action_is_a_usage_error() {
        assert!(Cli::try_parse_from(["clawg", "path"]).is_err());
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

    // ── memories ────────────────────────────────────────────────────────────

    fn memories(args: &[&str]) -> MemoriesAction {
        let mut argv = vec!["clawg", "memories"];
        argv.extend_from_slice(args);
        match parse(&argv).command {
            Command::Memories { action } => action,
            other => panic!("expected memories, got {other:?}"),
        }
    }

    #[test]
    fn memories_list_defaults() {
        assert_eq!(
            memories(&["list"]),
            MemoriesAction::List(ListOpts {
                kind: None,
                filters: Filters::default(),
                limit: 20,
                offset: 0,
                json: false,
                profile: ProfileArg { profile: None },
            })
        );
    }

    #[test]
    fn list_accepts_every_filter() {
        let action = memories(&[
            "list", "--kind", "episodic", "--agent", "a1", "--project", "p1", "--type",
            "semantic_fact", "--tier", "2", "--status", "disputed", "--limit", "5", "--offset",
            "10", "--json", "--profile", "dev",
        ]);
        assert_eq!(
            action,
            MemoriesAction::List(ListOpts {
                kind: Some(KindArg::Episodic),
                filters: Filters {
                    scope: Scope {
                        agent: Some("a1".into()),
                        project: Some("p1".into()),
                        here: false,
                    },
                    memory_type: Some("semantic_fact".into()),
                    tier: Some(2),
                    status: Some(StatusArg::Disputed),
                },
                limit: 5,
                offset: 10,
                json: true,
                profile: ProfileArg { profile: Some("dev".into()) },
            })
        );
    }

    #[test]
    fn here_is_a_flag_and_conflicts_with_project() {
        match memories(&["list", "--here"]) {
            MemoriesAction::List(opts) => assert!(opts.filters.scope.here),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn project_and_here_conflict() {
        for argv in [
            ["clawg", "memories", "list", "--project", "p", "--here"],
            ["clawg", "memories", "stats", "--here", "--project", "p"],
        ] {
            let err = Cli::try_parse_from(argv).expect_err("they name the project twice");
            assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
        }
    }

    #[test]
    fn scope_resolves_the_project_it_names_or_the_working_directory() {
        let cwd = || Some("claw-garden-1a2b".to_string());
        let named = Scope { project: Some("p1".into()), ..Scope::default() };
        assert_eq!(named.project_id(cwd()), Some("p1".to_string()));

        let here = Scope { here: true, ..Scope::default() };
        assert_eq!(here.project_id(cwd()), cwd());

        assert_eq!(Scope::default().project_id(cwd()), None, "no flag, no project");
    }

    #[test]
    fn search_requires_query() {
        assert!(Cli::try_parse_from(["clawg", "memories", "search"]).is_err());
        match memories(&["search", "deploy window", "--agent", "a1", "--limit", "3"]) {
            MemoriesAction::Search(opts) => {
                assert_eq!(opts.query, "deploy window");
                assert_eq!(opts.filters.scope.agent.as_deref(), Some("a1"));
                assert_eq!(opts.limit, 3);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn search_has_no_kind_flag() {
        assert!(Cli::try_parse_from(["clawg", "memories", "search", "x", "--kind", "memory"])
            .is_err());
    }

    #[test]
    fn show_requires_id_and_defaults_kind_memory() {
        assert!(Cli::try_parse_from(["clawg", "memories", "show"]).is_err());
        assert_eq!(
            memories(&["show", "57"]),
            MemoriesAction::Show(ShowOpts {
                id: 57,
                kind: KindArg::Memory,
                json: false,
                profile: ProfileArg { profile: None },
            })
        );
        match memories(&["show", "3", "--kind", "procedural", "--json"]) {
            MemoriesAction::Show(opts) => {
                assert_eq!((opts.id, opts.kind, opts.json), (3, KindArg::Procedural, true));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn stats_takes_a_scope_and_json() {
        assert_eq!(
            memories(&["stats", "--agent", "a1", "--json"]),
            MemoriesAction::Stats(StatsOpts {
                scope: Scope { agent: Some("a1".into()), project: None, here: false },
                json: true,
                profile: ProfileArg { profile: None },
            })
        );
    }

    #[test]
    fn tier_out_of_range_is_usage_error() {
        for tier in ["0", "4", "-1", "x"] {
            assert!(
                Cli::try_parse_from(["clawg", "memories", "list", "--tier", tier]).is_err(),
                "tier {tier} must be refused"
            );
        }
    }

    #[test]
    fn limit_outside_what_the_daemon_serves_is_usage_error() {
        for limit in ["0", "101"] {
            assert!(Cli::try_parse_from(["clawg", "memories", "list", "--limit", limit]).is_err());
        }
    }

    #[test]
    fn unknown_kind_and_status_are_usage_errors() {
        assert!(Cli::try_parse_from(["clawg", "memories", "list", "--kind", "bogus"]).is_err());
        assert!(Cli::try_parse_from(["clawg", "memories", "list", "--status", "stale"]).is_err());
    }

    #[test]
    fn every_memories_action_exposes_its_profile() {
        for argv in [
            &["list"][..],
            &["search", "q"],
            &["show", "1"],
            &["stats"],
        ] {
            let mut with_profile = argv.to_vec();
            with_profile.extend(["--profile", "dev"]);
            assert_eq!(memories(&with_profile).profile().profile.as_deref(), Some("dev"));
            assert_eq!(memories(argv).profile().profile, None);
        }
    }

    #[test]
    fn memories_without_action_is_usage_error() {
        assert!(Cli::try_parse_from(["clawg", "memories"]).is_err());
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
