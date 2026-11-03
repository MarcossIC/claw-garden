//! The `clawg` binary.
//!
//! Parses, dispatches, and turns a failure into an exit code and a line on
//! stderr. Nothing else — every subcommand's behaviour lives in `clawg_cli`,
//! where it is testable without running this.
//!
//! Exit codes: `0` success, `1` a runtime failure (message on stderr), `2` a
//! usage error (clap's own), except under `clawg hook` and for `clawg` alone,
//! where a usage error is `1`: a client reads `2` from a hook as "block", and a
//! client that ignores a hook's `args` runs the binary alone.

use std::process::ExitCode;

use clawg_cli::backup;
use clawg_cli::cli::{
    is_bare_invocation, is_hook_invocation, preferences_profile, sharing_profile, Cli, Command, MetricsAction,
    ProfileArg, SetupClient,
};
use clawg_cli::daemon::lifecycle::now_ms;
use clawg_cli::daemon::serve;
use clawg_cli::doctor;
use clawg_cli::error::CliError;
use clawg_cli::export;
use clawg_cli::hook;
use clawg_cli::import;
use clawg_cli::mcp;
use clawg_cli::memories;
use clawg_cli::metrics;
use clawg_cli::outbox;
use clawg_cli::projects;
use clawg_cli::restore;
use clawg_cli::running::Running;
use clawg_cli::secrets;
use clawg_cli::setup;
use clawg_cli::preferences::{self, language};
use clawg_cli::sharing;
use clawg_cli::status;
use clawg_cli::stop;
use clawg_cli::undelete;
use clawg_cli::update;
use clawg_cli::user_path;
use clawg_core::memory::embedder::keep_one_malloc_arena;
use clawg_core::paths::data_dir;

/// Exit code for "the command ran and failed", as opposed to clap's `2` for
/// "the command was never valid".
const EXIT_FAILURE: u8 = 1;

fn main() -> ExitCode {
    let hook = is_hook_invocation(std::env::args_os());
    let bare = is_bare_invocation(std::env::args_os());
    let cli = match Cli::parse_args(std::env::args_os()) {
        Ok(cli) => cli,
        // A client reads 2 from a hook as "block the session or the prompt".
        Err(e) if (hook || bare) && e.use_stderr() => {
            let _ = e.print();
            return ExitCode::from(EXIT_FAILURE);
        }
        Err(e) => e.exit(),
    };
    init_logging();
    let serves = matches!(cli.command, Command::Serve { .. });
    if cli.command.loads_the_model() {
        // Before the runtime exists: the allocator setting only reaches the
        // threads that have not allocated yet.
        keep_one_malloc_arena();
    }
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("clawg: cannot start the async runtime: {e}");
            return ExitCode::from(EXIT_FAILURE);
        }
    };

    let code = match runtime.block_on(dispatch(cli)) {
        Ok(code) => code,
        Err(message) => {
            // stderr, always: `clawg mcp` owns stdout as its protocol
            // transport, and the other subcommands print machine-readable JSON
            // there. An error on stdout would corrupt both.
            eprintln!("clawg: {message}");
            ExitCode::from(EXIT_FAILURE)
        }
    };
    if hook {
        // A stdin the client never closed still holds a blocking read, which
        // an ordinary drop of the runtime would wait for past the deadline.
        runtime.shutdown_background();
    } else if serves {
        // A model load still running on a blocking thread cannot be
        // cancelled, and an ordinary drop would wait for it past what
        // `clawg stop` waits for.
        runtime.shutdown_timeout(serve::RUNTIME_TEARDOWN);
    }
    code
}

/// The profile's daemon, found running, for a command that never starts one.
async fn running(profile: &ProfileArg) -> Result<Running, CliError> {
    let profile = profile.resolve()?;
    let base_dir = data_dir(profile)?;
    Running::find(profile, &base_dir, language(&base_dir)).await
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

async fn dispatch(cli: Cli) -> Result<ExitCode, CliError> {
    match cli.command {
        Command::Serve { profile, port } => {
            let profile = profile.resolve()?;
            serve::run(profile, port).await?;
            Ok(ExitCode::SUCCESS)
        }

        Command::Hook { event, agent, profile } => {
            let profile = profile.resolve()?;
            let base_dir = data_dir(profile)?;
            if let Some(out) = hook::respond(event, &agent, &base_dir, tokio::io::stdin(), hook::HOOK_DEADLINE).await {
                println!("{out}");
            }
            Ok(ExitCode::SUCCESS)
        }

        Command::Mcp { profile, no_autostart, agent } => {
            let profile = profile.resolve()?;
            mcp::run(profile, !no_autostart, agent).await?;
            Ok(ExitCode::SUCCESS)
        }

        Command::Status { profile } => {
            let profile = profile.resolve()?;
            let base_dir = data_dir(profile)?;
            let (text, code) = status::command(profile, &base_dir).await;
            println!("{text}");
            Ok(ExitCode::from(code))
        }

        Command::Stop { profile } => {
            let profile = profile.resolve()?;
            let base_dir = data_dir(profile)?;
            println!("{}", stop::command(language(&base_dir), profile, &base_dir).await?);
            Ok(ExitCode::SUCCESS)
        }

        Command::Backup { profile } => {
            let base_dir = data_dir(profile.resolve()?)?;
            println!("{}", backup::command(&base_dir).await?.display());
            Ok(ExitCode::SUCCESS)
        }

        Command::Restore { file, profile } => {
            let base_dir = data_dir(profile.resolve()?)?;
            println!("{}", restore::command(language(&base_dir), &base_dir, &file)?);
            Ok(ExitCode::SUCCESS)
        }

        Command::Export { out, project, agent, include_deleted, format, profile } => {
            let base_dir = data_dir(profile.resolve()?)?;
            println!("{}", export::command(&base_dir, &out, project.as_deref(), agent, include_deleted, format)?);
            Ok(ExitCode::SUCCESS)
        }

        Command::Import { file, undo, from, dry_run, follow, agent, map_root, project_map, profile } => {
            // The daemon is the store's only writer and has the model that
            // embeds each row; with none running, nothing is started.
            let daemon = running(&profile).await?;
            let maps = import::Maps { roots: &map_root, projects: &project_map };
            let finished =
                import::command(&daemon, file.as_deref(), undo, from, dry_run, follow, agent.as_deref(), maps).await?;
            if !finished.out.is_empty() {
                println!("{}", finished.out);
            }
            if let Some(note) = &finished.note {
                eprintln!("{note}");
            }
            Ok(if finished.ok { ExitCode::SUCCESS } else { ExitCode::from(EXIT_FAILURE) })
        }

        Command::Secrets { mask_reasons, yes, profile } => {
            let base_dir = data_dir(profile.resolve()?)?;
            println!("{}", secrets::command(&base_dir, mask_reasons, yes)?);
            Ok(ExitCode::SUCCESS)
        }

        Command::Purge { id, project_delete, yes, profile } => {
            let base_dir = data_dir(profile.resolve()?)?;
            println!("{}", secrets::purge(&base_dir, id, project_delete, yes)?);
            Ok(ExitCode::SUCCESS)
        }

        Command::Undelete { id, profile } => {
            // The model that embeds the row again lives in the daemon; with
            // none running, nothing is started and the store is not opened.
            let daemon = running(&profile).await?;
            println!("{}", undelete::run(daemon.lang, &daemon.client, id).await.map_err(|e| daemon.unavailable(e))?);
            Ok(ExitCode::SUCCESS)
        }

        Command::Outbox { action } => {
            let daemon = running(&action.target().profile).await?;
            println!("{}", outbox::run(daemon.lang, &daemon.client, &action).await.map_err(|e| daemon.unavailable(e))?);
            Ok(ExitCode::SUCCESS)
        }

        Command::Sharing { action, profile } => {
            let profile = sharing_profile(action.as_ref(), &profile).resolve()?;
            let base_dir = data_dir(profile)?;
            println!("{}", sharing::run(language(&base_dir), &base_dir, action.as_ref()).map_err(CliError::Failed)?);
            Ok(ExitCode::SUCCESS)
        }

        Command::Preferences { action, profile } => {
            let profile = preferences_profile(action.as_ref(), &profile).resolve()?;
            let base_dir = data_dir(profile)?;
            println!("{}", preferences::run(&base_dir, action.as_ref()).map_err(CliError::Failed)?);
            Ok(ExitCode::SUCCESS)
        }

        Command::Doctor { profile } => {
            let profile = profile.resolve()?;
            let report = doctor::run(profile)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&report)?
            );
            Ok(if !report.summary.fails() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(EXIT_FAILURE)
            })
        }

        Command::Update { check, profile } => {
            let profile = profile.resolve()?;
            let (text, code) = update::run(check, profile).await?;
            println!("{text}");
            Ok(ExitCode::from(code))
        }

        Command::Metrics { action } => {
            let MetricsAction::Report { profile, .. } = &action;
            let base_dir = data_dir(profile.resolve()?)?;
            println!("{}", metrics::command(language(&base_dir), &base_dir, &action, now_ms() as i64)?);
            Ok(ExitCode::SUCCESS)
        }

        Command::Projects { action } => {
            let daemon = running(action.profile()).await?;
            let (said, code) = projects::command(&daemon, action).await?;
            println!("{said}");
            Ok(ExitCode::from(code))
        }

        Command::Memories { action } => {
            // No autostart: reading must not launch a process, and a daemon
            // the user did not ask for would also hold the store lock.
            let daemon = running(action.profile()).await?;
            let out = memories::run(daemon.lang, &daemon.client, action, now_ms() as i64)
                .await
                .map_err(|e| daemon.unavailable(e))?;
            println!("{out}");
            Ok(ExitCode::SUCCESS)
        }

        Command::Setup { client: SetupClient::ClaudeCode, dry_run, remove, profile } => {
            let profile = profile.resolve()?;
            let mode = if remove { setup::Mode::Remove } else { setup::Mode::Install };
            let (text, code) = setup::run(profile, mode, dry_run)?;
            println!("{text}");
            Ok(ExitCode::from(code))
        }

        Command::Path { action } => {
            println!("{}", user_path::command(action)?);
            Ok(ExitCode::SUCCESS)
        }
    }
}
