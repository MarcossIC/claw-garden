//! What `clawg update` tells the user once it is done or has failed, in the
//! user's language. The cause of a failure is shown as it came.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use clawg_core::preferences::Lang;

use super::plan::{Answer, Failure, Step, StoreState, Undo};
use super::version::Version;

/// A daemon of the new version that is running at the end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewDaemon {
    pub pid: u32,
    pub embedder: String,
}

/// What a finished update leaves behind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Updated {
    pub from: Version,
    pub to: Version,
    /// Where the previous binary was set aside.
    pub aside: PathBuf,
    /// The snapshot the new daemon wrote before migrating, if it migrated.
    pub pre_migration: Option<PathBuf>,
    /// `None` when no daemon was running before, so none was started.
    pub daemon: Option<NewDaemon>,
}

pub fn success(lang: Lang, updated: &Updated) -> String {
    let (from, to, aside) = (updated.from, updated.to, updated.aside.display());
    let mut lines = vec![match lang {
        Lang::En => format!("clawg updated from {from} to {to}."),
        Lang::Es => format!("clawg se actualizó de {from} a {to}."),
    }];
    lines.push(match lang {
        Lang::En => format!("The previous binary is kept as {aside}."),
        Lang::Es => format!("El binario anterior queda como {aside}."),
    });
    if let Some(snapshot) = &updated.pre_migration {
        let snapshot = snapshot.display();
        lines.push(match lang {
            Lang::En => format!("The store was migrated; a snapshot from before is at {snapshot}."),
            Lang::Es => format!("El store se migró; hay un snapshot de antes en {snapshot}."),
        });
        lines.push(
            match lang {
                Lang::En => "Going back to the previous version means restoring it, and loses what is saved after it.",
                Lang::Es => "Volver a la versión anterior es restaurarlo, y pierde lo que se guarde después.",
            }
            .to_string(),
        );
    }
    lines.push(match (&updated.daemon, lang) {
        (Some(d), Lang::En) => format!("The new daemon is running (pid {}, embedder {}).", d.pid, d.embedder),
        (Some(d), Lang::Es) => format!("El daemon nuevo está corriendo (pid {}, embedder {}).", d.pid, d.embedder),
        (None, Lang::En) => "No daemon was running, so none was started.".to_string(),
        (None, Lang::Es) => "No había ningún daemon corriendo, así que no se arrancó ninguno.".to_string(),
    });
    lines.push(
        match lang {
            Lang::En => "Editors that were open keep the previous bridge until they are restarted.",
            Lang::Es => "Los editores que estaban abiertos siguen con el puente anterior hasta que se reinicien.",
        }
        .to_string(),
    );
    lines.join("\n")
}

/// The facts a failure report needs besides the failure itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureContext {
    pub from: Version,
    pub to: Version,
    /// The installed binary's path.
    pub exe: PathBuf,
    /// Where the installed binary was set aside, if it was.
    pub aside: Option<PathBuf>,
    pub data_dir: PathBuf,
    /// The snapshot the new daemon wrote before migrating, if it did.
    pub pre_migration: Option<PathBuf>,
    pub had_daemon: bool,
    /// The daemon that answered once the plan had ended, whoever started it.
    pub answering: Option<Answer>,
}

pub fn failure(lang: Lang, failure: &Failure, context: &FailureContext) -> String {
    let (doing, cause) = (doing(lang, failure.step), &failure.cause);
    let mut lines = vec![match lang {
        Lang::En => format!("clawg update failed while {doing}: {cause}"),
        Lang::Es => format!("clawg update falló al {doing}: {cause}"),
    }];
    if let StoreState::RestoreFailed(why) = &failure.store {
        lines.extend(kept_new_binary(lang, why, context));
        return lines.join("\n");
    }

    let from = context.from;
    if failure.stuck.is_empty() {
        lines.push(match lang {
            Lang::En => format!("Everything was put back: clawg {from} is installed."),
            Lang::Es => format!("Todo volvió a su lugar: clawg {from} está instalado."),
        });
    } else {
        lines.push(match lang {
            Lang::En => format!("clawg {from} may not be fully back in place:"),
            Lang::Es => format!("puede que clawg {from} no haya vuelto del todo a su lugar:"),
        });
        for (undo, why) in &failure.stuck {
            let undoing = undoing(lang, *undo);
            lines.push(match lang {
                Lang::En => format!("  could not {undoing}: {why}"),
                Lang::Es => format!("  no se pudo {undoing}: {why}"),
            });
        }
    }
    if let StoreState::Restored { from, kept_as } = &failure.store {
        let from = from.display();
        lines.push(match lang {
            Lang::En => format!("The store was restored from {from}."),
            Lang::Es => format!("El store se restauró desde {from}."),
        });
        if let Some(kept) = kept_as {
            let kept = kept.display();
            lines.push(match lang {
                Lang::En => {
                    format!("What the new daemon had migrated is kept at {kept}; anything saved since the snapshot is in it.")
                }
                Lang::Es => {
                    format!("Lo que el daemon nuevo había migrado queda en {kept}; lo guardado después del snapshot está ahí.")
                }
            });
        }
    }
    lines.push(daemon_state(lang, failure, context.had_daemon));
    lines.join("\n")
}

fn daemon_state(lang: Lang, failure: &Failure, had_daemon: bool) -> String {
    let text = if !had_daemon {
        match lang {
            Lang::En => "No daemon was running, and none was started.",
            Lang::Es => "No había ningún daemon corriendo, y no se arrancó ninguno.",
        }
    } else if failure.undone.contains(&Undo::StartOldDaemon) {
        match lang {
            Lang::En => "Its daemon is running again.",
            Lang::Es => "Su daemon está corriendo de nuevo.",
        }
    } else if failure.stuck.iter().any(|(undo, _)| *undo == Undo::StartOldDaemon) {
        if failure.step == Step::StopDaemon {
            match lang {
                Lang::En => {
                    "Its daemon was asked to stop and had not ended: once it has, start it with `clawg serve` \
                     or by reconnecting the editor's MCP server."
                }
                Lang::Es => {
                    "A su daemon se le pidió que parara y todavía no había terminado: cuando termine, arráncalo con \
                     `clawg serve` o reconectando el servidor MCP del editor."
                }
            }
        } else {
            match lang {
                Lang::En => "Its daemon is not running: start it with `clawg serve` or by reconnecting the editor's MCP server.",
                Lang::Es => {
                    "Su daemon no está corriendo: arráncalo con `clawg serve` o reconectando el servidor MCP del editor."
                }
            }
        }
    } else {
        match lang {
            Lang::En => "Its daemon was not touched.",
            Lang::Es => "Su daemon no se tocó.",
        }
    };
    text.to_string()
}

/// The update could not give the store back, so the new binary stays.
fn kept_new_binary(lang: Lang, restore_error: &str, context: &FailureContext) -> Vec<String> {
    let to = context.to;
    let mut lines = vec![
        match lang {
            Lang::En => format!("clawg {to} stays installed: it is the only version that opens the store it migrated."),
            Lang::Es => format!("clawg {to} queda instalado: es la única versión que abre el store que migró."),
        },
        match lang {
            Lang::En => format!("Restoring the store from before the migration failed: {restore_error}"),
            Lang::Es => format!("Falló la restauración del store de antes de la migración: {restore_error}"),
        },
        match (&context.answering, lang) {
            (Some(d), Lang::En) => format!("A daemon is running: pid {}, clawg {}.", d.pid, d.version),
            (Some(d), Lang::Es) => format!("Hay un daemon corriendo: pid {}, clawg {}.", d.pid, d.version),
            (None, Lang::En) => format!(
                "No daemon is running: start it with `clawg serve` or by reconnecting the editor's MCP server; \
                 clawg {to} opens the migrated store."
            ),
            (None, Lang::Es) => format!(
                "No hay ningún daemon corriendo: arráncalo con `clawg serve` o reconectando el servidor MCP del editor; \
                 clawg {to} abre el store migrado."
            ),
        },
        match lang {
            Lang::En => "Where everything is:",
            Lang::Es => "Dónde está cada cosa:",
        }
        .to_string(),
    ];
    let (binary, snapshot, migrated, pre_restore, not_found) = match lang {
        Lang::En => (
            "previous binary",
            "snapshot from before the migration",
            "migrated store",
            "any pre-restore snapshot",
            "not found",
        ),
        Lang::Es => (
            "binario anterior",
            "snapshot de antes de la migración",
            "store migrado",
            "el snapshot pre-restore, si hay",
            "no se encontró",
        ),
    };
    match &context.aside {
        Some(aside) => lines.push(format!("  {binary}: {}", aside.display())),
        None => lines.push(format!("  {binary}: {not_found}")),
    }
    match &context.pre_migration {
        Some(path) => lines.push(format!("  {snapshot}: {}", path.display())),
        None => lines.push(format!("  {snapshot}: {not_found}")),
    }
    lines.push(format!("  {migrated}: {}", context.data_dir.join("memory.db").display()));
    lines.push(format!("  {pre_restore}: {}", context.data_dir.join("backups").display()));
    lines.push(
        match lang {
            Lang::En => "To go back by hand:",
            Lang::Es => "Para volver atrás a mano:",
        }
        .to_string(),
    );
    let mut steps = vec![match lang {
        Lang::En => "clawg stop, if a daemon is running",
        Lang::Es => "clawg stop, si hay un daemon corriendo",
    }
    .to_string()];
    if let Some(aside) = &context.aside {
        let (aside, exe) = (aside.display(), context.exe.display());
        steps.push(match lang {
            Lang::En => format!("put {aside} back at {exe}"),
            Lang::Es => format!("poner {aside} de vuelta en {exe}"),
        });
    }
    if let Some(snapshot) = &context.pre_migration {
        steps.push(format!("clawg restore {}", snapshot.display()));
    }
    steps.push(
        match lang {
            Lang::En => "start the daemon again: `clawg serve`, or reconnect the editor's MCP server",
            Lang::Es => "arrancar el daemon de nuevo: `clawg serve`, o reconectar el servidor MCP del editor",
        }
        .to_string(),
    );
    lines.extend(steps.iter().enumerate().map(|(i, step)| format!("  {}. {step}", i + 1)));
    lines.push(
        match lang {
            Lang::En => "Restoring keeps the migrated store as a pre-restore snapshot, and loses nothing else.",
            Lang::Es => "Restaurar guarda el store migrado como un snapshot pre-restore, y no pierde nada más.",
        }
        .to_string(),
    );
    lines
}

fn doing(lang: Lang, step: Step) -> &'static str {
    match (lang, step) {
        (Lang::En, Step::Stage) => "downloading and verifying the new binary",
        (Lang::En, Step::Smoke) => "running the new binary",
        (Lang::En, Step::StopDaemon) => "stopping the daemon",
        (Lang::En, Step::SetAside) => "setting the installed binary aside",
        (Lang::En, Step::PutInPlace) => "putting the new binary in place",
        (Lang::En, Step::StartDaemon) => "starting the new daemon",
        (Lang::Es, Step::Stage) => "bajar y verificar el binario nuevo",
        (Lang::Es, Step::Smoke) => "ejecutar el binario nuevo",
        (Lang::Es, Step::StopDaemon) => "detener el daemon",
        (Lang::Es, Step::SetAside) => "apartar el binario instalado",
        (Lang::Es, Step::PutInPlace) => "poner el binario nuevo en su lugar",
        (Lang::Es, Step::StartDaemon) => "arrancar el daemon nuevo",
    }
}

fn undoing(lang: Lang, undo: Undo) -> &'static str {
    match (lang, undo) {
        (Lang::En, Undo::RemoveStaged) => "remove the downloaded binary",
        (Lang::En, Undo::StartOldDaemon) => "start the previous daemon",
        (Lang::En, Undo::RestoreAside) => "put the previous binary back",
        (Lang::En, Undo::TakeBackNew) => "take the new binary out",
        (Lang::En, Undo::EndNewDaemon) => "end the new daemon",
        (Lang::Es, Undo::RemoveStaged) => "borrar el binario bajado",
        (Lang::Es, Undo::StartOldDaemon) => "arrancar el daemon anterior",
        (Lang::Es, Undo::RestoreAside) => "volver a poner el binario anterior",
        (Lang::Es, Undo::TakeBackNew) => "sacar el binario nuevo",
        (Lang::Es, Undo::EndNewDaemon) => "terminar el daemon nuevo",
    }
}

/// Decides, from the result of creating a file beside the installed binary,
/// whether the update can go on. Creating it is the first thing `update` does,
/// before any download, so a directory the user cannot write to costs nothing.
/// It is the cause of a failed step, so it stays in English like the others.
pub fn install_dir_access(dir: &Path, created: Result<(), ErrorKind>) -> Result<(), String> {
    match created {
        Ok(()) => Ok(()),
        Err(ErrorKind::PermissionDenied) => Err(format!(
            "cannot write to {}, where clawg is installed: run the update as a user that can, \
             or install a release there some other way. clawg does not use sudo",
            dir.display()
        )),
        Err(kind) => Err(format!("cannot write to {}, where clawg is installed: {kind}", dir.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clawg_core::preferences::Lang;

    fn version(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    fn updated() -> Updated {
        Updated {
            from: version("0.18.0"),
            to: version("0.19.0"),
            aside: "/bin/clawg.old-20261012175116123".into(),
            pre_migration: Some("/data/backups/memory-2026-10-12T17-51-16.123Z-pre-migration.db".into()),
            daemon: Some(NewDaemon { pid: 4242, embedder: "ready".into() }),
        }
    }

    #[test]
    fn the_update_report_names_versions_snapshot_and_restart_notice() {
        let report = success(Lang::En, &updated());

        assert!(report.contains("from 0.18.0 to 0.19.0"), "{report}");
        assert!(report.contains("/data/backups/memory-2026-10-12T17-51-16.123Z-pre-migration.db"), "{report}");
        assert!(report.contains("loses what is saved after it"), "{report}");
        assert!(report.contains("/bin/clawg.old-20261012175116123"), "{report}");
        assert!(report.contains("pid 4242"), "{report}");
        assert!(report.contains("Editors that were open keep the previous bridge until they are restarted"), "{report}");
    }

    #[test]
    fn without_a_migration_there_is_no_snapshot_to_name() {
        let report = success(Lang::En, &Updated { pre_migration: None, ..updated() });

        assert!(!report.contains("snapshot"), "{report}");
        assert!(!report.contains("loses"), "{report}");
    }

    #[test]
    fn without_a_daemon_the_report_says_none_was_started() {
        let report = success(Lang::En, &Updated { daemon: None, ..updated() });

        assert!(report.contains("No daemon was running, so none was started"), "{report}");
        assert!(!report.contains("pid"), "{report}");
    }

    fn context() -> FailureContext {
        FailureContext {
            from: version("0.18.0"),
            to: version("0.19.0"),
            exe: "/bin/clawg".into(),
            aside: Some("/bin/clawg.old-1".into()),
            data_dir: "/data".into(),
            pre_migration: Some("/data/backups/pre-migration.db".into()),
            had_daemon: true,
            answering: None,
        }
    }

    fn failed(step: Step, cause: &str, undone: &[Undo]) -> Failure {
        Failure {
            step,
            cause: cause.into(),
            undone: undone.to_vec(),
            stuck: Vec::new(),
            skipped: Vec::new(),
            store: StoreState::Untouched,
        }
    }

    #[test]
    fn a_failure_before_the_stop_says_the_daemon_was_not_touched() {
        let report = failure(
            Lang::En,
            &failed(Step::Stage, "the download does not match SHA256SUMS", &[Undo::RemoveStaged]),
            &context(),
        );

        assert!(report.contains("while downloading and verifying the new binary"), "{report}");
        assert!(report.contains("does not match SHA256SUMS"), "{report}");
        assert!(report.contains("clawg 0.18.0 is installed"), "{report}");
        assert!(report.contains("Its daemon was not touched"), "{report}");
    }

    #[test]
    fn a_failure_after_the_stop_says_the_old_daemon_is_back() {
        let report = failure(
            Lang::En,
            &failed(
                Step::StartDaemon,
                "the new daemon exited with code 1",
                &[Undo::EndNewDaemon, Undo::TakeBackNew, Undo::RestoreAside, Undo::StartOldDaemon, Undo::RemoveStaged],
            ),
            &context(),
        );

        assert!(report.contains("exited with code 1"), "{report}");
        assert!(report.contains("Its daemon is running again"), "{report}");
    }

    #[test]
    fn a_failure_without_a_daemon_says_none_was_started() {
        let report = failure(
            Lang::En,
            &failed(Step::PutInPlace, "access denied", &[Undo::RestoreAside, Undo::RemoveStaged]),
            &FailureContext { had_daemon: false, ..context() },
        );

        assert!(report.contains("none was started"), "{report}");
    }

    #[test]
    fn a_restored_store_names_where_it_came_from_and_where_the_migrated_one_is_kept() {
        let mut restored = failed(Step::StartDaemon, "no answer", &[Undo::EndNewDaemon, Undo::StartOldDaemon]);
        restored.store = StoreState::Restored {
            from: "/data/backups/pre-migration.db".into(),
            kept_as: Some("/data/backups/pre-restore.db".into()),
        };

        let report = failure(Lang::En, &restored, &context());

        assert!(report.contains("restored from /data/backups/pre-migration.db"), "{report}");
        assert!(report.contains("kept at /data/backups/pre-restore.db"), "{report}");
    }

    #[test]
    fn an_undo_that_failed_is_named() {
        let mut stuck = failed(Step::StartDaemon, "no answer", &[Undo::EndNewDaemon]);
        stuck.stuck = vec![(Undo::RestoreAside, "access denied".into())];

        let report = failure(Lang::En, &stuck, &context());

        assert!(report.contains("could not put the previous binary back: access denied"), "{report}");
        assert!(!report.contains("Everything was put back"), "{report}");
    }

    #[test]
    fn a_failed_restore_explains_the_way_back() {
        let mut left = failed(Step::StartDaemon, "the new daemon stopped answering", &[Undo::EndNewDaemon]);
        left.store = StoreState::RestoreFailed("the snapshot is damaged".into());
        left.skipped = vec![Undo::TakeBackNew, Undo::RestoreAside, Undo::StartOldDaemon, Undo::RemoveStaged];

        let report = failure(Lang::En, &left, &context());

        for needed in [
            "the new daemon stopped answering",
            "the snapshot is damaged",
            "clawg 0.19.0 stays installed",
            "/bin/clawg.old-1",
            "/data/backups/pre-migration.db",
            "pre-restore",
            "No daemon is running: start it with `clawg serve` or by reconnecting the editor's MCP server",
            "1. clawg stop, if a daemon is running",
            "2. put /bin/clawg.old-1 back at /bin/clawg",
            "3. clawg restore /data/backups/pre-migration.db",
            "4. start the daemon again",
        ] {
            assert!(report.contains(needed), "missing {needed:?} in:\n{report}");
        }
        assert!(!report.contains("Everything was put back"), "{report}");
    }

    #[test]
    fn a_failed_restore_names_the_daemon_that_answers_and_numbers_only_the_steps_left() {
        let mut left = failed(Step::StartDaemon, "the new daemon stopped answering", &[Undo::EndNewDaemon]);
        left.store = StoreState::RestoreFailed("the store is in use".into());
        let context = FailureContext {
            aside: None,
            answering: Some(Answer { pid: 4343, version: "0.19.0".into() }),
            ..context()
        };

        let report = failure(Lang::En, &left, &context);

        assert!(report.contains("A daemon is running: pid 4343, clawg 0.19.0."), "{report}");
        assert!(!report.contains("No daemon is running"), "{report}");
        assert!(report.contains("2. clawg restore /data/backups/pre-migration.db"), "{report}");
        assert!(report.contains("3. start the daemon again"), "{report}");
    }

    #[test]
    fn a_daemon_that_could_not_be_started_again_says_what_starts_it() {
        let mut stuck = failed(Step::StartDaemon, "no answer", &[Undo::EndNewDaemon]);
        stuck.stuck = vec![(Undo::StartOldDaemon, "it did not answer".into())];

        let report = failure(Lang::En, &stuck, &context());

        assert!(
            report.contains("Its daemon is not running: start it with `clawg serve` or by reconnecting the editor's MCP server."),
            "{report}"
        );
    }

    #[test]
    fn a_daemon_still_shutting_down_says_how_to_start_it_once_it_ends() {
        let mut late = failed(
            Step::StopDaemon,
            "the daemon (pid 7) accepted the shutdown but is still running after 30s",
            &[Undo::RemoveStaged],
        );
        late.stuck = vec![(Undo::StartOldDaemon, "the daemon (pid 7) is still running".into())];

        let report = failure(Lang::En, &late, &context());

        assert!(
            report.contains(
                "Its daemon was asked to stop and had not ended: once it has, start it with `clawg serve` \
                 or by reconnecting the editor's MCP server."
            ),
            "{report}"
        );
        assert!(!report.contains("not touched"), "{report}");
    }

    #[test]
    fn an_unwritable_install_dir_fails_before_downloading() {
        let dir = Path::new("/usr/local/bin");

        assert_eq!(install_dir_access(dir, Ok(())), Ok(()));
        let denied = install_dir_access(dir, Err(ErrorKind::PermissionDenied)).unwrap_err();
        assert!(denied.contains("/usr/local/bin") && denied.contains("does not use sudo"), "{denied}");
        let other = install_dir_access(dir, Err(ErrorKind::ReadOnlyFilesystem)).unwrap_err();
        assert!(other.contains("/usr/local/bin"), "{other}");
        assert!(!other.contains("sudo"), "{other}");
    }

    #[test]
    fn the_update_reports_speak_spanish() {
        let report = success(Lang::Es, &updated());
        for needed in [
            "clawg se actualizó de 0.18.0 a 0.19.0.",
            "El binario anterior queda como /bin/clawg.old-20261012175116123.",
            "/data/backups/memory-2026-10-12T17-51-16.123Z-pre-migration.db",
            "pierde lo que se guarde después",
            "El daemon nuevo está corriendo (pid 4242, embedder ready).",
            "Los editores que estaban abiertos siguen con el puente anterior hasta que se reinicien.",
        ] {
            assert!(report.contains(needed), "missing {needed:?} in:\n{report}");
        }
        let none = success(Lang::Es, &Updated { daemon: None, pre_migration: None, ..updated() });
        assert!(none.contains("No había ningún daemon corriendo, así que no se arrancó ninguno."), "{none}");

        let report = failure(
            Lang::Es,
            &failed(Step::Stage, "the download does not match SHA256SUMS", &[Undo::RemoveStaged]),
            &context(),
        );
        assert!(
            report.starts_with("clawg update falló al bajar y verificar el binario nuevo: the download does not match SHA256SUMS"),
            "the cause goes as it is: {report}"
        );
        assert!(report.contains("Todo volvió a su lugar: clawg 0.18.0 está instalado."), "{report}");
        assert!(report.contains("Su daemon no se tocó."), "{report}");

        let mut stuck = failed(Step::StartDaemon, "no answer", &[Undo::EndNewDaemon]);
        stuck.stuck = vec![(Undo::RestoreAside, "access denied".into())];
        let report = failure(Lang::Es, &stuck, &context());
        assert!(report.contains("no se pudo volver a poner el binario anterior: access denied"), "{report}");

        let mut left = failed(Step::StartDaemon, "the new daemon stopped answering", &[Undo::EndNewDaemon]);
        left.store = StoreState::RestoreFailed("the snapshot is damaged".into());
        let report = failure(Lang::Es, &left, &context());
        for needed in [
            "clawg 0.19.0 queda instalado",
            "the snapshot is damaged",
            "1. clawg stop, si hay un daemon corriendo",
            "2. poner /bin/clawg.old-1 de vuelta en /bin/clawg",
            "3. clawg restore /data/backups/pre-migration.db",
            "4. arrancar el daemon de nuevo",
        ] {
            assert!(report.contains(needed), "missing {needed:?} in:\n{report}");
        }
    }
}
