//! The update as data: which steps run, in which order, and what undoes each.

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
