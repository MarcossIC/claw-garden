//! The update as data: which steps run, in which order, and what undoes each.

use std::future::Future;
use std::path::PathBuf;
use std::time::Duration;

use clawg_core::memory::backup::{parse_snapshot_name, snapshot_name, ProbeState, Reason, SchemaPlan};

/// A daemon that was answering when the update began.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Running {
    pub pid: u32,
    pub port: u16,
    pub version: String,
}

/// What the plan depends on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Initial {
    pub daemon: Option<Running>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Download and verify the candidate next to the installed binary.
    Stage,
    /// Run the candidate's `--version`.
    Smoke,
    StopDaemon,
    /// Move the installed binary to its `.old-*` name.
    SetAside,
    /// Move the candidate to the installed name.
    PutInPlace,
    StartDaemon,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Undo {
    RemoveStaged,
    StartOldDaemon,
    RestoreAside,
    TakeBackNew,
    EndNewDaemon,
}

pub type Planned = (Step, Option<Undo>);

/// With a daemon the whole sequence; without one the daemon is neither stopped
/// nor started, so a daemon the user stopped on purpose stays stopped.
pub fn plan(initial: &Initial) -> Vec<Planned> {
    let mut steps = vec![(Step::Stage, Some(Undo::RemoveStaged)), (Step::Smoke, None)];
    if initial.daemon.is_some() {
        steps.push((Step::StopDaemon, Some(Undo::StartOldDaemon)));
    }
    steps.push((Step::SetAside, Some(Undo::RestoreAside)));
    steps.push((Step::PutInPlace, Some(Undo::TakeBackNew)));
    if initial.daemon.is_some() {
        steps.push((Step::StartDaemon, Some(Undo::EndNewDaemon)));
    }
    steps
}

/// The boundary with disk, network and processes. One production
/// implementation; the unit tests use a model of the installation.
pub trait Effects {
    fn run(&mut self, step: Step) -> impl Future<Output = Result<(), String>>;
    fn undo(&mut self, undo: Undo) -> impl Future<Output = Result<(), String>>;
    /// Called once the new daemon is gone: gives the store back to the old
    /// binary if the new one migrated it past what the old one opens.
    fn restore_store(&mut self) -> impl Future<Output = Result<StoreRestore, String>>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreRestore {
    NotNeeded,
    /// `from` replaced the store; the store it replaced is `kept_as`.
    Restored { from: PathBuf, kept_as: Option<PathBuf> },
}

/// What happened to the store while undoing.
#[derive(Debug, PartialEq, Eq)]
pub enum StoreState {
    Untouched,
    Restored { from: PathBuf, kept_as: Option<PathBuf> },
    RestoreFailed(String),
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Updated,
    Failed(Failure),
}

/// Where a plan stopped and what was done about it.
#[derive(Debug, PartialEq, Eq)]
pub struct Failure {
    pub step: Step,
    pub cause: String,
    /// The undos that ran, in the order they ran.
    pub undone: Vec<Undo>,
    /// The undos that failed, with their cause.
    pub stuck: Vec<(Undo, String)>,
    /// The undos left undone because the store could not be given back: the
    /// new binary stays, the only one that opens the migrated store.
    pub skipped: Vec<Undo>,
    pub store: StoreState,
}

impl Step {
    /// Whether the step can leave something behind when it fails, so its undo
    /// runs too: a download that broke halfway left a file, a daemon that did
    /// not answer left a process. The other steps either happen or do not.
    fn leaves_traces_when_it_fails(self) -> bool {
        matches!(self, Step::Stage | Step::StartDaemon)
    }
}

/// Runs the plan in order. The first step that fails ends it, because the
/// next ones assume it worked, and what was done is undone in reverse.
pub async fn apply<E: Effects>(plan: &[Planned], fx: &mut E) -> Outcome {
    let mut done: Vec<Undo> = Vec::new();
    for &(step, undo) in plan {
        match fx.run(step).await {
            Ok(()) => done.extend(undo),
            Err(cause) => {
                if step.leaves_traces_when_it_fails() {
                    done.extend(undo);
                }
                return Outcome::Failed(roll_back(step, cause, done, fx).await);
            }
        }
    }
    Outcome::Updated
}

async fn roll_back<E: Effects>(step: Step, cause: String, done: Vec<Undo>, fx: &mut E) -> Failure {
    let mut undone = Vec::new();
    let mut stuck = Vec::new();
    let mut store = StoreState::Untouched;
    let mut pending = done.into_iter().rev();
    while let Some(undo) = pending.next() {
        match fx.undo(undo).await {
            Ok(()) => undone.push(undo),
            Err(why) => stuck.push((undo, why)),
        }
        if undo != Undo::EndNewDaemon {
            continue;
        }
        match fx.restore_store().await {
            Ok(StoreRestore::NotNeeded) => {}
            Ok(StoreRestore::Restored { from, kept_as }) => store = StoreState::Restored { from, kept_as },
            Err(why) => {
                let skipped = pending.collect();
                return Failure { step, cause, undone, stuck, skipped, store: StoreState::RestoreFailed(why) };
            }
        }
    }
    Failure { step, cause, undone, stuck, skipped: Vec::new(), store }
}

/// How long a new daemon has to take the store. A starting value, not measured.
pub const STORE_WAIT: Duration = Duration::from_secs(60);
/// How long a daemon that holds the store may spend on the snapshot and the
/// migrations of a big store before it binds. A starting value, not measured.
pub const PREPARE_WAIT: Duration = Duration::from_secs(600);

/// What `/health` said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    pub pid: u32,
    pub version: String,
}

/// What was seen of the daemon a moment ago.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    /// The process the update started has ended.
    pub exited: bool,
    /// The pid `memory.owner` records, read without testing the lock.
    pub store_owner: Option<u32>,
    pub answering: Option<Answer>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Wait,
    /// A daemon of the new version answers.
    Ready,
    /// The process ended and nothing answers: it did not start.
    Exited,
    TimedOut,
    /// A daemon of another version answers: the process lost the store to it.
    Other { pid: u32, version: String },
}

/// Decides, from what was seen, whether the daemon the update started is up,
/// still starting or failed.
///
/// Who answers counts before whether the child lives: an editor's bridge can
/// start a daemon between the stop and the start, and then the child is the
/// one that ended.
pub fn judge(seen: &Observation, elapsed: Duration, child_pid: u32, new_version: &str) -> Verdict {
    if let Some(answer) = &seen.answering {
        if answer.version == new_version {
            return Verdict::Ready;
        }
        if answer.pid != child_pid {
            return Verdict::Other { pid: answer.pid, version: answer.version.clone() };
        }
    }
    if seen.exited {
        return Verdict::Exited;
    }
    let limit = if seen.store_owner == Some(child_pid) { PREPARE_WAIT } else { STORE_WAIT };
    if elapsed >= limit {
        Verdict::TimedOut
    } else {
        Verdict::Wait
    }
}

/// The name the installed binary is set aside under. The format is the one
/// `install.ps1` uses (`yyyyMMddHHmmssfff`), so each tool cleans up the other's.
pub fn aside_name(exe_name: &str, now_ms: i64) -> String {
    format!("{exe_name}.old-{}", stamp(now_ms))
}

/// The name the candidate is staged under. It is executed, so it keeps the
/// extension of the installed binary.
pub fn staged_name(exe_name: &str, now_ms: i64) -> String {
    let (stem, extension) = split_name(exe_name);
    format!("{stem}.new-{}{extension}", stamp(now_ms))
}

/// The files an earlier run left beside the binary: set-aside and staged
/// ones, in the order given.
pub fn leftovers(names: &[String], exe_name: &str) -> Vec<String> {
    let (stem, _) = split_name(exe_name);
    let aside = format!("{exe_name}.old-");
    let staged = format!("{stem}.new-");
    names
        .iter()
        .filter(|name| name.starts_with(&aside) || name.starts_with(&staged))
        .cloned()
        .collect()
}

/// The `pre-migration` snapshot that is in `after` and was not in `before`:
/// the one the daemon just started wrote. The newest, if several.
pub fn new_pre_migration(before: &[String], after: &[String]) -> Option<String> {
    after
        .iter()
        .filter(|name| !before.contains(name))
        .filter(|name| parse_snapshot_name(name) == Some(Reason::PreMigration))
        .max()
        .cloned()
}

/// Whether the store, read by the binary that is running, is one it cannot
/// open: the new daemon migrated it past this binary's migrations. An
/// unreadable store is not judged: restoring over it would hide the cause.
pub fn store_needs_restore(state: &ProbeState) -> bool {
    matches!(state, ProbeState::Schema(SchemaPlan::Newer { .. } | SchemaPlan::Diverged { .. }))
}

/// `clawg.exe` as (`clawg`, `.exe`); a name without an extension has none.
fn split_name(exe_name: &str) -> (&str, &str) {
    match exe_name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => (stem, &exe_name[stem.len()..]),
        _ => (exe_name, ""),
    }
}

/// `yyyyMMddHHmmssfff` in UTC, the digits of a snapshot's own timestamp.
pub fn stamp(ms: i64) -> String {
    snapshot_name(ms, Reason::Manual)
        .chars()
        .filter(char::is_ascii_digit)
        .take(17)
        .collect()
}

#[cfg(test)]
mod plan_cases {
    use super::*;

    fn with_daemon() -> Initial {
        Initial { daemon: Some(Running { pid: 7, port: 27381, version: "0.18.0".into() }) }
    }

    fn steps(planned: &[Planned]) -> Vec<Step> {
        planned.iter().map(|(step, _)| *step).collect()
    }

    #[test]
    fn plan_order_with_and_without_daemon() {
        assert_eq!(
            steps(&plan(&with_daemon())),
            [Step::Stage, Step::Smoke, Step::StopDaemon, Step::SetAside, Step::PutInPlace, Step::StartDaemon]
        );
        assert_eq!(
            steps(&plan(&Initial { daemon: None })),
            [Step::Stage, Step::Smoke, Step::SetAside, Step::PutInPlace]
        );
    }

    #[test]
    fn each_step_that_changes_something_names_its_undo() {
        assert_eq!(
            plan(&with_daemon()),
            [
                (Step::Stage, Some(Undo::RemoveStaged)),
                (Step::Smoke, None),
                (Step::StopDaemon, Some(Undo::StartOldDaemon)),
                (Step::SetAside, Some(Undo::RestoreAside)),
                (Step::PutInPlace, Some(Undo::TakeBackNew)),
                (Step::StartDaemon, Some(Undo::EndNewDaemon)),
            ]
        );
    }

    #[test]
    fn update_without_a_daemon_does_not_start_one() {
        let planned = plan(&Initial { daemon: None });

        assert!(!steps(&planned).contains(&Step::StartDaemon));
        assert!(!steps(&planned).contains(&Step::StopDaemon));
        assert!(
            planned.iter().all(|(_, undo)| !matches!(undo, Some(Undo::StartOldDaemon | Undo::EndNewDaemon))),
            "nothing in the plan would bring a daemon up, not even to undo"
        );
    }
}

#[cfg(test)]
mod apply_cases {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Daemon {
        Old,
        Stopped,
        New,
    }

    /// The installation as the plan sees it: the installed binary, the
    /// candidate beside it, the old binary set aside, and which daemon runs.
    struct Fake {
        installed: Option<&'static str>,
        staged: Option<&'static str>,
        aside: Option<&'static str>,
        daemon: Daemon,
        calls: Vec<String>,
        fail_run: Option<(Step, &'static str)>,
        fail_undo: Option<Undo>,
        restore: Result<StoreRestore, String>,
    }

    impl Fake {
        fn new() -> Self {
            Fake {
                installed: Some("old"),
                staged: None,
                aside: None,
                daemon: Daemon::Old,
                calls: Vec::new(),
                fail_run: None,
                fail_undo: None,
                restore: Ok(StoreRestore::NotNeeded),
            }
        }

        fn failing(step: Step, cause: &'static str) -> Self {
            Fake { fail_run: Some((step, cause)), ..Fake::new() }
        }

        fn fails(&self, step: Step) -> bool {
            matches!(self.fail_run, Some((failing, _)) if failing == step)
        }

        fn as_it_was(&self) -> bool {
            self.installed == Some("old") && self.staged.is_none() && self.aside.is_none() && self.daemon == Daemon::Old
        }
    }

    impl Effects for Fake {
        async fn run(&mut self, step: Step) -> Result<(), String> {
            self.calls.push(format!("{step:?}"));
            // A step that fails after leaving a trace (a download that broke
            // halfway, a daemon that started and never answered) does; one
            // that is refused outright (a rename, a stop) changes nothing.
            match step {
                Step::Stage => self.staged = Some("new"),
                Step::Smoke => {}
                Step::StopDaemon if !self.fails(step) => self.daemon = Daemon::Stopped,
                Step::SetAside if !self.fails(step) => self.aside = self.installed.take(),
                Step::PutInPlace if !self.fails(step) => self.installed = self.staged.take(),
                Step::StartDaemon => self.daemon = Daemon::New,
                Step::StopDaemon | Step::SetAside | Step::PutInPlace => {}
            }
            match self.fail_run {
                Some((failing, cause)) if failing == step => Err(cause.to_string()),
                _ => Ok(()),
            }
        }

        async fn undo(&mut self, undo: Undo) -> Result<(), String> {
            self.calls.push(format!("undo {undo:?}"));
            if self.fail_undo == Some(undo) {
                return Err(format!("{undo:?} broke"));
            }
            match undo {
                Undo::RemoveStaged => self.staged = None,
                Undo::StartOldDaemon => self.daemon = Daemon::Old,
                Undo::RestoreAside => self.installed = self.aside.take(),
                Undo::TakeBackNew => self.staged = self.installed.take(),
                Undo::EndNewDaemon => self.daemon = Daemon::Stopped,
            }
            Ok(())
        }

        async fn restore_store(&mut self) -> Result<StoreRestore, String> {
            self.calls.push("restore_store".into());
            self.restore.clone()
        }
    }

    fn with_daemon() -> Vec<Planned> {
        plan(&Initial { daemon: Some(Running { pid: 7, port: 27381, version: "0.18.0".into() }) })
    }

    fn without_daemon() -> Vec<Planned> {
        plan(&Initial { daemon: None })
    }

    fn failure(outcome: Outcome) -> Failure {
        match outcome {
            Outcome::Failed(failure) => failure,
            Outcome::Updated => panic!("expected a failure"),
        }
    }

    #[tokio::test]
    async fn a_plan_that_works_changes_the_installation_and_undoes_nothing() {
        let mut fake = Fake::new();

        let outcome = apply(&with_daemon(), &mut fake).await;

        assert_eq!(outcome, Outcome::Updated);
        assert_eq!((fake.installed, fake.aside, fake.daemon), (Some("new"), Some("old"), Daemon::New));
        assert_eq!(fake.calls, ["Stage", "Smoke", "StopDaemon", "SetAside", "PutInPlace", "StartDaemon"]);
    }

    #[tokio::test]
    async fn each_failed_step_undoes_what_it_did() {
        let cases: [(Step, &[Undo]); 6] = [
            (Step::Stage, &[Undo::RemoveStaged]),
            (Step::Smoke, &[Undo::RemoveStaged]),
            (Step::StopDaemon, &[Undo::RemoveStaged]),
            (Step::SetAside, &[Undo::StartOldDaemon, Undo::RemoveStaged]),
            (Step::PutInPlace, &[Undo::RestoreAside, Undo::StartOldDaemon, Undo::RemoveStaged]),
            (
                Step::StartDaemon,
                &[
                    Undo::EndNewDaemon,
                    Undo::TakeBackNew,
                    Undo::RestoreAside,
                    Undo::StartOldDaemon,
                    Undo::RemoveStaged,
                ],
            ),
        ];
        for (step, expected) in cases {
            let mut fake = Fake::failing(step, "it broke");
            let failure = failure(apply(&with_daemon(), &mut fake).await);

            assert_eq!(failure.step, step);
            assert_eq!(failure.cause, "it broke");
            assert_eq!(failure.undone, expected, "{step:?}");
            assert!(failure.stuck.is_empty());
            assert!(fake.as_it_was(), "{step:?} left the installation changed");
        }
    }

    fn restored() -> StoreRestore {
        StoreRestore::Restored { from: "backups/pre-migration.db".into(), kept_as: Some("backups/pre-restore.db".into()) }
    }

    #[tokio::test]
    async fn a_store_the_old_binary_cannot_open_is_restored_before_the_old_binary_returns() {
        let mut fake = Fake::failing(Step::StartDaemon, "the new daemon migrated and stopped answering");
        fake.restore = Ok(restored());

        let failure = failure(apply(&with_daemon(), &mut fake).await);

        assert_eq!(
            failure.store,
            StoreState::Restored {
                from: "backups/pre-migration.db".into(),
                kept_as: Some("backups/pre-restore.db".into())
            }
        );
        assert_eq!(
            fake.calls[6..],
            [
                "undo EndNewDaemon",
                "restore_store",
                "undo TakeBackNew",
                "undo RestoreAside",
                "undo StartOldDaemon",
                "undo RemoveStaged"
            ],
            "the child is gone before the store is touched, and the old daemon starts on the restored store"
        );
        assert!(fake.as_it_was());
    }

    #[tokio::test]
    async fn a_failed_restore_leaves_the_new_binary_and_explains_the_way_back() {
        let mut fake = Fake::failing(Step::StartDaemon, "the new daemon migrated and stopped answering");
        fake.restore = Err("the snapshot is missing".into());

        let failure = failure(apply(&with_daemon(), &mut fake).await);

        assert_eq!(failure.store, StoreState::RestoreFailed("the snapshot is missing".into()));
        assert_eq!(failure.undone, [Undo::EndNewDaemon]);
        assert_eq!(
            failure.skipped,
            [Undo::TakeBackNew, Undo::RestoreAside, Undo::StartOldDaemon, Undo::RemoveStaged]
        );
        assert_eq!(
            (fake.installed, fake.aside, fake.daemon),
            (Some("new"), Some("old"), Daemon::Stopped),
            "the new binary stays: it is the only one that opens the migrated store"
        );
    }

    #[tokio::test]
    async fn the_store_is_only_looked_at_when_a_new_daemon_was_started() {
        for step in [Step::Stage, Step::StopDaemon, Step::SetAside, Step::PutInPlace] {
            let mut fake = Fake::failing(step, "it broke");
            fake.restore = Err("must not be asked".into());

            let failure = failure(apply(&with_daemon(), &mut fake).await);

            assert_eq!(failure.store, StoreState::Untouched, "{step:?}");
            assert!(!fake.calls.contains(&"restore_store".to_string()), "{step:?}");
        }
    }

    #[tokio::test]
    async fn a_store_that_needs_no_restore_is_left_alone() {
        let mut fake = Fake::failing(Step::StartDaemon, "the new daemon refused the store");

        let failure = failure(apply(&with_daemon(), &mut fake).await);

        assert_eq!(failure.store, StoreState::Untouched);
        assert!(fake.as_it_was());
    }

    #[tokio::test]
    async fn the_first_failure_ends_the_chain() {
        let mut fake = Fake::failing(Step::Smoke, "it printed another version");

        apply(&with_daemon(), &mut fake).await;

        assert_eq!(fake.calls, ["Stage", "Smoke", "undo RemoveStaged"]);
    }

    #[tokio::test]
    async fn a_hash_mismatch_never_touches_the_installed_binary() {
        let mut fake = Fake::failing(Step::Stage, "the download does not match SHA256SUMS");

        let failure = failure(apply(&with_daemon(), &mut fake).await);

        assert!(failure.cause.contains("SHA256SUMS"));
        assert!(fake.as_it_was());
        assert_eq!(fake.calls, ["Stage", "undo RemoveStaged"], "the daemon was not even stopped");
    }

    #[tokio::test]
    async fn a_failed_download_or_signature_leaves_everything_as_it_was() {
        for cause in ["the download broke at 40%", "the signature does not match any trusted key"] {
            for planned in [with_daemon(), without_daemon()] {
                let mut fake = Fake::failing(Step::Stage, cause);

                let failure = failure(apply(&planned, &mut fake).await);

                assert_eq!(failure.cause, cause);
                assert!(fake.as_it_was(), "{cause}");
            }
        }
    }

    #[tokio::test]
    async fn a_daemon_that_does_not_stop_renames_nothing() {
        let mut fake = Fake::failing(Step::StopDaemon, "still running after 15s");

        let failure = failure(apply(&with_daemon(), &mut fake).await);

        assert_eq!(failure.step, Step::StopDaemon);
        assert_eq!(fake.calls, ["Stage", "Smoke", "StopDaemon", "undo RemoveStaged"]);
        assert!(fake.as_it_was());
    }

    #[tokio::test]
    async fn without_a_daemon_a_failed_swap_brings_none_up() {
        let mut fake = Fake::failing(Step::PutInPlace, "access denied");
        fake.daemon = Daemon::Stopped;

        let failure = failure(apply(&without_daemon(), &mut fake).await);

        assert_eq!(failure.undone, [Undo::RestoreAside, Undo::RemoveStaged]);
        assert_eq!(fake.daemon, Daemon::Stopped);
        assert_eq!(fake.installed, Some("old"));
    }

    #[tokio::test]
    async fn an_undo_that_fails_is_reported_and_the_rest_still_run() {
        let mut fake = Fake::failing(Step::StartDaemon, "it did not start");
        fake.fail_undo = Some(Undo::RestoreAside);

        let failure = failure(apply(&with_daemon(), &mut fake).await);

        assert_eq!(failure.stuck, [(Undo::RestoreAside, "RestoreAside broke".to_string())]);
        assert_eq!(
            failure.undone,
            [Undo::EndNewDaemon, Undo::TakeBackNew, Undo::StartOldDaemon, Undo::RemoveStaged]
        );
        assert_eq!(fake.aside, Some("old"), "the old binary is still where the failed undo left it");
    }
}

#[cfg(test)]
mod watch_cases {
    use super::*;

    const CHILD: u32 = 500;
    const NEW: &str = "0.19.0";

    fn seen() -> Observation {
        Observation { exited: false, store_owner: None, answering: None }
    }

    fn answering(pid: u32, version: &str) -> Option<Answer> {
        Some(Answer { pid, version: version.into() })
    }

    fn judged(observation: &Observation, seconds: u64) -> Verdict {
        judge(observation, Duration::from_secs(seconds), CHILD, NEW)
    }

    #[test]
    fn a_daemon_answering_with_the_new_version_is_ready() {
        let observation = Observation { answering: answering(CHILD, NEW), ..seen() };

        assert_eq!(judged(&observation, 3), Verdict::Ready);
    }

    #[test]
    fn the_new_version_counts_whoever_started_it() {
        // An editor's bridge may have started it between the stop and the start.
        let observation = Observation { exited: true, answering: answering(901, NEW), ..seen() };

        assert_eq!(judged(&observation, 3), Verdict::Ready);
    }

    #[test]
    fn another_version_answering_is_not_the_child() {
        let observation = Observation { exited: true, answering: answering(901, "0.18.0"), ..seen() };

        assert_eq!(judged(&observation, 3), Verdict::Other { pid: 901, version: "0.18.0".into() });
    }

    #[test]
    fn a_child_that_ended_without_anyone_answering_did_not_start() {
        let observation = Observation { exited: true, ..seen() };

        assert_eq!(judged(&observation, 3), Verdict::Exited);
    }

    #[test]
    fn a_child_that_has_not_taken_the_store_has_a_minute() {
        assert_eq!(judged(&seen(), 59), Verdict::Wait);
        assert_eq!(judged(&seen(), 60), Verdict::TimedOut);
        let other_holder = Observation { store_owner: Some(CHILD + 1), ..seen() };
        assert_eq!(judged(&other_holder, 60), Verdict::TimedOut, "the record of another process is not the child's");
    }

    #[test]
    fn a_child_holding_the_store_is_preparing_it_and_has_ten_minutes() {
        let holding = Observation { store_owner: Some(CHILD), ..seen() };

        assert_eq!(judged(&holding, 61), Verdict::Wait);
        assert_eq!(judged(&holding, 599), Verdict::Wait);
        assert_eq!(judged(&holding, 600), Verdict::TimedOut);
    }

    #[test]
    fn an_answer_ends_the_wait_even_after_the_deadline() {
        let observation = Observation { answering: answering(CHILD, NEW), ..seen() };

        assert_eq!(judged(&observation, 10_000), Verdict::Ready);
    }

    const EXE: &str = "clawg.exe";

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn leftovers_picks_the_set_aside_and_staged_files_of_this_binary_only() {
        let found = leftovers(
            &names(&[
                "clawg.exe",
                "clawg.exe.old-20261012175116123",
                "clawg.new-20261012175116456.exe",
                "clawg.exe.old-x",
                "clawg.newest.exe",
                "clawg.exe.bak",
                "other.exe.old-1",
                "notes.txt",
            ]),
            EXE,
        );

        assert_eq!(found, names(&["clawg.exe.old-20261012175116123", "clawg.new-20261012175116456.exe", "clawg.exe.old-x"]));
    }

    #[test]
    fn leftovers_on_a_binary_without_an_extension() {
        let found = leftovers(&names(&["clawg", "clawg.old-1", "clawg.new-2", "clawg.exe.old-3"]), "clawg");

        assert_eq!(found, names(&["clawg.old-1", "clawg.new-2"]));
    }

    #[test]
    fn the_names_an_update_makes_are_the_ones_it_cleans_up_next_time() {
        for exe in [EXE, "clawg"] {
            let made = vec![aside_name(exe, 1_791_827_476_123), staged_name(exe, 1_791_827_476_123)];

            assert_eq!(leftovers(&made, exe), made, "{exe}");
        }
        assert_eq!(aside_name(EXE, 1_791_827_476_123), "clawg.exe.old-20261012175116123");
        assert_eq!(staged_name(EXE, 1_791_827_476_123), "clawg.new-20261012175116123.exe");
        assert_eq!(staged_name("clawg", 1_791_827_476_123), "clawg.new-20261012175116123");
    }

    #[test]
    fn the_snapshot_a_start_wrote_is_the_pre_migration_that_was_not_there_before() {
        use clawg_core::memory::backup::{snapshot_name, Reason};
        let old = snapshot_name(1_700_000_000_000, Reason::PreMigration);
        let manual = snapshot_name(1_700_000_001_000, Reason::Manual);
        let new = snapshot_name(1_700_000_002_000, Reason::PreMigration);
        let reembed = snapshot_name(1_700_000_003_000, Reason::PreReembed);
        let before = vec![old.clone(), manual.clone()];

        assert_eq!(new_pre_migration(&before, &[old.clone(), manual.clone()]), None);
        assert_eq!(new_pre_migration(&before, &[old.clone(), manual.clone(), reembed.clone()]), None);
        assert_eq!(
            new_pre_migration(&before, &[old.clone(), manual, new.clone(), reembed]),
            Some(new.clone())
        );
        assert_eq!(new_pre_migration(&[], &[old.clone(), new.clone()]), Some(new), "the newest of several");
        assert_eq!(new_pre_migration(&[], &["notes.txt".to_string()]), None);
    }

    #[test]
    fn a_store_the_running_binary_cannot_open_is_given_back() {
        use clawg_core::memory::backup::{ProbeState, SchemaPlan};
        for needed in [SchemaPlan::Newer { db: 16, binary: 15 }, SchemaPlan::Diverged { version: 12 }] {
            assert!(store_needs_restore(&ProbeState::Schema(needed)), "{needed:?}");
        }
        for fine in [SchemaPlan::Fresh, SchemaPlan::UpToDate, SchemaPlan::Pending { from: 14, to: 15 }] {
            assert!(!store_needs_restore(&ProbeState::Schema(fine)), "{fine:?}");
        }
        assert!(!store_needs_restore(&ProbeState::Missing));
        assert!(!store_needs_restore(&ProbeState::Unreadable("locked".into())));
    }
}
