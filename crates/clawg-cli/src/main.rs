//! The `clawg` binary.
//!
//! Parses, dispatches, and turns a failure into an exit code and a line on
//! stderr. Nothing else — every subcommand's behaviour lives in `clawg_cli`,
//! where it is testable without running this.
//!
//! Exit codes: `0` success, `1` a runtime failure (message on stderr), `2` a
//! usage error (clap's own, emitted before `main` ever runs).

use std::process::ExitCode;

use clawg_cli::cli::{Cli, Command};
use clawg_cli::client::{ClientError, DaemonClient};
use clawg_cli::daemon::lifecycle::read_state;
use clawg_cli::daemon::serve;
use clawg_cli::mcp;
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
    }
}
