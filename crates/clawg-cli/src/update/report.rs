//! What `clawg update` tells the user once it is done or has failed.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use super::plan::{Failure, Step, StoreState, Undo};
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

pub fn success(updated: &Updated) -> String {
    let mut lines = vec![format!("clawg updated from {} to {}.", updated.from, updated.to)];
    lines.push(format!("The previous binary is kept as {}.", updated.aside.display()));
    if let Some(snapshot) = &updated.pre_migration {
        lines.push(format!("The store was migrated; a snapshot from before is at {}.", snapshot.display()));
        lines.push(
            "Going back to the previous version means restoring it, and loses what is saved after it."
                .to_string(),
        );
    }
    match &updated.daemon {
        Some(daemon) => lines.push(format!(
            "The new daemon is running (pid {}, embedder {}).",
            daemon.pid, daemon.embedder
        )),
        None => lines.push("No daemon was running, so none was started.".to_string()),
    }
    lines.push("Editors that were open keep the previous bridge until they are restarted.".to_string());
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
}

pub fn failure(failure: &Failure, context: &FailureContext) -> String {
    let mut lines = vec![format!("clawg update failed while {}: {}", doing(failure.step), failure.cause)];
    if let StoreState::RestoreFailed(why) = &failure.store {
        lines.extend(kept_new_binary(why, context));
        return lines.join("\n");
    }

    if failure.stuck.is_empty() {
        lines.push(format!("Everything was put back: clawg {} is installed.", context.from));
    } else {
        lines.push(format!("clawg {} may not be fully back in place:", context.from));
        for (undo, why) in &failure.stuck {
            lines.push(format!("  could not {}: {why}", undoing(*undo)));
        }
    }
    if let StoreState::Restored { from, kept_as } = &failure.store {
        lines.push(format!("The store was restored from {}.", from.display()));
        if let Some(kept) = kept_as {
            lines.push(format!(
                "What the new daemon had migrated is kept at {}; anything saved since the snapshot is in it.",
                kept.display()
            ));
        }
    }
    lines.push(daemon_state(failure, context.had_daemon));
    lines.join("\n")
}

fn daemon_state(failure: &Failure, had_daemon: bool) -> String {
    if !had_daemon {
        "No daemon was running, and none was started.".to_string()
    } else if failure.undone.contains(&Undo::StartOldDaemon) {
        "Its daemon is running again.".to_string()
    } else if failure.stuck.iter().any(|(undo, _)| *undo == Undo::StartOldDaemon) {
        "Its daemon is not running: start it with any clawg command that needs it, or `clawg serve`."
            .to_string()
    } else {
        "Its daemon was not touched.".to_string()
    }
}

/// The update could not give the store back, so the new binary stays.
fn kept_new_binary(restore_error: &str, context: &FailureContext) -> Vec<String> {
    let mut lines = vec![
        format!(
            "clawg {} stays installed: it is the only version that opens the store it migrated.",
            context.to
        ),
        format!("Restoring the store from before the migration failed: {restore_error}"),
        "Where everything is:".to_string(),
    ];
    match &context.aside {
        Some(aside) => lines.push(format!("  previous binary: {}", aside.display())),
        None => lines.push("  previous binary: not found".to_string()),
    }
    match &context.pre_migration {
        Some(snapshot) => lines.push(format!("  snapshot from before the migration: {}", snapshot.display())),
        None => lines.push("  snapshot from before the migration: not found".to_string()),
    }
    lines.push(format!("  migrated store: {}", context.data_dir.join("memory.db").display()));
    lines.push(format!(
        "  any pre-restore snapshot: {}",
        context.data_dir.join("backups").display()
    ));
    lines.push("To go back by hand:".to_string());
    lines.push("  1. clawg stop".to_string());
    if let Some(aside) = &context.aside {
        lines.push(format!("  2. put {} back at {}", aside.display(), context.exe.display()));
    }
    if let Some(snapshot) = &context.pre_migration {
        lines.push(format!("  3. clawg restore {}", snapshot.display()));
    }
    lines.push(
        "Restoring keeps the migrated store as a pre-restore snapshot, and loses nothing else.".to_string(),
    );
    lines
}

fn doing(step: Step) -> &'static str {
    match step {
        Step::Stage => "downloading and verifying the new binary",
        Step::Smoke => "running the new binary",
        Step::StopDaemon => "stopping the daemon",
        Step::SetAside => "setting the installed binary aside",
        Step::PutInPlace => "putting the new binary in place",
        Step::StartDaemon => "starting the new daemon",
    }
}

fn undoing(undo: Undo) -> &'static str {
    match undo {
        Undo::RemoveStaged => "remove the downloaded binary",
        Undo::StartOldDaemon => "start the previous daemon",
        Undo::RestoreAside => "put the previous binary back",
        Undo::TakeBackNew => "take the new binary out",
        Undo::EndNewDaemon => "end the new daemon",
    }
}

/// Decides, from the result of creating a file beside the installed binary,
/// whether the update can go on. Creating it is the first thing `update` does,
/// before any download, so a directory the user cannot write to costs nothing.
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
        let report = success(&updated());

        assert!(report.contains("from 0.18.0 to 0.19.0"), "{report}");
        assert!(report.contains("/data/backups/memory-2026-10-12T17-51-16.123Z-pre-migration.db"), "{report}");
        assert!(report.contains("loses what is saved after it"), "{report}");
        assert!(report.contains("/bin/clawg.old-20261012175116123"), "{report}");
        assert!(report.contains("pid 4242"), "{report}");
        assert!(report.contains("Editors that were open keep the previous bridge until they are restarted"), "{report}");
    }

    #[test]
    fn without_a_migration_there_is_no_snapshot_to_name() {
        let report = success(&Updated { pre_migration: None, ..updated() });

        assert!(!report.contains("snapshot"), "{report}");
        assert!(!report.contains("loses"), "{report}");
    }

    #[test]
    fn without_a_daemon_the_report_says_none_was_started() {
        let report = success(&Updated { daemon: None, ..updated() });

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

        let report = failure(&restored, &context());

        assert!(report.contains("restored from /data/backups/pre-migration.db"), "{report}");
        assert!(report.contains("kept at /data/backups/pre-restore.db"), "{report}");
    }

    #[test]
    fn an_undo_that_failed_is_named() {
        let mut stuck = failed(Step::StartDaemon, "no answer", &[Undo::EndNewDaemon]);
        stuck.stuck = vec![(Undo::RestoreAside, "access denied".into())];

        let report = failure(&stuck, &context());

        assert!(report.contains("could not put the previous binary back: access denied"), "{report}");
        assert!(!report.contains("Everything was put back"), "{report}");
    }

    #[test]
    fn a_failed_restore_explains_the_way_back() {
        let mut left = failed(Step::StartDaemon, "the new daemon stopped answering", &[Undo::EndNewDaemon]);
        left.store = StoreState::RestoreFailed("the snapshot is damaged".into());
        left.skipped = vec![Undo::TakeBackNew, Undo::RestoreAside, Undo::StartOldDaemon, Undo::RemoveStaged];

        let report = failure(&left, &context());

        for needed in [
            "the new daemon stopped answering",
            "the snapshot is damaged",
            "clawg 0.19.0 stays installed",
            "/bin/clawg.old-1",
            "/data/backups/pre-migration.db",
            "pre-restore",
            "clawg stop",
            "put /bin/clawg.old-1 back at /bin/clawg",
            "clawg restore /data/backups/pre-migration.db",
        ] {
            assert!(report.contains(needed), "missing {needed:?} in:\n{report}");
        }
        assert!(!report.contains("Everything was put back"), "{report}");
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
}
