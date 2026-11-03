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

#[cfg(windows)]
use std::path::Path;
use std::process::ExitCode;

use clawg_cli::backup;
use clawg_cli::cli::{
    is_bare_invocation, is_hook_invocation, preferences_profile, sharing_profile, Cli, Command, MetricsAction, PathAction,
    ProfileArg, ProjectsAction, SetupClient,
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
use clawg_cli::preferences;
use clawg_cli::sharing;
use clawg_cli::status;
use clawg_cli::stop;
use clawg_cli::undelete;
use clawg_cli::update;
use clawg_core::memory::backup::restore;
use clawg_core::memory::embedder::keep_one_malloc_arena;
use clawg_core::memory::identity::Disk;
use clawg_core::memory::portability::export::project_id_of;
use clawg_core::memory::portability::ExportFilter;
use clawg_core::memory::ProjectIdentity;
use clawg_core::memory::secrets::{mask_reasons, purge, purge_delete, scan};
use clawg_core::paths::data_dir;
use clawg_core::preferences::{language_in, Lang};

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

/// The language a command speaks to the user in: the profile's preference,
/// read once, and English without one.
fn language(base_dir: &std::path::Path) -> Lang {
    language_in(base_dir).unwrap_or(Lang::En)
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
            let profile = profile.resolve()?;
            let base_dir = data_dir(profile)?;
            let lang = language(&base_dir);
            let restored = restore(&base_dir, &file).map_err(|e| CliError::Failed(restore::explain(lang, e)))?;
            println!("{}", restore::report(lang, &file, &restored));
            Ok(ExitCode::SUCCESS)
        }

        Command::Export { out, project, agent, include_deleted, format, profile } => {
            let profile = profile.resolve()?;
            let base_dir = data_dir(profile)?;
            let project_id = match project {
                Some(path) => {
                    let identity = ProjectIdentity::resolve(&path);
                    Some(project_id_of(&base_dir, &identity.root, Some(&identity.path), &Disk)?)
                }
                None => None,
            };
            let filter = ExportFilter { project_id, agent, include_deleted };
            println!("{}", export::run(language(&base_dir), &base_dir, &out, &filter, format, now_ms() as i64).map_err(CliError::Failed)?);
            Ok(ExitCode::SUCCESS)
        }

        Command::Import { file, undo, from, dry_run, follow, agent, map_root, project_map, profile } => {
            // The daemon is the store's only writer and has the model that
            // embeds each row; with none running, nothing is started.
            let daemon = running(&profile).await?;
            let maps = import::Maps { roots: &map_root, projects: &project_map };
            let finished = match (undo, file) {
                (Some(n), _) => import::undo(daemon.lang, &daemon.client, n, dry_run).await.map(import::Finished::done),
                (None, Some(file)) => {
                    // Before the daemon is asked, so a Ctrl-C meanwhile waits for the job.
                    let mut interrupts = import::interrupts();
                    let watch = import::Watch::new(follow);
                    import::run(daemon.lang, &daemon.client, &file, from, dry_run, agent.as_deref(), maps, watch, &mut interrupts).await
                }
                (None, None) => return Err(CliError::Failed("name the file to import, or --undo N".into())),
            }
            .map_err(|e| match e {
                import::ImportError::Client(e) => daemon.unavailable(e),
                other => other.into(),
            })?;
            if !finished.out.is_empty() {
                println!("{}", finished.out);
            }
            if let Some(note) = &finished.note {
                eprintln!("{note}");
            }
            Ok(if finished.ok { ExitCode::SUCCESS } else { ExitCode::from(EXIT_FAILURE) })
        }

        Command::Secrets { mask_reasons: true, yes, profile } => {
            let profile = profile.resolve()?;
            let base_dir = data_dir(profile)?;
            let lang = language(&base_dir);
            secrets::confirm_mask_reasons(lang, yes).map_err(CliError::Failed)?;
            let masked =
                mask_reasons(&base_dir).map_err(|e| CliError::Failed(secrets::explain(lang, e, secrets::Rewrite::MaskReasons)))?;
            println!("{}", secrets::mask_report(lang, &masked));
            Ok(ExitCode::SUCCESS)
        }

        Command::Secrets { mask_reasons: false, profile, .. } => {
            let profile = profile.resolve()?;
            let base_dir = data_dir(profile)?;
            let scanned = scan(&base_dir)?;
            println!("{}", secrets::report(language(&base_dir), &scanned));
            Ok(ExitCode::SUCCESS)
        }

        Command::Purge { id, project_delete, yes, profile } => {
            let profile = profile.resolve()?;
            let base_dir = data_dir(profile)?;
            let lang = language(&base_dir);
            match (id, project_delete) {
                (Some(id), None) => {
                    secrets::confirm(lang, id, yes).map_err(CliError::Failed)?;
                    let purged = purge(&base_dir, id).map_err(|e| CliError::Failed(secrets::explain(lang, e, secrets::Rewrite::Purge)))?;
                    println!("{}", secrets::purge_report(lang, id, &purged));
                }
                (None, Some(delete_id)) => {
                    secrets::confirm_delete(lang, delete_id, yes).map_err(CliError::Failed)?;
                    let purged =
                        purge_delete(&base_dir, delete_id).map_err(|e| CliError::Failed(secrets::explain(lang, e, secrets::Rewrite::Purge)))?;
                    println!("{}", secrets::purge_delete_report(lang, delete_id, &purged));
                }
                _ => return Err(CliError::Failed("name a memory to purge, or a delete with --project-delete".to_string())),
            }
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

        Command::Metrics { action: MetricsAction::Report { metric, events, labels, by, since, profile } } => {
            let profile = profile.resolve()?;
            let base_dir = data_dir(profile)?;
            let (lang, now) = (language(&base_dir), now_ms() as i64);
            let printed = match metric {
                Some(metric) if !events => metrics::report(lang, &base_dir, &metric, &labels, by, since, now).map_err(CliError::Failed)?,
                _ => metrics::events(lang, &base_dir, since, now).map_err(CliError::Failed)?,
            };
            println!("{printed}");
            Ok(ExitCode::SUCCESS)
        }

        Command::Projects { action: ProjectsAction::List { profile } } => {
            let daemon = running(&profile).await?;
            println!("{}", projects::list(daemon.lang, &daemon.client).await.map_err(|e| daemon.unavailable(e))?);
            Ok(ExitCode::SUCCESS)
        }

        Command::Projects { action: ProjectsAction::Rename { id, name, profile } } => {
            let daemon = running(&profile).await?;
            let renamed = projects::rename(daemon.lang, &daemon.client, &id, &name)
                .await
                .map_err(|e| projects::rename_refused(daemon.lang, &id, &e).map(CliError::Failed).unwrap_or_else(|| daemon.unavailable(e)))?;
            println!("{renamed}");
            Ok(ExitCode::SUCCESS)
        }

        Command::Projects { action: ProjectsAction::Create { name, new, profile } } => {
            let daemon = running(&profile).await?;
            let (said, done) = projects::create(daemon.lang, &daemon.client, &name, new)
                .await
                .map_err(|e| projects::create_refused(daemon.lang, &e).map(CliError::Failed).unwrap_or_else(|| daemon.unavailable(e)))?;
            println!("{said}");
            Ok(if done { ExitCode::SUCCESS } else { ExitCode::FAILURE })
        }

        Command::Projects { action: ProjectsAction::Locate { project, path, subfolder, profile } } => {
            let daemon = running(&profile).await?;
            let located = projects::locate(daemon.lang, &daemon.client, &project, &path, subfolder).await.map_err(|e| match e {
                projects::AddError::Client(e) => projects::locate_refused(daemon.lang, &e).map(CliError::Failed).unwrap_or_else(|| daemon.unavailable(e)),
                other => other.into(),
            })?;
            println!("{located}");
            Ok(ExitCode::SUCCESS)
        }

        Command::Projects { action: ProjectsAction::Add { path, profile } } => {
            // The daemon owns the store and does the indexing; with none
            // running, nothing is started and the store is not opened.
            let daemon = running(&profile).await?;
            let added = projects::add(daemon.lang, &daemon.client, &path).await.map_err(|e| match e {
                projects::AddError::Client(e) => daemon.unavailable(e),
                other => other.into(),
            })?;
            println!("{added}");
            Ok(ExitCode::SUCCESS)
        }

        Command::Projects { action: ProjectsAction::Move(opts) } => {
            let daemon = running(&opts.profile).await?;
            let cwd = std::env::current_dir()?;
            let moved = projects::move_memories(daemon.lang, &daemon.client, &opts, &cwd).await.map_err(|e| match e {
                projects::AddError::Client(e) => projects::move_refused(daemon.lang, &e).map(CliError::Failed).unwrap_or_else(|| daemon.unavailable(e)),
                other => other.into(),
            })?;
            println!("{moved}");
            Ok(ExitCode::SUCCESS)
        }

        Command::Projects { action: ProjectsAction::Merge(opts) } => {
            let daemon = running(&opts.profile).await?;
            let merged = projects::merge(daemon.lang, &daemon.client, &opts)
                .await
                .map_err(|e| projects::merge_refused(daemon.lang, &e).map(CliError::Failed).unwrap_or_else(|| daemon.unavailable(e)))?;
            println!("{merged}");
            Ok(ExitCode::SUCCESS)
        }

        Command::Projects { action: ProjectsAction::Delete(opts) } => {
            let daemon = running(&opts.profile).await?;
            let undo = opts.undo.is_some();
            let (said, done) = projects::delete(daemon.lang, &daemon.client, &opts)
                .await
                .map_err(|e| projects::delete_refused(daemon.lang, undo, &e).map(CliError::Failed).unwrap_or_else(|| daemon.unavailable(e)))?;
            println!("{said}");
            Ok(if done { ExitCode::SUCCESS } else { ExitCode::FAILURE })
        }

        Command::Projects { action: ProjectsAction::KeepApart { project_id, other_id, profile } } => {
            let daemon = running(&profile).await?;
            let kept = projects::keep_apart(daemon.lang, &daemon.client, &project_id, &other_id)
                .await
                .map_err(|e| projects::keep_apart_refused(daemon.lang, &e).map(CliError::Failed).unwrap_or_else(|| daemon.unavailable(e)))?;
            println!("{kept}");
            Ok(ExitCode::SUCCESS)
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

        Command::Path { action } => run_path(action),
    }
}

/// `clawg path add|remove`. Idempotent on purpose: the installer runs it on
/// every install and uninstall, and "already there" is success, not failure.
#[cfg(windows)]
fn run_path(action: PathAction) -> Result<ExitCode, CliError> {
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
            .ok_or_else(|| CliError::Failed("cannot locate the directory of this clawg binary; pass --dir".into()))?,
    };
    let dir = dir.to_str().ok_or_else(|| CliError::Failed("the directory is not valid Unicode".into()))?;

    let change = if adding {
        registry::add(registry::ENVIRONMENT_KEY, dir)
    } else {
        registry::remove(registry::ENVIRONMENT_KEY, dir)
    }
    .map_err(|e| CliError::Failed(format!("could not update the user PATH: {e}")))?;

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
fn run_path(_action: PathAction) -> Result<ExitCode, CliError> {
    Err(CliError::Failed(
        "`clawg path` manages the Windows user PATH; on this platform, add the \
         directory holding clawg to your shell profile instead"
            .to_string(),
    ))
}
