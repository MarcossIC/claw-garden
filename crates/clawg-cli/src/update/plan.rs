//! The update as data: which steps run, in which order, and what undoes each.

use std::future::Future;

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
    for undo in done.into_iter().rev() {
        match fx.undo(undo).await {
            Ok(()) => undone.push(undo),
            Err(why) => stuck.push((undo, why)),
        }
    }
    Failure { step, cause, undone, stuck }
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
