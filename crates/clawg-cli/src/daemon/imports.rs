//! The import job that runs, or the last one that ended, as the daemon
//! remembers it. Kept in memory only: a restart forgets every job, and the
//! store's `memory_import` journal is what says what an import wrote.

use std::sync::{Arc, Mutex, MutexGuard};

use clawg_core::memory::portability::timing::Step;
use clawg_core::memory::portability::{Counters, ImportControl, ImportReport, StopCause, Stopped};
use serde::Serialize;
use tokio::sync::watch;
use tokio::task::AbortHandle;

/// How a job ended.
pub enum End {
    Done(ImportReport),
    Stopped(Stopped),
    Failed { error: String, detail: Option<String> },
}

/// Why a job cannot take the slot.
#[derive(Debug, PartialEq, Eq)]
pub enum Busy {
    /// A job holds it, or a request is about to.
    Running(u64),
    /// The daemon is shutting down and takes no more jobs.
    Closing,
}

/// What a request to cancel a job found.
pub enum Cancel {
    /// The job runs and was asked to stop; its view says why.
    Asked(Arc<JobView>),
    /// The job ended already: this is how, and nothing was asked of it.
    Over(Arc<JobView>),
}

/// What a job was asked to do.
pub struct Meta {
    pub dry_run: bool,
    pub from: &'static str,
    pub started_at_ms: i64,
}

/// A job as `GET /v1/imports/{id}` shows it.
#[derive(Debug, Serialize)]
pub struct JobView {
    job_id: u64,
    phase: &'static str,
    dry_run: bool,
    from: &'static str,
    started_at_ms: i64,
    /// For a final phase, when the job ended.
    phase_started_at_ms: i64,
    #[serde(flatten)]
    counters: Counters,
    #[serde(skip_serializing_if = "Option::is_none")]
    import_id: Option<i64>,
    /// Why the job was asked to stop, while it has not stopped.
    #[serde(skip_serializing_if = "Option::is_none")]
    stopping: Option<StopCause>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stopped_in: Option<&'static str>,
    /// The shutdown cut the job off in `stopped_in`: a call that step had
    /// already sent may have been confirmed, and `memory_import` says what was.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    abandoned: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    report: Option<ImportReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
}

/// The phase a step shows as. Every job starts in its plan, and the read is
/// the request's own, before a job exists.
fn phase_of(step: Option<Step>) -> &'static str {
    match step {
        None | Some(Step::Read) | Some(Step::Plan) => "planning",
        Some(Step::Prepare) => "preparing",
        Some(Step::Translate) => "translating",
        Some(Step::Load) => "loading",
        Some(Step::Decide) => "deciding",
        Some(Step::Embed) => "embedding",
        Some(Step::Snapshot) => "snapshot",
        Some(Step::Write) => "writing",
        Some(Step::Links) => "linking",
    }
}

struct Running {
    id: u64,
    meta: Meta,
    control: ImportControl,
    task: AbortHandle,
}

impl Running {
    fn view(&self) -> JobView {
        let progress = self.control.progress();
        JobView {
            job_id: self.id,
            phase: phase_of(progress.step),
            dry_run: self.meta.dry_run,
            from: self.meta.from,
            started_at_ms: self.meta.started_at_ms,
            phase_started_at_ms: if progress.step.is_some() { progress.step_started_at_ms } else { self.meta.started_at_ms },
            counters: progress.counters,
            import_id: progress.import_id,
            stopping: self.control.cause(),
            stopped_in: None,
            abandoned: false,
            report: None,
            error: None,
            detail: None,
        }
    }

    fn ended(self, end: End, now_ms: i64) -> JobView {
        let (phase, stopped_in, report, error, detail) = match end {
            End::Done(report) => ("done", None, Some(report), None, None),
            End::Stopped(Stopped { cause, step, report }) => {
                let phase = match cause {
                    StopCause::Cancel => "cancelled",
                    StopCause::Shutdown => "interrupted",
                };
                (phase, Some(step.name()), report, None, None)
            }
            End::Failed { error, detail } => ("failed", None, None, Some(error), detail),
        };
        let progress = self.control.progress();
        JobView {
            job_id: self.id,
            phase,
            dry_run: self.meta.dry_run,
            from: self.meta.from,
            started_at_ms: self.meta.started_at_ms,
            phase_started_at_ms: now_ms,
            counters: progress.counters,
            import_id: progress.import_id,
            stopping: None,
            stopped_in,
            abandoned: false,
            report,
            error,
            detail,
        }
    }

    /// The job's task is aborted and its end is not known: it was in the step
    /// it was in, and nothing it has not sent will be.
    fn abandoned(self, now_ms: i64) -> JobView {
        self.task.abort();
        let progress = self.control.progress();
        JobView {
            job_id: self.id,
            phase: "interrupted",
            dry_run: self.meta.dry_run,
            from: self.meta.from,
            started_at_ms: self.meta.started_at_ms,
            phase_started_at_ms: now_ms,
            counters: progress.counters,
            import_id: progress.import_id,
            stopping: None,
            stopped_in: progress.step.map(Step::name),
            abandoned: true,
            report: None,
            error: None,
            detail: None,
        }
    }
}

#[derive(Default)]
enum Slot {
    #[default]
    Idle,
    Reserved(u64),
    Running(Running),
}

/// The registry as data: every transition is a method, so it is tested
/// without a runtime.
#[derive(Default)]
struct Jobs {
    next_id: u64,
    slot: Slot,
    last: Option<Arc<JobView>>,
    /// Set by the shutdown: no job is accepted from then on.
    closing: bool,
}

impl Jobs {
    fn reserve(&mut self) -> Result<u64, Busy> {
        if self.closing {
            return Err(Busy::Closing);
        }
        match &self.slot {
            Slot::Idle => {
                self.next_id += 1;
                self.slot = Slot::Reserved(self.next_id);
                Ok(self.next_id)
            }
            Slot::Reserved(job_id) | Slot::Running(Running { id: job_id, .. }) => Err(Busy::Running(*job_id)),
        }
    }

    fn is_idle(&self) -> bool {
        matches!(self.slot, Slot::Idle)
    }

    fn release(&mut self, id: u64) {
        if matches!(self.slot, Slot::Reserved(held) if held == id) {
            self.slot = Slot::Idle;
        }
    }

    /// Starts job `id`: `spawn` launches its task here, with the slot still
    /// held, so the task cannot end before the registry knows it.
    fn run(&mut self, id: u64, meta: Meta, control: ImportControl, spawn: impl FnOnce() -> AbortHandle) -> Result<Arc<JobView>, Busy> {
        if self.closing {
            self.release(id);
            return Err(Busy::Closing);
        }
        // A reservation the shutdown abandoned is not the slot's any more.
        if !matches!(self.slot, Slot::Reserved(held) if held == id) {
            return Err(Busy::Closing);
        }
        let running = Running { id, meta, control, task: spawn() };
        let view = Arc::new(running.view());
        self.slot = Slot::Running(running);
        Ok(view)
    }

    fn finish(&mut self, id: u64, end: End, now_ms: i64) {
        match std::mem::take(&mut self.slot) {
            Slot::Running(running) if running.id == id => self.last = Some(Arc::new(running.ended(end, now_ms))),
            other => self.slot = other,
        }
    }

    fn close(&mut self) {
        self.closing = true;
        if let Slot::Running(running) = &self.slot {
            running.control.stop(StopCause::Shutdown);
        }
    }

    /// Gives the slot back whatever holds it: the running task is aborted
    /// and its job is the last one, interrupted. Anything it sent the
    /// connection before is already queued ahead of the close that follows.
    fn abandon(&mut self, now_ms: i64) -> Option<Arc<JobView>> {
        match std::mem::take(&mut self.slot) {
            Slot::Running(running) => {
                let view = Arc::new(running.abandoned(now_ms));
                self.last = Some(view.clone());
                Some(view)
            }
            Slot::Reserved(_) | Slot::Idle => None,
        }
    }

    fn view(&self, id: u64) -> Option<Arc<JobView>> {
        match &self.slot {
            Slot::Running(running) if running.id == id => Some(Arc::new(running.view())),
            _ => self.last.clone().filter(|last| last.job_id == id),
        }
    }

    fn active(&self) -> Option<JobView> {
        match &self.slot {
            Slot::Running(running) => Some(running.view()),
            Slot::Reserved(_) | Slot::Idle => None,
        }
    }

    fn cancel(&self, id: u64) -> Option<Cancel> {
        match &self.slot {
            Slot::Running(running) if running.id == id => {
                // The first cause stays: a shutdown that came first is still the one.
                running.control.stop(StopCause::Cancel);
                Some(Cancel::Asked(Arc::new(running.view())))
            }
            _ => self.last.clone().filter(|last| last.job_id == id).map(Cancel::Over),
        }
    }
}

/// One import job at a time, and the last one that ended.
#[derive(Default)]
pub struct ImportJobs {
    state: Mutex<Jobs>,
    /// `true` from a reservation until its job ends or it is released.
    busy: watch::Sender<bool>,
}

/// The slot, held by a request that has not accepted its job yet: it frees
/// the slot if it is dropped first, so a request that fails or whose client
/// goes while it reads the file leaves nothing behind.
pub struct Reservation<'a> {
    jobs: &'a ImportJobs,
    id: u64,
    accepted: bool,
}

impl Reservation<'_> {
    pub fn id(&self) -> u64 {
        self.id
    }

    /// The job starts, and the slot is its until it ends. `spawn` launches
    /// its task; it is not called when the daemon is closing, and the
    /// reservation is released.
    pub fn run(mut self, meta: Meta, control: ImportControl, spawn: impl FnOnce() -> AbortHandle) -> Result<Arc<JobView>, Busy> {
        self.accepted = true;
        let mut jobs = self.jobs.lock();
        let started = jobs.run(self.id, meta, control, spawn);
        self.jobs.busy.send_replace(!jobs.is_idle());
        started
    }
}

impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        if !self.accepted {
            let mut jobs = self.jobs.lock();
            jobs.release(self.id);
            self.jobs.busy.send_replace(!jobs.is_idle());
        }
    }
}

impl ImportJobs {
    fn lock(&self) -> MutexGuard<'_, Jobs> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Takes the slot, or names the job that holds it: checking and taking
    /// are one step, so two requests never both win.
    pub fn reserve(&self) -> Result<Reservation<'_>, Busy> {
        let mut jobs = self.lock();
        let id = jobs.reserve()?;
        self.busy.send_replace(true);
        Ok(Reservation { jobs: self, id, accepted: false })
    }

    /// Ends job `id`, if it is the one running.
    pub fn finish(&self, id: u64, end: End, now_ms: i64) {
        let mut jobs = self.lock();
        jobs.finish(id, end, now_ms);
        self.busy.send_replace(!jobs.is_idle());
    }

    /// The shutdown begins: no job is accepted from now on and the one that
    /// runs is asked to stop.
    pub fn close(&self) {
        self.lock().close();
    }

    /// Resolves once no job holds the slot.
    pub async fn settled(&self) {
        let _ = self.busy.subscribe().wait_for(|busy| !*busy).await;
    }

    /// Cuts the running job off: its task is aborted and it is remembered as
    /// interrupted in the step it was in.
    pub fn abandon(&self, now_ms: i64) {
        let mut jobs = self.lock();
        let abandoned = jobs.abandon(now_ms);
        self.busy.send_replace(!jobs.is_idle());
        if let Some(view) = abandoned {
            log::warn!(
                "import job {} was cut off in {}; what it had already sent the store may have been confirmed, and `memory_import` says what was",
                view.job_id,
                view.stopped_in.unwrap_or("its start"),
            );
        }
    }

    /// The job that runs or the last one that ended, if it is `id`.
    pub fn view(&self, id: u64) -> Option<Arc<JobView>> {
        self.lock().view(id)
    }

    /// The job that runs, read from memory alone: health calls it without
    /// touching the store.
    pub fn active(&self) -> Option<JobView> {
        self.lock().active()
    }

    /// Asks job `id` to stop, if it runs.
    pub fn cancel(&self, id: u64) -> Option<Cancel> {
        self.lock().cancel(id)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use clawg_core::memory::portability::timing::Step;
    use clawg_core::memory::portability::{ImportControl, ImportReport, Stopped, StopCause};

    use super::*;

    fn meta() -> Meta {
        Meta { dry_run: false, from: "clawg", started_at_ms: 1_000 }
    }

    /// A task that never ends on its own, for a job to own.
    fn pending_task() -> AbortHandle {
        tokio::spawn(std::future::pending::<()>()).abort_handle()
    }

    /// A job that was accepted and runs.
    fn running(jobs: &ImportJobs) -> (u64, ImportControl) {
        let reservation = jobs.reserve().expect("a free slot");
        let (id, control) = (reservation.id(), ImportControl::default());
        reservation.run(meta(), control.clone(), pending_task).expect("the registry is open");
        (id, control)
    }

    #[tokio::test]
    async fn a_reservation_is_atomic_and_released_when_dropped() {
        let jobs = ImportJobs::default();
        let first = jobs.reserve().expect("the first reservation");
        let busy = jobs.reserve().err().expect("the slot is held");
        assert_eq!(busy, Busy::Running(first.id()), "the refusal names the job that holds the slot");

        drop(first);
        let again = jobs.reserve().expect("a dropped reservation frees the slot");
        assert_eq!(again.id(), 2, "an id is never handed out twice");

        let control = ImportControl::default();
        let id = again.id();
        again.run(meta(), control, pending_task).expect("the registry is open");
        assert_eq!(jobs.reserve().err(), Some(Busy::Running(id)), "a running job holds the slot");

        let contested = ImportJobs::default();
        let wins = std::thread::scope(|scope| {
            let jobs = &contested;
            let racers: Vec<_> = (0..16).map(|_| scope.spawn(move || jobs.reserve())).collect();
            let results: Vec<_> = racers.into_iter().map(|racer| racer.join().expect("a racer")).collect();
            results.iter().filter(|result| result.is_ok()).count()
        });
        assert_eq!(wins, 1, "of many reservations at once exactly one wins");
    }

    #[tokio::test]
    async fn a_reserved_id_that_is_never_accepted_is_a_404() {
        let jobs = ImportJobs::default();
        // Dropped by a handler that failed before it accepted anything.
        let reservation = jobs.reserve().expect("a slot");
        let lost = reservation.id();
        assert!(jobs.view(lost).is_none(), "a reservation is not a job yet");
        drop(reservation);

        let (id, _) = running(&jobs);
        assert_eq!(id, 2, "the id of the dropped reservation is skipped");
        assert!(jobs.view(lost).is_none(), "its GET is a 404 for good");
        assert!(jobs.view(id).is_some());
    }

    #[tokio::test]
    async fn only_the_last_finished_job_is_kept() {
        let jobs = ImportJobs::default();
        let (first, _) = running(&jobs);
        jobs.finish(first, End::Done(ImportReport::default()), 2_000);
        assert_eq!(jobs.view(first).expect("the last finished").phase, "done");

        let (second, _) = running(&jobs);
        assert_eq!(jobs.view(first).map(|view| view.phase), Some("done"), "the last one stays until the next ends");
        jobs.finish(second, End::Done(ImportReport::default()), 3_000);
        assert!(jobs.view(first).is_none(), "only the last finished job is remembered");
        assert_eq!(jobs.view(second).expect("the last finished").phase_started_at_ms, 3_000);
    }

    #[tokio::test]
    async fn a_finish_for_a_job_that_is_not_running_changes_nothing() {
        let jobs = ImportJobs::default();
        jobs.finish(7, End::Done(ImportReport::default()), 1);
        assert!(jobs.view(7).is_none());

        let (id, _) = running(&jobs);
        jobs.finish(id + 1, End::Done(ImportReport::default()), 1);
        assert_eq!(jobs.view(id).expect("still running").phase, "planning");
    }

    #[tokio::test]
    async fn a_running_job_shows_what_was_asked_of_it() {
        let jobs = ImportJobs::default();
        let (id, control) = running(&jobs);
        let view = jobs.view(id).expect("running");
        assert_eq!((view.job_id, view.phase, view.dry_run, view.from, view.started_at_ms), (id, "planning", false, "clawg", 1_000));
        assert_eq!(view.phase_started_at_ms, 1_000, "no step has begun");
        assert!(view.stopping.is_none() && view.report.is_none() && view.stopped_in.is_none());

        control.stop(StopCause::Cancel);
        assert_eq!(jobs.view(id).and_then(|view| view.stopping), Some(StopCause::Cancel));
        let json = serde_json::to_value(&*jobs.view(id).expect("running")).expect("serializes");
        assert_eq!(json["stopping"], "cancel");
        assert!(json.get("report").is_none() && json.get("error").is_none(), "{json}");
        assert_eq!(json["rows_embedded"], 0, "the counters sit beside the rest: {json}");
    }

    #[tokio::test]
    async fn a_finished_job_says_how_it_ended() {
        let stopped = |cause, report| End::Stopped(Stopped { cause, step: Step::Embed, report });
        let failed = End::Failed { error: "embedder_not_ready".into(), detail: Some("the model files are gone".into()) };
        let ended = [
            (End::Done(ImportReport::default()), "done", None, true),
            (stopped(StopCause::Cancel, None), "cancelled", Some("embed"), false),
            (stopped(StopCause::Shutdown, None), "interrupted", Some("embed"), false),
            (stopped(StopCause::Cancel, Some(ImportReport::default())), "cancelled", Some("embed"), true),
            (failed, "failed", None, false),
        ];
        for (end, phase, stopped_in, has_report) in ended {
            let jobs = ImportJobs::default();
            let (id, control) = running(&jobs);
            control.stop(StopCause::Cancel);
            jobs.finish(id, end, 5_000);
            let view = jobs.view(id).expect("the last finished");
            assert_eq!((view.phase, view.stopped_in, view.report.is_some()), (phase, stopped_in, has_report), "{phase}");
            assert!(view.stopping.is_none(), "{phase}: a finished job is not stopping");
            assert_eq!(view.phase_started_at_ms, 5_000, "{phase}: a final phase starts when the job ended");
        }

        let jobs = ImportJobs::default();
        let (id, _) = running(&jobs);
        jobs.finish(id, End::Failed { error: "project_deleted".into(), detail: Some("deleted".into()) }, 1);
        let view = jobs.view(id).expect("failed");
        assert_eq!((view.error.as_deref(), view.detail.as_deref()), (Some("project_deleted"), Some("deleted")));
    }

    #[tokio::test]
    async fn cancelling_asks_the_running_job_and_tells_a_finished_one() {
        let jobs = ImportJobs::default();
        assert!(jobs.cancel(1).is_none(), "no such job");

        let (id, control) = running(&jobs);
        let Some(Cancel::Asked(asked)) = jobs.cancel(id) else { panic!("a running job is asked") };
        assert_eq!(asked.stopping, Some(StopCause::Cancel), "the answer already says it is stopping");
        assert_eq!(control.cause(), Some(StopCause::Cancel));
        assert!(jobs.cancel(id + 1).is_none(), "another id is no job");

        jobs.finish(id, End::Stopped(Stopped { cause: StopCause::Cancel, step: Step::Embed, report: None }), 4_000);
        let Some(Cancel::Over(over)) = jobs.cancel(id) else { panic!("a finished job is told as it is") };
        assert_eq!((over.phase, over.stopped_in), ("cancelled", Some("embed")));
        assert!(jobs.cancel(id + 1).is_none());
    }

    #[tokio::test]
    async fn cancelling_after_a_shutdown_keeps_its_cause() {
        let jobs = ImportJobs::default();
        let (id, control) = running(&jobs);
        jobs.close();
        let Some(Cancel::Asked(asked)) = jobs.cancel(id) else { panic!("still running") };
        assert_eq!((asked.stopping, control.cause()), (Some(StopCause::Shutdown), Some(StopCause::Shutdown)));
    }

    #[tokio::test]
    async fn only_a_running_job_is_active() {
        let jobs = ImportJobs::default();
        assert!(jobs.active().is_none());
        let reservation = jobs.reserve().expect("a free slot");
        assert!(jobs.active().is_none(), "a reservation is not a job yet");
        let (id, control) = (reservation.id(), ImportControl::default());
        reservation.run(meta(), control, pending_task).expect("the registry is open");
        assert_eq!(jobs.active().map(|view| view.job_id), Some(id));

        jobs.finish(id, End::Done(ImportReport::default()), 2_000);
        assert!(jobs.active().is_none(), "the last finished job is not active");
    }

    #[tokio::test]
    async fn a_closed_registry_refuses_to_reserve() {
        let jobs = ImportJobs::default();
        jobs.close();
        assert_eq!(jobs.reserve().err(), Some(Busy::Closing));

        // A request that reserved before the close does not start its job.
        let jobs = ImportJobs::default();
        let reservation = jobs.reserve().expect("a free slot");
        jobs.close();
        let spawned = std::cell::Cell::new(false);
        let refused = reservation.run(meta(), ImportControl::default(), || {
            spawned.set(true);
            pending_task()
        });
        assert_eq!(refused.err(), Some(Busy::Closing));
        assert!(!spawned.get(), "no task is launched into a closed registry");
        tokio::time::timeout(Duration::from_secs(5), jobs.settled()).await.expect("the refused reservation freed the slot");

        // The job that runs is asked to stop for the shutdown.
        let jobs = ImportJobs::default();
        let (id, control) = running(&jobs);
        jobs.close();
        assert_eq!(control.cause(), Some(StopCause::Shutdown));
        assert_eq!(jobs.view(id).and_then(|view| view.stopping), Some(StopCause::Shutdown));
    }

    #[tokio::test]
    async fn closing_after_a_cancel_keeps_the_first_cause() {
        let jobs = ImportJobs::default();
        let (_, control) = running(&jobs);
        control.stop(StopCause::Cancel);
        jobs.close();
        assert_eq!(control.cause(), Some(StopCause::Cancel));
    }

    #[tokio::test]
    async fn settled_waits_for_the_slot_to_be_free() {
        let jobs = Arc::new(ImportJobs::default());
        tokio::time::timeout(Duration::from_secs(5), jobs.settled()).await.expect("nothing to wait for");

        let (id, _) = running(&jobs);
        let waiting = tokio::spawn({
            let jobs = jobs.clone();
            async move { jobs.settled().await }
        });
        tokio::task::yield_now().await;
        assert!(!waiting.is_finished(), "a running job is waited for");
        jobs.finish(id, End::Done(ImportReport::default()), 2_000);
        tokio::time::timeout(Duration::from_secs(5), waiting).await.expect("ended").expect("joined");
    }

    #[tokio::test]
    async fn an_abandoned_job_is_interrupted_and_named() {
        let jobs = ImportJobs::default();
        let task = tokio::spawn(std::future::pending::<()>());
        let reservation = jobs.reserve().expect("a free slot");
        let id = reservation.id();
        reservation.run(meta(), ImportControl::default(), || task.abort_handle()).expect("the registry is open");

        jobs.abandon(7_000);

        assert!(task.await.expect_err("the task was aborted").is_cancelled());
        let view = jobs.view(id).expect("the last finished");
        assert_eq!((view.phase, view.abandoned, view.report.is_none(), view.phase_started_at_ms), ("interrupted", true, true, 7_000));
        assert!(view.stopping.is_none());
        let json = serde_json::to_value(&*view).expect("serializes");
        assert_eq!(json["abandoned"], true, "{json}");
        tokio::time::timeout(Duration::from_secs(5), jobs.settled()).await.expect("the slot is free");
        assert!(jobs.reserve().is_ok());

        let other = ImportJobs::default();
        let (other_id, _) = running(&other);
        let json = serde_json::to_value(&*other.view(other_id).expect("running")).expect("serializes");
        assert!(json.get("abandoned").is_none(), "only an abandoned job says so: {json}");
    }

    #[tokio::test]
    async fn abandoning_nothing_changes_nothing() {
        let jobs = ImportJobs::default();
        jobs.abandon(1);
        assert!(jobs.view(1).is_none());

        let reservation = jobs.reserve().expect("a free slot");
        jobs.abandon(2);
        drop(reservation);
        assert!(jobs.reserve().is_ok(), "a reservation that was abandoned frees the slot once");
    }

    #[tokio::test]
    async fn a_finish_after_abandon_changes_nothing() {
        let jobs = ImportJobs::default();
        let (id, _) = running(&jobs);
        jobs.abandon(7_000);

        jobs.finish(id, End::Failed { error: "the import task ended without a result".into(), detail: None }, 8_000);

        let view = jobs.view(id).expect("the last finished");
        assert_eq!((view.phase, view.abandoned, view.phase_started_at_ms), ("interrupted", true, 7_000));
        assert!(view.error.is_none());
    }

    #[test]
    fn phase_of_covers_every_step() {
        let table = [
            (None, "planning"),
            (Some(Step::Read), "planning"),
            (Some(Step::Plan), "planning"),
            (Some(Step::Prepare), "preparing"),
            (Some(Step::Translate), "translating"),
            (Some(Step::Load), "loading"),
            (Some(Step::Decide), "deciding"),
            (Some(Step::Embed), "embedding"),
            (Some(Step::Snapshot), "snapshot"),
            (Some(Step::Write), "writing"),
            (Some(Step::Links), "linking"),
        ];
        for (step, phase) in table {
            assert_eq!(phase_of(step), phase, "{step:?}");
        }
    }
}
