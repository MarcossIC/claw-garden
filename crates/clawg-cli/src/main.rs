//! The `clawg` binary.
//!
//! Parses, dispatches, and turns a failure into an exit code and a line on
//! stderr. Nothing else — every subcommand's behaviour lives in `clawg_cli`,
//! where it is testable without running this.
//!
//! Exit codes: `0` success, `1` a runtime failure (message on stderr), `2` a
//! usage error (clap's own, emitted before `main` ever runs).

#[cfg(windows)]
use std::path::Path;
use std::process::ExitCode;

use clawg_cli::cli::{Cli, Command, PathAction};
use clawg_cli::client::{probe_running, ClientError, DaemonClient, Probe};
use clawg_cli::daemon::lifecycle::{now_ms, read_state};
use clawg_cli::daemon::serve;
use clawg_cli::mcp;
use clawg_cli::memories;
use clawg_cli::status::status_report;
use clawg_cli::stop::{stop_in, STOP_TIMEOUT};
use clawg_core::paths::data_dir;
use clap::Parser;

/// Exit code for "the command ran and failed", as opposed to clap's `2` for
/// "the command was never valid".
const EXIT_FAILURE: u8 = 1;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    init_logging();

    match dispatch(cli).await {
        Ok(code) => code,
        Err(message) => {
            // stderr, always: `clawg mcp` owns stdout as its protocol
            // transport, and the other subcommands print machine-readable JSON
            // there. An error on stdout would corrupt both.
            eprintln!("clawg: {message}");
            ExitCode::from(EXIT_FAILURE)
        }
    }
}

/// Logging to stderr, `info` by default, `RUST_LOG` overriding.
///
/// `Target::Stderr` is stated rather than relied on: it is env_logger's
/// default today, but `clawg mcp` breaks silently and confusingly if that ever
/// changes, so the requirement belongs in the code and not in a dependency's
/// defaults.
fn init_logging() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .target(env_logger::Target::Stderr)
        .init();
}

async fn dispatch(cli: Cli) -> Result<ExitCode, String> {
    match cli.command {
        Command::Serve { profile, port } => {
            let profile = profile.resolve().map_err(|e| e.to_string())?;
            let port = port.unwrap_or_else(|| profile.default_port());
            serve::run(profile, port).await.map_err(|e| e.to_string())?;
            Ok(ExitCode::SUCCESS)
        }

        Command::Mcp { profile, no_autostart } => {
            let profile = profile.resolve().map_err(|e| e.to_string())?;
            mcp::run(profile, !no_autostart).await.map_err(|e| e.to_string())?;
            Ok(ExitCode::SUCCESS)
        }

        Command::Status { profile } => {
            let profile = profile.resolve().map_err(|e| e.to_string())?;
            let base_dir = data_dir(profile);
            let state = read_state(&base_dir);

            // A state file proves only that a daemon once ran here. Liveness is
            // whatever `/health` says, and nothing else.
            let health = match DaemonClient::from_data_dir(&base_dir) {
                Some(client) => client.health().await.ok(),
                None => None,
            };

            let running = health.is_some();
            let report = status_report(profile, &base_dir, state.as_ref(), health.as_ref());
            println!(
                "{}",
                serde_json::to_string_pretty(&report).unwrap_or_else(|_| report.to_string())
            );

            // A non-zero exit is what makes this usable from a shell script:
            // `clawg status >/dev/null || clawg serve &`.
            Ok(if running { ExitCode::SUCCESS } else { ExitCode::from(EXIT_FAILURE) })
        }

        Command::Stop { profile } => {
            let profile = profile.resolve().map_err(|e| e.to_string())?;
            // Returns only once the process has ended, so `clawg stop && clawg
            // serve` cannot lose the race for the lock, and an installer can
            // replace `clawg.exe` right after.
            let pid = stop_in(&data_dir(profile), STOP_TIMEOUT)
                .await
                .map_err(|e| match e {
                    ClientError::NotRunning => {
                        format!("no clawg daemon is running for profile {}", profile.as_str())
                    }
                    other => other.to_string(),
                })?;
            println!("stopped the {} daemon (pid {pid})", profile.as_str());
            Ok(ExitCode::SUCCESS)
        }

        Command::Memories { action } => {
            let profile = action.profile().resolve().map_err(|e| e.to_string())?;
            let unavailable = |e: ClientError| memories::explain_unavailable(&e, profile);

            // No autostart: reading must not launch a process, and a daemon
            // the user did not ask for would also hold the store lock.
            let client = match probe_running(&data_dir(profile), false)
                .await
                .map_err(unavailable)?
            {
                Probe::Running(client) => client,
                Probe::NeedsStart => return Err(unavailable(ClientError::NotRunning)),
            };
            let out = memories::run(&client, action, now_ms() as i64)
                .await
                .map_err(unavailable)?;
            println!("{out}");
            Ok(ExitCode::SUCCESS)
        }

        Command::Path { action } => run_path(action),
    }
}

/// `clawg path add|remove`. Idempotent on purpose: the installer runs it on
/// every install and uninstall, and "already there" is success, not failure.
#[cfg(windows)]
fn run_path(action: PathAction) -> Result<ExitCode, String> {
    use clawg_cli::user_path::{registry, Change};

    let (dir, adding) = match action {
        PathAction::Add { dir } => (dir, true),
        PathAction::Remove { dir } => (dir, false),
    };
    let dir = match dir {
        Some(dir) => dir,
        None => std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf))
            .ok_or("cannot locate the directory of this clawg binary; pass --dir")?,
    };
    let dir = dir.to_str().ok_or("the directory is not valid Unicode")?;

    let change = if adding {
        registry::add(registry::ENVIRONMENT_KEY, dir)
    } else {
        registry::remove(registry::ENVIRONMENT_KEY, dir)
    }
    .map_err(|e| format!("could not update the user PATH: {e}"))?;

    if matches!(change, Change::Added | Change::Removed) {
        registry::broadcast_environment_change();
    }
    match change {
        Change::Added => println!("added {dir} to the user PATH; new terminals will find clawg"),
        Change::AlreadyPresent => println!("{dir} is already on the user PATH"),
        Change::Removed => println!("removed {dir} from the user PATH"),
        Change::NotPresent => println!("{dir} was not on the user PATH"),
    }
    Ok(ExitCode::SUCCESS)
}

#[cfg(not(windows))]
fn run_path(_action: PathAction) -> Result<ExitCode, String> {
    Err("`clawg path` manages the Windows user PATH; on this platform, add the \
         directory holding clawg to your shell profile instead"
        .to_string())
}
