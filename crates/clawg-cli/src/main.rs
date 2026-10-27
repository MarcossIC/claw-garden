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

use clawg_cli::cli::{
    is_bare_invocation, is_hook_invocation, preferences_profile, sharing_profile, Cli, Command, MetricsAction, PathAction,
    ProjectsAction, SetupClient,
};
use clawg_cli::client::{probe_running, ClientError, DaemonClient, Probe};
use clawg_cli::daemon::lifecycle::{now_ms, read_state};
use clawg_cli::daemon::serve;
use clawg_cli::doctor;
use clawg_cli::export;
use clawg_cli::hook;
use clawg_cli::import;
use clawg_cli::mcp;
use clawg_cli::memories;
use clawg_cli::metrics;
use clawg_cli::outbox;
use clawg_cli::projects;
use clawg_cli::restore;
use clawg_cli::secrets;
use clawg_cli::setup;
use clawg_cli::preferences;
use clawg_cli::sharing;
use clawg_cli::status::status_report;
use clawg_cli::stop::{stop_in, STOP_TIMEOUT};
use clawg_cli::undelete;
use clawg_cli::update;
use clawg_core::memory::backup::{backup_offline, probe, restore};
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
            serve::run(profile, port).await.map_err(|e| e.to_string())?;
            Ok(ExitCode::SUCCESS)
        }

        Command::Hook { event, agent, profile } => {
            let profile = profile.resolve().map_err(|e| e.to_string())?;
            let base_dir = data_dir(profile).map_err(|e| e.to_string())?;
            if let Some(out) = hook::respond(event, &agent, &base_dir, tokio::io::stdin(), hook::HOOK_DEADLINE).await {
                println!("{out}");
            }
            Ok(ExitCode::SUCCESS)
        }

        Command::Mcp { profile, no_autostart, agent } => {
            let profile = profile.resolve().map_err(|e| e.to_string())?;
            mcp::run(profile, !no_autostart, agent).await.map_err(|e| e.to_string())?;
            Ok(ExitCode::SUCCESS)
        }

        Command::Status { profile } => {
            let profile = profile.resolve().map_err(|e| e.to_string())?;
            let base_dir = data_dir(profile).map_err(|e| e.to_string())?;
            let state = read_state(&base_dir);

            // A state file proves only that a daemon once ran here. Liveness is
            // whatever `/health` says, and nothing else.
            let health = match DaemonClient::from_data_dir(&base_dir) {
                Some(client) => client.health().await.ok(),
                None => None,
            };

            let running = health.is_some();
            // Only without a daemon: a running one has already opened the
            // store, and its health says how that went.
            let store = if running { None } else { Some(probe(&base_dir)) };
            let report =
                status_report(profile, &base_dir, state.as_ref(), health.as_ref(), store.as_ref());
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
            let base_dir = data_dir(profile).map_err(|e| e.to_string())?;
            let lang = language(&base_dir);
            let name = profile.as_str();
            let pid = stop_in(&base_dir, STOP_TIMEOUT).await.map_err(|e| match (e, lang) {
                (ClientError::NotRunning, Lang::En) => format!("no clawg daemon is running for profile {name}"),
                (ClientError::NotRunning, Lang::Es) => {
                    format!("no hay ningún daemon de clawg corriendo para el perfil {name}")
                }
                (other, _) => other.to_string(),
            })?;
            match lang {
                Lang::En => println!("stopped the {name} daemon (pid {pid})"),
                Lang::Es => println!("se detuvo el daemon {name} (pid {pid})"),
            }
            Ok(ExitCode::SUCCESS)
        }

        Command::Backup { profile } => {
            let profile = profile.resolve().map_err(|e| e.to_string())?;
            let base_dir = data_dir(profile).map_err(|e| e.to_string())?;
            // A running daemon holds the store and its only connection, so the
            // snapshot has to come from it; with none, no daemon is started
            // just to copy a file.
            let path = match probe_running(&base_dir, false).await {
                Ok(Probe::Running(client)) => client.backup().await.map_err(|e| e.to_string())?,
                Ok(Probe::NeedsStart) | Err(ClientError::NotRunning) => {
                    backup_offline(&base_dir).map_err(|e| e.to_string())?
                }
                Err(e) => return Err(e.to_string()),
            };
            println!("{}", path.display());
            Ok(ExitCode::SUCCESS)
        }

        Command::Restore { file, profile } => {
            let profile = profile.resolve().map_err(|e| e.to_string())?;
            let base_dir = data_dir(profile).map_err(|e| e.to_string())?;
            let lang = language(&base_dir);
            let restored = restore(&base_dir, &file).map_err(|e| restore::explain(lang, e))?;
            println!("{}", restore::report(lang, &file, &restored));
            Ok(ExitCode::SUCCESS)
        }

        Command::Export { out, project, agent, include_deleted, format, profile } => {
            let profile = profile.resolve().map_err(|e| e.to_string())?;
            let base_dir = data_dir(profile).map_err(|e| e.to_string())?;
            let project_id = match project {
                Some(path) => {
                    let identity = ProjectIdentity::resolve(&path);
                    Some(project_id_of(&base_dir, &identity.root, Some(&identity.path), &Disk).map_err(|e| e.to_string())?)
                }
                None => None,
            };
            let filter = ExportFilter { project_id, agent, include_deleted };
            println!("{}", export::run(language(&base_dir), &base_dir, &out, &filter, format, now_ms() as i64)?);
            Ok(ExitCode::SUCCESS)
        }

        Command::Import { file, undo, from, dry_run, agent, map_root, project_map, profile } => {
            let profile = profile.resolve().map_err(|e| e.to_string())?;
            // The daemon is the store's only writer and has the model that
            // embeds each row; with none running, nothing is started.
            let base_dir = data_dir(profile).map_err(|e| e.to_string())?;
            let lang = language(&base_dir);
            let unavailable = |e: ClientError| memories::explain_unavailable(lang, &e, profile);
            let client = match probe_running(&base_dir, false).await.map_err(unavailable)? {
                Probe::Running(client) => client,
                Probe::NeedsStart => return Err(unavailable(ClientError::NotRunning)),
            };
            let maps = import::Maps { roots: &map_root, projects: &project_map };
            let out = match (undo, file) {
                (Some(n), _) => import::undo(lang, &client, n, dry_run).await,
                (None, Some(file)) => import::run(lang, &client, &file, from, dry_run, agent.as_deref(), maps).await,
                (None, None) => return Err("name the file to import, or --undo N".into()),
            }
            .map_err(|e| match e {
                import::ImportError::Client(e) => unavailable(e),
                other => other.to_string(),
            })?;
            println!("{out}");
            Ok(ExitCode::SUCCESS)
        }

        Command::Secrets { mask_reasons: true, yes, profile } => {
            let profile = profile.resolve().map_err(|e| e.to_string())?;
            let base_dir = data_dir(profile).map_err(|e| e.to_string())?;
            let lang = language(&base_dir);
            secrets::confirm_mask_reasons(lang, yes)?;
            let masked =
                mask_reasons(&base_dir).map_err(|e| secrets::explain(lang, e, secrets::Rewrite::MaskReasons))?;
            println!("{}", secrets::mask_report(lang, &masked));
            Ok(ExitCode::SUCCESS)
        }

        Command::Secrets { mask_reasons: false, profile, .. } => {
            let profile = profile.resolve().map_err(|e| e.to_string())?;
            let base_dir = data_dir(profile).map_err(|e| e.to_string())?;
            let scanned = scan(&base_dir).map_err(|e| e.to_string())?;
            println!("{}", secrets::report(language(&base_dir), &scanned));
            Ok(ExitCode::SUCCESS)
        }

        Command::Purge { id, project_delete, yes, profile } => {
            let profile = profile.resolve().map_err(|e| e.to_string())?;
            let base_dir = data_dir(profile).map_err(|e| e.to_string())?;
            let lang = language(&base_dir);
            match (id, project_delete) {
                (Some(id), None) => {
                    secrets::confirm(lang, id, yes)?;
                    let purged = purge(&base_dir, id).map_err(|e| secrets::explain(lang, e, secrets::Rewrite::Purge))?;
                    println!("{}", secrets::purge_report(lang, id, &purged));
                }
                (None, Some(delete_id)) => {
                    secrets::confirm_delete(lang, delete_id, yes)?;
                    let purged =
                        purge_delete(&base_dir, delete_id).map_err(|e| secrets::explain(lang, e, secrets::Rewrite::Purge))?;
                    println!("{}", secrets::purge_delete_report(lang, delete_id, &purged));
                }
                _ => return Err("name a memory to purge, or a delete with --project-delete".to_string()),
            }
            Ok(ExitCode::SUCCESS)
        }

        Command::Undelete { id, profile } => {
            let profile = profile.resolve().map_err(|e| e.to_string())?;
            // The model that embeds the row again lives in the daemon; with
            // none running, nothing is started and the store is not opened.
            let base_dir = data_dir(profile).map_err(|e| e.to_string())?;
            let lang = language(&base_dir);
            let unavailable = |e: ClientError| memories::explain_unavailable(lang, &e, profile);
            let client = match probe_running(&base_dir, false).await.map_err(unavailable)? {
                Probe::Running(client) => client,
                Probe::NeedsStart => return Err(unavailable(ClientError::NotRunning)),
            };
            println!("{}", undelete::run(lang, &client, id).await.map_err(unavailable)?);
            Ok(ExitCode::SUCCESS)
        }

        Command::Outbox { action } => {
            let profile = action.target().profile.resolve().map_err(|e| e.to_string())?;
            let base_dir = data_dir(profile).map_err(|e| e.to_string())?;
            let lang = language(&base_dir);
            let unavailable = |e: ClientError| memories::explain_unavailable(lang, &e, profile);
            let client = match probe_running(&base_dir, false).await.map_err(unavailable)? {
                Probe::Running(client) => client,
                Probe::NeedsStart => return Err(unavailable(ClientError::NotRunning)),
            };
            println!("{}", outbox::run(lang, &client, &action).await.map_err(unavailable)?);
            Ok(ExitCode::SUCCESS)
        }

        Command::Sharing { action, profile } => {
            let profile = sharing_profile(action.as_ref(), &profile).resolve().map_err(|e| e.to_string())?;
            let base_dir = data_dir(profile).map_err(|e| e.to_string())?;
            println!("{}", sharing::run(language(&base_dir), &base_dir, action.as_ref())?);
            Ok(ExitCode::SUCCESS)
        }

        Command::Preferences { action, profile } => {
            let profile = preferences_profile(action.as_ref(), &profile).resolve().map_err(|e| e.to_string())?;
            let base_dir = data_dir(profile).map_err(|e| e.to_string())?;
            println!("{}", preferences::run(&base_dir, action.as_ref())?);
            Ok(ExitCode::SUCCESS)
        }

        Command::Doctor { profile } => {
            let profile = profile.resolve().map_err(|e| e.to_string())?;
            let report = doctor::run(profile).map_err(|e| e.to_string())?;
            println!(
                "{}",
                serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
            );
            Ok(if !report.summary.fails() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(EXIT_FAILURE)
            })
        }

        Command::Update { check, profile } => {
            let profile = profile.resolve().map_err(|e| e.to_string())?;
            let (text, code) = update::run(check, profile).await.map_err(|e| e.to_string())?;
            println!("{text}");
            Ok(ExitCode::from(code))
        }

        Command::Metrics { action: MetricsAction::Report { metric, labels, by, since, profile } } => {
            let profile = profile.resolve().map_err(|e| e.to_string())?;
            let base_dir = data_dir(profile).map_err(|e| e.to_string())?;
            println!("{}", metrics::report(language(&base_dir), &base_dir, &metric, &labels, by, since, now_ms() as i64)?);
            Ok(ExitCode::SUCCESS)
        }

        Command::Projects { action: ProjectsAction::List { profile } } => {
            let profile = profile.resolve().map_err(|e| e.to_string())?;
            let base_dir = data_dir(profile).map_err(|e| e.to_string())?;
            let lang = language(&base_dir);
            let unavailable = |e: ClientError| memories::explain_unavailable(lang, &e, profile);
            let client = match probe_running(&base_dir, false).await.map_err(unavailable)? {
                Probe::Running(client) => client,
                Probe::NeedsStart => return Err(unavailable(ClientError::NotRunning)),
            };
            println!("{}", projects::list(lang, &client).await.map_err(unavailable)?);
            Ok(ExitCode::SUCCESS)
        }

        Command::Projects { action: ProjectsAction::Rename { id, name, profile } } => {
            let profile = profile.resolve().map_err(|e| e.to_string())?;
            let base_dir = data_dir(profile).map_err(|e| e.to_string())?;
            let lang = language(&base_dir);
            let unavailable = |e: ClientError| memories::explain_unavailable(lang, &e, profile);
            let client = match probe_running(&base_dir, false).await.map_err(unavailable)? {
                Probe::Running(client) => client,
                Probe::NeedsStart => return Err(unavailable(ClientError::NotRunning)),
            };
            let renamed = projects::rename(lang, &client, &id, &name)
                .await
                .map_err(|e| projects::rename_refused(lang, &id, &e).unwrap_or_else(|| unavailable(e)))?;
            println!("{renamed}");
            Ok(ExitCode::SUCCESS)
        }

        Command::Projects { action: ProjectsAction::Create { name, new, profile } } => {
            let profile = profile.resolve().map_err(|e| e.to_string())?;
            let base_dir = data_dir(profile).map_err(|e| e.to_string())?;
            let lang = language(&base_dir);
            let unavailable = |e: ClientError| memories::explain_unavailable(lang, &e, profile);
            let client = match probe_running(&base_dir, false).await.map_err(unavailable)? {
                Probe::Running(client) => client,
                Probe::NeedsStart => return Err(unavailable(ClientError::NotRunning)),
            };
            let (said, done) = projects::create(lang, &client, &name, new)
                .await
                .map_err(|e| projects::create_refused(lang, &e).unwrap_or_else(|| unavailable(e)))?;
            println!("{said}");
            Ok(if done { ExitCode::SUCCESS } else { ExitCode::FAILURE })
        }

        Command::Projects { action: ProjectsAction::Locate { project, path, profile } } => {
            let profile = profile.resolve().map_err(|e| e.to_string())?;
            let base_dir = data_dir(profile).map_err(|e| e.to_string())?;
            let lang = language(&base_dir);
            let unavailable = |e: ClientError| memories::explain_unavailable(lang, &e, profile);
            let client = match probe_running(&base_dir, false).await.map_err(unavailable)? {
                Probe::Running(client) => client,
                Probe::NeedsStart => return Err(unavailable(ClientError::NotRunning)),
            };
            let located = projects::locate(lang, &client, &project, &path).await.map_err(|e| match e {
                projects::AddError::Client(e) => projects::locate_refused(lang, &e).unwrap_or_else(|| unavailable(e)),
                other => other.to_string(),
            })?;
            println!("{located}");
            Ok(ExitCode::SUCCESS)
        }

        Command::Projects { action: ProjectsAction::Add { path, profile } } => {
            let profile = profile.resolve().map_err(|e| e.to_string())?;
            // The daemon owns the store and does the indexing; with none
            // running, nothing is started and the store is not opened.
            let base_dir = data_dir(profile).map_err(|e| e.to_string())?;
            let lang = language(&base_dir);
            let unavailable = |e: ClientError| memories::explain_unavailable(lang, &e, profile);
            let client = match probe_running(&base_dir, false).await.map_err(unavailable)? {
                Probe::Running(client) => client,
                Probe::NeedsStart => return Err(unavailable(ClientError::NotRunning)),
            };
            let added = projects::add(lang, &client, &path).await.map_err(|e| match e {
                projects::AddError::Client(e) => unavailable(e),
                other => other.to_string(),
            })?;
            println!("{added}");
            Ok(ExitCode::SUCCESS)
        }

        Command::Projects { action: ProjectsAction::Move(opts) } => {
            let profile = opts.profile.resolve().map_err(|e| e.to_string())?;
            let base_dir = data_dir(profile).map_err(|e| e.to_string())?;
            let lang = language(&base_dir);
            let unavailable = |e: ClientError| memories::explain_unavailable(lang, &e, profile);
            let client = match probe_running(&base_dir, false).await.map_err(unavailable)? {
                Probe::Running(client) => client,
                Probe::NeedsStart => return Err(unavailable(ClientError::NotRunning)),
            };
            let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
            let moved = projects::move_memories(lang, &client, &opts, &cwd).await.map_err(|e| match e {
                projects::AddError::Client(e) => projects::move_refused(lang, &e).unwrap_or_else(|| unavailable(e)),
                other => other.to_string(),
            })?;
            println!("{moved}");
            Ok(ExitCode::SUCCESS)
        }

        Command::Projects { action: ProjectsAction::Merge(opts) } => {
            let profile = opts.profile.resolve().map_err(|e| e.to_string())?;
            let base_dir = data_dir(profile).map_err(|e| e.to_string())?;
            let lang = language(&base_dir);
            let unavailable = |e: ClientError| memories::explain_unavailable(lang, &e, profile);
            let client = match probe_running(&base_dir, false).await.map_err(unavailable)? {
                Probe::Running(client) => client,
                Probe::NeedsStart => return Err(unavailable(ClientError::NotRunning)),
            };
            let merged = projects::merge(lang, &client, &opts)
                .await
                .map_err(|e| projects::merge_refused(lang, &e).unwrap_or_else(|| unavailable(e)))?;
            println!("{merged}");
            Ok(ExitCode::SUCCESS)
        }

        Command::Projects { action: ProjectsAction::Delete(opts) } => {
            let profile = opts.profile.resolve().map_err(|e| e.to_string())?;
            let base_dir = data_dir(profile).map_err(|e| e.to_string())?;
            let lang = language(&base_dir);
            let unavailable = |e: ClientError| memories::explain_unavailable(lang, &e, profile);
            let client = match probe_running(&base_dir, false).await.map_err(unavailable)? {
                Probe::Running(client) => client,
                Probe::NeedsStart => return Err(unavailable(ClientError::NotRunning)),
            };
            let undo = opts.undo.is_some();
            let (said, done) = projects::delete(lang, &client, &opts)
                .await
                .map_err(|e| projects::delete_refused(lang, undo, &e).unwrap_or_else(|| unavailable(e)))?;
            println!("{said}");
            Ok(if done { ExitCode::SUCCESS } else { ExitCode::FAILURE })
        }

        Command::Projects { action: ProjectsAction::KeepApart { project_id, other_id, profile } } => {
            let profile = profile.resolve().map_err(|e| e.to_string())?;
            let base_dir = data_dir(profile).map_err(|e| e.to_string())?;
            let lang = language(&base_dir);
            let unavailable = |e: ClientError| memories::explain_unavailable(lang, &e, profile);
            let client = match probe_running(&base_dir, false).await.map_err(unavailable)? {
                Probe::Running(client) => client,
                Probe::NeedsStart => return Err(unavailable(ClientError::NotRunning)),
            };
            let kept = projects::keep_apart(lang, &client, &project_id, &other_id)
                .await
                .map_err(|e| projects::keep_apart_refused(lang, &e).unwrap_or_else(|| unavailable(e)))?;
            println!("{kept}");
            Ok(ExitCode::SUCCESS)
        }

        Command::Memories { action } => {
            let profile = action.profile().resolve().map_err(|e| e.to_string())?;

            // No autostart: reading must not launch a process, and a daemon
            // the user did not ask for would also hold the store lock.
            let base_dir = data_dir(profile).map_err(|e| e.to_string())?;
            let lang = language(&base_dir);
            let unavailable = |e: ClientError| memories::explain_unavailable(lang, &e, profile);
            let client = match probe_running(&base_dir, false)
                .await
                .map_err(unavailable)?
            {
                Probe::Running(client) => client,
                Probe::NeedsStart => return Err(unavailable(ClientError::NotRunning)),
            };
            let out = memories::run(lang, &client, action, now_ms() as i64)
                .await
                .map_err(unavailable)?;
            println!("{out}");
            Ok(ExitCode::SUCCESS)
        }

        Command::Setup { client: SetupClient::ClaudeCode, dry_run, remove, profile } => {
            let profile = profile.resolve().map_err(|e| e.to_string())?;
            let mode = if remove { setup::Mode::Remove } else { setup::Mode::Install };
            let (text, code) = setup::run(profile, mode, dry_run).map_err(|e| e.to_string())?;
            println!("{text}");
            Ok(ExitCode::from(code))
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
