//! The import job that runs, or the last one that ended, as the daemon
//! remembers it. Kept in memory only: a restart forgets every job, and the
//! store's `memory_import` journal is what says what an import wrote.

use std::sync::{Arc, Mutex, MutexGuard};

use clawg_core::memory::portability::timing::Step;
use clawg_core::memory::portability::{Counters, ImportControl, ImportReport, StopCause, Stopped};
use serde::Serialize;

/// How a job ended.
pub enum End {
    Done(ImportReport),
    Stopped(Stopped),
    Failed { error: String, detail: Option<String> },
}

/// Refused because a job holds the slot.
#[derive(Debug, PartialEq, Eq)]
pub struct Busy {
    pub job_id: u64,
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
            report,
            error,
            detail,
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
}

impl Jobs {
    fn reserve(&mut self) -> Result<u64, Busy> {
        match &self.slot {
            Slot::Idle => {
                self.next_id += 1;
                self.slot = Slot::Reserved(self.next_id);
                Ok(self.next_id)
            }
            Slot::Reserved(job_id) | Slot::Running(Running { id: job_id, .. }) => Err(Busy { job_id: *job_id }),
        }
    }

    fn release(&mut self, id: u64) {
        if matches!(self.slot, Slot::Reserved(held) if held == id) {
            self.slot = Slot::Idle;
        }
    }

    fn run(&mut self, id: u64, meta: Meta, control: ImportControl) {
        if matches!(self.slot, Slot::Reserved(held) if held == id) {
            self.slot = Slot::Running(Running { id, meta, control });
        }
    }

    fn finish(&mut self, id: u64, end: End, now_ms: i64) {
        match std::mem::take(&mut self.slot) {
            Slot::Running(running) if running.id == id => self.last = Some(Arc::new(running.ended(end, now_ms))),
            other => self.slot = other,
        }
    }

    fn view(&self, id: u64) -> Option<Arc<JobView>> {
        match &self.slot {
            Slot::Running(running) if running.id == id => Some(Arc::new(running.view())),
            _ => self.last.clone().filter(|last| last.job_id == id),
        }
    }
}

/// One import job at a time, and the last one that ended.
#[derive(Default)]
pub struct ImportJobs {
    state: Mutex<Jobs>,
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

    /// The job starts, and the slot is its until it ends.
    pub fn run(mut self, meta: Meta, control: ImportControl) {
        self.accepted = true;
        self.jobs.lock().run(self.id, meta, control);
    }
}

impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        if !self.accepted {
            self.jobs.lock().release(self.id);
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
        let id = self.lock().reserve()?;
        Ok(Reservation { jobs: self, id, accepted: false })
    }

    /// Ends job `id`, if it is the one running.
    pub fn finish(&self, id: u64, end: End, now_ms: i64) {
        self.lock().finish(id, end, now_ms);
    }

    /// The job that runs or the last one that ended, if it is `id`.
    pub fn view(&self, id: u64) -> Option<Arc<JobView>> {
        self.lock().view(id)
    }
}

#[cfg(test)]
mod tests {
    use clawg_core::memory::portability::timing::Step;
    use clawg_core::memory::portability::{ImportControl, ImportReport, Stopped, StopCause};

    use super::*;

    fn meta() -> Meta {
        Meta { dry_run: false, from: "clawg", started_at_ms: 1_000 }
    }

    /// A job that was accepted and runs.
    fn running(jobs: &ImportJobs) -> (u64, ImportControl) {
        let reservation = jobs.reserve().expect("a free slot");
        let (id, control) = (reservation.id(), ImportControl::default());
        reservation.run(meta(), control.clone());
        (id, control)
    }

    #[test]
    fn a_reservation_is_atomic_and_released_when_dropped() {
        let jobs = ImportJobs::default();
        let first = jobs.reserve().expect("the first reservation");
        let busy = jobs.reserve().err().expect("the slot is held");
        assert_eq!(busy.job_id, first.id(), "the refusal names the job that holds the slot");

        drop(first);
        let again = jobs.reserve().expect("a dropped reservation frees the slot");
        assert_eq!(again.id(), 2, "an id is never handed out twice");

        let control = ImportControl::default();
        let id = again.id();
        again.run(meta(), control);
        assert_eq!(jobs.reserve().err().map(|busy| busy.job_id), Some(id), "a running job holds the slot");

        let contested = ImportJobs::default();
        let wins = std::thread::scope(|scope| {
            let jobs = &contested;
            let racers: Vec<_> = (0..16).map(|_| scope.spawn(move || jobs.reserve())).collect();
            let results: Vec<_> = racers.into_iter().map(|racer| racer.join().expect("a racer")).collect();
            results.iter().filter(|result| result.is_ok()).count()
        });
        assert_eq!(wins, 1, "of many reservations at once exactly one wins");
    }

    #[test]
    fn a_reserved_id_that_is_never_accepted_is_a_404() {
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

    #[test]
    fn only_the_last_finished_job_is_kept() {
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

    #[test]
    fn a_finish_for_a_job_that_is_not_running_changes_nothing() {
        let jobs = ImportJobs::default();
        jobs.finish(7, End::Done(ImportReport::default()), 1);
        assert!(jobs.view(7).is_none());

        let (id, _) = running(&jobs);
        jobs.finish(id + 1, End::Done(ImportReport::default()), 1);
        assert_eq!(jobs.view(id).expect("still running").phase, "planning");
    }

    #[test]
    fn a_running_job_shows_what_was_asked_of_it() {
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

    #[test]
    fn a_finished_job_says_how_it_ended() {
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
